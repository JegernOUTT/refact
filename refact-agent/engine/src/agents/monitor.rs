use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use crate::agents::progress_monitor::{
    ActivityClass, ProgressThresholds, RunActivity, classify, observe_evidence, retain_agents,
    turn_count, turns_from_messages,
};
use crate::agents::types::{BackgroundAgent, BgAgentStatus};
use crate::app_state::AppState;
use crate::file_activity::WriteObservation;

const MONITOR_INTERVAL: Duration = Duration::from_secs(60);
const STUCK_AFTER: TimeDelta = TimeDelta::minutes(20);

pub async fn run_background_agent_monitor(app: AppState, shutdown: Arc<AtomicBool>) {
    tracing::info!("Starting background agent monitor");
    let detector = crate::coordination::deadlock::DeadlockDetector::start(app.clone());
    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        if let Err(error) = monitor_once(app.clone()).await {
            tracing::warn!("background agent monitor error: {}", error);
        }
        tokio::select! {
            _ = tokio::time::sleep(MONITOR_INTERVAL) => {}
            _ = wait_for_shutdown(shutdown.clone()) => break,
        }
    }
    if let Some(detector) = detector {
        detector.shutdown().await;
    }
    app.agent_executions.begin_shutdown();
    crate::tasks::admission::shutdown(&app).await;
    app.agent_executions.shutdown().await;
}

pub(crate) async fn monitor_once(app: AppState) -> Result<(), String> {
    crate::tasks::admission::synchronize_all(&app).await;
    let now = Utc::now();
    let thresholds = ProgressThresholds::from_settings(
        &crate::file_activity::settings(),
        STUCK_AFTER.to_std().unwrap_or_default(),
    );
    let writes = crate::file_activity::observations(None).writes;
    let records = app.agents.list_all().await;
    let live: BTreeSet<String> = records
        .iter()
        .map(|record| record.agent_id.clone())
        .collect();
    for record in records {
        if matches!(
            record.status,
            BgAgentStatus::Running | BgAgentStatus::WaitingForApproval
        ) {
            let activity = observe_run(&app, &record, now, thresholds, &writes).await;
            let class = classify(&activity, thresholds);
            if let Some(note) = surfaced_note(&record, class) {
                let updated = app
                    .agents
                    .update_progress(&record.agent_id, note, record.step_count)
                    .await?;
                crate::agents::spawn::emit_background_agent_update(app.clone(), &updated).await;
            }
        }

        if record.status == BgAgentStatus::Running
            && !app.agents.has_runtime(&record.agent_id).await
            && child_session_missing(&app, record.child_chat_id.as_deref()).await
        {
            let updated = app
                .agents
                .mark_interrupted(
                    &record.agent_id,
                    "Engine restarted; child session lost".to_string(),
                )
                .await?;
            crate::agents::spawn::emit_background_agent_update(app.clone(), &updated).await;
            crate::agents::push::push_completion_to_parent(app.clone(), &updated).await?;
        }
    }

    for record in app
        .agents
        .list_with_completion_message_id(&["pending", "deferred"])
        .await
    {
        if crate::agents::push::should_retry_completion_push(&record) {
            crate::agents::push::push_completion_to_parent(app.clone(), &record).await?;
        }
    }

    retain_agents(&live);

    Ok(())
}

/// Report a classification once, not once per tick: rewriting an unchanged
/// note would keep resetting the record's activity stamp.
fn surfaced_note(record: &BackgroundAgent, class: ActivityClass) -> Option<String> {
    let note = class.progress_note()?;
    (record.progress.as_deref() != Some(note.as_str())).then_some(note)
}

