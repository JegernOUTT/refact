use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serial_test::serial;
use tokio::sync::Mutex;

use crate::agents::seal::is_sealed;
use crate::agents::types::{AgentCompletion, BackgroundAgent, BgAgentKind, CreateAgentRequest};
use crate::app_state::AppState;
use crate::call_validation::{ChatMessage, ChatUsage};
use crate::chat::goal_monitor::{
    dispatch_goal_nudge, GoalNudgeOutcome, GoalNudgeSkip, GoalNudgeTrigger,
};
use crate::chat::goal_verifier::{
    apply_goal_verdict_for_chat, run_goal_verification, verify_goal_before_completion,
    GoalCompletionGateOutcome, GoalVerdict, GoalVerificationApplyOutcome, GoalVerificationFork,
    GoalVerifierReply,
};
use crate::chat::types::{ChatSession, GoalBudget, GoalStatus, PushMode, SessionState};
use crate::exec::{ExecMode, ExecOwnerMeta, ExecProcessId, ExecSpawnRequest};

struct Fixture {
    _workspace: tempfile::TempDir,
    app: AppState,
    agent: BackgroundAgent,
    session: Arc<Mutex<ChatSession>>,
}

async fn fixture() -> Fixture {
    let workspace = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().to_path_buf()];
    let app = AppState::from_gcx(gcx).await;
    let chat_id = format!("seal-wake-{}", uuid::Uuid::new_v4());
    let mut session = ChatSession::new(chat_id.clone());
    session.add_message(ChatMessage::new("user".into(), "ship the fix".into()));
    session.add_message(ChatMessage::new("assistant".into(), "done".into()));
    session
        .queue_processor_running
        .store(true, Ordering::SeqCst);
    let session = Arc::new(Mutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert(chat_id.clone(), session.clone());
    let (agent, _, _) = app
        .agents
        .create(CreateAgentRequest {
            parent_chat_id: "seal-wake-parent".into(),
            parent_root_chat_id: Some("seal-wake-parent".into()),
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "test".into(),
            title: "seal wake".into(),
            prompt: "test".into(),
            target_files: vec![],
            model: "test".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        })
        .await
        .unwrap();
    let agent = app
        .agents
        .mark_running(&agent.agent_id, chat_id)
        .await
        .unwrap();
    Fixture {
        _workspace: workspace,
        app,
        agent,
        session,
    }
}

async fn complete(fixture: &Fixture) {
    fixture
        .app
        .agents
        .mark_completed(
            &fixture.agent.agent_id,
            AgentCompletion {
                result_summary: "done".into(),
                edited_files: vec![],
                diff_summary: None,
                conflict_summary: None,
                child_chat_id: fixture.agent.child_chat_id.clone(),
                deliverable_status: None,
                handoff: None,
            },
        )
        .await
        .unwrap();
    assert!(
        is_sealed(
            &fixture.app,
            fixture.agent.child_chat_id.as_deref().unwrap()
        )
        .await
    );
}

async fn active_goal(session: &Arc<Mutex<ChatSession>>) {
    let mut session = session.lock().await;
    session.install_goal("agent", "ship the fix", true, GoalBudget::default());
    session.last_activity = Instant::now() - Duration::from_secs(600);
}

async fn snapshot(session: &Arc<Mutex<ChatSession>>) -> (usize, usize) {
    let session = session.lock().await;
    (
        session.messages.len(),
        session
            .messages
            .iter()
            .filter(|message| message.role == "assistant")
            .count(),
    )
}

async fn no_wake(session: &Arc<Mutex<ChatSession>>, before: (usize, usize)) {
    assert_eq!(
        snapshot(session).await,
        before,
        "sealed wake source must not append or generate"
    );
    let session = session.lock().await;
    assert_ne!(session.runtime.state, SessionState::Generating);
    assert!(session.delivery_wake_sources.is_empty());
    assert!(session
        .pending_deliveries
        .iter()
        .all(|delivery| !delivery.wake));
    assert!(session.command_queue.is_empty());
}

