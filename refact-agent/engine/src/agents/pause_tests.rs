use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{TimeDelta, Utc};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};

use crate::agents::registry::BackgroundAgentRegistry;
use crate::agents::types::{BackgroundAgent, BgAgentKind, BgAgentStatus, CreateAgentRequest};
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::chat::goal_monitor::{
    dispatch_goal_nudge, GoalNudgeOutcome, GoalNudgeSkip, GoalNudgeTrigger,
};
use crate::chat::types::{ChatSession, GoalBudget};
use crate::subchat::{SubchatConfig, ToolsPolicy, TraceParent};
use crate::tools::tool_agent_interact::ToolAgentsOverview;
use crate::tools::tool_background_agents::{ToolAgentList, ToolAgentStatus};
use crate::tools::tool_pause::{ToolPause, ToolResume};
use crate::tools::tools_description::{MatchConfirmDenyResult, Tool};

fn request(parent: &str) -> CreateAgentRequest {
    CreateAgentRequest {
        parent_chat_id: parent.into(),
        parent_root_chat_id: Some("root".into()),
        parent_tool_call_id: None,
        kind: BgAgentKind::Subagent,
        config_name: "subagent".into(),
        title: "Paused child".into(),
        prompt: "gated fake steps".into(),
        target_files: vec![],
        model: "test/model".into(),
        model_type: None,
        goal_summary: None,
        plan_present: false,
        worktree_id: None,
        worktree_branch: None,
    }
}

async fn context(app: &AppState, chat_id: &str) -> Arc<Mutex<AtCommandsContext>> {
    Arc::new(Mutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            vec![],
            chat_id.into(),
            Some("root".into()),
            "test/model".into(),
            None,
            None,
        )
        .await,
    ))
}

async fn fixture() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    AppState,
    BackgroundAgent,
) {
    let workspace = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        workspace.path().to_path_buf(),
        config.path().to_path_buf(),
    )
    .await;
    let app = AppState::from_gcx(gcx).await;
    let (record, _, _) = app.agents.create(request("root")).await.unwrap();
    let record = app
        .agents
        .mark_running(&record.agent_id, "child".into())
        .await
        .unwrap();
    (workspace, config, app, record)
}

fn args(agent: &str, generation: u64) -> HashMap<String, Value> {
    HashMap::from([
        ("target".into(), json!({"kind": "child", "agent_id": agent})),
        ("expected_run_generation".into(), json!(generation)),
    ])
}

fn output(result: (bool, Vec<ContextEnum>)) -> String {
    let ContextEnum::ChatMessage(message) = &result.1[0] else {
        panic!("expected tool message")
    };
    message.content.content_text_only()
}

async fn control(app: &AppState, caller: &str, agent: &str, action: &str) -> Result<Value, String> {
    let mut tool: Box<dyn Tool + Send> = if action == "pause" {
        Box::new(ToolPause {
            config_path: String::new(),
        })
    } else {
        assert_eq!(action, "release");
        Box::new(ToolResume {
            config_path: String::new(),
        })
    };
    let generation = app.agents.get_any(agent).await.unwrap().run_generation;
    let result = tool
        .tool_execute(
            context(app, caller).await,
            &"control".into(),
            &args(agent, generation),
        )
        .await?;
    Ok(serde_json::from_str(&output(result)).unwrap())
}

fn config(agent: &str) -> SubchatConfig {
    SubchatConfig {
        tool_name: "subagent".into(),
        history_source_model: None,
        stateful: true,
        autonomous_no_confirm: false,
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
        chat_id: Some("child".into()),
        title: None,
        parent_id: Some("root".into()),
        link_type: Some("subagent".into()),
        root_chat_id: Some("root".into()),
        tools: ToolsPolicy::None,
        max_steps: 2,
        prepend_system_prompt: false,
        wrap_up: None,
        task_meta: None,
        worktree: None,
        model: "test/model".into(),
        mode: "agent".into(),
        n_ctx: 4096,
        implicit_context_window: false,
        max_new_tokens: 512,
        temperature: None,
        reasoning_effort: None,
        cache_control: crate::llm::params::CacheControl::Ephemeral,
        parent_tool_call_id: None,
        parent_subchat_tx: None,
        abort_flag: None,
        soft_abort: false,
        activity_stamp: None,
        background_agent_id: Some(agent.into()),
        subchat_depth: 1,
        final_step_force_answer: false,
        buddy_meta: None,
        step_progress: None,
        trace_parent: TraceParent::rooted("root", "root"),
    }
}

