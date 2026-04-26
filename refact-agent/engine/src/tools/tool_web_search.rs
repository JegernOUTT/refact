use std::sync::Arc;
use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;
use serde_json::json;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_web_search::{
    clamp_num_results, execute_web_search_results, DEFAULT_NUM_RESULTS, MAX_NUM_RESULTS,
};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};

pub struct ToolWebSearch {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolWebSearch {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "web_search".to_string(),
            display_name: "Web Search".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Search the web and return results with titles, URLs, and snippets. Uses DuckDuckGo.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("query", "string", "Search query."),
                    (
                        "num_results",
                        "integer",
                        "Optional. Maximum number of results to return (default: 8, max: 20).",
                    ),
                ],
                &["query"],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let query = match args.get("query") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `query` is not a string: {:?}", v)),
            None => return Err("Missing argument `query`".to_string()),
        };

        let num_results = args
            .get("num_results")
            .and_then(|v| match v {
                Value::String(s) => s.parse::<usize>().ok(),
                Value::Number(n) => n.as_u64().map(|n| n as usize),
                _ => None,
            })
            .map(clamp_num_results)
            .unwrap_or(DEFAULT_NUM_RESULTS);

        if num_results > MAX_NUM_RESULTS {
            return Err(format!(
                "argument `num_results` must be at most {}",
                MAX_NUM_RESULTS
            ));
        }

        let (text, search_results) = execute_web_search_results(&query, num_results).await?;

        let mut extra = serde_json::Map::new();
        extra.insert(
            "search_results".to_string(),
            json!(search_results
                .into_iter()
                .map(|result| {
                    json!({
                        "title": result.title,
                        "url": result.url,
                        "snippet": result.snippet,
                    })
                })
                .collect::<Vec<_>>()),
        );

        let result = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            extra,
            ..Default::default()
        })];

        Ok((false, result))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
