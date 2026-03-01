use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use rmcp::model::{RawContent, CallToolRequestParam, Tool as McpTool};
use tokio::sync::Mutex as AMutex;
use tokio::time::timeout;
use tokio::time::Duration;

use crate::caps::resolve_chat_model;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::scratchpads::multimodality::MultimodalElement;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::integrations::integr_abstract::{IntegrationCommon, IntegrationConfirmation};
use super::session_mcp::{McpRunningService, add_log_entry, mcp_session_wait_startup};

pub struct ToolMCP {
    pub common: IntegrationCommon,
    pub config_path: String,
    pub mcp_client: Arc<AMutex<Option<McpRunningService>>>,
    pub mcp_tool: McpTool,
    pub request_timeout: u64,
}

#[async_trait]
impl Tool for ToolMCP {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, serde_json::Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let session_key = format!("{}", self.config_path);
        let (gcx, current_model) = {
            let ccx_locked = ccx.lock().await;
            (
                ccx_locked.global_context.clone(),
                ccx_locked.current_model.clone(),
            )
        };
        let (session_maybe, caps_maybe) = {
            let gcx_locked = gcx.read().await;
            (
                gcx_locked.integration_sessions.get(&session_key).cloned(),
                gcx_locked.caps.clone(),
            )
        };
        if session_maybe.is_none() {
            tracing::error!("No session for {:?}, strange (2)", session_key);
            return Err(format!("No session for {:?}", session_key));
        }
        let session = session_maybe.unwrap();
        let model_supports_multimodality = caps_maybe.is_some_and(|caps| {
            resolve_chat_model(caps, &current_model).is_ok_and(|m| m.supports_multimodality)
        });
        mcp_session_wait_startup(session.clone()).await;

        let json_args = serde_json::json!(args);
        tracing::info!(
            "\n\nMCP CALL tool '{}' with arguments: {:?}",
            self.mcp_tool.name,
            json_args
        );

        let session_logs = {
            let mut session_locked = session.lock().await;
            let session_downcasted = session_locked
                .as_any_mut()
                .downcast_mut::<super::session_mcp::SessionMCP>()
                .unwrap();
            session_downcasted.logs.clone()
        };

        add_log_entry(
            session_logs.clone(),
            format!(
                "Executing tool '{}' with arguments: {:?}",
                self.mcp_tool.name, json_args
            ),
        )
        .await;

        let result_probably = {
            let mcp_client_locked = self.mcp_client.lock().await;
            if let Some(client) = &*mcp_client_locked {
                match timeout(
                    Duration::from_secs(self.request_timeout),
                    client.call_tool(CallToolRequestParam {
                        name: self.mcp_tool.name.clone(),
                        arguments: match json_args {
                            serde_json::Value::Object(map) => Some(map),
                            _ => None,
                        },
                    }),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(rmcp::service::ServiceError::Timeout {
                        timeout: Duration::from_secs(self.request_timeout),
                    }),
                }
            } else {
                return Err("MCP client is not available".to_string());
            }
        };

