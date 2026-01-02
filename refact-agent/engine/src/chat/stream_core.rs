use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use futures::StreamExt;
use reqwest_eventsource::{Event, Error as EventSourceError};
use serde_json::json;
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::{ChatMeta, ChatUsage, SamplingParameters};
use crate::caps::BaseModelRecord;
use crate::global_context::GlobalContext;
use crate::scratchpad_abstract::FinishReason;

use super::types::{DeltaOp, STREAM_HEARTBEAT, STREAM_IDLE_TIMEOUT, STREAM_TOTAL_TIMEOUT};
use super::openai_merge::merge_tool_call;

pub struct StreamRunParams {
    pub prompt: String,
    pub model_rec: BaseModelRecord,
    pub sampling: SamplingParameters,
    pub meta: Option<ChatMeta>,
    pub abort_flag: Option<Arc<AtomicBool>>,
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
    n: usize,
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

    let mut sampling = params.sampling.clone();
    if n > 1 {
        sampling.n = Some(n);
    }

    let mut event_source =
        crate::forward_to_openai_endpoint::forward_to_openai_style_endpoint_streaming(
            &params.model_rec,
            &params.prompt,
            &client,
            &sampling,
            params.meta,
        )
        .await
        .map_err(|e| format!("Failed to connect to LLM: {}", e))?;

    let mut accumulators: Vec<ChoiceAccumulator> =
        (0..n).map(|_| ChoiceAccumulator::default()).collect();

