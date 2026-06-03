use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};

use refact_core::chat_types::ChatUsage;
use crate::adapter::{
    AdapterSettings, HttpParts, LlmWireAdapter, StreamParseError, extract_extra_fields,
    insert_extra_headers,
};
use crate::canonical::{CanonicalToolChoice, LlmRequest, LlmStreamDelta};
use crate::params::CacheControl;
use super::claude_code_compat;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_THINKING_BUDGET: usize = 8192;
const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";
const ANTHROPIC_CACHE_CONTROL_MARKER_LIMIT: usize = 4;
const ANTHROPIC_CACHE_LOOKBACK_STRIDE_BLOCKS: usize = 14;

const PROTECTED_FIELDS: &[&str] = &[
    "model",
    "messages",
    "stream",
    "system",
    "tools",
    "tool_choice",
    "metadata",
    "cache_control",
];

pub struct AnthropicAdapter;

impl LlmWireAdapter for AnthropicAdapter {
    fn build_http(
        &self,
        req: &LlmRequest,
        settings: &AdapterSettings,
    ) -> Result<HttpParts, String> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let is_cc = claude_code_compat::is_claude_code_oauth(&settings.auth_token);
        let is_github_copilot = crate::provider_quirks::is_github_copilot_request(req, settings);
        if is_cc {
            claude_code_compat::apply_oauth_headers(&mut headers, &settings.auth_token)?;
            claude_code_compat::apply_stainless_headers(
                &mut headers,
                req.claude_code_identity.as_ref(),
            )?;
        } else if is_github_copilot && !settings.api_key.is_empty() {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", settings.api_key))
                    .map_err(|e| format!("invalid api_key for header: {e}"))?,
            );
        } else if !settings.api_key.is_empty() {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(&settings.api_key)
                    .map_err(|e| format!("invalid api_key: {e}"))?,
            );
        }

        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );

        let is_effort_mode = settings.reasoning_type.as_deref() == Some("anthropic_effort");
        let enable_prompt_cache =
            matches!(req.cache_control, CacheControl::Ephemeral) && settings.supports_cache_control;

        insert_extra_headers(&mut headers, &settings.extra_headers);
        crate::provider_quirks::apply_github_copilot_request_headers(&mut headers, req, settings);

        let context_sanitizer: Option<Box<dyn Fn(&str) -> String>> = if is_cc {
            Some(Box::new(|text: &str| {
                claude_code_compat::sanitize_system_text(text)
            }))
        } else {
            None
        };
        let (system, messages) = convert_to_anthropic(&req.messages, context_sanitizer.as_deref());

        let mut body = json!({
            "model": settings.model_name,
            "messages": messages,
            "max_tokens": req.params.max_tokens,
            "stream": req.stream,
        });

        if let Some(sys) = system {
            if is_cc {
                let sanitized = claude_code_compat::sanitize_system_for_cc(sys);
                body["system"] = claude_code_compat::prepend_system(sanitized);
            } else {
                body["system"] = sys;
            }
        } else if is_cc {
            body["system"] = json!(claude_code_compat::SYSTEM_PREFIX);
        }

        if let Some(temp) = req.params.temperature {
            body["temperature"] = json!(temp);
        }

        if !req.params.stop.is_empty() {
            body["stop_sequences"] = json!(req.params.stop);
        }

        if settings.supports_tools {
            if let Some(tools) = &req.tools {
                if !tools.is_empty() {
                    let mut converted_tools = convert_tools_to_anthropic(tools);
                    if is_cc {
                        claude_code_compat::apply_cc_tool_names(&mut converted_tools);
                    }
                    if settings.supports_web_search {
                        if let Some(arr) = converted_tools.as_array_mut() {
                            arr.push(json!({
                                "type": "web_search_20250305",
                                "name": "web_search"
                            }));
                        }
                    }
                    body["tools"] = converted_tools;
                    if let Some(choice) = &req.tool_choice {
                        let mut tc = tool_choice_to_anthropic(choice);
                        if is_cc {
                            if let Some(name) = tc
                                .get("name")
                                .and_then(|n| n.as_str())
                                .map(|s| s.to_string())
                            {
                                if name.starts_with(claude_code_compat::MCP_TOOL_PREFIX) {
                                    // already transformed
                                } else if name.starts_with("mcp_") {
                                    tc["name"] = json!(&name["mcp_".len()..]);
                                } else {
                                    let renamed = claude_code_compat::cc_rename_base_tool(&name);
                                    tc["name"] = json!(format!(
                                        "{}{}",
                                        claude_code_compat::MCP_TOOL_PREFIX,
                                        renamed
                                    ));
                                }
                            }
                        }
                        body["tool_choice"] = tc;
                    }
                }
            } else if settings.supports_web_search {
                body["tools"] = json!([{
                    "type": "web_search_20250305",
                    "name": "web_search"
                }]);
            }
        }

        if settings.supports_reasoning {
            if is_effort_mode {
                match &req.reasoning {
                    crate::params::ReasoningIntent::BudgetTokens(_n) => {
                        body["thinking"] = json!({"type": "adaptive", "display": "summarized"});
                        body["output_config"] = json!({"effort": "high"});
                    }
                    _ => {
                        if let Some(effort) = req.reasoning.to_anthropic_effort() {
                            body["thinking"] = json!({"type": "adaptive", "display": "summarized"});
                            body["output_config"] = json!({"effort": effort});
                        }
                    }
                }
            } else {
                if let Some(budget) = req.reasoning.to_anthropic_budget(DEFAULT_THINKING_BUDGET) {
                    body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
                    let current_max = req.params.max_tokens;
                    if current_max <= budget {
                        let adjusted_max = budget + std::cmp::max(current_max, 1024);
                        body["max_tokens"] = json!(adjusted_max);
                        tracing::debug!(
                            "Adjusted max_tokens from {} to {} (thinking budget: {})",
                            current_max,
                            adjusted_max,
                            budget
                        );
                    }
                }
            }
            body.as_object_mut().map(|obj| obj.remove("temperature"));
        }

        if is_cc {
            claude_code_compat::inject_billing_block(&mut body);
            claude_code_compat::inject_metadata(&mut body, req.claude_code_identity.as_ref());
        }

        if let Some(extra) = &req.extra_body {
            if let Some(obj) = body.as_object_mut() {
                for (k, v) in extra {
                    if PROTECTED_FIELDS.contains(&k.as_str()) {
                        tracing::warn!(
                            "extra_body attempted to override protected field '{}', ignoring",
                            k
                        );
                        continue;
                    }
                    obj.insert(k.clone(), v.clone());
                }
            }
        }

        crate::provider_quirks::remove_anthropic_unsupported_fields(&mut body, settings);

        if is_cc {
            if let Some(msgs) = body.get_mut("messages") {
                claude_code_compat::apply_cc_tool_use_in_messages(msgs);
            }
        }

        if enable_prompt_cache {
            inject_anthropic_cache_control(&mut body);
            let marker_count = count_cache_control_markers(&body);
            if marker_count > ANTHROPIC_CACHE_CONTROL_MARKER_LIMIT {
                tracing::warn!(
                    marker_count,
                    "anthropic prompt cache marker count exceeds provider limit"
                );
            }
        }

        {
            let mut betas: Vec<&str> = Vec::new();
            let thinking_type = body
                .get("thinking")
                .and_then(|t| t.get("type"))
                .and_then(|t| t.as_str());
            if thinking_type == Some("enabled") {
                if !betas.contains(&INTERLEAVED_THINKING_BETA) {
                    betas.push(INTERLEAVED_THINKING_BETA);
                }
            }
            if is_cc {
                for &beta in claude_code_compat::CC_OAUTH_BETAS {
                    if !betas.contains(&beta) {
                        betas.push(beta);
                    }
                }
            }
            if !betas.is_empty() {
                let beta_value = betas.join(",");
                headers.insert(
                    "anthropic-beta",
                    HeaderValue::from_str(&beta_value)
                        .map_err(|e| format!("invalid anthropic-beta: {e}"))?,
                );
            }
        }

        tracing::info!(
            model = %settings.model_name,
            endpoint = %settings.endpoint,
            stream = %req.stream,
            max_tokens = %req.params.max_tokens,
            temperature = ?req.params.temperature,
            stop_sequences = ?req.params.stop.len(),
            tools_count = ?req.tools.as_ref().map(|t| t.len()),
            tool_choice = ?req.tool_choice,
            reasoning = ?req.reasoning,
            cache_control = ?req.cache_control,
            messages_count = %req.messages.len(),
            has_auth_token = %!settings.auth_token.is_empty(),
            has_api_key = %!settings.api_key.is_empty(),
            "anthropic adapter request"
        );

        let url = if is_cc {
            claude_code_compat::build_oauth_url(&settings.endpoint)
        } else {
            settings.endpoint.clone()
        };

        Ok(HttpParts { url, headers, body })
    }

    fn parse_stream_chunk(&self, data: &str) -> Result<Vec<LlmStreamDelta>, StreamParseError> {
        let trimmed = data.trim();
        if trimmed.is_empty() {
            return Err(StreamParseError::Skip);
        }

        let json: Value = serde_json::from_str(trimmed)
            .map_err(|e| StreamParseError::MalformedChunk(format!("json: {e}")))?;

        if let Some(err) = json.get("error") {
            return Err(StreamParseError::FatalError(
                err.get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("error")
                    .to_string(),
            ));
        }

        let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let mut deltas = Vec::new();

        match event_type {
            "content_block_delta" => {
                if let Some(delta) = json.get("delta") {
                    let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match delta_type {
                        "text_delta" => {
                            if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                let block_index = json.get("index").and_then(|i| i.as_u64());
                                deltas.push(LlmStreamDelta::AppendContent {
                                    text: text.to_string(),
                                    block_index,
                                });
                            }
                        }
                        "thinking_delta" => {
                            if let Some(text) = delta.get("thinking").and_then(|t| t.as_str()) {
                                let block_index = json.get("index").and_then(|i| i.as_u64());
                                deltas.push(LlmStreamDelta::AppendReasoning {
                                    text: text.to_string(),
                                    block_index,
                                });
                            }
                        }
                        "signature_delta" => {
                            // Anthropic signature for thinking block verification
                            // Required for multi-turn tool calling conversations
                            if let Some(signature) = delta.get("signature").and_then(|s| s.as_str())
                            {
                                let block_index =
                                    json.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                                deltas.push(LlmStreamDelta::SetThinkingBlocks {
                                    blocks: vec![json!({
                                        "index": block_index,
                                        "type": "thinking",
                                        "signature": signature
                                    })],
                                });
                            }
                        }
                        "input_json_delta" => {
                            if let Some(partial) =
                                delta.get("partial_json").and_then(|p| p.as_str())
                            {
                                let index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                                deltas.push(LlmStreamDelta::SetToolCalls {
                                    tool_calls: vec![
                                        json!({"index": index, "function": {"arguments": partial}}),
                                    ],
                                });
                            }
                        }
                        "citations_delta" => {
                            // Anthropic citations streaming - citation is in delta.citation
                            // Include content block index to preserve association
                            if let Some(citation) = delta.get("citation") {
                                let block_index =
                                    json.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                                let mut enriched = citation.clone();
                                if let Some(obj) = enriched.as_object_mut() {
                                    obj.insert(
                                        "_content_block_index".to_string(),
                                        json!(block_index),
                                    );
                                }
                                deltas.push(LlmStreamDelta::AddCitation { citation: enriched });
                            }
                        }
                        _ => {}
                    }
                }
            }
            "message_start" => {
                if let Some(message) = json.get("message") {
                    if let Some(usage) = message.get("usage") {
                        if let Some(u) = parse_anthropic_usage(usage) {
                            deltas.push(LlmStreamDelta::SetUsage { usage: u });
                        }
                    }
                }
            }
            "message_delta" => {
                let mut has_stop_reason = false;
                if let Some(delta) = json.get("delta") {
                    if let Some(reason) = delta.get("stop_reason").and_then(|r| r.as_str()) {
                        has_stop_reason = true;
                        deltas.push(LlmStreamDelta::SetFinishReason {
                            reason: reason.to_string(),
                        });
                    }
                }
                if let Some(usage) = json.get("usage") {
                    if let Some(u) = parse_anthropic_usage(usage) {
                        deltas.push(LlmStreamDelta::SetUsage { usage: u });
                    }
                }
                if has_stop_reason {
                    deltas.push(LlmStreamDelta::Done);
                }
            }
            "message_stop" => {
                deltas.push(LlmStreamDelta::Done);
            }
            "ping" => {}
            "content_block_start" => {
                if let Some(cb) = json.get("content_block") {
                    let block_type = cb.get("type").and_then(|t| t.as_str());
                    match block_type {
                        Some("tool_use") => {
                            if let (Some(id), Some(name)) = (
                                cb.get("id").and_then(|v| v.as_str()),
                                cb.get("name").and_then(|v| v.as_str()),
                            ) {
                                let index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                                deltas.push(LlmStreamDelta::SetToolCalls {
                                    tool_calls: vec![json!({
                                        "index": index,
                                        "id": id,
                                        "type": "function",
                                        "function": {"name": name}
                                    })],
                                });
                            }
                        }
                        Some("thinking") => {
                            // Anthropic thinking content is streamed incrementally via thinking_delta
                            // which emits AppendReasoning. We don't emit SetThinkingBlocks here
                            // because the content arrives via deltas, not as a complete block.
                            // The thinking content accumulates in ChoiceFinal.reasoning.
                        }
                        Some("server_tool_use") | Some("web_search_tool_result") => {
                            // Server-executed tool blocks (e.g., web_search) must be
                            // preserved verbatim and passed back in multi-turn conversations.
                            // The full block arrives in content_block_start (no incremental deltas).
                            // Include streaming index for correct interleaved ordering
                            // with thinking blocks in multi-turn conversations.
                            let mut block = cb.clone();
                            if let Some(index) = json.get("index") {
                                if let Some(obj) = block.as_object_mut() {
                                    obj.insert("_order_index".to_string(), index.clone());
                                }
                            }
                            deltas.push(LlmStreamDelta::AddServerContentBlock { block });
                        }
                        _ => {}
                    }
                }
            }
            "content_block_stop" => {
                // Note: Anthropic's content_block_stop only contains {"type":"content_block_stop","index":N}
                // It does NOT include the content_block payload. Thinking content is already
                // streamed via thinking_delta -> AppendReasoning, so no action needed here.
            }
            _ => {
                tracing::warn!("Unknown event type: {json:?}");
            }
        }

        // Extract Refact-specific extra fields on ALL events consistently
        let extra = extract_extra_fields(&json);
        if !extra.is_empty() {
            deltas.push(LlmStreamDelta::MergeExtra { extra });
        }

        Ok(deltas)
    }
}

