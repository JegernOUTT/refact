use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const TITLE_MAX_CHARS: usize = 120;
const BODY_MAX_CHARS: usize = 4000;
const TRUNCATED_SUFFIX: &str = "...[truncated]";

pub struct ToolBuddyOpenIssue {
    pub config_path: String,
}

impl ToolBuddyOpenIssue {
    fn runner(&self) -> crate::tools::tool_buddy_create_issue::ToolBuddyCreateIssue {
        crate::tools::tool_buddy_create_issue::ToolBuddyCreateIssue {
            config_path: self.config_path.clone(),
        }
    }
}

fn truncate_with_suffix(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let suffix_chars = TRUNCATED_SUFFIX.chars().count();
    let keep_chars = max_chars.saturating_sub(suffix_chars);
    let mut truncated = value.chars().take(keep_chars).collect::<String>();
    truncated.push_str(TRUNCATED_SUFFIX);
    truncated
}

fn capped_redacted(value: &str, max_chars: usize) -> String {
    truncate_with_suffix(&crate::buddy::actor::redact_sensitive(value), max_chars)
}

fn valid_provider(provider: &Value) -> bool {
    matches!(provider.as_str(), Some("github") | Some("gitlab"))
}

fn prepare_forwarded_args(args: &HashMap<String, Value>) -> Result<HashMap<String, Value>, String> {
    let known = [
        "title",
        "body",
        "labels",
        "provider",
        "confidence",
        "error",
        "source_file",
        "tool_name",
        "diagnostic_index",
        "diagnostic_id",
        "collected_at",
    ];
    let mut unknown: Vec<_> = args
        .keys()
        .filter(|k| !known.contains(&k.as_str()))
        .collect();
    if !unknown.is_empty() {
        unknown.sort();
        return Err(format!(
            "Unknown arguments for buddy_open_issue: {:?}",
            unknown
        ));
    }

    if let Some(provider) = args.get("provider") {
        if !valid_provider(provider) {
            return Err("buddy_open_issue: provider must be 'github' or 'gitlab'".to_string());
        }
    }

    let sanitized_title = args
        .get("title")
        .and_then(Value::as_str)
        .map(|t| capped_redacted(t, TITLE_MAX_CHARS));

    let sanitized_body = args
        .get("body")
        .and_then(Value::as_str)
        .map(|b| capped_redacted(b, BODY_MAX_CHARS));

    let validated_labels = if let Some(labels_value) = args.get("labels") {
        let raw: Vec<String> = labels_value
            .as_array()
            .ok_or("labels must be an array of strings")?
            .iter()
            .enumerate()
            .map(|(i, v)| {
                v.as_str()
                    .ok_or_else(|| format!("labels[{}] must be a string", i))
                    .map(|s| s.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let processed: Vec<Value> = crate::buddy::issues::sanitize_labels(&raw)
            .into_iter()
            .map(Value::String)
            .collect();
        Some(Value::Array(processed))
    } else {
        None
    };

    let mut forwarded = HashMap::new();
    if let Some(title) = sanitized_title {
        forwarded.insert("title".to_string(), Value::String(title));
    }
    if let Some(body) = sanitized_body {
        forwarded.insert("body".to_string(), Value::String(body));
    }
    if let Some(labels) = validated_labels {
        forwarded.insert("labels".to_string(), labels);
    }
    if let Some(provider) = args.get("provider").and_then(Value::as_str) {
        forwarded.insert("provider".to_string(), Value::String(provider.to_string()));
    }
    // Forward diagnostic/finding references so the issue runner can resolve a stored
    // diagnostic or synthesize reproduction context. Free-text fields are redacted + capped.
    for key in ["error", "source_file", "tool_name"] {
        if let Some(text) = args.get(key).and_then(Value::as_str) {
            let cleaned = capped_redacted(text, BODY_MAX_CHARS);
            if !cleaned.trim().is_empty() {
                forwarded.insert(key.to_string(), Value::String(cleaned));
            }
        }
    }
    if let Some(diagnostic_id) = args.get("diagnostic_id").and_then(Value::as_str) {
        forwarded.insert(
            "diagnostic_id".to_string(),
            Value::String(diagnostic_id.to_string()),
        );
    }
    if let Some(collected_at) = args.get("collected_at").and_then(Value::as_str) {
        forwarded.insert(
            "collected_at".to_string(),
            Value::String(collected_at.to_string()),
        );
    }
    if let Some(diagnostic_index) = args.get("diagnostic_index").and_then(Value::as_u64) {
        forwarded.insert(
            "diagnostic_index".to_string(),
            Value::from(diagnostic_index),
        );
    }
    // The autonomous subchat owns the issue-filing decision; reaching this wrapper means confirmed.
    forwarded.insert(
        "confidence".to_string(),
        Value::String("confirmed".to_string()),
    );
    Ok(forwarded)
}

#[async_trait]
impl Tool for ToolBuddyOpenIssue {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_open_issue".to_string(),
            display_name: "Buddy Open Issue".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Alias for buddy_create_issue that files a confirmed issue through the same Buddy issue runner.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "body": {"type": "string"},
                    "labels": {"type": "array", "items": {"type": "string"}},
                    "provider": {"type": "string", "description": "Issue tracker provider: 'github' or 'gitlab'. Defaults to the configured tracker."},
                    "error": {"type": "string", "description": "Error/finding text. Provide this (with source_file or tool_name) when the issue is not tied to a stored diagnostic."},
                    "source_file": {"type": "string", "description": "Source file the finding relates to. Gives reproduction context for the issue gate."},
                    "tool_name": {"type": "string", "description": "Tool the finding relates to. Gives reproduction context for the issue gate."},
                    "diagnostic_index": {"type": "number", "description": "Index of a stored Buddy diagnostic to attach."},
                    "diagnostic_id": {"type": "string", "description": "Stable id of a stored Buddy diagnostic to attach."},
                    "collected_at": {"type": "string", "description": "collected_at timestamp (RFC3339) of a stored Buddy diagnostic to attach."}
                },
                "required": ["title", "body"],
                "additionalProperties": false
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let forwarded = prepare_forwarded_args(args)?;
        let mut runner = self.runner();
        runner.tool_execute(ccx, tool_call_id, &forwarded).await
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    fn forwarded(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
        prepare_forwarded_args(&args(entries)).expect("args should be valid")
    }

    #[test]
    fn buddy_open_issue_redacts_secrets_in_body() {
        let out = forwarded(vec![
            ("title", json!("Leak sk-1234567890abcdef")),
            ("body", json!("token=abc123 and Bearer topsecret")),
            ("labels", json!(["sk-1234567890abcdef"])),
        ]);

        assert_eq!(out["title"], json!("Leak [REDACTED_SK_TOKEN]"));
        assert_eq!(out["body"], json!("token=[REDACTED] and Bearer [REDACTED]"));
        assert_eq!(out["labels"], json!(["[REDACTED_SK_TOKEN]"]));
    }

    #[test]
    fn buddy_open_issue_caps_title_at_120_chars() {
        let title = "a".repeat(140);
        let out = forwarded(vec![("title", json!(title)), ("body", json!("body"))]);
        let title = out["title"].as_str().expect("title should be string");

        assert_eq!(title.chars().count(), TITLE_MAX_CHARS);
        assert!(title.ends_with(TRUNCATED_SUFFIX));
    }

    #[test]
    fn buddy_open_issue_rejects_invalid_provider() {
        let err = prepare_forwarded_args(&args(vec![
            ("title", json!("Bug")),
            ("body", json!("Details")),
            ("provider", json!("jira")),
        ]))
        .expect_err("invalid provider should fail");

        assert_eq!(
            err,
            "buddy_open_issue: provider must be 'github' or 'gitlab'"
        );
    }

    #[test]
    fn buddy_open_issue_caps_labels_count_and_length() {
        let out = forwarded(vec![
            ("title", json!("Bug")),
            ("body", json!("Details")),
            (
                "labels",
                json!([
                    "one",
                    "two",
                    "three",
                    "four",
                    "five",
                    "six",
                    "this-label-is-way-too-long-to-forward-because-it-crosses-fifty-characters"
                ]),
            ),
        ]);

        assert_eq!(
            out["labels"],
            json!(["one", "two", "three", "four", "five"])
        );
    }

    #[test]
    fn buddy_open_issue_passes_valid_args_to_runner() {
        let out = forwarded(vec![
            ("title", json!("Bug")),
            ("body", json!("Details")),
            ("provider", json!("github")),
        ]);

        assert_eq!(out["title"], json!("Bug"));
        assert_eq!(out["body"], json!("Details"));
        assert_eq!(out["provider"], json!("github"));
        assert_eq!(out["confidence"], json!("confirmed"));
    }

    #[test]
    fn buddy_open_issue_rejects_unknown_arguments() {
        let err = prepare_forwarded_args(&args(vec![
            ("title", json!("Bug")),
            ("body", json!("Details")),
            ("labels", json!(["fix"])),
            ("provider", json!("github")),
            ("extra", json!("evil")),
        ]))
        .expect_err("unknown arg should fail");

        assert!(err.contains("Unknown arguments for buddy_open_issue"));
        assert!(err.contains("extra"));
    }

    #[test]
    fn buddy_open_issue_rejects_non_string_label() {
        let err = prepare_forwarded_args(&args(vec![
            ("title", json!("Bug")),
            ("body", json!("Details")),
            ("labels", json!(["ok", 42])),
        ]))
        .expect_err("non-string label should fail");

        assert!(err.contains("labels[1]"));
        assert!(err.contains("must be a string"));
    }

    #[test]
    fn buddy_open_issue_rejects_non_array_labels() {
        let err = prepare_forwarded_args(&args(vec![
            ("title", json!("Bug")),
            ("body", json!("Details")),
            ("labels", json!("not-an-array")),
        ]))
        .expect_err("non-array labels should fail");

        assert_eq!(err, "labels must be an array of strings");
    }

    #[test]
    fn buddy_open_issue_forwards_only_allowlisted_keys() {
        let out = forwarded(vec![
            ("title", json!("Bug")),
            ("body", json!("Details")),
            ("labels", json!(["fix"])),
            ("provider", json!("github")),
        ]);

        assert_eq!(out.len(), 5);
        assert!(out.contains_key("title"));
        assert!(out.contains_key("body"));
        assert!(out.contains_key("labels"));
        assert!(out.contains_key("provider"));
        assert!(out.contains_key("confidence"));
    }

    #[test]
    fn buddy_open_issue_accepts_valid_input() {
        let out = forwarded(vec![
            ("title", json!("Simple bug")),
            ("body", json!("Repro steps")),
        ]);

        assert_eq!(out["title"], json!("Simple bug"));
        assert_eq!(out["body"], json!("Repro steps"));
        assert_eq!(out["confidence"], json!("confirmed"));
    }

    #[test]
    fn buddy_open_issue_forwards_finding_fields() {
        let out = forwarded(vec![
            ("title", json!("Unsafe default")),
            ("body", json!("Risky config")),
            ("error", json!("hardcoded credential in config")),
            ("source_file", json!("src/config.rs")),
            ("tool_name", json!("security_scan")),
        ]);

        assert_eq!(out["error"], json!("hardcoded credential in config"));
        assert_eq!(out["source_file"], json!("src/config.rs"));
        assert_eq!(out["tool_name"], json!("security_scan"));
        assert_eq!(out["confidence"], json!("confirmed"));
    }

    #[test]
    fn buddy_open_issue_forwards_diagnostic_reference() {
        let out = forwarded(vec![
            ("title", json!("Crash")),
            ("body", json!("Stack trace")),
            ("diagnostic_id", json!("abc123")),
            ("collected_at", json!("2026-06-29T00:00:00+00:00")),
            ("diagnostic_index", json!(2)),
        ]);

        assert_eq!(out["diagnostic_id"], json!("abc123"));
        assert_eq!(out["collected_at"], json!("2026-06-29T00:00:00+00:00"));
        assert_eq!(out["diagnostic_index"], json!(2));
    }

    #[test]
    fn buddy_open_issue_redacts_forwarded_error() {
        let out = forwarded(vec![
            ("title", json!("Leak")),
            ("body", json!("details")),
            ("error", json!("token=abc123 leaked")),
            ("source_file", json!("src/a.rs")),
        ]);

        assert_eq!(out["error"], json!("token=[REDACTED] leaked"));
    }

    #[test]
    fn buddy_open_issue_skips_blank_finding_fields() {
        let out = forwarded(vec![
            ("title", json!("Bug")),
            ("body", json!("Details")),
            ("error", json!("   ")),
        ]);

        assert!(!out.contains_key("error"));
    }
}
