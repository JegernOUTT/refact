use std::sync::Arc;
use serde_json::{Map, Value};
use tokenizers::Tokenizer;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::scratchpads::multimodality::MultimodalElement;
use crate::tokens::count_text_tokens_with_fallback;
use crate::postprocessing::pp_command_output::output_mini_postprocessing;

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
                new_text_lines.push("No content: tokens limit reached");
            }
            new_text_lines.push("Truncated: too many tokens\n");
            break;
        }
        tok_used += line_tokens;
        new_text_lines.push(line);
    }
    (new_text_lines.join("\n"), tok_used)
}

const STRUCTURED_TRUNCATION_WARNING: &str =
    "Response reduced to fit the context token budget.";

#[derive(Clone)]
enum JsonPathPart {
    Key(String),
    Index(usize),
}

#[derive(Clone)]
enum ShrinkKind {
    Array,
    String,
}

#[derive(Clone)]
struct ShrinkCandidate {
    path: Vec<JsonPathPart>,
    size: usize,
    kind: ShrinkKind,
}

fn structured_tool_object(text: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(text).ok()?;
    let object = value.as_object()?;
    if object.get("tool").and_then(Value::as_str).is_some()
        && object.get("summary").and_then(Value::as_str).is_some()
    {
        Some(value)
    } else {
        None
    }
}

fn find_largest_shrinkable(
    value: &Value,
    path: &mut Vec<JsonPathPart>,
    candidate: &mut Option<ShrinkCandidate>,
) {
    let proposed = match value {
        Value::Array(items) if !items.is_empty() => Some((
            serde_json::to_string(value).map_or(0, |text| text.len()),
            ShrinkKind::Array,
        )),
        Value::String(text) if !text.is_empty() => {
            Some((text.len(), ShrinkKind::String))
        }
        _ => None,
    };
    if let Some((size, kind)) = proposed {
        // Traversal and strict comparison make ties deterministic.
        if candidate.as_ref().map_or(true, |current| size > current.size) {
            *candidate = Some(ShrinkCandidate {
                path: path.clone(),
                size,
                kind,
            });
        }
    }

    match value {
        Value::Object(object) => {
            for (key, child) in object {
                // These envelope strings are required to survive compaction unchanged.
                if path.is_empty() && matches!(key.as_str(), "tool" | "summary" | "warning") {
                    continue;
                }
                path.push(JsonPathPart::Key(key.clone()));
                find_largest_shrinkable(child, path, candidate);
                path.pop();
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                path.push(JsonPathPart::Index(index));
                find_largest_shrinkable(child, path, candidate);
                path.pop();
            }
        }
        _ => {}
    }
}

fn value_at_path_mut<'a>(mut value: &'a mut Value, path: &[JsonPathPart]) -> Option<&'a mut Value> {
    for part in path {
        value = match part {
            JsonPathPart::Key(key) => value.as_object_mut()?.get_mut(key)?,
            JsonPathPart::Index(index) => value.as_array_mut()?.get_mut(*index)?,
        };
    }
    Some(value)
}

fn shrink_once(value: &mut Value) -> bool {
    let mut candidate = None;
    find_largest_shrinkable(value, &mut Vec::new(), &mut candidate);
    let Some(candidate) = candidate else {
        return false;
    };
    let Some(target) = value_at_path_mut(value, &candidate.path) else {
        return false;
    };
    match candidate.kind {
        ShrinkKind::Array => {
            let Some(items) = target.as_array_mut() else {
                return false;
            };
            let new_len = if items.len() == 1 { 0 } else { items.len() / 2 };
            items.truncate(new_len);
        }
        ShrinkKind::String => {
            let Some(text) = target.as_str() else {
                return false;
            };
            let keep_chars = text.chars().count() / 2;
            *target = Value::String(text.chars().take(keep_chars).collect());
        }
    }
    true
}

fn serialize_within_limit(
    tokenizer: Option<Arc<Tokenizer>>,
    value: &Value,
    limit_tokens: usize,
) -> Option<(String, usize)> {
    let text = serde_json::to_string(value).ok()?;
    let used = count_text_tokens_with_fallback(tokenizer, &text);
    (used <= limit_tokens).then_some((text, used))
}

