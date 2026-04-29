use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::buddy::types::DraftKind;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolBuddyCreateDraft {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolBuddyCreateDraft {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_create_draft".to_string(),
            display_name: "Buddy Create Draft".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Create a Buddy draft (skill, command, delegate, mode, agents_md, defaults_model, hook, pulse_report). Returns a draft_id the user can navigate to in the corresponding editor for review/save.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "description": "Draft type.",
                        "enum": ["skill", "command", "delegate", "mode", "agents_md", "defaults_model", "hook", "pulse_report"]
                    },
                    "title": {
                        "type": "string",
                        "description": "Human-readable title for the draft."
                    },
                    "yaml_or_json": {
                        "type": "string",
                        "description": "The draft content as YAML or JSON string."
                    },
                    "explanation": {
                        "type": "string",
                        "description": "Brief explanation of what this draft does and why."
                    }
                },
                "required": ["kind", "title", "yaml_or_json", "explanation"]
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
        let kind_str = args
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or("argument `kind` is missing or not a string")?;
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or("argument `title` is missing or not a string")?
            .to_string();
        let yaml_or_json = args
            .get("yaml_or_json")
            .and_then(|v| v.as_str())
            .ok_or("argument `yaml_or_json` is missing or not a string")?
            .to_string();
        let explanation = args
            .get("explanation")
            .and_then(|v| v.as_str())
            .ok_or("argument `explanation` is missing or not a string")?
            .to_string();

        let kind: DraftKind = serde_json::from_value(serde_json::json!(kind_str))
            .map_err(|e| format!("invalid kind '{}': {}", kind_str, e))?;

        let gcx = ccx.lock().await.global_context.clone();
        let buddy_arc = gcx.read().await.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        let svc = lock.as_mut().ok_or("buddy service not initialized")?;

        let draft = svc
            .create_draft(kind, title.clone(), yaml_or_json, explanation)
            .map_err(|e| e.to_string())?;

        let draft_id = draft.id.clone();
        let expires_at = draft.expires_at.to_rfc3339();
        let kind_label = serde_json::to_value(&draft.kind)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!(
                    "Draft {} '{}' created (id: {}, expires: {}). Open the editor to review.",
                    kind_label, title, draft_id, expires_at
                )),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
