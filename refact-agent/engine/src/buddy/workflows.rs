use std::future::Future;

use chrono::Utc;

pub use refact_buddy_core::workflows::*;

use crate::app_state::AppState;
use super::types::BuddyActivity;
use super::outcome_runtime::terminal_from_workflow_result;

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

#[derive(Clone, Debug)]
pub(crate) struct WorkflowReport {
    pub title: String,
    pub body: String,
    pub terminal: super::outcome_runtime::WorkflowTerminal,
    pub evidence: Vec<refact_buddy_core::operator_types::EvidenceRef>,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkflowReportScope {
    pub observed_scope: String,
    pub input_fingerprint: String,
}

pub(crate) async fn buddy_wrap_workflow_report<T, F, Fut>(
    gcx: AppState,
    workflow_id: &str,
    icon: &str,
    xp: u64,
    summary_fn: impl Fn(&T) -> String,
    scope: WorkflowReportScope,
    report_fn: impl FnOnce(&Result<T, String>) -> WorkflowReport,
    workflow_fn: F,
) -> Result<T, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    buddy_wrap_workflow_inner(
        gcx,
        workflow_id,
        icon,
        xp,
        summary_fn,
        Some(scope),
        |result| Some(report_fn(result)),
        workflow_fn,
    )
    .await
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
    buddy_wrap_workflow_inner(
        gcx,
        workflow_id,
        icon,
        xp,
        summary_fn,
        None,
        |_| None,
        workflow_fn,
    )
    .await
}

