use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use refact_chat_api::{AskQuestion, QuestionKind, QuestionSettlement, QuestionState};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::agents::types::{AgentCompletion, BgAgentKind, CreateAgentRequest};
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::attention::{AttentionRequest, AttentionScope};
use crate::call_validation::ContextEnum;
use crate::chat::task_agent_monitor::tests::setup_monitor_case;
use crate::chat::types::{ChatSession, SessionState};
use crate::questions::{board_card_asker, board_planner_answerer};
use crate::tasks::storage;
use crate::tasks::types::StatusUpdate;
use crate::tools::tool_agent_planner_qna::{
    ToolAgentAskPlanner, ToolPlannerReply, ToolTaskQuestionsList,
};
use crate::tools::tool_task_agent_finish::ToolTaskAgentFinish;
use crate::tools::tools_description::Tool;

pub(crate) struct Fixture {
    _temp: tempfile::TempDir,
    pub(crate) app: AppState,
    pub(crate) task_id: String,
    pub(crate) agent_id: String,
    agent: Arc<Mutex<ChatSession>>,
    planner: Arc<Mutex<ChatSession>>,
}

pub(crate) async fn fixture() -> Fixture {
    let (_temp, app, task_id, agent_id, agent, planner) = setup_monitor_case(
        "doing",
        SessionState::ExecutingTools,
        Duration::ZERO,
        vec![],
    )
    .await;
    {
        let mut session = planner.lock().await;
        session.thread.model = "provider/model".into();
        session.work_project_root = Some(_temp.path().to_path_buf());
    }
    {
        let mut session = agent.lock().await;
        session.thread.model = "provider/model".into();
        session.work_project_root = Some(_temp.path().to_path_buf());
    }
    Fixture {
        _temp,
        app,
        task_id,
        agent_id,
        agent,
        planner,
    }
}

async fn context(f: &Fixture, planner: bool) -> Arc<Mutex<AtCommandsContext>> {
    let session = if planner { &f.planner } else { &f.agent };
    let (id, meta) = {
        let session = session.lock().await;
        (session.thread.id.clone(), session.thread.task_meta.clone())
    };
    Arc::new(Mutex::new(
        AtCommandsContext::new_from_app(
            f.app.clone(),
            4096,
            20,
            false,
            vec![],
            id,
            None,
            "test-model".into(),
            meta,
            None,
        )
        .await,
    ))
}

fn args(values: &[(&str, Value)]) -> HashMap<String, Value> {
    values
        .iter()
        .map(|(key, value)| (key.to_string(), value.clone()))
        .collect()
}

fn text(result: (bool, Vec<ContextEnum>)) -> String {
    match result.1.into_iter().next().unwrap() {
        ContextEnum::ChatMessage(message) => message.content.content_text_only(),
        _ => panic!("expected tool output"),
    }
}

pub(crate) async fn ask(f: &Fixture) -> String {
    ToolAgentAskPlanner::new()
        .tool_execute(
            context(f, false).await,
            &"ask".into(),
            &args(&[
                ("question", json!("Which approach?")),
                ("urgency", json!("info")),
            ]),
        )
        .await
        .unwrap();
    f.app
        .questions()
        .pending_for_asker(&board_card_asker(&f.task_id, "T-1"), Utc::now())
        .into_iter()
        .find(|q| q.text == "Which approach?")
        .unwrap()
        .id
}

async fn finish(f: &Fixture, success: bool) {
    ToolTaskAgentFinish::new()
        .tool_execute(
            context(f, false).await,
            &"finish".into(),
            &args(&[
                ("success", json!(success)),
                ("report", json!("Question closure regression")),
            ]),
        )
        .await
        .unwrap();
}

fn attention_question(f: &Fixture, kind: QuestionKind, asker: &str, waiter: &str) -> String {
    f.app
        .attention()
        .join(
            AttentionRequest::new(
                AttentionScope::new(&f.task_id, "test"),
                kind,
                asker,
                if kind == QuestionKind::BoardPlanner {
                    board_planner_answerer(&f.task_id)
                } else {
                    "parent".into()
                },
                "Shared tracked question?",
            ),
            waiter,
            Utc::now(),
        )
        .unwrap()
        .question_id
}

