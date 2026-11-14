use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use async_trait::async_trait;
use refact_tool_api::coerce_args_to_schema;
use refact_privacy::{records_from_messages, FileRecord, PrivacyAuditError, PrivacyAudited};
use sha2::{Digest, Sha256};

/// Maximum bytes of text content returned from a single MCP tool call.
/// Prevents runaway context window growth from excessively large tool responses.
const MAX_TOOL_OUTPUT_BYTES: usize = 200 * 1024; // 200 KB
use rmcp::model::{RawContent, CallToolRequestParams, Tool as McpTool};
use tokio::sync::Mutex as AMutex;
use tokio::time::timeout;
use tokio::time::Duration;

use crate::caps::resolve_chat_model;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::scratchpads::multimodality::MultimodalElement;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::integrations::integr_abstract::{IntegrationCommon, IntegrationConfirmation};
use crate::privacy::destinations::clear_for_mcp;
use super::session_mcp::{
    McpRunningService, MCPConnectionStatus, add_log_entry, mcp_session_wait_startup_bounded,
    redact_sensitive_json,
};

const STARTUP_WAIT_MARGIN_SECS: u64 = 5;

fn not_connected_reason(config_path: &str, status: &MCPConnectionStatus) -> Option<String> {
    let server = super::mcp_interactions::server_name_from_config_path(config_path);
    let reason = match status {
        MCPConnectionStatus::Connected => return None,
        MCPConnectionStatus::Reconnecting {
            ..
        } => match super::integr_mcp_common::mcp_next_retry_in_secs(config_path) {
            Some(next) => format!("reconnecting, next try in {next}s"),
            None => "reconnecting".to_string(),
        },
        MCPConnectionStatus::Connecting => "still connecting".to_string(),
        MCPConnectionStatus::Failed {
            message,
        } => format!("connection failed: {message}"),
        MCPConnectionStatus::Disconnected => "disconnected".to_string(),
        MCPConnectionStatus::NeedsAuth => "authentication required".to_string(),
    };
    Some(format!(
        "MCP server '{server}' is not connected ({reason}). Try again later or check the MCP integration status."
    ))
}

struct McpCallAudit {
    records: Result<Vec<(usize, FileRecord)>, PrivacyAuditError>,
}

impl PrivacyAudited for McpCallAudit {
    fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
        self.records.clone()
    }
}

fn collect_serialized_argument_paths(value: &serde_json::Value, paths: &mut Vec<PathBuf>) {
    match value {
        serde_json::Value::String(value) if !value.is_empty() => {
            let path = PathBuf::from(value);
            if path.exists() {
                paths.push(path);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_serialized_argument_paths(value, paths);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                collect_serialized_argument_paths(value, paths);
            }
        }
        _ => {}
    }
}

fn call_param_records(
    gcx: &Arc<crate::global_context::GlobalContext>,
    call_params: &CallToolRequestParams,
) -> Result<Vec<FileRecord>, String> {
    let serialized = serde_json::to_value(call_params).map_err(|error| error.to_string())?;
    let mut paths = Vec::new();
    if let Some(arguments) = serialized.get("arguments") {
        collect_serialized_argument_paths(arguments, &mut paths);
    }
    crate::privacy::records::declared_file_records(gcx, paths)
}

fn dispatch_cleared_mcp_call<T>(
    gcx: &Arc<crate::global_context::GlobalContext>,
    server_name: &str,
    call_params: CallToolRequestParams,
    producing_messages: &[ChatMessage],
    dispatch: impl FnOnce(CallToolRequestParams) -> T,
) -> Result<T, String> {
    let mut records = records_from_messages(producing_messages);
    if let Ok(indexed_records) = &mut records {
        for record in call_param_records(gcx, &call_params)? {
            if !indexed_records
                .iter()
                .any(|(_, existing)| existing == &record)
            {
                indexed_records.push((producing_messages.len(), record));
            }
        }
    }
    clear_for_mcp(
        gcx,
        McpCallAudit {
            records,
        },
        server_name,
    )
    .map_err(|refusal| refusal.model_facing().to_string())?;
    Ok(dispatch(call_params))
}

/// Truncates `text` so that the running `total_bytes` counter does not exceed `limit`.
/// Appends a truncation notice when cutting. Returns the (possibly truncated) text.
fn truncate_to_byte_limit(text: String, limit: usize, total_bytes: &mut usize) -> String {
    if *total_bytes >= limit {
        return String::new();
    }
    let remaining = limit - *total_bytes;
    if text.len() <= remaining {
        *total_bytes += text.len();
        text
    } else {
        *total_bytes = limit;
        // Truncate on a UTF-8 char boundary
        let boundary = text
            .char_indices()
            .take_while(|(i, _)| *i < remaining.saturating_sub(64))
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        format!(
            "{}\n...(truncated, {} bytes omitted)",
            &text[..boundary],
            text.len() - boundary
        )
    }
}

