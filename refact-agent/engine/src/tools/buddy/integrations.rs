use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::buddy::actor::redact_sensitive;
use crate::call_validation::ContextEnum;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const TITLE_MAX_CHARS: usize = 120;
const BODY_MAX_CHARS: usize = 4000;
const LABEL_MAX_CHARS: usize = 50;
const MAX_LABELS: usize = 5;
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
    truncate_with_suffix(&redact_sensitive(value), max_chars)
}

fn valid_provider(provider: &Value) -> bool {
    matches!(provider.as_str(), Some("github") | Some("gitlab"))
}

fn prepare_forwarded_args(args: &HashMap<String, Value>) -> Result<HashMap<String, Value>, String> {
    let known = ["title", "body", "labels", "provider", "confidence"];
    let mut unknown: Vec<_> = args.keys().filter(|k| !known.contains(&k.as_str())).collect();
    if !unknown.is_empty() {
        unknown.sort();
        return Err(format!("Unknown arguments for buddy_open_issue: {:?}", unknown));
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
        let raw: Vec<Value> = labels_value
            .as_array()
            .ok_or("labels must be an array of strings")?
            .iter()
            .enumerate()
            .map(|(i, v)| {
                v.as_str()
                    .ok_or_else(|| format!("labels[{}] must be a string", i))
                    .map(|s| Value::String(redact_sensitive(s)))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let processed: Vec<Value> = raw
            .into_iter()
            .filter(|v| v.as_str().map_or(false, |s| s.chars().count() <= LABEL_MAX_CHARS))
            .take(MAX_LABELS)
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
    // The autonomous subchat owns the issue-filing decision; reaching this wrapper means confirmed.
    forwarded.insert("confidence".to_string(), Value::String("confirmed".to_string()));
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
                    "provider": {"type": "string"}
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
}
