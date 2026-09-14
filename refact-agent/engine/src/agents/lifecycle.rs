use crate::agents::types::{BackgroundAgent, BgAgentStatus};
use crate::app_state::AppState;

/// Close a direct child: stop its runner, release its execution slot and any
/// residency, and retire the durable record as `closed` so it can be resumed
/// later. Idempotent, and never rewrites an already finished outcome.
pub async fn close_agent(
    app: AppState,
    parent_chat_id: &str,
    agent_id: &str,
    expected_run_generation: Option<u64>,
    reason: Option<String>,
) -> Result<(BackgroundAgent, bool), String> {
    let record = app.agents.get(parent_chat_id, agent_id).await?;
    if let Some(generation) = expected_run_generation {
        if generation != record.run_generation {
            return Err(
                "agent moved to another generation; inspect its current state before closing"
                    .into(),
            );
        }
    }
    if record.status == BgAgentStatus::Closed {
        return Ok((record, true));
    }
    let (closed, already_closed) = app
        .agents
        .close(parent_chat_id, agent_id, expected_run_generation, reason)
        .await?;
    if let Some(child_chat_id) = closed.child_chat_id.as_deref() {
        app.agent_executions.release(child_chat_id);
    }
    crate::agents::spawn::emit_background_agent_update(app.clone(), &closed).await;
    Ok((closed, already_closed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::types::{BgAgentKind, CreateAgentRequest};
    use std::sync::Arc;
    use tokio::sync::Mutex as AMutex;

    const PARENT: &str = "lifecycle-parent";

    async fn fixture() -> (tempfile::TempDir, tempfile::TempDir, AppState) {
        let cache = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.path().to_path_buf(),
            config.path().to_path_buf(),
        )
        .await;
        let app = AppState::from_gcx(gcx).await;
        app.chat.sessions.write().await.insert(
            PARENT.to_string(),
            Arc::new(AMutex::new(crate::chat::types::ChatSession::new(
                PARENT.to_string(),
            ))),
        );
        (cache, config, app)
    }

    fn create_request(parent_chat_id: &str) -> CreateAgentRequest {
        CreateAgentRequest {
            parent_chat_id: parent_chat_id.to_string(),
            parent_root_chat_id: Some(parent_chat_id.to_string()),
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "test".to_string(),
            title: "Closable".to_string(),
            prompt: "Do a thing".to_string(),
            target_files: Vec::new(),
            model: "test-model".to_string(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        }
    }

    async fn running_agent(app: &AppState, child_chat_id: &str) -> BackgroundAgent {
        let (record, _, _) = app.agents.create(create_request(PARENT)).await.unwrap();
        app.agents
            .mark_running(&record.agent_id, child_chat_id.to_string())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn closing_releases_the_slot_stops_the_runner_and_is_idempotent() {
        let (_cache, _config, app) = fixture().await;
        let record = running_agent(&app, "child-close").await;
        let abort = app.agents.abort_flag(&record.agent_id).await.unwrap();
        let permit = app
            .work_scheduler
            .reserve(refact_work_core::AdmissionRequest::new(
                refact_work_core::WorkKey::background_agent(&record.agent_id).run(1),
                refact_work_core::WorkScope::single("project"),
            ))
            .unwrap()
            .acquire()
            .await
            .unwrap();
        app.agent_executions.install("child-close", permit).unwrap();
        assert_eq!(app.work_scheduler.snapshot().active, 1);

        let (closed, already) = close_agent(
            app.clone(),
            PARENT,
            &record.agent_id,
            Some(record.run_generation),
            Some("parked for now".into()),
        )
        .await
        .unwrap();

        assert!(!already);
        assert_eq!(closed.status, BgAgentStatus::Closed);
        assert!(closed.status.is_terminal());
        assert_eq!(closed.close_reason.as_deref(), Some("parked for now"));
        assert!(closed.finished_at.is_some());
        assert!(abort.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!app.agents.has_runtime(&record.agent_id).await);
        assert!(app.agent_executions.get("child-close").is_none());
        assert_eq!(app.work_scheduler.snapshot().active, 0);
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);

        let (again, already) = close_agent(
            app.clone(),
            PARENT,
            &record.agent_id,
            None,
            Some("second try".into()),
        )
        .await
        .unwrap();
        assert!(already);
        assert_eq!(again.change_seq, closed.change_seq);
        assert_eq!(again.close_reason.as_deref(), Some("parked for now"));

        // A close after a close settles once; a later cancel cannot rewrite it.
        let cancelled = app
            .agents
            .cancel(PARENT, &record.agent_id, Some("too late".into()))
            .await
            .unwrap();
        assert_eq!(cancelled.status, BgAgentStatus::Closed);
        assert_eq!(cancelled.change_seq, closed.change_seq);
    }

    #[tokio::test]
    async fn closing_refuses_a_stale_generation_a_foreign_parent_and_a_finished_child() {
        let (_cache, _config, app) = fixture().await;
        let record = running_agent(&app, "child-stale").await;

        let stale = close_agent(
            app.clone(),
            PARENT,
            &record.agent_id,
            Some(record.run_generation + 1),
            None,
        )
        .await
        .unwrap_err();
        assert!(stale.contains("another generation"), "{stale}");
        assert_eq!(
            app.agents.get_any(&record.agent_id).await.unwrap().status,
            BgAgentStatus::Running
        );

        let foreign = close_agent(app.clone(), "other-parent", &record.agent_id, None, None)
            .await
            .unwrap_err();
        assert_eq!(foreign, "agent not found");

        let done = running_agent(&app, "child-done").await;
        app.agents
            .mark_completed(
                &done.agent_id,
                crate::agents::types::AgentCompletion {
                    result_summary: "already answered".into(),
                    edited_files: Vec::new(),
                    diff_summary: None,
                    conflict_summary: None,
                    child_chat_id: None,
                    deliverable_status: None,
                    handoff: None,
                },
            )
            .await
            .unwrap();
        let finished = close_agent(app.clone(), PARENT, &done.agent_id, None, None)
            .await
            .unwrap_err();
        assert!(
            finished.contains("already finished as completed"),
            "{finished}"
        );
        let unchanged = app.agents.get_any(&done.agent_id).await.unwrap();
        assert_eq!(unchanged.status, BgAgentStatus::Completed);
        assert_eq!(
            unchanged.result_summary.as_deref(),
            Some("already answered")
        );
    }

    #[tokio::test]
    async fn a_closed_child_without_saved_configuration_stays_deny_all() {
        let (_cache, _config, app) = fixture().await;
        let record = running_agent(&app, "child-no-config").await;
        let (closed, _) = close_agent(app.clone(), PARENT, &record.agent_id, None, None)
            .await
            .unwrap();

        assert!(app
            .agent_executions
            .saved_config(&record.agent_id)
            .is_none());
        assert!(
            crate::agents::spawn::recover_run_config(&app, PARENT, &record.agent_id)
                .await
                .is_err()
        );
        let refused = crate::agents::spawn::resume_agent(
            app.clone(),
            PARENT,
            &record.agent_id,
            None,
            closed.change_seq,
            None,
            false,
        )
        .await
        .unwrap_err();
        assert!(!refused.is_empty());

        let after = app.agents.get_any(&record.agent_id).await.unwrap();
        assert_eq!(after.status, BgAgentStatus::Closed);
        assert_eq!(after.run_generation, closed.run_generation);
        assert!(!app.agents.has_runtime(&record.agent_id).await);
    }
}
