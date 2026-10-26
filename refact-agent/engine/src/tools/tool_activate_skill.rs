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

#[derive(Debug)]
struct ActivatedSkill {
    body: String,
    allowed_tools: Vec<String>,
    model_override: Option<String>,
    skill_dir: String,
}

fn build_cd_instruction_content(name: &str, activated: &ActivatedSkill) -> String {
    let header_json = serde_json::json!({
        "name": name,
        "allowed_tools": &activated.allowed_tools,
        "model_override": &activated.model_override,
        "skill_dir": &activated.skill_dir,
    });
    format!(
        "💿 SKILL_ACTIVATED {}\n\nSkill directory: `{}`\n\nUse absolute paths under this directory when reading skill companion files (docs, scripts, assets). Do not use workspace-relative paths like `skills/{}/...`.\n\n{}",
        header_json,
        activated.skill_dir,
        name,
        activated.body
    )
}

async fn activate_skill_inner(
    ext_dirs: &crate::ext::config_dirs::ExtDirs,
    name: &str,
) -> Result<ActivatedSkill, String> {
    if let Err(e) = crate::ext::skills::validate_skill_id(name) {
        return Err(format!("Invalid skill name '{}': {}", name, e));
    }
    let skill = load_skill_full(ext_dirs, name)
        .await
        .ok_or_else(|| format!("Skill '{}' not found", name))?;
    if !skill.index.user_invocable {
        return Err(format!("Skill '{}' is not available for activation", name));
    }
    let body = expand_skill_includes(&skill.body, &skill.skill_dir).await;
    Ok(ActivatedSkill {
        body,
        allowed_tools: skill.allowed_tools,
        model_override: skill.model,
        skill_dir: skill.skill_dir.display().to_string(),
    })
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
            description: "Load a skill's full instructions into the current context. Use when you determine a skill from the available index is relevant to the user's request. Once activated, the skill's instructions guide your approach.".to_string(),
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
            let cgcx = ccx.lock().await;
            (cgcx.app.gcx.clone(), cgcx.chat_id.clone())
        };

        {
            let session_arc_opt = {
                let sessions = gcx.chat_sessions.read().await;
                sessions.get(&chat_id).cloned()
            };
            if let Some(session_arc) = session_arc_opt {
                let session = session_arc.lock().await;
                if session.thread.active_skill.as_deref() == Some(name.as_str()) {
                    return Ok((false, vec![
                        ContextEnum::ChatMessage(ChatMessage {
                            role: "tool".to_string(),
                            content: ChatContent::SimpleText(format!("Skill '{}' is already active. Continue following its instructions.", name)),
                            tool_call_id: tool_call_id.clone(),
                            ..Default::default()
                        }),
                    ]));
                }
            }
        }

        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let ext_dirs = get_ext_dirs(app).await;
        let activated = activate_skill_inner(&ext_dirs, &name).await?;

        {
            let session_arc_opt = {
                let sessions = gcx.chat_sessions.read().await;
                sessions.get(&chat_id).cloned()
            };
            if let Some(session_arc) = session_arc_opt {
                let mut session = session_arc.lock().await;
                session.active_command.name = name.clone();
                session.active_command.allowed_tools = activated.allowed_tools.clone();
                session.active_command.model_override = activated.model_override.clone();
                session.set_active_skill(name.clone());
            }
        }

        let cd_instruction_content = build_cd_instruction_content(&name, &activated);

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
        let activated = result.unwrap();
        assert!(activated
            .body
            .contains("Do something useful with $ARGUMENTS"));
        assert!(activated.allowed_tools.is_empty());
        assert!(activated.model_override.is_none());
        assert!(Path::new(&activated.skill_dir).ends_with(Path::new("skills").join("my-skill")));
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
}
