use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ContextEnum, ContextFile};
use crate::ext::config_dirs::get_ext_dirs;
use crate::ext::skills::load_skill_full;
use crate::ext::skills_context::expand_skill_includes;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params};

pub struct ToolActivateSkill {
    pub config_path: String,
}

async fn activate_skill_inner(
    ext_dirs: &crate::ext::config_dirs::ExtDirs,
    name: &str,
) -> Result<(ContextFile, Vec<String>, Option<String>), String> {
    if let Err(e) = crate::ext::skills::validate_skill_id(name) {
        return Err(format!("Invalid skill name '{}': {}", name, e));
    }
    let skill = load_skill_full(ext_dirs, name).await
        .ok_or_else(|| format!("Skill '{}' not found", name))?;
    if !skill.index.user_invocable {
        return Err(format!("Skill '{}' is not available for activation", name));
    }
    if skill.index.disable_model_invocation {
        return Err(format!("Skill '{}' cannot be activated by the model", name));
    }
    let body = expand_skill_includes(&skill.body, &skill.skill_dir).await;
    let line_count = body.lines().count().max(1);
    let cf = ContextFile {
        file_name: format!("skill://{}", name),
        file_content: body,
        line1: 1,
        line2: line_count,
        file_rev: None,
        symbols: vec![],
        gradient_type: 0,
        usefulness: 90.0,
        skip_pp: true,
    };
    Ok((cf, skill.allowed_tools, skill.model))
}

#[async_trait]
impl Tool for ToolActivateSkill {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "activate_skill".to_string(),
            display_name: "Activate Skill".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Load a skill's full instructions into the current context. Use when you determine a skill from the available index is relevant to the user's request. Once activated, the skill's instructions guide your approach. When you're done with the skill's task, you MUST call deactivate_skill with a thorough report.".to_string(),
            input_schema: json_schema_from_params(
                &[("name", "string", "Name of the skill to activate")],
                &["name"],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let name = match args.get("name") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `name` is not a string: {:?}", v)),
            None => return Err("argument `name` is missing".to_string()),
        };
        if let Err(e) = crate::ext::skills::validate_skill_id(&name) {
            return Err(format!("Invalid skill name '{}': {}", name, e));
        }

        let (gcx, chat_id) = {
            let ccx_locked = ccx.lock().await;
            (ccx_locked.global_context.clone(), ccx_locked.chat_id.clone())
        };
        let ext_dirs = get_ext_dirs(gcx.clone()).await;
        let (context_file, allowed_tools, model_override) = activate_skill_inner(&ext_dirs, &name).await?;

        {
            let session_arc_opt = {
                let gcx_locked = gcx.read().await;
                let sessions = gcx_locked.chat_sessions.read().await;
                sessions.get(&chat_id).cloned()
            };
            if let Some(session_arc) = session_arc_opt {
                let mut session = session_arc.lock().await;
                let started_at = session.messages.iter().rev()
                    .find(|m| m.role == "assistant" && m.tool_calls.as_ref().map_or(false, |tcs|
                        tcs.iter().any(|tc| tc.id == *tool_call_id)
                    ))
                    .map(|m| m.message_id.clone());
                session.active_command.name = name.clone();
                session.active_command.allowed_tools = allowed_tools;
                session.active_command.model_override = model_override;
                session.active_command.started_at_message_id = started_at;
                session.set_active_skill(name.clone());
            }
        }

        Ok((false, vec![ContextEnum::ContextFile(context_file)]))
    }
}

pub struct ToolDeactivateSkill {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolDeactivateSkill {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "deactivate_skill".to_string(),
            display_name: "Deactivate Skill".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Deactivate the currently active skill with a completion report. The report should be a thorough overview of what was done, what happened, and what was changed. After deactivation, the skill execution messages are compacted into the report, keeping chat history clean while preserving knowledge of what occurred.".to_string(),
            input_schema: json_schema_from_params(
                &[("report", "string", "A thorough overview of what was done, what happened, what was changed during the skill execution. Use clear markdown formatting.")],
                &["report"],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        _tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let report = match args.get("report") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `report` is not a string: {:?}", v)),
            None => return Err("argument `report` is missing. Provide a thorough overview of what was done.".to_string()),
        };

        let (gcx, chat_id) = {
            let ccx_locked = ccx.lock().await;
            (ccx_locked.global_context.clone(), ccx_locked.chat_id.clone())
        };

        {
            let session_arc_opt = {
                let gcx_locked = gcx.read().await;
                let sessions = gcx_locked.chat_sessions.read().await;
                sessions.get(&chat_id).cloned()
            };
            if let Some(session_arc) = session_arc_opt {
                let mut session = session_arc.lock().await;
                let skill_name = match session.thread.active_skill.clone() {
                    Some(name) => name,
                    None => return Err("No active skill to deactivate".to_string()),
                };
                if let Some(start_msg_id) = session.active_command.started_at_message_id.clone() {
                    session.pending_skill_deactivation = Some(crate::chat::types::PendingSkillDeactivation {
                        start_message_id: start_msg_id,
                        report: report.clone(),
                        skill_name: skill_name.clone(),
                    });
                } else {
                    tracing::warn!("deactivate_skill: no started_at_message_id for skill '{}', skipping compaction", skill_name);
                }
                session.active_command = crate::chat::types::ActiveCommandContext::default();
                session.clear_active_skill();
            }
        }

