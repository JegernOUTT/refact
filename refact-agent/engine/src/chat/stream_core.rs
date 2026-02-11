use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use futures::StreamExt;
use reqwest_eventsource::{Event, EventSource, Error as EventSourceError};
use serde_json::json;
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

pub struct NoopCollector;

impl StreamCollector for NoopCollector {
    fn on_delta_ops(&mut self, _: usize, _: Vec<DeltaOp>) {}
    fn on_usage(&mut self, _: &ChatUsage) {}
    fn on_finish(&mut self, _: usize, _: Option<String>) {}
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

    if http_parts.url.is_empty() {
        return Err("LLM endpoint URL is empty".to_string());
    }

    tracing::debug!(
        url = %http_parts.url,
        model = %params.llm_request.model_id,
        messages_count = params.llm_request.messages.len(),
        "LLM streaming request"
    );

    // Create event source for streaming
    let request = client
        .post(&http_parts.url)
        .headers(http_parts.headers.clone())
        .json(&http_parts.body);

    let mut event_source = EventSource::new(request)
        .map_err(|e| format!("Failed to create event source: {}", e))?;

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
            maybe_event = event_source.next() => {
                match maybe_event {
                    Some(e) => e,
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

        match event {
            Ok(Event::Open) => {}
            Ok(Event::Message(msg)) => {
                // Use adapter to parse streaming chunk
                let deltas = match adapter.parse_stream_chunk(&msg.data) {
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

                // Process deltas from adapter
                let acc = &mut accumulators[0]; // Single choice for now
                let mut ops = Vec::new();

                for delta in deltas {
                    match delta {
                        LlmStreamDelta::AppendContent { text } => {
                            acc.content.push_str(&text);
                            ops.push(DeltaOp::AppendContent { text });
                        }
                        LlmStreamDelta::AppendReasoning { text } => {
                            acc.reasoning.push_str(&text);
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
                        LlmStreamDelta::SetThinkingBlocks { blocks } => {
                            acc.thinking_blocks = blocks.clone();
                            ops.push(DeltaOp::SetThinkingBlocks { blocks });
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
                                acc.extra.insert(k.clone(), v.clone());
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
            Err(e) => {
                return Err(format_stream_error(e).await);
            }
        }
    }

    let results: Vec<ChoiceFinal> = accumulators
        .into_iter()
        .enumerate()
        .map(|(idx, acc)| {
            collector.on_finish(idx, acc.finish_reason.clone());
            // Merge accumulated reasoning text into thinking_blocks if present.
            // This is required for Anthropic tool calling - the thinking_blocks must contain
            // both the thinking text AND the signature for multi-turn conversations.
            // Only merge into Anthropic-style "thinking" blocks — OpenAI "reasoning" items
            // are opaque and must not be modified (they're passed back verbatim).
            let thinking_blocks = if !acc.thinking_blocks.is_empty() && !acc.reasoning.is_empty() {
                acc.thinking_blocks.into_iter().map(|mut block| {
                    if let Some(obj) = block.as_object_mut() {
                        let is_anthropic_thinking = obj.get("type")
                            .and_then(|t| t.as_str()) == Some("thinking");
                        if is_anthropic_thinking && !obj.contains_key("thinking") {
                            obj.insert("thinking".to_string(), json!(acc.reasoning.clone()));
                        }
                    }
                    block
                }).collect()
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
        })
        .collect();

    Ok(results)
}

#[derive(Default)]
struct ChoiceAccumulator {
    content: String,
    reasoning: String,
    thinking_blocks: Vec<serde_json::Value>,
    tool_calls: ToolCallAccumulator,  // Use efficient accumulator instead of Vec
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
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(v) if !v.is_null() => serde_json::to_string(v).unwrap_or_default(),
        _ => String::new(),
    };

    let tool_type = tc
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("function")
        .to_string();

    let index = tc.get("index").and_then(|i| i.as_u64()).map(|i| i as usize);

    Some(crate::call_validation::ChatToolCall {
        id,
        index,
        function: crate::call_validation::ChatToolFunction {
            name: name.to_string(),
            arguments,
        },
        tool_type,
    })
}

async fn format_stream_error(err: EventSourceError) -> String {
    match err {
        EventSourceError::InvalidStatusCode(status, response) => {
            let text = response.text().await.unwrap_or_default();
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(detail) = json.get("detail") {
                    return format!("LLM error ({}): {}", status, detail);
                }
                if let Some(msg) = json.pointer("/error/message") {
                    return format!("LLM error ({}): {}", status, msg);
                }
                if let Some(err_obj) = json.get("error") {
                    return format!("LLM error ({}): {}", status, err_obj);
                }
            }
            let preview = safe_truncate(&text, 500);
            format!("LLM error ({}): {}", status, preview)
        }
        other => format!("Stream error: {}", other),
    }
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
}