fn compact_structured_json(
    tokenizer: Option<Arc<Tokenizer>>,
    original_text: &str,
    mut value: Value,
    limit_tokens: usize,
) -> (String, usize) {
    let original_tokens = count_text_tokens_with_fallback(tokenizer.clone(), original_text);
    if original_tokens <= limit_tokens {
        return (original_text.to_string(), original_tokens);
    }

    if let Some(object) = value.as_object_mut() {
        object.insert("truncated".to_string(), Value::Bool(true));
        object.insert(
            "warning".to_string(),
            Value::String(STRUCTURED_TRUNCATION_WARNING.to_string()),
        );
    }
    // Each pass at least halves a non-empty array or string, so this remains bounded
    // by the amount of structured content rather than the token limit.
    loop {
        if let Some(result) = serialize_within_limit(tokenizer.clone(), &value, limit_tokens) {
            return result;
        }
        if !shrink_once(&mut value) {
            break;
        }
    }

    let source = value.as_object().expect("structured tool JSON is an object");
    let mut minimal = Map::new();
    minimal.insert("tool".to_string(), source["tool"].clone());
    minimal.insert("summary".to_string(), source["summary"].clone());
    minimal.insert("truncated".to_string(), Value::Bool(true));
    minimal.insert(
        "warning".to_string(),
        Value::String(STRUCTURED_TRUNCATION_WARNING.to_string()),
    );
    let minimal = Value::Object(minimal);
    if let Some(result) = serialize_within_limit(tokenizer.clone(), &minimal, limit_tokens) {
        return result;
    }

    // There is no representation containing the required envelope below its own
    // token cost. Return it rather than corrupting JSON or its identifying fields.
    let text = serde_json::to_string(&minimal).expect("serializing JSON value cannot fail");
    let used = count_text_tokens_with_fallback(tokenizer, &text);
    (text, used)
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

    for mut msg in plain_text_messages.into_iter() {
        if let Some(ref filter) = msg.output_filter {
            if filter.limit_lines < usize::MAX
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

        let effective_limit = match per_msg_limit {
            Some(msg_limit) => msg_limit.min(remaining_budget),
            None => remaining_budget,
        };

        if let ChatContent::SimpleText(text) = &msg.content {
            if let Some(value) = structured_tool_object(text) {
                let (new_content, tokens_used) = compact_structured_json(
                    tokenizer.clone(),
                    text,
                    value,
                    effective_limit,
                );
                msg.content = ChatContent::SimpleText(new_content);
                remaining_budget = remaining_budget.saturating_sub(tokens_used);
                new_messages.push(msg);
                continue;
            }
        }

        if effective_limit < 50 {
            msg.content =
                ChatContent::SimpleText("... truncated (token limit reached)".to_string());
            new_messages.push(msg);
            continue;
        }

        let tokens_used = match msg.content {
            ChatContent::SimpleText(ref text) => {
                let (new_content, used) =
                    limit_text_by_tokens(tokenizer.clone(), text, effective_limit);
                msg.content = ChatContent::SimpleText(new_content);
                used
            }
            ChatContent::Multimodal(ref elements) => {
                let mut new_content = vec![];
                let mut used_in_msg = 0;

                for element in elements {
                    if element.is_text() {
                        let remaining = effective_limit.saturating_sub(used_in_msg);
                        let (new_text, used) =
                            limit_text_by_tokens(tokenizer.clone(), &element.m_content, remaining);
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

        remaining_budget = remaining_budget.saturating_sub(tokens_used);
        new_messages.push(msg);
    }

    (new_messages, remaining_budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_structured_json_is_compacted_and_parseable() {
        let input = serde_json::json!({
            "tool": "dead_code",
            "summary": "Dead code analysis",
            "total": 200,
            "entries": (0..100).map(|index| serde_json::json!({
                "name": format!("unused_function_{index}"),
                "path": format!("/project/src/very/long/path/{index}.rs")
            })).collect::<Vec<_>>()
        }).to_string();
        let limit = 80;
        let value = structured_tool_object(&input).unwrap();
        let (output, used) = compact_structured_json(None, &input, value, limit);
        let parsed: Value = serde_json::from_str(&output).unwrap();

        assert!(used <= limit);
        assert_eq!(parsed["tool"], "dead_code");
        assert_eq!(parsed["summary"], "Dead code analysis");
        assert_eq!(parsed["total"], 200);
        assert_eq!(parsed["truncated"], true);
        assert!(parsed["warning"].as_str().unwrap().contains("context"));
    }

    #[test]
    fn tiny_budget_uses_minimal_valid_envelope() {
        let input = serde_json::json!({
            "tool": "x",
            "summary": "y",
            "large": (0..100).map(|_| "content").collect::<Vec<_>>()
        }).to_string();
        let limit = 32;
        let value = structured_tool_object(&input).unwrap();
        let (output, used) = compact_structured_json(None, &input, value, limit);
        let parsed: Value = serde_json::from_str(&output).unwrap();

        assert!(used <= limit);
        assert_eq!(parsed["tool"], "x");
        assert_eq!(parsed["summary"], "y");
        assert_eq!(parsed["truncated"], true);
        assert!(parsed.get("warning").is_some());
        assert_eq!(parsed.as_object().unwrap().len(), 4);
    }

    #[test]
    fn ordinary_text_keeps_line_truncation_behavior() {
        let (output, _) = limit_text_by_tokens(None, "short\nthis line is much too long", 3);
        assert_eq!(output, "short\nTruncated: too many tokens\n");
    }

    #[test]
    fn small_structured_json_is_byte_identical() {
        let input = "{ \"tool\": \"dead_code\", \"summary\": \"ok\", \"entries\": [] }";
        let value = structured_tool_object(input).unwrap();
        let (output, _) = compact_structured_json(None, input, value, 100);
        assert_eq!(output, input);
    }
}