#[tokio::test]
async fn parent_pause_blocks_child_at_next_boundary_and_release_continues() {
    let (_workspace, _config, app, record) = fixture().await;
    let child_context = context(&app, "child").await;
    child_context.lock().await.background_agent_id = Some(record.agent_id.clone());
    let config = config(&record.agent_id);
    let gate = app.agents.pause_gate(&record.agent_id).await.unwrap();
    let (started_tx, started_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    let mut runner = tokio::spawn({
        let app = app.clone();
        let agent_id = record.agent_id.clone();
        async move {
            assert!(!crate::subchat::hold_native_pause(&child_context, &config).await);
            app.agents
                .update_progress(&agent_id, "step 1".into(), 1)
                .await
                .unwrap();
            started_tx.send(()).unwrap();
            finish_rx.await.unwrap();
            assert!(!crate::subchat::hold_native_pause(&child_context, &config).await);
            app.agents
                .update_progress(&agent_id, "step 2".into(), 2)
                .await
                .unwrap();
        }
    });
    started_rx.await.unwrap();
    let paused = control(&app, "root", &record.agent_id, "pause")
        .await
        .unwrap();
    assert_eq!(paused["status"], "paused");
    assert_eq!(paused["paused_by"], "root");
    assert_eq!(paused["pause"]["quiescent"], false);
    finish_tx.send(()).unwrap();
    tokio::select! {
        _ = gate.quiesced() => {}
        result = &mut runner => panic!("child crossed the paused boundary: {result:?}"),
        _ = tokio::time::sleep(Duration::from_secs(10)) => panic!("child did not reach its pause boundary"),
    }
    assert_eq!(
        app.agents
            .get_any(&record.agent_id)
            .await
            .unwrap()
            .step_count,
        1
    );
    assert!(!runner.is_finished());
    let released = control(&app, "root", &record.agent_id, "release")
        .await
        .unwrap();
    assert_eq!(released["status"], "running");
    assert!(released["paused_by"].is_null());
    tokio::time::timeout(Duration::from_secs(10), runner)
        .await
        .unwrap()
        .unwrap();
    let after = app.agents.get_any(&record.agent_id).await.unwrap();
    assert_eq!(after.step_count, 2);
    assert_eq!(after.run_generation, record.run_generation);
}

#[tokio::test]
async fn pause_survives_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("agents");
    let registry = BackgroundAgentRegistry::new(root.clone()).await.unwrap();
    let (record, _, _) = registry.create(request("parent")).await.unwrap();
    registry
        .mark_running(&record.agent_id, "child".into())
        .await
        .unwrap();
    let paused = registry
        .set_parent_pause(&record.agent_id, "parent", true)
        .await
        .unwrap();
    let disk = crate::agents::storage::load_all(&root).await.unwrap();
    assert_eq!(disk[&record.agent_id].paused_by.as_deref(), Some("parent"));
    assert_eq!(disk[&record.agent_id].paused_at, paused.paused_at);
    drop(registry);
    let restored = BackgroundAgentRegistry::new(root.clone()).await.unwrap();
    let interrupted = restored.get_any(&record.agent_id).await.unwrap();
    assert_eq!(interrupted.status, BgAgentStatus::Interrupted);
    assert_eq!(interrupted.paused_by, paused.paused_by);
    assert_eq!(interrupted.paused_at, paused.paused_at);
    assert!(!restored.has_runtime(&record.agent_id).await);
    let (reopened, abort, _) = restored
        .reopen(&record.agent_id, "parent", interrupted.change_seq, false)
        .await
        .unwrap();
    assert!(reopened.is_paused());
    let gate = restored.pause_gate(&record.agent_id).await.unwrap();
    let held = tokio::spawn({
        let gate = gate.clone();
        async move { gate.hold(Some(&abort)).await }
    });
    tokio::time::timeout(Duration::from_secs(10), gate.quiesced())
        .await
        .unwrap();
    assert!(!held.is_finished());
    restored
        .set_parent_pause(&record.agent_id, "root", false)
        .await
        .unwrap();
    assert!(!held.await.unwrap());
    let disk = crate::agents::storage::load_all(&root).await.unwrap();
    assert!(disk[&record.agent_id].paused_by.is_none());
    assert!(disk[&record.agent_id].paused_at.is_none());
    let mut legacy = serde_json::to_value(&reopened).unwrap();
    legacy.as_object_mut().unwrap().remove("paused_by");
    legacy.as_object_mut().unwrap().remove("paused_at");
    let legacy: BackgroundAgent = serde_json::from_value(legacy).unwrap();
    assert!(!legacy.is_paused());
}