async fn observe_run(
    app: &AppState,
    record: &BackgroundAgent,
    now: DateTime<Utc>,
    thresholds: ProgressThresholds,
    writes: &[WriteObservation],
) -> RunActivity {
    let mut activity = RunActivity {
        execution: record
            .child_chat_id
            .as_deref()
            .and_then(|child| app.agent_executions.get(child))
            .map(|execution| execution.state()),
        status: record.status,
        tool_call_in_flight: record.current_tool.is_some(),
        quiet_for: now
            .signed_duration_since(record.last_update_at)
            .to_std()
            .unwrap_or_default(),
        ..Default::default()
    };
    let Some(child_chat_id) = record.child_chat_id.as_deref() else {
        return activity;
    };
    let session = {
        let sessions = app.chat.sessions.read().await;
        sessions.get(child_chat_id).cloned()
    };
    let Some(session) = session else {
        return activity;
    };
    let (turns, turns_total) = {
        let session = session.lock().await;
        activity.session_state = session.runtime.state;
        activity.session_error = session.runtime.error.clone();
        activity.goal_waiting = session.goal_wait().is_some();
        (
            turns_from_messages(&session.messages, thresholds.window_turns),
            turn_count(&session.messages),
        )
    };
    activity.tool_call_in_flight = activity.tool_call_in_flight
        || activity.session_state == refact_chat_api::SessionState::ExecutingTools;
    activity.turns = turns;
    activity.turns_since_evidence_changed = observe_evidence(
        &record.agent_id,
        material_evidence(child_chat_id, writes),
        turns_total,
    );
    activity
}

/// What a run has materially produced: the current content of everything it
/// wrote. Nothing written means nothing to compare, which yields no verdict.
fn material_evidence(child_chat_id: &str, writes: &[WriteObservation]) -> Option<String> {
    let mut latest: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
    for write in writes
        .iter()
        .filter(|write| write.actor.as_deref() == Some(child_chat_id))
    {
        if let Some(digest) = write.digest_after.as_deref() {
            latest.insert(write.key.as_str(), digest);
        }
    }
    if latest.is_empty() {
        return None;
    }
    let joined = latest
        .into_iter()
        .map(|(key, digest)| format!("{key}:{digest}"))
        .collect::<Vec<_>>()
        .join("\n");
    Some(crate::file_activity::digest_of(&joined))
}

async fn child_session_missing(app: &AppState, child_chat_id: Option<&str>) -> bool {
    let Some(child_chat_id) = child_chat_id else {
        return true;
    };
    let sessions = app.chat.sessions.read().await;
    !sessions.contains_key(child_chat_id)
}

