use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use futures::StreamExt;
use eventsource_stream::Eventsource;
use serde_json::{json, Value};
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::ChatUsage;
use crate::caps::BaseModelRecord;
use crate::global_context::GlobalContext;
use crate::llm::{LlmRequest, LlmStreamDelta, get_adapter, safe_truncate};
use crate::llm::adapter::{AdapterSettings, StreamParseError};

use super::types::{DeltaOp, stream_heartbeat, stream_idle_timeout, stream_total_timeout};
use super::openai_merge::ToolCallAccumulator;

fn merge_usage(existing: Option<ChatUsage>, incoming: ChatUsage) -> ChatUsage {
    match existing {
        None => incoming,
        Some(prev) => {
            let prev_cache_read = prev.cache_read_tokens.unwrap_or(0);
            let incoming_cache_read = incoming.cache_read_tokens.unwrap_or(0);
            let cache_read_increased = incoming_cache_read > prev_cache_read;

            let merged_cache_creation = match (prev.cache_creation_tokens, incoming.cache_creation_tokens) {
                (Some(a), Some(b)) => Some(std::cmp::max(a, b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            let merged_cache_read = match (prev.cache_read_tokens, incoming.cache_read_tokens) {
                (Some(a), Some(b)) => Some(std::cmp::max(a, b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };

            let merged_prompt_tokens = if cache_read_increased {
                incoming.prompt_tokens
            } else if prev.prompt_tokens == 0 && incoming.prompt_tokens > 0 {
                incoming.prompt_tokens
            } else {
                std::cmp::max(prev.prompt_tokens, incoming.prompt_tokens)
            };

            let merged_completion = std::cmp::max(prev.completion_tokens, incoming.completion_tokens);

            let merged_metering = match (prev.metering_usd, incoming.metering_usd) {
                (_, Some(b)) => Some(b),
                (Some(a), None) => Some(a),
                (None, None) => None,
            };

            let merged_total = merged_prompt_tokens
                + merged_completion
                + merged_cache_creation.unwrap_or(0)
                + merged_cache_read.unwrap_or(0);

            ChatUsage {
                prompt_tokens: merged_prompt_tokens,
                completion_tokens: merged_completion,
                total_tokens: merged_total,
                cache_creation_tokens: merged_cache_creation,
                cache_read_tokens: merged_cache_read,
                metering_usd: merged_metering,
            }
        }
    }
}

pub struct StreamRunParams {
    pub llm_request: LlmRequest,
    pub model_rec: BaseModelRecord,
    pub chat_id: Option<String>,
    pub abort_flag: Option<Arc<AtomicBool>>,
    pub supports_tools: bool,
    pub supports_reasoning: bool,
    pub reasoning_type: Option<String>,
    pub supports_temperature: bool,
}

#[derive(Default, Clone)]
pub struct ChoiceFinal {
    pub content: String,
    pub reasoning: String,
    pub thinking_blocks: Vec<serde_json::Value>,
    pub tool_calls_raw: Vec<serde_json::Value>,
    pub citations: Vec<serde_json::Value>,
    pub server_content_blocks: Vec<serde_json::Value>,
    pub extra: serde_json::Map<String, serde_json::Value>,
    pub finish_reason: Option<String>,
    pub usage: Option<ChatUsage>,
}

pub trait StreamCollector: Send {
    fn on_delta_ops(&mut self, choice_idx: usize, ops: Vec<DeltaOp>);
    fn on_usage(&mut self, usage: &ChatUsage);
    fn on_finish(&mut self, choice_idx: usize, finish_reason: Option<String>);
}


pub async fn run_llm_stream<C: StreamCollector>(
    gcx: Arc<ARwLock<GlobalContext>>,
    params: StreamRunParams,
    collector: &mut C,
) -> Result<Vec<ChoiceFinal>, String> {
    if params.llm_request.params.n.unwrap_or(1) != 1 {
        return Err("Streaming with n > 1 is not supported".to_string());
    }

    let (client, slowdown_arc) = {
        let gcx_locked = gcx.read().await;
        (
            gcx_locked.http_client.clone(),
            gcx_locked.http_client_slowdown.clone(),
        )
    };

    let _ = slowdown_arc.acquire().await;

    let wire_format = params.model_rec.wire_format;
    let adapter = get_adapter(wire_format);

    let adapter_settings = AdapterSettings {
        api_key: params.model_rec.api_key.clone(),
        auth_token: params.model_rec.auth_token.clone(),
        endpoint: params.model_rec.endpoint.clone(),
        extra_headers: params.model_rec.extra_headers.clone(),
        model_name: params.model_rec.name.clone(),
        supports_tools: params.supports_tools,
        supports_reasoning: params.supports_reasoning,
        reasoning_type: params.reasoning_type.clone(),
        supports_temperature: params.supports_temperature,
        supports_max_completion_tokens: params.model_rec.supports_max_completion_tokens,
        support_metadata: params.model_rec.support_metadata,
        eof_is_done: params.model_rec.eof_is_done,
        supports_web_search: params.model_rec.supports_web_search,
    };

    let http_parts = adapter.build_http(&params.llm_request, &adapter_settings)
        .map_err(|e| format!("Failed to build LLM request: {}", e))?;

    let mut sanitized_for_commit: Option<serde_json::Value> = None;
    if let Some(chat_id) = &params.chat_id {
        let session_arc_opt = {
            let gcx_locked = gcx.read().await;
            let sessions = gcx_locked.chat_sessions.read().await;
            sessions.get(chat_id).cloned()
        };
        if let Some(session_arc) = session_arc_opt {
            sanitized_for_commit = crate::chat::cache_guard::check_or_pause_cache_guard(
                gcx.clone(),
                session_arc,
                &params.llm_request.model_id,
                &http_parts.body,
            )
            .await?;
        }
    }

    if http_parts.url.is_empty() {
        return Err("LLM endpoint URL is empty".to_string());
    }

    tracing::debug!(
        url = %http_parts.url,
        model = %params.llm_request.model_id,
        messages_count = params.llm_request.messages.len(),
        "LLM streaming request"
    );

    let response = client
        .post(&http_parts.url)
        .headers(http_parts.headers.clone())
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .json(&http_parts.body)
        .send()
        .await
        .map_err(|e| format!("LLM request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format_llm_error_body(&format!("{}", status), &text));
    }

    if let (Some(chat_id), Some(sanitized)) = (&params.chat_id, sanitized_for_commit) {
        let session_arc_opt = {
            let gcx_locked = gcx.read().await;
            let sessions = gcx_locked.chat_sessions.read().await;
            sessions.get(chat_id).cloned()
        };
        if let Some(session_arc) = session_arc_opt {
            crate::chat::cache_guard::commit_cache_guard_snapshot(session_arc, sanitized).await;
        }
    }

    let mut stream = response.bytes_stream().eventsource();

    let mut accumulators: Vec<ChoiceAccumulator> = vec![ChoiceAccumulator::default()];
    let mut stream_done = false;

    let stream_started_at = Instant::now();
    let mut last_event_at = Instant::now();
    let mut heartbeat = tokio::time::interval(stream_heartbeat());
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        if stream_done {
            break;
        }
        let event = tokio::select! {
            _ = heartbeat.tick() => {
                if let Some(ref flag) = params.abort_flag {
                    if flag.load(Ordering::SeqCst) {
                        return Err("Aborted".to_string());
                    }
                }
                if stream_started_at.elapsed() > stream_total_timeout() {
                    return Err("LLM stream timeout".to_string());
                }
                if last_event_at.elapsed() > stream_idle_timeout() {
                    return Err("LLM stream stalled".to_string());
                }
                continue;
            }
            maybe_event = stream.next() => {
                match maybe_event {
                    Some(Ok(ev)) => ev,
                    Some(Err(e)) => {
                        return Err(format!("Stream error: {}", e));
                    }
                    None => {
                        if !stream_done && !adapter_settings.eof_is_done {
                            return Err("LLM stream ended unexpectedly without completion signal".to_string());
                        }
                        break;
                    }
                }
            }
        };
        last_event_at = Instant::now();

        let deltas = match adapter.parse_stream_chunk(&event.data) {
            Ok(d) => d,
            Err(StreamParseError::Skip) => continue,
            Err(StreamParseError::MalformedChunk(e)) => {
                tracing::warn!("Malformed stream chunk: {}", e);
                continue;
            }
            Err(StreamParseError::FatalError(e)) => {
                return Err(format!("LLM error: {}", e));
            }
        };

        let acc = &mut accumulators[0];
        let mut ops = Vec::new();

        for delta in deltas {
            match delta {
                LlmStreamDelta::AppendContent { text, block_index } => {
                    acc.content.push_str(&text);
                    if let Some(idx) = block_index {
                        acc.content_per_block.entry(idx).or_default().push_str(&text);
                    }
                    ops.push(DeltaOp::AppendContent { text });
                }
                LlmStreamDelta::AppendReasoning { text, block_index } => {
                    acc.reasoning.push_str(&text);
                    if let Some(idx) = block_index {
                        acc.reasoning_per_block.entry(idx).or_default().push_str(&text);
                    }
                    ops.push(DeltaOp::AppendReasoning { text });
                }
                LlmStreamDelta::SetToolCalls { tool_calls } => {
                    let tool_calls = if !params.model_rec.auth_token.is_empty() {
                        tool_calls.into_iter().map(|mut tc| {
                            strip_mcp_prefix_from_tool_call(&mut tc);
                            tc
                        }).collect()
                    } else {
                        tool_calls
                    };
                    for tc in &tool_calls {
                        acc.tool_calls.merge(tc);
                    }
                    ops.push(DeltaOp::SetToolCalls { tool_calls: acc.tool_calls.finalize() });
                }
                LlmStreamDelta::FinalizeToolCalls { tool_calls } => {
                    let tool_calls = if !params.model_rec.auth_token.is_empty() {
                        tool_calls.into_iter().map(|mut tc| {
                            strip_mcp_prefix_from_tool_call(&mut tc);
                            tc
                        }).collect()
                    } else {
                        tool_calls
                    };
                    for tc in &tool_calls {
                        acc.tool_calls.set_final(tc);
                    }
                    ops.push(DeltaOp::SetToolCalls { tool_calls: acc.tool_calls.finalize() });
                }
                LlmStreamDelta::SetThinkingBlocks { blocks } => {
                    merge_thinking_blocks(&mut acc.thinking_blocks, blocks);
                    ops.push(DeltaOp::SetThinkingBlocks { blocks: acc.thinking_blocks.clone() });
                }
                LlmStreamDelta::AddCitation { citation } => {
                    acc.citations.push(citation.clone());
                    ops.push(DeltaOp::AddCitation { citation });
                }
                LlmStreamDelta::AddServerContentBlock { block } => {
                    acc.server_content_blocks.push(block.clone());
                    ops.push(DeltaOp::AddServerContentBlock { block });
                }
                LlmStreamDelta::SetUsage { usage } => {
                    acc.usage = Some(merge_usage(acc.usage.take(), usage.clone()));
                    if let Some(ref merged) = acc.usage {
                        collector.on_usage(merged);
                        ops.push(DeltaOp::SetUsage { usage: json!(merged) });
                    }
                }
                LlmStreamDelta::SetFinishReason { reason } => {
                    acc.finish_reason = Some(reason);
                }
                LlmStreamDelta::MergeExtra { extra } => {
                    for (k, v) in &extra {
                        match (acc.extra.get_mut(k), v) {
                            (Some(Value::Array(existing)), Value::Array(incoming)) => {
                                existing.extend(incoming.clone());
                            }
                            (Some(Value::Object(existing)), Value::Object(incoming)) => {
                                // Shallow-merge objects.
                                for (ik, iv) in incoming {
                                    existing.insert(ik.clone(), iv.clone());
                                }
                            }
                            _ => {
                                acc.extra.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    ops.push(DeltaOp::MergeExtra { extra });
                }
                LlmStreamDelta::Done => {
                    stream_done = true;
                    break;
                }
            }
        }

        if !ops.is_empty() {
            collector.on_delta_ops(0, ops);
        }
    }

    let results: Vec<ChoiceFinal> = accumulators
        .into_iter()
        .enumerate()
        .map(|(idx, acc)| {
            collector.on_finish(idx, acc.finish_reason.clone());
            // Merge accumulated reasoning text into thinking_blocks.
            // Three cases:
            // 1) Anthropic: thinking_blocks exist with type="thinking" — merge per-block reasoning
            //    (for interleaved thinking, each block gets its own text via block_index)
            // 2) OpenAI: thinking_blocks exist with type="reasoning" — leave opaque, don't modify
            // 3) No blocks but reasoning text exists — create a synthetic reasoning block
            let thinking_blocks = if !acc.thinking_blocks.is_empty() && !acc.reasoning.is_empty() {
                acc.thinking_blocks.into_iter().map(|mut block| {
                    if let Some(obj) = block.as_object_mut() {
                        let is_anthropic_thinking = obj.get("type")
                            .and_then(|t| t.as_str()) == Some("thinking");
                        // Check if thinking text is missing or empty/whitespace.
                        // LiteLLM sends final signed blocks with empty "thinking": ""
                        // since the text was already streamed via reasoning_content.
                        let thinking_is_empty = obj.get("thinking")
                            .and_then(|v| v.as_str())
                            .map_or(true, |s| s.trim().is_empty());
                        if is_anthropic_thinking && thinking_is_empty {
                            // Use per-block reasoning when available (interleaved thinking),
                            // fall back to global reasoning for single-block case.
                            let block_idx = obj.get("index").and_then(|v| v.as_u64());
                            let reasoning_text = block_idx
                                .and_then(|idx| acc.reasoning_per_block.get(&idx))
                                .unwrap_or(&acc.reasoning);
                            if !reasoning_text.is_empty() {
                                obj.insert("thinking".to_string(), json!(reasoning_text.clone()));
                            }
                        }
                    }
                    block
                }).collect()
            } else if acc.thinking_blocks.is_empty() && !acc.reasoning.is_empty() {
                vec![json!({
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": acc.reasoning.clone()}]
                })]
            } else {
                acc.thinking_blocks
            };

            ChoiceFinal {
                content: acc.content,
                reasoning: acc.reasoning,
                thinking_blocks,
                tool_calls_raw: acc.tool_calls.finalize(),
                citations: acc.citations,
                server_content_blocks: acc.server_content_blocks,
                extra: {
                    let mut extra = acc.extra;
                    if !acc.content_per_block.is_empty() {
                        let mut text_blocks: Vec<_> = acc.content_per_block
                            .into_iter()
                            .collect();
                        text_blocks.sort_by_key(|(idx, _)| *idx);
                        extra.insert(
                            "_anthropic_text_blocks".to_string(),
                            json!(text_blocks.into_iter().map(|(idx, text)| {
                                json!({"index": idx, "text": text})
                            }).collect::<Vec<_>>()),
                        );
                    }
                    extra
                },
                finish_reason: acc.finish_reason,
                usage: acc.usage,
            }
        })
        .collect();

    Ok(results)
}

/// Merges incoming thinking blocks into the accumulator, deduplicating by:
/// 1. `id` field (if present)
/// 2. `(type, index)` pair (Anthropic signature deltas)
/// 3. `(type, signature)` pair (LiteLLM blocks without index)
///
/// When a duplicate is found, the existing block's signature is updated
/// to the latest value (handles streaming signature updates).
pub(crate) fn merge_thinking_blocks(dst: &mut Vec<serde_json::Value>, incoming: Vec<serde_json::Value>) {
    for block in incoming {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let block_id = block.get("id").and_then(|v| v.as_str());
        let block_index = block.get("index").and_then(|v| v.as_u64());
        let block_sig = block.get("signature").and_then(|v| v.as_str());

        let existing_idx = if let Some(id) = block_id {
            dst.iter().position(|b| b.get("id").and_then(|v| v.as_str()) == Some(id))
        } else if let Some(idx) = block_index {
            dst.iter().position(|b| {
                b.get("type").and_then(|v| v.as_str()).unwrap_or("") == block_type
                    && b.get("index").and_then(|v| v.as_u64()) == Some(idx)
            })
        } else if let Some(sig) = block_sig {
            dst.iter().position(|b| {
                b.get("type").and_then(|v| v.as_str()).unwrap_or("") == block_type
                    && b.get("signature").and_then(|v| v.as_str()) == Some(sig)
            })
        } else {
            None
        };

        if let Some(pos) = existing_idx {
            if let Some(new_sig) = block.get("signature").and_then(|v| v.as_str()) {
                if let Some(obj) = dst[pos].as_object_mut() {
                    obj.insert("signature".to_string(), json!(new_sig));
                }
            }
        } else {
            dst.push(block);
        }
    }
}

#[derive(Default)]
struct ChoiceAccumulator {
    content: String,
    /// Per-block content text for Anthropic interleaved output.
    /// Key is the content block index from the stream.
    content_per_block: HashMap<u64, String>,
    reasoning: String,
    /// Per-block reasoning text for Anthropic interleaved thinking.
    /// Key is the content block index from the stream.
    reasoning_per_block: HashMap<u64, String>,
    thinking_blocks: Vec<serde_json::Value>,
    tool_calls: ToolCallAccumulator,
    citations: Vec<serde_json::Value>,
    server_content_blocks: Vec<serde_json::Value>,
    extra: serde_json::Map<String, serde_json::Value>,
    finish_reason: Option<String>,
    usage: Option<ChatUsage>,
}

fn strip_mcp_prefix_from_tool_call(tc: &mut serde_json::Value) {
    if let Some(func) = tc.get_mut("function") {
        if let Some(name) = func.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()) {
            if let Some(stripped) = name.strip_prefix("mcp_") {
                func["name"] = serde_json::json!(stripped);
            }
        }
    }
}

pub fn normalize_tool_call(tc: &serde_json::Value) -> Option<crate::call_validation::ChatToolCall> {
    let function = tc.get("function")?;
    let name = function
        .get("name")
        .and_then(|n| n.as_str())
        .filter(|s| !s.is_empty())?;

    let id = tc
        .get("id")
        .and_then(|i| i.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            format!(
                "call_{}",
                uuid::Uuid::new_v4().to_string().replace("-", "")[..24].to_string()
            )
        });

    let arguments = match function.get("arguments") {
        Some(serde_json::Value::String(s)) if s.trim().starts_with('{') => s.clone(),
        Some(serde_json::Value::Object(_)) => serde_json::to_string(&function["arguments"]).unwrap_or_else(|_| "{}".to_string()),
        _ => "{}".to_string(),
    };

    let tool_type = tc
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("function")
        .to_string();

    let index = tc.get("index").and_then(|i| i.as_u64()).map(|i| i as usize);

    let extra_content = tc.get("extra_content").filter(|v| !v.is_null()).cloned();

    Some(crate::call_validation::ChatToolCall {
        id,
        index,
        function: crate::call_validation::ChatToolFunction {
            name: name.to_string(),
            arguments,
        },
        tool_type,
        extra_content,
    })
}

fn format_llm_error_body(status_label: &str, text: &str) -> String {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(detail) = json.get("detail") {
            return format!("LLM error ({}): {}", status_label, detail);
        }
        if let Some(msg) = json.pointer("/error/message") {
            return format!("LLM error ({}): {}", status_label, msg);
        }
        if let Some(err_obj) = json.get("error") {
            return format!("LLM error ({}): {}", status_label, err_obj);
        }
    }
    let preview = safe_truncate(text, 500);
    format!("LLM error ({}): {}", status_label, preview)
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_usage_cache_read_appears_later() {
        let prev = ChatUsage {
            prompt_tokens: 1500,
            completion_tokens: 100,
            total_tokens: 1600,
            cache_creation_tokens: None,
            cache_read_tokens: None,
            metering_usd: None,
        };

        let incoming = ChatUsage {
            prompt_tokens: 500,
            completion_tokens: 200,
            total_tokens: 1700,
            cache_creation_tokens: None,
            cache_read_tokens: Some(1000),
            metering_usd: None,
        };

        let merged = merge_usage(Some(prev), incoming);

        assert_eq!(merged.prompt_tokens, 500);
        assert_eq!(merged.completion_tokens, 200);
        assert_eq!(merged.cache_read_tokens, Some(1000));
        assert_eq!(merged.total_tokens, 1700);
    }

    #[test]
    fn test_merge_usage_prompt_increases_normally() {
        let prev = ChatUsage {
            prompt_tokens: 500,
            completion_tokens: 100,
            total_tokens: 600,
            cache_creation_tokens: None,
            cache_read_tokens: Some(1000),
            metering_usd: None,
        };

        let incoming = ChatUsage {
            prompt_tokens: 600,
            completion_tokens: 150,
            total_tokens: 750,
            cache_creation_tokens: None,
            cache_read_tokens: Some(1000),
            metering_usd: None,
        };

        let merged = merge_usage(Some(prev), incoming);

        assert_eq!(merged.prompt_tokens, 600);
        assert_eq!(merged.completion_tokens, 150);
    }

    #[test]
    fn test_merge_usage_from_none() {
        let incoming = ChatUsage {
            prompt_tokens: 500,
            completion_tokens: 200,
            total_tokens: 700,
            cache_creation_tokens: Some(100),
            cache_read_tokens: Some(200),
            metering_usd: None,
        };

        let merged = merge_usage(None, incoming.clone());

        assert_eq!(merged.prompt_tokens, 500);
        assert_eq!(merged.completion_tokens, 200);
        assert_eq!(merged.cache_creation_tokens, Some(100));
        assert_eq!(merged.cache_read_tokens, Some(200));
    }

    #[test]
    fn test_merge_usage_metering_incoming_wins() {
        use crate::call_validation::MeteringUsd;

        let prev = ChatUsage {
            prompt_tokens: 500,
            completion_tokens: 200,
            total_tokens: 700,
            cache_creation_tokens: None,
            cache_read_tokens: None,
            metering_usd: Some(MeteringUsd {
                prompt_usd: 0.001,
                generated_usd: 0.002,
                cache_read_usd: None,
                cache_creation_usd: None,
                total_usd: 0.003,
            }),
        };

        let incoming = ChatUsage {
            prompt_tokens: 500,
            completion_tokens: 300,
            total_tokens: 800,
            cache_creation_tokens: None,
            cache_read_tokens: None,
            metering_usd: Some(MeteringUsd {
                prompt_usd: 0.002,
                generated_usd: 0.004,
                cache_read_usd: None,
                cache_creation_usd: None,
                total_usd: 0.006,
            }),
        };

        let merged = merge_usage(Some(prev), incoming);

        assert!(merged.metering_usd.is_some());
        assert_eq!(merged.metering_usd.unwrap().total_usd, 0.006);
    }

    /// Helper: simulate accumulator finalization (same logic as run_llm_stream).
    fn finalize_accumulator(acc: ChoiceAccumulator) -> ChoiceFinal {
        let thinking_blocks = if !acc.thinking_blocks.is_empty() && !acc.reasoning.is_empty() {
            acc.thinking_blocks.into_iter().map(|mut block| {
                if let Some(obj) = block.as_object_mut() {
                    let is_anthropic_thinking = obj.get("type")
                        .and_then(|t| t.as_str()) == Some("thinking");
                    let thinking_is_empty = obj.get("thinking")
                        .and_then(|v| v.as_str())
                        .map_or(true, |s| s.trim().is_empty());
                    if is_anthropic_thinking && thinking_is_empty {
                        let block_idx = obj.get("index").and_then(|v| v.as_u64());
                        let reasoning_text = block_idx
                            .and_then(|idx| acc.reasoning_per_block.get(&idx))
                            .unwrap_or(&acc.reasoning);
                        if !reasoning_text.is_empty() {
                            obj.insert("thinking".to_string(), json!(reasoning_text.clone()));
                        }
                    }
                }
                block
            }).collect()
        } else if acc.thinking_blocks.is_empty() && !acc.reasoning.is_empty() {
            vec![json!({
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": acc.reasoning.clone()}]
            })]
        } else {
            acc.thinking_blocks
        };

        ChoiceFinal {
            content: acc.content,
            reasoning: acc.reasoning,
            thinking_blocks,
            tool_calls_raw: acc.tool_calls.finalize(),
            citations: acc.citations,
            server_content_blocks: acc.server_content_blocks,
            extra: acc.extra,
            finish_reason: acc.finish_reason,
            usage: acc.usage,
        }
    }

    #[test]
    fn test_litellm_empty_thinking_text_gets_reasoning_merged() {
        // LiteLLM sends signed thinking blocks with empty "thinking": ""
        // because reasoning was already streamed via reasoning_content.
        // The accumulator must merge the accumulated reasoning text in.
        let mut acc = ChoiceAccumulator::default();
        acc.reasoning = "Let me think about this step by step...".to_string();
        acc.thinking_blocks = vec![json!({
            "type": "thinking",
            "thinking": "",
            "signature": "sig_abc123"
        })];

        let result = finalize_accumulator(acc);

        assert_eq!(result.thinking_blocks.len(), 1);
        assert_eq!(
            result.thinking_blocks[0]["thinking"].as_str().unwrap(),
            "Let me think about this step by step..."
        );
        assert_eq!(result.thinking_blocks[0]["signature"], "sig_abc123");
    }

    #[test]
    fn test_litellm_null_thinking_text_gets_reasoning_merged() {
        // Edge case: thinking field is present but null (not a string)
        let mut acc = ChoiceAccumulator::default();
        acc.reasoning = "Reasoning text".to_string();
        acc.thinking_blocks = vec![json!({
            "type": "thinking",
            "thinking": null,
            "signature": "sig_xyz"
        })];

        let result = finalize_accumulator(acc);

        assert_eq!(result.thinking_blocks.len(), 1);
        assert_eq!(
            result.thinking_blocks[0]["thinking"].as_str().unwrap(),
            "Reasoning text"
        );
    }

    #[test]
    fn test_anthropic_signature_only_block_gets_reasoning() {
        // Native Anthropic adapter: signature_delta creates blocks with no "thinking" key.
        let mut acc = ChoiceAccumulator::default();
        acc.reasoning = "Deep analysis here".to_string();
        acc.thinking_blocks = vec![json!({
            "index": 0,
            "type": "thinking",
            "signature": "sig_native"
        })];

        let result = finalize_accumulator(acc);

        assert_eq!(result.thinking_blocks.len(), 1);
        assert_eq!(
            result.thinking_blocks[0]["thinking"].as_str().unwrap(),
            "Deep analysis here"
        );
    }

    #[test]
    fn test_interleaved_thinking_per_block_reasoning() {
        // Anthropic interleaved thinking: multiple thinking blocks at different indices.
        // Each block must get only its own reasoning text, not the concatenation.
        let mut acc = ChoiceAccumulator::default();
        acc.reasoning = "First thought...Second thought...".to_string();
        acc.reasoning_per_block.insert(0, "First thought...".to_string());
        acc.reasoning_per_block.insert(4, "Second thought...".to_string());
        acc.thinking_blocks = vec![
            json!({"index": 0, "type": "thinking", "signature": "sig1"}),
            json!({"index": 4, "type": "thinking", "signature": "sig2"}),
        ];

        let result = finalize_accumulator(acc);

        assert_eq!(result.thinking_blocks.len(), 2);
        assert_eq!(
            result.thinking_blocks[0]["thinking"].as_str().unwrap(),
            "First thought...",
            "Block 0 should get only its own reasoning text"
        );
        assert_eq!(
            result.thinking_blocks[1]["thinking"].as_str().unwrap(),
            "Second thought...",
            "Block 4 should get only its own reasoning text"
        );
    }

    #[test]
    fn test_signature_delta_concatenation() {
        // Anthropic sends a single signature_delta per thinking block.
        // Some proxies may emit multiple updates; signature must be treated as
        // an opaque integrity token, so the latest update must replace the prior.
        let mut blocks = vec![json!({
            "index": 0,
            "type": "thinking",
        })];

        // First signature chunk
        merge_thinking_blocks(&mut blocks, vec![json!({
            "index": 0,
            "type": "thinking",
            "signature": "abc"
        })]);
        assert_eq!(blocks[0]["signature"].as_str().unwrap(), "abc");

        // Second signature chunk — should concatenate, not replace
        merge_thinking_blocks(&mut blocks, vec![json!({
            "index": 0,
            "type": "thinking",
            "signature": "def"
        })]);
        assert_eq!(blocks[0]["signature"].as_str().unwrap(), "def",
            "Signature updates must replace, not concatenate");

        // Third chunk
        merge_thinking_blocks(&mut blocks, vec![json!({
            "index": 0,
            "type": "thinking",
            "signature": "ghi"
        })]);
        assert_eq!(blocks[0]["signature"].as_str().unwrap(), "ghi",
            "Latest signature update must win");
    }

    #[test]
    fn test_thinking_block_with_existing_text_not_overwritten() {
        // If a thinking block already has non-empty thinking text (e.g., from LiteLLM
        // final chunk that included the text), it should NOT be overwritten.
        let mut acc = ChoiceAccumulator::default();
        acc.reasoning = "Streamed reasoning".to_string();
        acc.thinking_blocks = vec![json!({
            "type": "thinking",
            "thinking": "Original block text",
            "signature": "sig_keep"
        })];

        let result = finalize_accumulator(acc);

        assert_eq!(result.thinking_blocks.len(), 1);
        assert_eq!(
            result.thinking_blocks[0]["thinking"].as_str().unwrap(),
            "Original block text",
            "Pre-existing thinking text should be preserved"
        );
    }

    #[test]
    fn test_redacted_thinking_blocks_unchanged() {
        // Redacted thinking blocks should pass through without modification.
        let mut acc = ChoiceAccumulator::default();
        acc.reasoning = "Some reasoning".to_string();
        acc.thinking_blocks = vec![
            json!({"type": "thinking", "signature": "sig1"}),
            json!({"type": "redacted_thinking", "data": "encrypted_blob"}),
        ];

        let result = finalize_accumulator(acc);

        assert_eq!(result.thinking_blocks.len(), 2);
        // thinking block gets reasoning merged
        assert_eq!(result.thinking_blocks[0]["thinking"].as_str().unwrap(), "Some reasoning");
        // redacted block untouched
        assert_eq!(result.thinking_blocks[1]["type"], "redacted_thinking");
        assert_eq!(result.thinking_blocks[1]["data"], "encrypted_blob");
        assert!(result.thinking_blocks[1].get("thinking").is_none());
    }

    #[test]
    fn test_synthetic_reasoning_block_when_no_thinking_blocks() {
        // When there are no thinking_blocks but reasoning exists,
        // a synthetic reasoning block should be created.
        let mut acc = ChoiceAccumulator::default();
        acc.reasoning = "Some reasoning from OpenAI".to_string();

        let result = finalize_accumulator(acc);

        assert_eq!(result.thinking_blocks.len(), 1);
        assert_eq!(result.thinking_blocks[0]["type"], "reasoning");
    }

    #[test]
    fn test_whitespace_only_thinking_text_gets_replaced() {
        // Whitespace-only thinking text should be treated as empty.
        let mut acc = ChoiceAccumulator::default();
        acc.reasoning = "Real reasoning".to_string();
        acc.thinking_blocks = vec![json!({
            "type": "thinking",
            "thinking": "   \n\t  ",
            "signature": "sig_ws"
        })];

        let result = finalize_accumulator(acc);

        assert_eq!(
            result.thinking_blocks[0]["thinking"].as_str().unwrap(),
            "Real reasoning",
            "Whitespace-only thinking should be replaced with accumulated reasoning"
        );
    }

    #[test]
    fn test_cache_guard_sanitize_removes_fields() {
        let body = serde_json::json!({
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "x", "cache_control": {"type": "ephemeral"}}]}
            ],
            "temperature": 0.2,
            "max_tokens": 1000,
            "reasoning_effort": "high"
        });

        let sanitized = crate::chat::cache_guard::sanitize_body_for_cache_guard(&body);
        assert!(sanitized.get("temperature").is_none());
        assert!(sanitized.get("max_tokens").is_none());
        assert_eq!(sanitized["reasoning_effort"], "high");
        assert!(sanitized["messages"][0]["content"][0]
            .get("cache_control")
            .is_none());
    }

    #[test]
    fn test_cache_guard_append_only_prefix_logic() {
        let prev = serde_json::json!({
            "messages": [
                {"role": "user", "content": "a"},
                {"role": "assistant", "content": "b"}
            ],
            "meta": {"chat_id": "c1"}
        });
        let next_ok = serde_json::json!({
            "messages": [
                {"role": "user", "content": "a"},
                {"role": "assistant", "content": "b"},
                {"role": "user", "content": "c"}
            ],
            "meta": {"chat_id": "c1", "request_attempt_id": "r2"}
        });
        let next_bad = serde_json::json!({
            "messages": [
                {"role": "user", "content": "a"},
                {"role": "assistant", "content": "CHANGED"}
            ],
            "meta": {"chat_id": "c1"}
        });

        assert!(crate::chat::cache_guard::is_append_only_prefix(&prev, &next_ok));
        assert!(!crate::chat::cache_guard::is_append_only_prefix(&prev, &next_bad));
    }

    #[test]
    fn test_merge_thinking_blocks_dedupe_by_index() {
        let mut dst = vec![
            json!({"index": 0, "type": "thinking", "signature": "sig_v1"}),
        ];

        merge_thinking_blocks(&mut dst, vec![
            json!({"index": 0, "type": "thinking", "signature": "sig_v2"}),
        ]);

        assert_eq!(dst.len(), 1, "Same (type, index) should dedupe");
        assert_eq!(dst[0]["signature"], "sig_v2", "Signature should be updated to latest");
    }

    #[test]
    fn test_merge_thinking_blocks_streaming_signature_does_not_concat() {
        // Even if the upstream sends multiple signature updates, we must NOT
        // concatenate them: signatures are integrity-checked by the provider.
        let mut dst = vec![
            json!({"index": 0, "type": "thinking", "signature": "sig_part1"}),
        ];

        merge_thinking_blocks(&mut dst, vec![
            json!({"index": 0, "type": "thinking", "signature": "sig_part2"}),
        ]);

        assert_eq!(dst.len(), 1);
        assert_eq!(dst[0]["signature"], "sig_part2", "Signature must be replaced, not concatenated");
    }

    #[test]
    fn test_merge_thinking_blocks_different_indices_kept() {
        let mut dst = Vec::new();

        merge_thinking_blocks(&mut dst, vec![
            json!({"index": 0, "type": "thinking", "signature": "sig1"}),
            json!({"index": 4, "type": "thinking", "signature": "sig2"}),
        ]);

        assert_eq!(dst.len(), 2, "Different indices should produce separate blocks");
    }

    #[test]
    fn test_merge_thinking_blocks_dedupe_by_signature_no_index() {
        // LiteLLM blocks often have no index — dedupe by (type, signature)
        let mut dst = vec![
            json!({"type": "thinking", "thinking": "text", "signature": "sig_abc"}),
        ];

        merge_thinking_blocks(&mut dst, vec![
            json!({"type": "thinking", "thinking": "text", "signature": "sig_abc"}),
        ]);

        assert_eq!(dst.len(), 1, "Same (type, signature) without index should dedupe");
    }

    #[test]
    fn test_merge_thinking_blocks_different_types_same_index_not_deduped() {
        let mut dst = vec![
            json!({"index": 0, "type": "thinking", "signature": "sig1"}),
        ];

        merge_thinking_blocks(&mut dst, vec![
            json!({"index": 0, "type": "redacted_thinking", "data": "encrypted"}),
        ]);

        assert_eq!(dst.len(), 2,
            "Different types at same index should not dedupe");
    }

    #[test]
    fn test_merge_thinking_blocks_signature_added_to_existing() {
        // First block has no signature, second adds it
        let mut dst = vec![
            json!({"index": 0, "type": "thinking"}),
        ];

        merge_thinking_blocks(&mut dst, vec![
            json!({"index": 0, "type": "thinking", "signature": "sig_new"}),
        ]);

        assert_eq!(dst.len(), 1);
        assert_eq!(dst[0]["signature"], "sig_new",
            "Signature should be added to existing block");
    }

    #[test]
    fn test_merge_thinking_blocks_dedupe_by_id() {
        let mut dst = vec![
            json!({"id": "block_1", "type": "thinking", "signature": "sig_old"}),
        ];

        merge_thinking_blocks(&mut dst, vec![
            json!({"id": "block_1", "type": "thinking", "signature": "sig_new"}),
        ]);

        assert_eq!(dst.len(), 1, "Same id should dedupe");
        assert_eq!(dst[0]["signature"], "sig_new");
    }

    #[test]
    fn test_merge_thinking_blocks_no_key_never_dedupes() {
        // Blocks with no id, no index, no signature always append
        let mut dst = vec![
            json!({"type": "thinking", "thinking": "text1"}),
        ];

        merge_thinking_blocks(&mut dst, vec![
            json!({"type": "thinking", "thinking": "text2"}),
        ]);

        assert_eq!(dst.len(), 2,
            "Blocks with no dedup key should always append");
    }

}
