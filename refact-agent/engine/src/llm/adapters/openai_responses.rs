use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};

use crate::call_validation::ChatUsage;
use crate::llm::adapter::{
    AdapterSettings, HttpParts, LlmWireAdapter, StreamParseError, extract_extra_fields,
    insert_extra_headers,
};
use crate::llm::canonical::{CanonicalToolChoice, LlmRequest, LlmStreamDelta, ResponseFormat};

/// Fields that cannot be overridden via extra_body for security
const PROTECTED_FIELDS: &[&str] = &[
    "model",
    "input",
    "stream",
    "tools",
    "tool_choice",
    "instructions",
    "include",
    "store",
    "previous_response_id",
    "conversation",
    "top_p",
];

pub struct OpenAiResponsesAdapter;

const CHATGPT_BACKEND_DEFAULT_INSTRUCTIONS: &str = "You are a helpful assistant.";

const ALL_INCLUDE_FIELDS: &[&str] = &[
    // Tool outputs / results
    "web_search_call.results",
    "web_search_call.action.sources",
    "file_search_call.results",
    "code_interpreter_call.outputs",
    "computer_call_output.output.image_url",
    // Message extras
    "message.input_image.image_url",
];

impl LlmWireAdapter for OpenAiResponsesAdapter {
    fn build_http(
        &self,
        req: &LlmRequest,
        settings: &AdapterSettings,
    ) -> Result<HttpParts, String> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if !settings.api_key.is_empty() {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", settings.api_key))
                    .map_err(|e| format!("invalid api_key for header: {e}"))?,
            );
        }
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&format!("refact-lsp {}", env!("CARGO_PKG_VERSION")))
                .unwrap_or_else(|_| HeaderValue::from_static("refact-lsp")),
        );

        insert_extra_headers(&mut headers, &settings.extra_headers);
        crate::llm::provider_quirks::apply_github_copilot_request_headers(
            &mut headers,
            req,
            settings,
        );

        let (input, instructions) = convert_to_responses_format(&req.messages);
        let mut body = json!({
            "model": settings.model_name,
            "stream": req.stream,
        });

        // ChatGPT backend (Codex-style) only accepts a minimal set of fields:
        // model, instructions, input, tools, tool_choice, parallel_tool_calls,
        // reasoning, store, stream, include, text.
        // It rejects max_output_tokens, temperature, frequency_penalty, stop, etc.
        let is_chatgpt_backend = settings.endpoint.contains("chatgpt.com/backend-api");

        // ChatGPT backend rejects store=true; Platform API needs it for previous_response_id chaining.
        body["store"] = json!(!is_chatgpt_backend);

        if is_chatgpt_backend {
            // ChatGPT backend rejects most sampling params.
        } else {
            body["max_output_tokens"] = json!(req.params.max_tokens);

            if settings.supports_temperature {
                if let Some(temp) = req.params.temperature {
                    body["temperature"] = json!(temp);
                }

                if let Some(top_p) = req.params.top_p {
                    body["top_p"] = json!(top_p);
                }
            }

            if let Some(freq_penalty) = req.params.frequency_penalty {
                body["frequency_penalty"] = json!(freq_penalty);
            }
        }

        if !input.is_null() {
            // ChatGPT backend uses store=false, so reasoning items (rs_*) aren't
            // persisted server-side. Sending them back causes 404 "Item not found".
            if is_chatgpt_backend {
                if let Some(arr) = input.as_array() {
                    let filtered: Vec<_> = arr
                        .iter()
                        .filter(|item| {
                            item.get("type").and_then(|t| t.as_str()) != Some("reasoning")
                        })
                        .cloned()
                        .collect();
                    body["input"] = json!(filtered);
                } else {
                    body["input"] = input;
                }
            } else {
                body["input"] = input;
            }
        }
        match instructions.as_deref() {
            Some(inst) if !inst.trim().is_empty() => {
                body["instructions"] = json!(inst);
            }
            _ if is_chatgpt_backend => {
                body["instructions"] = json!(CHATGPT_BACKEND_DEFAULT_INSTRUCTIONS);
            }
            _ => {}
        }

        if !is_chatgpt_backend {
            if let Some(prev) = &req.previous_response_id {
                if !prev.is_empty() {
                    body["previous_response_id"] = json!(prev);
                }
            }
        }

        if settings.supports_tools {
            if let Some(tools) = &req.tools {
                if !tools.is_empty() {
                    body["tools"] = convert_tools_to_responses(tools);
                    if let Some(choice) = &req.tool_choice {
                        body["tool_choice"] = tool_choice_to_responses(choice);
                    }
                    if req.parallel_tool_calls {
                        body["parallel_tool_calls"] = json!(true);
                    }
                }
            }
        } else if req.tools.is_some() {
            tracing::warn!(
                "model {} does not support tools, skipping tools in request",
                settings.model_name
            );
        }

        if settings.supports_reasoning {
            if let Some(effort) = req.reasoning.to_openai_effort() {
                body["reasoning"] = json!({"effort": effort, "summary": "auto"});
            }
            body.as_object_mut().map(|obj| obj.remove("temperature"));
            body.as_object_mut().map(|obj| obj.remove("top_p"));
        }

        // Ask server to include extra fields we rely on for rich tool cards.
        // ChatGPT backend rejects `include`; only send on Platform API.
        if !is_chatgpt_backend {
            let mut include_fields: Vec<&str> = ALL_INCLUDE_FIELDS.to_vec();
            if settings.supports_reasoning {
                include_fields.push("reasoning.encrypted_content");
            }
            body["include"] = json!(include_fields);
        }

        if !is_chatgpt_backend && !req.params.stop.is_empty() {
            body["stop"] = json!(req.params.stop);
        }

        if let Some(ref response_format) = req.response_format {
            body["text"] = response_format_to_responses(response_format);
        }

        if let Some(extra) = &req.extra_body {
            // Fields the ChatGPT backend rejects — block even via extra_body.
            const CHATGPT_REJECTED: &[&str] = &[
                "max_output_tokens",
                "max_tokens",
                "max_completion_tokens",
                "temperature",
                "top_p",
                "frequency_penalty",
                "stop",
            ];
            if let Some(obj) = body.as_object_mut() {
                for (k, v) in extra {
                    if PROTECTED_FIELDS.contains(&k.as_str()) {
                        tracing::warn!(
                            "extra_body attempted to override protected field '{}', ignoring",
                            k
                        );
                        continue;
                    }
                    if is_chatgpt_backend && CHATGPT_REJECTED.contains(&k.as_str()) {
                        tracing::warn!(
                            "extra_body field '{}' rejected by ChatGPT backend, ignoring",
                            k
                        );
                        continue;
                    }
                    obj.insert(k.clone(), v.clone());
                }
            }
        }

        tracing::info!(
            model = %settings.model_name,
            endpoint = %settings.endpoint,
            stream = %req.stream,
            max_tokens = %req.params.max_tokens,
            temperature = ?req.params.temperature,
            frequency_penalty = ?req.params.frequency_penalty,
            stop_sequences = ?req.params.stop.len(),
            tools_count = ?req.tools.as_ref().map(|t| t.len()),
            tool_choice = ?req.tool_choice,
            reasoning = ?req.reasoning,
            response_format = ?req.response_format.is_some(),
            messages_count = %req.messages.len(),
            "openai responses adapter request"
        );

        Ok(HttpParts {
            url: settings.endpoint.clone(),
            headers,
            body,
        })
    }

    fn parse_stream_chunk(&self, data: &str) -> Result<Vec<LlmStreamDelta>, StreamParseError> {
        let trimmed = data.trim();

        if trimmed.is_empty() {
            return Err(StreamParseError::Skip);
        }

        let json: Value = serde_json::from_str(trimmed)
            .map_err(|e| StreamParseError::MalformedChunk(format!("json parse: {e}")))?;

        let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let mut deltas = Vec::new();

        // IMPORTANT: Some Responses events (e.g. MCP/tool failures) legitimately carry an `error` field.
        // Only treat errors as fatal when the event itself is a fatal error lifecycle event.
        if let Some(error) = json.get("error").filter(|e| !e.is_null()) {
            if event_type == "error" {
                return Err(StreamParseError::FatalError(
                    error
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown error")
                        .to_string(),
                ));
            }
        }

        match event_type {
            // ── Response lifecycle (extract ID only, no server content block) ──
            "response.created" | "response.queued" | "response.in_progress" => {
                if let Some(resp_id) = json
                    .get("response")
                    .and_then(|r| r.get("id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    let mut extra = serde_json::Map::new();
                    extra.insert("openai_response_id".to_string(), json!(resp_id));
                    deltas.push(LlmStreamDelta::MergeExtra { extra });
                }
            }

            // ── Text content streaming ──
            "response.output_text.delta" => {
                if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                    deltas.push(LlmStreamDelta::AppendContent {
                        text: delta.to_string(),
                        block_index: None,
                    });
                }
            }

            // Text already accumulated via output_text.delta — skip redundant .done
            "response.output_text.done" => {
                tracing::trace!("output_text.done (redundant, text already streamed via deltas)");
            }

            // ── Reasoning streaming (3 flavours) ──
            // 1. Legacy: response.reasoning.delta (older models)
            // 2. GPT-OSS: response.reasoning_text.delta (reasoning content)
            // 3. Summary: response.reasoning_summary_text.delta (shareable summary)
            "response.reasoning.delta"
            | "response.reasoning_text.delta"
            | "response.reasoning_summary_text.delta" => {
                if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                    deltas.push(LlmStreamDelta::AppendReasoning {
                        text: delta.to_string(),
                        block_index: None,
                    });
                }
            }

            // Reasoning summary lifecycle events — content already streamed via *.delta above
            "response.reasoning_summary_part.added" => {
                tracing::trace!(
                    "reasoning_summary_part.added (summary part opened, text arrives via delta)"
                );
            }
            "response.reasoning_summary_part.done" => {
                tracing::trace!(
                    "reasoning_summary_part.done (redundant, text already streamed via deltas)"
                );
            }

            // ── Refusal streaming ──
            "response.refusal.delta" => {
                if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                    deltas.push(LlmStreamDelta::AppendContent {
                        text: delta.to_string(),
                        block_index: None,
                    });
                }
            }

            // ── Built-in tool lifecycle events (preserve as server content blocks) ──
            "response.web_search_call.searching"
            | "response.web_search_call.in_progress"
            | "response.web_search_call.completed"
            | "response.file_search_call.searching"
            | "response.file_search_call.in_progress"
            | "response.file_search_call.completed"
            | "response.code_interpreter_call.interpreting"
            | "response.code_interpreter_call.in_progress"
            | "response.code_interpreter_call.completed"
            | "response.image_generation_call.generating"
            | "response.image_generation_call.in_progress"
            | "response.image_generation_call.completed"
            | "response.image_generation_call.partial_image"
            | "response.audio.delta"
            | "response.audio.done"
            | "response.audio.transcript.delta"
            | "response.audio.transcript.done"
            | "response.code_interpreter_call_code.delta"
            | "response.code_interpreter_call_code.done"
            | "response.custom_tool_call_input.delta"
            | "response.custom_tool_call_input.done"
            | "response.mcp_call_arguments.delta"
            | "response.mcp_call_arguments.done"
            | "response.mcp_call.in_progress"
            | "response.mcp_call.completed"
            | "response.mcp_call.failed"
            | "response.mcp_list_tools.in_progress"
            | "response.mcp_list_tools.completed"
            | "response.mcp_list_tools.failed" => {
                deltas.push(LlmStreamDelta::AddServerContentBlock {
                    block: json!({
                        "type": event_type,
                        "payload": json,
                    }),
                });
            }

            // ── Redundant lifecycle/done events — data already captured by deltas ──
            "response.reasoning_text.done"
            | "response.reasoning_summary_text.done"
            | "response.refusal.done"
            | "response.content_part.added"
            | "response.content_part.done"
            | "keepalive" => {
                tracing::trace!("{} (redundant/lifecycle, skipping)", event_type);
            }

            // ── Text annotations (citations in output text) ──
            "response.output_text.annotation.added" => {
                if let Some(annotation) = json.get("annotation") {
                    deltas.push(LlmStreamDelta::AddCitation {
                        citation: annotation.clone(),
                    });
                }
            }

            // ── Function call / output item start ──
            "response.output_item.added" => {
                if let Some(item) = json.get("item") {
                    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match item_type {
                        "function_call" => {
                            if let Some(tc) = extract_tool_call_from_item(item, &json) {
                                deltas.push(LlmStreamDelta::SetToolCalls {
                                    tool_calls: vec![tc],
                                });
                            }
                        }
                        // Server-executed tools: register as srvtoolu_ but no content block yet
                        // (the real content arrives in output_item.done)
                        "web_search_call"
                        | "file_search_call"
                        | "code_interpreter_call"
                        | "computer_call"
                        | "image_generation_call"
                        | "mcp_call" => {
                            if let Some(tc) = extract_server_tool_call_from_output_item(item, &json)
                            {
                                deltas.push(LlmStreamDelta::SetToolCalls {
                                    tool_calls: vec![tc],
                                });
                            }
                        }
                        // reasoning, message, etc. — just "starting", no useful data yet
                        _ => {
                            tracing::trace!(
                                "output_item.added type={} (no content yet)",
                                item_type
                            );
                        }
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(tc) = extract_tool_call_delta(&json) {
                    deltas.push(LlmStreamDelta::SetToolCalls {
                        tool_calls: vec![tc],
                    });
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(tc) = extract_tool_call_final(&json) {
                    deltas.push(LlmStreamDelta::FinalizeToolCalls {
                        tool_calls: vec![tc],
                    });
                }
            }

            // ── Output item completion ──
            "response.output_item.done" => {
                if let Some(item) = json.get("item") {
                    let item_type = item.get("type").and_then(|t| t.as_str());
                    match item_type {
                        Some("function_call") => {
                            if let Some(tc) = extract_tool_call_from_item(item, &json) {
                                deltas.push(LlmStreamDelta::FinalizeToolCalls {
                                    tool_calls: vec![tc],
                                });
                            }
                        }
                        Some("web_search_call") => {
                            deltas.push(LlmStreamDelta::AddServerContentBlock {
                                block: json!({
                                    "type": "web_search_call",
                                    "payload": item,
                                }),
                            });
                            if let Some(results) = item.get("results").and_then(|r| r.as_array()) {
                                for result in results {
                                    deltas.push(LlmStreamDelta::AddCitation {
                                        citation: result.clone(),
                                    });
                                }
                            }
                        }
                        Some("reasoning") => {
                            // Capture opaque reasoning items (id + encrypted_content)
                            // for multi-turn tool-calling flows.
                            deltas.push(LlmStreamDelta::SetThinkingBlocks {
                                blocks: vec![item.clone()],
                            });
                        }
                        // Server-executed tools with results — emit content block
                        Some("file_search_call")
                        | Some("code_interpreter_call")
                        | Some("computer_call")
                        | Some("computer_call_output")
                        | Some("image_generation_call")
                        | Some("audio") => {
                            if let Some(tc) = extract_server_tool_call_from_output_item(item, &json)
                            {
                                deltas.push(LlmStreamDelta::FinalizeToolCalls {
                                    tool_calls: vec![tc],
                                });
                            }
                            deltas.push(LlmStreamDelta::AddServerContentBlock {
                                block: json!({
                                    "type": item_type.unwrap_or("output_item"),
                                    "payload": item,
                                }),
                            });
                        }
                        // message, output_text, refusal — content already streamed via deltas
                        Some("message") | Some("output_text") | Some("refusal") => {
                            tracing::trace!(
                                "output_item.done type={:?} (redundant, content already streamed)",
                                item_type
                            );
                        }
                        _ => {
                            deltas.push(LlmStreamDelta::AddServerContentBlock {
                                block: json!({
                                    "type": event_type,
                                    "payload": json,
                                }),
                            });
                        }
                    }
                }
            }

            // ── Incomplete response (hit max_output_tokens or content_filter) ──
            "response.incomplete" => {
                let finish_reason = json
                    .get("response")
                    .and_then(|r| r.get("incomplete_details"))
                    .and_then(|d| d.get("reason"))
                    .and_then(|r| r.as_str())
                    .map(|s| match s {
                        "max_output_tokens" => "length",
                        "content_filter" => "content_filter",
                        other => other,
                    })
                    .unwrap_or("length");
                deltas.push(LlmStreamDelta::SetFinishReason {
                    reason: finish_reason.to_string(),
                });
                if let Some(usage) = extract_usage(&json) {
                    deltas.push(LlmStreamDelta::SetUsage { usage });
                }
                deltas.push(LlmStreamDelta::Done);
            }

            // ── Completed response ──
            "response.completed" => {
                if let Some(resp_id) = json
                    .get("response")
                    .and_then(|r| r.get("id"))
                    .and_then(|v| v.as_str())
                {
                    let mut extra = serde_json::Map::new();
                    extra.insert("openai_response_id".to_string(), json!(resp_id));
                    deltas.push(LlmStreamDelta::MergeExtra { extra });
                }

                let raw_status = json
                    .get("response")
                    .and_then(|r| r.get("status"))
                    .and_then(|s| s.as_str())
                    .or_else(|| json.get("status").and_then(|s| s.as_str()));
                let finish_reason = raw_status
                    .map(|s| match s {
                        "completed" => "stop",
                        "cancelled" => "stop",
                        "failed" => "error",
                        "incomplete" => "length",
                        other => other,
                    })
                    .unwrap_or("stop");
                tracing::info!(
                    "response.completed: raw_status={:?}, finish_reason={}",
                    raw_status,
                    finish_reason
                );
                deltas.push(LlmStreamDelta::SetFinishReason {
                    reason: finish_reason.to_string(),
                });
                if let Some(usage) = extract_usage(&json) {
                    deltas.push(LlmStreamDelta::SetUsage { usage });
                }
                // Safety net: extract tool calls and reasoning from response.output[]
                // in case output_item.done events were missed during streaming.
                if let Some(output) = json
                    .get("response")
                    .and_then(|r| r.get("output"))
                    .and_then(|o| o.as_array())
                {
                    let output_types: Vec<_> = output
                        .iter()
                        .map(|item| {
                            item.get("type")
                                .and_then(|t| t.as_str())
                                .unwrap_or("unknown")
                        })
                        .collect();
                    tracing::info!("response.completed output items: {:?}", output_types);
                    for (idx, item) in output.iter().enumerate() {
                        let item_type = item.get("type").and_then(|t| t.as_str());
                        match item_type {
                            Some("function_call") => {
                                let event_wrapper = json!({"output_index": idx});
                                if let Some(tc) = extract_tool_call_from_item(item, &event_wrapper)
                                {
                                    deltas.push(LlmStreamDelta::FinalizeToolCalls {
                                        tool_calls: vec![tc],
                                    });
                                }
                            }
                            Some("reasoning") => {
                                deltas.push(LlmStreamDelta::SetThinkingBlocks {
                                    blocks: vec![item.clone()],
                                });
                            }
                            // Rehydrate server-executed tool cards in case we missed output_item.done events.
                            Some(
                                "web_search_call"
                                | "file_search_call"
                                | "code_interpreter_call"
                                | "computer_call"
                                | "computer_call_output"
                                | "image_generation_call"
                                | "audio"
                                | "mcp_call"
                                | "mcp_list_tools",
                            ) => {
                                let event_wrapper = json!({"output_index": idx});
                                if let Some(tc) =
                                    extract_server_tool_call_from_output_item(item, &event_wrapper)
                                {
                                    deltas.push(LlmStreamDelta::FinalizeToolCalls {
                                        tool_calls: vec![tc],
                                    });
                                }
                                deltas.push(LlmStreamDelta::AddServerContentBlock {
                                    block: json!({
                                        "type": item_type.unwrap_or("output_item"),
                                        "payload": item,
                                    }),
                                });
                            }
                            // message, output_text, refusal — already streamed via deltas
                            Some("message") | Some("output_text") | Some("refusal") => {}
                            _ => {}
                        }
                    }
                }
                deltas.push(LlmStreamDelta::Done);
            }
            // ── Error events ──
            "response.failed" => {
                let error_msg = json
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .or_else(|| {
                        json.get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                    })
                    .unwrap_or("response failed");
                return Err(StreamParseError::FatalError(error_msg.to_string()));
            }
            "error" => {
                let error_msg = json
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("stream error");
                return Err(StreamParseError::FatalError(error_msg.to_string()));
            }

            // ── Unhandled events — preserve raw payload and make it visible ──
            _ => {
                tracing::warn!("Unhandled Responses API event: {}", event_type);
                // Keep an append-only array in extra (don't overwrite prior events).
                // We still emit a visible server_content_block for every event.
                let mut extra = serde_json::Map::new();
                extra.insert(
                    "unhandled_openai_responses_events".to_string(),
                    json!([
                        {
                            "sequence_number": json.get("sequence_number").cloned().unwrap_or(Value::Null),
                            "event_type": event_type,
                            "payload": json,
                        }
                    ]),
                );
                deltas.push(LlmStreamDelta::MergeExtra { extra });
                deltas.push(LlmStreamDelta::AddServerContentBlock {
                    block: json!({
                        "type": "unhandled_openai_responses_event",
                        "event_type": event_type,
                        "payload": json,
                    }),
                });
            }
        }

        // Extract Refact-specific extra fields on ALL events consistently
        // This handles both top-level and nested "response" wrapper fields
        let extra = extract_extra_fields(&json);
        if !extra.is_empty() {
            deltas.push(LlmStreamDelta::MergeExtra { extra });
        }

        Ok(deltas)
    }
}

