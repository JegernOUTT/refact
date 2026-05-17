use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use futures::{SinkExt, StreamExt};
use eventsource_stream::Eventsource;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::app_state::AppState;
use crate::call_validation::ChatUsage;
use crate::caps::BaseModelRecord;
use crate::llm::{LlmRequest, LlmStreamDelta, WireFormat, get_adapter, safe_truncate};
use crate::llm::adapter::{AdapterSettings, HttpParts, StreamParseError};

use super::types::{DeltaOp, stream_heartbeat, stream_idle_timeout, stream_total_timeout};
use super::retry_policy::{classify_llm_error_for_retry, RetryDecision};
use super::openai_merge::ToolCallAccumulator;

fn merge_usage(existing: Option<ChatUsage>, incoming: ChatUsage) -> ChatUsage {
    match existing {
        None => incoming,
        Some(prev) => {
            let prev_cache_read = prev.cache_read_tokens.unwrap_or(0);
            let incoming_cache_read = incoming.cache_read_tokens.unwrap_or(0);
            let cache_read_increased = incoming_cache_read > prev_cache_read;

            let merged_cache_creation =
                match (prev.cache_creation_tokens, incoming.cache_creation_tokens) {
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

            let merged_completion =
                std::cmp::max(prev.completion_tokens, incoming.completion_tokens);

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

async fn send_llm_http_request(
    client: &reqwest::Client,
    http_parts: &HttpParts,
    wire_format: WireFormat,
) -> Result<reqwest::Response, String> {
    let accept = match wire_format {
        WireFormat::OllamaNative => "application/x-ndjson",
        _ => "text/event-stream",
    };

    client
        .post(&http_parts.url)
        .headers(http_parts.headers.clone())
        .header(reqwest::header::ACCEPT, accept)
        .json(&http_parts.body)
        .send()
        .await
        .map_err(|e| format!("LLM request failed: {}", e))
}

fn should_commit_cache_guard_after_http_success(status: reqwest::StatusCode, text: &str) -> bool {
    if status.is_success() {
        return true;
    }
    !matches!(
        classify_llm_error_for_retry(&format_llm_error_body(&format!("{}", status), text)),
        RetryDecision::Retry { .. }
    )
}

fn openai_codex_instance_id(model_rec: &BaseModelRecord) -> Option<&str> {
    let (provider_name, _) = model_rec.id.split_once('/')?;
    (model_rec.endpoint.contains("chatgpt.com/backend-api")
        && (provider_name == "openai_codex" || provider_name.starts_with("openai_codex_")))
    .then_some(provider_name)
}

fn is_openai_codex_chatgpt_backend(model_rec: &BaseModelRecord) -> bool {
    openai_codex_instance_id(model_rec).is_some()
}

async fn force_refresh_openai_codex_for_retry(
    app: AppState,
    http_client: &reqwest::Client,
    provider_instance_id: &str,
    status: reqwest::StatusCode,
    current_access_token: &str,
) -> Result<Option<String>, String> {
    let _guard = crate::providers::openai_codex::OpenAICodexProvider::lock_refresh_guard().await?;

    let (config_dir, provider) = {
        let registry = app.model.providers.read().await;
        let provider = registry
            .get(provider_instance_id)
            .and_then(|p| {
                p.as_any()
                    .downcast_ref::<crate::providers::openai_codex::OpenAICodexProvider>()
            })
            .cloned();
        let config_dir = app.paths.config_dir.read().unwrap().clone();
        (config_dir, provider)
    };

    let Some(mut provider) = provider else {
        if let Some(message) =
            crate::providers::openai_codex::OpenAICodexProvider::codex_cli_unmanaged_refresh_message(
                current_access_token,
            )
        {
            return Err(message);
        }
        return Ok(None);
    };

    if let Some(access_token) = provider.access_token_changed_since_rejection(current_access_token)
    {
        return Ok(Some(access_token));
    }

    if provider.oauth_tokens.refresh_token.is_empty() {
        if let Some(message) =
            crate::providers::openai_codex::OpenAICodexProvider::codex_cli_unmanaged_refresh_message(
                current_access_token,
            )
        {
            return Err(message);
        }
    }

    if !crate::providers::openai_codex::OpenAICodexProvider::should_force_refresh_for_status(
        status,
        &provider.oauth_tokens.refresh_token,
        false,
    ) {
        return Ok(None);
    }

    let previous_tokens = provider.oauth_tokens.clone();
    let previous_session_id = provider.session_id.clone();
    let refresh_result = provider
        .force_refresh_after_auth_rejection(http_client, &config_dir, provider_instance_id)
        .await;

    if !provider.auth_state_matches(&previous_tokens, &previous_session_id) {
        let changed = {
            let mut registry = app.model.providers.write().await;
            registry
                .get_mut(provider_instance_id)
                .and_then(|p| {
                    p.as_any_mut()
                        .downcast_mut::<crate::providers::openai_codex::OpenAICodexProvider>()
                })
                .map(|current| {
                    current.update_auth_state_from_if_current(
                        &provider,
                        &previous_tokens,
                        &previous_session_id,
                    )
                })
                .unwrap_or(false)
        };

        if changed {
            let mut caps = app.model.caps.write().await;
            caps.caps = None;
            caps.last_attempted_ts = 0;
        }
    }

    refresh_result
}

#[derive(Default, Clone)]
pub struct ChoiceFinal {
    pub content: String,
    pub raw_content: String,
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

enum CollectorReplayEvent {
    DeltaOps(usize, Vec<DeltaOp>),
    Usage(ChatUsage),
    Finish(usize, Option<String>),
}

#[derive(Default)]
struct ReplayCollector {
    events: Vec<CollectorReplayEvent>,
}

impl ReplayCollector {
    fn replay<C: StreamCollector>(self, collector: &mut C) {
        for event in self.events {
            match event {
                CollectorReplayEvent::DeltaOps(choice_idx, ops) => {
                    collector.on_delta_ops(choice_idx, ops);
                }
                CollectorReplayEvent::Usage(usage) => {
                    collector.on_usage(&usage);
                }
                CollectorReplayEvent::Finish(choice_idx, finish_reason) => {
                    collector.on_finish(choice_idx, finish_reason);
                }
            }
        }
    }
}

impl StreamCollector for ReplayCollector {
    fn on_delta_ops(&mut self, choice_idx: usize, ops: Vec<DeltaOp>) {
        self.events
            .push(CollectorReplayEvent::DeltaOps(choice_idx, ops));
    }

    fn on_usage(&mut self, usage: &ChatUsage) {
        self.events.push(CollectorReplayEvent::Usage(usage.clone()));
    }

    fn on_finish(&mut self, choice_idx: usize, finish_reason: Option<String>) {
        self.events
            .push(CollectorReplayEvent::Finish(choice_idx, finish_reason));
    }
}

const FUNCTION_CALLS_OPEN_TAG: &str = "<function_calls>";
const FUNCTION_CALLS_CLOSE_TAG: &str = "</function_calls>";

fn find_function_calls_close_tag(text: &str) -> Option<usize> {
    text.find(FUNCTION_CALLS_CLOSE_TAG)
}

fn split_with_partial_literal_suffix<'a>(text: &'a str, tag: &str) -> (&'a str, &'a str) {
    let max_len = tag.len().saturating_sub(1).min(text.len());
    for len in (1..=max_len).rev() {
        let suffix_start = text.len() - len;
        let suffix = &text[suffix_start..];
        if tag.starts_with(suffix) {
            return (&text[..suffix_start], suffix);
        }
    }
    (text, "")
}

fn route_append_content_without_function_calls(
    acc: &mut ChoiceAccumulator,
    ops: &mut Vec<DeltaOp>,
    incoming_text: String,
    block_index: Option<u64>,
) {
    if !acc.inside_function_calls_tag
        && acc.pending_function_calls_parse.is_empty()
        && !incoming_text.contains('<')
    {
        handle_append_content_delta(acc, ops, incoming_text, block_index);
        return;
    }

    acc.pending_function_calls_parse.push_str(&incoming_text);

    loop {
        if acc.inside_function_calls_tag {
            if let Some(close_idx) =
                find_function_calls_close_tag(&acc.pending_function_calls_parse)
            {
                let drain_until = close_idx + FUNCTION_CALLS_CLOSE_TAG.len();
                acc.function_calls_xml_raw
                    .push_str(&acc.pending_function_calls_parse[..drain_until]);
                acc.pending_function_calls_parse.drain(..drain_until);
                acc.inside_function_calls_tag = false;
                continue;
            }
            let (_, keep) = split_with_partial_literal_suffix(
                &acc.pending_function_calls_parse,
                FUNCTION_CALLS_CLOSE_TAG,
            );
            let consumed_len = acc.pending_function_calls_parse.len() - keep.len();
            acc.function_calls_xml_raw
                .push_str(&acc.pending_function_calls_parse[..consumed_len]);
            acc.pending_function_calls_parse = keep.to_string();
            break;
        }

        if let Some(open_idx) = acc
            .pending_function_calls_parse
            .find(FUNCTION_CALLS_OPEN_TAG)
        {
            let content_text = acc.pending_function_calls_parse[..open_idx].to_string();
            handle_append_content_delta(acc, ops, content_text, block_index);
            let drain_until = open_idx + FUNCTION_CALLS_OPEN_TAG.len();
            acc.function_calls_xml_raw
                .push_str(&acc.pending_function_calls_parse[open_idx..drain_until]);
            acc.pending_function_calls_parse.drain(..drain_until);
            acc.inside_function_calls_tag = true;
            continue;
        }

        let (emit, keep) = split_with_partial_literal_suffix(
            &acc.pending_function_calls_parse,
            FUNCTION_CALLS_OPEN_TAG,
        );
        let content_text = emit.to_string();
        acc.pending_function_calls_parse = keep.to_string();
        handle_append_content_delta(acc, ops, content_text, block_index);
        break;
    }
}

fn flush_pending_function_calls_parse(acc: &mut ChoiceAccumulator, ops: &mut Vec<DeltaOp>) {
    if acc.pending_function_calls_parse.is_empty() {
        return;
    }

    let pending = std::mem::take(&mut acc.pending_function_calls_parse);
    if acc.inside_function_calls_tag {
        acc.function_calls_xml_raw.push_str(&pending);
        return;
    }
    handle_append_content_delta(acc, ops, pending, None);
}

const THINK_OPEN_TAG: &str = "<think>";
const THINK_CLOSE_TAG: &str = "</think>";

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    for (idx, _) in haystack.match_indices('<') {
        if idx + needle.len() > haystack.len() {
            continue;
        }
        if let Some(candidate) = haystack.get(idx..idx + needle.len()) {
            if candidate.eq_ignore_ascii_case(needle) {
                return Some(idx);
            }
        }
    }
    None
}

fn split_with_partial_tag_suffix<'a>(text: &'a str, tag: &str) -> (&'a str, &'a str) {
    if let Some(last_lt) = text.rfind('<') {
        let suffix = &text[last_lt..];
        if suffix.len() < tag.len() {
            if let Some(tag_prefix) = tag.get(..suffix.len()) {
                if suffix.eq_ignore_ascii_case(tag_prefix) {
                    return (&text[..last_lt], suffix);
                }
            }
        }
    }
    (text, "")
}

fn push_content_delta(
    acc: &mut ChoiceAccumulator,
    ops: &mut Vec<DeltaOp>,
    text: String,
    block_index: Option<u64>,
) {
    if text.is_empty() {
        return;
    }
    acc.content.push_str(&text);
    if let Some(idx) = block_index {
        acc.content_per_block
            .entry(idx)
            .or_default()
            .push_str(&text);
    }
    ops.push(DeltaOp::AppendContent { text });
}

fn push_reasoning_delta(
    acc: &mut ChoiceAccumulator,
    ops: &mut Vec<DeltaOp>,
    text: String,
    block_index: Option<u64>,
) {
    if text.is_empty() {
        return;
    }
    acc.reasoning.push_str(&text);
    if let Some(idx) = block_index {
        acc.reasoning_per_block
            .entry(idx)
            .or_default()
            .push_str(&text);
    }
    ops.push(DeltaOp::AppendReasoning { text });
}

fn route_append_content_with_think_tags(
    acc: &mut ChoiceAccumulator,
    ops: &mut Vec<DeltaOp>,
    incoming_text: String,
    block_index: Option<u64>,
) {
    if !acc.inside_think_tag && acc.pending_think_parse.is_empty() && !incoming_text.contains('<') {
        push_content_delta(acc, ops, incoming_text, block_index);
        return;
    }

    acc.pending_think_parse.push_str(&incoming_text);

    loop {
        if acc.inside_think_tag {
            if let Some(close_idx) =
                find_ascii_case_insensitive(&acc.pending_think_parse, THINK_CLOSE_TAG)
            {
                let reasoning_text = acc.pending_think_parse[..close_idx].to_string();
                push_reasoning_delta(acc, ops, reasoning_text, block_index);
                let drain_until = close_idx + THINK_CLOSE_TAG.len();
                acc.pending_think_parse.drain(..drain_until);
                acc.inside_think_tag = false;
                continue;
            }

            let (emit, keep) =
                split_with_partial_tag_suffix(&acc.pending_think_parse, THINK_CLOSE_TAG);
            let reasoning_text = emit.to_string();
            let keep_text = keep.to_string();
            push_reasoning_delta(acc, ops, reasoning_text, block_index);
            acc.pending_think_parse = keep_text;
            break;
        }

        if let Some(open_idx) =
            find_ascii_case_insensitive(&acc.pending_think_parse, THINK_OPEN_TAG)
        {
            let content_text = acc.pending_think_parse[..open_idx].to_string();
            push_content_delta(acc, ops, content_text, block_index);
            let drain_until = open_idx + THINK_OPEN_TAG.len();
            acc.pending_think_parse.drain(..drain_until);
            acc.inside_think_tag = true;
            continue;
        }

        let (emit, keep) = split_with_partial_tag_suffix(&acc.pending_think_parse, THINK_OPEN_TAG);
        let content_text = emit.to_string();
        let keep_text = keep.to_string();
        push_content_delta(acc, ops, content_text, block_index);
        acc.pending_think_parse = keep_text;
        break;
    }
}

fn flush_pending_think_parse(acc: &mut ChoiceAccumulator, ops: &mut Vec<DeltaOp>) {
    if acc.pending_think_parse.is_empty() {
        return;
    }

    let pending = std::mem::take(&mut acc.pending_think_parse);
    if acc.inside_think_tag {
        push_reasoning_delta(acc, ops, pending, None);
    } else {
        push_content_delta(acc, ops, pending, None);
    }
}

fn handle_append_content_delta(
    acc: &mut ChoiceAccumulator,
    ops: &mut Vec<DeltaOp>,
    text: String,
    block_index: Option<u64>,
) {
    if block_index.is_some() {
        flush_pending_think_parse(acc, ops);
        push_content_delta(acc, ops, text, block_index);
    } else {
        route_append_content_with_think_tags(acc, ops, text, block_index);
    }
}

fn process_stream_event_data<C: StreamCollector>(
    adapter: &dyn crate::llm::adapter::LlmWireAdapter,
    auth_token: &str,
    data: &str,
    accumulators: &mut [ChoiceAccumulator],
    collector: &mut C,
    malformed_is_fatal: bool,
) -> Result<bool, String> {
    let deltas = match adapter.parse_stream_chunk(data) {
        Ok(d) => d,
        Err(StreamParseError::Skip) => return Ok(false),
        Err(StreamParseError::MalformedChunk(e)) if malformed_is_fatal => {
            return Err(format!("Malformed stream chunk: {}", e));
        }
        Err(StreamParseError::MalformedChunk(e)) => {
            tracing::warn!("Malformed stream chunk: {}", e);
            return Ok(false);
        }
        Err(StreamParseError::FatalError(e)) => {
            return Err(format!("LLM error: {}", e));
        }
    };

    let acc = &mut accumulators[0];
    let mut ops = Vec::new();
    let mut stream_done = false;

    for delta in deltas {
        match delta {
            LlmStreamDelta::AppendContent { text, block_index } => {
                route_append_content_without_function_calls(acc, &mut ops, text, block_index);
            }
            LlmStreamDelta::AppendReasoning { text, block_index } => {
                flush_pending_think_parse(acc, &mut ops);
                push_reasoning_delta(acc, &mut ops, text, block_index);
            }
            LlmStreamDelta::SetToolCalls { tool_calls } => {
                let tool_calls = if !auth_token.is_empty() {
                    tool_calls
                        .into_iter()
                        .map(|mut tc| {
                            strip_mcp_prefix_from_tool_call(&mut tc);
                            tc
                        })
                        .collect()
                } else {
                    tool_calls
                };
                for tc in &tool_calls {
                    acc.tool_calls.merge(tc);
                }
                ops.push(DeltaOp::SetToolCalls {
                    tool_calls: acc.tool_calls.finalize(),
                });
            }
            LlmStreamDelta::FinalizeToolCalls { tool_calls } => {
                let tool_calls = if !auth_token.is_empty() {
                    tool_calls
                        .into_iter()
                        .map(|mut tc| {
                            strip_mcp_prefix_from_tool_call(&mut tc);
                            tc
                        })
                        .collect()
                } else {
                    tool_calls
                };
                for tc in &tool_calls {
                    acc.tool_calls.set_final(tc);
                }
                ops.push(DeltaOp::SetToolCalls {
                    tool_calls: acc.tool_calls.finalize(),
                });
            }
            LlmStreamDelta::SetThinkingBlocks { blocks } => {
                merge_thinking_blocks(&mut acc.thinking_blocks, blocks);
                ops.push(DeltaOp::SetThinkingBlocks {
                    blocks: acc.thinking_blocks.clone(),
                });
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
                    ops.push(DeltaOp::SetUsage {
                        usage: json!(merged),
                    });
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

    if stream_done {
        flush_pending_function_calls_parse(acc, &mut ops);
    }

    if !ops.is_empty() {
        collector.on_delta_ops(0, ops);
    }

    Ok(stream_done)
}

const MAX_NDJSON_LINE_BYTES: usize = 8 * 1024 * 1024;

fn ndjson_line_size_error(size: usize) -> String {
    format!("Ollama NDJSON line exceeds {MAX_NDJSON_LINE_BYTES} bytes ({size} bytes)")
}

fn process_ndjson_bytes<C: StreamCollector>(
    adapter: &dyn crate::llm::adapter::LlmWireAdapter,
    auth_token: &str,
    pending: &mut Vec<u8>,
    bytes: &[u8],
    accumulators: &mut [ChoiceAccumulator],
    collector: &mut C,
) -> Result<bool, String> {
    for segment in bytes.split_inclusive(|b| *b == b'\n') {
        let segment_line_len = segment
            .iter()
            .position(|b| *b == b'\n')
            .unwrap_or(segment.len());
        let next_line_len = pending.len().saturating_add(segment_line_len);
        if next_line_len > MAX_NDJSON_LINE_BYTES {
            return Err(ndjson_line_size_error(next_line_len));
        }

        pending.extend_from_slice(segment);
        if process_complete_ndjson_lines(adapter, auth_token, pending, accumulators, collector)? {
            return Ok(true);
        }
        if pending.len() > MAX_NDJSON_LINE_BYTES {
            return Err(ndjson_line_size_error(pending.len()));
        }
    }
    Ok(false)
}

fn process_complete_ndjson_lines<C: StreamCollector>(
    adapter: &dyn crate::llm::adapter::LlmWireAdapter,
    auth_token: &str,
    pending: &mut Vec<u8>,
    accumulators: &mut [ChoiceAccumulator],
    collector: &mut C,
) -> Result<bool, String> {
    loop {
        let Some(pos) = pending.iter().position(|b| *b == b'\n') else {
            if pending.len() > MAX_NDJSON_LINE_BYTES {
                return Err(ndjson_line_size_error(pending.len()));
            }
            return Ok(false);
        };
        let mut line: Vec<u8> = pending.drain(..=pos).collect();
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.len() > MAX_NDJSON_LINE_BYTES {
            return Err(ndjson_line_size_error(line.len()));
        }
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        let data = std::str::from_utf8(&line)
            .map_err(|e| format!("Malformed stream chunk: utf8: {}", e))?;
        if process_stream_event_data(adapter, auth_token, data, accumulators, collector, true)? {
            return Ok(true);
        }
    }
}

fn process_ndjson_eof<C: StreamCollector>(
    adapter: &dyn crate::llm::adapter::LlmWireAdapter,
    auth_token: &str,
    pending: &mut Vec<u8>,
    accumulators: &mut [ChoiceAccumulator],
    collector: &mut C,
) -> Result<bool, String> {
    if pending.len() > MAX_NDJSON_LINE_BYTES {
        return Err(ndjson_line_size_error(pending.len()));
    }
    if pending.iter().all(|b| b.is_ascii_whitespace()) {
        pending.clear();
        return Ok(false);
    }
    let mut line = std::mem::take(pending);
    while line.last().is_some_and(|b| b.is_ascii_whitespace()) {
        line.pop();
    }
    if line.len() > MAX_NDJSON_LINE_BYTES {
        return Err(ndjson_line_size_error(line.len()));
    }
    let data =
        std::str::from_utf8(&line).map_err(|e| format!("Malformed stream chunk: utf8: {}", e))?;
    process_stream_event_data(adapter, auth_token, data, accumulators, collector, true)
}

fn finalize_accumulators<C: StreamCollector>(
    mut accumulators: Vec<ChoiceAccumulator>,
    collector: &mut C,
) -> Vec<ChoiceFinal> {
    for (idx, acc) in accumulators.iter_mut().enumerate() {
        let mut tail_ops = Vec::new();
        flush_pending_function_calls_parse(acc, &mut tail_ops);
        flush_pending_think_parse(acc, &mut tail_ops);
        if !tail_ops.is_empty() {
            collector.on_delta_ops(idx, tail_ops);
        }
    }

    accumulators
        .into_iter()
        .enumerate()
        .map(|(idx, acc)| {
            collector.on_finish(idx, acc.finish_reason.clone());
            let thinking_blocks = if !acc.thinking_blocks.is_empty() && !acc.reasoning.is_empty() {
                acc.thinking_blocks
                    .into_iter()
                    .map(|mut block| {
                        if let Some(obj) = block.as_object_mut() {
                            let is_anthropic_thinking =
                                obj.get("type").and_then(|t| t.as_str()) == Some("thinking");
                            let thinking_is_empty = obj
                                .get("thinking")
                                .and_then(|v| v.as_str())
                                .map_or(true, |s| s.trim().is_empty());
                            if is_anthropic_thinking && thinking_is_empty {
                                let block_idx = obj.get("index").and_then(|v| v.as_u64());
                                let reasoning_text = block_idx
                                    .and_then(|idx| acc.reasoning_per_block.get(&idx))
                                    .unwrap_or(&acc.reasoning);
                                if !reasoning_text.is_empty() {
                                    obj.insert(
                                        "thinking".to_string(),
                                        json!(reasoning_text.clone()),
                                    );
                                }
                            }
                        }
                        block
                    })
                    .collect()
            } else if acc.thinking_blocks.is_empty() && !acc.reasoning.is_empty() {
                vec![json!({
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": acc.reasoning.clone()}]
                })]
            } else {
                acc.thinking_blocks
            };

            let raw_content = format!(
                "{}{}{}",
                acc.content, acc.function_calls_xml_raw, acc.pending_function_calls_parse
            );

            ChoiceFinal {
                content: acc.content,
                raw_content: raw_content,
                reasoning: acc.reasoning,
                thinking_blocks,
                tool_calls_raw: acc.tool_calls.finalize(),
                citations: acc.citations,
                server_content_blocks: acc.server_content_blocks,
                extra: {
                    let mut extra = acc.extra;
                    if !acc.content_per_block.is_empty() {
                        let mut text_blocks: Vec<_> = acc.content_per_block.into_iter().collect();
                        text_blocks.sort_by_key(|(idx, _)| *idx);
                        extra.insert(
                            "_anthropic_text_blocks".to_string(),
                            json!(text_blocks
                                .into_iter()
                                .map(|(idx, text)| { json!({"index": idx, "text": text}) })
                                .collect::<Vec<_>>()),
                        );
                    }
                    extra
                },
                finish_reason: acc.finish_reason,
                usage: acc.usage,
            }
        })
        .collect()
}

fn openai_codex_websocket_endpoint(model_rec: &BaseModelRecord) -> Option<&str> {
    if !is_openai_codex_chatgpt_backend(model_rec) {
        return None;
    }
    model_rec
        .extra_headers
        .get(crate::providers::openai_codex::CODEX_WEBSOCKET_ENDPOINT_HEADER)
        .map(String::as_str)
        .filter(|endpoint| !endpoint.trim().is_empty())
}

fn build_openai_codex_websocket_message(body: &Value) -> Value {
    let mut message = body.clone();
    if let Some(obj) = message.as_object_mut() {
        obj.insert("type".to_string(), json!("response.create"));
        message
    } else {
        json!({"type": "response.create", "request": body})
    }
}

fn websocket_header_entries(
    headers: &reqwest::header::HeaderMap,
) -> Result<Vec<(String, String)>, String> {
    let mut entries = Vec::new();
    for (key, value) in headers {
        let key = key.as_str().to_ascii_lowercase();
        if key == "accept"
            || key == "content-type"
            || key == "content-length"
            || key == "connection"
            || key == "host"
            || key.starts_with("sec-websocket-")
            || key.starts_with("x-refact-internal-")
        {
            continue;
        }
        let value = value
            .to_str()
            .map_err(|e| format!("WebSocket header '{}' is not valid UTF-8: {}", key, e))?;
        entries.push((key, value.to_string()));
    }
    entries.push((
        "openai-beta".to_string(),
        "responses_websockets=2026-02-06".to_string(),
    ));
    Ok(entries)
}

async fn commit_cache_guard_snapshot_if_needed(
    app: AppState,
    chat_id: Option<&String>,
    sanitized_for_commit: Option<serde_json::Value>,
) {
    let (Some(chat_id), Some(sanitized)) = (chat_id, sanitized_for_commit) else {
        return;
    };
    let session_arc_opt = {
        let sessions = app.chat.sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    if let Some(session_arc) = session_arc_opt {
        crate::chat::cache_guard::commit_cache_guard_snapshot(session_arc, sanitized).await;
    }
}

async fn run_llm_websocket_request<C: StreamCollector>(
    websocket_endpoint: &str,
    http_parts: &HttpParts,
    adapter: &dyn crate::llm::adapter::LlmWireAdapter,
    auth_token: &str,
    abort_flag: Option<Arc<AtomicBool>>,
    collector: &mut C,
) -> Result<Vec<ChoiceFinal>, String> {
    let mut request = websocket_endpoint
        .into_client_request()
        .map_err(|e| format!("OpenAI Codex WebSocket request build failed: {}", e))?;
    {
        let headers = request.headers_mut();
        for (key, value) in websocket_header_entries(&http_parts.headers)? {
            let name = tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(key.as_bytes())
                .map_err(|e| {
                    format!("OpenAI Codex WebSocket header '{}' is invalid: {}", key, e)
                })?;
            let value = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&value)
                .map_err(|e| {
                    format!(
                        "OpenAI Codex WebSocket header '{}' value is invalid: {}",
                        key, e
                    )
                })?;
            headers.insert(name, value);
        }
    }

    let (mut websocket, _) = tokio::time::timeout(
        stream_idle_timeout(),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map_err(|_| "OpenAI Codex WebSocket connect timed out".to_string())?
    .map_err(|e| format!("OpenAI Codex WebSocket connect failed: {}", e))?;
    websocket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            build_openai_codex_websocket_message(&http_parts.body).to_string(),
        ))
        .await
        .map_err(|e| format!("OpenAI Codex WebSocket send failed: {}", e))?;

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
        let message = tokio::select! {
            _ = heartbeat.tick() => {
                if let Some(ref flag) = abort_flag {
                    if flag.load(Ordering::SeqCst) {
                        return Err("Aborted".to_string());
                    }
                }
                if stream_started_at.elapsed() > stream_total_timeout() {
                    return Err("OpenAI Codex WebSocket stream timeout".to_string());
                }
                if last_event_at.elapsed() > stream_idle_timeout() {
                    return Err("OpenAI Codex WebSocket stream stalled".to_string());
                }
                continue;
            }
            maybe_message = websocket.next() => {
                match maybe_message {
                    Some(Ok(message)) => message,
                    Some(Err(e)) => {
                        return Err(format!("OpenAI Codex WebSocket stream error: {}", e));
                    }
                    None => {
                        if !stream_done {
                            return Err("OpenAI Codex WebSocket ended before completion".to_string());
                        }
                        break;
                    }
                }
            }
        };
        last_event_at = Instant::now();

        let data = match message {
            tokio_tungstenite::tungstenite::Message::Text(text) => text,
            tokio_tungstenite::tungstenite::Message::Binary(bytes) => String::from_utf8(bytes)
                .map_err(|e| {
                    format!("OpenAI Codex WebSocket binary message was not UTF-8: {}", e)
                })?,
            tokio_tungstenite::tungstenite::Message::Ping(_)
            | tokio_tungstenite::tungstenite::Message::Pong(_) => continue,
            tokio_tungstenite::tungstenite::Message::Close(_) => {
                if stream_done {
                    break;
                }
                return Err("OpenAI Codex WebSocket closed before completion".to_string());
            }
            tokio_tungstenite::tungstenite::Message::Frame(_) => continue,
        };

        stream_done = process_stream_event_data(
            adapter,
            auth_token,
            &data,
            &mut accumulators,
            collector,
            true,
        )?;
    }

    Ok(finalize_accumulators(accumulators, collector))
}

async fn run_llm_ndjson_request<C: StreamCollector>(
    response: reqwest::Response,
    adapter: &dyn crate::llm::adapter::LlmWireAdapter,
    auth_token: &str,
    abort_flag: Option<Arc<AtomicBool>>,
    collector: &mut C,
) -> Result<Vec<ChoiceFinal>, String> {
    let mut stream = response.bytes_stream();
    let mut pending = Vec::new();
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
        let bytes = tokio::select! {
            _ = heartbeat.tick() => {
                if let Some(ref flag) = abort_flag {
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
            maybe_bytes = stream.next() => {
                match maybe_bytes {
                    Some(Ok(bytes)) => bytes,
                    Some(Err(e)) => {
                        return Err(format!("Stream error: {}", e));
                    }
                    None => {
                        if process_ndjson_eof(
                            adapter,
                            auth_token,
                            &mut pending,
                            &mut accumulators,
                            collector,
                        )? {
                            break;
                        }
                        return Err("LLM stream ended unexpectedly without completion signal".to_string());
                    }
                }
            }
        };
        last_event_at = Instant::now();
        stream_done = process_ndjson_bytes(
            adapter,
            auth_token,
            &mut pending,
            &bytes,
            &mut accumulators,
            collector,
        )?;
    }

    Ok(finalize_accumulators(accumulators, collector))
}

pub async fn run_llm_stream<C: StreamCollector>(
    app: AppState,
    params: StreamRunParams,
    collector: &mut C,
) -> Result<Vec<ChoiceFinal>, String> {
    if params.llm_request.params.n.unwrap_or(1) != 1 {
        return Err("Streaming with n > 1 is not supported".to_string());
    }

    let client = app.runtime.http_client.clone();
    let slowdown_arc = app.runtime.http_client_slowdown.clone();

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
        eof_is_done: params.model_rec.eof_is_done,
        supports_web_search: params.model_rec.supports_web_search,
        supports_cache_control: params.model_rec.supports_cache_control,
    };

    let http_parts = adapter
        .build_http(&params.llm_request, &adapter_settings)
        .map_err(|e| format!("Failed to build LLM request: {}", e))?;

    let mut sanitized_for_commit: Option<serde_json::Value> = None;
    if let Some(chat_id) = &params.chat_id {
        let session_arc_opt = {
            let sessions = app.chat.sessions.read().await;
            sessions.get(chat_id).cloned()
        };
        if let Some(session_arc) = session_arc_opt {
            sanitized_for_commit = crate::chat::cache_guard::check_or_pause_cache_guard(
                app.clone(),
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

    if let Some(websocket_endpoint) = openai_codex_websocket_endpoint(&params.model_rec) {
        let mut replay_collector = ReplayCollector::default();
        match run_llm_websocket_request(
            websocket_endpoint,
            &http_parts,
            adapter,
            &params.model_rec.auth_token,
            params.abort_flag.clone(),
            &mut replay_collector,
        )
        .await
        {
            Ok(results) => {
                commit_cache_guard_snapshot_if_needed(
                    app.clone(),
                    params.chat_id.as_ref(),
                    sanitized_for_commit.clone(),
                )
                .await;
                replay_collector.replay(collector);
                return Ok(results);
            }
            Err(error) => {
                tracing::warn!(
                    "OpenAI Codex WebSocket streaming failed, falling back to HTTP SSE: {}",
                    error
                );
            }
        }
    }

    let mut response = send_llm_http_request(&client, &http_parts, wire_format).await?;
    let mut status = response.status();
    if !status.is_success()
        && is_openai_codex_chatgpt_backend(&params.model_rec)
        && matches!(
            status,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        )
    {
        let provider_instance_id =
            openai_codex_instance_id(&params.model_rec).unwrap_or("openai_codex");
        match force_refresh_openai_codex_for_retry(
            app.clone(),
            &client,
            provider_instance_id,
            status,
            &params.model_rec.api_key,
        )
        .await?
        {
            Some(new_access_token) => {
                let mut retry_parts = HttpParts {
                    url: http_parts.url.clone(),
                    headers: http_parts.headers.clone(),
                    body: http_parts.body.clone(),
                };
                let auth_value =
                    reqwest::header::HeaderValue::from_str(&format!("Bearer {}", new_access_token))
                        .map_err(|e| {
                            format!("OpenAI Codex refreshed token cannot be used: {}", e)
                        })?;
                retry_parts
                    .headers
                    .insert(reqwest::header::AUTHORIZATION, auth_value);
                response = send_llm_http_request(&client, &retry_parts, wire_format).await?;
                status = response.status();
            }
            None => {}
        }
    }

    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        if should_commit_cache_guard_after_http_success(status, &text) {
            commit_cache_guard_snapshot_if_needed(
                app.clone(),
                params.chat_id.as_ref(),
                sanitized_for_commit,
            )
            .await;
        }
        return Err(format_llm_error_body(&format!("{}", status), &text));
    }

    commit_cache_guard_snapshot_if_needed(
        app.clone(),
        params.chat_id.as_ref(),
        sanitized_for_commit,
    )
    .await;

    if wire_format == WireFormat::OllamaNative {
        return run_llm_ndjson_request(
            response,
            adapter,
            &params.model_rec.auth_token,
            params.abort_flag.clone(),
            collector,
        )
        .await;
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

        stream_done = process_stream_event_data(
            adapter,
            &params.model_rec.auth_token,
            &event.data,
            &mut accumulators,
            collector,
            false,
        )?;
    }

    let results = finalize_accumulators(accumulators, collector);

    Ok(results)
}

/// Merges incoming thinking blocks into the accumulator, deduplicating by:
/// 1. `id` field (if present)
/// 2. `(type, index)` pair (Anthropic signature deltas)
/// 3. `(type, signature)` pair (LiteLLM blocks without index)
///
/// When a duplicate is found, the existing block's signature is updated
/// to the latest value (handles streaming signature updates).
pub(crate) fn merge_thinking_blocks(
    dst: &mut Vec<serde_json::Value>,
    incoming: Vec<serde_json::Value>,
) {
    for block in incoming {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let block_id = block.get("id").and_then(|v| v.as_str());
        let block_index = block.get("index").and_then(|v| v.as_u64());
        let block_sig = block.get("signature").and_then(|v| v.as_str());

        let existing_idx = if let Some(id) = block_id {
            dst.iter()
                .position(|b| b.get("id").and_then(|v| v.as_str()) == Some(id))
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
    pending_think_parse: String,
    inside_think_tag: bool,
    pending_function_calls_parse: String,
    inside_function_calls_tag: bool,
    function_calls_xml_raw: String,
}

fn strip_mcp_prefix_from_tool_call(tc: &mut serde_json::Value) {
    if let Some(func) = tc.get_mut("function") {
        if let Some(name) = func
            .get("name")
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
        {
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
        Some(serde_json::Value::Object(_)) => {
            serde_json::to_string(&function["arguments"]).unwrap_or_else(|_| "{}".to_string())
        }
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
    fn websocket_endpoint_requires_codex_backend_and_internal_marker() {
        let mut model = BaseModelRecord {
            id: "openai_codex/gpt-5.6-codex".to_string(),
            endpoint: "https://chatgpt.com/backend-api/codex/responses".to_string(),
            ..Default::default()
        };

        assert!(openai_codex_websocket_endpoint(&model).is_none());

        model.extra_headers.insert(
            crate::providers::openai_codex::CODEX_WEBSOCKET_ENDPOINT_HEADER.to_string(),
            "wss://chatgpt.com/backend-api/codex/responses".to_string(),
        );
        assert_eq!(
            openai_codex_websocket_endpoint(&model),
            Some("wss://chatgpt.com/backend-api/codex/responses")
        );

        model.endpoint = "https://api.openai.com/v1/responses".to_string();
        assert!(openai_codex_websocket_endpoint(&model).is_none());

        model.endpoint = "https://chatgpt.com/backend-api/codex/responses".to_string();
        model.id = "openai/gpt-5.6-codex".to_string();
        assert!(openai_codex_websocket_endpoint(&model).is_none());
    }

    #[test]
    fn websocket_create_message_wraps_responses_body() {
        let body = json!({
            "model": "gpt-5.6-codex",
            "stream": true,
            "input": []
        });

        let message = build_openai_codex_websocket_message(&body);

        assert_eq!(message["type"], json!("response.create"));
        assert_eq!(message["model"], json!("gpt-5.6-codex"));
        assert_eq!(message["stream"], json!(true));
    }

    #[test]
    fn cache_guard_is_not_committed_for_retryable_http_errors() {
        assert!(!should_commit_cache_guard_after_http_success(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"message":"rate limit"}}"#,
        ));
        assert!(!should_commit_cache_guard_after_http_success(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            r#"{"error":{"message":"overloaded"}}"#,
        ));
    }

    #[test]
    fn cache_guard_is_committed_for_non_retryable_http_errors() {
        assert!(should_commit_cache_guard_after_http_success(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":{"message":"context length exceeded"}}"#,
        ));
        assert!(should_commit_cache_guard_after_http_success(
            reqwest::StatusCode::UNAUTHORIZED,
            r#"{"error":{"message":"invalid api key"}}"#,
        ));
    }

    #[test]
    fn function_calls_xml_is_not_emitted_as_content() {
        let mut acc = ChoiceAccumulator::default();
        let mut ops = Vec::new();

        route_append_content_without_function_calls(
            &mut acc,
            &mut ops,
            "Before <function_calls><invoke name=\"mcp_tool_search\"><parameter name=\"query\">x"
                .to_string(),
            None,
        );
        route_append_content_without_function_calls(
            &mut acc,
            &mut ops,
            "</parameter></invoke></function_calls> after".to_string(),
            None,
        );
        flush_pending_function_calls_parse(&mut acc, &mut ops);

        assert_eq!(acc.content, "Before  after");
        assert!(ops.iter().all(|op| match op {
            DeltaOp::AppendContent { text } =>
                !text.contains("<function_calls>")
                    && !text.contains("<invoke")
                    && !text.contains("<parameter"),
            _ => true,
        }));
    }

    #[test]
    fn partial_function_calls_open_tag_is_buffered() {
        let mut acc = ChoiceAccumulator::default();
        let mut ops = Vec::new();

        route_append_content_without_function_calls(
            &mut acc,
            &mut ops,
            "Before <function".to_string(),
            None,
        );
        assert_eq!(acc.content, "Before ");
        assert_eq!(acc.pending_function_calls_parse, "<function");

        route_append_content_without_function_calls(
            &mut acc,
            &mut ops,
            "_calls>hidden</function_calls> after".to_string(),
            None,
        );
        flush_pending_function_calls_parse(&mut acc, &mut ops);

        assert_eq!(acc.content, "Before  after");
    }

    #[test]
    fn websocket_headers_override_beta_and_omit_http_only_metadata() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_static("Bearer tok"),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("text/event-stream"),
        );
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            "OpenAI-Beta",
            reqwest::header::HeaderValue::from_static("responses=experimental"),
        );
        headers.insert(
            crate::providers::openai_codex::CODEX_WEBSOCKET_ENDPOINT_HEADER,
            reqwest::header::HeaderValue::from_static(
                "wss://chatgpt.com/backend-api/codex/responses",
            ),
        );

        let entries = websocket_header_entries(&headers).unwrap();
        let map: HashMap<_, _> = entries.into_iter().collect();

        assert_eq!(
            map.get("authorization").map(String::as_str),
            Some("Bearer tok")
        );
        assert_eq!(
            map.get("openai-beta").map(String::as_str),
            Some("responses_websockets=2026-02-06")
        );
        assert!(!map.contains_key("accept"));
        assert!(!map.contains_key("content-type"));
        assert!(!map.contains_key(crate::providers::openai_codex::CODEX_WEBSOCKET_ENDPOINT_HEADER));
    }

    #[test]
    fn websocket_parse_errors_are_fatal_before_replay() {
        let adapter = get_adapter(crate::llm::WireFormat::OpenaiResponses);
        let mut accumulators = vec![ChoiceAccumulator::default()];
        let mut collector = ReplayCollector::default();

        let err = process_stream_event_data(
            adapter,
            "",
            "not json",
            &mut accumulators,
            &mut collector,
            true,
        )
        .unwrap_err();

        assert!(err.contains("Malformed stream chunk"));
        assert!(collector.events.is_empty());
    }

    #[test]
    fn ollama_ndjson_handles_split_and_multiple_lines() {
        let adapter = get_adapter(crate::llm::WireFormat::OllamaNative);
        let mut accumulators = vec![ChoiceAccumulator::default()];
        let mut collector = ReplayCollector::default();
        let mut pending = Vec::new();

        let done = process_ndjson_bytes(
            adapter,
            "",
            &mut pending,
            br#"{"message":{"content":"Hel"#,
            &mut accumulators,
            &mut collector,
        )
        .unwrap();
        assert!(!done);
        assert!(collector.events.is_empty());

        let done = process_ndjson_bytes(
            adapter,
            "",
            &mut pending,
            br#"lo"}}
{"message":{"content":"!"}}
{"prompt_eval_count":7,"eval_count":3,"done":true}
"#,
            &mut accumulators,
            &mut collector,
        )
        .unwrap();

        assert!(done);
        assert_eq!(accumulators[0].content, "Hello!");
        let usage = accumulators[0].usage.as_ref().unwrap();
        assert_eq!(usage.prompt_tokens, 7);
        assert_eq!(usage.completion_tokens, 3);
        assert!(pending.is_empty());
    }

    #[test]
    fn ollama_ndjson_malformed_line_is_fatal() {
        let adapter = get_adapter(crate::llm::WireFormat::OllamaNative);
        let mut accumulators = vec![ChoiceAccumulator::default()];
        let mut collector = ReplayCollector::default();
        let mut pending = Vec::new();

        let err = process_ndjson_bytes(
            adapter,
            "",
            &mut pending,
            b"not-json\n",
            &mut accumulators,
            &mut collector,
        )
        .unwrap_err();

        assert!(err.contains("Malformed stream chunk"));
        assert!(collector.events.is_empty());
    }

    #[test]
    fn ollama_ndjson_oversized_complete_line_is_fatal() {
        let adapter = get_adapter(crate::llm::WireFormat::OllamaNative);
        let mut accumulators = vec![ChoiceAccumulator::default()];
        let mut collector = ReplayCollector::default();
        let mut pending = Vec::new();
        let mut line = vec![b' '; MAX_NDJSON_LINE_BYTES + 1];
        line.push(b'\n');

        let err = process_ndjson_bytes(
            adapter,
            "",
            &mut pending,
            &line,
            &mut accumulators,
            &mut collector,
        )
        .unwrap_err();

        assert!(err.contains("exceeds"));
        assert!(collector.events.is_empty());
    }

    #[test]
    fn ollama_ndjson_oversized_pending_buffer_is_fatal() {
        let adapter = get_adapter(crate::llm::WireFormat::OllamaNative);
        let mut accumulators = vec![ChoiceAccumulator::default()];
        let mut collector = ReplayCollector::default();
        let mut pending = Vec::new();
        let bytes = vec![b'a'; MAX_NDJSON_LINE_BYTES + 1];

        let err = process_ndjson_bytes(
            adapter,
            "",
            &mut pending,
            &bytes,
            &mut accumulators,
            &mut collector,
        )
        .unwrap_err();

        assert!(err.contains("exceeds"));
        assert!(collector.events.is_empty());
    }

    #[test]
    fn ollama_ndjson_processes_final_line_without_newline() {
        let adapter = get_adapter(crate::llm::WireFormat::OllamaNative);
        let mut accumulators = vec![ChoiceAccumulator::default()];
        let mut collector = ReplayCollector::default();
        let mut pending = Vec::new();

        process_ndjson_bytes(
            adapter,
            "",
            &mut pending,
            br#"{"message":{"content":"done"}}"#,
            &mut accumulators,
            &mut collector,
        )
        .unwrap();
        assert_eq!(accumulators[0].content, "");

        let done = process_ndjson_eof(adapter, "", &mut pending, &mut accumulators, &mut collector)
            .unwrap();

        assert!(!done);
        assert_eq!(accumulators[0].content, "done");
        assert!(pending.is_empty());
    }

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
            acc.thinking_blocks
                .into_iter()
                .map(|mut block| {
                    if let Some(obj) = block.as_object_mut() {
                        let is_anthropic_thinking =
                            obj.get("type").and_then(|t| t.as_str()) == Some("thinking");
                        let thinking_is_empty = obj
                            .get("thinking")
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
                })
                .collect()
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
            raw_content: String::new(),
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
        acc.reasoning_per_block
            .insert(0, "First thought...".to_string());
        acc.reasoning_per_block
            .insert(4, "Second thought...".to_string());
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
        merge_thinking_blocks(
            &mut blocks,
            vec![json!({
                "index": 0,
                "type": "thinking",
                "signature": "abc"
            })],
        );
        assert_eq!(blocks[0]["signature"].as_str().unwrap(), "abc");

        // Second signature chunk — should replace, not concatenate
        merge_thinking_blocks(
            &mut blocks,
            vec![json!({
                "index": 0,
                "type": "thinking",
                "signature": "def"
            })],
        );
        assert_eq!(
            blocks[0]["signature"].as_str().unwrap(),
            "def",
            "Signature updates must replace, not concatenate"
        );

        // Third chunk
        merge_thinking_blocks(
            &mut blocks,
            vec![json!({
                "index": 0,
                "type": "thinking",
                "signature": "ghi"
            })],
        );
        assert_eq!(
            blocks[0]["signature"].as_str().unwrap(),
            "ghi",
            "Latest signature update must win"
        );
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
        assert_eq!(
            result.thinking_blocks[0]["thinking"].as_str().unwrap(),
            "Some reasoning"
        );
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

        assert!(crate::chat::cache_guard::is_append_only_prefix(
            &prev, &next_ok
        ));
        assert!(!crate::chat::cache_guard::is_append_only_prefix(
            &prev, &next_bad
        ));
    }

    #[test]
    fn test_merge_thinking_blocks_dedupe_by_index() {
        let mut dst = vec![json!({"index": 0, "type": "thinking", "signature": "sig_v1"})];

        merge_thinking_blocks(
            &mut dst,
            vec![json!({"index": 0, "type": "thinking", "signature": "sig_v2"})],
        );

        assert_eq!(dst.len(), 1, "Same (type, index) should dedupe");
        assert_eq!(
            dst[0]["signature"], "sig_v2",
            "Signature should be updated to latest"
        );
    }

    #[test]
    fn test_merge_thinking_blocks_streaming_signature_does_not_concat() {
        // Even if the upstream sends multiple signature updates, we must NOT
        // concatenate them: signatures are integrity-checked by the provider.
        let mut dst = vec![json!({"index": 0, "type": "thinking", "signature": "sig_part1"})];

        merge_thinking_blocks(
            &mut dst,
            vec![json!({"index": 0, "type": "thinking", "signature": "sig_part2"})],
        );

        assert_eq!(dst.len(), 1);
        assert_eq!(
            dst[0]["signature"], "sig_part2",
            "Signature must be replaced, not concatenated"
        );
    }

    #[test]
    fn test_merge_thinking_blocks_different_indices_kept() {
        let mut dst = Vec::new();

        merge_thinking_blocks(
            &mut dst,
            vec![
                json!({"index": 0, "type": "thinking", "signature": "sig1"}),
                json!({"index": 4, "type": "thinking", "signature": "sig2"}),
            ],
        );

        assert_eq!(
            dst.len(),
            2,
            "Different indices should produce separate blocks"
        );
    }

    #[test]
    fn test_merge_thinking_blocks_dedupe_by_signature_no_index() {
        // LiteLLM blocks often have no index — dedupe by (type, signature)
        let mut dst = vec![json!({"type": "thinking", "thinking": "text", "signature": "sig_abc"})];

        merge_thinking_blocks(
            &mut dst,
            vec![json!({"type": "thinking", "thinking": "text", "signature": "sig_abc"})],
        );

        assert_eq!(
            dst.len(),
            1,
            "Same (type, signature) without index should dedupe"
        );
    }

    #[test]
    fn test_merge_thinking_blocks_different_types_same_index_not_deduped() {
        let mut dst = vec![json!({"index": 0, "type": "thinking", "signature": "sig1"})];

        merge_thinking_blocks(
            &mut dst,
            vec![json!({"index": 0, "type": "redacted_thinking", "data": "encrypted"})],
        );

        assert_eq!(
            dst.len(),
            2,
            "Different types at same index should not dedupe"
        );
    }

    #[test]
    fn test_merge_thinking_blocks_signature_added_to_existing() {
        // First block has no signature, second adds it
        let mut dst = vec![json!({"index": 0, "type": "thinking"})];

        merge_thinking_blocks(
            &mut dst,
            vec![json!({"index": 0, "type": "thinking", "signature": "sig_new"})],
        );

        assert_eq!(dst.len(), 1);
        assert_eq!(
            dst[0]["signature"], "sig_new",
            "Signature should be added to existing block"
        );
    }

    #[test]
    fn test_merge_thinking_blocks_dedupe_by_id() {
        let mut dst = vec![json!({"id": "block_1", "type": "thinking", "signature": "sig_old"})];

        merge_thinking_blocks(
            &mut dst,
            vec![json!({"id": "block_1", "type": "thinking", "signature": "sig_new"})],
        );

        assert_eq!(dst.len(), 1, "Same id should dedupe");
        assert_eq!(dst[0]["signature"], "sig_new");
    }

    #[test]
    fn test_merge_thinking_blocks_no_key_never_dedupes() {
        // Blocks with no id, no index, no signature always append
        let mut dst = vec![json!({"type": "thinking", "thinking": "text1"})];

        merge_thinking_blocks(
            &mut dst,
            vec![json!({"type": "thinking", "thinking": "text2"})],
        );

        assert_eq!(
            dst.len(),
            2,
            "Blocks with no dedup key should always append"
        );
    }

    #[test]
    fn test_route_append_content_with_think_tags_single_chunk() {
        let mut acc = ChoiceAccumulator::default();
        let mut ops = Vec::new();

        route_append_content_with_think_tags(
            &mut acc,
            &mut ops,
            "before <think>secret</think> after".to_string(),
            None,
        );

        assert_eq!(acc.content, "before  after");
        assert_eq!(acc.reasoning, "secret");
        assert_eq!(ops.len(), 3);
        assert!(matches!(&ops[0], DeltaOp::AppendContent { text } if text == "before "));
        assert!(matches!(&ops[1], DeltaOp::AppendReasoning { text } if text == "secret"));
        assert!(matches!(&ops[2], DeltaOp::AppendContent { text } if text == " after"));
    }

    #[test]
    fn test_route_append_content_with_think_tags_split_open_and_close() {
        let mut acc = ChoiceAccumulator::default();
        let mut ops = Vec::new();

        route_append_content_with_think_tags(&mut acc, &mut ops, "before <thi".to_string(), None);
        route_append_content_with_think_tags(&mut acc, &mut ops, "nk>secret</th".to_string(), None);
        route_append_content_with_think_tags(&mut acc, &mut ops, "ink> after".to_string(), None);

        assert_eq!(acc.content, "before  after");
        assert_eq!(acc.reasoning, "secret");
        assert!(!acc.inside_think_tag);
        assert!(acc.pending_think_parse.is_empty());
    }

    #[test]
    fn test_route_append_content_with_think_tags_case_insensitive() {
        let mut acc = ChoiceAccumulator::default();
        let mut ops = Vec::new();

        route_append_content_with_think_tags(
            &mut acc,
            &mut ops,
            "A<THINK>B</THINK>C".to_string(),
            None,
        );

        assert_eq!(acc.content, "AC");
        assert_eq!(acc.reasoning, "B");
    }

    #[test]
    fn test_flush_pending_think_parse_outside_think_keeps_text() {
        let mut acc = ChoiceAccumulator::default();
        let mut ops = Vec::new();

        route_append_content_with_think_tags(&mut acc, &mut ops, "hello <thi".to_string(), None);
        flush_pending_think_parse(&mut acc, &mut ops);

        assert_eq!(acc.content, "hello <thi");
        assert_eq!(acc.reasoning, "");
    }

    #[test]
    fn test_flush_pending_think_parse_inside_think_goes_to_reasoning() {
        let mut acc = ChoiceAccumulator::default();
        let mut ops = Vec::new();

        acc.inside_think_tag = true;
        acc.pending_think_parse = "secret tail".to_string();
        flush_pending_think_parse(&mut acc, &mut ops);

        assert_eq!(acc.content, "");
        assert_eq!(acc.reasoning, "secret tail");
    }

    #[test]
    fn test_route_append_content_with_think_tags_multiple_segments_single_chunk() {
        let mut acc = ChoiceAccumulator::default();
        let mut ops = Vec::new();

        route_append_content_with_think_tags(
            &mut acc,
            &mut ops,
            "a <think>x</think> b <think>y</think> c".to_string(),
            None,
        );

        assert_eq!(acc.content, "a  b  c");
        assert_eq!(acc.reasoning, "xy");
    }

    #[test]
    fn test_route_append_content_with_think_tags_close_without_open_is_content() {
        let mut acc = ChoiceAccumulator::default();
        let mut ops = Vec::new();

        route_append_content_with_think_tags(&mut acc, &mut ops, "a </think> b".to_string(), None);

        assert_eq!(acc.content, "a </think> b");
        assert_eq!(acc.reasoning, "");
    }

    #[test]
    fn test_handle_append_content_delta_indexed_keeps_tags_as_content() {
        let mut acc = ChoiceAccumulator::default();
        let mut ops = Vec::new();

        handle_append_content_delta(
            &mut acc,
            &mut ops,
            "before <think>secret</think> after".to_string(),
            Some(4),
        );

        assert_eq!(acc.content, "before <think>secret</think> after");
        assert_eq!(acc.reasoning, "");
        assert_eq!(
            acc.content_per_block.get(&4).map(|s| s.as_str()),
            Some("before <think>secret</think> after")
        );
    }
}