fn mcp_result_type(content: &ChatContent) -> &'static str {
    match content {
        ChatContent::SimpleText(_) => "text",
        ChatContent::Multimodal(elements)
            if elements.iter().any(|element| element.m_type != "text") =>
        {
            "multimodal"
        }
        ChatContent::Multimodal(_) => "text",
        ChatContent::ContextFiles(_) => "context_files",
    }
}

fn mcp_metadata(tool: &ToolMCP, content: &ChatContent) -> Option<serde_json::Value> {
    let schema = tool.mcp_tool.input_schema.as_ref();
    let schema_hash = hex::encode(Sha256::digest(
        serde_json::to_vec(schema).unwrap_or_default(),
    ));
    let config_path = PathBuf::from(&tool.config_path);
    let server = config_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string();
    if !mcp_metadata_name_is_safe(&server) || !mcp_metadata_name_is_safe(&tool.mcp_tool.name) {
        return None;
    }
    Some(serde_json::json!({
        "server": server,
        "tool": tool.mcp_tool.name.to_string(),
        "status": "success",
        "schema_hash": &schema_hash[..16],
        "result_type": mcp_result_type(content),
    }))
}

fn mcp_metadata_name_is_safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

const MAX_MCP_FAILURE_BYTES: usize = 1024;

fn mcp_failure_name(value: &str) -> &str {
    if mcp_metadata_name_is_safe(value)
        && refact_core::string_utils::redact_sensitive(value) == value
    {
        value
    } else {
        "[REDACTED]"
    }
}

fn mcp_call_failure_receipt(
    config_path: &str,
    tool_name: &str,
    error: &rmcp::service::ServiceError,
) -> String {
    let server = super::mcp_interactions::server_name_from_config_path(config_path);
    let server = mcp_failure_name(&server);
    let tool = mcp_failure_name(tool_name);
    let cause = match error {
        rmcp::service::ServiceError::TransportClosed => "Transport closed".to_string(),
        rmcp::service::ServiceError::TransportSend(_) => "Transport send failed".to_string(),
        rmcp::service::ServiceError::McpError(_) => "MCP protocol error".to_string(),
        rmcp::service::ServiceError::UnexpectedResponse => "Unexpected MCP response".to_string(),
        rmcp::service::ServiceError::Cancelled {
            ..
        } => "MCP request cancelled".to_string(),
        rmcp::service::ServiceError::Timeout {
            timeout,
        } => {
            format!("Request timed out after {}s", timeout.as_secs())
        }
        _ => "Unclassified MCP service failure".to_string(),
    };
    let receipt = format!(
        "MCP call failed (server '{server}', tool '{tool}'): {cause}. \
         Opaque error details withheld. External outcome is unknown; this is not a confirmed success. \
         No automatic replay or reconnect was attempted by this call. Check the MCP integration status and \
         reconnect it if needed. Verify external effects before explicitly deciding whether to retry."
    );
    refact_core::string_utils::safe_truncate(&receipt, MAX_MCP_FAILURE_BYTES).to_string()
}

