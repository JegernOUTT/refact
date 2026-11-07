use std::sync::Arc;
use tokenizers::Tokenizer;

use refact_core::chat_types::{ChatContent, ChatMessage, MultimodalElement};
use refact_core::chunk_utils::count_text_tokens_with_fallback;
use crate::pp_command_output::output_mini_postprocessing;

const MIN_PLAIN_TEXT_MESSAGE_TOKENS: usize = 50;
const TRUNCATION_MARKER: &str = "Truncated: too many tokens";
/// Emitted when not even the verdict-only JSON tier of a structured report fits. Deliberately
/// not JSON, so it can never be mistaken for a complete or successful report.
pub const STRUCTURED_REPORT_STARVED_MARKER: &str =
    "report omitted: token budget exhausted; outcome unknown";

fn truncate_text_to_token_budget(
    tokenizer: Option<Arc<Tokenizer>>,
    text: &str,
    limit_tokens: usize,
) -> String {
    if count_text_tokens_with_fallback(tokenizer.clone(), text) <= limit_tokens {
        return text.to_string();
    }

    let marker = format!("\n{TRUNCATION_MARKER}");
    if count_text_tokens_with_fallback(tokenizer.clone(), &marker) >= limit_tokens {
        return TRUNCATION_MARKER.to_string();
    }

    let chars: Vec<char> = text.chars().collect();
    let mut low = 0usize;
    let mut high = chars.len();
    let mut best = 0usize;

    while low <= high {
        let mid = low + (high - low) / 2;
        let prefix: String = chars[..mid].iter().collect();
        let candidate = format!("{prefix}{marker}");
        let tokens = count_text_tokens_with_fallback(tokenizer.clone(), &candidate);
        if tokens <= limit_tokens {
            best = mid;
            low = mid.saturating_add(1);
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }

    let prefix: String = chars[..best].iter().collect();
    format!("{prefix}{marker}")
}

/// Top-level keys of a structured tool report that the reducer never drops: they carry the
/// aggregate verdict, completion accounting, recovery state and omission bookkeeping.
const STRUCTURED_REPORT_PROTECTED_KEYS: &[&str] = &[
    "tool",
    "ok",
    "summary",
    "completion",
    "recovery",
    "session_generation",
    "full_report",
    "artifact_error",
    "omitted",
    "error",
];
const STRUCTURED_REPORT_STEP_KEYS: &[&str] = &[
    "step_index",
    "ok",
    "outcome",
    "skipped",
    "action",
    "error",
    "summary",
];
const STRUCTURED_STRING_CAPS: &[usize] = &[4_000, 1_000, 300, 120];
const STRUCTURED_ARRAY_CAPS: &[usize] = &[200, 50, 12, 4];
const STRUCTURED_PROTECTED_STRING_CAP: usize = 240;
const STRUCTURED_MINIMAL_STRING_CAP: usize = 96;

/// A text payload is a structured report when it is a JSON object declaring its producing
/// `tool` and a `completion` object. Such payloads are reduced field-by-field so the result
/// stays parseable JSON; everything else keeps the plain line truncator.
fn parse_structured_report(text: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('{') {
        return None;
    }
    let serde_json::Value::Object(map) = serde_json::from_str(trimmed).ok()? else {
        return None;
    };
    (map.get("tool").is_some_and(serde_json::Value::is_string)
        && (map
            .get("completion")
            .is_some_and(serde_json::Value::is_object)
            || (map
                .get("incomplete")
                .is_some_and(serde_json::Value::is_boolean)
                && map
                    .get("effect_unknown")
                    .is_some_and(serde_json::Value::is_boolean)
                && map.get("omitted").is_some_and(serde_json::Value::is_object))))
    .then_some(map)
}

#[derive(Default)]
struct StructuredOmissions {
    strings_shortened: usize,
    array_items: usize,
    fields: Vec<String>,
    steps: usize,
}

fn cap_string(text: &mut String, cap: usize) -> bool {
    let total = text.chars().count();
    if total <= cap {
        return false;
    }
    let kept: String = text.chars().take(cap).collect();
    *text = format!("{kept}…[+{} chars]", total - cap);
    true
}

fn shrink_value(
    value: &mut serde_json::Value,
    string_cap: usize,
    array_cap: usize,
    omissions: &mut StructuredOmissions,
) {
    match value {
        serde_json::Value::String(text) => {
            if cap_string(text, string_cap) {
                omissions.strings_shortened += 1;
            }
        }
        serde_json::Value::Array(items) => {
            if items.len() > array_cap {
                omissions.array_items += items.len() - array_cap;
                items.truncate(array_cap);
            }
            for item in items.iter_mut() {
                shrink_value(item, string_cap, array_cap, omissions);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                shrink_value(item, string_cap, array_cap, omissions);
            }
        }
        _ => {}
    }
}

fn step_is_protected(step: &serde_json::Value) -> bool {
    step.get("ok").and_then(serde_json::Value::as_bool) != Some(true)
        || step
            .get("outcome")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|outcome| outcome != "ok" && outcome != "success")
}

