use std::future::Future;

use chrono::Utc;

pub use refact_buddy_core::workflows::*;

use crate::app_state::AppState;
use super::types::BuddyActivity;

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
    if !crate::buddy::actor::validate_workflow_id(workflow_id) {
        return Err(format!("invalid workflow_id: {workflow_id}"));
    }

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

    let handle = tokio::spawn(async move {
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
                svc.complete_runtime_event(&dedupe_key, "completed");
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
                } else if let Some(report) = failure_report.as_ref() {
                    failure_write = svc.record_workflow_failure_report(report.clone());
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
        } else if let (false, Some(report), Some(root)) =
            (success, failure_report.as_ref(), project_root.as_ref())
        {
            let path = root.join(format!(
                ".refact/buddy/chats/workflows/{}.json",
                report.workflow_id
            ));
            crate::buddy::actor::BuddyService::append_workflow_failure_transcript(&path, report)
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

    let untracked_handle = {
        let mut buddy = gcx.buddy.buddy.lock().await;
        if let Some(svc) = buddy.as_mut() {
            svc.track_background_task(handle);
            None
        } else {
            Some(handle)
        }
    };
    if let Some(handle) = untracked_handle {
        let _ = handle.await;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn wait_for_workflow_side_effects() {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    async fn wait_for_path(path: &std::path::Path) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if tokio::fs::try_exists(path).await.unwrap_or(false) {
                return;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    }

    #[test]
    fn workflow_failure_report_classifies_model_tool_and_cancellation_errors() {
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

        for error in [
            "cancelled by user",
            "canceled by user",
            "request aborted by client",
            "operation aborted",
            "AbortError: stopped",
            "context canceled",
        ] {
            let report = workflow_failure_report("commit_msg", error, None);
            assert_eq!(
                report.category,
                WorkflowFailureCategory::Cancelled,
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn workflow_wrapper_rejects_invalid_id_before_event_or_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }
        let (tx, mut rx) = tokio::sync::broadcast::channel(16);
        *app.buddy.buddy.lock().await = Some(crate::buddy::actor::BuddyService::new(
            dir.path().to_path_buf(),
            crate::buddy::state::default_buddy_state(),
            crate::buddy::settings::BuddySettings::default(),
            Vec::new(),
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        ));

        let result = buddy_wrap_workflow(
            app.clone(),
            "../bad",
            "⚙️",
            1,
            |_| "ok".to_string(),
            || async { Ok::<_, String>(()) },
        )
        .await;

        assert_eq!(result.unwrap_err(), "invalid workflow_id: ../bad");
        let lock = app.buddy.buddy.lock().await;
        let svc = lock.as_ref().unwrap();
        assert!(svc.runtime_queue.items.is_empty());
        assert!(svc.state.recent_activities.is_empty());
        assert!(svc.state.workflow_summaries.is_empty());
        assert!(rx.try_recv().is_err());
        assert!(
            !tokio::fs::try_exists(dir.path().join(".refact/buddy/chats/workflows/bad.json"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn workflow_wrapper_failure_fallback_transcript_uses_report_not_raw_error() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }
        *app.buddy.buddy.lock().await = None;
        let raw_error = "OpenAI 404: model secret-model-not-found token=rawsecret not found";

        let result = buddy_wrap_workflow(
            app.clone(),
            "commit_msg",
            "⚙️",
            1,
            |_| "ok".to_string(),
            || async move { Err::<(), _>(raw_error.to_string()) },
        )
        .await;

        assert_eq!(result.unwrap_err(), raw_error);
        wait_for_workflow_side_effects().await;
        let path = dir
            .path()
            .join(".refact/buddy/chats/workflows/commit_msg.json");
        wait_for_path(&path).await;
        let value: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        let entry = &value["entries"][0];
        let output = entry["output_summary"].as_str().unwrap();
        let summary = entry["failure_summary"].as_str().unwrap();
        assert_eq!(entry["success"], false);
        assert_eq!(entry["failure_category"], "model_unavailable");
        assert!(summary.contains("Model unavailable"));
        assert!(output.contains("[REDACTED"));
        assert!(!output.contains("rawsecret"));
        assert!(!summary.contains("rawsecret"));
    }

    #[tokio::test]
    async fn workflow_wrapper_failure_has_single_durable_failure_runtime_surface() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }
        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        *app.buddy.buddy.lock().await = Some(crate::buddy::actor::BuddyService::new(
            dir.path().to_path_buf(),
            crate::buddy::state::default_buddy_state(),
            crate::buddy::settings::BuddySettings::default(),
            Vec::new(),
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        ));

        let result = buddy_wrap_workflow(
            app.clone(),
            "commit_msg",
            "⚙️",
            1,
            |_| "ok".to_string(),
            || async {
                Err::<(), _>("OpenAI 404: model refact/gpt-4.1-nano not found".to_string())
            },
        )
        .await;

        assert!(result.is_err());
        wait_for_workflow_side_effects().await;
        let lock = app.buddy.buddy.lock().await;
        let svc = lock.as_ref().unwrap();
        let workflow_events = svc
            .runtime_queue
            .items
            .iter()
            .filter(|event| event.dedupe_key.as_deref() == Some("workflow_commit_msg"))
            .collect::<Vec<_>>();
        assert_eq!(workflow_events.len(), 1);
        assert_eq!(workflow_events[0].status, "completed");
        assert!(workflow_events[0].failure_category.is_none());
        let failure_events = svc
            .runtime_queue
            .items
            .iter()
            .filter(|event| {
                event
                    .dedupe_key
                    .as_deref()
                    .is_some_and(|key| key.starts_with("workflow_failure:commit_msg:"))
            })
            .collect::<Vec<_>>();
        assert_eq!(failure_events.len(), 1);
        assert_eq!(failure_events[0].status, "failed");
        assert_eq!(
            failure_events[0].failure_category.as_deref(),
            Some("model_unavailable")
        );
        assert!(failure_events[0].persistent);
    }
}
