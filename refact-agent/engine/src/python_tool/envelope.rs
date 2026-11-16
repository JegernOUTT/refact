use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};
use crate::call_validation::{ChatContent, ChatMessage, MultimodalElement};

pub struct BuiltEnvelope {
    pub envelope: Value,
    pub privacy: Vec<Value>,
    pub images: Vec<MultimodalElement>,
    pub summary: String,
    pub diffs: Vec<Value>,
}

pub fn preview(text: &str) -> String {
    text.chars().take(200).collect()
}

pub fn build_envelope(
    messages: &[ChatMessage],
    max_bytes: usize,
    image_index: &AtomicUsize,
) -> BuiltEnvelope {
    let mut text = Vec::new();
    let mut files = Vec::new();
    let mut diffs = Vec::new();
    let mut images = Vec::new();
    let mut refs = Vec::new();
    let mut images_dropped = 0usize;
    let mut privacy = Vec::new();
    let mut data = Value::Null;
    let mut ok = true;
    let mut have_data = false;
    for message in messages {
        for key in ["privacy", "privacy_shell", "privacy_observation"] {
            if let Some(record) = message.extra.get(key) {
                privacy.push(if key == "privacy" {
                    record.clone()
                } else {
                    json!({key: record})
                });
            }
        }
        if !have_data {
            if let Some(output) = message.extra.get("tool_output") {
                data = output.clone();
                have_data = true;
            }
        }
        if message.role == "tool" {
            ok &= message.tool_failed != Some(true);
            let content = message.content.content_text_only();
            if !content.is_empty() {
                text.push(content);
            }
        }
        if message.role == "diff" {
            if let Ok(value) = serde_json::from_str::<Value>(&message.content.content_text_only()) {
                match value {
                    Value::Array(chunks) => diffs.extend(chunks),
                    value => diffs.push(value),
                }
            }
        }
        match &message.content {
            ChatContent::ContextFiles(chunks) => files.extend(chunks.iter().map(|f| json!({"path":f.file_name,"line1":f.line1,"line2":f.line2,"content":f.file_content}))),
            ChatContent::Multimodal(parts) => for part in parts.iter().filter(|p| p.is_image()) {
                let encoded = part.m_content.split_once(",").map_or(part.m_content.as_str(), |(_, data)| data);
                if images.len() >= 8 || encoded.len().saturating_mul(3) / 4 > 5 * 1024 * 1024 {
                    images_dropped += 1;
                    continue;
                }
                match image_index.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| (n < 16).then_some(n + 1)) {
                    Ok(index) => {
                        refs.push(json!({"image":part.m_type,"index":index}));
                        images.push(part.clone());
                    }
                    Err(_) => images_dropped += 1,
                }
            },
            _ => {}
        }
    }
    let cut_upstream = files
        .iter()
        .any(|f| f["content"].as_str().is_some_and(has_cut_marker))
        || text.iter().any(|t| has_skip_note(t))
        || (!files.is_empty() && text.iter().any(|t| has_cut_marker(t)));
    let text = text.join("\n\n");
    let summary = preview(text.lines().next().unwrap_or_default());
    let mut envelope = json!({"ok":ok,"data":data,"text":text,"files":files,"diffs":diffs,"images":refs,"images_dropped":images_dropped,"error":if ok {Value::Null} else {json!(text)},"truncated":cut_upstream});
    let size = encoded_size(&envelope);
    if size > max_bytes {
        let text = format!(
            "[result cut: it is {size} bytes, over python_max_nested_result_bytes ({max_bytes} bytes); narrow the call to get it whole]\n{text}"
        );
        envelope["truncated"] = json!(true);
        fit_text(&mut envelope, &text, max_bytes, max_bytes / 4);
        for key in ["data", "files", "diffs", "images"] {
            if encoded_size(&envelope) <= max_bytes {
                break;
            }
            if key == "images" {
                images_dropped += images.len();
                images.clear();
                envelope["images_dropped"] = json!(images_dropped);
            }
            envelope[key] = if key == "data" {
                Value::Null
            } else {
                json!([])
            };
        }
        fit_text(&mut envelope, &text, max_bytes, 0);
    }
    let retained_diffs = envelope["diffs"].as_array().cloned().unwrap_or_default();
    let summary = if retained_diffs.len() != diffs.len() {
        format!("{summary} [diffs dropped]")
    } else {
        summary
    };
    BuiltEnvelope {
        envelope,
        privacy,
        images,
        summary,
        diffs: retained_diffs,
    }
}