fn compact_step(step: &serde_json::Value, string_cap: usize) -> serde_json::Value {
    let mut compact = serde_json::Map::new();
    if let Some(object) = step.as_object() {
        for key in STRUCTURED_REPORT_STEP_KEYS {
            if let Some(value) = object.get(*key) {
                let mut value = value.clone();
                if let serde_json::Value::String(text) = &mut value {
                    cap_string(text, string_cap);
                }
                compact.insert((*key).to_string(), value);
            }
        }
    }
    serde_json::Value::Object(compact)
}

fn protected_value(
    report: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    string_cap: usize,
) -> Option<serde_json::Value> {
    let mut value = report.get(key)?.clone();
    if key != "session_generation" {
        let mut sink = StructuredOmissions::default();
        shrink_value(&mut value, string_cap, usize::MAX, &mut sink);
    }
    Some(value)
}

fn render_structured(
    tokenizer: &Option<Arc<Tokenizer>>,
    mut report: serde_json::Map<String, serde_json::Value>,
    omissions: &StructuredOmissions,
    mode: &str,
) -> (String, usize) {
    let mut omitted = report
        .remove("omitted")
        .and_then(|value| match value {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    omitted.insert("reduction".to_string(), serde_json::json!(mode));
    if omissions.strings_shortened > 0 {
        omitted.insert(
            "strings_shortened".to_string(),
            serde_json::json!(omissions.strings_shortened),
        );
    }
    if omissions.array_items > 0 {
        omitted.insert(
            "array_items".to_string(),
            serde_json::json!(omissions.array_items),
        );
    }
    if !omissions.fields.is_empty() {
        omitted.insert("fields".to_string(), serde_json::json!(omissions.fields));
    }
    if omissions.steps > 0 {
        omitted.insert("steps".to_string(), serde_json::json!(omissions.steps));
    }
    report.insert("omitted".to_string(), serde_json::Value::Object(omitted));
    let text = serde_json::to_string(&serde_json::Value::Object(report)).unwrap_or_default();
    let tokens = count_text_tokens_with_fallback(tokenizer.clone(), &text);
    (text, tokens)
}

/// Reduce a structured report to `limit_tokens` while keeping it valid JSON. Reduction order:
/// shorten strings and cap arrays everywhere, then drop unprotected top-level fields, then keep
/// only failed/unknown steps in compact form, then a summary-only envelope. The summary-only
/// envelope degrades to verdict-only tiers; `None` means not even the bare verdict fits, and
/// the caller must emit an explicit non-JSON starvation marker instead of an over-budget report.
fn reduce_structured_report(
    tokenizer: Option<Arc<Tokenizer>>,
    original: serde_json::Map<String, serde_json::Value>,
    limit_tokens: usize,
) -> Option<(String, usize)> {
    let mut omissions = StructuredOmissions::default();
    let mut report = original.clone();
    for (string_cap, array_cap) in STRUCTURED_STRING_CAPS
        .iter()
        .zip(STRUCTURED_ARRAY_CAPS.iter())
    {
        omissions = StructuredOmissions::default();
        report = original.clone();
        for (key, value) in report.iter_mut() {
            let (string_cap, array_cap) =
                if STRUCTURED_REPORT_PROTECTED_KEYS.contains(&key.as_str()) {
                    (STRUCTURED_PROTECTED_STRING_CAP.max(*string_cap), usize::MAX)
                } else if key == "steps" {
                    (*string_cap, usize::MAX)
                } else {
                    (*string_cap, *array_cap)
                };
            if key != "session_generation" {
                shrink_value(value, string_cap, array_cap, &mut omissions);
            }
        }
        let (text, tokens) = render_structured(&tokenizer, report.clone(), &omissions, "shortened");
        if tokens <= limit_tokens {
            return Some((text, tokens));
        }
    }

    let mut droppable: Vec<(String, usize)> = report
        .iter()
        .filter(|(key, _)| {
            !STRUCTURED_REPORT_PROTECTED_KEYS.contains(&key.as_str()) && key.as_str() != "steps"
        })
        .map(|(key, value)| (key.clone(), value.to_string().len()))
        .collect();
    droppable.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    for (key, _) in droppable {
        report.remove(&key);
        omissions.fields.push(key);
        let (text, tokens) =
            render_structured(&tokenizer, report.clone(), &omissions, "fields_dropped");
        if tokens <= limit_tokens {
            return Some((text, tokens));
        }
    }

    if let Some(serde_json::Value::Array(steps)) = report.get("steps").cloned() {
        let kept: Vec<serde_json::Value> = steps
            .iter()
            .filter(|step| step_is_protected(step))
            .map(|step| compact_step(step, STRUCTURED_MINIMAL_STRING_CAP))
            .collect();
        omissions.steps = steps.len() - kept.len();
        report.insert("steps".to_string(), serde_json::Value::Array(kept));
        let (text, tokens) =
            render_structured(&tokenizer, report.clone(), &omissions, "failed_steps_only");
        if tokens <= limit_tokens {
            return Some((text, tokens));
        }
        omissions.steps = steps.len();
    }

    let mut minimal = serde_json::Map::new();
    for key in STRUCTURED_REPORT_PROTECTED_KEYS {
        if let Some(value) = protected_value(&original, key, STRUCTURED_MINIMAL_STRING_CAP) {
            minimal.insert((*key).to_string(), value);
        }
    }
    if let Some(serde_json::Value::Array(steps)) = original.get("steps") {
        let protected_indices: Vec<serde_json::Value> = steps
            .iter()
            .filter(|step| step_is_protected(step))
            .map(|step| {
                serde_json::json!({
                    "step_index": step.get("step_index").cloned().unwrap_or(serde_json::Value::Null),
                    "ok": step.get("ok").cloned().unwrap_or(serde_json::Value::Null),
                    "outcome": step.get("outcome").cloned().unwrap_or(serde_json::Value::Null),
                })
            })
            .collect();
        minimal.insert(
            "failed_or_unknown_steps".to_string(),
            serde_json::Value::Array(protected_indices),
        );
    }
    omissions.fields = original
        .keys()
        .filter(|key| !minimal.contains_key(key.as_str()))
        .cloned()
        .collect();
    let (text, tokens) = render_structured(&tokenizer, minimal.clone(), &omissions, "summary_only");
    if tokens <= limit_tokens {
        return Some((text, tokens));
    }

    // Verdict-only tiers. Every tier carries the aggregate `ok`, `incomplete` and
    // `effect_unknown` booleans; `session_generation` is kept exactly or explicitly marked
    // omitted, never shortened. The first tier that fits wins; nothing over budget is returned.
    let steps = original.get("steps").and_then(serde_json::Value::as_array);
    let steps_total = steps.map(Vec::len).unwrap_or(0);
    let protected_steps = steps
        .map(|steps| steps.iter().filter(|step| step_is_protected(step)).count())
        .unwrap_or(0);
    let flag = |section: &str, key: &str| {
        original
            .get(section)
            .and_then(|value| value.get(key))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    };
    let effect_unknown = original
        .get("effect_unknown")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        || flag("recovery", "effect_unknown")
        || original
            .get("completion")
            .and_then(|c| c.get("unknown"))
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|unknown| unknown > 0)
        || steps.is_some_and(|steps| {
            steps.iter().any(|step| {
                step.get("outcome").and_then(serde_json::Value::as_str) == Some("effect_unknown")
            })
        });
    let incomplete = original
        .get("incomplete")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        || flag("completion", "incomplete")
        || flag("recovery", "incomplete")
        || effect_unknown;
    let tool = original
        .get("tool")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let ok = original
        .get("ok")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let generation = original.get("session_generation").cloned();
    let mut tiers = Vec::new();
    if let Some(generation) = generation.clone() {
        tiers.push(serde_json::json!({
            "tool": tool, "ok": ok, "incomplete": incomplete, "effect_unknown": effect_unknown,
            "session_generation": generation,
            "omitted": {"reduction": "verdict_only", "steps": steps_total,
                        "failed_or_unknown_steps": protected_steps},
        }));
    }
    let mut counts = serde_json::json!({"reduction": "verdict_only", "steps": steps_total,
                                         "failed_or_unknown_steps": protected_steps});
    let mut bare = serde_json::json!({"reduction": "verdict_only"});
    if generation.is_some() {
        counts["session_generation"] = serde_json::json!(true);
        bare["session_generation"] = serde_json::json!(true);
    }
    tiers.push(serde_json::json!({
        "tool": tool, "ok": ok, "incomplete": incomplete, "effect_unknown": effect_unknown,
        "omitted": counts,
    }));
    tiers.push(serde_json::json!({
        "tool": tool, "ok": ok, "incomplete": incomplete, "effect_unknown": effect_unknown,
        "omitted": bare,
    }));
    for tier in tiers {
        let text = serde_json::to_string(&tier).unwrap_or_default();
        let tokens = count_text_tokens_with_fallback(tokenizer.clone(), &text);
        if tokens <= limit_tokens {
            return Some((text, tokens));
        }
    }
    None
}

/// Key set in `ChatMessage::extra` when a structured report could not be delivered even in its
/// verdict-only form. Consumers must treat the tool outcome as unknown, never as success.
pub const STRUCTURED_REPORT_DELIVERY_KEY: &str = "structured_report_delivery";

fn mark_structured_report_undelivered(msg: &mut ChatMessage) {
    msg.tool_failed = Some(true);
    msg.extra.insert(
        STRUCTURED_REPORT_DELIVERY_KEY.to_string(),
        serde_json::json!("budget_exhausted"),
    );
}

/// True when `text` is a structured tool report that must only be reduced as JSON.
pub fn is_structured_report(text: &str) -> bool {
    parse_structured_report(text).is_some()
}

/// A starved message (budget below the plain-text floor) that carries a structured report keeps
/// its summary-only envelope instead of an opaque marker; images are dropped with a count.
fn starved_structured_report(
    tokenizer: &Option<Arc<Tokenizer>>,
    content: &ChatContent,
    limit_tokens: usize,
) -> Option<(ChatContent, usize)> {
    let (text, images) = match content {
        ChatContent::SimpleText(text) => (text.as_str(), 0),
        ChatContent::Multimodal(elements) => (
            elements.iter().find(|el| el.is_text())?.m_content.as_str(),
            elements.iter().filter(|el| el.is_image()).count(),
        ),
        ChatContent::ContextFiles(_) => return None,
    };
    let mut report = parse_structured_report(text)?;
    if images > 0 {
        let omitted = report
            .entry("omitted".to_string())
            .or_insert_with(|| serde_json::json!({}));
        if let Some(omitted) = omitted.as_object_mut() {
            omitted.insert("images".to_string(), serde_json::json!(images));
        }
    }
    let (reduced, used) = reduce_structured_report(tokenizer.clone(), report, limit_tokens)?;
    Some((ChatContent::SimpleText(reduced), used))
}

/// Fit `text` into `limit_tokens`: structured reports are reduced as JSON, other text goes
/// through the line truncator.
fn limit_payload_by_tokens(
    tokenizer: Option<Arc<Tokenizer>>,
    text: &str,
    limit_tokens: usize,
) -> (String, usize) {
    if let Some(report) = parse_structured_report(text) {
        let tokens = count_text_tokens_with_fallback(tokenizer.clone(), text);
        if tokens <= limit_tokens {
            return (text.to_string(), tokens);
        }
        return reduce_structured_report(tokenizer.clone(), report, limit_tokens).unwrap_or_else(
            || {
                let marker = STRUCTURED_REPORT_STARVED_MARKER.to_string();
                let used = count_text_tokens_with_fallback(tokenizer, &marker);
                (marker, used)
            },
        );
    }
    limit_text_by_tokens(tokenizer, text, limit_tokens)
}

fn limit_text_by_tokens(
    tokenizer: Option<Arc<Tokenizer>>,
    text: &str,
    limit_tokens: usize,
) -> (String, usize) {
    let mut new_text_lines = vec![];
    let mut tok_used = 0;
    for line in text.lines() {
        let line_tokens = count_text_tokens_with_fallback(tokenizer.clone(), line);
        if tok_used + line_tokens > limit_tokens {
            if new_text_lines.is_empty() {
                let truncated =
                    truncate_text_to_token_budget(tokenizer.clone(), line, limit_tokens);
                let used = count_text_tokens_with_fallback(tokenizer.clone(), &truncated);
                return (truncated, used.min(limit_tokens));
            }
            new_text_lines.push(TRUNCATION_MARKER);
            break;
        }
        tok_used += line_tokens;
        new_text_lines.push(line);
    }
    let result = new_text_lines.join("\n");
    let used = count_text_tokens_with_fallback(tokenizer, &result);
    (result, used.min(limit_tokens))
}

fn fair_message_token_limit(
    remaining_budget: usize,
    messages_left: usize,
    per_message_limit: Option<usize>,
) -> usize {
    if messages_left <= 1 {
        return per_message_limit
            .unwrap_or(remaining_budget)
            .min(remaining_budget);
    }

    let fair_share = remaining_budget / messages_left;
    per_message_limit
        .unwrap_or(fair_share)
        .min(fair_share)
        .min(remaining_budget)
}

pub async fn postprocess_plain_text(
    plain_text_messages: Vec<ChatMessage>,
    tokenizer: Option<Arc<Tokenizer>>,
    tokens_limit: usize,
    style: &Option<String>,
) -> (Vec<ChatMessage>, usize) {
    if plain_text_messages.is_empty() {
        return (vec![], tokens_limit);
    }

    let mut remaining_budget = tokens_limit;
    let mut new_messages = vec![];
    let total_messages = plain_text_messages.len();

    for (idx, mut msg) in plain_text_messages.into_iter().enumerate() {
        let carries_structured_report = match &msg.content {
            ChatContent::SimpleText(text) => is_structured_report(text),
            ChatContent::Multimodal(elements) => elements
                .iter()
                .any(|el| el.is_text() && is_structured_report(&el.m_content)),
            ChatContent::ContextFiles(_) => false,
        };
        if let Some(ref filter) = msg.output_filter {
            // Line/char/grep filters would cut a structured report mid-JSON; such reports are
            // bounded by the token reducer below instead.
            if carries_structured_report {
                if !filter.skip && !filter.remove_from_output.is_empty() {
                    if let Ok(pattern) = regex::Regex::new(&filter.remove_from_output) {
                        if pattern.is_match(&msg.content.content_text_only()) {
                            msg.content = ChatContent::SimpleText(
                                serde_json::json!({
                                    "tool": "chrome",
                                    "ok": false,
                                    "completion": {"incomplete": true},
                                    "error": "report_withheld_by_output_filter"
                                })
                                .to_string(),
                            );
                            msg.tool_failed = Some(true);
                        }
                    }
                }
            } else if filter.limit_lines < usize::MAX
                || filter.limit_chars < usize::MAX
                || !filter.grep.is_empty()
                || !filter.remove_from_output.is_empty()
            {
                msg.content = match msg.content {
                    ChatContent::SimpleText(text) => {
                        ChatContent::SimpleText(output_mini_postprocessing(filter, &text))
                    }
                    ChatContent::Multimodal(elements) => {
                        let filtered_elements = elements
                            .into_iter()
                            .map(|mut el| {
                                if el.is_text() {
                                    el.m_content =
                                        output_mini_postprocessing(filter, &el.m_content);
                                }
                                el
                            })
                            .collect();
                        ChatContent::Multimodal(filtered_elements)
                    }
                    ChatContent::ContextFiles(files) => ChatContent::ContextFiles(files),
                };
            }
        }

        let per_msg_limit = msg.output_filter.as_ref().and_then(|f| f.limit_tokens);
        msg.output_filter = None;

        let messages_left = total_messages.saturating_sub(idx).max(1);
        let effective_limit =
            fair_message_token_limit(remaining_budget, messages_left, per_msg_limit);

        if effective_limit < MIN_PLAIN_TEXT_MESSAGE_TOKENS {
            if let Some((reduced, used)) =
                starved_structured_report(&tokenizer, &msg.content, effective_limit)
            {
                msg.content = reduced;
                remaining_budget = remaining_budget.saturating_sub(used);
                new_messages.push(msg);
                continue;
            }
            if carries_structured_report {
                msg.content = ChatContent::SimpleText(STRUCTURED_REPORT_STARVED_MARKER.to_string());
                mark_structured_report_undelivered(&mut msg);
            } else {
                msg.content =
                    ChatContent::SimpleText("... truncated (token limit reached)".to_string());
            }
            new_messages.push(msg);
            continue;
        }

        let tokens_used = match msg.content {
            ChatContent::SimpleText(ref text) => {
                let (new_content, used) =
                    limit_payload_by_tokens(tokenizer.clone(), text, effective_limit);
                msg.content = ChatContent::SimpleText(new_content);
                used
            }
            ChatContent::Multimodal(ref elements) => {
                let mut new_content = vec![];
                let mut used_in_msg = 0;

                for element in elements {
                    if element.is_text() {
                        let remaining = effective_limit.saturating_sub(used_in_msg);
                        let (new_text, used) = limit_payload_by_tokens(
                            tokenizer.clone(),
                            &element.m_content,
                            remaining,
                        );
                        used_in_msg += used;
                        new_content.push(MultimodalElement {
                            m_type: element.m_type.clone(),
                            m_content: new_text,
                        });
                    } else if element.is_image() {
                        let tokens = element.count_tokens(None, style).unwrap_or(0) as usize;
                        if used_in_msg + tokens > effective_limit {
                            new_content.push(MultimodalElement {
                                m_type: "text".to_string(),
                                m_content: "Image truncated: too many tokens".to_string(),
                            });
                        } else {
                            new_content.push(element.clone());
                            used_in_msg += tokens;
                        }
                    }
                }
                msg.content = ChatContent::Multimodal(new_content);
                used_in_msg
            }
            ChatContent::ContextFiles(_) => msg.content.size_estimate(tokenizer.clone(), style),
        };

        if carries_structured_report
            && msg
                .content
                .content_text_only()
                .contains(STRUCTURED_REPORT_STARVED_MARKER)
        {
            mark_structured_report_undelivered(&mut msg);
        }
        remaining_budget = remaining_budget.saturating_sub(tokens_used);
        new_messages.push(msg);
    }

    (new_messages, remaining_budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_message(id: &str, content: String) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            tool_call_id: id.to_string(),
            content: ChatContent::SimpleText(content),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn parallel_plain_text_messages_share_budget() {
        let messages: Vec<ChatMessage> = (0..4)
            .map(|idx| {
                tool_message(
                    &format!("call_{idx}"),
                    format!("tool-{idx}: {}", "x".repeat(1000)),
                )
            })
            .collect();

        let (processed, remaining) = postprocess_plain_text(messages, None, 240, &None).await;

        assert_eq!(processed.len(), 4);
        assert!(remaining < MIN_PLAIN_TEXT_MESSAGE_TOKENS);
        for (idx, msg) in processed.iter().enumerate() {
            let text = msg.content.content_text_only();
            assert!(
                text.contains(&format!("tool-{idx}:")),
                "message {idx} should keep its own prefix, got {text:?}"
            );
            assert!(text.contains(TRUNCATION_MARKER));
            assert!(!text.contains("No content: tokens limit reached"));
        }
    }

    #[tokio::test]
    async fn single_huge_line_keeps_prefix_before_truncation_marker() {
        let content = format!("start:{}", "x".repeat(4_000));
        let messages = vec![tool_message("call_1", content)];

        let (processed, remaining) = postprocess_plain_text(messages, None, 100, &None).await;

        assert_eq!(processed.len(), 1);
        let text = processed[0].content.content_text_only();
        assert!(text.starts_with("start:"));
        assert!(text.contains(TRUNCATION_MARKER));
        assert!(!text.contains("No content: tokens limit reached"));
        assert_eq!(remaining, 0);
    }

    fn big_browser_report(steps: usize) -> serde_json::Value {
        let steps: Vec<serde_json::Value> = (0..steps)
            .map(|index| {
                let mut step = serde_json::json!({
                    "step_index": index,
                    "ok": true,
                    "summary": format!("Шаг {index} 完了 ✓ {}", "текст 漢字 ".repeat(200)),
                    "data": {"text": "ünïcödé 🚀 ".repeat(400)},
                });
                if index == 7 {
                    step["ok"] = serde_json::json!(false);
                    step["error"] =
                        serde_json::json!(format!("locator timed out {}", "ошибка ".repeat(300)));
                }
                if index == 9 {
                    step["outcome"] = serde_json::json!("effect_unknown");
                }
                step
            })
            .collect();
        serde_json::json!({
            "tool": "chrome",
            "ok": false,
            "summary": "1 failed, 1 effect unknown of 40 steps",
            "completion": {"steps_requested": 40, "steps_reported": 40, "incomplete": true},
            "recovery": {"incomplete": true, "state_lost": false, "effect_unknown": true, "retry_safe": false},
            "session_generation": "rt-42",
            "steps": steps,
            "page": {"url": "https://example.test", "snapshot": {"yaml": "- button \"Сохранить\" [ref=e1]\n".repeat(2000)}},
            "network": (0..300).map(|i| format!("GET https://example.test/{i} 200")).collect::<Vec<_>>(),
        })
    }

    fn assert_protected(text: &str) -> serde_json::Value {
        let parsed: serde_json::Value = serde_json::from_str(text)
            .unwrap_or_else(|error| panic!("reduced report is not valid JSON ({error}): {text}"));
        assert_eq!(parsed["tool"], "chrome");
        assert_eq!(parsed["ok"], false);
        assert!(parsed["omitted"]["reduction"].is_string(), "{text}");
        if parsed["omitted"]["reduction"] == "verdict_only" {
            // Every verdict tier keeps the aggregate unknown/incomplete flags.
            assert_eq!(parsed["incomplete"], true, "{text}");
            assert_eq!(parsed["effect_unknown"], true, "{text}");
            if parsed["omitted"].get("failed_or_unknown_steps").is_some() {
                assert_eq!(parsed["omitted"]["failed_or_unknown_steps"], 2, "{text}");
            }
            match parsed.get("session_generation") {
                Some(generation) => assert_eq!(generation, "rt-42"),
                None => assert_eq!(parsed["omitted"]["session_generation"], true, "{text}"),
            }
            return parsed;
        }
        assert_eq!(parsed["session_generation"], "rt-42");
        assert_eq!(parsed["recovery"]["effect_unknown"], true);
        assert_eq!(parsed["completion"]["incomplete"], true);
        let steps = parsed
            .get("steps")
            .or_else(|| parsed.get("failed_or_unknown_steps"))
            .and_then(serde_json::Value::as_array)
            .expect("failed/unknown steps survive");
        let indices: Vec<u64> = steps
            .iter()
            .filter(|step| step["ok"] == false || step["outcome"] == "effect_unknown")
            .filter_map(|step| step["step_index"].as_u64())
            .collect();
        assert!(indices.contains(&7), "failed step dropped: {text}");
        assert!(indices.contains(&9), "unknown step dropped: {text}");
        parsed
    }

    #[tokio::test]
    async fn oversized_structured_report_stays_valid_json_within_every_budget() {
        let report = big_browser_report(40).to_string();
        assert!(report.len() > 32_768);
        let mut reductions = std::collections::BTreeSet::new();
        for budget in [50usize, 60, 120, 400, 1_500, 6_000, 20_000] {
            let messages = vec![tool_message("call_1", report.clone())];
            let (processed, _) = postprocess_plain_text(messages, None, budget, &None).await;
            let text = processed[0].content.content_text_only();
            assert!(!text.contains(TRUNCATION_MARKER), "line-cut at {budget}");
            let parsed = assert_protected(&text);
            reductions.insert(parsed["omitted"]["reduction"].as_str().unwrap().to_string());
            let tokens = count_text_tokens_with_fallback(None, &text);
            assert!(tokens <= budget, "{tokens} > {budget}: {text}");
        }
        assert!(reductions.contains("verdict_only"), "{reductions:?}");
        assert!(reductions.len() >= 3, "{reductions:?}");
    }

    #[tokio::test]
    async fn omitted_steps_are_counted_honestly() {
        let report = big_browser_report(40).to_string();
        let messages = vec![tool_message("call_1", report)];
        let (processed, _) = postprocess_plain_text(messages, None, 400, &None).await;
        let parsed = assert_protected(&processed[0].content.content_text_only());
        let shown = parsed
            .get("steps")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let omitted = parsed["omitted"]["steps"].as_u64().unwrap_or(0) as usize;
        assert_eq!(shown + omitted, 40, "{parsed}");
    }

    #[tokio::test]
    async fn starved_structured_report_uses_its_share_not_an_opaque_marker() {
        let report = big_browser_report(40).to_string();
        let messages = vec![
            tool_message("call_1", "x".repeat(4000)),
            tool_message("call_2", report),
        ];
        let (processed, _) = postprocess_plain_text(messages, None, 70, &None).await;
        let text = processed[1].content.content_text_only();
        let parsed = assert_protected(&text);
        assert_eq!(parsed["omitted"]["reduction"], "verdict_only");
        let total: usize = processed
            .iter()
            .map(|msg| count_text_tokens_with_fallback(None, &msg.content.content_text_only()))
            .sum();
        assert!(total <= 70, "aggregate budget exceeded: {total}");
    }

    #[tokio::test]
    async fn zero_budget_structured_report_gets_explicit_non_json_marker() {
        let report = big_browser_report(40).to_string();
        let messages = vec![tool_message("call_1", report)];
        let (processed, _) = postprocess_plain_text(messages, None, 0, &None).await;
        let text = processed[0].content.content_text_only();
        assert_eq!(text, STRUCTURED_REPORT_STARVED_MARKER);
        assert!(serde_json::from_str::<serde_json::Value>(&text).is_err());
        assert_eq!(processed[0].tool_failed, Some(true));
        assert_eq!(
            processed[0].extra[STRUCTURED_REPORT_DELIVERY_KEY],
            "budget_exhausted"
        );
    }

    #[tokio::test]
    async fn output_filter_never_line_cuts_a_structured_report() {
        let report = big_browser_report(40).to_string();
        let pretty = serde_json::to_string_pretty(
            &serde_json::from_str::<serde_json::Value>(&report).unwrap(),
        )
        .unwrap();
        let mut msg = tool_message("call_1", pretty);
        msg.output_filter = Some(refact_core::chat_types::OutputFilter {
            limit_lines: 20,
            limit_chars: 2_000,
            ..Default::default()
        });
        let (processed, _) = postprocess_plain_text(vec![msg], None, 3_000, &None).await;
        let text = processed[0].content.content_text_only();
        let parsed = assert_protected(&text);
        assert!(count_text_tokens_with_fallback(None, &text) <= 3_000);
        assert_ne!(parsed["omitted"]["reduction"], "verdict_only");

        let mut msg = tool_message("call_2", report);
        msg.output_filter = Some(refact_core::chat_types::OutputFilter {
            remove_from_output: "rt-42".to_string(),
            ..Default::default()
        });
        let (processed, _) = postprocess_plain_text(vec![msg], None, 3_000, &None).await;
        let text = processed[0].content.content_text_only();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["error"], "report_withheld_by_output_filter");
        assert!(!text.contains("rt-42"));
        assert_eq!(processed[0].tool_failed, Some(true));
    }

    #[tokio::test]
    async fn multimodal_report_keeps_parseable_text_and_counts_images() {
        const TINY_PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let report = big_browser_report(40).to_string();
        let mut msg = tool_message("call_1", String::new());
        msg.content = ChatContent::Multimodal(vec![
            MultimodalElement {
                m_type: "text".into(),
                m_content: report,
            },
            MultimodalElement {
                m_type: "image/png".into(),
                m_content: TINY_PNG.into(),
            },
            MultimodalElement {
                m_type: "image/png".into(),
                m_content: TINY_PNG.into(),
            },
        ]);
        let (processed, _) = postprocess_plain_text(vec![msg], None, 2_000, &None).await;
        let ChatContent::Multimodal(elements) = &processed[0].content else {
            panic!("multimodal content expected");
        };
        assert_protected(&elements[0].m_content);
        assert_eq!(elements.len(), 3);
    }

    #[tokio::test]
    async fn non_report_json_keeps_line_truncation() {
        let text = serde_json::json!({"ok": true, "data": "y".repeat(4000)}).to_string();
        let messages = vec![tool_message("call_1", text)];
        let (processed, _) = postprocess_plain_text(messages, None, 100, &None).await;
        assert!(processed[0]
            .content
            .content_text_only()
            .contains(TRUNCATION_MARKER));
    }
}
