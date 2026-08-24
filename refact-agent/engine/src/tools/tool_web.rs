use std::sync::Arc;
use std::collections::HashMap;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_web::execute_at_web_result;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::postprocessing::pp_command_output::OutputFilter;

pub struct ToolWeb {
    pub config_path: String,
}

const DEFAULT_OUTPUT_LIMIT: usize = 200;

fn parse_output_filter(args: &HashMap<String, Value>) -> OutputFilter {
    let output_filter = args
        .get("output_filter")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let output_limit = args
        .get("output_limit")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let is_unlimited =
        output_limit.eq_ignore_ascii_case("all") || output_limit.eq_ignore_ascii_case("full");

    let limit_lines = if is_unlimited {
        usize::MAX
    } else {
        output_limit
            .parse::<usize>()
            .unwrap_or(DEFAULT_OUTPUT_LIMIT)
    };

    OutputFilter {
        limit_lines,
        limit_chars: usize::MAX,
        valuable_top_or_bottom: "top".to_string(),
        grep: output_filter.to_string(),
        grep_context_lines: 3,
        remove_from_output: "".to_string(),
        limit_tokens: if is_unlimited {
            None
        } else {
            Some(limit_lines.saturating_mul(50))
        },
        skip: false,
    }
}

#[async_trait]
impl Tool for ToolWeb {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "web".to_string(),
            display_name: "Web".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Fetch a web page and convert to readable plain text. Supports regular web pages, PDFs, and JavaScript-rendered pages. Uses Jina Reader API with automatic fallback.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL of the web page to fetch."
                    },
                    "options": {
                        "type": "object",
                        "description": "Jina Reader API options passed as request headers.",
                        "properties": {
                            "respond_with": {
                                "type": "string",
                                "description": "Controls response format (x-respond-with header)."
                            },
                            "target_selector": {
                                "type": "string",
                                "description": "CSS selector to extract a specific element (x-target-selector header)."
                            },
                            "wait_for_selector": {
                                "type": "string",
                                "description": "CSS selector to wait for before returning content (x-wait-for-selector header)."
                            },
                            "timeout": {
                                "type": "number",
                                "description": "Request timeout in seconds (x-timeout header)."
                            },
                            "no_cache": {
                                "type": "boolean",
                                "description": "Bypass Jina cache when true (x-no-cache header)."
                            },
                            "cache_tolerance": {
                                "type": "number",
                                "description": "Cache staleness tolerance in seconds (x-cache-tolerance header)."
                            },
                            "with_generated_alt": {
                                "type": "boolean",
                                "description": "Include AI-generated alt text for images (x-with-generated-alt header)."
                            },
                            "streaming": {
                                "type": "boolean",
                                "description": "Stream the response as SSE (sets Accept: text/event-stream)."
                            },
                            "set_cookie": {
                                "type": "string",
                                "description": "Cookie string to send with the request (x-set-cookie header)."
                            },
                            "proxy_url": {
                                "type": "string",
                                "description": "Proxy URL to route the request through (x-proxy-url header)."
                            }
                        }
                    },
                    "output_filter": {
                        "type": "string",
                        "description": "Optional regex pattern to filter output lines. Only lines matching this pattern (and context) will be shown."
                    },
                    "output_limit": {
                        "type": "string",
                        "description": "Optional. Max lines to show (default: 200). Use higher values like '500' or 'all' to see more output."
                    }
                },
                "required": ["url"]
            }),
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
        let url = match args.get("url") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `url` is not a string: {:?}", v)),
            None => return Err("Missing argument `url`".to_string()),
        };

        let options: Option<HashMap<String, Value>> = match args.get("options") {
            Some(Value::Object(obj)) => {
                Some(obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            }
            Some(Value::Null) | None => None,
            Some(v) => return Err(format!("argument `options` is not an object: {:?}", v)),
        };

        let fetched = execute_at_web_result(&url, options.as_ref()).await?;
        let text = if fetched.text.is_empty() {
            "No content retrieved from the URL.".to_string()
        } else {
            fetched.text
        };
        let output_filter = parse_output_filter(args);
        let mut extra = serde_json::Map::new();
        extra.insert(
            "web_fetch".to_string(),
            serde_json::json!({
                "requested_url": fetched.requested_url,
                "final_url": fetched.final_url,
                "status": fetched.status,
                "content_type": fetched.content_type,
                "source": fetched.source,
            }),
        );

        let result = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            output_filter: Some(output_filter),
            extra,
            ..Default::default()
        })];

        Ok((false, result))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