#[tokio::test]
async fn pause_refused_for_terminal_agent() {
    let (_workspace, _config, app, record) = fixture().await;
    for status in [
        BgAgentStatus::Completed,
        BgAgentStatus::Failed,
        BgAgentStatus::Cancelled,
        BgAgentStatus::Interrupted,
        BgAgentStatus::Closed,
    ] {
        let (record, _, _) = app.agents.create(request("root")).await.unwrap();
        let terminal = match status {
            BgAgentStatus::Completed => app
                .agents
                .mark_completed(
                    &record.agent_id,
                    crate::agents::types::AgentCompletion {
                        result_summary: "done".into(),
                        edited_files: vec![],
                        diff_summary: None,
                        conflict_summary: None,
                        child_chat_id: None,
                        deliverable_status: None,
                        handoff: None,
                    },
                )
                .await
                .unwrap(),
            BgAgentStatus::Failed => app
                .agents
                .mark_failed(&record.agent_id, "failed".into())
                .await
                .unwrap(),
            BgAgentStatus::Cancelled => app
                .agents
                .mark_cancelled(&record.agent_id, None)
                .await
                .unwrap(),
            BgAgentStatus::Interrupted => app
                .agents
                .mark_interrupted(&record.agent_id, "restart".into())
                .await
                .unwrap(),
            BgAgentStatus::Closed => {
                app.agents
                    .mark_closed(&record.agent_id, None, None)
                    .await
                    .unwrap()
                    .0
            }
            _ => unreachable!(),
        };
        for action in ["pause", "release"] {
            let error = control(&app, "root", &record.agent_id, action)
                .await
                .unwrap_err();
            assert_eq!(
                error,
                refact_chat_api::SealReason::AgentTerminal {
                    agent_id: record.agent_id.clone(),
                    status: status.as_str().into(),
                    run_generation: record.run_generation,
                }
                .message()
            );
            assert_eq!(
                app.agents.get_any(&record.agent_id).await.unwrap(),
                terminal
            );
        }
    }
    assert!(!app
        .agents
        .get_any(&record.agent_id)
        .await
        .unwrap()
        .is_paused());
}

#[tokio::test]
async fn only_parent_or_root_may_pause() {
    let (_workspace, _config, app, _) = fixture().await;
    let (record, _, _) = app.agents.create(request("parent")).await.unwrap();
    let gate = app.agents.pause_gate(&record.agent_id).await.unwrap();
    for caller in ["sibling", "child", "unrelated"] {
        for action in ["pause", "release"] {
            let error = control(&app, caller, &record.agent_id, action)
                .await
                .unwrap_err();
            assert!(error.contains("not found in this chat"), "{error}");
        }
    }
    assert!(!gate.is_requested());
    let paused = control(&app, "parent", &record.agent_id, "pause")
        .await
        .unwrap();
    assert_eq!(paused["paused_by"], "parent");
    let repeated = control(&app, "root", &record.agent_id, "pause")
        .await
        .unwrap();
    assert_eq!(repeated["paused_by"], "parent");
    assert_eq!(repeated["paused_at"], paused["paused_at"]);
    control(&app, "root", &record.agent_id, "release")
        .await
        .unwrap();
    assert!(!gate.is_requested());
    let paused = control(&app, "root", &record.agent_id, "pause")
        .await
        .unwrap();
    assert_eq!(paused["paused_by"], "root");
    control(&app, "parent", &record.agent_id, "release")
        .await
        .unwrap();
    assert!(!gate.is_requested());
}