#[tokio::test]
async fn agent_finish_withdraws_open_questions() {
    for success in [true, false] {
        let f = fixture().await;
        let open = ask(&f).await;
        let tracked = attention_question(
            &f,
            QuestionKind::BoardPlanner,
            &board_card_asker(&f.task_id, "T-1"),
            &f.agent_id,
        );
        let user_question = attention_question(
            &f,
            QuestionKind::PermissionEscalation,
            &f.agent_id,
            &f.agent_id,
        );
        let answered = "aaaaaaaa";
        let mut legacy = storage::load_board(f.app.gcx.clone(), &f.task_id)
            .await
            .unwrap();
        legacy
            .get_card_mut("T-1")
            .unwrap()
            .status_updates
            .extend([update(&format!("[ASK:{answered}] Already answered"))]);
        storage::save_board(f.app.gcx.clone(), &f.task_id, &legacy)
            .await
            .unwrap();
        ToolPlannerReply::new()
            .tool_execute(
                context(&f, true).await,
                &"answer".into(),
                &args(&[
                    ("card_id", json!("T-1")),
                    ("question_id", json!(answered)),
                    ("answer", json!("Proceed")),
                ]),
            )
            .await
            .unwrap();
        storage::update_board_atomic(f.app.gcx.clone(), &f.task_id, |board| {
            board
                .get_card_mut("T-1")
                .unwrap()
                .status_updates
                .extend([update(
                    "[ASK:bbbbbbbb] Durable question from before process restart",
                )]);
            Ok(())
        })
        .await
        .unwrap();

        finish(&f, success).await;

        assert_eq!(
            f.app.questions().get(&open).unwrap().state,
            QuestionState::Withdrawn,
            "agent_finish must withdraw its open planner question"
        );
        assert_eq!(
            f.app.questions().get("bbbbbbbb").unwrap().state,
            QuestionState::Withdrawn
        );
        assert_eq!(
            f.app.questions().settlement(&tracked, Utc::now()),
            Some(QuestionSettlement::Withdrawn)
        );
        assert_eq!(f.app.attention().blocked_runs(&tracked), 0);
        assert!(!f
            .app
            .attention()
            .questions_for_waiter(&f.agent_id)
            .contains(&tracked));
        assert_eq!(
            f.app.questions().get(&user_question).unwrap().state,
            QuestionState::Pending
        );
        let board = storage::load_board(f.app.gcx.clone(), &f.task_id)
            .await
            .unwrap();
        let withdrawals: Vec<_> = board
            .get_card("T-1")
            .unwrap()
            .status_updates
            .iter()
            .filter(|u| u.message.starts_with("[WITHDRAWN:"))
            .collect();
        assert_eq!(withdrawals.len(), 1);
        let ids: Vec<_> = withdrawals[0]
            .message
            .strip_prefix("[WITHDRAWN:")
            .unwrap()
            .split_once(']')
            .unwrap()
            .0
            .split(',')
            .collect();
        for id in [&open, &tracked, "bbbbbbbb"] {
            assert!(ids.contains(&id));
        }
        assert!(!ids.contains(&answered));
    }
}

#[tokio::test]
async fn cancel_and_restart_withdraw_old_agent_questions() {
    for replacement in [false, true] {
        let f = fixture().await;
        let old = ask(&f).await;
        storage::update_board_atomic(f.app.gcx.clone(), &f.task_id, |board| {
            let card = board.get_card_mut("T-1").unwrap();
            if replacement {
                card.agent_chat_id = Some("replacement-chat".into());
            } else {
                card.column = "failed".into();
                card.agent_chat_id = None;
            }
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            f.app.questions().get(&old).unwrap().state,
            QuestionState::Withdrawn
        );
        storage::update_board_atomic(f.app.gcx.clone(), &f.task_id, |board| {
            let card = board.get_card_mut("T-1").unwrap();
            card.column = "doing".into();
            card.agent_chat_id = Some("replacement-chat".into());
            Ok(())
        })
        .await
        .unwrap();
        let ccx = context(&f, false).await;
        ccx.lock().await.chat_id = "replacement-chat".into();
        ToolAgentAskPlanner::new()
            .tool_execute(
                ccx,
                &"new-ask".into(),
                &args(&[
                    ("question", json!("New question")),
                    ("urgency", json!("info")),
                ]),
            )
            .await
            .unwrap();
        let pending = f
            .app
            .questions()
            .pending_for_asker(&board_card_asker(&f.task_id, "T-1"), Utc::now());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text, "New question");
        let error = ToolAgentAskPlanner::new()
            .tool_execute(
                context(&f, false).await,
                &"stale-ask".into(),
                &args(&[
                    ("question", json!("Zombie question")),
                    ("urgency", json!("info")),
                ]),
            )
            .await
            .unwrap_err();
        assert!(error.contains("current doing card agent"));
    }
}