async fn wait_for_shutdown(shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::execution::ExecutionState;
    use crate::agents::types::{BgAgentKind, CreateAgentRequest};

    #[tokio::test]
    async fn monitor_marks_old_running_record_with_progress() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let (record, _, _) = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: "parent-monitor".to_string(),
                parent_root_chat_id: None,
                parent_tool_call_id: None,
                kind: BgAgentKind::Subagent,
                config_name: "subagent".to_string(),
                title: "title".to_string(),
                prompt: "prompt".to_string(),
                target_files: vec![],
                model: "model".to_string(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        let running = app
            .agents
            .mark_running(&record.agent_id, "missing-child".to_string())
            .await
            .unwrap();
        app.agents
            .set_last_update_at_for_test(&record.agent_id, Utc::now() - TimeDelta::minutes(25))
            .await
            .unwrap();
        let _ = running;

        monitor_once(app.clone()).await.unwrap();

        let updated = app
            .agents
            .get("parent-monitor", &record.agent_id)
            .await
            .unwrap();
        assert_eq!(updated.status, BgAgentStatus::Running);
        assert!(updated
            .progress
            .as_deref()
            .unwrap_or("")
            .contains("20+ minutes"));
    }

    #[tokio::test]
    async fn monitor_never_flags_parked_or_queued_waiting_as_stalled() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let create = |title: &str| CreateAgentRequest {
            parent_chat_id: "parent-waiting".to_string(),
            parent_root_chat_id: None,
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "subagent".to_string(),
            title: title.to_string(),
            prompt: "prompt".to_string(),
            target_files: vec![],
            model: "model".to_string(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        };
        let (parked, _, _) = app.agents.create(create("parked")).await.unwrap();
        let (queued, _, _) = app.agents.create(create("queued")).await.unwrap();
        app.agents
            .mark_running(&parked.agent_id, "parked-child".to_string())
            .await
            .unwrap();
        let permit = app
            .work_scheduler
            .reserve(refact_work_core::AdmissionRequest::new(
                refact_work_core::WorkKey::background_agent(parked.agent_id.clone()).run(1),
                refact_work_core::WorkScope::new(
                    refact_work_core::ProjectKey::new("project"),
                    refact_work_core::CheckoutKey::new("parent-waiting"),
                ),
            ))
            .unwrap()
            .acquire()
            .await
            .unwrap();
        let execution = app
            .agent_executions
            .install("parked-child", permit)
            .unwrap();
        execution.park().await.unwrap();
        assert_eq!(execution.state(), ExecutionState::Parked);
        for agent_id in [&parked.agent_id, &queued.agent_id] {
            app.agents
                .set_last_update_at_for_test(agent_id, Utc::now() - TimeDelta::minutes(25))
                .await
                .unwrap();
        }
        monitor_once(app.clone()).await.unwrap();
        for agent_id in [&parked.agent_id, &queued.agent_id] {
            let record = app.agents.get("parent-waiting", agent_id).await.unwrap();
            assert!(!record
                .progress
                .as_deref()
                .unwrap_or("")
                .contains("20+ minutes"));
            assert!(!record.status.is_terminal(), "{:?}", record.status);
        }
        assert_eq!(
            app.agents
                .get("parent-waiting", &queued.agent_id)
                .await
                .unwrap()
                .status,
            BgAgentStatus::Queued
        );
        execution.resume().await.unwrap();
        assert_eq!(execution.state(), ExecutionState::Running);
        app.agents
            .set_last_update_at_for_test(&parked.agent_id, Utc::now() - TimeDelta::minutes(25))
            .await
            .unwrap();
        monitor_once(app.clone()).await.unwrap();
        let record = app
            .agents
            .get("parent-waiting", &parked.agent_id)
            .await
            .unwrap();
        assert!(record
            .progress
            .as_deref()
            .unwrap_or("")
            .contains("20+ minutes"));
        app.agent_executions.shutdown().await;
    }

    #[tokio::test]
    async fn monitor_never_flags_approval_waiting_as_stalled() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let (record, _, _) = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: "parent-approval".to_string(),
                parent_root_chat_id: None,
                parent_tool_call_id: None,
                kind: BgAgentKind::Subagent,
                config_name: "subagent".to_string(),
                title: "approval".to_string(),
                prompt: "prompt".to_string(),
                target_files: vec![],
                model: "model".to_string(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        app.agents
            .mark_running(&record.agent_id, "approval-child".to_string())
            .await
            .unwrap();
        let waiting = app
            .agents
            .mark_waiting_for_approval(&record.agent_id)
            .await
            .unwrap();
        assert_eq!(waiting.status, BgAgentStatus::WaitingForApproval);
        app.agents
            .set_last_update_at_for_test(&record.agent_id, Utc::now() - TimeDelta::minutes(25))
            .await
            .unwrap();

        monitor_once(app.clone()).await.unwrap();

        let updated = app
            .agents
            .get("parent-approval", &record.agent_id)
            .await
            .unwrap();
        assert_eq!(updated.status, BgAgentStatus::WaitingForApproval);
        assert!(!updated.status.is_terminal());
        assert!(!updated
            .progress
            .as_deref()
            .unwrap_or("")
            .contains("20+ minutes"));
    }

    async fn running_agent(app: &AppState, parent: &str, child: &str) -> String {
        let (record, _, _) = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: parent.to_string(),
                parent_root_chat_id: None,
                parent_tool_call_id: None,
                kind: BgAgentKind::Subagent,
                config_name: "subagent".to_string(),
                title: "title".to_string(),
                prompt: "prompt".to_string(),
                target_files: vec![],
                model: "model".to_string(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        app.agents
            .mark_running(&record.agent_id, child.to_string())
            .await
            .unwrap();
        record.agent_id
    }

    /// A provider that will not answer is reported as a provider problem, not
    /// as a stalled agent.
    #[tokio::test]
    async fn monitor_reports_a_provider_failure_instead_of_a_stall() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let agent_id = running_agent(&app, "parent-provider", "child-provider").await;
        let mut session = crate::chat::types::ChatSession::new("child-provider".to_string());
        session.set_runtime_state(
            crate::chat::types::SessionState::Error,
            Some("HTTP 503 upstream service unavailable".to_string()),
        );
        app.chat.sessions.write().await.insert(
            "child-provider".to_string(),
            Arc::new(tokio::sync::Mutex::new(session)),
        );
        app.agents
            .set_last_update_at_for_test(&agent_id, Utc::now() - TimeDelta::minutes(25))
            .await
            .unwrap();

        monitor_once(app.clone()).await.unwrap();

        let updated = app.agents.get("parent-provider", &agent_id).await.unwrap();
        let progress = updated.progress.unwrap_or_default();
        assert!(progress.contains("provider"), "{progress}");
        assert!(!progress.contains("20+ minutes"), "{progress}");
        assert!(!progress.contains("no material progress"), "{progress}");
    }

    /// Negative control: one long tool call is one operation, not a stall, no
    /// matter how long it has been quiet.
    #[tokio::test]
    async fn monitor_never_flags_a_run_with_a_tool_call_in_flight() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let agent_id = running_agent(&app, "parent-slow", "child-slow").await;
        app.agents
            .update_activity(&agent_id, None, None, Some(Some("shell".to_string())))
            .await
            .unwrap();
        app.agents
            .set_last_update_at_for_test(&agent_id, Utc::now() - TimeDelta::minutes(45))
            .await
            .unwrap();

        monitor_once(app.clone()).await.unwrap();

        let updated = app.agents.get("parent-slow", &agent_id).await.unwrap();
        assert!(updated.progress.is_none(), "{:?}", updated.progress);
    }

    #[tokio::test]
    async fn monitor_retries_cooled_deferred_completion() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let app = AppState::from_gcx(gcx).await;
        let session = Arc::new(tokio::sync::Mutex::new(
            crate::chat::types::ChatSession::new("parent-monitor-deferred".to_string()),
        ));
        session
            .lock()
            .await
            .queue_processor_running
            .store(true, Ordering::SeqCst);
        app.chat
            .sessions
            .write()
            .await
            .insert("parent-monitor-deferred".to_string(), session.clone());
        let (record, _, _) = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: "parent-monitor-deferred".to_string(),
                parent_root_chat_id: None,
                parent_tool_call_id: None,

                kind: BgAgentKind::Subagent,
                config_name: "subagent".to_string(),
                title: "title".to_string(),
                prompt: "prompt".to_string(),
                target_files: vec![],
                model: "model".to_string(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        app.agents
            .mark_completed(
                &record.agent_id,
                crate::agents::types::AgentCompletion {
                    result_summary: "done".to_string(),
                    edited_files: vec![],
                    diff_summary: None,
                    conflict_summary: None,
                    child_chat_id: Some("child-monitor-deferred".to_string()),
                    deliverable_status: None,
                    handoff: None,
                },
            )
            .await
            .unwrap();
        app.agents
            .set_completion_message_id(&record.agent_id, "deferred".to_string())
            .await
            .unwrap();
        app.agents
            .set_deferred_at_for_test(&record.agent_id, Utc::now() - TimeDelta::seconds(11))
            .await
            .unwrap();

        monitor_once(app.clone()).await.unwrap();

        assert_eq!(session.lock().await.delivery_wake_sources.len(), 1);
        let updated = app
            .agents
            .get("parent-monitor-deferred", &record.agent_id)
            .await
            .unwrap();
        assert_ne!(updated.completion_message_id.as_deref(), Some("deferred"));
        assert!(updated.completion_pushed_at.is_some());
        assert!(updated.deferred_at.is_none());
    }
}
