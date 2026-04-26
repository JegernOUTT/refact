use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{
    Tool, ToolConfig, ToolDesc, ToolGroupCategory, ToolSource, ToolSourceType,
};
use crate::tools::tools_list::get_integration_tools;

pub struct ToolMcpSearch {}

#[async_trait]
impl Tool for ToolMcpSearch {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "mcp_tool_search".to_string(),
            experimental: false,
            allow_parallel: false,
            description:
                "Search available MCP tools by regex pattern (case-insensitive, matched \
                against tool name and description). Returns matching tool names and their full \
                JSON schemas as text. After discovering a tool here, call `mcp_call` to execute it."
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Regex pattern to match MCP tool names and descriptions. \
                            Examples: \"github\", \"file.*read|write\", \"git.*(commit|push)\""
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of tools to return (default 10)"
                    }
                },
                "required": ["query"]
            }),
            output_schema: None,
            annotations: None,
            display_name: "MCP Tool Search".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
        }
    }

    fn config(&self) -> Result<ToolConfig, String> {
        Ok(ToolConfig {
            enabled: true,
            allow_parallel: None,
        })
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        let re = regex::Regex::new(&format!("(?i){}", query))
            .map_err(|e| format!("Invalid regex pattern '{}': {}", query, e))?;

        let gcx = ccx.lock().await.global_context.clone();
        let integration_groups = get_integration_tools(gcx).await;

        let matched: Vec<(String, String, Value)> = integration_groups
            .iter()
            .filter(|g| matches!(g.category, ToolGroupCategory::MCP))
            .flat_map(|g| g.tools.iter())
            .filter(|tool| tool.config().unwrap_or_default().enabled)
            .filter(|tool| {
                let d = tool.tool_description();
                re.is_match(&d.name) || re.is_match(&d.description)
            })
            .take(max_results)
            .map(|tool| {
                let d = tool.tool_description();
                (d.name, d.description, d.input_schema)
            })
            .collect();

        let total_mcp: usize = integration_groups
            .iter()
            .filter(|g| matches!(g.category, ToolGroupCategory::MCP))
            .map(|g| g.tools.len())
            .sum();

        if matched.is_empty() {
            let text = format!(
                "No MCP tools found matching '{}'. Try a broader pattern. \
                 Use mcp_tool_search({{\"query\": \".\"}}) to list all {} available tools.",
                query, total_mcp
            );
            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    tool_call_id: tool_call_id.clone(),
                    content: ChatContent::SimpleText(text),
                    tool_failed: Some(false),
                    ..Default::default()
                })],
            ));
        }

        // Return schemas as text — no session state modified (cache-safe).
        let mut lines = vec![format!(
            "Found {} MCP tool(s) matching '{}'. Use `mcp_call` to execute them.\n",
            matched.len(),
            query
        )];
        for (name, description, schema) in &matched {
            lines.push(format!(
                "### {}\n{}\n\nInput schema:\n```json\n{}\n```\n",
                name,
                description,
                serde_json::to_string_pretty(schema).unwrap_or_default()
            ));
        }

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                tool_call_id: tool_call_id.clone(),
                content: ChatContent::SimpleText(lines.join("\n")),
                tool_failed: Some(false),
                ..Default::default()
            })],
        ))
    }
}
