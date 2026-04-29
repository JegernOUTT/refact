use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::buddy::types::BuddyPage;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolBuddyOpenView {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolBuddyOpenView {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_open_view".to_string(),
            display_name: "Buddy Open View".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Navigate the user's GUI to a specific Refact page (Buddy home, Stats, Customization, Providers, Default Models, Integrations, Extensions, Marketplace Hub & sub-marketplaces, Tasks list/workspace, Knowledge Graph).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "page": {
                        "type": "object",
                        "description": "The page to navigate to.",
                        "properties": {
                            "type": {
                                "type": "string",
                                "description": "Page type identifier.",
                                "enum": [
                                    "buddy", "stats", "customization", "providers",
                                    "default_models", "integrations", "extensions",
                                    "marketplace_hub", "marketplace", "skills_marketplace",
                                    "commands_marketplace", "delegates_marketplace",
                                    "tasks_list", "task_workspace", "knowledge_graph"
                                ]
                            },
                            "task_id": {
                                "type": "string",
                                "description": "Required when type=task_workspace."
                            }
                        },
                        "required": ["type"]
                    }
                },
                "required": ["page"]
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
        let page_val = args.get("page").ok_or("argument `page` is missing")?;
        let page: BuddyPage =
            serde_json::from_value(page_val.clone()).map_err(|e| format!("invalid page: {}", e))?;
        let page_type = serde_json::to_value(&page)
            .ok()
            .and_then(|v| {
                v.get("type")
                    .and_then(|t| t.as_str().map(|s| s.to_string()))
            })
            .unwrap_or_default();

        let gcx = ccx.lock().await.global_context.clone();
        let buddy_arc = gcx.read().await.buddy.clone();
        let lock = buddy_arc.lock().await;
        let svc = lock
            .as_ref()
            .ok_or_else(|| "buddy service not initialized".to_string())?;
        svc.send_navigation(page);

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!(
                    "Navigation request sent: open '{}'",
                    page_type
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
