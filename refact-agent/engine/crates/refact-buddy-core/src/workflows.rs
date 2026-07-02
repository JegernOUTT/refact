use std::path::Path;

use chrono::Utc;
use refact_core::retry_policy::{classify_user_error, UserErrorCategory};
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowFailureCategory {
    ModelUnavailable,
    ContextTooLarge,
    ToolUnavailable,
    ToolFailed,
    InvalidRequest,
    ProviderTransient,
    ProviderRateLimit,
    AuthenticationFailed,
    BillingQuota,
    ContentPolicy,
    Cancelled,
    Unknown,
}

impl WorkflowFailureCategory {
    pub fn classify(error: &str) -> Self {
        let lower = error.to_lowercase();
        if lower.trim() == "aborted"
            || lower.contains("original error: aborted")
            || lower.contains("cancelled")
            || lower.contains("canceled")
            || lower.contains("request aborted")
            || lower.contains("operation aborted")
            || lower.contains("aborterror")
            || lower.contains("context canceled")
        {
            return Self::Cancelled;
        }
        if lower.contains("tool '") && lower.contains("not found")
            || lower.contains("unknown tool")
            || lower.contains("tool not found")
        {
            return Self::ToolUnavailable;
        }
        if lower.contains("tool failed")
            || lower.contains("tool execution failed")
            || lower.contains("tool call failed")
        {
            return Self::ToolFailed;
        }
        match classify_user_error(error) {
            UserErrorCategory::ModelUnavailable => Self::ModelUnavailable,
            UserErrorCategory::ContextTooLarge => Self::ContextTooLarge,
            UserErrorCategory::InvalidRequest | UserErrorCategory::ToolSchemaInvalid => {
                Self::InvalidRequest
            }
            UserErrorCategory::ProviderTransient
            | UserErrorCategory::NetworkFailure
            | UserErrorCategory::StreamCorrupted => Self::ProviderTransient,
            UserErrorCategory::ProviderRateLimit => Self::ProviderRateLimit,
            UserErrorCategory::AuthenticationFailed => Self::AuthenticationFailed,
            UserErrorCategory::BillingQuota => Self::BillingQuota,
            UserErrorCategory::ContentPolicy => Self::ContentPolicy,
            UserErrorCategory::Unknown => Self::Unknown,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ModelUnavailable => "model_unavailable",
            Self::ContextTooLarge => "context_too_large",
            Self::ToolUnavailable => "tool_unavailable",
            Self::ToolFailed => "tool_failed",
            Self::InvalidRequest => "invalid_request",
            Self::ProviderTransient => "provider_transient",
            Self::ProviderRateLimit => "provider_rate_limit",
            Self::AuthenticationFailed => "authentication_failed",
            Self::BillingQuota => "billing_quota",
            Self::ContentPolicy => "content_policy",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::ModelUnavailable => "Model unavailable",
            Self::ContextTooLarge => "Context too large",
            Self::ToolUnavailable => "Tool unavailable",
            Self::ToolFailed => "Tool failed",
            Self::InvalidRequest => "Invalid request",
            Self::ProviderTransient => "Provider temporarily unavailable",
            Self::ProviderRateLimit => "Rate limit reached",
            Self::AuthenticationFailed => "Authentication failed",
            Self::BillingQuota => "Billing or quota limit reached",
            Self::ContentPolicy => "Content policy blocked request",
            Self::Cancelled => "Workflow cancelled",
            Self::Unknown => "Workflow failed",
        }
    }