    let stream_started_at = Instant::now();
    let mut last_event_at = Instant::now();
    let mut heartbeat = tokio::time::interval(STREAM_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let event = tokio::select! {
            _ = heartbeat.tick() => {
                if let Some(ref flag) = params.abort_flag {
                    if flag.load(Ordering::SeqCst) {
                        return Err("Aborted".to_string());
                    }
                }
                if stream_started_at.elapsed() > STREAM_TOTAL_TIMEOUT {
                    return Err("LLM stream timeout".to_string());
                }
                if last_event_at.elapsed() > STREAM_IDLE_TIMEOUT {
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
                if msg.data.starts_with("[DONE]") {
                    break;
                }

                let json: serde_json::Value = serde_json::from_str(&msg.data)
                    .map_err(|e| format!("JSON parse error: {}", e))?;

                if let Some(err) = json.get("error") {
                    return Err(format!("LLM error: {}", err));
                }
                if let Some(detail) = json.get("detail") {
                    return Err(format!("LLM error: {}", detail));
                }

                if let Some(usage) = json.get("usage").filter(|u| !u.is_null()) {
                    if let Ok(parsed) = serde_json::from_value::<ChatUsage>(usage.clone()) {
                        for acc in &mut accumulators {
                            acc.usage = Some(parsed.clone());
                        }
                        collector.on_usage(&parsed);
                    }
                }

                let choices = match json.get("choices").and_then(|c| c.as_array()) {
                    Some(arr) => arr,
                    None => continue,
                };

                for choice in choices {
                    let choice_idx =
                        choice.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                    if choice_idx >= accumulators.len() {
                        accumulators.resize_with(choice_idx + 1, ChoiceAccumulator::default);
                    }

                    let acc = &mut accumulators[choice_idx];

                    if let Some(fr) = choice.get("finish_reason").filter(|f| !f.is_null()) {
                        acc.finish_reason = FinishReason::from_json_val(fr).ok();
                    }

                    let delta = match choice.get("delta") {
                        Some(d) => d,
                        None => continue,
                    };

                    let ops = process_delta(acc, delta, &json);
                    if !ops.is_empty() {
                        collector.on_delta_ops(choice_idx, ops);
                    }
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
            let finish_reason = match acc.finish_reason {
                Some(FinishReason::Stop) | Some(FinishReason::ScratchpadStop) => {
                    Some("stop".to_string())
                }
                Some(FinishReason::Length) => Some("length".to_string()),
                _ => None,
            };
            collector.on_finish(idx, finish_reason.clone());
            ChoiceFinal {
                content: acc.content,
                reasoning: acc.reasoning,
                thinking_blocks: acc.thinking_blocks,
                tool_calls_raw: acc.tool_calls,
                citations: acc.citations,
                extra: acc.extra,
                finish_reason,
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
    finish_reason: Option<FinishReason>,
    usage: Option<ChatUsage>,
}

fn process_delta(
    acc: &mut ChoiceAccumulator,
    delta: &serde_json::Value,
    json: &serde_json::Value,
) -> Vec<DeltaOp> {
    let mut ops = Vec::new();

    if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
        if !content.is_empty() {
            acc.content.push_str(content);
            ops.push(DeltaOp::AppendContent {
                text: content.to_string(),
            });
        }
    }

    if let Some(reasoning) = delta.get("reasoning_content").and_then(|c| c.as_str()) {
        if !reasoning.is_empty() {
            acc.reasoning.push_str(reasoning);
            ops.push(DeltaOp::AppendReasoning {
                text: reasoning.to_string(),
            });
        }
    }

    if let Some(tool_calls) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
        for tc in tool_calls {
            merge_tool_call(&mut acc.tool_calls, tc.clone());
        }
        if !acc.tool_calls.is_empty() {
            ops.push(DeltaOp::SetToolCalls {
                tool_calls: acc.tool_calls.clone(),
            });
        }
    }

    let thinking_blocks_raw = delta
        .get("thinking_blocks")
        .and_then(|tb| tb.as_array())
        .or_else(|| {
            delta
                .get("provider_specific_fields")
                .and_then(|psf| psf.get("thinking_blocks"))
                .and_then(|tb| tb.as_array())
        })
        .or_else(|| {
            json.get("provider_specific_fields")
                .and_then(|psf| psf.get("thinking_blocks"))
                .and_then(|tb| tb.as_array())
        });

    if let Some(thinking) = thinking_blocks_raw {
        let normalized: Vec<serde_json::Value> = thinking.iter().map(|block| {
            if block.get("thinking").is_some() {
                block.clone()
            } else if let Some(text) = block.get("text") {
                json!({"type": "thinking", "thinking": text, "signature": block.get("signature").cloned()})
            } else if let Some(content) = block.get("content") {
                json!({"type": "thinking", "thinking": content, "signature": block.get("signature").cloned()})
            } else if block.is_string() {
                json!({"type": "thinking", "thinking": block, "signature": null})
            } else {
                block.clone()
            }
        }).collect();
        acc.thinking_blocks = normalized.clone();
        ops.push(DeltaOp::SetThinkingBlocks { blocks: normalized });
    }

    for source in [
        json.get("provider_specific_fields"),
        delta.get("provider_specific_fields"),
    ] {
        if let Some(citation) = source
            .and_then(|psf| psf.get("citation"))
            .filter(|c| !c.is_null())
        {
            acc.citations.push(citation.clone());
            ops.push(DeltaOp::AddCitation {
                citation: citation.clone(),
            });
        }
    }

    let mut changed_extra = serde_json::Map::new();
    if let Some(obj) = json.as_object() {
        for (key, val) in obj {
            if val.is_null() {
                continue;
            }
            let dominated = key.starts_with("metering_")
                || key.starts_with("billing_")
                || key.starts_with("cost_")
                || key.starts_with("cache_")
                || key == "system_fingerprint";
            if dominated && acc.extra.get(key) != Some(val) {
                acc.extra.insert(key.clone(), val.clone());
                changed_extra.insert(key.clone(), val.clone());
            }
        }
    }
    if let Some(psf) = json
        .get("provider_specific_fields")
        .filter(|p| !p.is_null())
    {
        if acc.extra.get("provider_specific_fields") != Some(psf) {
            acc.extra
                .insert("provider_specific_fields".to_string(), psf.clone());
            changed_extra.insert("provider_specific_fields".to_string(), psf.clone());
        }
    }
    if !changed_extra.is_empty() {
        ops.push(DeltaOp::MergeExtra {
            extra: changed_extra,
        });
    }

    if let Some(usage) = json.get("usage").filter(|u| !u.is_null()) {
        ops.push(DeltaOp::SetUsage {
            usage: usage.clone(),
        });
    }

    ops
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
            let preview = if text.len() > 500 { &text[..500] } else { &text };
            format!("LLM error ({}): {}", status, preview)
        }
        other => format!("Stream error: {}", other),
    }
}