async fn buddy_wrap_workflow_inner<T, F, Fut>(
    gcx: AppState,
    workflow_id: &str,
    icon: &str,
    _xp: u64,
    summary_fn: impl Fn(&T) -> String,
    scope: Option<WorkflowReportScope>,
    report_fn: impl FnOnce(&Result<T, String>) -> Option<WorkflowReport>,
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

    let outcome_runtime = {
        let buddy = gcx.buddy.buddy.lock().await;
        buddy
            .as_ref()
            .and_then(|service| service.outcome_runtime.clone())
    };
    let outcome_start = outcome_runtime.as_ref().map(|runtime| {
        let mut start = runtime.start_workflow(workflow_id, Utc::now().to_rfc3339());
        if let Some(scope) = scope {
            start.coverage = scope.observed_scope;
            start.input_fingerprint = scope.input_fingerprint;
        }
        start
    });
    if let (Some(runtime), Some(start)) = (&outcome_runtime, &outcome_start) {
        if let Err(error) = runtime.begin_workflow(start.clone()).await {
            tracing::error!(workflow_id, error = %error, "buddy workflow start was not persisted; crash recovery capture unavailable");
        }
    }

    let result = workflow_fn().await;

    let (success, summary) = match &result {
        Ok(output) => (true, summary_fn(output)),
        Err(e) => (false, e.clone()),
    };

    if let (Some(runtime), Some(start)) = (outcome_runtime, outcome_start) {
        // A housekeeping workflow's terminal result never becomes a per-run work
        // item. The outcome is still recorded; the Desk stays empty until the
        // loop is durably blocked.
        let publication = if workflow_visibility(workflow_id).is_housekeeping() {
            let terminal = match result.as_ref() {
                Ok(_) => super::outcome_runtime::WorkflowTerminal::Inconclusive,
                Err(error) => terminal_from_workflow_result(&Err(error.clone())),
            };
            let category = result
                .as_ref()
                .err()
                .map(|error| WorkflowFailureCategory::classify(error));
            runtime
                .publish_housekeeping_result(start, terminal, category)
                .await
                .map(|()| None)
        } else if let Some(report) = report_fn(&result) {
            runtime
                .publish_report(
                    start,
                    report.title,
                    report.body,
                    report.evidence,
                    report.terminal,
                )
                .await
                .map(Some)
        } else {
            let terminal = match result.as_ref() {
                Ok(_) => super::outcome_runtime::WorkflowTerminal::Inconclusive,
                Err(error) => terminal_from_workflow_result(&Err(error.clone())),
            };
            runtime
                .publish_workflow_result(start, summary.clone(), terminal)
                .await
                .map(Some)
        };
        if let Err(error) = publication {
            tracing::error!(
                workflow_id,
                error = %error,
                "buddy operator outcome was not persisted; no operator delivery was recorded"
            );
        }
    }

    let failure_report = result
        .as_ref()
        .err()
        .map(|error| workflow_failure_report(workflow_id, error, None));

    let buddy_arc = gcx.buddy.buddy.clone();
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

        let mut failure_write = None;
        let mut recovery_runtime = None;
        {
            let mut buddy = buddy_arc.lock().await;
            if let Some(svc) = buddy.as_mut() {
                svc.complete_runtime_event(&dedupe_key, "completed");
                if success {
                    recovery_runtime = svc.record_provider_rate_limit_recovery();
                    svc.add_activity(activity);
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
                                ..Default::default()
                            },
                        );
                    }
                    svc.dirty = true;
                    let _ = svc
                        .events_tx
                        .send(crate::buddy::events::BuddyEvent::StateUpdated {
                            state: svc.state.clone(),
                        });
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

        if let Some(runtime) = recovery_runtime {
            if let Err(error) = runtime.clear_provider_rate_limit_work().await {
                tracing::warn!(%error, "buddy: provider rate-limit recovery was not persisted");
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

    async fn isolated_app() -> (
        tempfile::TempDir,
        AppState,
        super::super::outcome_runtime::OutcomeRuntime,
    ) {
        use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity};
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        let app = AppState::from_gcx(gcx).await;
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![dir.path().to_path_buf()];
        let runtime = super::super::outcome_runtime::OutcomeRuntime::open(
            dir.path(),
            ProjectIdentity {
                project_id: "workflow-test".into(),
                canonical_root: dir.path().to_string_lossy().into_owned(),
            },
            WorkerIdentity {
                worker_id: "worker".into(),
                engine_instance_id: "engine".into(),
                epoch: 1,
            },
        )
        .unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let mut service = crate::buddy::actor::BuddyService::new(
            dir.path().to_path_buf(),
            crate::buddy::state::default_buddy_state(),
            crate::buddy::settings::BuddySettings::default(),
            Vec::new(),
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        service.outcome_runtime = Some(runtime.clone());
        *app.buddy.buddy.lock().await = Some(service);
        (dir, app, runtime)
    }

    async fn drain_side_effects(app: &AppState) {
        let tasks = app
            .buddy
            .buddy
            .lock()
            .await
            .as_mut()
            .unwrap()
            .take_background_tasks();
        for task in tasks {
            task.await.unwrap();
        }
    }

    fn typed_report(body: String) -> WorkflowReport {
        WorkflowReport {
            title: "Trusted proposal".into(),
            body,
            terminal: super::super::outcome_runtime::WorkflowTerminal::Completed,
            evidence: vec![],
        }
    }

    fn typed_scope() -> WorkflowReportScope {
        WorkflowReportScope {
            observed_scope: "Proposal only; no application or verification".into(),
            input_fingerprint: "sha256:exact-input".into(),
        }
    }

    #[tokio::test]
    async fn typed_wrapper_publishes_full_report_and_preserves_legacy_summary() {
        use refact_buddy_core::operator_types::RunStatus;
        let (dir, app, runtime) = isolated_app().await;
        let message = "Subject\n\nDetailed proposal body".to_string();
        let result = buddy_wrap_workflow_report(
            app.clone(),
            "commit_message",
            "x",
            0,
            |_: &String| "Legacy summary".into(),
            typed_scope(),
            |result| typed_report(result.as_ref().unwrap().clone()),
            || async { Ok(message.clone()) },
        )
        .await;
        assert_eq!(result, Ok(message.clone()));
        drain_side_effects(&app).await;
        let work = runtime
            .list_work_items(None, 10)
            .await
            .unwrap()
            .entries
            .remove(0)
            .value;
        let artifact = runtime
            .get_artifact_revision(work.current_artifact_id.unwrap(), None)
            .await
            .unwrap()
            .unwrap()
            .value;
        assert_eq!(artifact.title, "Trusted proposal");
        assert_eq!(artifact.body, message);
        let outcome = runtime
            .list_work_outcomes(work.id, None, 10)
            .await
            .unwrap()
            .entries
            .remove(0)
            .value;
        assert_eq!(outcome.status, RunStatus::Completed);
        assert_eq!(outcome.input_fingerprint, "sha256:exact-input");
        assert_eq!(
            outcome.coverage,
            "Proposal only; no application or verification"
        );
        assert!(outcome.effects.is_empty());
        assert!(artifact.proposed_action_ids.is_empty());
        let transcript: serde_json::Value = serde_json::from_str(
            &tokio::fs::read_to_string(
                dir.path()
                    .join(".refact/buddy/chats/workflows/commit_message.json"),
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(transcript["entries"][0]["output_summary"], "Legacy summary");
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn compatibility_wrapper_never_infers_no_change_or_completed() {
        use refact_buddy_core::operator_types::RunStatus;
        let (_dir, app, runtime) = isolated_app().await;
        for summary in ["", "No changes needed; completed successfully"] {
            let result = buddy_wrap_workflow(
                app.clone(),
                "commit_message",
                "x",
                0,
                |_| summary.into(),
                || async { Ok::<_, String>(42) },
            )
            .await;
            assert_eq!(result, Ok(42));
            drain_side_effects(&app).await;
        }
        let work = runtime
            .list_work_items(None, 10)
            .await
            .unwrap()
            .entries
            .remove(0)
            .value;
        let outcomes = runtime.list_work_outcomes(work.id, None, 10).await.unwrap();
        assert_eq!(outcomes.entries.len(), 2);
        assert!(outcomes
            .entries
            .iter()
            .all(|entry| entry.value.status == RunStatus::Inconclusive));
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn wrapper_publication_failure_does_not_change_callback_results_or_side_effects() {
        let (_dir, app, runtime) = isolated_app().await;
        runtime.shutdown().await;
        for expected in [
            Ok("original value".to_string()),
            Err("original error".to_string()),
        ] {
            let result = buddy_wrap_workflow_report(
                app.clone(),
                "commit_message",
                "x",
                0,
                |_| "Legacy summary".into(),
                typed_scope(),
                |_| typed_report("Report cannot be stored".into()),
                || async { expected.clone() },
            )
            .await;
            assert_eq!(result, expected);
            drain_side_effects(&app).await;
            let buddy = app.buddy.buddy.lock().await;
            let summary = buddy
                .as_ref()
                .unwrap()
                .state
                .workflow_summaries
                .iter()
                .find(|summary| summary.workflow_id == "commit_message")
                .unwrap();
            assert_eq!(
                summary.last_outcome.as_deref(),
                Some(if expected.is_ok() {
                    "success"
                } else {
                    "failed:unknown"
                })
            );
        }
    }

    async fn wait_for_workflow_side_effects() {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // ---- R-5: memo_extraction is housekeeping ----------------------------

    /// Drive the housekeeping wrapper once with a fixed outcome.
    async fn run_memo_extraction(app: &AppState, outcome: Result<usize, String>) {
        let expected = outcome.clone();
        let result = buddy_wrap_workflow(
            app.clone(),
            "memo_extraction",
            "🧠",
            8,
            |memos: &usize| format!("Memory extracted: {memos} memos"),
            || async move { outcome },
        )
        .await;
        assert_eq!(result, expected);
        drain_side_effects(app).await;
    }

    async fn work_items(
        runtime: &super::super::outcome_runtime::OutcomeRuntime,
    ) -> Vec<refact_buddy_core::operator_types::WorkItem> {
        runtime
            .list_work_items(None, 100)
            .await
            .unwrap()
            .entries
            .into_iter()
            .map(|entry| entry.value)
            .collect()
    }

    /// Items the user would actually see: anything not already terminal.
    async fn visible_work_items(
        runtime: &super::super::outcome_runtime::OutcomeRuntime,
    ) -> Vec<refact_buddy_core::operator_types::WorkItem> {
        use refact_buddy_core::operator_types::WorkItemState;
        work_items(runtime)
            .await
            .into_iter()
            .filter(|item| {
                !matches!(
                    item.state,
                    WorkItemState::Cancelled | WorkItemState::Delivered
                )
            })
            .collect()
    }

    fn rate_limit_error() -> String {
        "OpenAI API error (429): rate_limit_exceeded".to_string()
    }

    #[tokio::test]
    async fn housekeeping_run_creates_no_work_item() {
        let (_dir, app, runtime) = isolated_app().await;
        for memos in [7usize, 6, 8] {
            run_memo_extraction(&app, Ok(memos)).await;
        }

        // Zero Desk rows, zero artifact revisions.
        assert!(
            work_items(&runtime).await.is_empty(),
            "a successful housekeeping run published a work item"
        );
        let subject = super::super::outcome_runtime::housekeeping_subject(
            &runtime.project(),
            "memo_extraction",
        )
        .unwrap();
        assert!(runtime
            .list_artifact_revisions(subject.artifact_id(), None, 100)
            .await
            .unwrap()
            .entries
            .is_empty());

        // Accountability is preserved: every run still recorded a RunOutcome.
        assert_eq!(
            recorded_run_outcomes(&runtime).await,
            3,
            "housekeeping outcomes were not recorded"
        );
        runtime.shutdown().await;
    }

    /// Count of persisted `RunOutcome` rows. Housekeeping outcomes have no work
    /// item to list them under, which is the point, so count them at the store.
    async fn recorded_run_outcomes(runtime: &super::super::outcome_runtime::OutcomeRuntime) -> u64 {
        runtime
            .with_store(|store| store.diagnostic_summary())
            .await
            .unwrap()
            .tables
            .into_iter()
            .find(|table| table.table == "run_outcomes")
            .map(|table| table.sampled_rows)
            .expect("run_outcomes is always sampled")
    }

    #[tokio::test]
    async fn housekeeping_failure_creates_no_work_item() {
        let (_dir, app, runtime) = isolated_app().await;
        // Below the block threshold: unlucky, not blocked. Still invisible.
        for _ in 0..(refact_buddy_core::workflows::HOUSEKEEPING_BLOCK_THRESHOLD - 1) {
            run_memo_extraction(&app, Err("model unavailable".into())).await;
        }
        assert!(
            work_items(&runtime).await.is_empty(),
            "an ordinary housekeeping failure published a work item"
        );
        let subject = super::super::outcome_runtime::housekeeping_subject(
            &runtime.project(),
            "memo_extraction",
        )
        .unwrap();
        assert!(runtime
            .list_artifact_revisions(subject.artifact_id(), None, 100)
            .await
            .unwrap()
            .entries
            .is_empty());
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn blocked_housekeeping_creates_exactly_one_item() {
        use refact_buddy_core::operator_types::WorkItemState;
        let (_dir, app, runtime) = isolated_app().await;
        for _ in 0..refact_buddy_core::workflows::HOUSEKEEPING_BLOCK_THRESHOLD {
            run_memo_extraction(&app, Err(rate_limit_error())).await;
        }
        let items = visible_work_items(&runtime).await;
        assert_eq!(
            items.len(),
            1,
            "blocked housekeeping is not exactly one item"
        );
        assert_eq!(items[0].state, WorkItemState::Blocked);
        assert!(items[0].title.contains("memo extraction"));
        let artifact_id = items[0].current_artifact_id.clone().unwrap();
        let body = runtime
            .get_artifact_revision(artifact_id.clone(), None)
            .await
            .unwrap()
            .unwrap()
            .value
            .body;
        // It says what stopped and what the user can do.
        assert!(body.contains("rate limit"));
        assert!(body.contains("What you can do"));

        // Repeat: still one item, refreshed rather than duplicated.
        for _ in 0..5 {
            run_memo_extraction(&app, Err(rate_limit_error())).await;
        }
        let repeated = visible_work_items(&runtime).await;
        assert_eq!(repeated.len(), 1, "a repeated block duplicated the item");
        assert_eq!(repeated[0].id, items[0].id);
        assert_eq!(
            runtime
                .list_artifact_revisions(artifact_id, None, 100)
                .await
                .unwrap()
                .entries
                .len(),
            1,
            "a repeated block appended an artifact revision instead of folding"
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn blocked_item_clears_on_recovery() {
        use refact_buddy_core::operator_types::WorkItemState;
        let (_dir, app, runtime) = isolated_app().await;
        for _ in 0..refact_buddy_core::workflows::HOUSEKEEPING_BLOCK_THRESHOLD {
            run_memo_extraction(&app, Err(rate_limit_error())).await;
        }
        let blocked = visible_work_items(&runtime).await;
        assert_eq!(blocked.len(), 1);
        let id = blocked[0].id.clone();

        run_memo_extraction(&app, Ok(4)).await;

        assert!(
            visible_work_items(&runtime).await.is_empty(),
            "the blocked item did not clear once housekeeping resumed succeeding"
        );
        // Cleared means terminal, not deleted: the row and its artifact remain.
        let retired = runtime.get_work_item(id).await.unwrap().unwrap().value;
        assert_eq!(retired.state, WorkItemState::Cancelled);
        assert!(runtime
            .get_artifact_revision(retired.current_artifact_id.unwrap(), Some(1))
            .await
            .unwrap()
            .is_some());
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn no_double_report_with_provider_rate_limit_fact() {
        use refact_buddy_core::operator_types::WorkItemState;
        let (_dir, app, runtime) = isolated_app().await;
        // R-4's aggregated provider-rate-limit item, already on the Desk.
        seed_provider_rate_limit_aggregate(&runtime).await;
        assert_eq!(visible_work_items(&runtime).await.len(), 1);

        for _ in 0..(refact_buddy_core::workflows::HOUSEKEEPING_BLOCK_THRESHOLD + 3) {
            run_memo_extraction(&app, Err(rate_limit_error())).await;
        }

        let items = visible_work_items(&runtime).await;
        assert_eq!(
            items.len(),
            1,
            "housekeeping raised a second item beside R-4's aggregate: {items:#?}"
        );
        assert_eq!(items[0].kind, "legacy_opportunity");
        assert_eq!(items[0].state, WorkItemState::NeedsAttention);
        runtime.shutdown().await;
    }

    /// Seed R-4's aggregated provider-rate-limit work item exactly as the
    /// opportunity migration produces it.
    async fn seed_provider_rate_limit_aggregate(
        runtime: &super::super::outcome_runtime::OutcomeRuntime,
    ) {
        let worker = runtime.worker();
        runtime
            .with_store(move |store| {
                let producer_id = format!(
                    "producer:{}",
                    crate::buddy::work_items::stable_hash(&["provider:rate_limited:global"])
                );
                let opportunity: crate::buddy::types::BuddyOpportunity =
                    serde_json::from_value(serde_json::json!({
                        "id": "provider-rate-limit", "kind": "config_drift",
                        "summary": "Rate limit reached", "priority": "critical", "confidence": 0.98,
                        "fact_keys": ["provider:rate_limited:global"],
                        "cooldown_key": "provider:rate_limited:global", "cooldown_secs": 0,
                        "status": "new", "proposed_actions": [],
                        "humor": null, "humor_allowed": false, "related": {},
                        "created_at": "2026-01-01T00:00:00Z",
                        "expires_at": "2026-01-02T00:00:00Z", "resolved_at": null
                    }))
                    .unwrap();
                crate::buddy::legacy_work::migrate_batch(store, &worker, &[opportunity], &[])?;
                let subject =
                    crate::buddy::outcome_runtime::provider_rate_limit_subject(store.identity())?;
                assert_eq!(subject.resource_id, producer_id);
                let seeded = store
                    .get_work_item(&subject.work_item_id())?
                    .expect("R-4 aggregate fixture must exist under the expected subject");
                let mut live = seeded.value.clone();
                live.state = refact_buddy_core::operator_types::WorkItemState::NeedsAttention;
                store.upsert_work_item(&live, Some(seeded.revision))?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// The subject key of a user-facing workflow is a stored identity: changing
    /// it re-keys existing work items and orphans the user's history. Asserted
    /// byte-for-byte against a golden fixture rather than recomputed.
    #[tokio::test]
    async fn user_facing_workflows_are_unchanged() {
        use refact_buddy_core::operator_types::WorkItemState;
        const GOLDEN_COMMIT_MESSAGE_WORK_ID: &str =
            "work:f180476eb72011ab024fadf503565bfd2e52ffc0184924280ef66953fb91ec69";
        const GOLDEN_COMMIT_MESSAGE_ARTIFACT_ID: &str =
            "artifact:f180476eb72011ab024fadf503565bfd2e52ffc0184924280ef66953fb91ec69";
        let (_dir, app, runtime) = isolated_app().await;
        assert_eq!(runtime.project().project_id, "workflow-test");
        assert_eq!(
            workflow_visibility("commit_message"),
            WorkflowVisibility::UserFacing
        );

        buddy_wrap_workflow(
            app.clone(),
            "commit_message",
            "x",
            0,
            |_| "Subject line".to_string(),
            || async { Ok::<_, String>(()) },
        )
        .await
        .unwrap();
        drain_side_effects(&app).await;

        let items = work_items(&runtime).await;
        assert_eq!(items.len(), 1, "a user-facing workflow stopped publishing");
        assert_eq!(items[0].id, GOLDEN_COMMIT_MESSAGE_WORK_ID);
        assert_eq!(
            items[0].current_artifact_id.as_deref(),
            Some(GOLDEN_COMMIT_MESSAGE_ARTIFACT_ID)
        );
        assert_eq!(items[0].subject.resource_kind, "workflow");
        assert_eq!(items[0].subject.resource_id, "commit_message");
        assert_eq!(items[0].subject.version, "workflow:commit_message");
        assert_eq!(
            items[0].scope.as_deref(),
            Some("Latest run of Commit message generation")
        );
        assert_ne!(items[0].state, WorkItemState::Cancelled);
        runtime.shutdown().await;
    }

    /// The live store holds one collapsed `memo_extraction` row carrying 1,698
    /// artifact revisions and its outcomes. Upgrading retires it; it must not
    /// delete it, its artifacts or its outcomes.
    #[tokio::test]
    async fn existing_collapsed_row_is_retired_not_deleted() {
        use refact_buddy_core::operator_types::{RunStatus, WorkItemState};
        use super::super::outcome_runtime::{
            collapsed_workflow_subject, OutcomeRuntime, WorkflowOutcomeStart, WorkflowTerminal,
        };
        let dir = tempfile::tempdir().unwrap();
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "collapsed-row".into(),
            canonical_root: dir.path().canonicalize().unwrap().to_string_lossy().into(),
        };
        let worker = refact_buddy_core::operator_types::WorkerIdentity {
            worker_id: "worker".into(),
            engine_instance_id: "engine".into(),
            epoch: 1,
        };

        // Seed the pre-fix shape directly: one subject, many differing bodies,
        // exactly as the constant fingerprint produced in the user's store.
        let subject = collapsed_workflow_subject(&project, "memo_extraction").unwrap();
        let collapsed_id = subject.work_item_id();
        let store =
            crate::buddy::operator_store::OperatorStore::open(dir.path(), &project).unwrap();
        let seeder = OutcomeRuntime::from_store(store, project.clone(), worker.clone());
        const SEEDED_REVISIONS: usize = 12;
        for index in 0..SEEDED_REVISIONS {
            let mut start = WorkflowOutcomeStart {
                run_id: format!("seeded-run-{index}"),
                work_id: collapsed_id.clone(),
                parent_run_id: None,
                project: project.clone(),
                worker: worker.clone(),
                origin: "buddy_workflow".into(),
                workflow: "memo_extraction".into(),
                input_fingerprint: "workflow:memo_extraction".into(),
                coverage: "workflow invocation and terminal result".into(),
                started_at: "2026-09-05T00:00:00Z".into(),
            };
            start.work_id = collapsed_id.clone();
            seeder
                .publish_report(
                    start,
                    "Memo Extraction didn't finish".into(),
                    format!("Memory extracted: {index} memos"),
                    vec![],
                    WorkflowTerminal::Failed {
                        failure_class: Some("provider_rate_limit".into()),
                    },
                )
                .await
                .unwrap();
        }
        let before = seeder
            .get_work_item(collapsed_id.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before.value.state, WorkItemState::Blocked);
        let artifact_id = before.value.current_artifact_id.clone().unwrap();
        assert_eq!(
            before.value.current_artifact_revision,
            Some(SEEDED_REVISIONS as u32)
        );
        seeder.shutdown().await;

        // Upgrade: reopening the store runs the retirement.
        let upgraded = OutcomeRuntime::open(dir.path(), project.clone(), worker.clone()).unwrap();
        let retired = upgraded
            .get_work_item(collapsed_id.clone())
            .await
            .unwrap()
            .expect("the collapsed row must still exist");
        assert_eq!(
            retired.value.state,
            WorkItemState::Cancelled,
            "the collapsed row was not retired"
        );

        // Nothing was deleted. Every revision and every outcome still reads.
        let revisions = upgraded
            .list_artifact_revisions(artifact_id.clone(), None, 200)
            .await
            .unwrap();
        assert_eq!(revisions.entries.len(), SEEDED_REVISIONS);
        for index in 0..SEEDED_REVISIONS {
            let revision = upgraded
                .get_artifact_revision(artifact_id.clone(), Some(index as u32 + 1))
                .await
                .unwrap()
                .expect("every seeded revision must stay readable");
            assert!(revision.value.body.contains(&format!("{index} memos")));
        }
        let outcomes = upgraded
            .list_work_outcomes(collapsed_id.clone(), None, 200)
            .await
            .unwrap();
        assert_eq!(outcomes.entries.len(), SEEDED_REVISIONS);
        assert!(outcomes
            .entries
            .iter()
            .all(|entry| entry.value.status == RunStatus::Failed));
        for index in 0..SEEDED_REVISIONS {
            assert!(upgraded
                .get_run_outcome(format!("seeded-run-{index}"))
                .await
                .unwrap()
                .is_some());
        }

        // Idempotent: reopening again does not churn the row.
        upgraded.shutdown().await;
        let reopened = OutcomeRuntime::open(dir.path(), project, worker).unwrap();
        let again = reopened.get_work_item(collapsed_id).await.unwrap().unwrap();
        assert_eq!(again.value.state, WorkItemState::Cancelled);
        assert_eq!(again.revision, retired.revision);
        reopened.shutdown().await;
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
}