pub struct ToolMCP {
    /// Model-facing tool name, precomputed with collision disambiguation in
    /// `mcp_integr_tools` (see `mcp_naming::disambiguate_model_tool_names`).
    pub model_name: String,
    pub common: IntegrationCommon,
    pub config_path: String,
    pub mcp_client: Arc<AMutex<Option<McpRunningService>>>,
    pub mcp_tool: McpTool,
    pub request_timeout: u64,
    pub init_timeout: u64,
    pub auto_approve: bool,
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
        let (gcx, current_model, producing_messages) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.global_context.clone(),
                cgcx.current_model.clone(),
                cgcx.messages.clone(),
            )
        };
        // Fail closed (no replay) before any peer send if a managed LiteLLM
        // binding is stale, disabled or its credential identity changed.
        super::litellm_transport::check_dispatch(&gcx, &self.config_path).await?;
        let (session_maybe, caps_maybe) = {
            let integration_sessions = gcx.integration_sessions.clone();
            let caps_state = gcx.caps_state.clone();
            let caps = caps_state.read().await.caps.clone();
            let integration_sessions = integration_sessions.lock().await;
            (integration_sessions.get(&session_key).cloned(), caps)
        };
        if session_maybe.is_none() {
            let msg = format!(
                "No session for {:?}, MCP server may not be running",
                session_key
            );
            tracing::error!("{}", msg);
            crate::buddy::actor::report_error_persisted(
                crate::app_state::AppState::from_gcx(gcx.clone()).await,
                "mcp_no_session",
                &msg,
                Some("mcp/tool_mcp.rs"),
                None,
            )
            .await;
            return Err(format!("No session for {:?}", session_key));
        }
        let session = session_maybe.unwrap();
        let model_supports_multimodality = caps_maybe.is_some_and(|caps| {
            resolve_chat_model(caps, &current_model).is_ok_and(|m| m.supports_multimodality)
        });
        let still_connecting = {
            let mut session_locked = session.lock().await;
            session_locked
                .as_any_mut()
                .downcast_mut::<super::session_mcp::SessionMCP>()
                .is_some_and(|s| s.connection_status == MCPConnectionStatus::Connecting)
        };
        if still_connecting {
            let startup_wait = Duration::from_secs(self.init_timeout + STARTUP_WAIT_MARGIN_SECS);
            mcp_session_wait_startup_bounded(session.clone(), startup_wait).await;
        }

        {
            let mut session_locked = session.lock().await;
            let session_downcasted = session_locked
                .as_any_mut()
                .downcast_mut::<super::session_mcp::SessionMCP>()
                .ok_or_else(|| {
                    format!(
                        "Internal error: session is not an MCP session for '{}'",
                        self.mcp_tool.name
                    )
                })?;
            let status = session_downcasted.connection_status.clone();
            drop(session_locked);
            if let Some(msg) = not_connected_reason(&self.config_path, &status) {
                let kind = match status {
                    MCPConnectionStatus::Reconnecting {
                        ..
                    } => "mcp_reconnecting",
                    MCPConnectionStatus::Failed {
                        ..
                    } => "mcp_connection_failed",
                    _ => "mcp_not_connected",
                };
                crate::buddy::actor::report_error_persisted(
                    crate::app_state::AppState::from_gcx(gcx.clone()).await,
                    kind,
                    &msg,
                    Some("mcp/tool_mcp.rs"),
                    None,
                )
                .await;
                return Err(msg);
            }
        }

        let json_args = serde_json::json!(args);
        let redacted_args = redact_sensitive_json(&json_args);
        tracing::info!(
            "\n\nMCP CALL tool '{}' with arguments: {:?}",
            self.mcp_tool.name,
            redacted_args
        );

        let (session_logs, session_metrics, observation) = {
            let mut session_locked = session.lock().await;
            let session_downcasted = session_locked
                .as_any_mut()
                .downcast_mut::<super::session_mcp::SessionMCP>()
                .ok_or_else(|| {
                    format!(
                        "Internal error: session is not an MCP session for '{}'",
                        self.mcp_tool.name
                    )
                })?;
            (
                session_downcasted.logs.clone(),
                session_downcasted.metrics.clone(),
                #[cfg(target_os = "linux")]
                session_downcasted.observation.clone(),
                #[cfg(not(target_os = "linux"))]
                None::<refact_exec::ObservationReader>,
            )
        };

        add_log_entry(
            session_logs.clone(),
            format!(
                "Executing tool '{}' with arguments: {:?}",
                self.mcp_tool.name, redacted_args
            ),
        )
        .await;

        let call_params = {
            let mut p = CallToolRequestParams::new(self.mcp_tool.name.clone());
            if let serde_json::Value::Object(mut map) = json_args {
                let input_schema =
                    serde_json::Value::Object(self.mcp_tool.input_schema.as_ref().clone());
                coerce_args_to_schema(&mut map, &input_schema);
                p = p.with_arguments(map);
            }
            p
        };
        let server_name = super::mcp_interactions::server_name_from_config_path(&self.config_path);
        let call_params = dispatch_cleared_mcp_call(
            &gcx,
            &server_name,
            call_params,
            &producing_messages,
            |call_params| call_params,
        )?;
        let peer = {
            let mcp_client_locked = self.mcp_client.lock().await;
            match &*mcp_client_locked {
                Some(client) => client.peer().clone(),
                None => {
                    let msg = format!("MCP client for '{}' is not available", self.mcp_tool.name);
                    crate::buddy::actor::report_error_persisted(
                        crate::app_state::AppState::from_gcx(gcx.clone()).await,
                        "mcp_client_unavailable",
                        &msg,
                        Some("mcp/tool_mcp.rs"),
                        None,
                    )
                    .await;
                    return Err(msg);
                }
            }
        };

        let call_start = session_metrics.lock().await.record_call_start();
        let observed_reads_before = observation
            .as_ref()
            .and_then(observed_reads)
            .unwrap_or_default();
        let result_probably = match timeout(
            Duration::from_secs(self.request_timeout),
            peer.call_tool(call_params),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(rmcp::service::ServiceError::Timeout {
                timeout: Duration::from_secs(self.request_timeout),
            }),
        };

        let result_message = match result_probably {
            Ok(result) => {
                if result.is_error.unwrap_or(false) {
                    let error_msg = format!("Tool execution error: {:?}", result.content);
                    add_log_entry(session_logs.clone(), error_msg.clone()).await;
                    {
                        let mut m = session_metrics.lock().await;
                        m.record_call_failure(&self.mcp_tool.name, call_start);
                    }
                    {
                        crate::buddy::actor::report_error_persisted(
                            crate::app_state::AppState::from_gcx(gcx.clone()).await,
                            "mcp_tool_error",
                            &error_msg,
                            Some("mcp/tool_mcp.rs"),
                            None,
                        )
                        .await;
                    }
                    return Err(error_msg);
                }

                let mut elements = Vec::new();
                let mut total_text_bytes: usize = 0;
                for content in result.content {
                    match content.raw {
                        RawContent::Text(text_content) => {
                            let text = truncate_to_byte_limit(
                                text_content.text,
                                MAX_TOOL_OUTPUT_BYTES,
                                &mut total_text_bytes,
                            );
                            elements.push(MultimodalElement {
                                m_type: "text".to_string(),
                                m_content: text,
                            });
                        }
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
                                audio_content.mime_type,
                                audio_content.data.len(),
                            ),
                        }),
                        RawContent::Resource(embedded) => {
                            let raw_text = match &embedded.resource {
                                rmcp::model::ResourceContents::TextResourceContents {
                                    uri,
                                    mime_type,
                                    text,
                                    ..
                                } => {
                                    format!(
                                        "[Resource: {} ({}) - {}]\n{}",
                                        uri,
                                        mime_type.as_deref().unwrap_or("unknown"),
                                        uri,
                                        text,
                                    )
                                }
                                rmcp::model::ResourceContents::BlobResourceContents {
                                    uri,
                                    mime_type,
                                    blob,
                                    ..
                                } => {
                                    format!(
                                        "[Resource: {} ({}) - {} bytes blob]",
                                        uri,
                                        mime_type.as_deref().unwrap_or("unknown"),
                                        blob.len(),
                                    )
                                }
                            };
                            let text = truncate_to_byte_limit(
                                raw_text,
                                MAX_TOOL_OUTPUT_BYTES,
                                &mut total_text_bytes,
                            );
                            elements.push(MultimodalElement {
                                m_type: "text".to_string(),
                                m_content: text,
                            });
                        }
                        RawContent::ResourceLink(resource) => {
                            elements.push(MultimodalElement {
                                m_type: "text".to_string(),
                                m_content: format!("[Resource link: {}]", resource.uri),
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

                {
                    let mut m = session_metrics.lock().await;
                    m.record_call_success(&self.mcp_tool.name, call_start);
                }
                let mut message = ChatMessage {
                    role: "tool".to_string(),
                    content,
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                };
                if let Some(metadata) = mcp_metadata(self, &message.content) {
                    message.extra.insert("mcp".to_string(), metadata);
                }
                attach_mcp_observation(
                    &gcx,
                    observation.as_ref(),
                    &observed_reads_before,
                    &mut message,
                );
                ContextEnum::ChatMessage(message)
            }
            Err(e) => {
                let error_msg =
                    mcp_call_failure_receipt(&self.config_path, &self.mcp_tool.name, &e);
                tracing::error!("{}", error_msg);
                add_log_entry(session_logs.clone(), error_msg.clone()).await;
                {
                    let mut m = session_metrics.lock().await;
                    m.record_call_failure(&self.mcp_tool.name, call_start);
                }
                {
                    crate::buddy::actor::report_error_persisted(
                        crate::app_state::AppState::from_gcx(gcx.clone()).await,
                        "mcp_tool_error",
                        &error_msg,
                        Some("mcp/tool_mcp.rs"),
                        None,
                    )
                    .await;
                }
                return Err(error_msg);
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

        let tool_name = self.model_name.clone();

        let annotations = self
            .mcp_tool
            .annotations
            .as_ref()
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
            description: self
                .mcp_tool
                .description
                .to_owned()
                .unwrap_or_default()
                .to_string(),
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
        let mut confirmation = self.common.confirmation.clone();
        if self.auto_approve {
            let tool_name = self.mcp_tool.name.to_string();
            confirmation
                .ask_user
                .retain(|p| p != &tool_name && p != "*");
        }
        Some(confirmation)
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

fn attach_mcp_observation(
    gcx: &Arc<crate::global_context::GlobalContext>,
    observation: Option<&refact_exec::ObservationReader>,
    observed_reads_before: &std::collections::HashSet<std::path::PathBuf>,
    message: &mut ChatMessage,
) {
    let Some(observation) = observation else {
        return;
    };
    attach_mcp_observation_status(gcx, observation.status(), observed_reads_before, message);
}

fn attach_mcp_observation_status(
    gcx: &Arc<crate::global_context::GlobalContext>,
    status: refact_exec::ObservationStatus,
    observed_reads_before: &std::collections::HashSet<std::path::PathBuf>,
    message: &mut ChatMessage,
) {
    crate::privacy::record_observation_status(gcx, &status);
    match status {
        refact_exec::ObservationStatus::Observed(access)
        | refact_exec::ObservationStatus::Pending(access)
        | refact_exec::ObservationStatus::Incomplete(access) => {
            let paths = access
                .reads
                .into_iter()
                .filter(|path| !observed_reads_before.contains(path));
            match crate::privacy::records::observed_file_records(gcx, paths) {
                Ok(records) => crate::privacy::records::merge_records(message, records),
                Err(reason) => attach_observation_unavailable(message, reason),
            }
        }
        refact_exec::ObservationStatus::Unavailable(reason) => {
            attach_observation_unavailable(message, reason)
        }
    }
}

fn observed_reads(
    observation: &refact_exec::ObservationReader,
) -> Option<std::collections::HashSet<std::path::PathBuf>> {
    match observation.status() {
        refact_exec::ObservationStatus::Observed(access)
        | refact_exec::ObservationStatus::Pending(access)
        | refact_exec::ObservationStatus::Incomplete(access) => {
            Some(access.reads.into_iter().collect())
        }
        refact_exec::ObservationStatus::Unavailable(_) => None,
    }
}

fn attach_observation_unavailable(message: &mut ChatMessage, reason: String) {
    message.extra.insert(
        "privacy_observation".to_string(),
        serde_json::json!({"status": "unavailable", "reason": reason}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_privacy::{Attribution, PrivacyPolicy, PrivacyRecord, Zone};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn make_tool_mcp(schema: serde_json::Value) -> ToolMCP {
        let mcp_tool: McpTool = serde_json::from_value(json!({
            "name": "test_tool",
            "description": "A test tool",
            "inputSchema": schema
        }))
        .expect("failed to deserialize McpTool");
        ToolMCP {
            model_name: "mcp_server_test_tool".to_string(),
            common: crate::integrations::integr_abstract::IntegrationCommon::default(),
            config_path: "mcp_stdio_server.yaml".to_string(),
            mcp_client: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            mcp_tool,
            request_timeout: 30,
            init_timeout: 1,
            auto_approve: false,
        }
    }

    #[test]
    fn mcp_failure_receipt_withholds_opaque_details_and_unsafe_identity() {
        use rmcp::service::ServiceError;

        let private = "https://private.example.test/path?token=abc123 MY_ENV=hidden /opt/private/config server-stderr";
        let errors = [
            (ServiceError::TransportClosed, "Transport closed"),
            (
                ServiceError::TransportSend(rmcp::transport::DynamicTransportError::from_parts(
                    private.to_string(),
                    std::any::TypeId::of::<std::io::Error>(),
                    Box::new(std::io::Error::new(std::io::ErrorKind::BrokenPipe, private)),
                )),
                "Transport send failed",
            ),
            (ServiceError::UnexpectedResponse, "Unexpected MCP response"),
            (
                ServiceError::Timeout {
                    timeout: Duration::from_secs(u64::MAX),
                },
                "Request timed out",
            ),
            (
                ServiceError::Cancelled {
                    reason: Some(private.repeat(10_000)),
                },
                "MCP request cancelled",
            ),
            (
                ServiceError::McpError(rmcp::ErrorData::internal_error(
                    private.repeat(10_000),
                    Some(json!({"env": private, "stderr": private})),
                )),
                "MCP protocol error",
            ),
        ];
        for (error, cause) in errors {
            let receipt = mcp_call_failure_receipt(
                "/opt/private/mcp_stdio_sk-serversecret123.yaml",
                "ghp_toolsecret1234567890",
                &error,
            );
            assert!(receipt.contains("server '[REDACTED]', tool '[REDACTED]'"));
            assert!(receipt.contains(cause));
            assert!(receipt.contains("Opaque error details withheld"));
            assert!(receipt.contains("External outcome is unknown"));
            assert!(receipt.len() <= MAX_MCP_FAILURE_BYTES);
            for secret in [
                "private",
                "abc123",
                "MY_ENV",
                "hidden",
                "server-stderr",
                "serversecret",
                "toolsecret",
            ] {
                assert!(!receipt.contains(secret), "leaked {secret}: {receipt}");
            }
        }
        for name in [
            "token=hidden",
            "https://private.test",
            "/opt/private",
            "bad\nname",
        ] {
            assert_eq!(mcp_failure_name(name), "[REDACTED]");
        }
        assert_eq!(mcp_failure_name(&"a".repeat(129)), "[REDACTED]");
        let receipt = mcp_call_failure_receipt(
            &format!("/opt/private/mcp_stdio_{}.yaml", "a".repeat(128)),
            &"b".repeat(128),
            &ServiceError::Timeout {
                timeout: Duration::from_secs(u64::MAX),
            },
        );
        assert!(receipt.len() <= MAX_MCP_FAILURE_BYTES);
        assert!(receipt.ends_with("whether to retry."));
    }

    #[tokio::test]
    async fn mcp_wrapper_closed_peer_and_secret_error_receipts() {
        use super::super::session_mcp::{McpClientHandler, SessionMCP};
        use crate::integrations::sessions::IntegrationSession;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        for (fail_closed, expected_cause) in
            [(true, "Transport closed"), (false, "MCP protocol error")]
        {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            set_policy(&gcx, Vec::new());
            let mut tool = make_tool_mcp(json!({"type": "object"}));
            tool.config_path = "/opt/private/mcp_stdio_local_fixture.yaml".into();
            tool.request_timeout = 2;
            let mut session = SessionMCP::detached(&tool.config_path);
            session.connection_status = MCPConnectionStatus::Connected;
            session.mcp_client = Some(tool.mcp_client.clone());
            let logs = session.logs.clone();
            let metrics = session.metrics.clone();
            let session: Arc<AMutex<Box<dyn IntegrationSession>>> =
                Arc::new(AMutex::new(Box::new(session)));
            gcx.integration_sessions
                .lock()
                .await
                .insert(tool.config_path.clone(), session.clone());
            let handler = McpClientHandler {
                restricted: true,
                peer_arc: Arc::new(AMutex::new(None)),
                session_arc: session,
                logs: logs.clone(),
                debug_name: "local_fixture".into(),
                request_timeout: 2,
                gcx: Arc::downgrade(&gcx),
                tool_refresh_handle: Arc::new(AMutex::new(None)),
                resource_refresh_handle: Arc::new(AMutex::new(None)),
                prompt_refresh_handle: Arc::new(AMutex::new(None)),
            };
            let (client_io, server_io) = tokio::io::duplex(16384);
            let calls = Arc::new(AtomicUsize::new(0));
            let server_calls = calls.clone();
            let fixture = tokio::spawn(async move {
                let (read, mut write) = tokio::io::split(server_io);
                let mut lines = BufReader::new(read).lines();
                while let Some(line) = lines.next_line().await.unwrap() {
                    let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                    let response = match request["method"].as_str() {
                        Some("initialize") => {
                            json!({"jsonrpc":"2.0","id":request["id"],"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"local_fixture","version":"1"}}})
                        }
                        Some("tools/call") => {
                            server_calls.fetch_add(1, Ordering::Relaxed);
                            if fail_closed {
                                break;
                            }
                            json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32603,"message":"Transport closed https://private.example.test?token=abc123 MY_ENV=hidden /opt/private/config server-stderr","data":{"token":"private-token"}}})
                        }
                        _ => continue,
                    };
                    write
                        .write_all(format!("{response}\n").as_bytes())
                        .await
                        .unwrap();
                }
            });
            let client = timeout(
                Duration::from_secs(2),
                rmcp::serve_client(handler, client_io),
            )
            .await
            .unwrap()
            .unwrap();
            *tool.mcp_client.lock().await = Some(client);
            let ccx = AtCommandsContext::new_with_abort(
                crate::app_state::AppState::from_gcx(gcx.clone()).await,
                4096,
                20,
                false,
                Vec::new(),
                "mcp-wrapper-fixture".into(),
                None,
                "model".into(),
                None,
                None,
                None,
            )
            .await;
            let result = timeout(
                Duration::from_secs(3),
                tool.tool_execute(Arc::new(AMutex::new(ccx)), &"call".into(), &HashMap::new()),
            )
            .await
            .unwrap();
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("MCP failure must not be accepted as success"),
            };
            assert!(error.contains("server 'local_fixture', tool 'test_tool'"));
            assert!(error.contains(expected_cause));
            assert!(error.contains("External outcome is unknown"));
            assert!(error.contains("No automatic replay or reconnect was attempted by this call"));
            assert!(error.contains("Check the MCP integration status"));
            assert!(error.ends_with("whether to retry."));
            assert!(error.len() <= MAX_MCP_FAILURE_BYTES);
            let rendered_logs = serde_json::to_string(&*logs.lock().await).unwrap();
            assert!(rendered_logs.contains(&error));
            for secret in ["private", "abc123", "MY_ENV", "hidden", "server-stderr"] {
                assert!(!error.contains(secret));
                assert!(!rendered_logs.contains(secret));
            }
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            {
                let recorded = metrics.lock().await;
                assert_eq!(recorded.metrics.failed_calls, 1);
                assert_eq!(recorded.metrics.successful_calls, 0);
                assert_eq!(recorded.metrics.reconnect_count, 0);
            }
            let client = tool.mcp_client.lock().await.take().unwrap();
            let _ = timeout(Duration::from_secs(2), client.cancel())
                .await
                .unwrap();
            timeout(Duration::from_secs(2), fixture)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[test]
    fn metadata_hashes_schema_and_uses_only_allowlisted_descriptors() {
        let tool = make_tool_mcp(serde_json::json!({"type": "object"}));
        let content = ChatContent::SimpleText("{\"ok\":true}".to_string());

        let metadata = mcp_metadata(&tool, &content).unwrap();

        assert_eq!(metadata["server"], "mcp_stdio_server");
        assert_eq!(metadata["tool"], "test_tool");
        assert_eq!(metadata["result_type"], "text");
        assert_eq!(metadata["schema_hash"].as_str().unwrap().len(), 16);
        assert!(!metadata.to_string().contains("args"));
    }

    #[test]
    fn metadata_rejects_secret_like_tool_names() {
        let mut tool = make_tool_mcp(serde_json::json!({"type": "object"}));
        tool.mcp_tool.name = "get secret=token".into();

        assert!(mcp_metadata(&tool, &ChatContent::SimpleText("ok".to_string())).is_none());
    }

    fn call_params(arguments: serde_json::Value) -> CallToolRequestParams {
        let mut params = CallToolRequestParams::new("test_tool");
        params.arguments = arguments.as_object().cloned();
        params
    }

    fn set_policy(gcx: &Arc<crate::global_context::GlobalContext>, zones: Vec<Zone>) {
        *gcx.privacy_policy_load.write().unwrap() = refact_privacy::PolicyLoad {
            policy: Arc::new(PrivacyPolicy {
                blocked: Vec::new(),
                zones,
                ..Default::default()
            }),
            error: None,
            source_paths: Vec::new(),
        };
    }

    #[tokio::test]
    async fn mcp_gate_refuses_guarded_argument_before_dispatch() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let secret = gcx.cache_dir.join("guarded.txt");
        std::fs::write(&secret, "secret").unwrap();
        set_policy(
            &gcx,
            vec![
                Zone {
                    name: "secrets".to_string(),
                    patterns: vec![secret.to_string_lossy().to_string()],
                    send_to: vec!["trusted".to_string()],
                    ..Default::default()
                },
                Zone {
                    name: "normal".to_string(),
                    patterns: vec!["*".to_string()],
                    send_to: vec!["*".to_string()],
                    ..Default::default()
                },
            ],
        );
        let dispatched = AtomicUsize::new(0);

        let error = dispatch_cleared_mcp_call(
            &gcx,
            "untrusted",
            call_params(json!({"path": secret})),
            &[],
            |params| {
                dispatched.fetch_add(1, Ordering::Relaxed);
                params
            },
        )
        .unwrap_err();

        assert_eq!(error, format!(
            "Request not sent: user privacy policy does not allow \"untrusted\" to receive data from these guarded files present in this chat's context:\n  - {} (zone \"secrets\", declared)\nRetrying the same request will be refused again. Ask the user to allow \"untrusted\" for zone(s) \"secrets\" in Privacy settings, or to clear this chat's privacy record for these files.",
            secret.to_string_lossy()
        ));
        assert_eq!(dispatched.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn mcp_gate_unions_producing_message_records() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_policy(
            &gcx,
            vec![
                Zone {
                    name: "secrets".to_string(),
                    patterns: vec![".env".to_string()],
                    send_to: Vec::new(),
                    ..Default::default()
                },
                Zone {
                    name: "normal".to_string(),
                    patterns: vec!["*".to_string()],
                    send_to: vec!["*".to_string()],
                    ..Default::default()
                },
            ],
        );
        let mut producing_message = ChatMessage::default();
        producing_message.extra.insert(
            "privacy".to_string(),
            serde_json::to_value(PrivacyRecord {
                files: vec![FileRecord {
                    path: ".env".to_string(),
                    zone: "secrets".to_string(),
                    attribution: Attribution::Declared,
                }],
            })
            .unwrap(),
        );
        let dispatched = AtomicUsize::new(0);

        let error = dispatch_cleared_mcp_call(
            &gcx,
            "server",
            call_params(json!({"query": "hello"})),
            &[producing_message],
            |params| {
                dispatched.fetch_add(1, Ordering::Relaxed);
                params
            },
        )
        .unwrap_err();

        assert!(error.starts_with("Request not sent: user privacy policy"));
        assert!(error.contains("zone \""));
        assert_eq!(dispatched.load(Ordering::Relaxed), 0);
    }

    #[derive(Clone)]
    struct OfflineTransport;

    #[async_trait::async_trait]
    impl super::super::integr_mcp_common::MCPTransportInitializer for OfflineTransport {
        async fn init_mcp_transport(
            &self,
            _logs: Arc<AMutex<Vec<String>>>,
            _name: String,
            _init_timeout: u64,
            _request_timeout: u64,
            _session: Arc<AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
            _handler: super::super::session_mcp::McpClientHandler,
        ) -> Option<McpRunningService> {
            None
        }
    }

    async fn fixture_ccx(
        gcx: &Arc<crate::global_context::GlobalContext>,
    ) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_with_abort(
                crate::app_state::AppState::from_gcx(gcx.clone()).await,
                4096,
                20,
                false,
                Vec::new(),
                "mcp-offline-fixture".into(),
                None,
                "model".into(),
                None,
                None,
                None,
            )
            .await,
        ))
    }

    #[tokio::test]
    async fn offline_server_with_cached_catalog_fails_tool_call_promptly() {
        use super::super::integr_mcp_common::{
            mcp_integr_tools, mcp_session_setup, remove_mcp_session_for_config,
            tool_catalog_cache_store, CommonMCPSettings,
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_policy(&gcx, Vec::new());
        let path = "/tmp/mcp_http_offline_cached_fixture.yaml";
        let cached: McpTool =
            serde_json::from_value(json!({"name": "read", "inputSchema": {"type": "object"}}))
                .unwrap();
        tool_catalog_cache_store(path, &[cached]);
        mcp_session_setup(
            gcx.clone(),
            path.into(),
            json!({}),
            OfflineTransport,
            1,
            1,
            30,
            3,
            true,
        )
        .await;
        let mcp_common = CommonMCPSettings {
            init_timeout: 1,
            request_timeout: 1,
            ..Default::default()
        };
        let mut tools = Vec::new();
        timeout(Duration::from_secs(5), async {
            loop {
                tools = mcp_integr_tools(
                    Some(Arc::downgrade(&gcx)),
                    path,
                    &IntegrationCommon::default(),
                    &mcp_common,
                )
                .await;
                if !tools.is_empty()
                    && super::super::integr_mcp_common::mcp_next_retry_in_secs(path).is_some()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cached catalog must be served while reconnecting");
        let started = std::time::Instant::now();
        let error = match timeout(
            Duration::from_secs(5),
            tools[0].tool_execute(fixture_ccx(&gcx).await, &"call".into(), &HashMap::new()),
        )
        .await
        .expect("tool call must not wait for the reconnect supervisor")
        {
            Err(error) => error,
            Ok(_) => panic!("offline MCP call must fail"),
        };
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(
            error.contains("MCP server 'offline_cached_fixture' is not connected (reconnecting"),
            "{error}"
        );
        remove_mcp_session_for_config(gcx, path).await;
    }

    #[tokio::test(start_paused = true)]
    async fn startup_wait_in_tool_call_is_bounded() {
        use super::super::session_mcp::SessionMCP;
        use crate::integrations::sessions::IntegrationSession;
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_policy(&gcx, Vec::new());
        let mut tool = make_tool_mcp(json!({"type": "object"}));
        tool.config_path = "/tmp/mcp_stdio_hung_startup_fixture.yaml".into();
        let startup = tokio::spawn(std::future::pending::<()>());
        let abort = startup.abort_handle();
        let mut session = SessionMCP::detached(&tool.config_path);
        session.connection_status = MCPConnectionStatus::Connecting;
        session.startup_task_handles = Some((Arc::new(AMutex::new(Some(startup))), abort.clone()));
        let session: Arc<AMutex<Box<dyn IntegrationSession>>> =
            Arc::new(AMutex::new(Box::new(session)));
        gcx.integration_sessions
            .lock()
            .await
            .insert(tool.config_path.clone(), session);
        let started = tokio::time::Instant::now();
        let result = tool
            .tool_execute(fixture_ccx(&gcx).await, &"call".into(), &HashMap::new())
            .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("unconnected MCP call must fail"),
        };
        let waited = started.elapsed();
        assert!(waited >= Duration::from_secs(tool.init_timeout + STARTUP_WAIT_MARGIN_SECS));
        assert!(waited < Duration::from_secs(tool.init_timeout + STARTUP_WAIT_MARGIN_SECS + 2));
        assert!(
            error.contains("is not connected (still connecting)"),
            "{error}"
        );
        abort.abort();
    }

    #[tokio::test]
    async fn integr_stdio_observation_unavailable_is_fail_open() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut message = ChatMessage::default();

        attach_mcp_observation_status(
            &gcx,
            refact_exec::ObservationStatus::Unavailable("ptrace denied".to_string()),
            &std::collections::HashSet::new(),
            &mut message,
        );

        assert_eq!(
            message.extra["privacy_observation"],
            json!({"status": "unavailable", "reason": "ptrace denied"})
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore]
    async fn integr_stdio_observed_reads_reach_tool_result_records() {
        if std::env::var_os("REFACT_TEST_PTRACE").is_none() {
            return;
        }
        let probe = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(probe.path(), "probe").unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.privacy_policy_load.write().unwrap() = refact_privacy::PolicyLoad {
            policy: Arc::new(PrivacyPolicy {
                blocked: Vec::new(),
                zones: vec![
                    Zone {
                        name: "secrets".to_string(),
                        patterns: vec![probe.path().to_string_lossy().to_string()],
                        send_to: Vec::new(),
                        ..Default::default()
                    },
                    Zone {
                        name: "normal".to_string(),
                        patterns: vec!["*".to_string()],
                        send_to: vec!["*".to_string()],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            error: None,
            source_paths: Vec::new(),
        };
        let mut command = tokio::process::Command::new("cat");
        command.arg(probe.path());
        command.stdout(std::process::Stdio::null());
        let setup = refact_exec::ObservationSetup::prepare(&mut command);
        let mut child = command.spawn().unwrap();
        let runtime = setup.start_with_external_reaper(child.id());
        let observation = runtime.reader();
        let status =
            tokio::time::timeout(std::time::Duration::from_secs(2), observation.wait_status())
                .await
                .unwrap();
        let exit = child.wait().await.unwrap();
        assert!(exit.success());
        assert!(matches!(
            status,
            refact_exec::ObservationStatus::Observed(ref access)
                if access.reads.iter().any(|path| path == probe.path())
        ));

        let mut message = ChatMessage::default();
        attach_mcp_observation(
            &gcx,
            Some(&observation),
            &std::collections::HashSet::new(),
            &mut message,
        );
        let records = message.extra["privacy"]["files"].as_array().unwrap();
        assert!(records.iter().any(|record| {
            record["path"] == probe.path().to_string_lossy().as_ref()
                && record["zone"] == "secrets"
                && record["attribution"] == "observed"
        }));
    }
}