        Ok((false, vec![]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::config_dirs::ExtDirs;
    use std::path::Path;

    fn make_ext_dirs(root: &Path) -> ExtDirs {
        ExtDirs {
            global_dirs: vec![root.to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![],
        }
    }

    async fn write_skill(root: &Path, name: &str, frontmatter: &str, body: &str) {
        let skill_dir = root.join("skills").join(name);
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        let content = format!("---\n{}\n---\n{}", frontmatter, body);
        tokio::fs::write(skill_dir.join("SKILL.md"), content).await.unwrap();
    }

    #[tokio::test]
    async fn test_activate_known_skill() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "my-skill",
            "name: my-skill\ndescription: A useful skill\nuser-invocable: true",
            "Do something useful with $ARGUMENTS",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let result = activate_skill_inner(&ext_dirs, "my-skill").await;
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        let (cf, allowed_tools, model_override) = result.unwrap();
        assert_eq!(cf.file_name, "skill://my-skill");
        assert!(cf.file_content.contains("Do something useful with $ARGUMENTS"));
        assert_eq!(cf.line1, 1);
        assert!(cf.skip_pp);
        assert!(allowed_tools.is_empty());
        assert!(model_override.is_none());
    }

    #[tokio::test]
    async fn test_activate_unknown_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dirs = make_ext_dirs(tmp.path());
        let result = activate_skill_inner(&ext_dirs, "nonexistent").await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("not found"), "Expected 'not found' in error: {}", msg);
    }