fn convert_to_anthropic(
    messages: &[refact_core::chat_types::ChatMessage],
    context_sanitizer: Option<&dyn Fn(&str) -> String>,
) -> (Option<Value>, Vec<Value>) {
    use super::render_extra::{
        is_context_role, is_event_role, is_plan_role, render_context_message, render_event_message,
        render_plan_message,
    };

    let mut system_text = None;
    let mut result: Vec<Value> = Vec::new();
    let mut pending_tool_results: Vec<Value> = Vec::new();
    let mut pending_context_text: Vec<PendingText> = Vec::new();
    let normalized_messages = normalize_tool_results_after_assistant(messages);

    for msg in normalized_messages.iter() {
        match msg.role.as_str() {
            "system" => {
                system_text = Some(msg.content.content_text_only());
            }
            role if is_plan_role(role) => {
                if !pending_context_text.is_empty() && pending_tool_results.is_empty() {
                    flush_pending_context_text(&mut result, &mut pending_context_text);
                }
                flush_tool_results(&mut result, &mut pending_tool_results);
                if let Some(text) = render_plan_message(msg) {
                    result.push(json!({
                        "role": "user",
                        "content": [{"type": "text", "text": text}]
                    }));
                }
            }
            role if is_context_role(role) || is_event_role(role) => {
                let raw_text = if is_event_role(role) {
                    Some(render_event_message(msg))
                } else {
                    render_context_message(msg)
                };
                let Some(raw_text) = raw_text else {
                    continue;
                };
                let text = match context_sanitizer {
                    Some(f) => f(&raw_text),
                    None => raw_text,
                };
                if !pending_tool_results.is_empty() {
                    // Keep supplemental context/events with the pending tool-result turn.
                    // Delaying them until a later assistant turn rewrites Anthropic message
                    // positions and can invalidate prompt-cache prefix comparisons.
                    pending_tool_results.push(json!({"type": "text", "text": text}));
                } else {
                    pending_context_text.push(PendingText {
                        text,
                        is_event: is_event_role(role),
                    });
                }
            }
            "user" | "assistant" => {
                let mut content = Vec::new();
                // Merge pending tool_results (and any trailing context blocks) into
                // the user message to avoid consecutive user turns.
                if msg.role == "user" && !pending_context_text.is_empty() {
                    flush_tool_results(&mut result, &mut pending_tool_results);
                    for pending in pending_context_text.drain(..) {
                        content.push(json!({"type": "text", "text": pending.text}));
                    }
                } else if msg.role == "user" && !pending_tool_results.is_empty() {
                    content.extend(drain_tool_results_ordered(&mut pending_tool_results));
                } else {
                    // Flush any open tool-results group before an assistant turn.
                    if !pending_context_text.is_empty() && pending_tool_results.is_empty() {
                        // Emit buffered context as a standalone user turn so it is
                        // not lost when an assistant message follows without a user.
                        flush_pending_context_text(&mut result, &mut pending_context_text);
                    }
                    flush_tool_results(&mut result, &mut pending_tool_results);
                }
                if msg.role == "assistant" {
                    let has_stream_text = msg
                        .extra
                        .get("_anthropic_text_blocks")
                        .and_then(|v| v.as_array())
                        .is_some();

                    // Collect positional blocks (text + thinking + server blocks + tool_use)
                    // with their original streaming indices to preserve interleaved order.
                    let mut ordered_blocks: Vec<(u64, u64, Value)> = Vec::new();
                    let mut seq: u64 = 0;

                    if let Some(text_blocks) = msg
                        .extra
                        .get("_anthropic_text_blocks")
                        .and_then(|v| v.as_array())
                    {
                        for block in text_blocks {
                            let (Some(order_idx), Some(text)) = (
                                block.get("index").and_then(|v| v.as_u64()),
                                block.get("text").and_then(|v| v.as_str()),
                            ) else {
                                continue;
                            };
                            if text.is_empty() {
                                continue;
                            }
                            ordered_blocks.push((
                                order_idx,
                                seq,
                                json!({"type": "text", "text": text}),
                            ));
                            seq += 1;
                        }
                    } else {
                        for block in msg_content_to_anthropic(&msg.content) {
                            ordered_blocks.push((u64::MAX, seq, block));
                            seq += 1;
                        }
                    }

                    if let Some(blocks) = &msg.thinking_blocks {
                        for block in blocks {
                            if let Some(block_type) = block.get("type").and_then(|t| t.as_str()) {
                                let order_idx =
                                    block.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                                match block_type {
                                    "thinking" => {
                                        let thinking_text = block
                                            .get("thinking")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        if thinking_text.trim().is_empty() {
                                            tracing::warn!(
                                                "skipping thinking block with empty thinking text"
                                            );
                                            continue;
                                        }
                                        let mut tb = json!({
                                            "type": "thinking",
                                            "thinking": thinking_text,
                                        });
                                        if let Some(sig) = block.get("signature") {
                                            tb["signature"] = sig.clone();
                                        }
                                        ordered_blocks.push((order_idx, seq, tb));
                                        seq += 1;
                                    }
                                    "redacted_thinking" => {
                                        let mut rb = json!({"type": "redacted_thinking"});
                                        if let Some(data) = block.get("data") {
                                            rb["data"] = data.clone();
                                        }
                                        ordered_blocks.push((order_idx, seq, rb));
                                        seq += 1;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    if !msg.server_content_blocks.is_empty() {
                        let result_ids: std::collections::HashSet<&str> = msg
                            .server_content_blocks
                            .iter()
                            .filter(|b| {
                                b.get("type").and_then(|t| t.as_str())
                                    == Some("web_search_tool_result")
                            })
                            .filter_map(|b| b.get("tool_use_id").and_then(|v| v.as_str()))
                            .collect();

                        let server_tool_use_ids: std::collections::HashSet<&str> = msg
                            .server_content_blocks
                            .iter()
                            .filter(|b| {
                                b.get("type").and_then(|t| t.as_str()) == Some("server_tool_use")
                            })
                            .filter_map(|b| b.get("id").and_then(|v| v.as_str()))
                            .collect();

                        let is_complete_historical = !server_tool_use_ids.is_empty()
                            && server_tool_use_ids.iter().all(|id| result_ids.contains(id));

                        for block in &msg.server_content_blocks {
                            if !is_complete_historical
                                && block.get("type").and_then(|t| t.as_str())
                                    == Some("server_tool_use")
                            {
                                let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                if !result_ids.contains(id) {
                                    tracing::warn!("stripping orphaned server_tool_use '{}' (no matching web_search_tool_result)", id);
                                    continue;
                                }
                            }

                            let order_idx = block
                                .get("_order_index")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(u64::MAX);
                            let mut clean = block.clone();
                            if let Some(obj) = clean.as_object_mut() {
                                obj.remove("_order_index");
                            }
                            ordered_blocks.push((order_idx, seq, clean));
                            seq += 1;
                        }
                    }

                    if let Some(tcs) = &msg.tool_calls {
                        for tc in tcs.iter().filter(|tc| !tc.id.starts_with("srvtoolu_")) {
                            let input = match serde_json::from_str::<Value>(&tc.function.arguments)
                            {
                                Ok(v) => v,
                                Err(e) => {
                                    tracing::warn!(
                                        "Invalid JSON in tool call arguments for '{}': {} - using empty object",
                                        tc.function.name, e
                                    );
                                    json!({})
                                }
                            };

                            let order_idx = if has_stream_text {
                                tc.index.map(|i| i as u64).unwrap_or(u64::MAX)
                            } else {
                                u64::MAX
                            };

                            ordered_blocks.push((
                                order_idx,
                                seq,
                                json!({
                                    "type": "tool_use",
                                    "id": tc.id,
                                    "name": tc.function.name,
                                    "input": input,
                                }),
                            ));
                            seq += 1;
                        }
                    }

                    if !msg.citations.is_empty() {
                        let mut citations_by_idx: std::collections::HashMap<
                            Option<u64>,
                            Vec<Value>,
                        > = std::collections::HashMap::new();
                        for c in &msg.citations {
                            let has_encrypted = c.get("encrypted_index").is_some();
                            if has_encrypted && msg.server_content_blocks.is_empty() {
                                tracing::warn!("stripping orphaned web search citation (no server_content_blocks)");
                                continue;
                            }
                            let idx = c.get("_content_block_index").and_then(|v| v.as_u64());
                            let mut cleaned = c.clone();
                            if let Some(obj) = cleaned.as_object_mut() {
                                obj.remove("_content_block_index");
                            }
                            citations_by_idx.entry(idx).or_default().push(cleaned);
                        }

                        // Attach indexed citations to matching text blocks.
                        for (idx, _seq, block) in ordered_blocks.iter_mut() {
                            if block.get("type").and_then(|t| t.as_str()) != Some("text") {
                                continue;
                            }
                            if let Some(cits) = citations_by_idx.remove(&Some(*idx)) {
                                if let Some(obj) = block.as_object_mut() {
                                    obj.insert("citations".to_string(), json!(cits));
                                }
                            }
                        }

                        // Attach remaining citations (unindexed or unmatched) to the last text block.
                        let mut remaining: Vec<Value> =
                            citations_by_idx.remove(&None).unwrap_or_default();
                        for (_idx, mut cits) in citations_by_idx {
                            remaining.append(&mut cits);
                        }
                        if !remaining.is_empty() {
                            if let Some((_idx, _seq, block)) =
                                ordered_blocks.iter_mut().rev().find(|(_, _, b)| {
                                    b.get("type").and_then(|t| t.as_str()) == Some("text")
                                })
                            {
                                if let Some(obj) = block.as_object_mut() {
                                    obj.insert("citations".to_string(), json!(remaining));
                                }
                            }
                        }
                    }

                    ordered_blocks.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
                    for (_, _, block) in ordered_blocks {
                        content.push(block);
                    }
                } else {
                    content.extend(msg_content_to_anthropic(&msg.content));
                }
                let content = sanitize_anthropic_content(content);
                result.push(json!({"role": msg.role, "content": content}));
            }
            "tool" | "diff" => {
                if !msg.tool_call_id.starts_with("srvtoolu_") {
                    let tool_text = msg.content.content_text_only();
                    let tool_text = if tool_text.is_empty() {
                        "(empty)".to_string()
                    } else {
                        tool_text
                    };

                    // Anthropic supports images directly inside tool_result.content as
                    // an array of content blocks.  Build an array when images are present
                    // so the model can see them as part of the tool output.
                    let content_value = match &msg.content {
                        refact_core::chat_types::ChatContent::Multimodal(elements)
                            if elements.iter().any(|el| el.is_image()) =>
                        {
                            let mut blocks = vec![json!({"type": "text", "text": tool_text})];
                            for el in elements.iter().filter(|el| el.is_image()) {
                                blocks.push(json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": el.m_type,
                                        "data": el.m_content
                                    }
                                }));
                            }
                            json!(blocks)
                        }
                        _ => json!(tool_text),
                    };

                    pending_tool_results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": msg.tool_call_id,
                        "content": content_value
                    }));
                }
            }
            _ => {}
        }
    }

    // Flush any remaining context and tool results.
    if !pending_context_text.is_empty() {
        if pending_context_text.iter().any(|pending| pending.is_event) {
            flush_tool_results(&mut result, &mut pending_tool_results);
            flush_pending_context_text(&mut result, &mut pending_context_text);
        } else {
            for pending in pending_context_text.drain(..) {
                pending_tool_results.push(json!({"type": "text", "text": pending.text}));
            }
        }
    }
    flush_tool_results(&mut result, &mut pending_tool_results);

    let system = system_text.map(|text| json!(text));

    (system, result)
}

fn is_client_tool_result_message(message: &refact_core::chat_types::ChatMessage) -> bool {
    (message.role == "tool" || message.role == "diff")
        && !message.tool_call_id.is_empty()
        && !message.tool_call_id.starts_with("srvtoolu_")
}

fn is_tool_result_context_message(message: &refact_core::chat_types::ChatMessage) -> bool {
    super::render_extra::is_context_role(&message.role)
        || super::render_extra::is_event_role(&message.role)
}

fn has_stateful_anthropic_blocks(message: &refact_core::chat_types::ChatMessage) -> bool {
    message
        .thinking_blocks
        .as_ref()
        .is_some_and(|blocks| !blocks.is_empty())
        || !message.server_content_blocks.is_empty()
        || message
            .extra
            .get("_anthropic_text_blocks")
            .and_then(|blocks| blocks.as_array())
            .is_some_and(|blocks| !blocks.is_empty())
}

fn tool_result_bundle_indices(
    messages: &[refact_core::chat_types::ChatMessage],
    result_idx: usize,
    consumed_result_indices: &std::collections::HashSet<usize>,
) -> Vec<usize> {
    let mut indices = vec![result_idx];
    let mut next_idx = result_idx + 1;
    while let Some(message) = messages.get(next_idx) {
        if consumed_result_indices.contains(&next_idx) || !is_tool_result_context_message(message) {
            break;
        }
        indices.push(next_idx);
        next_idx += 1;
    }
    indices
}

fn next_client_tool_owner_index(
    messages: &[refact_core::chat_types::ChatMessage],
    start_idx: usize,
    tool_call_id: &str,
) -> Option<usize> {
    messages
        .iter()
        .enumerate()
        .skip(start_idx + 1)
        .find(|(_, candidate)| {
            candidate.role == "assistant"
                && candidate.tool_calls.as_ref().is_some_and(|tool_calls| {
                    tool_calls.iter().any(|tool_call| {
                        !tool_call.id.starts_with("srvtoolu_") && tool_call.id == tool_call_id
                    })
                })
        })
        .map(|(idx, _)| idx)
}

