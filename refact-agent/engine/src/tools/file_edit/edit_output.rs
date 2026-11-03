use crate::call_validation::{ChatMessage, ContextEnum, DiffChunk};
use refact_postprocessing::line_truncation::truncate_tool_line;
use serde_json::json;

pub fn bound_model_content(text: &str) -> String {
    let limit = refact_chat_history::config::limits().max_file_size;
    let marker = "\n[edit excerpt truncated; remaining content omitted; raise max_file_size]";
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit.saturating_sub(marker.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &text[..end], marker)
}

pub fn attach_edit_result(message: &mut ChatMessage, content: &str) {
    message.extra.insert(
        "edit_result".into(),
        json!({"version": 1, "model_content": bound_model_content(content)}),
    );
}

pub fn finish_edit_output(out: &mut Vec<ContextEnum>, mut content: String) {
    let mut notes = Vec::new();
    for item in out.iter() {
        if let ContextEnum::ChatMessage(message) = item {
            if message.role == "tool" {
                content.push('\n');
                content.push_str(&message.content.content_text_only());
                notes.push(message.content.content_text_only());
            }
        }
    }
    let primary = out
        .iter()
        .position(
            |item| matches!(item, ContextEnum::ChatMessage(message) if message.role == "diff"),
        )
        .or_else(|| {
            out.iter().rposition(
                |item| matches!(item, ContextEnum::ChatMessage(message) if message.role == "tool"),
            )
        });
    if let Some(primary) = primary {
        let ContextEnum::ChatMessage(message) = &mut out[primary] else {
            return;
        };
        attach_edit_result(message, &content);
        if !notes.is_empty() {
            message.extra.insert("edit_notes".into(), json!(notes));
        }
        for (index, item) in out.iter_mut().enumerate() {
            if index != primary {
                if let ContextEnum::ChatMessage(message) = item {
                    if message.role == "tool" {
                        message.extra.insert("_ui_only".into(), json!(true));
                    }
                }
            }
        }
    }
}

