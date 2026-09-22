use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::coordination::{metrics, metrics_view, scope};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetricsArgs {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

pub struct ToolMetrics {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolMetrics {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "metrics".into(),
            display_name: "Metrics".into(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Read the current bounded project metrics and auditable automatic-decision explanations. Verdict series remain distinct; entries are redacted and scoped to this chat's model destination.".into(),
            input_schema: json!({"type":"object", "additionalProperties":false, "properties": {
                "limit":{"type":"integer", "minimum":1, "maximum":crate::file_activity::settings().overview_max_rows},
                "cursor":{"type":"string", "description":"Opaque continuation; fetch a fresh snapshot on cursor_expired."}
            }}),
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
        let request: MetricsArgs = serde_json::from_value(json!(args))
            .map_err(|_| "invalid metrics request".to_string())?;
        let (app, chat_id, model) = {
            let ccx = ccx.lock().await;
            (
                ccx.app.clone(),
                ccx.chat_id.clone(),
                ccx.current_model.clone(),
            )
        };
        let caller = scope::model_caller(&app, &chat_id, &model)
            .await
            .map_err(|error| error.to_string())?;
        let page = metrics_view::page(
            &app,
            &caller,
            metrics_view::MetricsRequest {
                rows: request.limit,
                cursor: request.cursor,
            },
            metrics::recorder().snapshot(),
        )
        .map_err(|error| error.to_string())?;
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".into(),
                tool_call_id: tool_call_id.clone(),
                content: ChatContent::SimpleText(
                    serde_json::to_string(&page).map_err(|error| error.to_string())?,
                ),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_tool_description_keeps_the_read_surface_bounded_and_parallel() {
        let tool = ToolMetrics {
            config_path: String::new(),
        };
        let description = tool.tool_description();
        assert_eq!(description.name, "metrics");
        assert!(description.allow_parallel);
        assert_eq!(description.input_schema["additionalProperties"], false);
        assert_eq!(
            description.input_schema["properties"]["limit"]["minimum"],
            1
        );
        assert!(serde_json::from_value::<MetricsArgs>(json!({"unknown": true})).is_err());
    }
}