fn normalize_tool_results_after_assistant(
    messages: &[refact_core::chat_types::ChatMessage],
) -> std::borrow::Cow<'_, [refact_core::chat_types::ChatMessage]> {
    use std::collections::HashSet;

    let needs_normalization = messages.iter().any(|message| {
        is_client_tool_result_message(message)
            || (message.role == "assistant"
                && message.tool_calls.as_ref().is_some_and(|tool_calls| {
                    tool_calls
                        .iter()
                        .any(|tool_call| !tool_call.id.starts_with("srvtoolu_"))
                }))
    });
    if !needs_normalization {
        return std::borrow::Cow::Borrowed(messages);
    }

    let mut consumed_result_indices: HashSet<usize> = HashSet::new();
    let mut normalized = Vec::with_capacity(messages.len());

    for (idx, msg) in messages.iter().enumerate() {
        if consumed_result_indices.contains(&idx) {
            continue;
        }

        if is_client_tool_result_message(msg) {
            tracing::warn!(
                "dropping unmatched Anthropic tool_result '{}' before request",
                msg.tool_call_id
            );
            continue;
        }

        let Some(tool_calls) = msg.tool_calls.as_ref().filter(|calls| !calls.is_empty()) else {
            normalized.push(msg.clone());
            continue;
        };

        if msg.role != "assistant" {
            normalized.push(msg.clone());
            continue;
        }

        let mut matched_tool_call_indices: HashSet<usize> = HashSet::new();
        let mut result_indices = Vec::new();
        let mut seen_tool_call_ids: HashSet<&str> = HashSet::new();

        for (tool_call_idx, tool_call) in tool_calls
            .iter()
            .enumerate()
            .filter(|(_, tool_call)| !tool_call.id.starts_with("srvtoolu_"))
        {
            if !seen_tool_call_ids.insert(tool_call.id.as_str()) {
                tracing::warn!(
                    "stripping duplicate Anthropic tool_use '{}' before request",
                    tool_call.id
                );
                continue;
            }

            let next_owner_idx = next_client_tool_owner_index(messages, idx, &tool_call.id);
            let Some((result_idx, _)) = messages
                .iter()
                .enumerate()
                .skip(idx + 1)
                .take_while(|(candidate_idx, _)| Some(*candidate_idx) != next_owner_idx)
                .find(|(candidate_idx, candidate)| {
                    !consumed_result_indices.contains(candidate_idx)
                        && !result_indices.contains(candidate_idx)
                        && is_client_tool_result_message(candidate)
                        && candidate.tool_call_id == tool_call.id
                })
            else {
                tracing::warn!(
                    "stripping unanswered Anthropic tool_use '{}' before request",
                    tool_call.id
                );
                continue;
            };

            matched_tool_call_indices.insert(tool_call_idx);
            result_indices.push(result_idx);
        }

        let original_client_tool_calls = tool_calls
            .iter()
            .filter(|tool_call| !tool_call.id.starts_with("srvtoolu_"))
            .count();

        let mut assistant = msg.clone();
        if matched_tool_call_indices.len() != original_client_tool_calls {
            if has_stateful_anthropic_blocks(msg) {
                tracing::warn!(
                    "dropping Anthropic assistant message with stateful blocks and incomplete tool results before request"
                );
                for result_idx in result_indices {
                    for bundle_idx in
                        tool_result_bundle_indices(messages, result_idx, &consumed_result_indices)
                    {
                        consumed_result_indices.insert(bundle_idx);
                    }
                }
                continue;
            }

            let filtered_tool_calls = tool_calls
                .iter()
                .enumerate()
                .filter(|tool_call| {
                    tool_call.1.id.starts_with("srvtoolu_")
                        || matched_tool_call_indices.contains(&tool_call.0)
                })
                .map(|(_, tool_call)| tool_call)
                .cloned()
                .collect::<Vec<_>>();
            assistant.tool_calls = if filtered_tool_calls.is_empty() {
                None
            } else {
                Some(filtered_tool_calls)
            };
        }

        normalized.push(assistant);
        for result_idx in result_indices {
            for bundle_idx in
                tool_result_bundle_indices(messages, result_idx, &consumed_result_indices)
            {
                consumed_result_indices.insert(bundle_idx);
                normalized.push(messages[bundle_idx].clone());
            }
        }
    }

    std::borrow::Cow::Owned(normalized)
}

fn flush_tool_results(result: &mut Vec<Value>, pending: &mut Vec<Value>) {
    if pending.is_empty() {
        return;
    }
    let content = drain_tool_results_ordered(pending);
    result.push(json!({
        "role": "user",
        "content": content
    }));
}

fn drain_tool_results_ordered(pending: &mut Vec<Value>) -> Vec<Value> {
    let content = pending.drain(..).partition::<Vec<_>, _>(|block| {
        block.get("type").and_then(Value::as_str) == Some("tool_result")
    });
    content.0.into_iter().chain(content.1).collect()
}

fn flush_pending_context_text(result: &mut Vec<Value>, pending: &mut Vec<PendingText>) {
    if pending.is_empty() {
        return;
    }
    let content: Vec<Value> = pending
        .drain(..)
        .map(|pending| json!({"type": "text", "text": pending.text}))
        .collect();
    result.push(json!({"role": "user", "content": content}));
}

struct PendingText {
    text: String,
    is_event: bool,
}

/// Anthropic rejects `{"type":"text","text":""}` content blocks with 400 Bad Request.
/// This removes empty text blocks, keeping non-text blocks (images, etc.) intact.
/// If nothing remains, inserts a placeholder so the message stays valid.
fn sanitize_anthropic_content(mut blocks: Vec<Value>) -> Vec<Value> {
    blocks.retain(|block| {
        let is_empty_text = block.get("type").and_then(|t| t.as_str()) == Some("text")
            && block
                .get("text")
                .and_then(|t| t.as_str())
                .map_or(false, |s| s.is_empty());
        !is_empty_text
    });
    if blocks.is_empty() {
        blocks.push(json!({"type": "text", "text": "(empty)"}));
    }
    blocks
}

fn msg_content_to_anthropic(content: &refact_core::chat_types::ChatContent) -> Vec<Value> {
    match content {
        refact_core::chat_types::ChatContent::SimpleText(text) => vec![json!({"type": "text", "text": text})],
        refact_core::chat_types::ChatContent::Multimodal(elements) => {
            elements.iter().map(|el| {
                if el.is_image() {
                    json!({"type": "image", "source": {"type": "base64", "media_type": el.m_type, "data": el.m_content}})
                } else {
                    json!({"type": "text", "text": el.m_content})
                }
            }).collect()
        }
        refact_core::chat_types::ChatContent::ContextFiles(_) => {
            vec![json!({"type": "text", "text": content.content_text_only()})]
        }
    }
}

fn convert_tools_to_anthropic(tools: &[Value]) -> Value {
    let converted: Vec<Value> = tools.iter().filter_map(|t| {
        let f = t.get("function")?;
        Some(json!({"name": f.get("name")?, "description": f.get("description").unwrap_or(&json!("")), "input_schema": f.get("parameters").unwrap_or(&json!({}))}))
    }).collect();
    json!(converted)
}

fn anthropic_cache_control() -> Value {
    json!({"type": "ephemeral", "ttl": "1h"})
}

fn inject_anthropic_cache_control(body: &mut Value) {
    let cache_control = anthropic_cache_control();
    let mut remaining_markers = ANTHROPIC_CACHE_CONTROL_MARKER_LIMIT;
    let prefer_message_breakpoints = body
        .get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            message_content_positions(messages).len() > ANTHROPIC_CACHE_LOOKBACK_STRIDE_BLOCKS * 2
        })
        .unwrap_or(false);

    if prefer_message_breakpoints {
        add_body_message_cache_breakpoints(body, &cache_control, &mut remaining_markers);
    }

    if remaining_markers > 0 {
        if let Some(system) = body.get_mut("system") {
            if add_system_cache_breakpoint(system, &cache_control) {
                remaining_markers = remaining_markers.saturating_sub(1);
            }
        }
    }
    if remaining_markers > 0 {
        if let Some(tools) = body.get_mut("tools") {
            if add_tool_cache_breakpoint(tools, &cache_control) {
                remaining_markers = remaining_markers.saturating_sub(1);
            }
        }
    }
    if !prefer_message_breakpoints {
        add_body_message_cache_breakpoints(body, &cache_control, &mut remaining_markers);
    }
}

fn add_body_message_cache_breakpoints(
    body: &mut Value,
    cache_control: &Value,
    remaining_markers: &mut usize,
) {
    if *remaining_markers == 0 {
        return;
    }
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        *remaining_markers = remaining_markers.saturating_sub(add_message_cache_breakpoints(
            messages,
            cache_control,
            *remaining_markers,
        ));
    }
}

fn add_system_cache_breakpoint(system: &mut Value, cache_control: &Value) -> bool {
    match system {
        Value::String(text) => {
            if text.is_empty() {
                return false;
            }
            *system = json!([{
                "type": "text",
                "text": text.clone(),
                "cache_control": cache_control.clone()
            }]);
            true
        }
        Value::Array(blocks) => add_cache_to_last_cacheable_system_block(blocks, cache_control),
        _ => false,
    }
}

#[cfg(test)]
fn add_message_cache_breakpoint(messages: &mut [Value], cache_control: &Value) -> bool {
    add_message_cache_breakpoints(messages, cache_control, 1) > 0
}

fn add_message_cache_breakpoints(
    messages: &mut [Value],
    cache_control: &Value,
    max_markers: usize,
) -> usize {
    if max_markers == 0 {
        return 0;
    }

    let Some(latest) = latest_user_message_cache_position(messages) else {
        return 0;
    };
    let positions = select_message_cache_breakpoints(messages, latest, max_markers);
    let mut added = 0;
    for position in positions {
        if insert_message_cache_breakpoint(messages, position, cache_control) {
            added += 1;
        }
    }
    added
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MessageContentPosition {
    message_index: usize,
    content_index: usize,
}

fn select_message_cache_breakpoints(
    messages: &[Value],
    latest: MessageContentPosition,
    max_markers: usize,
) -> Vec<MessageContentPosition> {
    let positions = message_content_positions(messages);
    let Some(latest_seq) = positions
        .iter()
        .position(|position| position.message_content == latest)
    else {
        return Vec::new();
    };

    let mut selected = vec![latest];
    let mut cursor = latest_seq;
    while selected.len() < max_markers && cursor >= ANTHROPIC_CACHE_LOOKBACK_STRIDE_BLOCKS {
        let window_start = cursor - ANTHROPIC_CACHE_LOOKBACK_STRIDE_BLOCKS;
        let Some(next_cursor) = positions[window_start..cursor]
            .iter()
            .position(|position| position.cacheable)
            .map(|offset| window_start + offset)
        else {
            break;
        };
        selected.push(positions[next_cursor].message_content);
        cursor = next_cursor;
    }
    selected
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MessageProviderBlockPosition {
    message_content: MessageContentPosition,
    cacheable: bool,
}

fn message_content_positions(messages: &[Value]) -> Vec<MessageProviderBlockPosition> {
    let mut positions = Vec::new();
    for (message_index, message) in messages.iter().enumerate() {
        let Some(content) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        for (content_index, block) in content.iter().enumerate() {
            positions.push(MessageProviderBlockPosition {
                message_content: MessageContentPosition {
                    message_index,
                    content_index,
                },
                cacheable: is_cacheable_content_block(block),
            });
        }
    }
    positions
}

fn latest_user_message_cache_position(messages: &[Value]) -> Option<MessageContentPosition> {
    if messages.is_empty() {
        return None;
    }

    let preferred_index = messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))?;

    let content = messages[preferred_index]
        .get("content")
        .and_then(Value::as_array)?;
    latest_user_message_content_index(content).map(|content_index| MessageContentPosition {
        message_index: preferred_index,
        content_index,
    })
}

fn latest_user_message_content_index(content: &[Value]) -> Option<usize> {
    content
        .iter()
        .rposition(is_cacheable_text_content_block)
        .or_else(|| content.iter().rposition(is_cacheable_content_block))
}

fn insert_message_cache_breakpoint(
    messages: &mut [Value],
    position: MessageContentPosition,
    cache_control: &Value,
) -> bool {
    let Some(obj) = messages
        .get_mut(position.message_index)
        .and_then(|message| message.get_mut("content"))
        .and_then(Value::as_array_mut)
        .and_then(|content| content.get_mut(position.content_index))
        .and_then(Value::as_object_mut)
    else {
        return false;
    };
    obj.insert("cache_control".to_string(), cache_control.clone());
    true
}

fn add_tool_cache_breakpoint(tools: &mut Value, cache_control: &Value) -> bool {
    let Some(tools) = tools.as_array_mut() else {
        return false;
    };

    for tool in tools.iter_mut().rev() {
        if is_server_side_tool(tool) {
            continue;
        }
        if let Some(obj) = tool.as_object_mut() {
            obj.insert("cache_control".to_string(), cache_control.clone());
            return true;
        }
    }
    false
}

fn is_cacheable_text_content_block(block: &Value) -> bool {
    block.get("type").and_then(Value::as_str) == Some("text")
        && block
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
}

fn add_cache_to_last_cacheable_system_block(content: &mut [Value], cache_control: &Value) -> bool {
    for block in content.iter_mut().rev() {
        if !is_cacheable_system_block(block) {
            continue;
        }
        if let Some(obj) = block.as_object_mut() {
            obj.insert("cache_control".to_string(), cache_control.clone());
            return true;
        }
    }
    false
}

fn is_cacheable_content_block(block: &Value) -> bool {
    let Some(obj) = block.as_object() else {
        return false;
    };
    matches!(
        obj.get("type").and_then(Value::as_str),
        Some("text" | "image" | "document" | "tool_use" | "tool_result")
    )
}

fn is_cacheable_system_block(block: &Value) -> bool {
    block.get("type").and_then(Value::as_str) == Some("text")
        && block
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
}

fn is_server_side_tool(tool: &Value) -> bool {
    tool.get("type").and_then(Value::as_str) == Some("web_search_20250305")
        || tool.get("name").and_then(Value::as_str) == Some("web_search")
}

fn count_cache_control_markers(value: &Value) -> usize {
    count_block_markers(value.get("system"))
        + value
            .get("messages")
            .and_then(Value::as_array)
            .map(|messages| {
                messages
                    .iter()
                    .map(|message| count_block_markers(message.get("content")))
                    .sum::<usize>()
            })
            .unwrap_or(0)
        + value
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .filter(|tool| tool.get("cache_control").is_some())
                    .count()
            })
            .unwrap_or(0)
}

fn count_block_markers(value: Option<&Value>) -> usize {
    value
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("cache_control").is_some())
                .count()
        })
        .unwrap_or(0)
}