fn convert_to_responses_format(
    messages: &[crate::call_validation::ChatMessage],
) -> (Value, Option<String>) {
    use super::render_extra::{is_context_role, render_context_message};

    let mut instructions = None;
    let mut input_messages: Vec<Value> = Vec::new();
    let mut system_count = 0;
    // Unified buffer of Responses API content blocks to inject into the next user turn.
    // Text-context blocks ({"type":"input_text",...}) and images deferred from tool
    // results ({"type":"input_image",...}) both accumulate here.
    let mut pending_user_content: Vec<Value> = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" | "developer" => {
                system_count += 1;
                if system_count > 1 {
                    tracing::warn!(
                        "Multiple instruction messages detected ({}), only the last one will be used",
                        system_count
                    );
                }
                instructions = Some(msg.content.content_text_only());
            }
            role if is_context_role(role) => {
                let Some(text) = render_context_message(msg) else {
                    continue;
                };
                // Fold into the matching function_call_output by call_id when possible
                // so the model receives file content as part of the correct tool output.
                // Fall back to the last function_call_output if tool_call_id is absent.
                let target = if !msg.tool_call_id.is_empty() {
                    input_messages.iter_mut().rev().find(|m| {
                        m["type"].as_str() == Some("function_call_output")
                            && m["call_id"].as_str() == Some(msg.tool_call_id.as_str())
                    })
                } else {
                    input_messages
                        .last_mut()
                        .filter(|m| m["type"].as_str() == Some("function_call_output"))
                };
                if let Some(item) = target {
                    let existing = item["output"].as_str().unwrap_or("").to_string();
                    item["output"] = json!(if existing.is_empty() {
                        text
                    } else {
                        format!("{}\n\n{}", existing, text)
                    });
                } else {
                    pending_user_content.push(json!({"type": "input_text", "text": text}));
                }
            }
            "user" => {
                let mut content = msg_content_to_responses(&msg.content);
                // Prepend pending blocks (context text + deferred tool images).
                if !pending_user_content.is_empty() {
                    content = [std::mem::take(&mut pending_user_content), content].concat();
                }
                input_messages.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": content
                }));
            }
            "assistant" => {
                // Flush pending user content before an assistant turn so ordering is preserved.
                if !pending_user_content.is_empty() {
                    input_messages.push(json!({
                        "type": "message",
                        "role": "user",
                        "content": std::mem::take(&mut pending_user_content),
                    }));
                }
                // Re-send reasoning items from prior turns for multi-turn tool-calling.
                // OpenAI Responses API reasoning items are opaque JSON with type="reasoning",
                // and must be included in input[] for the model to continue its reasoning.
                if let Some(blocks) = &msg.thinking_blocks {
                    for block in blocks {
                        if block.get("type").and_then(|t| t.as_str()) == Some("reasoning") {
                            input_messages.push(block.clone());
                        }
                    }
                }
                let text_content = msg.content.content_text_only();
                if !text_content.is_empty() {
                    input_messages.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": text_content
                    }));
                }
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        if !tc.id.starts_with("srvtoolu_") {
                            input_messages.push(json!({
                                "type": "function_call",
                                "call_id": tc.id,
                                "name": tc.function.name,
                                "arguments": tc.function.arguments
                            }));
                        }
                    }
                }
            }
            "tool" | "diff" => {
                if !msg.tool_call_id.starts_with("srvtoolu_") {
                    input_messages.push(json!({
                        "type": "function_call_output",
                        "call_id": msg.tool_call_id,
                        "output": msg.content.content_text_only()
                    }));

                    if let crate::call_validation::ChatContent::Multimodal(elements) = &msg.content
                    {
                        for el in elements.iter().filter(|el| el.is_image()) {
                            pending_user_content.push(json!({
                                "type": "input_image",
                                "image_url": format!("data:{};base64,{}", el.m_type, el.m_content)
                            }));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if !pending_user_content.is_empty() {
        input_messages.push(json!({
            "type": "message",
            "role": "user",
            "content": pending_user_content,
        }));
    }

    let input = if input_messages.is_empty() {
        Value::Null
    } else {
        json!(input_messages)
    };

    (input, instructions)
}

fn msg_content_to_responses(content: &crate::call_validation::ChatContent) -> Vec<Value> {
    match content {
        crate::call_validation::ChatContent::SimpleText(text) => {
            vec![json!({"type": "input_text", "text": text})]
        }
        crate::call_validation::ChatContent::Multimodal(elements) => elements
            .iter()
            .map(|el| {
                if el.is_image() {
                    json!({
                        "type": "input_image",
                        "image_url": format!("data:{};base64,{}", el.m_type, el.m_content)
                    })
                } else {
                    json!({"type": "input_text", "text": el.m_content})
                }
            })
            .collect(),
        crate::call_validation::ChatContent::ContextFiles(_) => {
            vec![json!({"type": "input_text", "text": content.content_text_only()})]
        }
    }
}

fn convert_tools_to_responses(tools: &[Value]) -> Value {
    let converted: Vec<Value> = tools
        .iter()
        .filter_map(|tool| {
            // Chat Completions format: {"type":"function","function":{"name":...,"parameters":...}}
            if let Some(func) = tool.get("function") {
                return Some(json!({
                    "type": "function",
                    "name": func.get("name")?,
                    "description": func.get("description").unwrap_or(&json!("")),
                    "parameters": func.get("parameters").unwrap_or(&json!({})),
                    // Responses API defaults strict=true, which causes the model to fill optional
                    // parameters with empty strings "" instead of omitting them. Pass through the
                    // strict value from the original tool definition, defaulting to false so that
                    // optional params are simply absent (matching Chat Completions behavior).
                    "strict": func.get("strict").unwrap_or(&json!(false))
                }));
            }
            // Already in Responses API format (has "type" + "name" but no "function" wrapper)
            if tool.get("type").is_some() && tool.get("name").is_some() {
                return Some(tool.clone());
            }
            tracing::warn!("Dropping unrecognized tool shape: {}", tool);
            None
        })
        .collect();
    json!(converted)
}

fn tool_choice_to_responses(choice: &CanonicalToolChoice) -> Value {
    match choice {
        CanonicalToolChoice::Auto => json!("auto"),
        CanonicalToolChoice::None => json!("none"),
        CanonicalToolChoice::Required => json!("required"),
        CanonicalToolChoice::Function { name } => json!({"type": "function", "name": name}),
    }
}

fn response_format_to_responses(format: &ResponseFormat) -> Value {
    match format {
        ResponseFormat::Text => json!({"format": {"type": "text"}}),
        ResponseFormat::JsonObject => json!({"format": {"type": "json_object"}}),
        ResponseFormat::JsonSchema {
            name,
            description,
            schema,
            strict,
        } => {
            let mut json_schema = json!({
                "type": "json_schema",
                "name": name,
                "schema": schema,
                "strict": strict,
            });
            if let Some(desc) = description {
                json_schema["description"] = json!(desc);
            }
            json!({"format": json_schema})
        }
    }
}

fn value_to_arguments_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn positive_usize(value: Option<&Value>) -> Option<usize> {
    value
        .and_then(|t| t.as_u64())
        .filter(|&v| v > 0)
        .map(|v| v as usize)
}

fn extract_tool_call_from_item(item: &Value, event: &Value) -> Option<Value> {
    let call_id = item.get("call_id")?;
    let output_index = event
        .get("output_index")
        .and_then(|i| i.as_u64())
        .unwrap_or(0);
    let arguments = item
        .get("arguments")
        .map(value_to_arguments_string)
        .unwrap_or_default();
    Some(json!({
        "index": output_index,
        "id": call_id,
        "type": "function",
        "function": {
            "name": item.get("name"),
            "arguments": arguments
        }
    }))
}

fn extract_tool_call_final(json: &Value) -> Option<Value> {
    let output_index = json
        .get("output_index")
        .and_then(|i| i.as_u64())
        .unwrap_or(0);
    let call_id = json
        .get("call_id")
        .or_else(|| json.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            json.get("item_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| format!("pending_call_{}", output_index));
    let name = json.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let arguments = json
        .get("arguments")
        .map(value_to_arguments_string)
        .unwrap_or_default();
    Some(json!({
        "index": output_index,
        "id": call_id,
        "type": "function",
        "function": {
            "name": name,
            "arguments": arguments
        }
    }))
}

fn extract_tool_call_delta(json: &Value) -> Option<Value> {
    let output_index = json
        .get("output_index")
        .and_then(|i| i.as_u64())
        .unwrap_or(0);
    let arguments = json.get("delta").map(value_to_arguments_string)?;
    Some(json!({
        "index": output_index,
        "type": "function",
        "function": {
            "arguments": arguments
        }
    }))
}

fn extract_server_tool_call_from_output_item(item: &Value, event: &Value) -> Option<Value> {
    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if item_type.is_empty() {
        return None;
    }

    let output_index = event
        .get("output_index")
        .and_then(|i| i.as_u64())
        .unwrap_or(0);

    let item_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");

    // Must start with srvtoolu_ so chat/tools.rs treats it as server-executed.
    let id = if !item_id.is_empty() {
        format!("srvtoolu_{}", item_id)
    } else {
        format!("srvtoolu_output_{}", output_index)
    };

    // Keep the entire output item as JSON in arguments so GUI can show it
    // and so we can round-trip/debug.
    let arguments = serde_json::to_string(item).unwrap_or_else(|_| "{}".to_string());

    Some(json!({
        "index": output_index,
        "id": id,
        "type": "function",
        "function": {
            "name": format!("openai_{}", item_type),
            "arguments": arguments,
        }
    }))
}

fn extract_usage(json: &Value) -> Option<ChatUsage> {
    let usage = json
        .get("usage")
        .or_else(|| json.get("response").and_then(|r| r.get("usage")))?;
    let raw_prompt_tokens = usage
        .get("input_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as usize;
    let completion_tokens = usage
        .get("output_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as usize;

    let cache_creation = positive_usize(usage.get("cache_creation_input_tokens"));
    let top_level_cache_read = positive_usize(usage.get("cache_read_input_tokens"));
    let details_cache_read = usage
        .get("input_tokens_details")
        .or_else(|| usage.get("prompt_tokens_details"))
        .and_then(|details| positive_usize(details.get("cached_tokens")));
    let cache_read = top_level_cache_read.or(details_cache_read);

    let prompt_tokens = raw_prompt_tokens
        .saturating_sub(cache_read.unwrap_or(0))
        .saturating_sub(cache_creation.unwrap_or(0));
    let total_tokens = usage
        .get("total_tokens")
        .and_then(|t| t.as_u64())
        .map(|t| t as usize)
        .unwrap_or_else(|| {
            prompt_tokens
                + completion_tokens
                + cache_creation.unwrap_or(0)
                + cache_read.unwrap_or(0)
        });

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

    fn default_settings() -> AdapterSettings {
        AdapterSettings {
            api_key: "test-key".to_string(),
            auth_token: String::new(),
            endpoint: "https://api.openai.com/v1/responses".to_string(),
            extra_headers: Default::default(),
            model_name: "gpt-4.1".to_string(),
            supports_tools: true,
            supports_reasoning: true,
            reasoning_type: Some("openai".to_string()),
            supports_temperature: true,
            supports_max_completion_tokens: false,
            eof_is_done: false,
            supports_web_search: false,
            supports_cache_control: true,
        }
    }

    fn chatgpt_backend_settings() -> AdapterSettings {
        AdapterSettings {
            endpoint: "https://chatgpt.com/backend-api/codex/responses".to_string(),
            ..default_settings()
        }
    }

    #[test]
    fn test_chatgpt_backend_omits_rejected_params() {
        let adapter = OpenAiResponsesAdapter;
        let mut req = LlmRequest::new(
            "gpt-5.3-codex".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        req.params.temperature = Some(0.5);
        req.params.top_p = Some(0.9);
        req.params.frequency_penalty = Some(0.3);
        req.params.stop = vec!["STOP".to_string()];

        let http = adapter
            .build_http(&req, &chatgpt_backend_settings())
            .unwrap();

        assert!(
            http.body.get("max_output_tokens").is_none(),
            "ChatGPT backend must not have max_output_tokens"
        );
        assert!(
            http.body.get("temperature").is_none(),
            "ChatGPT backend must not have temperature"
        );
        assert!(
            http.body.get("top_p").is_none(),
            "ChatGPT backend must not have top_p"
        );
        assert!(
            http.body.get("frequency_penalty").is_none(),
            "ChatGPT backend must not have frequency_penalty"
        );
        assert!(
            http.body.get("stop").is_none(),
            "ChatGPT backend must not have stop"
        );
        assert_eq!(
            http.body["store"], false,
            "ChatGPT backend must have store=false"
        );
    }

    #[test]
    fn test_chatgpt_backend_adds_default_instructions_without_system() {
        let adapter = OpenAiResponsesAdapter;
        let req = LlmRequest::new(
            "gpt-5.3-codex".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );

        let http = adapter
            .build_http(&req, &chatgpt_backend_settings())
            .unwrap();

        assert_eq!(
            http.body["instructions"],
            json!(CHATGPT_BACKEND_DEFAULT_INSTRUCTIONS),
            "ChatGPT backend requires a non-empty instructions field"
        );
    }

    #[test]
    fn test_chatgpt_backend_keeps_system_instructions() {
        let adapter = OpenAiResponsesAdapter;
        let req = LlmRequest::new(
            "gpt-5.3-codex".to_string(),
            vec![
                ChatMessage::new("system".to_string(), "Be precise".to_string()),
                ChatMessage::new("user".to_string(), "Hello".to_string()),
            ],
        );

        let http = adapter
            .build_http(&req, &chatgpt_backend_settings())
            .unwrap();

        assert_eq!(http.body["instructions"], json!("Be precise"));
    }

    #[test]
    fn test_chatgpt_backend_extra_body_blocked() {
        let adapter = OpenAiResponsesAdapter;
        let mut req = LlmRequest::new(
            "gpt-5.3-codex".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        let mut extra = serde_json::Map::new();
        extra.insert("temperature".to_string(), json!(0.7));
        extra.insert("max_output_tokens".to_string(), json!(1000));
        extra.insert("custom_field".to_string(), json!("allowed"));
        req.extra_body = Some(extra);

        let http = adapter
            .build_http(&req, &chatgpt_backend_settings())
            .unwrap();

        assert!(
            http.body.get("temperature").is_none(),
            "extra_body temperature should be blocked on ChatGPT backend"
        );
        assert!(
            http.body.get("max_output_tokens").is_none(),
            "extra_body max_output_tokens should be blocked on ChatGPT backend"
        );
        assert_eq!(
            http.body["custom_field"], "allowed",
            "non-rejected extra_body fields should still be passed"
        );
    }

    #[test]
    fn test_chatgpt_backend_http_request_omits_internal_websocket_header() {
        let adapter = OpenAiResponsesAdapter;
        let req = LlmRequest::new(
            "gpt-5.3-codex".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        let mut settings = chatgpt_backend_settings();
        settings.extra_headers.insert(
            crate::providers::openai_codex::CODEX_WEBSOCKET_ENDPOINT_HEADER.to_string(),
            "wss://chatgpt.com/backend-api/codex/responses".to_string(),
        );

        let http = adapter.build_http(&req, &settings).unwrap();

        assert!(http
            .headers
            .get(crate::providers::openai_codex::CODEX_WEBSOCKET_ENDPOINT_HEADER)
            .is_none());
        assert_eq!(
            http.headers
                .get(reqwest::header::ACCEPT)
                .map(|v| v.to_str().unwrap()),
            None
        );
    }

    #[test]
    fn test_previous_response_id_forwarded() {
        let adapter = OpenAiResponsesAdapter;
        let mut req = LlmRequest::new(
            "gpt-4.1".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        req.previous_response_id = Some("resp_123".to_string());

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert_eq!(http.body["previous_response_id"], "resp_123");
        assert_eq!(
            http.body["store"], true,
            "Responses chaining requires store=true"
        );
    }

    #[test]
    fn test_standard_api_store_true_by_default() {
        let adapter = OpenAiResponsesAdapter;
        let req = LlmRequest::new(
            "gpt-4.1".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );

        let http = adapter.build_http(&req, &default_settings()).unwrap();
        assert_eq!(
            http.body["store"], true,
            "Responses API should default to store=true"
        );
    }

    #[test]
    fn test_standard_api_includes_params() {
        let adapter = OpenAiResponsesAdapter;
        let mut req = LlmRequest::new(
            "gpt-4.1".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        req.params.temperature = Some(0.5);
        req.params.top_p = Some(0.9);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert!(
            http.body.get("max_output_tokens").is_some(),
            "Standard API should have max_output_tokens"
        );
        assert!(
            http.body.get("temperature").is_none(),
            "Responses requests for reasoning-capable models must omit temperature"
        );
        assert!(
            http.body.get("top_p").is_none(),
            "Responses requests for reasoning-capable models must omit top_p"
        );
    }

    #[test]
    fn test_standard_api_includes_top_p_for_non_reasoning_model() {
        let adapter = OpenAiResponsesAdapter;
        let mut req = LlmRequest::new(
            "gpt-4.1".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        req.params.top_p = Some(0.9);

        let mut settings = default_settings();
        settings.supports_reasoning = false;

        let http = adapter.build_http(&req, &settings).unwrap();

        assert!((http.body["top_p"].as_f64().unwrap() - 0.9).abs() < 0.000_001);
    }

    #[test]
    fn test_build_http_basic() {
        let adapter = OpenAiResponsesAdapter;
        let req = LlmRequest::new(
            "gpt-4.1".to_string(),
            vec![
                ChatMessage::new("system".to_string(), "You are helpful".to_string()),
                ChatMessage::new("user".to_string(), "Hello".to_string()),
            ],
        );
        let settings = default_settings();

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(http.body["model"], "gpt-4.1");
        assert_eq!(http.body["instructions"], "You are helpful");
        assert!(http.body["input"].is_array());
    }

    #[test]
    fn github_copilot_openai_responses_adds_vision_header_only_for_images() {
        use crate::call_validation::ChatContent;
        use crate::scratchpads::multimodality::MultimodalElement;

        let adapter = OpenAiResponsesAdapter;
        let image_message = ChatMessage {
            role: "user".to_string(),
            content: ChatContent::Multimodal(vec![MultimodalElement {
                m_type: "image/png".to_string(),
                m_content: "base64data".to_string(),
            }]),
            ..Default::default()
        };
        let req = LlmRequest::new("github_copilot/gpt-4.1".to_string(), vec![image_message]);
        let mut settings = default_settings();
        settings.endpoint = "https://api.githubcopilot.com/v1/responses".to_string();
        settings.api_key = "copilot-token".to_string();

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(
            http.headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer copilot-token"
        );
        assert_eq!(
            http.headers
                .get("Copilot-Vision-Request")
                .unwrap()
                .to_str()
                .unwrap(),
            "true"
        );

        let text_req = LlmRequest::new(
            "github_copilot/gpt-4.1".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        let text_http = adapter.build_http(&text_req, &settings).unwrap();
        assert!(text_http.headers.get("Copilot-Vision-Request").is_none());
    }

    #[test]
    fn test_build_http_with_reasoning() {
        let adapter = OpenAiResponsesAdapter;
        let mut req = LlmRequest::new("gpt-4.1".to_string(), vec![])
            .with_reasoning(crate::llm::params::ReasoningIntent::Medium);
        req.params.temperature = Some(0.5);
        let settings = default_settings();

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(http.body["reasoning"]["effort"], "medium");
        assert!(
            http.body.get("temperature").is_none(),
            "Responses reasoning requests must omit temperature"
        );
    }

    #[test]
    fn test_parse_stream_chunk_text_delta() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.output_text.delta","delta":"Hello"}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::AppendContent { text, .. } => assert_eq!(text, "Hello"),
            _ => panic!("expected AppendContent"),
        }
    }

    #[test]
    fn test_parse_stream_chunk_completed() {
        let adapter = OpenAiResponsesAdapter;
        let chunk =
            r#"{"type":"response.completed","usage":{"input_tokens":10,"output_tokens":5}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(deltas.iter().any(|d| matches!(d, LlmStreamDelta::Done)));
        assert!(deltas
            .iter()
            .any(|d| matches!(d, LlmStreamDelta::SetFinishReason { .. })));
    }

    #[test]
    fn test_parse_stream_chunk_completed_with_response_wrapper() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.completed","response":{"id":"resp_123","usage":{"input_tokens":20,"output_tokens":10}}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(deltas.iter().any(|d| matches!(d, LlmStreamDelta::Done)));
        assert!(deltas
            .iter()
            .any(|d| matches!(d, LlmStreamDelta::SetUsage { .. })));
    }

    #[test]
    fn test_parse_stream_chunk_failed() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.failed","error":{"message":"rate limit"}}"#;

        let result = adapter.parse_stream_chunk(chunk);

        assert!(matches!(result, Err(StreamParseError::FatalError(_))));
    }

    #[test]
    fn test_convert_system_to_instructions() {
        let messages = vec![
            ChatMessage::new("system".to_string(), "Be helpful".to_string()),
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ];

        let (input, instructions) = convert_to_responses_format(&messages);

        assert_eq!(instructions, Some("Be helpful".to_string()));
        assert_eq!(input.as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_convert_developer_to_instructions() {
        let messages = vec![
            ChatMessage::new("developer".to_string(), "Be concise".to_string()),
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ];

        let (input, instructions) = convert_to_responses_format(&messages);

        assert_eq!(instructions, Some("Be concise".to_string()));
        assert_eq!(input.as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_convert_tool_loop_history() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Get the weather".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_123".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: ChatToolFunction {
                        name: "get_weather".to_string(),
                        arguments: r#"{"location":"NYC"}"#.to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Sunny, 72F".to_string()),
                tool_call_id: "call_123".to_string(),
                ..Default::default()
            },
        ];

        let (input, _) = convert_to_responses_format(&messages);
        let input_arr = input.as_array().unwrap();

        assert_eq!(input_arr.len(), 3);
        assert_eq!(input_arr[0]["role"], "user");
        assert_eq!(input_arr[1]["type"], "function_call");
        assert_eq!(input_arr[1]["call_id"], "call_123");
        assert_eq!(input_arr[1]["name"], "get_weather");
        assert_eq!(input_arr[2]["type"], "function_call_output");
        assert_eq!(input_arr[2]["call_id"], "call_123");
    }

    #[test]
    fn test_parse_stream_tool_call_output_item_added() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_123","call_id":"call_abc123","name":"get_weather","arguments":""}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::SetToolCalls { tool_calls } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0]["id"], "call_abc123");
                assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
            }
            _ => panic!("expected SetToolCalls"),
        }
    }

    #[test]
    fn test_parse_stream_tool_call_arguments_delta() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.function_call_arguments.delta","item_id":"fc_123","output_index":0,"delta":"{\"loc"}"#;

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
    fn test_parse_stream_function_call_arguments_done() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.function_call_arguments.done","item_id":"fc_123","output_index":0,"name":"get_weather","arguments":"{\"location\":\"Paris\"}"}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(
            deltas
                .iter()
                .any(|d| matches!(d, LlmStreamDelta::FinalizeToolCalls { .. })),
            "arguments.done should emit FinalizeToolCalls"
        );
        if let Some(LlmStreamDelta::FinalizeToolCalls { tool_calls }) = deltas
            .iter()
            .find(|d| matches!(d, LlmStreamDelta::FinalizeToolCalls { .. }))
        {
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
            assert_eq!(
                tool_calls[0]["function"]["arguments"],
                "{\"location\":\"Paris\"}"
            );
        }
    }

    #[test]
    fn test_parse_stream_output_item_done_function_call() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_123","call_id":"call_abc123","name":"get_weather","arguments":"{\"location\":\"Paris\"}"}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(
            deltas
                .iter()
                .any(|d| matches!(d, LlmStreamDelta::FinalizeToolCalls { .. })),
            "output_item.done (function_call) should emit FinalizeToolCalls"
        );
        if let Some(LlmStreamDelta::FinalizeToolCalls { tool_calls }) = deltas
            .iter()
            .find(|d| matches!(d, LlmStreamDelta::FinalizeToolCalls { .. }))
        {
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0]["id"], "call_abc123");
            assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
            assert_eq!(
                tool_calls[0]["function"]["arguments"],
                "{\"location\":\"Paris\"}"
            );
        }
    }

    #[test]
    fn test_parse_stream_output_item_added_web_search_creates_srvtool() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"web_search_call","id":"ws_123","status":"in_progress"}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let tool_calls: Vec<_> = deltas
            .iter()
            .filter_map(|d| match d {
                LlmStreamDelta::SetToolCalls { tool_calls } => Some(tool_calls.clone()),
                _ => None,
            })
            .flatten()
            .collect();

        assert!(
            tool_calls
                .iter()
                .any(|tc| tc.get("id") == Some(&json!("srvtoolu_ws_123"))),
            "should create a srvtoolu_ tool call for web_search_call output item"
        );
        assert!(tool_calls
            .iter()
            .any(|tc| tc["function"]["name"] == "openai_web_search_call"));
    }

    #[test]
    fn test_usage_total_tokens_fallback() {
        let adapter = OpenAiResponsesAdapter;
        let chunk =
            r#"{"type":"response.completed","usage":{"input_tokens":100,"output_tokens":50}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let usage_delta = deltas
            .iter()
            .find(|d| matches!(d, LlmStreamDelta::SetUsage { .. }));
        assert!(usage_delta.is_some());
        match usage_delta.unwrap() {
            LlmStreamDelta::SetUsage { usage } => {
                assert_eq!(usage.prompt_tokens, 100);
                assert_eq!(usage.completion_tokens, 50);
                assert_eq!(usage.total_tokens, 150);
            }
            _ => panic!("expected SetUsage"),
        }
    }

    #[test]
    fn test_usage_extracts_openai_cached_input_tokens() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.completed","usage":{"input_tokens":1000,"input_tokens_details":{"cached_tokens":800},"output_tokens":100,"total_tokens":1100}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let usage_delta = deltas
            .iter()
            .find(|d| matches!(d, LlmStreamDelta::SetUsage { .. }));
        assert!(usage_delta.is_some());
        match usage_delta.unwrap() {
            LlmStreamDelta::SetUsage { usage } => {
                assert_eq!(usage.prompt_tokens, 200);
                assert_eq!(usage.completion_tokens, 100);
                assert_eq!(usage.total_tokens, 1100);
                assert_eq!(usage.cache_creation_tokens, None);
                assert_eq!(usage.cache_read_tokens, Some(800));
            }
            _ => panic!("expected SetUsage"),
        }
    }

    #[test]
    fn test_usage_extracts_cached_tokens_from_response_wrapper() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.completed","response":{"usage":{"input_tokens":1000,"input_tokens_details":{"cached_tokens":750},"output_tokens":100}}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let usage_delta = deltas
            .iter()
            .find(|d| matches!(d, LlmStreamDelta::SetUsage { .. }));
        assert!(usage_delta.is_some());
        match usage_delta.unwrap() {
            LlmStreamDelta::SetUsage { usage } => {
                assert_eq!(usage.prompt_tokens, 250);
                assert_eq!(usage.completion_tokens, 100);
                assert_eq!(usage.total_tokens, 1100);
                assert_eq!(usage.cache_read_tokens, Some(750));
            }
            _ => panic!("expected SetUsage"),
        }
    }

    #[test]
    fn test_convert_messages_uniform_type() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Hi there".to_string()),
                tool_calls: None,
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "Follow up question".to_string()),
        ];

        let (input, _) = convert_to_responses_format(&messages);
        let input_arr = input.as_array().unwrap();

        // All message-type items should have type: "message"
        assert_eq!(input_arr[0]["type"], "message");
        assert_eq!(input_arr[0]["role"], "user");
        assert_eq!(input_arr[1]["type"], "message");
        assert_eq!(input_arr[1]["role"], "assistant");
        // Assistant history should use simple string content, not output_text array
        assert!(
            input_arr[1]["content"].is_string(),
            "assistant content should be a string, not an array"
        );
        assert_eq!(input_arr[1]["content"], "Hi there");
        assert_eq!(input_arr[2]["type"], "message");
        assert_eq!(input_arr[2]["role"], "user");
    }

    #[test]
    fn test_tool_arguments_object_stringified() {
        let json = json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "type": "function_call",
                "call_id": "call_123",
                "name": "test_func",
                "arguments": {"key": "value", "num": 42}
            }
        });

        let tc = extract_tool_call_from_item(&json["item"], &json).unwrap();
        // Arguments should be stringified JSON
        assert_eq!(tc["function"]["arguments"], r#"{"key":"value","num":42}"#);
    }

    #[test]
    fn test_stream_web_search_citations() {
        let adapter = OpenAiResponsesAdapter;
        // Web search results in output_item.done event
        let chunk = r#"{"type":"response.output_item.done","output_index":0,"item":{"type":"web_search_call","id":"ws_123","results":[{"url":"https://example.com","title":"Example","snippet":"Some content"},{"url":"https://other.com","title":"Other","snippet":"Other content"}]}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        let citation_count = deltas
            .iter()
            .filter(|d| matches!(d, LlmStreamDelta::AddCitation { .. }))
            .count();
        assert_eq!(citation_count, 2);

        // Verify first citation
        let citations: Vec<_> = deltas
            .iter()
            .filter_map(|d| {
                if let LlmStreamDelta::AddCitation { citation } = d {
                    Some(citation)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            citations[0].get("url").and_then(|v| v.as_str()),
            Some("https://example.com")
        );
        assert_eq!(
            citations[1].get("url").and_then(|v| v.as_str()),
            Some("https://other.com")
        );
    }

    #[test]
    fn test_stream_completed_no_duplicate_citations() {
        let adapter = OpenAiResponsesAdapter;
        // response.completed should NOT emit citations (they come from output_item.done)
        let chunk = r#"{"type":"response.completed","response":{"status":"completed","output":[{"type":"web_search_call","results":[{"url":"https://search.com","title":"Search Result"}]}]}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        let citation_count = deltas
            .iter()
            .filter(|d| matches!(d, LlmStreamDelta::AddCitation { .. }))
            .count();
        assert_eq!(
            citation_count, 0,
            "response.completed should not duplicate citations from output_item.done"
        );
    }

    #[test]
    fn test_reasoning_item_captured_from_output_item_done() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.output_item.done","item":{"id":"rs_abc123","type":"reasoning","summary":[],"status":null}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let has_thinking = deltas
            .iter()
            .any(|d| matches!(d, LlmStreamDelta::SetThinkingBlocks { .. }));
        assert!(
            has_thinking,
            "Should capture reasoning item as SetThinkingBlocks"
        );

        if let Some(LlmStreamDelta::SetThinkingBlocks { blocks }) = deltas
            .iter()
            .find(|d| matches!(d, LlmStreamDelta::SetThinkingBlocks { .. }))
        {
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0]["type"], "reasoning");
            assert_eq!(blocks[0]["id"], "rs_abc123");
        }
    }

    #[test]
    fn test_reasoning_item_with_encrypted_content() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.output_item.done","item":{"id":"rs_xyz789","type":"reasoning","summary":[],"encrypted_content":"gAAAAABo...encrypted...","status":null}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        if let Some(LlmStreamDelta::SetThinkingBlocks { blocks }) = deltas
            .iter()
            .find(|d| matches!(d, LlmStreamDelta::SetThinkingBlocks { .. }))
        {
            assert_eq!(blocks[0]["type"], "reasoning");
            assert_eq!(blocks[0]["encrypted_content"], "gAAAAABo...encrypted...");
        }
    }

    #[test]
    fn test_reasoning_items_resent_in_multi_turn() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "What's the weather?".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("".to_string()),
                thinking_blocks: Some(vec![json!({
                    "id": "rs_abc123",
                    "type": "reasoning",
                    "summary": [],
                    "encrypted_content": "gAAAAABo...encrypted..."
                })]),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_weather".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: ChatToolFunction {
                        name: "get_weather".to_string(),
                        arguments: r#"{"city":"Paris"}"#.to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("20°C sunny".to_string()),
                tool_call_id: "call_weather".to_string(),
                ..Default::default()
            },
        ];

        let (input, _) = convert_to_responses_format(&messages);
        let items = input.as_array().unwrap();

        // Should be: user message, reasoning item, function_call, function_call_output
        assert_eq!(items.len(), 4);

        // First item: user message
        assert_eq!(items[0]["type"], "message");
        assert_eq!(items[0]["role"], "user");

        // Second item: reasoning item (re-sent from prior assistant turn)
        assert_eq!(items[1]["type"], "reasoning");
        assert_eq!(items[1]["id"], "rs_abc123");
        assert_eq!(items[1]["encrypted_content"], "gAAAAABo...encrypted...");

        // Third item: function call
        assert_eq!(items[2]["type"], "function_call");
        assert_eq!(items[2]["name"], "get_weather");

        // Fourth item: function call output
        assert_eq!(items[3]["type"], "function_call_output");
        assert_eq!(items[3]["output"], "20°C sunny");
    }

    #[test]
    fn test_non_reasoning_thinking_blocks_not_resent() {
        // Thinking blocks from Anthropic (type="thinking") should NOT be included
        // in Responses API input — only type="reasoning" items are valid
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Hi".to_string()),
                thinking_blocks: Some(vec![json!({
                    "type": "thinking",
                    "thinking": "Let me think...",
                    "signature": "sig_abc"
                })]),
                ..Default::default()
            },
        ];

        let (input, _) = convert_to_responses_format(&messages);
        let items = input.as_array().unwrap();

        // Should only have user message + assistant message, no thinking blocks
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], "message");
        assert_eq!(items[0]["role"], "user");
        assert_eq!(items[1]["type"], "message");
        assert_eq!(items[1]["role"], "assistant");
    }

    #[test]
    fn test_include_encrypted_reasoning_in_request() {
        let adapter = OpenAiResponsesAdapter;
        let req = LlmRequest::new(
            "o4-mini".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hi".to_string())],
        )
        .with_reasoning(crate::llm::params::ReasoningIntent::Medium);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        // Should include reasoning.encrypted_content for multi-turn support
        let include = http.body["include"].as_array().unwrap();
        assert!(
            include.contains(&json!("reasoning.encrypted_content")),
            "Should request encrypted reasoning content"
        );
    }

    #[test]
    fn test_no_include_when_reasoning_not_supported() {
        let adapter = OpenAiResponsesAdapter;
        let mut settings = default_settings();
        settings.supports_reasoning = false;

        let req = LlmRequest::new(
            "gpt-4.1".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hi".to_string())],
        );

        let http = adapter.build_http(&req, &settings).unwrap();

        let include = http.body["include"].as_array().unwrap();
        assert!(
            !include.contains(&json!("reasoning.encrypted_content")),
            "Should not request reasoning.encrypted_content when reasoning not supported"
        );
    }

    #[test]
    fn test_reasoning_summary_text_delta() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.reasoning_summary_text.delta","item_id":"rs_abc","output_index":0,"summary_index":0,"delta":"Thinking about"}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(
            deltas
                .iter()
                .any(|d| matches!(d, LlmStreamDelta::AppendReasoning { .. })),
            "reasoning_summary_text.delta should produce AppendReasoning"
        );
        if let Some(LlmStreamDelta::AppendReasoning { text, .. }) = deltas
            .iter()
            .find(|d| matches!(d, LlmStreamDelta::AppendReasoning { .. }))
        {
            assert_eq!(text, "Thinking about");
        }
    }

    #[test]
    fn test_reasoning_summary_part_added_ignored() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.reasoning_summary_part.added","item_id":"rs_abc","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":""},"sequence_number":3}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(
            deltas.is_empty(),
            "reasoning_summary_part.added should produce no deltas"
        );
    }

    #[test]
    fn test_reasoning_summary_part_done_ignored() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.reasoning_summary_part.done","item_id":"rs_abc","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":"**Sending friendly greeting**"},"sequence_number":9}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(
            deltas.is_empty(),
            "reasoning_summary_part.done should produce no deltas"
        );
    }

    #[test]
    fn test_reasoning_text_delta() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.reasoning_text.delta","item_id":"rs_abc","output_index":0,"content_index":0,"delta":"Let me reason"}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(
            deltas
                .iter()
                .any(|d| matches!(d, LlmStreamDelta::AppendReasoning { .. })),
            "reasoning_text.delta should produce AppendReasoning"
        );
    }

    #[test]
    fn test_refusal_delta() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.refusal.delta","item_id":"msg_abc","output_index":0,"content_index":0,"delta":"I cannot help with"}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(
            deltas
                .iter()
                .any(|d| matches!(d, LlmStreamDelta::AppendContent { .. })),
            "refusal.delta should produce AppendContent"
        );
    }

    #[test]
    fn test_response_incomplete() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":100,"output_tokens":4096}}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(deltas.iter().any(|d| matches!(d, LlmStreamDelta::Done)));
        if let Some(LlmStreamDelta::SetFinishReason { reason }) = deltas
            .iter()
            .find(|d| matches!(d, LlmStreamDelta::SetFinishReason { .. }))
        {
            assert_eq!(reason, "length");
        }
    }

    #[test]
    fn test_error_event() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"error","code":"server_error","message":"Internal server error"}"#;

        let result = adapter.parse_stream_chunk(chunk);
        assert!(matches!(result, Err(StreamParseError::FatalError(_))));
    }

    #[test]
    fn test_annotation_added() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.output_text.annotation.added","item_id":"msg_abc","output_index":0,"content_index":0,"annotation_index":0,"annotation":{"type":"url_citation","url":"https://example.com","title":"Example"}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(
            deltas
                .iter()
                .any(|d| matches!(d, LlmStreamDelta::AddCitation { .. })),
            "annotation.added should produce AddCitation"
        );
    }

    #[test]
    fn test_convert_tools_to_responses_preserves_schema() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "search",
                "description": "Search the web",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Search query"},
                        "limit": {"type": "integer"},
                        "tags": {
                            "type": "array",
                            "items": {"type": "string"}
                        }
                    },
                    "required": ["query"]
                }
            }
        })];

        let result = convert_tools_to_responses(&tools);
        let converted = result.as_array().unwrap();
        assert_eq!(converted.len(), 1);

        let tool = &converted[0];
        assert_eq!(tool["type"], json!("function"));
        assert_eq!(tool["name"], json!("search"));
        assert_eq!(tool["description"], json!("Search the web"));
        assert_eq!(tool["strict"], json!(false), "strict must default to false to prevent optional params being filled with empty strings");

        let params = &tool["parameters"];
        assert_eq!(params["type"], json!("object"));
        assert_eq!(params["properties"]["query"]["type"], json!("string"));
        assert_eq!(params["properties"]["limit"]["type"], json!("integer"));
        assert_eq!(params["properties"]["tags"]["type"], json!("array"));
        assert_eq!(
            params["properties"]["tags"]["items"]["type"],
            json!("string")
        );
        assert_eq!(params["required"], json!(["query"]));
        assert!(
            tool.get("function").is_none(),
            "function wrapper must not be present in responses format"
        );
    }

    #[test]
    fn test_convert_tools_to_responses_strict_true_preserved() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "strict_tool",
                "description": "A strict tool",
                "strict": true,
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"}
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }
            }
        })];

        let result = convert_tools_to_responses(&tools);
        let converted = result.as_array().unwrap();
        assert_eq!(
            converted[0]["strict"],
            json!(true),
            "strict=true must be passed through to Responses API format"
        );
    }

    #[test]
    fn test_convert_tools_to_responses_already_in_responses_format() {
        let tools = vec![json!({
            "type": "function",
            "name": "already_converted",
            "description": "Already in responses format",
            "parameters": {"type": "object", "properties": {}}
        })];

        let result = convert_tools_to_responses(&tools);
        let converted = result.as_array().unwrap();
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["name"], json!("already_converted"));
    }
}
