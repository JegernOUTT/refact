use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolBuddyCreateIssue {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolBuddyCreateIssue {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_create_issue".to_string(),
            display_name: "Buddy Create Issue".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Create an issue for a confirmed product bug in the detected repository. \
                This helper checks GitHub MCP first, then existing integration/CLI fallback, \
                and only files when confidence is high."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Short issue title."
                    },
                    "body": {
                        "type": "string",
                        "description": "Issue body with investigation summary, repro, and evidence."
                    },
                    "confidence": {
                        "type": "string",
                        "description": "Confidence level. Must be 'high' or 'confirmed' to create automatically.",
                        "enum": ["medium", "high", "confirmed"]
                    },
                    "diagnostic_index": {
                        "type": "number",
                        "description": "Optional Buddy diagnostic index to associate with the issue helper fallback path."
                    },
                    "diagnostic_id": {
                        "type": "string",
                        "description": "Optional stable Buddy diagnostic identifier."
                    },
                    "collected_at": {
                        "type": "string",
                        "description": "Optional Buddy diagnostic collected_at timestamp in RFC3339 format."
                    },
                    "error": {
                        "type": "string",
                        "description": "Optional diagnostic error text when no diagnostic index is available."
                    },
                    "labels": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional GitHub labels for MCP issue creation."
                    }
                },
                "required": ["title", "body", "confidence"],
                "additionalProperties": false
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
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                "buddy_create_issue: missing required string argument 'title'".to_string()
            })?;
        let body = args
            .get("body")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                "buddy_create_issue: missing required string argument 'body'".to_string()
            })?;
        let confidence = args
            .get("confidence")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");

        if confidence != "high" && confidence != "confirmed" {
            return Err(
                "buddy_create_issue: confidence must be 'high' or 'confirmed' before filing"
                    .to_string(),
            );
        }

        let error = args
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let diagnostic_index = args
            .get("diagnostic_index")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let diagnostic_id = args
            .get("diagnostic_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let collected_at = args
            .get("collected_at")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let labels = args
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec!["bug".to_string(), "buddy".to_string()]);

        let gcx = ccx.lock().await.global_context.clone();

        let has_mcp = crate::buddy::issues::has_github_mcp(gcx.clone()).await;
        let result = if has_mcp {
            let context = crate::buddy::issues::resolve_issue_context(
                gcx.clone(),
                diagnostic_index,
                diagnostic_id,
                collected_at,
                error,
            )
            .await?;
            crate::buddy::issues::create_issue_via_mcp(
                gcx.clone(),
                &context,
                title,
                body,
                labels,
                false,
            )
            .await
        } else {
            crate::buddy::issues::create_issue_via_native(
                gcx.clone(),
                diagnostic_index,
                diagnostic_id,
                collected_at,
                error,
            )
            .await
        }?;

        let output = json!({
            "url": result.url,
            "provider": result.provider,
            "repo": result.repo,
        })
        .to_string();

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(output),
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