fn request() -> CreateAgentRequest {
    CreateAgentRequest {
        parent_chat_id: "parent".into(),
        parent_root_chat_id: None,
        parent_tool_call_id: None,
        kind: BgAgentKind::Subagent,
        config_name: "test".into(),
        title: "Question child".into(),
        prompt: "Ask".into(),
        target_files: vec![],
        model: "test-model".into(),
        model_type: None,
        goal_summary: None,
        plan_present: false,
        worktree_id: None,
        worktree_branch: None,
    }
}

#[tokio::test]
async fn terminal_background_agent_withdraws_its_questions() {
    for terminal in [
        "completed",
        "failed",
        "cancelled",
        "interrupted",
        "closed",
        "restore",
    ] {
        let f = fixture().await;
        let root = tempfile::tempdir().unwrap();
        let registry =
            crate::agents::registry::BackgroundAgentRegistry::new(root.path().to_path_buf())
                .await
                .unwrap();
        let record = registry.create(request()).await.unwrap().0;
        let (_, question) = registry
            .add_question(&record.agent_id, "Still needed?".into())
            .await
            .unwrap();
        let (_, answered) = registry
            .add_question(&record.agent_id, "Answered".into())
            .await
            .unwrap();
        registry
            .answer_question(&record.agent_id, &answered, "yes".into())
            .await
            .unwrap();
        let user = registry
            .questions()
            .ask(
                AskQuestion::new(
                    QuestionKind::PermissionEscalation,
                    &record.agent_id,
                    "user",
                    "approval",
                    "Allow?",
                ),
                Utc::now(),
            )
            .unwrap();
        let budget =
            crate::attention::AttentionBudget::from_settings(registry.questions().clone()).unwrap();
        let attention = budget
            .join(
                AttentionRequest::new(
                    AttentionScope::new(&f.task_id, "test"),
                    QuestionKind::AgentParent,
                    &record.agent_id,
                    "parent",
                    "Attention?",
                ),
                "child",
                Utc::now(),
            )
            .unwrap();
        let result = match terminal {
            "completed" => registry
                .mark_completed(
                    &record.agent_id,
                    AgentCompletion {
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
            "failed" => registry
                .mark_failed(&record.agent_id, "failed".into())
                .await
                .unwrap(),
            "cancelled" => registry
                .cancel("parent", &record.agent_id, None)
                .await
                .unwrap(),
            "interrupted" => registry
                .mark_interrupted(&record.agent_id, "stopped".into())
                .await
                .unwrap(),
            "closed" => {
                registry
                    .close("parent", &record.agent_id, None, None)
                    .await
                    .unwrap()
                    .0
            }
            _ => {
                registry.flush_pending_writes_for_test().await.unwrap();
                let restored = crate::agents::registry::BackgroundAgentRegistry::new(
                    root.path().to_path_buf(),
                )
                .await
                .unwrap();
                assert_eq!(
                    restored.questions().get(&question).unwrap().state,
                    QuestionState::Withdrawn
                );
                restored.get_any(&record.agent_id).await.unwrap()
            }
        };
        assert_eq!(
            result
                .questions
                .iter()
                .find(|q| q.id == question)
                .unwrap()
                .state,
            QuestionState::Withdrawn
        );
        assert_eq!(
            result
                .questions
                .iter()
                .find(|q| q.id == answered)
                .unwrap()
                .state,
            QuestionState::Answered
        );
        if terminal != "restore" {
            assert_eq!(budget.blocked_runs(&attention.question_id), 0);
            assert!(budget.questions_for_waiter("child").is_empty());
            assert!(registry
                .answer_question(&record.agent_id, &question, "late".into())
                .await
                .is_err());
            assert_eq!(
                registry.questions().get(&user.id).unwrap().state,
                QuestionState::Pending
            );
        }
        let restored =
            crate::agents::registry::BackgroundAgentRegistry::new(root.path().to_path_buf())
                .await
                .unwrap();
        assert_eq!(
            restored.questions().get(&question).unwrap().state,
            QuestionState::Withdrawn
        );
    }
}

fn update(message: &str) -> StatusUpdate {
    StatusUpdate {
        timestamp: Utc::now().to_rfc3339(),
        message: message.into(),
    }
}

#[tokio::test]
async fn questions_list_hides_withdrawn_and_done_card_history() {
    let f = fixture().await;
    let open = ask(&f).await;
    f.app
        .questions()
        .ask(
            AskQuestion::new(
                QuestionKind::BoardPlanner,
                board_card_asker(&f.task_id, "T-1"),
                board_planner_answerer(&f.task_id),
                format!("{}:T-1:dddddddd", f.task_id),
                "Expired info question",
            )
            .with_id("dddddddd")
            .with_expiry_secs(Some(1)),
            Utc::now() - chrono::Duration::seconds(2),
        )
        .unwrap();
    storage::update_board_atomic(f.app.gcx.clone(), &f.task_id, |board| {
        let card = board.get_card_mut("T-1").unwrap();
        card.status_updates.extend([
            update("[ASK:aaaaaaaa] Withdrawn question"),
            update("[WITHDRAWN:aaaaaaaa] Finished old run"),
            update("[ASK:bbbbbbbb] Answered question"),
            update("[REPLY:bbbbbbbb] Answer"),
            update("[ASK:dddddddd] Expired info question (urgency=info)"),
        ]);
        let template = card.clone();
        for (id, column) in [("T-2", "done"), ("T-3", "failed"), ("T-4", "planned")] {
            let mut historical = template.clone();
            historical.id = id.into();
            historical.column = column.into();
            historical.status_updates = vec![update("[ASK:cccccccc] Historical unanswered")];
            board.cards.push(historical);
        }
        Ok(())
    })
    .await
    .unwrap();
    ToolPlannerReply::new()
        .tool_execute(
            context(&f, true).await,
            &"answer".into(),
            &args(&[
                ("card_id", json!("T-1")),
                ("question_id", json!("bbbbbbbb")),
                ("answer", json!("Answer")),
            ]),
        )
        .await
        .unwrap();
    let before = storage::load_board(f.app.gcx.clone(), &f.task_id)
        .await
        .unwrap();
    let output = text(
        ToolTaskQuestionsList::new()
            .tool_execute(context(&f, true).await, &"list".into(), &HashMap::new())
            .await
            .unwrap(),
    );
    assert!(output.contains(&format!("| `{open}` |")));
    assert!(!output.contains("| `dddddddd` |"));
    for hidden in ["aaaaaaaa", "bbbbbbbb", "cccccccc"] {
        assert!(!output.contains(&format!("| `{hidden}` |")));
    }
    let after = storage::load_board(f.app.gcx.clone(), &f.task_id)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    let settled = ToolPlannerReply::new()
        .tool_execute(
            context(&f, true).await,
            &"settled-reply".into(),
            &args(&[
                ("card_id", json!("T-1")),
                ("question_id", json!("dddddddd")),
                ("answer", json!("Too late")),
            ]),
        )
        .await
        .unwrap_err();
    assert_eq!(
        settled,
        "Question already settled differently; nothing was delivered"
    );
    let listed = text(
        ToolTaskQuestionsList::new()
            .tool_execute(
                context(&f, true).await,
                &"settled-list".into(),
                &HashMap::new(),
            )
            .await
            .unwrap(),
    );
    assert!(!listed.contains("dddddddd"));
}

#[tokio::test]
async fn planner_reply_to_withdrawn_question_delivers_nothing() {
    for column in ["done", "failed", "doing"] {
        let f = fixture().await;
        let question = ask(&f).await;
        storage::update_board_atomic(f.app.gcx.clone(), &f.task_id, |board| {
            let card = board.get_card_mut("T-1").unwrap();
            if column == "doing" {
                card.agent_chat_id = Some("replacement-chat".into());
            } else {
                card.column = column.into();
            }
            Ok(())
        })
        .await
        .unwrap();
        let before = storage::load_board(f.app.gcx.clone(), &f.task_id)
            .await
            .unwrap();
        let messages = serde_json::to_value(&f.agent.lock().await.messages).unwrap();
        let output = text(
            ToolPlannerReply::new()
                .tool_execute(
                    context(&f, true).await,
                    &"reply".into(),
                    &args(&[
                        ("card_id", json!("T-1")),
                        ("question_id", json!(question)),
                        ("answer", json!("Late answer")),
                    ]),
                )
                .await
                .unwrap(),
        );
        assert_eq!(
            output,
            "question was withdrawn when the agent finished; nothing was delivered"
        );
        let after = storage::load_board(f.app.gcx.clone(), &f.task_id)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
        let agent = f.agent.lock().await;
        assert_eq!(serde_json::to_value(&agent.messages).unwrap(), messages);
        assert!(agent.pending_deliveries.is_empty());
        assert!(agent.command_queue.is_empty());
        assert_eq!(
            f.app.questions().get(&question).unwrap().state,
            QuestionState::Withdrawn
        );
    }
}
