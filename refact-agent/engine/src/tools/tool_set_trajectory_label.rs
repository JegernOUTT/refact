use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::trajectories::{set_trajectory_label, TRAJECTORY_LABEL_MAX_CHARS};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolSetTrajectoryLabel {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolSetTrajectoryLabel {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "set_trajectory_label".to_string(),
            display_name: "Set Trajectory Label".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Set a short user-authored label for the current trajectory.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "label": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": TRAJECTORY_LABEL_MAX_CHARS,
                        "description": "Non-empty trajectory label (at most 120 characters)."
                    }
                },
                "required": ["label"]
            }),
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
        let label = match args.get("label") {
            Some(Value::String(label)) => label.as_str(),
            _ => return Err("argument `label` must be a string".to_string()),
        };
        let (app, invoking_chat_id) = {
            let ccx = ccx.lock().await;
            (ccx.app.clone(), ccx.chat_id.clone())
        };
        let normalized_label = crate::chat::trajectories::validate_trajectory_label(label)?;
        set_trajectory_label(app, &invoking_chat_id, &normalized_label).await?;
        let result = json!({
            "trajectory_id": invoking_chat_id,
            "label": normalized_label,
            "is_title_generated": false
        });
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result.to_string()),
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::trajectories::load_trajectory_for_chat;
    use crate::chat::types::ChatSession;

    async fn context(
        gcx: Arc<crate::global_context::GlobalContext>,
        chat_id: &str,
    ) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                chat_id.to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    #[tokio::test]
    async fn labels_invoking_live_session_and_persists_it() {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let chat_id = "label-current-session";
        let session = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        gcx.chat_sessions
            .write()
            .await
            .insert(chat_id.to_string(), session.clone());
        let mut tool = ToolSetTrajectoryLabel {
            config_path: String::new(),
        };
        tool.tool_execute(
            context(gcx.clone(), chat_id).await,
            &"call-1".to_string(),
            &HashMap::from([("label".to_string(), json!("Focused label"))]),
        )
        .await
        .unwrap();

        let session = session.lock().await;
        assert_eq!(session.thread.title, "Focused label");
        assert!(!session.thread.is_title_generated);
        drop(session);
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.thread.title, "Focused label");
        assert!(!loaded.thread.is_title_generated);
    }

    #[test]
    fn validates_required_non_empty_and_bounded_label() {
        assert!(crate::chat::trajectories::validate_trajectory_label("  ").is_err());
        assert!(crate::chat::trajectories::validate_trajectory_label(
            &"x".repeat(TRAJECTORY_LABEL_MAX_CHARS + 1)
        )
        .is_err());
        assert_eq!(
            crate::chat::trajectories::validate_trajectory_label("  valid\n label  ").unwrap(),
            "valid label"
        );
    }
}
