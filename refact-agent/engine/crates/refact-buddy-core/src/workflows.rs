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

/// How a workflow's runs are allowed to reach the user's Desk.
///
/// `UserFacing` is the default for every workflow: each run publishes a work
/// item the user can read. `Housekeeping` is for continuous background
/// maintenance whose individual runs are not news to anybody — those publish
/// no work item at all and become visible only when they are durably blocked,
/// as exactly one item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowVisibility {
    #[default]
    UserFacing,
    Housekeeping,
}

impl WorkflowVisibility {
    pub fn is_housekeeping(self) -> bool {
        matches!(self, Self::Housekeeping)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserFacing => "user_facing",
            Self::Housekeeping => "housekeeping",
        }
    }
}

/// Classification is declared per workflow id and defaults to `UserFacing`, so
/// a workflow is never silently hidden by being forgotten here.
///
/// `memo_extraction` is housekeeping: it sweeps abandoned trajectories on a
/// five-minute timer, so its per-run result is maintenance noise rather than a
/// reading the user asked for. Publishing it per run collapsed 7,867 distinct
/// trajectories onto one constant-fingerprint row carrying 1,698 artifact
/// revisions; giving each run a real fingerprint would have produced ~1,698
/// separate Desk rows instead. Neither is a report anybody wants, so it
/// publishes nothing until it is blocked.
pub fn workflow_visibility(workflow_id: &str) -> WorkflowVisibility {
    if HOUSEKEEPING_WORKFLOWS.contains(&workflow_id) {
        WorkflowVisibility::Housekeeping
    } else {
        WorkflowVisibility::UserFacing
    }
}

/// Every workflow id classified as `Housekeeping`, enumerable so an upgrade can
/// retire the per-run rows these ids published before the classification existed.
/// `memory_extract` is the legacy alias of `memo_extraction` and is listed for
/// the same reason, not because a second sweep exists.
pub const HOUSEKEEPING_WORKFLOWS: &[&str] = &["memo_extraction", "memory_extract"];

/// Consecutive failed housekeeping runs before background maintenance counts as
/// durably blocked rather than transiently unlucky. Deliberately the same 3 as
/// `trajectory_memos`' rate-limit breaker threshold and its per-trajectory
/// abandonment threshold, so the Desk becomes visible at the same moment the
/// loop actually stops doing work.
pub const HOUSEKEEPING_BLOCK_THRESHOLD: u32 = 3;

/// The one reading a blocked housekeeping loop owes the user: what stopped, and
/// what they can do about it. Deliberately free of counters and timestamps so
/// repeated publication folds into the existing revision instead of growing a
/// revision per retry.
pub fn housekeeping_blocked_report(
    workflow_id: &str,
    category: Option<&WorkflowFailureCategory>,
) -> (String, String) {
    let label = workflow_label(workflow_id);
    let title = format!("Background {label} is blocked");
    let cause = match category {
        Some(WorkflowFailureCategory::ProviderRateLimit) => {
            "the configured provider refused the requests with a rate limit"
        }
        Some(WorkflowFailureCategory::BillingQuota) => "a billing or quota limit was reached",
        Some(WorkflowFailureCategory::AuthenticationFailed) => {
            "the provider rejected the credentials"
        }
        Some(WorkflowFailureCategory::ModelUnavailable) => "the model it runs on is not available",
        Some(WorkflowFailureCategory::ContextTooLarge) => {
            "the inputs it was given exceeded the model's context"
        }
        Some(WorkflowFailureCategory::ToolUnavailable)
        | Some(WorkflowFailureCategory::ToolFailed) => "a tool it depends on failed",
        Some(WorkflowFailureCategory::ContentPolicy) => "a content policy blocked the requests",
        _ => "its runs kept failing",
    };
    let body = format!(
        "Background {label} stopped retrying because {cause}. \
         This is maintenance Buddy normally does silently, so nothing you were working on has been lost \
         and no earlier result has been discarded.\n\n\
         What you can do: wait for the provider to recover, switch the Buddy model slot to a provider that is answering, \
         or turn the background sweep off. Buddy retries on its own and this item clears as soon as a run succeeds again."
    );
    (title, body)
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
