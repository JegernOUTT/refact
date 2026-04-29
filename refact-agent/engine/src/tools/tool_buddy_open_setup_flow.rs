use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::buddy::types::BuddyPage;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const VALID_FLOWS: &[&str] = &[
    "setup",
    "setup_mcp",
    "setup_skills",
    "setup_commands",
    "setup_agents_md",
    "setup_subagents",
];

pub struct ToolBuddyOpenSetupFlow {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolBuddyOpenSetupFlow {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_open_setup_flow".to_string(),
            display_name: "Buddy Open Setup Flow".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Navigate the frontend to a specific setup flow. The GUI will open the specified setup mode as a new chat.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "flow": {
                        "type": "string",
                        "description": "Which setup flow to launch.",
                        "enum": VALID_FLOWS
                    }
                },
                "required": ["flow"]
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
        let flow = args
            .get("flow")
            .and_then(|v| v.as_str())
            .ok_or("argument `flow` is missing or not a string")?
            .to_string();

        if !VALID_FLOWS.contains(&flow.as_str()) {
            return Err(format!(
                "invalid flow '{}'. Valid flows: {}",
                flow,
                VALID_FLOWS.join(", ")
            ));
        }

        let gcx = ccx.lock().await.global_context.clone();

        let buddy_arc = gcx.read().await.buddy.clone();
        let lock = buddy_arc.lock().await;
        let svc = lock
            .as_ref()
            .ok_or_else(|| "buddy service not initialized".to_string())?;
        svc.send_navigation(BuddyPage::SetupMode { mode: flow.clone() });

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!("Setup flow '{}' launched.", flow)),
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
