use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolBuddyOpenIssue {
    pub config_path: String,
}

impl ToolBuddyOpenIssue {
    fn runner(&self) -> crate::tools::tool_buddy_create_issue::ToolBuddyCreateIssue {
        crate::tools::tool_buddy_create_issue::ToolBuddyCreateIssue {
            config_path: self.config_path.clone(),
        }
    }
}

#[async_trait]
impl Tool for ToolBuddyOpenIssue {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_open_issue".to_string(),
            display_name: "Buddy Open Issue".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Alias for buddy_create_issue that files a confirmed issue through the same Buddy issue runner.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "body": {"type": "string"},
                    "labels": {"type": "array", "items": {"type": "string"}},
                    "provider": {"type": "string"}
                },
                "required": ["title", "body"],
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
        let mut forwarded = args.clone();
        forwarded
            .entry("confidence".to_string())
            .or_insert_with(|| json!("confirmed"));
        let mut runner = self.runner();
        runner.tool_execute(ccx, tool_call_id, &forwarded).await
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tools_description::Tool;

    #[test]
    fn buddy_open_issue_calls_same_runner_as_buddy_create_issue() {
        let tool = ToolBuddyOpenIssue {
            config_path: "config.yaml".to_string(),
        };
        let runner = tool.runner();
        assert_eq!(runner.tool_description().name, "buddy_create_issue");
        assert_eq!(tool.tool_description().name, "buddy_open_issue");
        assert_eq!(
            runner.tool_description().source.config_path,
            tool.tool_description().source.config_path
        );
    }
}