/// Markers that tool-result post-processing leaves where it cut context files.
const CUT_MARKERS: [&str; 3] = [
    "; raise pp_max_",
    "; raise tool_max_line_chars] ...",
    " files skipped",
];
/// Notes it appends to the tool text when it left a file out as already in the chat.
const SKIP_NOTES: [&str; 2] = [
    "`). Skipping to save tokens.",
    "`: already retrieved in message #",
];

fn has_cut_marker(text: &str) -> bool {
    CUT_MARKERS.iter().any(|marker| text.contains(marker))
}

fn has_skip_note(text: &str) -> bool {
    text.contains("📎 ") && SKIP_NOTES.iter().any(|note| text.contains(note))
}

fn encoded_size(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |v| v.len())
}

/// Sets the longest prefix of `text` that fits (at least `floor` encoded bytes) as `text`,
/// and as `error` of a failed call, which keeps a non-empty `error` even when no text fits.
fn fit_text(envelope: &mut Value, text: &str, max_bytes: usize, floor: usize) {
    let failed = envelope["ok"] == false;
    envelope["text"] = json!("");
    if failed {
        envelope["error"] = json!(DROPPED_ERROR);
    }
    let rest = encoded_size(envelope);
    let room = if failed {
        (max_bytes + DROPPED_ERROR.len() + 4).saturating_sub(rest) / 2
    } else {
        (max_bytes + 2).saturating_sub(rest)
    };
    let kept = prefix_within(text, room.max(floor));
    envelope["text"] = json!(kept);
    if failed && !kept.is_empty() {
        envelope["error"] = json!(kept);
    }
}

const DROPPED_ERROR: &str =
    "the tool failed; its output was over the result size limit and was dropped";

/// The longest prefix of `s` whose JSON string encoding takes at most `max` bytes.
fn prefix_within(s: &str, max: usize) -> &str {
    let mut used = 2;
    for (i, c) in s.char_indices() {
        let cost = match c {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            c if (c as u32) < 0x20 => 6,
            c => c.len_utf8(),
        };
        if used + cost > max {
            return &s[..i];
        }
        used += cost;
    }
    s
}

#[cfg(test)]
#[path = "envelope_tests.rs"]
mod tests;

#[cfg(test)]
mod cap_tests {
    use super::*;
    fn fits(envelope: &Value, max: usize) -> bool {
        serde_json::to_vec(envelope).unwrap().len() <= max
    }

    #[test]
    fn files_cut_by_postprocessing_mark_the_envelope_truncated() {
        let file = |content: &str| ChatMessage {
            role: "context_file".into(),
            content: ChatContent::ContextFiles(vec![crate::call_validation::ContextFile {
                file_name: "a.rs".into(),
                line1: 1,
                line2: 2,
                file_content: content.into(),
                ..Default::default()
            }]),
            ..Default::default()
        };
        let tool = |text: &str| ChatMessage {
            role: "tool".into(),
            content: ChatContent::SimpleText(text.into()),
            ..Default::default()
        };
        let cut = "fn a() {}\n\n... (truncated: showing 1 of 9 lines, 10 of 90 tokens to fit the token budget; raise pp_max_per_file_budget_tokens in settings)";
        let skipped = "done\n\n📎 `b.rs` already in context (message #3, via `cat`). Skipping to save tokens.";
        for (messages, expected) in [
            (vec![tool("done"), file("fn a() {}\n")], false),
            (vec![tool("done"), file(cut)], true),
            (vec![tool(skipped)], true),
            (vec![tool("1 files skipped")], false),
        ] {
            let built = build_envelope(&messages, 1 << 20, &AtomicUsize::new(0));
            assert_eq!(built.envelope["truncated"], expected, "{messages:?}");
        }
    }