        let result_message = match result_probably {
            Ok(result) => {
                if result.is_error.unwrap_or(false) {
                    let error_msg = format!("Tool execution error: {:?}", result.content);
                    add_log_entry(session_logs.clone(), error_msg.clone()).await;
                    return Err(error_msg);
                }

                let mut elements = Vec::new();
                for content in result.content {
                    match content.raw {
                        RawContent::Text(text_content) => elements.push(MultimodalElement {
                            m_type: "text".to_string(),
                            m_content: text_content.text,
                        }),
                        RawContent::Image(image_content) => {
                            if model_supports_multimodality {
                                let mime_type = if image_content.mime_type.starts_with("image/") {
                                    image_content.mime_type
                                } else {
                                    format!("image/{}", image_content.mime_type)
                                };
                                elements.push(MultimodalElement {
                                    m_type: mime_type,
                                    m_content: image_content.data,
                                })
                            } else {
                                elements.push(MultimodalElement {
                                    m_type: "text".to_string(),
                                    m_content: "Server returned an image, but model does not support multimodality".to_string(),
                                })
                            }
                        }
                        RawContent::Audio(audio_content) => elements.push(MultimodalElement {
                            m_type: "text".to_string(),
                            m_content: format!(
                                "[Audio content: {}, {} bytes - audio playback not supported]",
                                audio_content.raw.mime_type,
                                audio_content.raw.data.len(),
                            ),
                        }),
                        RawContent::Resource(embedded) => {
                            let text = match &embedded.resource {
                                rmcp::model::ResourceContents::TextResourceContents { uri, mime_type, text } => {
                                    format!(
                                        "[Resource: {} ({}) - {}]\n{}",
                                        uri,
                                        mime_type.as_deref().unwrap_or("unknown"),
                                        uri,
                                        text,
                                    )
                                }
                                rmcp::model::ResourceContents::BlobResourceContents { uri, mime_type, blob } => {
                                    format!(
                                        "[Resource: {} ({}) - {} bytes blob]",
                                        uri,
                                        mime_type.as_deref().unwrap_or("unknown"),
                                        blob.len(),
                                    )
                                }
                            };
                            elements.push(MultimodalElement {
                                m_type: "text".to_string(),
                                m_content: text,
                            });
                        }
                    }
                }

                let content = if elements.iter().all(|el| el.m_type == "text") {
                    ChatContent::SimpleText(
                        elements
                            .into_iter()
                            .map(|el| el.m_content)
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                    )
                } else {
                    ChatContent::Multimodal(elements)
                };

                ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content,
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })
            }
            Err(e) => {
                let error_msg = format!("Failed to call tool: {:?}", e);
                tracing::error!("{}", error_msg);
                add_log_entry(session_logs.clone(), error_msg).await;
                return Err(e.to_string());
            }
        };

        Ok((false, vec![result_message]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn tool_description(&self) -> ToolDesc {
        let input_schema = {
            let mut map = self.mcp_tool.input_schema.as_ref().clone();
            if !map.contains_key("type") {
                map.insert("type".to_string(), serde_json::json!("object"));
            }
            serde_json::Value::Object(map)
        };

        let tool_name = {
            let yaml_name = std::path::Path::new(&self.config_path)
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown");
            let shortened_yaml_name = if let Some(stripped) = yaml_name.strip_prefix("mcp_stdio_") {
                format!("mcp_{}", stripped)
            } else if let Some(stripped) = yaml_name.strip_prefix("mcp_sse_") {
                format!("mcp_{}", stripped)
            } else {
                yaml_name.to_string()
            };
            format!("{}_{}", shortened_yaml_name, self.mcp_tool.name)
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        };

        let annotations = self.mcp_tool.annotations.as_ref()
            .and_then(|a| serde_json::to_value(a).ok());

        ToolDesc {
            name: tool_name,
            display_name: self.mcp_tool.name.to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Integration,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: self.mcp_tool.description.to_owned().unwrap_or_default().to_string(),
            input_schema,
            output_schema: None,
            annotations,
        }
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &HashMap<String, serde_json::Value>,
    ) -> Result<String, String> {
        let command = self.mcp_tool.name.clone();
        tracing::info!(
            "MCP command_to_match_against_confirm_deny() returns {:?}",
            command
        );
        Ok(command.to_string())
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(self.common.confirmation.clone())
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_tool_mcp(schema: serde_json::Value) -> ToolMCP {
        let mcp_tool: McpTool = serde_json::from_value(json!({
            "name": "test_tool",
            "description": "A test tool",
            "inputSchema": schema
        })).expect("failed to deserialize McpTool");
        ToolMCP {
            common: crate::integrations::integr_abstract::IntegrationCommon::default(),
            config_path: "mcp_stdio_server.yaml".to_string(),
            mcp_client: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            mcp_tool,
            request_timeout: 30,
        }
    }

    fn make_tool_mcp_with_annotations(schema: serde_json::Value, annotations: serde_json::Value) -> ToolMCP {
        let mcp_tool: McpTool = serde_json::from_value(json!({
            "name": "test_tool",
            "description": "A test tool",
            "inputSchema": schema,
            "annotations": annotations
        })).expect("failed to deserialize McpTool");
        ToolMCP {
            common: crate::integrations::integr_abstract::IntegrationCommon::default(),
            config_path: "mcp_stdio_server.yaml".to_string(),
            mcp_client: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            mcp_tool,
            request_timeout: 30,
        }
    }

    #[test]
    fn test_complex_mcp_schema_preserved() {
        let complex_schema = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of items"
                },
                "config": {
                    "type": "object",
                    "properties": {
                        "verbose": {"type": "boolean"},
                        "max_count": {"type": "integer"}
                    }
                },
                "mode": {
                    "type": "string",
                    "enum": ["fast", "slow", "medium"]
                }
            },
            "required": ["items"]
        });

        let tool = make_tool_mcp(complex_schema.clone());
        let desc = tool.tool_description();

        assert_eq!(desc.input_schema["type"], json!("object"));
        assert_eq!(desc.input_schema["properties"]["items"]["type"], json!("array"));
        assert_eq!(desc.input_schema["properties"]["items"]["items"]["type"], json!("string"));
        assert_eq!(desc.input_schema["properties"]["config"]["type"], json!("object"));
        assert_eq!(desc.input_schema["properties"]["mode"]["enum"], json!(["fast", "slow", "medium"]));
        assert_eq!(desc.input_schema["required"], json!(["items"]));
        assert_eq!(desc.name, "mcp_server_test_tool");
    }

    #[test]
    fn test_mcp_schema_without_type_gets_object_type() {
        let schema_without_type = json!({
            "properties": {
                "a": {"type": "integer"},
                "b": {"type": "integer"}
            },
            "required": ["a", "b"]
        });

        let tool = make_tool_mcp(schema_without_type);
        let desc = tool.tool_description();

        assert_eq!(desc.input_schema["type"], json!("object"));
        assert_eq!(desc.input_schema["properties"]["a"]["type"], json!("integer"));
    }

    #[test]
    fn test_annotations_preserved() {
        let schema = json!({"type": "object", "properties": {}});
        let annotations = json!({
            "title": "My Tool",
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        });
        let tool = make_tool_mcp_with_annotations(schema, annotations);
        let desc = tool.tool_description();
        let ann = desc.annotations.expect("annotations should be present");
        assert_eq!(ann["title"], json!("My Tool"));
        assert_eq!(ann["readOnlyHint"], json!(true));
        assert_eq!(ann["destructiveHint"], json!(false));
        assert_eq!(ann["idempotentHint"], json!(true));
        assert_eq!(ann["openWorldHint"], json!(false));
    }

    #[test]
    fn test_no_annotations_is_none() {
        let schema = json!({"type": "object", "properties": {}});
        let tool = make_tool_mcp(schema);
        let desc = tool.tool_description();
        assert!(desc.annotations.is_none());
    }

    #[test]
    fn test_audio_content_produces_metadata_text() {
        use rmcp::model::{RawContent, RawAudioContent, Annotated};
        let audio = RawContent::Audio(Annotated {
            raw: RawAudioContent {
                data: "AAABBBCCC".to_string(),
                mime_type: "audio/mp3".to_string(),
            },
            annotations: None,
        });
        let text = match audio {
            RawContent::Audio(audio_content) => format!(
                "[Audio content: {}, {} bytes - audio playback not supported]",
                audio_content.raw.mime_type,
                audio_content.raw.data.len(),
            ),
            _ => panic!("expected audio"),
        };
        assert!(text.contains("audio/mp3"));
        assert!(text.contains("9 bytes"));
        assert!(text.contains("audio playback not supported"));
    }

    #[test]
    fn test_resource_text_content_includes_uri_and_text() {
        use rmcp::model::{RawContent, RawEmbeddedResource, ResourceContents};
        let resource = RawContent::Resource(RawEmbeddedResource {
            resource: ResourceContents::TextResourceContents {
                uri: "file:///path/to/file.txt".to_string(),
                mime_type: Some("text/plain".to_string()),
                text: "Hello from resource".to_string(),
            },
        });
        let text = match resource {
            RawContent::Resource(embedded) => match &embedded.resource {
                ResourceContents::TextResourceContents { uri, mime_type, text } => {
                    format!(
                        "[Resource: {} ({}) - {}]\n{}",
                        uri,
                        mime_type.as_deref().unwrap_or("unknown"),
                        uri,
                        text,
                    )
                }
                _ => panic!("expected text resource"),
            },
            _ => panic!("expected resource"),
        };
        assert!(text.contains("file:///path/to/file.txt"));
        assert!(text.contains("text/plain"));
        assert!(text.contains("Hello from resource"));
    }

    #[test]
    fn test_resource_blob_content_includes_uri_and_size() {
        use rmcp::model::{RawContent, RawEmbeddedResource, ResourceContents};
        let resource = RawContent::Resource(RawEmbeddedResource {
            resource: ResourceContents::BlobResourceContents {
                uri: "file:///path/to/data.bin".to_string(),
                mime_type: Some("application/octet-stream".to_string()),
                blob: "AABBCCDD".to_string(),
            },
        });
        let text = match resource {
            RawContent::Resource(embedded) => match &embedded.resource {
                ResourceContents::BlobResourceContents { uri, mime_type, blob } => {
                    format!(
                        "[Resource: {} ({}) - {} bytes blob]",
                        uri,
                        mime_type.as_deref().unwrap_or("unknown"),
                        blob.len(),
                    )
                }
                _ => panic!("expected blob resource"),
            },
            _ => panic!("expected resource"),
        };
        assert!(text.contains("file:///path/to/data.bin"));
        assert!(text.contains("application/octet-stream"));
        assert!(text.contains("8 bytes blob"));
    }
}