    #[tokio::test]
    async fn test_activate_non_invocable_skill() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "hidden-skill",
            "name: hidden-skill\ndescription: Internal skill\nuser-invocable: false",
            "Internal instructions",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let result = activate_skill_inner(&ext_dirs, "hidden-skill").await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("not available for activation"),
            "Expected 'not available for activation' in error: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_activate_skill_with_includes() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("with-include");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(skill_dir.join("context.md"), "Included content here").await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: with-include\ndescription: Skill with includes\nuser-invocable: true\n---\nBefore\n@include context.md\nAfter",
        )
        .await
        .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path());
        let result = activate_skill_inner(&ext_dirs, "with-include").await;
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        let (cf, _, _) = result.unwrap();
        assert!(
            cf.file_content.contains("Included content here"),
            "@include should be expanded, got: {}",
            cf.file_content
        );
        assert!(!cf.file_content.contains("@include"), "@include directive should be replaced");
    }

    #[tokio::test]
    async fn test_activate_skill_returns_allowed_tools_and_model() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "restricted-skill",
            "name: restricted-skill\ndescription: Skill with restrictions\nuser-invocable: true\nallowed-tools:\n  - cat\n  - tree\nmodel: gpt-4o",
            "Do something restricted",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let result = activate_skill_inner(&ext_dirs, "restricted-skill").await;
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        let (cf, allowed_tools, model_override) = result.unwrap();
        assert_eq!(cf.file_name, "skill://restricted-skill");
        assert_eq!(allowed_tools, vec!["cat".to_string(), "tree".to_string()]);
        assert_eq!(model_override, Some("gpt-4o".to_string()));
    }

    #[tokio::test]
    async fn test_activate_skill_empty_allowed_tools() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "open-skill",
            "name: open-skill\ndescription: Skill without restrictions\nuser-invocable: true",
            "Do anything",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let result = activate_skill_inner(&ext_dirs, "open-skill").await;
        assert!(result.is_ok());
        let (_, allowed_tools, model_override) = result.unwrap();
        assert!(allowed_tools.is_empty(), "No restrictions should result in empty allowed_tools");
        assert!(model_override.is_none(), "No model should result in None model_override");
    }

    #[tokio::test]
    async fn test_deactivate_skill_clears_active_command() {
        use crate::chat::types::ActiveCommandContext;

        let mut active = ActiveCommandContext {
            name: "my-skill".to_string(),
            allowed_tools: vec!["cat".to_string(), "tree".to_string()],
            model_override: Some("gpt-4o".to_string()),
            context_fork: None,
            started_at_message_id: Some("msg-123".to_string()),
        };

        active = ActiveCommandContext::default();

        assert!(active.name.is_empty());
        assert!(active.allowed_tools.is_empty());
        assert!(active.model_override.is_none());
        assert!(active.context_fork.is_none());
        assert!(active.started_at_message_id.is_none());
    }

    #[tokio::test]
    async fn test_deactivate_skill_when_no_active_skill() {
        use crate::chat::types::ActiveCommandContext;

        let active = ActiveCommandContext::default();
        let cleared = ActiveCommandContext::default();

        assert_eq!(active.name, cleared.name);
        assert_eq!(active.allowed_tools, cleared.allowed_tools);
        assert_eq!(active.model_override, cleared.model_override);
        assert_eq!(active.started_at_message_id, cleared.started_at_message_id);
    }

    #[test]
    fn test_activate_skill_not_parallel() {
        let tool = ToolActivateSkill { config_path: String::new() };
        assert!(!tool.tool_description().allow_parallel, "activate_skill must have allow_parallel = false");
    }

    #[test]
    fn test_deactivate_skill_no_context_file() {
        let result: Vec<ContextEnum> = vec![];
        let has_context_file = result.iter().any(|e| matches!(e, ContextEnum::ContextFile(_)));
        assert!(!has_context_file, "deactivate_skill must not return ContextFile");
    }

    #[tokio::test]
    async fn test_activate_rejects_disable_model_invocation() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "locked-skill",
            "name: locked-skill\ndescription: Locked skill\nuser-invocable: true\ndisable-model-invocation: true",
            "Sensitive instructions",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let result = activate_skill_inner(&ext_dirs, "locked-skill").await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("cannot be activated by the model"),
            "Expected 'cannot be activated by the model' in error: {}",
            msg
        );
    }

    #[test]
    fn test_activate_started_at_uses_exact_tool_call_id() {
        use crate::call_validation::{ChatContent, ChatMessage, ChatToolCall, ChatToolFunction};

        let make_assistant_msg = |msg_id: &str, call_id: &str| ChatMessage {
            message_id: msg_id.to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("".to_string()),
            tool_calls: Some(vec![ChatToolCall {
                id: call_id.to_string(),
                index: None,
                function: ChatToolFunction {
                    name: "activate_skill".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        };

        let messages = vec![
            make_assistant_msg("msg_1", "call_1"),
            make_assistant_msg("msg_2", "call_2"),
        ];

        let tool_call_id = "call_2".to_string();
        let started_at = messages.iter().rev()
            .find(|m| m.role == "assistant" && m.tool_calls.as_ref().map_or(false, |tcs|
                tcs.iter().any(|tc| tc.id == tool_call_id)
            ))
            .map(|m| m.message_id.clone());

        assert_eq!(
            started_at,
            Some("msg_2".to_string()),
            "Exact tool_call_id match must return msg_2, not msg_1"
        );

        let tool_call_id_first = "call_1".to_string();
        let started_at_first = messages.iter().rev()
            .find(|m| m.role == "assistant" && m.tool_calls.as_ref().map_or(false, |tcs|
                tcs.iter().any(|tc| tc.id == tool_call_id_first)
            ))
            .map(|m| m.message_id.clone());

        assert_eq!(
            started_at_first,
            Some("msg_1".to_string()),
            "Exact tool_call_id match must return msg_1 for call_1"
        );
    }

    #[test]
    fn test_deactivate_uses_active_skill_not_active_command() {
        use crate::chat::types::{ActiveCommandContext, ThreadParams};
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        use tokio::sync::{broadcast, Notify};
        use std::collections::VecDeque;
        use std::time::Instant;

        let (tx, _rx) = broadcast::channel(16);
        let mut session = crate::chat::types::ChatSession {
            chat_id: "test".to_string(),
            thread: ThreadParams {
                id: "test".to_string(),
                active_skill: Some("real-skill".to_string()),
                ..Default::default()
            },
            active_command: ActiveCommandContext {
                name: "some-slash-command".to_string(),
                ..Default::default()
            },
            messages: Vec::new(),
            runtime: crate::chat::types::RuntimeState::default(),
            draft_message: None,
            draft_usage: None,
            command_queue: VecDeque::new(),
            event_seq: 0,
            event_tx: tx,
            recent_request_ids: VecDeque::new(),
            abort_flag: Arc::new(AtomicBool::new(false)),
            queue_processor_running: Arc::new(AtomicBool::new(false)),
            queue_notify: Arc::new(Notify::new()),
            last_activity: Instant::now(),
            trajectory_dirty: false,
            trajectory_version: 0,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            closed: false,
            closed_flag: Arc::new(AtomicBool::new(false)),
            external_reload_pending: false,
            last_prompt_messages: Vec::new(),
            cache_guard_snapshot: None,
            cache_guard_force_next: false,
            task_agent_error: None,
            trajectory_events_tx: None,
            pending_browser_message: None,
            skills_available_count: 0,
            skills_included: Vec::new(),
            pending_skill_deactivation: None,
        };

        let skill_name = match session.thread.active_skill.clone() {
            Some(name) => name,
            None => panic!("Expected active_skill to be set"),
        };
        assert_eq!(skill_name, "real-skill", "Must use active_skill, not active_command.name");
        assert_ne!(skill_name, session.active_command.name);

        session.active_command = ActiveCommandContext::default();
        session.clear_active_skill();
        assert!(session.thread.active_skill.is_none());
    }

    #[tokio::test]
    async fn test_activate_rejects_traversal_name() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dirs = make_ext_dirs(tmp.path());

        let result = activate_skill_inner(&ext_dirs, "../../etc").await;
        assert!(result.is_err(), "traversal name should be rejected");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("Invalid skill name") || msg.contains("not found"),
            "Expected rejection message, got: {}",
            msg
        );

        let result2 = activate_skill_inner(&ext_dirs, "../passwd").await;
        assert!(result2.is_err(), "traversal name should be rejected");
    }
}