    pub fn priority(&self) -> &'static str {
        match self {
            Self::AuthenticationFailed | Self::BillingQuota | Self::ContentPolicy => "critical",
            Self::ModelUnavailable
            | Self::ContextTooLarge
            | Self::ToolUnavailable
            | Self::ToolFailed
            | Self::InvalidRequest => "high",
            Self::ProviderTransient | Self::ProviderRateLimit => "normal",
            Self::Cancelled | Self::Unknown => "normal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowFailureReport {
    pub workflow_id: String,
    pub category: WorkflowFailureCategory,
    pub summary: String,
    pub detail: String,
    pub chat_id: Option<String>,
}

pub fn workflow_label(workflow_id: &str) -> &str {
    match workflow_id {
        "commit_msg" => "commit message generation",
        "follow_up" => "follow-up suggestions",
        "compression" => "chat compression",
        "memory_extract" => "memo extraction",
        "knowledge_update" => "knowledge graph update",
        "title_generating" => "title generation",
        "commit_message" => "commit message generation",
        "compress_trajectory" => "chat compression",
        "memo_extraction" => "memo extraction",
        "kg_enrich" => "knowledge graph enrichment",
        "kg_deprecate" => "knowledge cleanup",
        _ => workflow_id,
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkflowEntry {
    timestamp: String,
    input_summary: String,
    output_summary: String,
    success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure_category: Option<WorkflowFailureCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure_summary: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkflowTranscript {
    entries: Vec<WorkflowEntry>,
}

const MAX_ENTRIES: usize = 100;

pub async fn append_workflow_entry(path: &Path, output_summary: &str, success: bool) {
    append_workflow_entry_with_failure(path, output_summary, success, None).await;
}

pub async fn append_workflow_entry_with_failure(
    path: &Path,
    output_summary: &str,
    success: bool,
    failure: Option<(&WorkflowFailureCategory, &str)>,
) {
    let entry = WorkflowEntry {
        timestamp: Utc::now().to_rfc3339(),
        input_summary: String::new(),
        output_summary: output_summary.to_string(),
        success,
        failure_category: failure.map(|(category, _)| category.clone()),
        failure_summary: failure
            .map(|(_, summary)| summary.trim().to_string())
            .filter(|summary| !summary.is_empty()),
    };

    let mut transcript = match tokio::fs::read_to_string(path).await {
        Ok(content) => serde_json::from_str::<WorkflowTranscript>(&content)
            .unwrap_or(WorkflowTranscript { entries: vec![] }),
        Err(_) => WorkflowTranscript { entries: vec![] },
    };

    transcript.entries.push(entry);
    if transcript.entries.len() > MAX_ENTRIES {
        let drain = transcript.entries.len() - MAX_ENTRIES;
        transcript.entries.drain(0..drain);
    }

    if let Err(e) = crate::storage::atomic_write_json(path, &transcript).await {
        warn!(
            "buddy: failed to write workflow transcript {:?}: {}",
            path, e
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_failure_category_classifies_model_tool_and_cancellation_errors() {
        assert_eq!(
            WorkflowFailureCategory::classify("OpenAI 404: model refact/gpt-4.1-nano not found"),
            WorkflowFailureCategory::ModelUnavailable
        );
        assert_eq!(
            WorkflowFailureCategory::classify("Error: tool 'buddy_log_activity' not found"),
            WorkflowFailureCategory::ToolUnavailable
        );

        for error in [
            "cancelled by user",
            "canceled by user",
            "request aborted by client",
            "operation aborted",
            "AbortError: stopped",
            "context canceled",
        ] {
            assert_eq!(
                WorkflowFailureCategory::classify(error),
                WorkflowFailureCategory::Cancelled,
                "{error}"
            );
        }
    }

    #[test]
    fn workflow_label_mapping() {
        assert_eq!(
            workflow_label("commit_message"),
            "commit message generation"
        );
        assert_eq!(workflow_label("follow_up"), "follow-up suggestions");
        assert_eq!(workflow_label("compress_trajectory"), "chat compression");
        assert_eq!(workflow_label("memo_extraction"), "memo extraction");
        assert_eq!(workflow_label("kg_enrich"), "knowledge graph enrichment");
        assert_eq!(workflow_label("kg_deprecate"), "knowledge cleanup");
        assert_eq!(workflow_label("title_generating"), "title generation");
        assert_eq!(workflow_label("unknown_workflow"), "unknown_workflow");
    }

    #[tokio::test]
    async fn workflow_transcript_records_failure_category() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workflow.json");

        append_workflow_entry_with_failure(
            &path,
            "model unavailable: refact/gpt-4.1-nano",
            false,
            Some((
                &WorkflowFailureCategory::ModelUnavailable,
                "Model unavailable",
            )),
        )
        .await;

        let value: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        let entry = &value["entries"][0];
        assert_eq!(entry["success"], false);
        assert_eq!(entry["failure_category"], "model_unavailable");
        assert_eq!(entry["failure_summary"], "Model unavailable");
    }
}
