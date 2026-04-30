use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};

use crate::call_validation::ChatUsage;
use crate::llm::adapter::{
    AdapterSettings, HttpParts, LlmWireAdapter, StreamParseError, extract_extra_fields,
    insert_extra_headers,
};
use crate::llm::canonical::{CanonicalToolChoice, LlmRequest, LlmStreamDelta};
use crate::llm::params::CacheControl;
use super::claude_code_compat;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_THINKING_BUDGET: usize = 8192;
const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";

const PROTECTED_FIELDS: &[&str] = &[
    "model",
    "messages",
    "stream",
    "system",
    "tools",
    "tool_choice",
    "metadata",
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
        let is_github_copilot =
            crate::llm::provider_quirks::is_github_copilot_request(req, settings);
        if is_cc {
            claude_code_compat::apply_oauth_headers(&mut headers, &settings.auth_token)?;
            claude_code_compat::apply_stainless_headers(&mut headers)?;
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

        insert_extra_headers(&mut headers, &settings.extra_headers);
        crate::llm::provider_quirks::apply_github_copilot_request_headers(
            &mut headers,
            req,
            settings,
        );

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
                        claude_code_compat::strip_tool_descriptions(&mut converted_tools);
                        claude_code_compat::inject_cc_tool_stubs(&mut converted_tools);
                    }
                    // Add Anthropic's server-side web_search tool if enabled
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

        if matches!(req.cache_control, CacheControl::Ephemeral) && settings.supports_cache_control {
            body["cache_control"] = json!({"type": "ephemeral", "ttl": "1h"});
        }

        if settings.supports_reasoning {
            if is_effort_mode {
                match &req.reasoning {
                    crate::llm::params::ReasoningIntent::BudgetTokens(_n) => {
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
            claude_code_compat::inject_metadata(&mut body);
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

        crate::llm::provider_quirks::remove_anthropic_unsupported_fields(&mut body, settings);

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

        if is_cc {
            if let Some(msgs) = body.get_mut("messages") {
                claude_code_compat::apply_cc_tool_use_in_messages(msgs);
            }
        }

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
                if let Some(delta) = json.get("delta") {
                    if let Some(reason) = delta.get("stop_reason").and_then(|r| r.as_str()) {
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
            }
            "message_stop" => {
                deltas.push(LlmStreamDelta::Done);
            }
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
    messages: &[crate::call_validation::ChatMessage],
    context_sanitizer: Option<&dyn Fn(&str) -> String>,
) -> (Option<Value>, Vec<Value>) {
    use super::render_extra::{is_context_role, render_context_message};

    let mut system_text = None;
    let mut result: Vec<Value> = Vec::new();
    let mut pending_tool_results: Vec<Value> = Vec::new();
    // Context buffered when there are no pending tool results; merged into the
    // next user message to avoid introducing extra consecutive user turns.
    let mut pending_context_text: Vec<String> = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                system_text = Some(msg.content.content_text_only());
            }
            role if is_context_role(role) => {
                let Some(raw_text) = render_context_message(msg) else {
                    continue;
                };
                let text = match context_sanitizer {
                    Some(f) => f(&raw_text),
                    None => raw_text,
                };
                if !pending_tool_results.is_empty() {
                    // Inside a tool-results group: add as a plain text content block
                    // so it is delivered in the same user turn as the tool outputs.
                    pending_tool_results.push(json!({"type": "text", "text": text}));
                } else {
                    // No open tool-results group: buffer for the next user message.
                    pending_context_text.push(text);
                }
            }
            "user" | "assistant" => {
                let mut content = Vec::new();
                // Merge pending tool_results (and any trailing context blocks) into
                // the user message to avoid consecutive user turns.
                if msg.role == "user"
                    && (!pending_tool_results.is_empty() || !pending_context_text.is_empty())
                {
                    content.extend(pending_tool_results.drain(..));
                    for text in pending_context_text.drain(..) {
                        content.push(json!({"type": "text", "text": text}));
                    }
                } else {
                    // Flush any open tool-results group before an assistant turn.
                    if !pending_context_text.is_empty() && pending_tool_results.is_empty() {
                        // Emit buffered context as a standalone user turn so it is
                        // not lost when an assistant message follows without a user.
                        let ctx: Vec<Value> = pending_context_text
                            .drain(..)
                            .map(|t| json!({"type": "text", "text": t}))
                            .collect();
                        result.push(json!({"role": "user", "content": ctx}));
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
                        crate::call_validation::ChatContent::Multimodal(elements)
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
        for text in pending_context_text.drain(..) {
            pending_tool_results.push(json!({"type": "text", "text": text}));
        }
    }
    flush_tool_results(&mut result, &mut pending_tool_results);

    // Claude prompt caching breakpoints are handled on messages (not system).
    let system = system_text.map(|text| json!(text));

    (system, result)
}

fn flush_tool_results(result: &mut Vec<Value>, pending: &mut Vec<Value>) {
    if pending.is_empty() {
        return;
    }
    result.push(json!({
        "role": "user",
        "content": pending.drain(..).collect::<Vec<_>>()
    }));
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

fn msg_content_to_anthropic(content: &crate::call_validation::ChatContent) -> Vec<Value> {
    match content {
        crate::call_validation::ChatContent::SimpleText(text) => vec![json!({"type": "text", "text": text})],
        crate::call_validation::ChatContent::Multimodal(elements) => {
            elements.iter().map(|el| {
                if el.is_image() {
                    json!({"type": "image", "source": {"type": "base64", "media_type": el.m_type, "data": el.m_content}})
                } else {
                    json!({"type": "text", "text": el.m_content})
                }
            }).collect()
        }
        crate::call_validation::ChatContent::ContextFiles(_) => {
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
    use crate::call_validation::ChatMessage;

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
        use crate::call_validation::ChatContent;
        use crate::scratchpads::multimodality::MultimodalElement;

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
        use crate::llm::params::ReasoningIntent;

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
    fn test_top_level_cache_control_ephemeral() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        )
        .with_cache_control(CacheControl::Ephemeral);

        let http = adapter.build_http(&req, &settings()).unwrap();
        assert_eq!(http.body["cache_control"]["type"], "ephemeral");
        assert_eq!(http.body["cache_control"]["ttl"], "1h");
    }

    #[test]
    fn test_top_level_cache_control_omitted_when_unsupported() {
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
    }

    #[test]
    fn test_minimax_unsupported_cache_control_beta_and_tools_remain_absent() {
        use crate::llm::params::ReasoningIntent;
        use std::collections::HashMap;

        let adapter = AnthropicAdapter;
        let mut extra = HashMap::new();
        extra.insert(
            "cache_control".to_string(),
            json!({"type": "ephemeral", "ttl": "1h"}),
        );
        extra.insert(
            "thinking".to_string(),
            json!({"type": "enabled", "budget_tokens": 4096}),
        );
        extra.insert("output_config".to_string(), json!({"effort": "high"}));
        extra.insert("tools".to_string(), json!([{"name": "blocked"}]));
        extra.insert("tool_choice".to_string(), json!({"type": "any"}));

        let req = LlmRequest::new(
            "minimax/MiniMax-M2".to_string(),
            vec![
                ChatMessage::new("user".to_string(), "test".to_string()),
                ChatMessage {
                    role: "assistant".to_string(),
                    content: crate::call_validation::ChatContent::SimpleText("visible".to_string()),
                    thinking_blocks: Some(vec![json!({
                        "type": "thinking",
                        "thinking": "hidden",
                        "signature": "sig"
                    })]),
                    ..Default::default()
                },
            ],
        )
        .with_cache_control(CacheControl::Ephemeral)
        .with_reasoning(ReasoningIntent::High)
        .with_tools(
            vec![json!({
                "type": "function",
                "function": {"name": "tool", "parameters": {}}
            })],
            Some(CanonicalToolChoice::Required),
        )
        .with_extra_body(extra);

        let mut settings = settings();
        settings.model_name = "MiniMax-M2".to_string();
        settings.endpoint = "https://api.minimax.io/anthropic/v1/messages".to_string();
        settings.supports_cache_control = false;
        settings.supports_reasoning = false;
        settings.supports_tools = false;

        let http = adapter.build_http(&req, &settings).unwrap();

        assert!(http.body.get("cache_control").is_none());
        assert!(http.body.get("thinking").is_none());
        assert!(http.body.get("output_config").is_none());
        assert!(http.body.get("tools").is_none());
        assert!(http.body.get("tool_choice").is_none());
        assert!(http.headers.get("anthropic-beta").is_none());
        let messages = http.body["messages"].as_array().unwrap();
        assert!(!messages.iter().any(|message| {
            message["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|block| block["type"] == "thinking")
        }));
    }

    #[test]
    fn test_anthropic_supported_flags_keep_cache_control_tools_and_reasoning() {
        use crate::llm::params::ReasoningIntent;

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

        assert_eq!(http.body["cache_control"]["type"], "ephemeral");
        assert!(http.body.get("tools").is_some());
        assert_eq!(http.body["tool_choice"], json!({"type": "any"}));
        assert_eq!(http.body["thinking"]["type"], "enabled");
        assert!(http.headers.get("anthropic-beta").is_some());
    }

    #[test]
    fn test_no_beta_header_when_reasoning_not_supported() {
        use crate::llm::params::ReasoningIntent;

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
    fn test_system_no_block_level_cache_control() {
        let messages = vec![
            ChatMessage::new("system".to_string(), "Be helpful".to_string()),
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ];
        let (system, msgs) = convert_to_anthropic(&messages, None);
        assert_eq!(system, Some(json!("Be helpful")));
        assert_eq!(msgs.len(), 1);
        // Block-level cache_control is no longer injected by the adapter
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
        use crate::call_validation::{ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Do two things".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("".to_string()),
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
                content: crate::call_validation::ChatContent::SimpleText("Result A".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Result B".to_string()),
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
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

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
    fn test_diff_role_as_tool_result() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Edit file".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![crate::call_validation::ChatToolCall {
                    id: "call_edit".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: crate::call_validation::ChatToolFunction {
                        name: "file_edit".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "diff".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("@@ -1 +1 @@".to_string()),
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
        use crate::llm::adapter::LlmWireAdapter;
        use crate::llm::params::ReasoningIntent;

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
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

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
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

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
                content: crate::call_validation::ChatContent::SimpleText(
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
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

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
                content: crate::call_validation::ChatContent::SimpleText("Response".to_string()),
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
                content: crate::call_validation::ChatContent::SimpleText(
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
            content: crate::call_validation::ChatContent::SimpleText("Hello".to_string()),
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
                content: crate::call_validation::ChatContent::SimpleText("Hi there".to_string()),
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
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

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
                content: crate::call_validation::ChatContent::SimpleText("Found it.".to_string()),
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
                content: crate::call_validation::ChatContent::SimpleText("It's sunny.".to_string()),
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
                content: crate::call_validation::ChatContent::SimpleText("Response".to_string()),
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
                content: crate::call_validation::ChatContent::SimpleText("Response".to_string()),
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
                content: crate::call_validation::ChatContent::SimpleText("Response".to_string()),
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
                content: crate::call_validation::ChatContent::SimpleText("Response".to_string()),
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
                content: crate::call_validation::ChatContent::SimpleText(
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

    #[test]
    fn test_context_file_sanitized_in_cc_mode() {
        use crate::call_validation::{ChatContent, ContextFile};
        use super::claude_code_compat;

        // Simulate an AGENTS.md context file injection
        let agents_md_content = "# Refact Agent Engine\nBinary: `refact-lsp` — AI coding agent.\nConfig: `.refact/integrations/*.yaml`.";
        let messages = vec![
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(vec![ContextFile {
                    file_name: "AGENTS.md".to_string(),
                    file_content: agents_md_content.to_string(),
                    line1: 1,
                    line2: 3,
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "What tools do I have?".to_string()),
        ];

        // Non-CC mode: content passes through unsanitized
        let (_, msgs_plain) = convert_to_anthropic(&messages, None);
        let user_content = msgs_plain[0]["content"].as_array().unwrap();
        let has_context = user_content.iter().any(|b| {
            b.get("text")
                .and_then(|t| t.as_str())
                .map_or(false, |t| t.contains("refact-lsp"))
        });
        assert!(
            has_context,
            "Non-CC mode should pass context files through unchanged"
        );

        // CC mode: identifying strings are sanitized
        let sanitizer: Box<dyn Fn(&str) -> String> =
            Box::new(|text: &str| claude_code_compat::sanitize_system_text(text));
        let (_, msgs_cc) = convert_to_anthropic(&messages, Some(sanitizer.as_ref()));
        let user_content_cc = msgs_cc[0]["content"].as_array().unwrap();
        let has_refact_lsp = user_content_cc.iter().any(|b| {
            b.get("text")
                .and_then(|t| t.as_str())
                .map_or(false, |t| t.contains("refact-lsp"))
        });
        assert!(
            !has_refact_lsp,
            "CC mode should sanitize refact-lsp from context files"
        );
        let has_refact_dir = user_content_cc.iter().any(|b| {
            b.get("text")
                .and_then(|t| t.as_str())
                .map_or(false, |t| t.contains(".refact/"))
        });
        assert!(
            !has_refact_dir,
            "CC mode should sanitize .refact/ from context files"
        );
        // Content should still be present (just sanitized)
        let has_content = user_content_cc.iter().any(|b| {
            b.get("text")
                .and_then(|t| t.as_str())
                .map_or(false, |t| t.contains("AGENTS.md"))
        });
        assert!(
            has_content,
            "CC mode sanitization should preserve file header"
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
        use crate::llm::params::ReasoningIntent;
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
        use crate::llm::params::ReasoningIntent;
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
        use crate::llm::params::ReasoningIntent;
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
