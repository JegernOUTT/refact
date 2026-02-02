use std::collections::HashMap;
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
use crate::llm::{LlmRequest, LlmStreamDelta, get_adapter};
use crate::llm::adapter::{AdapterSettings, StreamParseError};

use super::types::{DeltaOp, stream_heartbeat, stream_idle_timeout, stream_total_timeout};
use super::openai_merge::merge_tool_call;

pub struct StreamRunParams {
    pub llm_request: LlmRequest,
    pub model_rec: BaseModelRecord,
    pub abort_flag: Option<Arc<AtomicBool>>,
    pub supports_tools: bool,
    pub supports_reasoning: bool,
}

#[derive(Default, Clone)]
pub struct ChoiceFinal {
    pub content: String,
    pub reasoning: String,
    pub thinking_blocks: Vec<serde_json::Value>,
    pub tool_calls_raw: Vec<serde_json::Value>,
    pub citations: Vec<serde_json::Value>,
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
    let (client, slowdown_arc) = {
        let gcx_locked = gcx.read().await;
        (
            gcx_locked.http_client.clone(),
            gcx_locked.http_client_slowdown.clone(),
        )
    };

    let _ = slowdown_arc.acquire().await;

    // Build adapter settings from model record
    let wire_format = params.model_rec.wire_format;
    let adapter = get_adapter(wire_format);

    let adapter_settings = AdapterSettings {
        api_key: params.model_rec.api_key.clone(),
        endpoint: params.model_rec.endpoint.clone(),
        extra_headers: HashMap::new(),
        model_name: params.model_rec.name.clone(),
        supports_tools: params.supports_tools,
        supports_reasoning: params.supports_reasoning,
    };

    // Build HTTP request using adapter
    let http_parts = adapter.build_http(&params.llm_request, &adapter_settings)
        .map_err(|e| format!("Failed to build LLM request: {}", e))?;

    if http_parts.url.is_empty() {
        return Err("LLM endpoint URL is empty".to_string());
    }

    // Create event source for streaming
    let request = client
        .post(&http_parts.url)
        .headers(http_parts.headers)
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
                    None => break,
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
                            for tc in &tool_calls {
                                merge_tool_call(&mut acc.tool_calls, tc.clone());
                            }
                            ops.push(DeltaOp::SetToolCalls { tool_calls: acc.tool_calls.clone() });
                        }
                        LlmStreamDelta::SetThinkingBlocks { blocks } => {
                            acc.thinking_blocks = blocks.clone();
                            ops.push(DeltaOp::SetThinkingBlocks { blocks });
                        }
                        LlmStreamDelta::AddCitation { citation } => {
                            acc.citations.push(citation.clone());
                            ops.push(DeltaOp::AddCitation { citation });
                        }
                        LlmStreamDelta::SetUsage { usage } => {
                            acc.usage = Some(usage.clone());
                            collector.on_usage(&usage);
                            ops.push(DeltaOp::SetUsage { usage: json!(usage) });
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
            ChoiceFinal {
                content: acc.content,
                reasoning: acc.reasoning,
                thinking_blocks: acc.thinking_blocks,
                tool_calls_raw: acc.tool_calls,
                citations: acc.citations,
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
    tool_calls: Vec<serde_json::Value>,
    citations: Vec<serde_json::Value>,
    extra: serde_json::Map<String, serde_json::Value>,
    finish_reason: Option<String>,
    usage: Option<ChatUsage>,
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

fn safe_truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    // Find the last char boundary at or before max_len
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
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
