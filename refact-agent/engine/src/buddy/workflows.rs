use std::future::Future;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::app_state::AppState;
use super::types::BuddyActivity;

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
        if lower.trim() == "aborted" || lower.contains("original error: aborted") {
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
        match refact_chat_history::retry_policy::classify_user_error(error) {
            refact_chat_history::retry_policy::UserErrorCategory::ModelUnavailable => {
                Self::ModelUnavailable
            }
            refact_chat_history::retry_policy::UserErrorCategory::ContextTooLarge => {
                Self::ContextTooLarge
            }
            refact_chat_history::retry_policy::UserErrorCategory::InvalidRequest
            | refact_chat_history::retry_policy::UserErrorCategory::ToolSchemaInvalid => {
                Self::InvalidRequest
            }
            refact_chat_history::retry_policy::UserErrorCategory::ProviderTransient
            | refact_chat_history::retry_policy::UserErrorCategory::NetworkFailure
            | refact_chat_history::retry_policy::UserErrorCategory::StreamCorrupted => {
                Self::ProviderTransient
            }
            refact_chat_history::retry_policy::UserErrorCategory::ProviderRateLimit => {
                Self::ProviderRateLimit
            }
            refact_chat_history::retry_policy::UserErrorCategory::AuthenticationFailed => {
                Self::AuthenticationFailed
            }
            refact_chat_history::retry_policy::UserErrorCategory::BillingQuota => {
                Self::BillingQuota
            }
            refact_chat_history::retry_policy::UserErrorCategory::ContentPolicy => {
                Self::ContentPolicy
            }
            refact_chat_history::retry_policy::UserErrorCategory::Unknown => Self::Unknown,
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

pub fn workflow_failure_summary(category: &WorkflowFailureCategory, error: &str) -> String {
    let detail = super::jobs::autonomous_chats::redact_and_cap_text(error, 240);
    match category {
        WorkflowFailureCategory::ModelUnavailable => format!(
            "Model unavailable — check Buddy/default model settings. {}",
            detail
        ),
        WorkflowFailureCategory::ContextTooLarge => {
            "Context too large — Buddy needs a smaller prompt or compaction before retrying."
                .to_string()
        }
        WorkflowFailureCategory::ToolUnavailable => format!(
            "Tool unavailable — the workflow referenced a tool that is not registered. {}",
            detail
        ),
        WorkflowFailureCategory::ToolFailed => format!("Tool failed during workflow. {}", detail),
        WorkflowFailureCategory::InvalidRequest => format!("Invalid provider request. {}", detail),
        WorkflowFailureCategory::ProviderTransient => {
            format!("Provider temporarily unavailable. {}", detail)
        }
        WorkflowFailureCategory::ProviderRateLimit => {
            format!("Provider rate limit reached. {}", detail)
        }
        WorkflowFailureCategory::AuthenticationFailed => {
            format!("Authentication failed. {}", detail)
        }
        WorkflowFailureCategory::BillingQuota => {
            format!("Billing or quota limit reached. {}", detail)
        }
        WorkflowFailureCategory::ContentPolicy => {
            format!("Content policy blocked the request. {}", detail)
        }
        WorkflowFailureCategory::Cancelled => "Workflow cancelled before completion.".to_string(),
        WorkflowFailureCategory::Unknown => format!("Workflow failed. {}", detail),
    }
}

pub fn workflow_failure_report(
    workflow_id: &str,
    error: &str,
    chat_id: Option<String>,
) -> WorkflowFailureReport {
    let category = WorkflowFailureCategory::classify(error);
    WorkflowFailureReport {
        workflow_id: workflow_id.to_string(),
        summary: workflow_failure_summary(&category, error),
        detail: super::jobs::autonomous_chats::redact_and_cap_text(error, 1_000),
        category,
        chat_id,
    }
}

pub fn workflow_label(workflow_id: &str) -> &str {
    match workflow_id {
        "commit_msg" => "commit message generation",
        "follow_up" => "follow-up suggestions",
        "compression" => "chat compression",
        "memory_extract" => "memo extraction",
        "knowledge_update" => "knowledge graph update",
        "title_generating" => "title generation",
        // Legacy IDs still map to labels for backwards-compat transcripts
        "commit_message" => "commit message generation",
        "compress_trajectory" => "chat compression",
        "memo_extraction" => "memo extraction",
        "kg_enrich" => "knowledge graph enrichment",
        "kg_deprecate" => "knowledge cleanup",
        _ => workflow_id,
    }
}

/// Maps internal workflow IDs to canonical Buddy signal_type names.
/// The GUI uses these names in its signal catalog.
pub fn canonical_signal_type(workflow_id: &str) -> &str {
    match workflow_id {
        "commit_message" | "commit_msg" => "commit_msg",
        "compress_trajectory" | "compression" => "compression",
        "memo_extraction" | "memory_extract" => "memory_extract",
        "kg_enrich" | "kg_deprecate" | "knowledge_update" => "knowledge_update",
        "title_generating" | "title_generation" => "title_generating",
        "follow_up" => "generating",
        other => other,
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkflowEntry {
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
struct WorkflowTranscript {
    entries: Vec<WorkflowEntry>,
}

const MAX_ENTRIES: usize = 100;

pub async fn buddy_wrap_workflow<T, F, Fut>(
    gcx: AppState,
    workflow_id: &str,
    icon: &str,
    xp: u64,
    summary_fn: impl Fn(&T) -> String,
    workflow_fn: F,
) -> Result<T, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    let label = workflow_label(workflow_id);
    let signal_type = canonical_signal_type(workflow_id);
    let dedupe_key = format!("workflow_{}", workflow_id);
    let mut started = crate::buddy::actor::make_runtime_event(
        signal_type,
        &format!("Running {}...", label),
        "system",
        &dedupe_key,
        "started",
        None,
    );
    started.speech_text = Some(format!("I'm working on {}...", label));
    started.scene = Some("working".to_string());
    started.persistent = true;
    crate::buddy::actor::buddy_enqueue_event(gcx.clone(), started).await;

    let result = workflow_fn().await;

    let (success, summary) = match &result {
        Ok(output) => (true, summary_fn(output)),
        Err(e) => (false, e.clone()),
    };

    let failure_report = result
        .as_ref()
        .err()
        .map(|error| workflow_failure_report(workflow_id, error, None));

    let buddy_arc = gcx.buddy.buddy.clone();
    let voice_gcx = gcx.clone();
    let project_dirs = crate::files_correction::get_project_dirs(gcx.gcx.clone()).await;
    let project_root = project_dirs.into_iter().next();
    let workflow_id_owned = workflow_id.to_string();
    let icon_owned = icon.to_string();

    tokio::spawn(async move {
        let activity = BuddyActivity {
            icon: icon_owned,
            title: summary.clone(),
            description: String::new(),
            timestamp: Utc::now().to_rfc3339(),
            activity_type: "workflow".to_string(),
            chat_id: None,
            failure_category: None,
            failure_summary: None,
        };

        let mut completed_quest = None;
        let mut quest_voice_state = None;
        let mut failure_write = None;
        {
            let mut buddy = buddy_arc.lock().await;
            if let Some(svc) = buddy.as_mut() {
                let status = if success { "completed" } else { "failed" };
                svc.complete_runtime_event(&dedupe_key, status);
                if success {
                    svc.add_activity(activity);
                    crate::buddy::state::grant_xp(&mut svc.state, xp);
                    let now = Utc::now().to_rfc3339();
                    if let Some(ws) = svc
                        .state
                        .workflow_summaries
                        .iter_mut()
                        .find(|ws| ws.workflow_id == workflow_id_owned)
                    {
                        ws.run_count = ws.run_count.saturating_add(1);
                        ws.last_run = Some(now.clone());
                        ws.last_outcome = Some("success".to_string());
                        ws.failure_category = None;
                        ws.failure_summary = None;
                    } else {
                        svc.state.workflow_summaries.push(
                            crate::buddy::types::BuddyWorkflowSummary {
                                workflow_id: workflow_id_owned.clone(),
                                last_run: Some(now.clone()),
                                run_count: 1,
                                last_outcome: Some("success".to_string()),
                                failure_category: None,
                                failure_summary: None,
                            },
                        );
                    }
                    svc.refresh_active_quest();
                    svc.dirty = true;
                    let _ = svc
                        .events_tx
                        .send(crate::buddy::events::BuddyEvent::StateUpdated {
                            state: svc.state.clone(),
                        });
                    let reward = svc
                        .state
                        .active_quest
                        .as_ref()
                        .filter(|quest| quest.status == "active" && quest.progress >= quest.goal)
                        .map(|quest| quest.reward_xp);
                    if let Some(reward) = reward {
                        completed_quest =
                            crate::buddy::state::complete_active_quest(&mut svc.state);
                        quest_voice_state = Some((
                            svc.state.personality.clone(),
                            svc.state.identity.name.clone(),
                            svc.pulse.clone(),
                            reward,
                        ));
                        svc.dirty = true;
                        let _ =
                            svc.events_tx
                                .send(crate::buddy::events::BuddyEvent::StateUpdated {
                                    state: svc.state.clone(),
                                });
                    }
                } else if let Some(report) = failure_report {
                    failure_write = svc.record_workflow_failure_report(report);
                } else {
                    tracing::warn!(
                        "buddy: workflow {} failed without a failure report",
                        workflow_id_owned
                    );
                }
            }
        }

        if let Some((path, report)) = failure_write {
            crate::buddy::actor::BuddyService::append_workflow_failure_transcript(&path, &report)
                .await;
        } else if let Some(ref root) = project_root {
            crate::buddy::actor::BuddyService::append_workflow_transcript_to_path(
                root,
                &workflow_id_owned,
                &summary,
                success,
            )
            .await;
        }

        if let (Some(quest), Some((persona, identity_name, pulse, reward))) =
            (completed_quest, quest_voice_state)
        {
            let completed = crate::buddy::actor::complete_quest_with_voice(
                voice_gcx.clone(),
                quest,
                persona,
                identity_name,
                pulse,
            )
            .await;
            crate::buddy::actor::buddy_update_speech(voice_gcx.clone(), completed.speech).await;
            crate::buddy::actor::buddy_apply(voice_gcx.clone(), completed.mutation).await;
            if reward > 0 {
                let buddy_arc = voice_gcx.buddy.buddy.clone();
                let mut buddy = buddy_arc.lock().await;
                if let Some(svc) = buddy.as_mut() {
                    svc.grant_xp(reward);
                }
            }
        }
    });

    result
}

pub async fn append_workflow_entry(path: &std::path::Path, output_summary: &str, success: bool) {
    append_workflow_entry_with_failure(path, output_summary, success, None).await;
}

pub async fn append_workflow_entry_with_failure(
    path: &std::path::Path,
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

    if let Err(e) = super::storage::atomic_write_json(path, &transcript).await {
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
    fn workflow_failure_report_classifies_model_and_tool_errors() {
        let report = workflow_failure_report(
            "commit_msg",
            "OpenAI 404: model refact/gpt-4.1-nano not found",
            Some("chat-1".to_string()),
        );
        assert_eq!(report.category, WorkflowFailureCategory::ModelUnavailable);
        assert!(report.summary.contains("Model unavailable"));
        assert_eq!(report.chat_id.as_deref(), Some("chat-1"));

        let report = workflow_failure_report(
            "commit_msg",
            "Error: tool 'buddy_log_activity' not found",
            None,
        );
        assert_eq!(report.category, WorkflowFailureCategory::ToolUnavailable);
        assert!(report.summary.contains("Tool unavailable"));
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