pub fn edit_model_content(
    path: &str,
    action: &str,
    before: Option<&str>,
    after: Option<&str>,
    chunks: &[DiffChunk],
) -> String {
    let count = |snapshot: Option<&str>| {
        snapshot
            .map(|s| s.lines().count().to_string())
            .unwrap_or_else(|| "absent/unavailable".into())
    };
    let mut output = format!(
        "{action}: {path}\nLines: old {}, final {}\n",
        count(before),
        count(after)
    );
    let deleted = after.is_none() && (action.starts_with("delete") || action.starts_with("moved to "));
    if after.is_none() && !deleted {
        output.push_str("Final text snapshot unavailable; destination exists but may be a directory or binary file.\n");
        return bound_model_content(&output);
    }
    let Some(snapshot) = after.or(before) else {
        output.push_str(
            "Text snapshot unavailable (directory, binary, or unreadable file); no excerpt.\n",
        );
        return bound_model_content(&output);
    };
    if snapshot.contains('\0') {
        output.push_str("Binary snapshot; text excerpt omitted.\n");
        return bound_model_content(&output);
    }
    let lines: Vec<_> = snapshot.lines().collect();
    if lines.is_empty() {
        output.push_str(if deleted {
            "Old/deleted file was empty.\n"
        } else {
            "Final file is empty.\n"
        });
        return bound_model_content(&output);
    }
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut offset: isize = 0;
    let mut sorted: Vec<_> = chunks.iter().collect();
    sorted.sort_by_key(|chunk| chunk.line1);
    for chunk in sorted {
        let removed = chunk.lines_remove.lines().count();
        let added = chunk.lines_add.lines().count();
        let start = if deleted {
            chunk.line1.saturating_sub(1)
        } else {
            chunk.line1.saturating_sub(1).saturating_add_signed(offset)
        }
        .min(lines.len());
        let length = if deleted { removed } else { added };
        let end = start.saturating_add(length).min(lines.len());
        if length == 0 {
            output.push_str(&format!("Removed at final boundary after line {start}: {removed} removed, {added} added; no surviving changed line\n"));
        } else {
            output.push_str(&format!(
                "Changed {} lines {}..{}: {} removed, {} added\n",
                if deleted { "old/deleted" } else { "final" },
                start + 1,
                end,
                removed,
                added
            ));
        }
        let range = (
            start.saturating_sub(10),
            end.saturating_add(10).min(lines.len()),
        );
        if let Some(last) = ranges.last_mut().filter(|last| range.0 <= last.1) {
            last.1 = last.1.max(range.1);
        } else {
            ranges.push(range);
        }
        offset += added as isize - removed as isize;
    }
    if ranges.is_empty() {
        if action.starts_with("rename") || before.is_none() || deleted {
            ranges.push((0, lines.len()));
        } else {
            output.push_str("No changed text lines.\n");
        }
    }
    let limit = refact_chat_history::config::limits().max_file_size;
    for (start, end) in ranges {
        output.push_str(&format!(
            "{} lines {}..{}\n",
            if deleted { "OLD/DELETED" } else { "FINAL" },
            start + 1,
            end
        ));
        for (index, line) in lines.iter().enumerate().take(end).skip(start) {
            output.push_str(&truncate_tool_line(&format!("{} | {}", index + 1, line)));
            output.push('\n');
            if output.len() > limit {
                return bound_model_content(&output);
            }
        }
    }
    bound_model_content(&output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(line: usize, removed: &str, added: &str) -> DiffChunk {
        DiffChunk {
            line1: line,
            lines_remove: removed.into(),
            lines_add: added.into(),
            ..Default::default()
        }
    }

    #[test]
    fn final_positions_account_for_prior_insertions_and_merge_context() {
        let before = (1..=45)
            .map(|n| format!("original{n}\n"))
            .collect::<String>();
        let after = format!(
            "inserted\n{}",
            before.replace("original20\n", "replacement\n")
        );
        let output = edit_model_content(
            "file",
            "edit",
            Some(&before),
            Some(&after),
            &[
                chunk(1, "", "inserted\n"),
                chunk(20, "original20\n", "replacement\n"),
            ],
        );
        assert!(output.contains("Changed final lines 21..21: 1 removed, 1 added"));
        assert!(output.contains("FINAL lines 1..31\n"));
        assert!(output.contains("21 | replacement\n"));
        assert!(!output.contains("32 |"));
        assert_eq!(output.matches("11 | original10\n").count(), 1);
    }

    #[test]
    fn deleted_snapshot_is_labelled_old_and_rename_uses_destination() {
        let deleted = edit_model_content("gone", "delete", Some("old\n"), None, &[]);
        assert!(deleted.contains("OLD/DELETED lines 1..1\n1 | old\n"));
        let renamed = edit_model_content("destination", "rename", None, Some("final\n"), &[]);
        assert!(renamed.starts_with("rename: destination\n"));
        assert!(renamed.contains("FINAL lines 1..1\n1 | final\n"));
    }

    #[test]
    fn excerpt_respects_unicode_line_and_total_limits() {
        let line_limit = refact_postprocessing::config::limits().tool_max_line_chars;
        let total_limit = refact_chat_history::config::limits().max_file_size;
        let snapshot = format!("{}\n", "🦀".repeat(line_limit + 100));
        let output = edit_model_content("unicode", "add", None, Some(&snapshot), &[]);
        assert!(output.contains("line truncated:"));
        assert!(output
            .lines()
            .all(|line| line.chars().count() <= line_limit));
        let bounded = bound_model_content(&"🦀".repeat(total_limit));
        assert!(bounded.len() <= total_limit);
        assert!(bounded
            .ends_with("[edit excerpt truncated; remaining content omitted; raise max_file_size]"));
    }

    #[test]
    fn attachment_preserves_diff_payload_and_existing_extra() {
        let mut message = ChatMessage {
            role: "diff".into(),
            content: crate::call_validation::ChatContent::SimpleText(
                "[{\"file_action\":\"edit\"}]".into(),
            ),
            tool_call_id: "call".into(),
            ..Default::default()
        };
        message.extra.insert("diff_state".into(), json!("applied"));
        let original = message.clone();
        attach_edit_result(&mut message, "FINAL\n1 | final");
        assert_eq!(
            message.content.content_text_only(),
            original.content.content_text_only()
        );
        assert_eq!(message.role, original.role);
        assert_eq!(message.tool_call_id, original.tool_call_id);
        assert_eq!(message.extra["diff_state"], json!("applied"));
        assert_eq!(
            message.extra["edit_result"],
            json!({"version":1,"model_content":"FINAL\n1 | final"})
        );
        let projected = refact_core::edit_results::project_edit_result(&message);
        assert_eq!(projected.content.content_text_only(), "FINAL\n1 | final");
        assert_eq!(projected.tool_call_id, "call");
        assert_eq!(projected.extra["diff_state"], json!("applied"));
        assert_eq!(message.content, original.content);
    }

    #[test]
    fn finished_edit_survives_repair_and_branch_without_exposing_diff() {
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};
        let mut assistant = ChatMessage::new("assistant".into(), String::new());
        assistant.tool_calls = Some(vec![ChatToolCall {
            id: "call-edit".into(),
            function: ChatToolFunction {
                name: "edit".into(),
                arguments: "{}".into(),
            },
            tool_type: "function".into(),
            index: None,
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        }]);
        let raw = r#"[{"file_name":"a.rs","lines_remove":"REMOVED_PRIVATE_SENTINEL","lines_add":"new value"}]"#;
        let mut diff = ChatMessage::new("diff".into(), raw.into());
        diff.tool_call_id = "call-edit".into();
        diff.extra.insert(
            "privacy".into(),
            json!({"files":[{"path":"a.rs","zone":"secrets","attribution":"declared"}]}),
        );
        let mut note = ChatMessage::new(
            "tool".into(),
            "Scope warning: verify surrounding code".into(),
        );
        note.tool_call_id = "call-edit".into();
        let mut output = vec![
            ContextEnum::ChatMessage(diff),
            ContextEnum::ChatMessage(note),
        ];
        let excerpt = edit_model_content(
            "a.rs",
            "edit",
            Some("REMOVED_PRIVATE_SENTINEL\n"),
            Some("new value\n"),
            &[chunk(1, "REMOVED_PRIVATE_SENTINEL\n", "new value\n")],
        );
        finish_edit_output(&mut output, excerpt);
        let mut stored = vec![assistant];
        stored.extend(output.into_iter().map(|item| match item {
            ContextEnum::ChatMessage(message) => message,
            _ => panic!("expected message"),
        }));
        refact_chat_history::history_limit::remove_invalid_tool_calls_and_tool_calls_results(
            &mut stored,
        );
        assert_eq!(stored.len(), 3);
        assert_eq!(stored[1].content.content_text_only(), raw);
        assert_eq!(stored[2].extra["_ui_only"], json!(true));
        assert!(stored[2]
            .content
            .content_text_only()
            .contains("Scope warning"));
        let privacy = stored[1].extra["privacy"].clone();
        refact_chat_history::trajectory_ops::demote_goal_ownership_for_branch(&mut stored);
        refact_chat_history::trajectory_ops::sanitize_messages_for_model_switch(&mut stored);
        let wire = crate::chat::linearize::apply_summarization_linearize(stored.clone()).unwrap();
        let results: Vec<_> = wire
            .iter()
            .filter(|m| m.role == "diff" || m.role == "tool")
            .collect();
        assert_eq!(results.len(), 1);
        let projected = refact_core::edit_results::project_edit_result(results[0]);
        let ChatContent::SimpleText(text) = &projected.content else {
            panic!("expected text")
        };
        assert!(text.contains("1 | new value"));
        assert!(text.contains("Scope warning"));
        assert!(!serde_json::to_string(&wire)
            .unwrap()
            .contains("REMOVED_PRIVATE_SENTINEL"));
        assert_eq!(projected.extra["privacy"], privacy);
        assert_eq!(stored[1].content.content_text_only(), raw);
    }

    #[test]
    fn tool_primary_empty_move_keeps_final_excerpt_after_model_switch() {
        let mut message = ChatMessage::new("tool".into(), "Moved empty file".into());
        message.tool_call_id = "move".into();
        let mut output = vec![ContextEnum::ChatMessage(message)];
        let content = edit_model_content(
            "destination",
            "rename from source",
            Some("old\n"),
            Some(""),
            &[],
        );
        finish_edit_output(&mut output, content);
        let ContextEnum::ChatMessage(message) = output.remove(0) else {
            panic!("expected message")
        };
        let mut history = vec![message];
        refact_chat_history::trajectory_ops::sanitize_messages_for_model_switch(&mut history);
        let projected = refact_core::edit_results::project_edit_result(&history[0]);
        let text = projected.content.content_text_only();
        assert!(text.contains("Lines: old 1, final 0"), "{text}");
        assert!(text.contains("Final file is empty."), "{text}");
        assert!(!text.contains("OLD/DELETED"), "{text}");
    }
}