#[tokio::test]
async fn paused_children_are_not_stalled_or_nudged_and_consume_no_goal_turns() {
    let (_workspace, _config, app, record) = fixture().await;
    let mut session = ChatSession::new("child".into());
    session.install_goal("agent", "fake goal", true, GoalBudget::default());
    session.last_activity = Instant::now() - Duration::from_secs(3600);
    session
        .queue_processor_running
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let session = Arc::new(Mutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert("child".into(), session.clone());
    let before = session.lock().await.goal.clone();
    control(&app, "root", &record.agent_id, "pause")
        .await
        .unwrap();
    app.agents
        .set_last_update_at_for_test(&record.agent_id, Utc::now() - TimeDelta::minutes(60))
        .await
        .unwrap();
    for trigger in [GoalNudgeTrigger::Monitor, GoalNudgeTrigger::InlineTurnEnd] {
        assert_eq!(
            dispatch_goal_nudge(app.clone(), session.clone(), trigger).await,
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::IntentionallyWaiting)
        );
    }
    crate::agents::monitor::monitor_once(app.clone())
        .await
        .unwrap();
    assert!(app
        .agents
        .get_any(&record.agent_id)
        .await
        .unwrap()
        .progress
        .is_none());
    let held = session.lock().await;
    assert_eq!(held.goal, before);
    assert!(held.command_queue.is_empty());
    assert!(held.pending_deliveries.is_empty());
    drop(held);
    control(&app, "root", &record.agent_id, "release")
        .await
        .unwrap();
    assert!(matches!(
        dispatch_goal_nudge(app.clone(), session, GoalNudgeTrigger::InlineTurnEnd).await,
        GoalNudgeOutcome::Nudged(_)
    ));
}

#[tokio::test]
async fn parent_and_native_pause_owners_do_not_release_each_other() {
    let (_workspace, _config, app, record) = fixture().await;
    let gate = app.agents.pause_gate(&record.agent_id).await.unwrap();
    app.agents.request_pause(&record.agent_id).await.unwrap();
    control(&app, "root", &record.agent_id, "pause")
        .await
        .unwrap();
    control(&app, "root", &record.agent_id, "release")
        .await
        .unwrap();
    assert!(gate.is_requested());
    control(&app, "root", &record.agent_id, "pause")
        .await
        .unwrap();
    app.agents.release_pause(&record.agent_id).await.unwrap();
    assert!(gate.is_requested());
    control(&app, "root", &record.agent_id, "release")
        .await
        .unwrap();
    assert!(!gate.is_requested());
}

#[tokio::test]
async fn pause_and_release_reject_mutation_fields_and_require_confirmation() {
    let (_workspace, _config, app, record) = fixture().await;
    for action in ["pause", "release"] {
        let mut tool: Box<dyn Tool + Send> = if action == "pause" {
            Box::new(ToolPause {
                config_path: String::new(),
            })
        } else {
            Box::new(ToolResume {
                config_path: String::new(),
            })
        };
        let request = args(&record.agent_id, record.run_generation);
        let confirmation = tool
            .match_against_confirm_deny(context(&app, "root").await, &request)
            .await
            .unwrap();
        assert!(matches!(
            confirmation.result,
            MatchConfirmDenyResult::CONFIRMATION
        ));
        for key in [
            "max_steps",
            "expected_change_seq",
            "instruction",
            "reconcile_incomplete",
            "action",
            "until_ms",
        ] {
            let mut invalid = request.clone();
            invalid.insert(key.into(), json!(1));
            let error = tool
                .tool_execute(context(&app, "root").await, &"invalid".into(), &invalid)
                .await
                .unwrap_err();
            assert!(
                error.contains(&format!("field {key} is inapplicable to child")),
                "{error}"
            );
        }
    }
}