#[serial(test_runner, permission_decision_hooks, runtime_settings)]
#[tokio::test]
async fn goal_nudge_skips_sealed_chat() {
    let fixture = fixture().await;
    active_goal(&fixture.session).await;
    complete(&fixture).await;
    let before = snapshot(&fixture.session).await;
    let outcome = dispatch_goal_nudge(
        fixture.app.clone(),
        fixture.session.clone(),
        GoalNudgeTrigger::Monitor,
    )
    .await;
    no_wake(&fixture.session, before).await;
    assert_eq!(outcome, GoalNudgeOutcome::Skipped(GoalNudgeSkip::Sealed));
    assert!(
        crate::chat::goal_monitor::handle_goal_turn_end(
            fixture.app.clone(),
            fixture.session.clone()
        )
        .await
    );
    no_wake(&fixture.session, before).await;
    assert_eq!(
        fixture.session.lock().await.goal_status,
        Some(GoalStatus::Active)
    );
}

fn gaps() -> GoalVerifierReply {
    GoalVerifierReply {
        verdict: GoalVerdict::Unmet(vec!["tests missing".into()]),
        verifier_reply: "GOAL: UNMET".into(),
        tokens: 1,
        criteria_verdicts: vec![],
    }
}

async fn apply_gaps(
    app: &AppState,
    session: &Arc<Mutex<ChatSession>>,
) -> GoalVerificationApplyOutcome {
    let fork = GoalVerificationFork::capture(&*session.lock().await);
    apply_goal_verdict_for_chat(
        app,
        session,
        "agent_finish",
        gaps(),
        fork,
        PushMode::Append,
        None,
    )
    .await
}

#[serial(test_runner, permission_decision_hooks, runtime_settings)]
#[tokio::test]
async fn verifier_gaps_do_not_rearm_sealed_chat() {
    let fixture = fixture().await;
    active_goal(&fixture.session).await;
    let fork = {
        let mut session = fixture.session.lock().await;
        session.goal_set_status(GoalStatus::Verifying);
        GoalVerificationFork::capture(&session)
    };
    complete(&fixture).await;
    let before = snapshot(&fixture.session).await;
    assert_eq!(
        apply_goal_verdict_for_chat(
            &fixture.app,
            &fixture.session,
            "agent_finish",
            gaps(),
            fork,
            PushMode::Append,
            None
        )
        .await,
        GoalVerificationApplyOutcome::Sealed
    );
    assert_eq!(
        verify_goal_before_completion(fixture.app.clone(), fixture.session.clone(), "agent_finish")
            .await,
        GoalCompletionGateOutcome::Passthrough
    );
    assert!(
        run_goal_verification(fixture.app.clone(), fixture.session.clone())
            .await
            .is_err()
    );
    no_wake(&fixture.session, before).await;
    assert_eq!(
        fixture.session.lock().await.goal_status,
        Some(GoalStatus::Verifying)
    );

    let (_workspace, app, task_id, chat_id, session, _planner) =
        crate::chat::task_agent_monitor::tests::setup_monitor_case(
            "doing",
            SessionState::Idle,
            Duration::ZERO,
            vec![],
        )
        .await;
    active_goal(&session).await;
    crate::tasks::storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
        board.get_card_mut("T-1").unwrap().column = "done".into();
        Ok(())
    })
    .await
    .unwrap();
    assert!(is_sealed(&app, &chat_id).await);
    let before = snapshot(&session).await;
    assert_eq!(
        apply_gaps(&app, &session).await,
        GoalVerificationApplyOutcome::Sealed
    );
    assert_eq!(
        verify_goal_before_completion(app.clone(), session.clone(), "agent_finish").await,
        GoalCompletionGateOutcome::Passthrough
    );
    assert!(run_goal_verification(app.clone(), session.clone())
        .await
        .is_err());
    no_wake(&session, before).await;
    let report = crate::tasks::types::VerifierReport {
        passed: false,
        command_results: vec![],
        concerns: vec!["tests missing".into()],
        recommendation: "fix-needed".into(),
        classification: crate::tasks::types::VerifierReportClassification::VerificationFailed,
    };
    assert!(crate::chat::verifier::store_verifier_report(
        app.gcx.clone(),
        &task_id,
        "T-1",
        None,
        report,
        vec![]
    )
    .await
    .unwrap());
    no_wake(&session, before).await;
}