fn tool_choice_to_anthropic(choice: &CanonicalToolChoice) -> Value {
    match choice {
        CanonicalToolChoice::Auto => json!({"type": "auto"}),
        CanonicalToolChoice::None => json!({"type": "none"}),
        CanonicalToolChoice::Required => json!({"type": "any"}),
        CanonicalToolChoice::Function { name } => json!({"type": "tool", "name": name}),
    }
}

fn parse_anthropic_usage(usage: &Value) -> Option<ChatUsage> {
    let prompt_tokens = usage
        .get("input_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as usize;
    let completion_tokens = usage
        .get("output_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as usize;
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(|t| t.as_u64())
        .map(|v| v as usize);
    let cache_read = usage
        .get("cache_read_input_tokens")
        .and_then(|t| t.as_u64())
        .map(|v| v as usize);
    let total_tokens =
        prompt_tokens + completion_tokens + cache_creation.unwrap_or(0) + cache_read.unwrap_or(0);
    Some(ChatUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_creation_tokens: cache_creation,
        cache_read_tokens: cache_read,
        metering_usd: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::ClaudeCodeIdentity;
    use crate::params::CacheControl;
    use refact_core::chat_types::{
        ChatContent, ChatMessage, ChatToolCall, ChatToolFunction, ContextFile,
    };

    fn settings() -> AdapterSettings {
        AdapterSettings {
            api_key: "sk-ant-test".to_string(),
            auth_token: String::new(),
            endpoint: "https://api.anthropic.com/v1/messages".to_string(),
            extra_headers: Default::default(),
            model_name: "claude-3-sonnet".to_string(),
            supports_tools: true,
            supports_reasoning: true,
            reasoning_type: Some("anthropic_budget".to_string()),
            supports_temperature: true,
            supports_max_completion_tokens: false,
            eof_is_done: false,
            supports_web_search: false,
            supports_cache_control: true,
        }
    }

    fn claude_code_settings() -> AdapterSettings {
        AdapterSettings {
            auth_token: "cc-oauth-token".to_string(),
            api_key: String::new(),
            ..settings()
        }
    }

    fn event_message(subkind: &str, source: &str, payload: Value, content: &str) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "event".to_string(),
            json!({"subkind": subkind, "source": source, "payload": payload}),
        );
        ChatMessage {
            role: "event".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            extra,
            ..Default::default()
        }
    }

    fn plan_message(mode: &str, version: u32, content: &str) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "plan".to_string(),
            json!({"mode": mode, "version": version}),
        );
        ChatMessage {
            role: "plan".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            extra,
            ..Default::default()
        }
    }

    fn tool_call(id: &str, name: &str) -> ChatToolCall {
        ChatToolCall {
            id: id.to_string(),
            tool_type: "function".to_string(),
            extra_content: None,
            function: ChatToolFunction {
                name: name.to_string(),
                arguments: "{}".to_string(),
            },
            index: None,
        }
    }

    fn assistant_with_tool_calls(calls: Vec<ChatToolCall>) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            tool_calls: Some(calls),
            ..Default::default()
        }
    }

    fn tool_result(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            tool_call_id: id.to_string(),
            ..Default::default()
        }
    }

    fn plain_text_message(content: &str) -> ChatMessage {
        ChatMessage {
            role: "plain_text".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn context_file_message(tool_call_id: &str, path: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(vec![ContextFile {
                file_name: path.to_string(),
                file_content: content.to_string(),
                line1: 1,
                line2: 2,
                ..Default::default()
            }]),
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        }
    }

    fn assert_tool_results_follow_tool_uses(messages: &[Value]) {
        for (idx, message) in messages.iter().enumerate() {
            if message["role"] != "assistant" {
                continue;
            }
            let tool_use_ids: Vec<_> = message["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|block| block["type"] == "tool_use")
                .filter_map(|block| block["id"].as_str())
                .collect();
            if tool_use_ids.is_empty() {
                continue;
            }

            let next = messages
                .get(idx + 1)
                .unwrap_or_else(|| panic!("assistant message {idx} has no following tool result"));
            assert_eq!(next["role"], "user");
            let result_ids: std::collections::HashSet<_> = next["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|block| block["type"] == "tool_result")
                .filter_map(|block| block["tool_use_id"].as_str())
                .collect();
            for tool_use_id in tool_use_ids {
                assert!(
                    result_ids.contains(tool_use_id),
                    "assistant tool_use {tool_use_id} has no immediately following tool_result"
                );
            }
        }
    }

    fn metadata_user_id(body: &Value) -> Value {
        serde_json::from_str(body["metadata"]["user_id"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn test_claude_code_request_uses_persisted_identity() {
        let adapter = AnthropicAdapter;
        let identity = ClaudeCodeIdentity {
            device_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
            session_id: "11111111-2222-4333-8444-555555555555".to_string(),
        };
        let req = LlmRequest::new(
            "claude_code/claude-sonnet-4".to_string(),
            vec![ChatMessage::new(
                "user".to_string(),
                "Hello from reload".to_string(),
            )],
        )
        .with_claude_code_identity(Some(identity.clone()));

        let http = adapter.build_http(&req, &claude_code_settings()).unwrap();

        assert_eq!(
            http.headers.get("x-claude-code-session-id").unwrap(),
            identity.session_id.as_str()
        );
        let metadata = metadata_user_id(&http.body);
        assert_eq!(metadata["device_id"], identity.device_id);
        assert_eq!(metadata["session_id"], identity.session_id);
        assert!(http.body["metadata"].get("rename_table_version").is_none());
    }

    #[test]
    fn test_claude_code_persisted_identity_bytes_repeat_after_reload() {
        let adapter = AnthropicAdapter;
        let serialized = r#"{
            "device_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "session_id":"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
        }"#;
        let identity: ClaudeCodeIdentity = serde_json::from_str(serialized).unwrap();
        let messages = vec![ChatMessage::new(
            "user".to_string(),
            "Stable first message for billing".to_string(),
        )];
        let req_a = LlmRequest::new("claude_code/claude-sonnet-4".to_string(), messages.clone())
            .with_claude_code_identity(Some(identity.clone()));
        let req_b = LlmRequest::new("claude_code/claude-sonnet-4".to_string(), messages)
            .with_claude_code_identity(Some(identity));

        let http_a = adapter.build_http(&req_a, &claude_code_settings()).unwrap();
        let http_b = adapter.build_http(&req_b, &claude_code_settings()).unwrap();

        assert_eq!(
            http_a.headers.get("x-claude-code-session-id"),
            http_b.headers.get("x-claude-code-session-id")
        );
        assert_eq!(http_a.body["metadata"], http_b.body["metadata"]);
    }

    #[test]
    fn convert_event_to_user_wrapped_xml() {
        let messages = vec![event_message(
            "mode_switch",
            "mode",
            json!({"next":"agent"}),
            "Switched to agent",
        )];

        let (system, converted) = convert_to_anthropic(&messages, None);

        assert!(system.is_none());
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");
        let text = converted[0]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("<event subkind=\"mode_switch\""));
        assert!(text.contains("<message>Switched to agent</message>"));
    }

    #[test]
    fn convert_plan_to_user_wrapped_xml() {
        let messages = vec![plan_message("agent", 1, "Do the thing")];

        let (system, converted) = convert_to_anthropic(&messages, None);

        assert!(system.is_none());
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");
        let text = converted[0]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("<plan mode=\"agent\" version=\"1\">"));
        assert!(text.contains("Do the thing"));
    }

    #[test]
    fn convert_multiple_plan_versions_chronologically() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "first user".to_string()),
            plan_message("agent", 1, "first plan"),
            ChatMessage::new("user".to_string(), "second user".to_string()),
            plan_message("agent", 2, "second plan"),
            ChatMessage::new("user".to_string(), "third user".to_string()),
        ];

        let (system, converted) = convert_to_anthropic(&messages, None);
        let serialized = json!({"system": system, "messages": converted}).to_string();

        assert!(system.is_none());
        assert_eq!(converted.len(), 5);
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[0]["content"][0]["text"], "first user");
        assert!(converted[1]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("version=\"1\""));
        assert_eq!(converted[2]["role"], "user");
        assert_eq!(converted[2]["content"][0]["text"], "second user");
        assert!(converted[3]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("version=\"2\""));
        assert_eq!(converted[4]["role"], "user");
        assert_eq!(converted[4]["content"][0]["text"], "third user");
        assert_eq!(serialized.matches("<plan mode=").count(), 2);
        assert!(!serialized.contains("<plan-history>"));
    }

    #[test]
    fn convert_no_literal_role_strings() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
            event_message("mode_switch", "mode", json!({"next":"agent"}), "Switched"),
            plan_message("agent", 1, "Do it"),
            ChatMessage::new("assistant".to_string(), "Ok".to_string()),
        ];

        let (system, converted) = convert_to_anthropic(&messages, None);
        let body = json!({"system": system, "messages": converted}).to_string();

        assert!(!body.contains("\"role\":\"event\""));
        assert!(!body.contains("\"role\":\"plan\""));
    }

    #[test]
    fn convert_event_does_not_split_thinking_blocks() {
        use refact_core::chat_types::{ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Search for X".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("Answer draft".to_string()),
                thinking_blocks: Some(vec![json!({
                    "type": "thinking",
                    "thinking": "I should search for X",
                    "signature": "sig_search"
                })]),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: ChatToolFunction {
                        name: "search".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("Found results".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
            event_message(
                "tool_decision",
                "tools",
                json!({"allowed":true}),
                "Tool approved",
            ),
        ];

        let (_, converted) = convert_to_anthropic(&messages, None);
        let assistant_index = converted
            .iter()
            .position(|msg| msg["role"] == "assistant")
            .unwrap();
        let assistant_content = converted[assistant_index]["content"].as_array().unwrap();

        assert_eq!(converted[assistant_index]["role"], "assistant");
        assert_eq!(assistant_content[0]["type"], "thinking");
        assert_eq!(assistant_content[1]["type"], "text");
        assert_eq!(assistant_content[2]["type"], "tool_use");
        let tool_result_index = converted
            .iter()
            .position(|msg| {
                msg["content"].as_array().is_some_and(|content| {
                    content.iter().any(|block| block["type"] == "tool_result")
                })
            })
            .unwrap();
        let tool_result_content = converted[tool_result_index]["content"].as_array().unwrap();
        assert_eq!(tool_result_content[0]["type"], "tool_result");
        let event_index = converted
            .iter()
            .position(|msg| {
                msg["content"].as_array().is_some_and(|content| {
                    content.iter().any(|block| {
                        block["type"] == "text"
                            && block["text"]
                                .as_str()
                                .is_some_and(|text| text.contains("<event"))
                    })
                })
            })
            .unwrap();
        assert_eq!(converted[event_index]["role"], "user");
    }

    #[test]
    fn test_build_http_headers() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new("claude".to_string(), vec![]);
        let http = adapter.build_http(&req, &settings()).unwrap();
        assert!(http.headers.get("x-api-key").is_some());
        assert!(http.headers.get("anthropic-version").is_some());
    }

    #[test]
    fn github_copilot_anthropic_messages_uses_bearer_auth_and_copilot_headers() {
        use refact_core::chat_types::MultimodalElement;

        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "github_copilot/claude-sonnet-4".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        let mut settings = settings();
        settings.endpoint = "https://api.githubcopilot.com/v1/messages".to_string();
        settings.api_key = "copilot-token".to_string();
        settings
            .extra_headers
            .insert("Authorization".to_string(), "Bearer hacked".to_string());
        settings
            .extra_headers
            .insert("x-api-key".to_string(), "hacked".to_string());
        settings
            .extra_headers
            .insert("Copilot-Vision-Request".to_string(), "true".to_string());

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(
            http.headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer copilot-token"
        );
        assert!(http.headers.get("x-api-key").is_none());
        assert_eq!(
            http.headers.get("Openai-Intent").unwrap().to_str().unwrap(),
            "conversation-edits"
        );
        assert_eq!(
            http.headers.get("x-initiator").unwrap().to_str().unwrap(),
            "user"
        );
        assert!(http.headers.get("Copilot-Vision-Request").is_none());

        let image_req = LlmRequest::new(
            "github_copilot/claude-sonnet-4".to_string(),
            vec![ChatMessage {
                role: "user".to_string(),
                content: ChatContent::Multimodal(vec![MultimodalElement {
                    m_type: "image/png".to_string(),
                    m_content: "base64data".to_string(),
                }]),
                ..Default::default()
            }],
        );
        let image_http = adapter.build_http(&image_req, &settings).unwrap();
        assert_eq!(
            image_http
                .headers
                .get("Copilot-Vision-Request")
                .unwrap()
                .to_str()
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn test_interleaved_thinking_beta_header() {
        use crate::params::ReasoningIntent;

        let adapter = AnthropicAdapter;

        let req_with_reasoning = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        )
        .with_reasoning(ReasoningIntent::High);

        let http = adapter
            .build_http(&req_with_reasoning, &settings())
            .unwrap();
        let beta = http
            .headers
            .get("anthropic-beta")
            .map(|v| v.to_str().unwrap().to_string());
        // When thinking is enabled, the adapter may include multiple beta flags.
        assert!(beta.is_some());
        let beta = beta.unwrap();
        assert!(beta.contains(INTERLEAVED_THINKING_BETA));
    }

    #[test]
    fn test_no_beta_header_without_reasoning() {
        let adapter = AnthropicAdapter;

        let req_no_reasoning = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        );

        let http = adapter.build_http(&req_no_reasoning, &settings()).unwrap();
        assert!(http.headers.get("anthropic-beta").is_none());
    }

    #[test]
    fn anthropic_cache_control_uses_block_markers_not_top_level() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        )
        .with_cache_control(CacheControl::Ephemeral);

        let http = adapter.build_http(&req, &settings()).unwrap();
        assert!(http.body.get("cache_control").is_none());
        assert_eq!(count_cache_control_markers(&http.body), 1);
        assert_eq!(
            http.body["messages"][0]["content"][0]["cache_control"],
            json!({"type": "ephemeral", "ttl": "1h"})
        );
    }

    #[test]
    fn anthropic_cache_control_marks_system_tools_and_latest_user_message() {
        use refact_core::chat_types::{ChatToolCall, ChatToolFunction};

        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude".to_string(),
            vec![
                ChatMessage::new("system".to_string(), "Be stable".to_string()),
                ChatMessage::new("user".to_string(), "Call the tool".to_string()),
                ChatMessage {
                    role: "assistant".to_string(),
                    content: ChatContent::SimpleText("I'll check".to_string()),
                    tool_calls: Some(vec![ChatToolCall {
                        id: "call_1".to_string(),
                        tool_type: "function".to_string(),
                        extra_content: None,
                        function: ChatToolFunction {
                            name: "lookup".to_string(),
                            arguments: r#"{"query":"cache"}"#.to_string(),
                        },
                        index: None,
                    }]),
                    ..Default::default()
                },
                ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText("tool output".to_string()),
                    tool_call_id: "call_1".to_string(),
                    ..Default::default()
                },
                ChatMessage::new("user".to_string(), "Continue".to_string()),
            ],
        )
        .with_cache_control(CacheControl::Ephemeral)
        .with_tools(
            vec![json!({
                "type": "function",
                "function": {
                    "name": "lookup",
                    "description": "Look up data",
                    "parameters": {"type": "object", "properties": {"query": {"type": "string"}}}
                }
            })],
            Some(CanonicalToolChoice::Auto),
        );

        let http = adapter.build_http(&req, &settings()).unwrap();

        assert!(http.body.get("cache_control").is_none());
        assert_eq!(count_cache_control_markers(&http.body), 3);
        assert_eq!(
            http.body["system"][0]["cache_control"],
            json!({"type": "ephemeral", "ttl": "1h"})
        );
        assert_eq!(
            http.body["tools"][0]["cache_control"],
            json!({"type": "ephemeral", "ttl": "1h"})
        );

        let messages = http.body["messages"].as_array().unwrap();
        let latest_user = messages
            .iter()
            .rev()
            .find(|message| message["role"] == "user")
            .unwrap();
        let latest_user_content = latest_user["content"].as_array().unwrap();
        assert_eq!(latest_user_content[0]["type"], "tool_result");
        assert_eq!(latest_user_content[1]["type"], "text");
        assert_eq!(
            latest_user_content[1]["cache_control"],
            json!({"type": "ephemeral", "ttl": "1h"})
        );
    }

    #[test]
    fn anthropic_supported_flags_keep_cache_markers_tools_and_reasoning() {
        use crate::params::ReasoningIntent;

        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        )
        .with_cache_control(CacheControl::Ephemeral)
        .with_reasoning(ReasoningIntent::High)
        .with_tools(
            vec![json!({
                "type": "function",
                "function": {"name": "tool", "parameters": {}}
            })],
            Some(CanonicalToolChoice::Required),
        );

        let http = adapter.build_http(&req, &settings()).unwrap();

        assert!(http.body.get("cache_control").is_none());
        assert_eq!(count_cache_control_markers(&http.body), 2);
        assert_eq!(
            http.body["messages"][0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert_eq!(http.body["tools"][0]["cache_control"]["type"], "ephemeral");
        assert!(http.body.get("tools").is_some());
        assert_eq!(http.body["tool_choice"], json!({"type": "any"}));
        assert_eq!(http.body["thinking"]["type"], "enabled");
        assert!(http.headers.get("anthropic-beta").is_some());
    }

    #[test]
    fn anthropic_cache_control_off_has_no_markers() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        )
        .with_cache_control(CacheControl::Off);

        let http = adapter.build_http(&req, &settings()).unwrap();

        assert!(http.body.get("cache_control").is_none());
        assert_eq!(count_cache_control_markers(&http.body), 0);
    }

    #[test]
    fn anthropic_cache_control_disabled_by_settings_has_no_markers() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        )
        .with_cache_control(CacheControl::Ephemeral);
        let mut settings = settings();
        settings.supports_cache_control = false;

        let http = adapter.build_http(&req, &settings).unwrap();

        assert!(http.body.get("cache_control").is_none());
        assert_eq!(count_cache_control_markers(&http.body), 0);
    }

    #[test]
    fn anthropic_cache_marks_system_block() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude".to_string(),
            vec![
                ChatMessage::new("system".to_string(), "Be helpful".to_string()),
                ChatMessage::new("user".to_string(), "Hi".to_string()),
            ],
        )
        .with_cache_control(CacheControl::Ephemeral);

        let http = adapter.build_http(&req, &settings()).unwrap();

        let system = http.body["system"].as_array().unwrap();
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["type"], "text");
        assert_eq!(system[0]["text"], "Be helpful");
        assert_eq!(
            system[0]["cache_control"],
            json!({"type": "ephemeral", "ttl": "1h"})
        );
    }

    #[test]
    fn anthropic_cache_marks_last_system_text_block_only() {
        let mut system = json!([
            {"type": "text", "text": "Be helpful"},
            {"type": "image", "source": {"type": "url", "url": "https://example.com/img.png"}}
        ]);

        assert!(add_system_cache_breakpoint(
            &mut system,
            &anthropic_cache_control()
        ));

        assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
        assert!(system[1].get("cache_control").is_none());
    }

    #[test]
    fn anthropic_cache_marks_tool_schema_before_web_search() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        )
        .with_cache_control(CacheControl::Ephemeral)
        .with_tools(
            vec![json!({
                "type": "function",
                "function": {"name": "lookup", "parameters": {}}
            })],
            None,
        );
        let mut settings = settings();
        settings.supports_web_search = true;

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(http.body["tools"][0]["name"], "lookup");
        assert_eq!(http.body["tools"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(http.body["tools"][1]["name"], "web_search");
        assert!(http.body["tools"][1].get("cache_control").is_none());
    }

    #[test]
    fn anthropic_cache_marker_count_stays_under_limit() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude".to_string(),
            vec![
                ChatMessage::new("system".to_string(), "Be stable".to_string()),
                ChatMessage::new("user".to_string(), "test".to_string()),
            ],
        )
        .with_cache_control(CacheControl::Ephemeral)
        .with_tools(
            vec![json!({
                "type": "function",
                "function": {"name": "lookup", "parameters": {}}
            })],
            None,
        );

        let http = adapter.build_http(&req, &settings()).unwrap();

        assert!(count_cache_control_markers(&http.body) <= 4);
        assert_eq!(count_cache_control_markers(&http.body), 3);
    }

    #[test]
    fn anthropic_cache_adds_rolling_message_breakpoint_for_long_conversations() {
        let messages = (0..=44)
            .map(|index| {
                let role = if index % 2 == 0 { "user" } else { "assistant" };
                json!({
                    "role": role,
                    "content": [{"type": "text", "text": format!("message block {index}")}]
                })
            })
            .collect::<Vec<_>>();
        let mut body = json!({
            "system": [{"type": "text", "text": "stable system"}],
            "tools": [{"name": "lookup", "input_schema": {"type": "object"}}],
            "messages": messages,
        });

        inject_anthropic_cache_control(&mut body);

        let message_markers = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .flat_map(|(message_index, message)| {
                message["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .filter(|(_, block)| block.get("cache_control").is_some())
                    .map(move |(content_index, _)| (message_index, content_index))
            })
            .collect::<Vec<_>>();

        assert_eq!(count_cache_control_markers(&body), 4);
        assert_eq!(message_markers, vec![(2, 0), (16, 0), (30, 0), (44, 0)]);
        assert!(body["system"][0].get("cache_control").is_none());
        assert!(body["tools"][0].get("cache_control").is_none());
    }

    #[test]
    fn anthropic_cache_rolling_breakpoints_count_provider_blocks() {
        let messages = (0..=24)
            .map(|index| {
                let role = if index % 2 == 0 { "user" } else { "assistant" };
                json!({
                    "role": role,
                    "content": [
                        {"type": "text", "text": format!("message block {index}")},
                        {"type": "thinking", "thinking": format!("thinking block {index}"), "signature": "sig"}
                    ]
                })
            })
            .collect::<Vec<_>>();
        let mut body = json!({"messages": messages});

        inject_anthropic_cache_control(&mut body);

        let message_markers = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .flat_map(|(message_index, message)| {
                message["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .filter(|(_, block)| block.get("cache_control").is_some())
                    .map(move |(content_index, _)| (message_index, content_index))
            })
            .collect::<Vec<_>>();

        assert_eq!(message_markers, vec![(3, 0), (10, 0), (17, 0), (24, 0)]);
        for marker in message_markers {
            assert_eq!(
                body["messages"][marker.0]["content"][marker.1]["type"],
                "text"
            );
        }
    }

    #[test]
    fn anthropic_cache_does_not_mark_thinking_blocks() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude".to_string(),
            vec![
                ChatMessage::new("user".to_string(), "Do something".to_string()),
                ChatMessage {
                    role: "assistant".to_string(),
                    content: ChatContent::SimpleText("".to_string()),
                    thinking_blocks: Some(vec![
                        json!({
                            "type": "thinking",
                            "thinking": "Let me think",
                            "signature": "sig_1"
                        }),
                        json!({
                            "type": "redacted_thinking",
                            "data": "encrypted"
                        }),
                    ]),
                    tool_calls: Some(vec![tool_call("call_1", "lookup")]),
                    ..Default::default()
                },
                tool_result("call_1", "tool output"),
            ],
        )
        .with_cache_control(CacheControl::Ephemeral);

        let http = adapter.build_http(&req, &settings()).unwrap();

        let assistant_content = http.body["messages"][1]["content"].as_array().unwrap();
        assert_eq!(assistant_content[0]["type"], "thinking");
        assert_eq!(assistant_content[1]["type"], "redacted_thinking");
        assert!(assistant_content[0].get("cache_control").is_none());
        assert!(assistant_content[1].get("cache_control").is_none());
        assert_eq!(assistant_content[2]["type"], "tool_use");
        assert!(assistant_content[2].get("cache_control").is_none());
        assert_eq!(
            http.body["messages"][2]["content"][0]["type"],
            "tool_result"
        );
        assert_eq!(
            http.body["messages"][2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn anthropic_message_cache_marker_targets_latest_user_message() {
        let mut messages = vec![
            json!({"role": "user", "content": [{"type": "text", "text": "markable"}]}),
            json!({"role": "assistant", "content": [{"type": "text", "text": "assistant tail"}]}),
        ];

        assert!(add_message_cache_breakpoint(
            &mut messages,
            &anthropic_cache_control()
        ));

        assert_eq!(
            messages[0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert_eq!(count_block_markers(messages[1].get("content")), 0);
    }

    #[test]
    fn anthropic_message_cache_marker_prefers_latest_user_text_block() {
        let mut messages = vec![json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "first text"},
                {"type": "tool_result", "tool_use_id": "call_1", "content": "result"},
                {"type": "text", "text": "last text"}
            ]
        })];

        assert!(add_message_cache_breakpoint(
            &mut messages,
            &anthropic_cache_control()
        ));

        let content = messages[0]["content"].as_array().unwrap();
        assert!(content[0].get("cache_control").is_none());
        assert!(content[1].get("cache_control").is_none());
        assert_eq!(content[2]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn anthropic_message_cache_marker_falls_back_to_last_user_content_part() {
        let mut messages = vec![json!({
            "role": "user",
            "content": [
                {"type": "tool_result", "tool_use_id": "call_1", "content": "result"},
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "abc"}}
            ]
        })];

        assert!(add_message_cache_breakpoint(
            &mut messages,
            &anthropic_cache_control()
        ));

        let content = messages[0]["content"].as_array().unwrap();
        assert!(content[0].get("cache_control").is_none());
        assert_eq!(content[1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn anthropic_message_cache_marker_skips_unknown_and_server_blocks() {
        let mut messages = vec![json!({
            "role": "user",
            "content": [
                {"type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {}},
                {"type": "future_provider_block", "data": true}
            ]
        })];

        assert!(!add_message_cache_breakpoint(
            &mut messages,
            &anthropic_cache_control()
        ));

        let content = messages[0]["content"].as_array().unwrap();
        assert!(content[0].get("cache_control").is_none());
        assert!(content[1].get("cache_control").is_none());
    }

    #[test]
    fn claude_code_oauth_cache_markers_preserve_system_prefix_and_betas() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude_code/claude-sonnet-4".to_string(),
            vec![
                ChatMessage::new("system".to_string(), "Be helpful".to_string()),
                ChatMessage::new("user".to_string(), "Hello".to_string()),
            ],
        )
        .with_cache_control(CacheControl::Ephemeral)
        .with_tools(
            vec![json!({
                "type": "function",
                "function": {"name": "strategic_planning", "parameters": {}}
            })],
            None,
        );

        let http = adapter.build_http(&req, &claude_code_settings()).unwrap();

        assert!(http.body.get("cache_control").is_none());
        let system = http.body["system"].as_array().unwrap();
        assert!(system[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("x-anthropic-billing-header:"));
        assert!(system[0].get("cache_control").is_none());
        assert_eq!(system[1]["text"], claude_code_compat::SYSTEM_PREFIX);
        assert!(system[1].get("cache_control").is_none());
        assert_eq!(system[2]["text"], "Be helpful");
        assert_eq!(system[2]["cache_control"]["type"], "ephemeral");
        assert_eq!(http.body["tools"][0]["name"], "t_plan");
        assert_eq!(http.body["tools"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(
            http.body["messages"][0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );

        let beta = http
            .headers
            .get("anthropic-beta")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(beta.contains("prompt-caching-scope-2026-01-05"));
    }

    #[test]
    fn test_no_beta_header_when_reasoning_not_supported() {
        use crate::params::ReasoningIntent;

        let adapter = AnthropicAdapter;
        let mut no_reasoning_settings = settings();
        no_reasoning_settings.supports_reasoning = false;

        let req = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        )
        .with_reasoning(ReasoningIntent::High);

        let http = adapter.build_http(&req, &no_reasoning_settings).unwrap();
        assert!(http.headers.get("anthropic-beta").is_none());
    }

    #[test]
    fn test_system_as_top_level() {
        let messages = vec![
            ChatMessage::new("system".to_string(), "Be helpful".to_string()),
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ];
        let (system, msgs) = convert_to_anthropic(&messages, None);
        assert_eq!(system, Some(json!("Be helpful")));
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_convert_system_no_block_level_cache_control() {
        let messages = vec![
            ChatMessage::new("system".to_string(), "Be helpful".to_string()),
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ];
        let (system, msgs) = convert_to_anthropic(&messages, None);
        assert_eq!(system, Some(json!("Be helpful")));
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0]["content"][0].get("cache_control").is_none());
    }

    #[test]
    fn test_parse_stream_text_delta() {
        let adapter = AnthropicAdapter;
        let chunk =
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello"}}"#;
        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        assert!(
            matches!(&deltas[0], LlmStreamDelta::AppendContent { text, .. } if text == "Hello")
        );
    }

    #[test]
    fn test_parse_stream_tool_use_start() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_123","name":"get_weather"}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::SetToolCalls { tool_calls } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0]["id"], "toolu_123");
                assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
            }
            _ => panic!("expected SetToolCalls"),
        }
    }

    #[test]
    fn test_parse_stream_tool_use_input_delta() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"loc"}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::SetToolCalls { tool_calls } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0]["index"], 0);
                assert_eq!(tool_calls[0]["function"]["arguments"], "{\"loc");
            }
            _ => panic!("expected SetToolCalls"),
        }
    }

    #[test]
    fn test_parse_stream_content_block_stop() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_stop","index":0}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        assert!(deltas.is_empty());
    }

    #[test]
    fn test_parse_stream_message_stop() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"message_stop"}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        assert!(matches!(&deltas[0], LlmStreamDelta::Done));
    }

    #[test]
    fn test_parse_stream_message_delta_stop_reason_finishes() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":10,"output_tokens":5}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(deltas.iter().any(|d| {
            matches!(d, LlmStreamDelta::SetFinishReason { reason } if reason == "end_turn")
        }));
        assert!(deltas
            .iter()
            .any(|d| matches!(d, LlmStreamDelta::SetUsage { .. })));
        assert!(matches!(deltas.last(), Some(LlmStreamDelta::Done)));
    }

    #[test]
    fn test_parse_stream_ping_is_ignored() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"ping"}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(deltas.is_empty());
    }

    #[test]
    fn test_parse_stream_thinking_delta() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think..."}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::AppendReasoning { text, .. } => {
                assert_eq!(text, "Let me think...");
            }
            _ => panic!("expected AppendReasoning"),
        }
    }

    #[test]
    fn test_parse_stream_thinking_block_start() {
        let adapter = AnthropicAdapter;
        let chunk =
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        // Thinking blocks are NOT emitted on content_block_start - content arrives via thinking_delta
        // which emits AppendReasoning. This is intentional to avoid empty placeholder blocks.
        assert!(!deltas
            .iter()
            .any(|d| matches!(d, LlmStreamDelta::SetThinkingBlocks { .. })));
    }

    #[test]
    fn test_extra_body_protected_fields_ignored() {
        let adapter = AnthropicAdapter;
        let mut req = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hi".to_string())],
        );
        req.extra_body = Some(serde_json::Map::from_iter([
            ("model".to_string(), json!("hacked-model")),
            (
                "messages".to_string(),
                json!([{"role": "user", "content": "hacked"}]),
            ),
            ("custom_field".to_string(), json!("allowed")),
        ]));

        let http = adapter.build_http(&req, &settings()).unwrap();

        assert_eq!(http.body["model"], "claude-3-sonnet");
        assert_ne!(
            http.body["messages"],
            json!([{"role": "user", "content": "hacked"}])
        );
        assert_eq!(http.body["custom_field"], "allowed");
    }

    #[test]
    fn test_multi_tool_results_grouped() {
        use refact_core::chat_types::{ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Do two things".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![
                    ChatToolCall {
                        id: "call_1".to_string(),
                        tool_type: "function".to_string(),
                        extra_content: None,
                        function: ChatToolFunction {
                            name: "tool_a".to_string(),
                            arguments: "{}".to_string(),
                        },
                        index: None,
                    },
                    ChatToolCall {
                        id: "call_2".to_string(),
                        tool_type: "function".to_string(),
                        extra_content: None,
                        function: ChatToolFunction {
                            name: "tool_b".to_string(),
                            arguments: "{}".to_string(),
                        },
                        index: None,
                    },
                ]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText("Result A".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText("Result B".to_string()),
                tool_call_id: "call_2".to_string(),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["role"], "user");

        let tool_results = msgs[2]["content"].as_array().unwrap();
        assert_eq!(tool_results.len(), 2);
        assert_eq!(tool_results[0]["type"], "tool_result");
        assert_eq!(tool_results[0]["tool_use_id"], "call_1");
        assert_eq!(tool_results[1]["type"], "tool_result");
        assert_eq!(tool_results[1]["tool_use_id"], "call_2");
    }

    #[test]
    fn test_tool_result_merged_into_following_user() {
        use refact_core::chat_types::{ChatContent, ChatToolCall, ChatToolFunction};

        // Simulates post-linearization input: tool reply followed by user message
        // (linearizer folds cf into tool; real user message stays separate)
        let messages = vec![
            ChatMessage::new("user".to_string(), "start".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("calling tool".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: ChatToolFunction {
                        name: "search".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("tool output".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "now fix it".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        // Should be 3 messages: user, assistant, user(tool_result + text)
        // NOT 4: user, assistant, user(tool_result), user(text)
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["role"], "user");

        let content = msgs[2]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "call_1");
        assert_eq!(content[0]["content"], "tool output");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "now fix it");
    }

    #[test]
    fn test_event_after_tool_result_stays_with_tool_result_turn() {
        use refact_core::chat_types::{ChatContent, ChatToolCall, ChatToolFunction};

        let mut event_extra = serde_json::Map::new();
        event_extra.insert(
            "event".to_string(),
            json!({
                "subkind": "system_notice",
                "source": "tools.shell",
                "payload": {"decision": "approve"}
            }),
        );
        let messages = vec![
            ChatMessage::new("user".to_string(), "start".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: ChatToolFunction {
                        name: "shell".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("tool output".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
            ChatMessage {
                role: "event".to_string(),
                content: ChatContent::SimpleText("User approved".to_string()),
                extra: event_extra,
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("next tool".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_2".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: ChatToolFunction {
                        name: "shell".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[3]["role"], "assistant");
        let tool_turn_content = msgs[2]["content"].as_array().unwrap();
        assert_eq!(tool_turn_content[0]["type"], "tool_result");
        assert_eq!(tool_turn_content[0]["tool_use_id"], "call_1");
        assert_eq!(tool_turn_content[1]["type"], "text");
        assert!(tool_turn_content[1]["text"]
            .as_str()
            .unwrap()
            .contains("User approved"));
    }

    #[test]
    fn test_diff_role_as_tool_result() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Edit file".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![refact_core::chat_types::ChatToolCall {
                    id: "call_edit".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: refact_core::chat_types::ChatToolFunction {
                        name: "file_edit".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "diff".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText(
                    "@@ -1 +1 @@".to_string(),
                ),
                tool_call_id: "call_edit".to_string(),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        assert_eq!(msgs.len(), 3);
        let tool_result = &msgs[2]["content"][0];
        assert_eq!(tool_result["type"], "tool_result");
        assert_eq!(tool_result["tool_use_id"], "call_edit");
    }

    #[test]
    fn tool_result_is_moved_immediately_after_assistant_tool_use() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "start".to_string()),
            assistant_with_tool_calls(vec![tool_call("call_1", "search")]),
            event_message(
                "system_notice",
                "tool.search",
                json!({"phase":"running"}),
                "Searching...",
            ),
            ChatMessage::new("assistant".to_string(), "intervening answer".to_string()),
            tool_result("call_1", "tool output"),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
        assert_eq!(msgs[2]["content"][0]["tool_use_id"], "call_1");
        assert_eq!(msgs[2]["content"][1]["type"], "text");
        assert!(msgs[2]["content"][1]["text"]
            .as_str()
            .unwrap()
            .contains("Searching"));
        assert_eq!(msgs[3]["role"], "assistant");
    }

    #[test]
    fn unmatched_duplicate_tool_result_is_not_sent() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "start".to_string()),
            assistant_with_tool_calls(vec![tool_call("call_1", "search")]),
            tool_result("call_1", "nearest output"),
            ChatMessage::new("user".to_string(), "continue".to_string()),
            tool_result("call_1", "later stale output"),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);
        let serialized = serde_json::to_string(&msgs).unwrap();

        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
        assert_eq!(msgs[2]["content"][0]["content"], "nearest output");
        assert!(!serialized.contains("later stale output"));
        assert_tool_results_follow_tool_uses(&msgs);
    }

    #[test]
    fn moved_tool_result_keeps_adjacent_context() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "start".to_string()),
            assistant_with_tool_calls(vec![tool_call("call_1", "search")]),
            ChatMessage::new("assistant".to_string(), "intervening answer".to_string()),
            tool_result("call_1", "tool output"),
            plain_text_message("adjacent tool context"),
            context_file_message("call_1", "src/lib.rs", "fn searched() {}"),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
        assert_eq!(msgs[2]["content"][1]["type"], "text");
        assert!(msgs[2]["content"][1]["text"]
            .as_str()
            .unwrap()
            .contains("adjacent tool context"));
        assert_eq!(msgs[2]["content"][2]["type"], "text");
        let context_text = msgs[2]["content"][2]["text"].as_str().unwrap();
        assert!(context_text.contains("src/lib.rs:1-2"));
        assert!(context_text.contains("fn searched() {}"));
        assert_eq!(msgs[3]["role"], "assistant");
        assert_tool_results_follow_tool_uses(&msgs);
    }

    #[test]
    fn multiple_tool_results_stay_before_adjacent_context() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "start".to_string()),
            assistant_with_tool_calls(vec![
                tool_call("call_1", "search"),
                tool_call("call_2", "read"),
            ]),
            tool_result("call_1", "search output"),
            context_file_message("call_1", "src/lib.rs", "fn searched() {}"),
            tool_result("call_2", "read output"),
            ChatMessage::new("user".to_string(), "continue".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);
        let content = msgs[2]["content"].as_array().unwrap();

        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "call_1");
        assert_eq!(content[1]["type"], "tool_result");
        assert_eq!(content[1]["tool_use_id"], "call_2");
        assert_eq!(content[2]["type"], "text");
        assert!(content[2]["text"]
            .as_str()
            .unwrap()
            .contains("fn searched() {}"));
        assert_eq!(content[3]["type"], "text");
        assert_eq!(content[3]["text"], "continue");
        assert_tool_results_follow_tool_uses(&msgs);
    }

    #[test]
    fn incomplete_stateful_assistant_is_dropped_instead_of_rewritten() {
        let mut assistant = assistant_with_tool_calls(vec![
            tool_call("call_answered", "search"),
            tool_call("call_missing", "search"),
        ]);
        assistant.thinking_blocks = Some(vec![json!({
            "type": "thinking",
            "thinking": "signed reasoning",
            "signature": "sig_1"
        })]);
        let messages = vec![
            ChatMessage::new("user".to_string(), "start".to_string()),
            assistant,
            tool_result("call_answered", "tool output"),
            ChatMessage::new("user".to_string(), "continue".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);
        let serialized = serde_json::to_string(&msgs).unwrap();

        assert!(!serialized.contains("tool_use"));
        assert!(!serialized.contains("tool_result"));
        assert!(!serialized.contains("signed reasoning"));
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "user");
        assert_tool_results_follow_tool_uses(&msgs);
    }

    #[test]
    fn duplicate_tool_use_ids_do_not_escape_without_results() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "start".to_string()),
            assistant_with_tool_calls(vec![
                tool_call("call_1", "search"),
                tool_call("call_1", "search"),
            ]),
            tool_result("call_1", "tool output"),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);
        let assistant_content = msgs[1]["content"].as_array().unwrap();
        let tool_uses = assistant_content
            .iter()
            .filter(|block| block["type"] == "tool_use")
            .count();

        assert_eq!(tool_uses, 1);
        assert_tool_results_follow_tool_uses(&msgs);
    }

    #[test]
    fn repeated_tool_use_id_across_turns_does_not_steal_later_result() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "start".to_string()),
            assistant_with_tool_calls(vec![tool_call("call_1", "search")]),
            ChatMessage::new("user".to_string(), "new turn".to_string()),
            assistant_with_tool_calls(vec![tool_call("call_1", "search")]),
            tool_result("call_1", "later owner output"),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);
        let serialized = serde_json::to_string(&msgs).unwrap();

        assert_eq!(serialized.matches("\"type\":\"tool_use\"").count(), 1);
        assert_eq!(serialized.matches("\"type\":\"tool_result\"").count(), 1);
        assert_eq!(msgs[4]["content"][0]["content"], "later owner output");
        assert_tool_results_follow_tool_uses(&msgs);
    }

    #[test]
    fn unanswered_client_tool_use_is_stripped_before_anthropic_request() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "start".to_string()),
            assistant_with_tool_calls(vec![
                tool_call("call_answered", "search"),
                tool_call("call_missing", "search"),
            ]),
            tool_result("call_answered", "tool output"),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);
        let assistant_content = msgs[1]["content"].as_array().unwrap();
        let tool_uses: Vec<_> = assistant_content
            .iter()
            .filter(|block| block["type"] == "tool_use")
            .collect();

        assert_eq!(tool_uses.len(), 1);
        assert_eq!(tool_uses[0]["id"], "call_answered");
        assert_eq!(msgs[2]["content"][0]["tool_use_id"], "call_answered");
        assert_tool_results_follow_tool_uses(&msgs);
    }

    #[test]
    fn test_stream_tool_use_missing_fields_skipped() {
        let adapter = AnthropicAdapter;
        let chunk_missing_id = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","name":"get_weather"}}"#;
        let chunk_missing_name = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_123"}}"#;

        let deltas1 = adapter.parse_stream_chunk(chunk_missing_id).unwrap();
        let deltas2 = adapter.parse_stream_chunk(chunk_missing_name).unwrap();

        let has_tool_calls1 = deltas1
            .iter()
            .any(|d| matches!(d, LlmStreamDelta::SetToolCalls { .. }));
        let has_tool_calls2 = deltas2
            .iter()
            .any(|d| matches!(d, LlmStreamDelta::SetToolCalls { .. }));

        assert!(!has_tool_calls1);
        assert!(!has_tool_calls2);
    }

    #[test]
    fn test_stream_citations_delta() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_delta","index":2,"delta":{"type":"citations_delta","citation":{"type":"char_location","cited_text":"Some text","document_index":0,"document_title":"doc.txt","start_char_index":0,"end_char_index":10}}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        let has_citation = deltas
            .iter()
            .any(|d| matches!(d, LlmStreamDelta::AddCitation { .. }));
        assert!(has_citation);

        // Verify citation content and block index preservation
        if let Some(LlmStreamDelta::AddCitation { citation }) = deltas
            .iter()
            .find(|d| matches!(d, LlmStreamDelta::AddCitation { .. }))
        {
            assert_eq!(
                citation.get("type").and_then(|v| v.as_str()),
                Some("char_location")
            );
            assert_eq!(
                citation.get("cited_text").and_then(|v| v.as_str()),
                Some("Some text")
            );
            // Verify block index is preserved for multi-block association
            assert_eq!(
                citation
                    .get("_content_block_index")
                    .and_then(|v| v.as_u64()),
                Some(2)
            );
        }
    }

    #[test]
    fn test_thinking_block_start_no_empty_blocks() {
        let adapter = AnthropicAdapter;
        let chunk =
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        // Should NOT emit SetThinkingBlocks - thinking content comes via thinking_delta -> AppendReasoning
        let has_thinking_blocks = deltas
            .iter()
            .any(|d| matches!(d, LlmStreamDelta::SetThinkingBlocks { .. }));
        assert!(!has_thinking_blocks);
    }

    #[test]
    fn test_thinking_max_tokens_adjustment() {
        use crate::adapter::LlmWireAdapter;
        use crate::params::ReasoningIntent;

        let adapter = AnthropicAdapter;

        // Test with max_tokens < thinking budget (should be adjusted)
        let mut req_low_max = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        );
        req_low_max.params.max_tokens = 4096; // Less than DEFAULT_THINKING_BUDGET
        req_low_max.reasoning = ReasoningIntent::High; // Will use DEFAULT_THINKING_BUDGET
        req_low_max.stream = true;

        let http = adapter.build_http(&req_low_max, &settings()).unwrap();
        // Should be adjusted: budget + max(current_max, 1024)
        assert_eq!(http.body["max_tokens"], DEFAULT_THINKING_BUDGET + 4096);
        assert_eq!(
            http.body["thinking"]["budget_tokens"],
            DEFAULT_THINKING_BUDGET
        );

        // Test with max_tokens > thinking budget (should NOT be adjusted)
        let mut req_high_max = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        );
        req_high_max.params.max_tokens = 20000; // More than DEFAULT_THINKING_BUDGET
        req_high_max.reasoning = ReasoningIntent::High;
        req_high_max.stream = true;

        let http2 = adapter.build_http(&req_high_max, &settings()).unwrap();
        // Should remain unchanged
        assert_eq!(http2.body["max_tokens"], 20000);

        // Test with reasoning off (no adjustment needed)
        let mut req_no_thinking = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        );
        req_no_thinking.params.max_tokens = 4096;
        req_no_thinking.reasoning = ReasoningIntent::Off;
        req_no_thinking.stream = true;

        let http3 = adapter.build_http(&req_no_thinking, &settings()).unwrap();
        assert_eq!(http3.body["max_tokens"], 4096);
        assert!(http3.body.get("thinking").is_none());
    }

    #[test]
    fn test_no_block_level_cache_breakpoints_on_messages() {
        // After linearization: user, assistant+tool_use, tool_result, user
        use refact_core::chat_types::{ChatContent, ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("system".to_string(), "Be helpful".to_string()),
            ChatMessage::new("user".to_string(), "What does this do?".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("Let me check".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: ChatToolFunction {
                        name: "tool_a".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("Result".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "Thanks, now explain".to_string()),
        ];

        let (system, msgs) = convert_to_anthropic(&messages, None);

        // System should be plain text (no cache_control)
        assert_eq!(system, Some(json!("Be helpful")));

        // Messages: [0]=user, [1]=assistant+tool_use, [2]=user(tool_result+text)
        // Tool result is merged into the following user message (no consecutive user blocks)
        assert_eq!(msgs.len(), 3);

        // No block-level cache_control in message content
        for i in 0..msgs.len() {
            assert!(msgs[i]["content"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .get("cache_control")
                .is_none());
        }

        // Verify the merged user message contains both tool_result and text
        let last_content = msgs[2]["content"].as_array().unwrap();
        let has_tool_result = last_content.iter().any(|b| b["type"] == "tool_result");
        let has_text = last_content.iter().any(|b| b["type"] == "text");
        assert!(
            has_tool_result,
            "Merged user message should contain tool_result"
        );
        assert!(has_text, "Merged user message should contain user text");
    }

    #[test]
    fn test_no_block_level_cache_breakpoints_single_message() {
        let messages = vec![ChatMessage::new("user".to_string(), "Hello".to_string())];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        assert_eq!(msgs.len(), 1);
        assert!(msgs[0]["content"][0].get("cache_control").is_none());
    }

    #[test]
    fn test_no_block_level_cache_breakpoints_two_messages() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage::new("assistant".to_string(), "Hi there".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        assert_eq!(msgs.len(), 2);
        assert!(msgs[0]["content"][0].get("cache_control").is_none());
        assert!(msgs[1]["content"][0].get("cache_control").is_none());
    }

    #[test]
    fn test_no_cache_breakpoints_when_off() {
        let messages = vec![
            ChatMessage::new("system".to_string(), "Be helpful".to_string()),
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage::new("assistant".to_string(), "Hi".to_string()),
            ChatMessage::new("user".to_string(), "Thanks".to_string()),
        ];

        let (system, msgs) = convert_to_anthropic(&messages, None);

        // System should be plain text, no cache_control
        assert_eq!(system, Some(json!("Be helpful")));

        // No messages should have cache_control
        for msg in &msgs {
            if let Some(content) = msg["content"].as_array() {
                for block in content {
                    assert!(
                        block.get("cache_control").is_none(),
                        "No cache breakpoints expected when CacheControl::Off"
                    );
                }
            }
        }
    }

    #[test]
    fn test_no_block_level_cache_breakpoint_on_tool_use_last_block() {
        use refact_core::chat_types::{ChatContent, ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Do something".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: ChatToolFunction {
                        name: "get_weather".to_string(),
                        arguments: r#"{"city":"London"}"#.to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("Sunny, 20C".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        // [0]=user, [1]=assistant(text+tool_use), [2]=tool_result(user)
        assert_eq!(msgs.len(), 3);

        // No block-level cache_control in message content
        for i in 0..msgs.len() {
            assert!(msgs[i]["content"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .get("cache_control")
                .is_none());
        }
    }

    #[test]
    fn test_thinking_blocks_included_in_assistant() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Solve this".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText(
                    "The answer is 42".to_string(),
                ),
                thinking_blocks: Some(vec![json!({
                    "type": "thinking",
                    "thinking": "Let me work through this...",
                    "signature": "abc123signature"
                })]),
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "Explain more".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        assert_eq!(msgs.len(), 3);
        let assistant_content = msgs[1]["content"].as_array().unwrap();
        // Thinking block should come first, then text
        assert_eq!(assistant_content[0]["type"], "thinking");
        assert_eq!(
            assistant_content[0]["thinking"],
            "Let me work through this..."
        );
        assert_eq!(assistant_content[0]["signature"], "abc123signature");
        assert_eq!(assistant_content[1]["type"], "text");
        assert_eq!(assistant_content[1]["text"], "The answer is 42");
    }

    #[test]
    fn test_thinking_blocks_before_tool_use() {
        use refact_core::chat_types::{ChatContent, ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Search for X".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                thinking_blocks: Some(vec![json!({
                    "type": "thinking",
                    "thinking": "I should search for X",
                    "signature": "sig_search"
                })]),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: ChatToolFunction {
                        name: "search".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("Found results".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        // assistant content: [thinking, (empty text removed), tool_use]
        let assistant_content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(assistant_content[0]["type"], "thinking");
        assert_eq!(assistant_content[0]["signature"], "sig_search");
        // Last block should be tool_use (empty text sanitized away)
        let last = assistant_content.last().unwrap();
        assert_eq!(last["type"], "tool_use");
    }

    #[test]
    fn test_redacted_thinking_blocks() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Test".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText("Response".to_string()),
                thinking_blocks: Some(vec![
                    json!({
                        "type": "thinking",
                        "thinking": "Normal thinking",
                        "signature": "sig1"
                    }),
                    json!({
                        "type": "redacted_thinking",
                        "data": "encrypted_data_here"
                    }),
                ]),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        let content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "thinking");
        assert_eq!(content[0]["thinking"], "Normal thinking");
        assert_eq!(content[1]["type"], "redacted_thinking");
        assert_eq!(content[1]["data"], "encrypted_data_here");
        assert_eq!(content[2]["type"], "text");
    }

    #[test]
    fn test_citations_resent_in_multi_turn() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "What color is the grass?".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText(
                    "The grass is green.".to_string(),
                ),
                citations: vec![json!({
                    "type": "char_location",
                    "cited_text": "The grass is green.",
                    "document_index": 0,
                    "document_title": "My Document",
                    "start_char_index": 0,
                    "end_char_index": 20
                })],
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "And the sky?".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        assert_eq!(msgs.len(), 3);
        let assistant_content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(assistant_content.len(), 1);
        // Text block should have citations attached
        assert_eq!(assistant_content[0]["type"], "text");
        assert_eq!(assistant_content[0]["text"], "The grass is green.");
        let citations = assistant_content[0]["citations"].as_array().unwrap();
        assert_eq!(citations.len(), 1);
        assert_eq!(citations[0]["type"], "char_location");
        assert_eq!(citations[0]["cited_text"], "The grass is green.");
    }

    #[test]
    fn test_empty_citations_not_included_in_resend() {
        let messages = vec![ChatMessage {
            role: "assistant".to_string(),
            content: refact_core::chat_types::ChatContent::SimpleText("Hello".to_string()),
            citations: vec![],
            ..Default::default()
        }];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        let content = msgs[0]["content"].as_array().unwrap();
        assert!(
            content[0].get("citations").is_none(),
            "Empty citations should not be included in re-sent messages"
        );
    }

    #[test]
    fn test_no_thinking_blocks_when_none() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText("Hi there".to_string()),
                thinking_blocks: None,
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        let content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
    }

    #[test]
    fn test_thinking_blocks_no_block_level_cache_breakpoint_on_last_block() {
        use refact_core::chat_types::{ChatContent, ChatToolCall, ChatToolFunction};

        // Simulate call 2: user + assistant(thinking+tool_use) + tool_result
        let messages = vec![
            ChatMessage::new("user".to_string(), "Do something".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                thinking_blocks: Some(vec![json!({
                    "type": "thinking",
                    "thinking": "Let me think...",
                    "signature": "sig_abc"
                })]),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: ChatToolFunction {
                        name: "tool_a".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("Result".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        // No block-level cache_control in message content
        for i in 0..msgs.len() {
            assert!(msgs[i]["content"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .get("cache_control")
                .is_none());
        }
    }

    #[test]
    fn test_content_block_index_cleaned_from_citations() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Search for something".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText("Found it.".to_string()),
                citations: vec![json!({
                    "type": "web_search_result_location",
                    "url": "https://example.com",
                    "title": "Example",
                    "encrypted_index": "abc123",
                    "cited_text": "Found it.",
                    "_content_block_index": 2
                })],
                server_content_blocks: vec![
                    json!({
                        "type": "server_tool_use",
                        "id": "srvtoolu_test",
                        "name": "web_search",
                        "input": {"query": "something"}
                    }),
                    json!({
                        "type": "web_search_tool_result",
                        "tool_use_id": "srvtoolu_test",
                        "content": [{"type": "web_search_result", "url": "https://example.com", "encrypted_content": "enc456"}]
                    }),
                ],
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "Tell me more".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        let assistant_content = msgs[1]["content"].as_array().unwrap();
        // Find the text block (may not be at index 0 due to interleaved server content blocks)
        let text_block = assistant_content
            .iter()
            .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .expect("should have a text block");
        let citations = text_block["citations"].as_array().unwrap();
        assert_eq!(citations.len(), 1);
        assert!(
            citations[0].get("_content_block_index").is_none(),
            "Internal _content_block_index should be stripped from re-sent citations"
        );
        assert_eq!(
            citations[0]["encrypted_index"], "abc123",
            "encrypted_index should be preserved"
        );
    }

    #[test]
    fn test_server_content_blocks_included_in_multi_turn() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "What's the weather?".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText(
                    "It's sunny.".to_string(),
                ),
                server_content_blocks: vec![
                    json!({
                        "type": "server_tool_use",
                        "id": "srvtoolu_abc",
                        "name": "web_search",
                        "input": {"query": "weather today"}
                    }),
                    json!({
                        "type": "web_search_tool_result",
                        "tool_use_id": "srvtoolu_abc",
                        "content": [{"type": "web_search_result", "url": "https://weather.com", "encrypted_content": "enc123"}]
                    }),
                ],
                citations: vec![json!({
                    "type": "web_search_result_location",
                    "url": "https://weather.com",
                    "title": "Weather",
                    "encrypted_index": "idx123",
                    "cited_text": "It's sunny."
                })],
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "And tomorrow?".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        let assistant_content = msgs[1]["content"].as_array().unwrap();
        // Should contain: text block (with citations), server_tool_use, web_search_tool_result
        assert!(
            assistant_content.len() >= 3,
            "Assistant should have text + server content blocks, got {} blocks",
            assistant_content.len()
        );

        let has_server_tool_use = assistant_content
            .iter()
            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("server_tool_use"));
        let has_web_search_result = assistant_content
            .iter()
            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("web_search_tool_result"));
        assert!(
            has_server_tool_use,
            "server_tool_use block should be included"
        );
        assert!(
            has_web_search_result,
            "web_search_tool_result block should be included"
        );
    }

    #[test]
    fn test_parse_stream_server_tool_use() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_start","index":1,"content_block":{"type":"server_tool_use","id":"srvtoolu_abc","name":"web_search","input":{"query":"test"}}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        let has_server_block = deltas
            .iter()
            .any(|d| matches!(d, LlmStreamDelta::AddServerContentBlock { .. }));
        assert!(
            has_server_block,
            "Should emit AddServerContentBlock for server_tool_use"
        );

        if let Some(LlmStreamDelta::AddServerContentBlock { block }) = deltas
            .iter()
            .find(|d| matches!(d, LlmStreamDelta::AddServerContentBlock { .. }))
        {
            assert_eq!(
                block.get("type").and_then(|v| v.as_str()),
                Some("server_tool_use")
            );
            assert_eq!(
                block.get("name").and_then(|v| v.as_str()),
                Some("web_search")
            );
            // Verify streaming index is preserved for interleaved ordering
            assert_eq!(
                block.get("_order_index").and_then(|v| v.as_u64()),
                Some(1),
                "Server content block should carry original streaming index"
            );
        }
    }

    #[test]
    fn test_parse_stream_web_search_tool_result() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_start","index":2,"content_block":{"type":"web_search_tool_result","tool_use_id":"srvtoolu_abc","content":[{"type":"web_search_result","url":"https://example.com","title":"Example","encrypted_content":"enc123"}]}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        let has_server_block = deltas
            .iter()
            .any(|d| matches!(d, LlmStreamDelta::AddServerContentBlock { .. }));
        assert!(
            has_server_block,
            "Should emit AddServerContentBlock for web_search_tool_result"
        );
    }

    #[test]
    fn test_web_search_tool_added_when_supported() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new("claude".to_string(), vec![]).with_tools(
            vec![json!({"type": "function", "function": {"name": "test", "parameters": {}}})],
            None,
        );
        let mut s = settings();
        s.supports_web_search = true;
        let http = adapter.build_http(&req, &s).unwrap();

        let tools = http.body["tools"].as_array().unwrap();
        let has_web_search = tools
            .iter()
            .any(|t| t.get("type").and_then(|v| v.as_str()) == Some("web_search_20250305"));
        assert!(
            has_web_search,
            "web_search tool should be included when supports_web_search is true"
        );
    }

    #[test]
    fn test_web_search_tool_not_added_when_unsupported() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new("claude".to_string(), vec![]).with_tools(
            vec![json!({"type": "function", "function": {"name": "test", "parameters": {}}})],
            None,
        );
        let s = settings(); // supports_web_search: false
        let http = adapter.build_http(&req, &s).unwrap();

        let tools = http.body["tools"].as_array().unwrap();
        let has_web_search = tools
            .iter()
            .any(|t| t.get("type").and_then(|v| v.as_str()) == Some("web_search_20250305"));
        assert!(
            !has_web_search,
            "web_search tool should NOT be included when supports_web_search is false"
        );
    }

    #[test]
    fn test_empty_thinking_blocks_filtered_in_convert() {
        // Thinking blocks with empty/missing thinking text must be filtered out,
        // because Anthropic rejects them with "each thinking block must contain thinking".
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText("Response".to_string()),
                thinking_blocks: Some(vec![json!({
                    "type": "thinking",
                    "thinking": "",
                    "signature": "sig_empty"
                })]),
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "Follow up".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        let assistant_content = msgs[1]["content"].as_array().unwrap();
        let thinking_blocks: Vec<_> = assistant_content
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("thinking"))
            .collect();
        assert!(
            thinking_blocks.is_empty(),
            "Empty thinking blocks should be filtered out, got {:?}",
            thinking_blocks
        );
    }

    #[test]
    fn test_whitespace_thinking_blocks_filtered_in_convert() {
        // Whitespace-only thinking text should also be filtered out.
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText("Response".to_string()),
                thinking_blocks: Some(vec![json!({
                    "type": "thinking",
                    "thinking": "   \n\t  ",
                    "signature": "sig_ws"
                })]),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        let assistant_content = msgs[0]["content"].as_array().unwrap();
        let thinking_blocks: Vec<_> = assistant_content
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("thinking"))
            .collect();
        assert!(
            thinking_blocks.is_empty(),
            "Whitespace-only thinking blocks should be filtered out"
        );
    }

    #[test]
    fn test_missing_thinking_field_filtered_in_convert() {
        // Blocks with no "thinking" field at all (only type + signature).
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText("Response".to_string()),
                thinking_blocks: Some(vec![json!({
                    "type": "thinking",
                    "signature": "sig_no_text"
                })]),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        let assistant_content = msgs[0]["content"].as_array().unwrap();
        let thinking_blocks: Vec<_> = assistant_content
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("thinking"))
            .collect();
        assert!(
            thinking_blocks.is_empty(),
            "Thinking blocks without thinking text should be filtered out"
        );
    }

    #[test]
    fn test_valid_thinking_block_kept_empty_filtered_mixed() {
        // Mix of valid and invalid thinking blocks: only valid ones should survive.
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText("Response".to_string()),
                thinking_blocks: Some(vec![
                    json!({
                        "type": "thinking",
                        "thinking": "Valid reasoning",
                        "signature": "sig_valid"
                    }),
                    json!({
                        "type": "thinking",
                        "thinking": "",
                        "signature": "sig_empty"
                    }),
                    json!({
                        "type": "redacted_thinking",
                        "data": "encrypted"
                    }),
                ]),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        // msgs[0] = user, msgs[1] = assistant
        let assistant_content = msgs[1]["content"].as_array().unwrap();
        let thinking_blocks: Vec<_> = assistant_content
            .iter()
            .filter(|b| {
                let t = b.get("type").and_then(|t| t.as_str());
                t == Some("thinking") || t == Some("redacted_thinking")
            })
            .collect();
        assert_eq!(
            thinking_blocks.len(),
            2,
            "Should keep valid thinking + redacted, filter empty: {:?}",
            thinking_blocks
        );
        assert_eq!(thinking_blocks[0]["thinking"], "Valid reasoning");
        assert_eq!(thinking_blocks[1]["type"], "redacted_thinking");
    }

    #[test]
    fn test_interleaved_thinking_and_server_blocks_ordering() {
        // Simulates interleaved thinking with web search:
        // thinking(0) → server_tool_use(1) → web_search_result(2) → thinking(3) → text(4)
        let messages = vec![
            ChatMessage::new("user".to_string(), "Search for X".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText(
                    "Found results.".to_string(),
                ),
                thinking_blocks: Some(vec![
                    json!({
                        "type": "thinking",
                        "thinking": "Let me search for X",
                        "signature": "sig_0",
                        "index": 0
                    }),
                    json!({
                        "type": "thinking",
                        "thinking": "Now I have the results",
                        "signature": "sig_3",
                        "index": 3
                    }),
                ]),
                server_content_blocks: vec![
                    json!({
                        "type": "server_tool_use",
                        "id": "srvtoolu_abc",
                        "name": "web_search",
                        "input": {"query": "X"},
                        "_order_index": 1
                    }),
                    json!({
                        "type": "web_search_tool_result",
                        "tool_use_id": "srvtoolu_abc",
                        "content": [{"type": "web_search_result", "url": "https://example.com", "encrypted_content": "enc"}],
                        "_order_index": 2
                    }),
                ],
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "Tell me more".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        let assistant_content = msgs[1]["content"].as_array().unwrap();
        // Verify interleaved order: thinking(0), server_tool_use(1), web_search_result(2), thinking(3), text(4)
        assert_eq!(
            assistant_content[0]["type"], "thinking",
            "Block 0 should be thinking"
        );
        assert_eq!(assistant_content[0]["thinking"], "Let me search for X");
        assert_eq!(
            assistant_content[1]["type"], "server_tool_use",
            "Block 1 should be server_tool_use"
        );
        assert_eq!(
            assistant_content[2]["type"], "web_search_tool_result",
            "Block 2 should be web_search_tool_result"
        );
        assert_eq!(
            assistant_content[3]["type"], "thinking",
            "Block 3 should be thinking"
        );
        assert_eq!(assistant_content[3]["thinking"], "Now I have the results");
        assert_eq!(
            assistant_content[4]["type"], "text",
            "Block 4 should be text"
        );
        assert_eq!(assistant_content[4]["text"], "Found results.");

        // Verify _order_index is stripped from server content blocks
        assert!(
            assistant_content[1].get("_order_index").is_none(),
            "Internal _order_index should be stripped from server content blocks"
        );
    }

    #[test]
    fn test_server_content_blocks_preserved_for_cache_consistency() {
        // Regression test: When replaying historical messages with complete
        // server_tool_use + web_search_tool_result pairs, ALL blocks must be
        // preserved exactly to maintain cache prefix consistency.
        // Previously, orphan detection would strip blocks on subsequent turns.

        // Simulate a historical assistant message from storage with complete
        // web_search server content blocks
        let mut assistant_msg =
            ChatMessage::new("assistant".to_string(), "Here are the results.".to_string());

        assistant_msg.server_content_blocks = vec![
            json!({
                "type": "server_tool_use",
                "id": "srvtoolu_01ABC",
                "name": "web_search",
                "input": {},
                "_order_index": 1
            }),
            json!({
                "type": "web_search_tool_result",
                "tool_use_id": "srvtoolu_01ABC",
                "content": [{
                    "type": "web_search_result",
                    "title": "Result",
                    "url": "https://example.com",
                    "encrypted_content": "data"
                }],
                "_order_index": 2
            }),
        ];

        let messages = vec![
            ChatMessage::new("user".to_string(), "Search for X".to_string()),
            assistant_msg,
            ChatMessage::new("user".to_string(), "Tell me more".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        // Verify both blocks are preserved in the re-processed message
        let assistant_content = msgs[1]["content"].as_array().unwrap();

        let server_tool_use_count = assistant_content
            .iter()
            .filter(|b| b["type"] == "server_tool_use")
            .count();
        let web_search_result_count = assistant_content
            .iter()
            .filter(|b| b["type"] == "web_search_tool_result")
            .count();

        assert_eq!(
            server_tool_use_count, 1,
            "server_tool_use block must be preserved for cache consistency"
        );
        assert_eq!(
            web_search_result_count, 1,
            "web_search_tool_result block must be preserved for cache consistency"
        );

        // Verify _order_index was stripped (not part of Anthropic wire format)
        for block in assistant_content {
            assert!(
                block.get("_order_index").is_none(),
                "_order_index should be stripped from all blocks"
            );
        }
    }

    #[test]
    fn test_orphaned_server_tool_use_filtered_for_fresh_responses() {
        // Test that orphan filtering still works for incomplete/fresh responses
        // (where server_tool_use exists but matching result is missing)

        let mut assistant_msg =
            ChatMessage::new("assistant".to_string(), "Searching...".to_string());

        // Simulate incomplete response: server_tool_use without matching result
        assistant_msg.server_content_blocks = vec![json!({
            "type": "server_tool_use",
            "id": "srvtoolu_01ORPHAN",
            "name": "web_search",
            "input": {}
        })];

        let messages = vec![
            ChatMessage::new("user".to_string(), "Search for Y".to_string()),
            assistant_msg,
        ];

        let (_, msgs) = convert_to_anthropic(&messages, None);

        // Verify orphaned server_tool_use is filtered out
        let assistant_content = msgs[1]["content"].as_array().unwrap();

        let has_orphaned_block = assistant_content
            .iter()
            .any(|b| b["type"] == "server_tool_use" && b["id"] == "srvtoolu_01ORPHAN");

        assert!(!has_orphaned_block,
            "Orphaned server_tool_use without matching result should be filtered for incomplete responses");
    }

    #[test]
    fn test_convert_tools_to_anthropic_maps_parameters_to_input_schema() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "search",
                "description": "Search the web",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Search query"},
                        "limit": {"type": "integer"}
                    },
                    "required": ["query"]
                }
            }
        })];

        let result = convert_tools_to_anthropic(&tools);
        let converted = result.as_array().unwrap();
        assert_eq!(converted.len(), 1);

        let tool = &converted[0];
        assert_eq!(tool["name"], json!("search"));
        assert_eq!(tool["description"], json!("Search the web"));

        let input_schema = &tool["input_schema"];
        assert_eq!(input_schema["type"], json!("object"));
        assert_eq!(input_schema["properties"]["query"]["type"], json!("string"));
        assert_eq!(
            input_schema["properties"]["limit"]["type"],
            json!("integer")
        );
        assert_eq!(input_schema["required"], json!(["query"]));

        assert!(
            tool.get("parameters").is_none(),
            "parameters field should not be present"
        );
    }

    fn effort_settings() -> AdapterSettings {
        AdapterSettings {
            reasoning_type: Some("anthropic_effort".to_string()),
            ..settings()
        }
    }

    #[test]
    fn test_effort_mode_medium_sends_adaptive() {
        use crate::params::ReasoningIntent;
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude-opus-4-7".to_string(),
            vec![ChatMessage::new("user".to_string(), "hi".to_string())],
        )
        .with_reasoning(ReasoningIntent::Medium);
        let http = adapter.build_http(&req, &effort_settings()).unwrap();
        assert_eq!(http.body["thinking"]["type"], "adaptive");
        assert_eq!(http.body["thinking"]["display"], "summarized");
        assert!(http.body["thinking"].get("budget_tokens").is_none());
        assert_eq!(http.body["output_config"]["effort"], "medium");
        assert!(http.headers.get("anthropic-beta").is_none());
    }

    #[test]
    fn test_effort_mode_budget_tokens_sends_adaptive() {
        use crate::params::ReasoningIntent;
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude-opus-4-7".to_string(),
            vec![ChatMessage::new("user".to_string(), "hi".to_string())],
        )
        .with_reasoning(ReasoningIntent::BudgetTokens(5000));
        let http = adapter.build_http(&req, &effort_settings()).unwrap();
        assert_eq!(http.body["thinking"]["type"], "adaptive");
        assert_eq!(http.body["thinking"]["display"], "summarized");
        assert!(http.body["thinking"].get("budget_tokens").is_none());
        assert_eq!(http.body["output_config"]["effort"], "high");
        assert!(http.headers.get("anthropic-beta").is_none());
    }

    #[test]
    fn test_effort_mode_extra_body_thinking_override_updates_betas() {
        use crate::params::ReasoningIntent;
        use std::collections::HashMap;
        let adapter = AnthropicAdapter;
        let mut extra = HashMap::new();
        extra.insert(
            "thinking".to_string(),
            json!({"type": "enabled", "budget_tokens": 4096}),
        );
        let req = LlmRequest::new(
            "claude-opus-4-7".to_string(),
            vec![ChatMessage::new("user".to_string(), "hi".to_string())],
        )
        .with_reasoning(ReasoningIntent::Medium)
        .with_extra_body(extra);
        let http = adapter.build_http(&req, &effort_settings()).unwrap();
        assert_eq!(http.body["thinking"]["type"], "enabled");
        let beta = http
            .headers
            .get("anthropic-beta")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(beta.contains(INTERLEAVED_THINKING_BETA));
        assert!(!beta.contains("effort-2025-11-24"));
    }
}