    #[test]
    fn large_list_data_within_the_cap_is_kept_whole() {
        let mut message = ChatMessage {
            role: "tool".into(),
            content: ChatContent::SimpleText("t".repeat(1024 * 1024)),
            ..Default::default()
        };
        let items: Vec<Value> = (0..50_000).map(|i| json!({"i": i})).collect();
        message
            .extra
            .insert("tool_output".into(), json!({"items": items}));
        let built = build_envelope(&[message], 16 * 1024 * 1024, &AtomicUsize::new(0));
        assert_eq!(built.envelope["truncated"], false);
        assert_eq!(
            built.envelope["data"]["items"].as_array().unwrap().len(),
            50_000
        );
        assert_eq!(built.envelope["text"].as_str().unwrap().len(), 1024 * 1024);
    }

    #[test]
    fn over_cap_envelope_cuts_text_then_drops_data_and_lists_and_keeps_ok_and_error() {
        let max = 64 * 1024;
        let context = |n: usize| ChatMessage {
            role: "context_file".into(),
            content: ChatContent::ContextFiles(
                (0..n)
                    .map(|i| crate::call_validation::ContextFile {
                        file_name: format!("f{i}.rs"),
                        line1: 1,
                        line2: 2,
                        file_content: "c".repeat(100),
                        ..Default::default()
                    })
                    .collect(),
            ),
            ..Default::default()
        };
        for failed in [false, true] {
            let mut message = ChatMessage {
                role: "tool".into(),
                content: ChatContent::SimpleText(format!("first line\n{}", "t".repeat(200_000))),
                tool_failed: Some(failed),
                ..Default::default()
            };
            message.extra.insert(
                "tool_output".into(),
                json!({"items": (0..20_000).collect::<Vec<_>>()}),
            );
            let built = build_envelope(&[message, context(2000)], max, &AtomicUsize::new(0));
            let e = &built.envelope;
            assert!(
                fits(e, max),
                "{} > {max}",
                serde_json::to_vec(e).unwrap().len()
            );
            assert_eq!(e["truncated"], true);
            assert_eq!(e["ok"], !failed);
            assert_eq!(e["data"], Value::Null);
            assert_eq!(e["files"], json!([]));
            let text = e["text"].as_str().unwrap();
            let (note, rest) = text.split_once('\n').unwrap();
            assert!(
                note.contains("python_max_nested_result_bytes")
                    && note.contains(&format!("({max} bytes)"))
                    && note.contains("narrow the call"),
                "{note}"
            );
            assert!(rest.starts_with("first line"), "{text:.200}");
            if failed {
                assert!(
                    e["error"].as_str().is_some_and(|s| !s.is_empty()),
                    "{}",
                    e["error"]
                );
            } else {
                assert_eq!(e["error"], Value::Null);
            }
            for key in [
                "ok",
                "data",
                "text",
                "files",
                "diffs",
                "images",
                "images_dropped",
                "error",
                "truncated",
            ] {
                assert!(e.get(key).is_some(), "{key}");
            }
        }
    }

    #[test]
    fn over_cap_data_alone_is_dropped_and_text_stays_whole() {
        let mut message = ChatMessage {
            role: "tool".into(),
            content: ChatContent::SimpleText("short text".into()),
            ..Default::default()
        };
        message.extra.insert(
            "tool_output".into(),
            json!({"items": (0..50_000).collect::<Vec<_>>()}),
        );
        let built = build_envelope(&[message], 64 * 1024, &AtomicUsize::new(0));
        let e = &built.envelope;
        assert!(fits(e, 64 * 1024));
        assert_eq!(
            (e["truncated"].clone(), e["data"].clone()),
            (json!(true), Value::Null)
        );
        let text = e["text"].as_str().unwrap();
        assert!(
            text.starts_with("[result cut: it is ") && text.ends_with("]\nshort text"),
            "{text}"
        );
        assert_eq!(e["ok"], true);
    }

    #[test]
    fn every_over_cap_envelope_fits_even_with_a_huge_error() {
        let max = 65_536;
        let message = ChatMessage {
            role: "tool".into(),
            content: ChatContent::SimpleText("e".repeat(1_000_000)),
            tool_failed: Some(true),
            ..Default::default()
        };
        let built = build_envelope(&[message], max, &AtomicUsize::new(0));
        assert!(fits(&built.envelope, max));
        assert_eq!(built.envelope["ok"], false);
        assert!(built.envelope["error"]
            .as_str()
            .is_some_and(|s| s.contains("python_max_nested_result_bytes")));
    }
}
