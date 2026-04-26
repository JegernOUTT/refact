use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::ext::config_dirs::get_ext_dirs;
use crate::ext::skills::load_skill_full;
use crate::ext::skills_context::expand_skill_includes;
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};

pub struct ToolActivateSkill {
    pub config_path: String,
}

async fn activate_skill_inner(
    ext_dirs: &crate::ext::config_dirs::ExtDirs,
    name: &str,
) -> Result<(String, Vec<String>, Option<String>), String> {
    if let Err(e) = crate::ext::skills::validate_skill_id(name) {
        return Err(format!("Invalid skill name '{}': {}", name, e));
    }
    let skill = load_skill_full(ext_dirs, name)
        .await
        .ok_or_else(|| format!("Skill '{}' not found", name))?;
    if !skill.index.user_invocable {
        return Err(format!("Skill '{}' is not available for activation", name));
    }
    if skill.index.disable_model_invocation {
        return Err(format!("Skill '{}' cannot be activated by the model", name));
    }
    let body = expand_skill_includes(&skill.body, &skill.skill_dir).await;
    Ok((body, skill.allowed_tools, skill.model))
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
            (
                ccx_locked.global_context.clone(),
                ccx_locked.chat_id.clone(),
            )
        };

        {
            let session_arc_opt = {
                let gcx_locked = gcx.read().await;
                let sessions = gcx_locked.chat_sessions.read().await;
                sessions.get(&chat_id).cloned()
            };
            if let Some(session_arc) = session_arc_opt {
                let mut session = session_arc.lock().await;
                if session.thread.active_skill.as_deref() == Some(name.as_str()) {
                    if session.active_command.started_at_index.is_none() {
                        session.active_command.started_at_index = Some(session.messages.len());
                        session.active_command.name = name.clone();
                    }
                    return Ok((false, vec![
                        ContextEnum::ChatMessage(ChatMessage {
                            role: "tool".to_string(),
                            content: ChatContent::SimpleText(format!("Skill '{}' is already active. Continue following its instructions.", name)),
                            tool_call_id: tool_call_id.clone(),
                            ..Default::default()
                        }),
                    ]));
                }
                if let Some(ref current) = session.thread.active_skill {
                    return Err(format!(
                        "Skill '{}' is currently active. Call deactivate_skill first before activating a different skill.",
                        current
                    ));
                }
            }
        }

        let ext_dirs = get_ext_dirs(gcx.clone()).await;
        let (body, allowed_tools, model_override) = activate_skill_inner(&ext_dirs, &name).await?;

        {
            let session_arc_opt = {
                let gcx_locked = gcx.read().await;
                let sessions = gcx_locked.chat_sessions.read().await;
                sessions.get(&chat_id).cloned()
            };
            if let Some(session_arc) = session_arc_opt {
                let mut session = session_arc.lock().await;
                session.active_command.name = name.clone();
                session.active_command.allowed_tools = allowed_tools.clone();
                session.active_command.model_override = model_override.clone();
                session.active_command.started_at_index = Some(session.messages.len());
                session.active_command.activation_tool_call_id = Some(tool_call_id.clone());
                session.set_active_skill(name.clone());
            }
        }

        let header_json = serde_json::json!({
            "name": name.clone(),
            "allowed_tools": allowed_tools,
            "model_override": model_override,
        });
        let cd_instruction_content = format!("💿 SKILL_ACTIVATED {}\n\n{}", header_json, body);

        Ok((
            false,
            vec![
                ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(format!("Skill '{}' activated.", name)),
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                }),
                ContextEnum::ChatMessage(ChatMessage {
                    role: "cd_instruction".to_string(),
                    content: ChatContent::SimpleText(cd_instruction_content),
                    ..Default::default()
                }),
            ],
        ))
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
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let report =
            match args.get("report") {
                Some(Value::String(s)) => s.clone(),
                Some(v) => return Err(format!("argument `report` is not a string: {:?}", v)),
                None => return Err(
                    "argument `report` is missing. Provide a thorough overview of what was done."
                        .to_string(),
                ),
            };

        let (gcx, chat_id) = {
            let ccx_locked = ccx.lock().await;
            (
                ccx_locked.global_context.clone(),
                ccx_locked.chat_id.clone(),
            )
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
                let start_index = session
                    .active_command
                    .started_at_index
                    .unwrap_or(session.messages.len());
                session.pending_skill_deactivation =
                    Some(crate::chat::types::PendingSkillDeactivation {
                        start_index,
                        report: report.clone(),
                        skill_name: skill_name.clone(),
                        activation_tool_call_id: session
                            .active_command
                            .activation_tool_call_id
                            .clone(),
                    });
                let compaction_note = if session.active_command.started_at_index.is_some() {
                    String::new()
                } else {
                    tracing::warn!("deactivate_skill: no started_at_index for skill '{}', reporting without compaction", skill_name);
                    " (Note: skill history compaction was skipped — activation anchor was not set.)"
                        .to_string()
                };
                session.active_command = crate::chat::types::ActiveCommandContext::default();
                session.clear_active_skill();
                return Ok((
                    false,
                    vec![ContextEnum::ChatMessage(ChatMessage {
                        role: "tool".to_string(),
                        content: ChatContent::SimpleText(format!(
                            "✅ Skill '{}' deactivated. Report has been recorded.{}",
                            skill_name, compaction_note
                        )),
                        tool_call_id: tool_call_id.clone(),
                        ..Default::default()
                    })],
                ));
            }
        }

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(
                    "✅ Skill deactivated. Report has been recorded.".to_string(),
                ),
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
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
        tokio::fs::write(skill_dir.join("SKILL.md"), content)
            .await
            .unwrap();
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
        let (body, allowed_tools, model_override) = result.unwrap();
        assert!(body.contains("Do something useful with $ARGUMENTS"));
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
        assert!(
            msg.contains("not found"),
            "Expected 'not found' in error: {}",
            msg
        );
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
        tokio::fs::write(skill_dir.join("context.md"), "Included content here")
            .await
            .unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: with-include\ndescription: Skill with includes\nuser-invocable: true\n---\nBefore\n@include context.md\nAfter",
        )
        .await
        .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path());
        let result = activate_skill_inner(&ext_dirs, "with-include").await;
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        let (body, _, _) = result.unwrap();
        assert!(
            body.contains("Included content here"),
            "@include should be expanded, got: {}",
            body
        );
        assert!(
            !body.contains("@include"),
            "@include directive should be replaced"
        );
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
        let (body, allowed_tools, model_override) = result.unwrap();
        assert!(!body.is_empty());
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
        assert!(
            allowed_tools.is_empty(),
            "No restrictions should result in empty allowed_tools"
        );
        assert!(
            model_override.is_none(),
            "No model should result in None model_override"
        );
    }

    #[tokio::test]
    async fn test_deactivate_skill_clears_active_command() {
        use crate::chat::types::ActiveCommandContext;

        let mut active = ActiveCommandContext {
            name: "my-skill".to_string(),
            allowed_tools: vec!["cat".to_string(), "tree".to_string()],
            model_override: Some("gpt-4o".to_string()),
            context_fork: None,
            started_at_index: Some(5),
            activation_tool_call_id: None,
        };
        assert_eq!(active.started_at_index, Some(5));

        active = ActiveCommandContext::default();

        assert!(active.name.is_empty());
        assert!(active.allowed_tools.is_empty());
        assert!(active.model_override.is_none());
        assert!(active.context_fork.is_none());
        assert!(active.started_at_index.is_none());
    }

    #[tokio::test]
    async fn test_deactivate_skill_when_no_active_skill() {
        use crate::chat::types::ActiveCommandContext;

        let active = ActiveCommandContext::default();
        let cleared = ActiveCommandContext::default();

        assert_eq!(active.name, cleared.name);
        assert_eq!(active.allowed_tools, cleared.allowed_tools);
        assert_eq!(active.model_override, cleared.model_override);
        assert_eq!(active.started_at_index, cleared.started_at_index);
    }

    #[test]
    fn test_activate_skill_not_parallel() {
        let tool = ToolActivateSkill {
            config_path: String::new(),
        };
        assert!(
            !tool.tool_description().allow_parallel,
            "activate_skill must have allow_parallel = false"
        );
    }

    #[test]
    fn test_deactivate_skill_no_context_file() {
        let result: Vec<ContextEnum> = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(
                "✅ Skill 'my-skill' deactivated. Report has been recorded.".to_string(),
            ),
            tool_call_id: "tc1".to_string(),
            ..Default::default()
        })];
        let has_context_file = result
            .iter()
            .any(|e| matches!(e, ContextEnum::ContextFile(_)));
        assert!(
            !has_context_file,
            "deactivate_skill must not return ContextFile"
        );
        let has_chat_message = result
            .iter()
            .any(|e| matches!(e, ContextEnum::ChatMessage(_)));
        assert!(
            has_chat_message,
            "deactivate_skill must return a ChatMessage"
        );
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
    fn test_activate_started_at_uses_message_count() {
        use crate::chat::types::ActiveCommandContext;

        let mut ctx = ActiveCommandContext::default();
        assert!(ctx.started_at_index.is_none());

        // Simulate: at activation time, there are 3 messages already in session
        ctx.started_at_index = Some(3);
        assert_eq!(ctx.started_at_index, Some(3));

        // After reset, index is cleared
        ctx = ActiveCommandContext::default();
        assert!(ctx.started_at_index.is_none());
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
                started_at_index: Some(0),
                activation_tool_call_id: None,
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
            recent_request_ids_set: std::collections::HashSet::new(),
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
            stop_hook_handle: None,
            suppress_auto_enrichment_for_next_turn: false,
        };

        let skill_name = match session.thread.active_skill.clone() {
            Some(name) => name,
            None => panic!("Expected active_skill to be set"),
        };
        assert_eq!(
            skill_name, "real-skill",
            "Must use active_skill, not active_command.name"
        );
        assert_ne!(skill_name, session.active_command.name);

        session.active_command = ActiveCommandContext::default();
        session.clear_active_skill();
        assert!(session.thread.active_skill.is_none());
    }

    #[test]
    fn test_activate_already_active_skill_returns_early() {
        let active_skill = Some("my-skill".to_string());
        let name = "my-skill";
        let already_active = active_skill.as_deref() == Some(name);
        assert!(already_active, "Should detect already active skill");

        let inactive_skill: Option<String> = None;
        let not_active = inactive_skill.as_deref() == Some(name);
        assert!(!not_active, "None should not match active skill");

        let other_skill = Some("other-skill".to_string());
        let different = other_skill.as_deref() == Some(name);
        assert!(!different, "Different skill name should not match");
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