async fn process(app: &AppState, chat_id: &str, mode: ExecMode) -> ExecProcessId {
    let command = if cfg!(windows) {
        "Start-Sleep -Seconds 900"
    } else {
        "sleep 900"
    };
    app.gcx
        .exec_registry
        .spawn(
            ExecSpawnRequest::new(mode.clone(), command)
                .with_owner(ExecOwnerMeta {
                    chat_id: Some(chat_id.into()),
                    service_name: (mode == ExecMode::Service).then(|| "seal-wake-service".into()),
                    ..Default::default()
                })
                .with_short_description("seal wake test"),
        )
        .await
        .unwrap()
        .snapshot
        .meta
        .process_id
}

#[serial(test_runner, permission_decision_hooks, runtime_settings)]
#[tokio::test]
async fn process_exit_does_not_wake_sealed_chat() {
    for board_chat in [false, true] {
        let fixture = fixture().await;
        let (_workspace, app, task_id, chat_id, session, _planner) =
            crate::chat::task_agent_monitor::tests::setup_monitor_case(
                "doing",
                SessionState::Idle,
                Duration::ZERO,
                vec![],
            )
            .await;
        let (app, chat_id, session) = if board_chat {
            (app, chat_id, session)
        } else {
            (
                fixture.app.clone(),
                fixture.agent.child_chat_id.clone().unwrap(),
                fixture.session.clone(),
            )
        };
        active_goal(&session).await;
        let process_id = process(&app, &chat_id, ExecMode::Background).await;
        let mut completion = app.gcx.exec_registry.subscribe_completion();
        tokio::time::timeout(Duration::from_secs(10), async {
            if board_chat {
                crate::tasks::storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
                    board.get_card_mut("T-1").unwrap().column = "done".into();
                    Ok(())
                })
                .await
                .unwrap();
            } else {
                let cancelled = fixture
                    .app
                    .agents
                    .mark_cancelled(&fixture.agent.agent_id, Some("cancel test".into()))
                    .await
                    .unwrap();
                crate::agents::spawn::emit_background_agent_update(app.clone(), &cancelled).await;
            }
        })
        .await
        .expect("terminal event must stop the process without the 30-second backstop");
        let before = snapshot(&session).await;
        assert!(app
            .gcx
            .exec_registry
            .get(&process_id)
            .await
            .unwrap()
            .status
            .is_terminal());
        let event = tokio::time::timeout(Duration::from_secs(10), completion.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.process_id, process_id);
        assert!(app
            .gcx
            .exec_registry
            .wait(&process_id)
            .await
            .unwrap()
            .status
            .is_terminal());
        crate::chat::notifications::deliver_process_completion(app.clone(), &event)
            .await
            .unwrap();
        let after = snapshot(&session).await;
        assert_eq!(after, (before.0 + 1, before.1));
        no_wake(&session, after).await;
        assert_eq!(
            session.lock().await.messages.last().unwrap().extra["event"]["subkind"],
            "process_completed"
        );

        let late = process(&app, &chat_id, ExecMode::Service).await;
        crate::chat::notifications::handle_process_spawn(
            app.gcx.clone(),
            crate::exec::ProcessSpawnEvent {
                process_id: late.clone(),
                chat_id,
                command_preview: "late process".into(),
                mode: ExecMode::Service,
                tty: false,
                status: crate::exec::ExecStatus::Running,
                started_at_ms: 1,
            },
        )
        .await;
        assert!(app
            .gcx
            .exec_registry
            .wait(&late)
            .await
            .unwrap()
            .status
            .is_terminal());
        no_wake(&session, after).await;
    }
}

#[serial(test_runner, permission_decision_hooks, runtime_settings)]
#[tokio::test]
async fn sibling_notice_not_delivered_to_sealed_chat() {
    let fixture = fixture().await;
    let req = crate::agents::spawn_lifecycle_tests::spawn_request(
        "seal-wake-parent",
        BgAgentKind::Subagent,
    );
    for text in ["▸ sibling started", "✓ sibling finished"] {
        crate::agents::spawn::push_sibling_notice(&fixture.app, "other-agent", &req, text.into())
            .await;
    }
    let deliveries = fixture
        .app
        .agents
        .drain_deliveries(&fixture.agent.agent_id, false)
        .await
        .unwrap();
    assert_eq!(deliveries.len(), 2);
    assert!(deliveries.iter().all(|delivery| delivery.wake));
    complete(&fixture).await;
    let pending_before = fixture
        .app
        .agents
        .pending_deliveries(&fixture.agent.agent_id)
        .await
        .len();
    let before = snapshot(&fixture.session).await;
    for text in ["▸ sibling started", "✓ sibling finished"] {
        crate::agents::spawn::push_sibling_notice(&fixture.app, "other-agent", &req, text.into())
            .await;
    }
    assert_eq!(
        fixture
            .app
            .agents
            .pending_deliveries(&fixture.agent.agent_id)
            .await
            .len(),
        pending_before
    );
    no_wake(&fixture.session, before).await;
}

async fn budget(app: &AppState, session: &Arc<Mutex<ChatSession>>) -> bool {
    crate::chat::inject_token_budget_for_chat(app, session, 100_000, 1, Default::default()).await
}

async fn cutoff(app: &AppState, session: &Arc<Mutex<ChatSession>>) -> bool {
    let thread = session.lock().await.thread.clone();
    crate::chat::maybe_recover_after_length_stop(
        app.gcx.clone(),
        session,
        &thread,
        Some(100_000),
        Default::default(),
    )
    .await
}

async fn length_stop(session: &Arc<Mutex<ChatSession>>) {
    let mut session = session.lock().await;
    session.add_message(ChatMessage {
        role: "assistant".into(),
        finish_reason: Some("length".into()),
        usage: Some(ChatUsage {
            prompt_tokens: 100,
            total_tokens: 100,
            ..Default::default()
        }),
        ..Default::default()
    });
}

#[serial(test_runner, permission_decision_hooks, runtime_settings)]
#[tokio::test]
async fn budget_continuation_skips_sealed_chat() {
    let fixture = fixture().await;
    complete(&fixture).await;
    let before = snapshot(&fixture.session).await;
    assert!(!budget(&fixture.app, &fixture.session).await);
    no_wake(&fixture.session, before).await;
    length_stop(&fixture.session).await;
    let before = snapshot(&fixture.session).await;
    assert!(!cutoff(&fixture.app, &fixture.session).await);
    crate::chat::start_generation(fixture.app.clone(), fixture.session.clone()).await;
    no_wake(&fixture.session, before).await;
}

#[serial(test_runner, permission_decision_hooks, runtime_settings)]
#[tokio::test]
async fn unsealed_sources_request_continuation() {
    let goal = fixture().await;
    active_goal(&goal.session).await;
    assert!(matches!(
        dispatch_goal_nudge(
            goal.app.clone(),
            goal.session.clone(),
            GoalNudgeTrigger::Monitor
        )
        .await,
        GoalNudgeOutcome::Nudged(_)
    ));
    assert!(!goal.session.lock().await.delivery_wake_sources.is_empty());

    let verifier = fixture().await;
    active_goal(&verifier.session).await;
    assert_eq!(
        apply_gaps(&verifier.app, &verifier.session).await,
        GoalVerificationApplyOutcome::Rearmed
    );
    assert!(!verifier
        .session
        .lock()
        .await
        .delivery_wake_sources
        .is_empty());

    let running = fixture().await;
    let chat_id = running.agent.child_chat_id.as_deref().unwrap();
    let process_id = process(&running.app, chat_id, ExecMode::Background).await;
    let mut completion = running.app.gcx.exec_registry.subscribe_completion();
    crate::chat::notifications::stop_sealed_chat_processes(&running.app).await;
    assert!(!running
        .app
        .gcx
        .exec_registry
        .get(&process_id)
        .await
        .unwrap()
        .status
        .is_terminal());
    running
        .app
        .gcx
        .exec_registry
        .kill(&process_id)
        .await
        .unwrap();
    let event = completion.recv().await.unwrap();
    crate::chat::notifications::deliver_process_completion(running.app.clone(), &event)
        .await
        .unwrap();
    let deliveries = running
        .app
        .agents
        .drain_deliveries(&running.agent.agent_id, false)
        .await
        .unwrap();
    assert_eq!(deliveries.len(), 1);
    assert!(deliveries[0].wake);

    let continuation = fixture().await;
    assert!(budget(&continuation.app, &continuation.session).await);
    length_stop(&continuation.session).await;
    assert!(cutoff(&continuation.app, &continuation.session).await);
    assert!(continuation
        .session
        .lock()
        .await
        .messages
        .last()
        .unwrap()
        .content
        .content_text_only()
        .contains("cut off"));
}
