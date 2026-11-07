use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::agents::spawn::{
    install_test_runner, resume_agent, spawn_background_agent, SpawnWorktreeMode,
};
use crate::agents::spawn_lifecycle_tests::{await_completion, repo_fixture, spawn_request};
use crate::agents::types::{
    AgentCompletion, BackgroundAgent, BgAgentKind, BgAgentStatus, CreateAgentRequest,
};
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::chat::types::ChatSession;
use crate::subchat::SubchatResult;
use crate::tools::tool_agent_interact::ToolAgentMessage;
use crate::tools::tools_description::Tool;
use refact_core::chat_types::{DeliveryStatus, PendingDelivery, PushMode};

const PARENT: &str = "followup-parent";

fn git(root: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn context(app: &AppState) -> Arc<Mutex<AtCommandsContext>> {
    Arc::new(Mutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            vec![],
            PARENT.into(),
            Some(PARENT.into()),
            "openai/gpt-4o".into(),
            None,
            None,
        )
        .await,
    ))
}

async fn send(app: &AppState, child: &BackgroundAgent, text: &str) -> (String, Value) {
    let args = HashMap::from([
        ("to".into(), json!(child.agent_id)),
        ("text".into(), json!(text)),
    ]);
    let result = ToolAgentMessage {
        config_path: String::new(),
    }
    .tool_execute(context(app).await, &"message".into(), &args)
    .await
    .unwrap();
    let ContextEnum::ChatMessage(message) = &result.1[0] else {
        panic!("tool receipt")
    };
    (
        message.content.content_text_only(),
        message.extra["delivery_receipt"].clone(),
    )
}

async fn child(app: &AppState) -> BackgroundAgent {
    app.agents
        .create(CreateAgentRequest {
            parent_chat_id: PARENT.into(),
            parent_root_chat_id: Some(PARENT.into()),
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "subagent".into(),
            title: "followups".into(),
            prompt: "test".into(),
            target_files: vec![],
            model: "openai/gpt-4o".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        })
        .await
        .unwrap()
        .0
}

fn completion() -> AgentCompletion {
    AgentCompletion {
        result_summary: "done".into(),
        edited_files: vec![],
        diff_summary: None,
        conflict_summary: None,
        child_chat_id: None,
        deliverable_status: None,
        handoff: None,
    }
}

async fn settled(app: &AppState, child: &BackgroundAgent) -> BackgroundAgent {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let record = app.agents.get_any(&child.agent_id).await.unwrap();
            let chat = record.child_chat_id.as_deref().unwrap();
            if record.status.is_terminal()
                && !app.agent_executions.task_active(chat)
                && app.agent_executions.get(chat).is_none()
            {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("child settled")
}

#[tokio::test]
#[serial_test::serial(test_runner)]
async fn message_to_completed_child_is_saved_and_delivered_on_continue() {
    for isolated in [false, true] {
        let fixture = repo_fixture().await;
        std::fs::write(fixture.source.join(".gitignore"), ".refact/\n").unwrap();
        std::fs::write(fixture.source.join(".git/info/exclude"), ".refact/\n").unwrap();
        git(&fixture.source, &["add", ".gitignore"]);
        git(&fixture.source, &["commit", "-m", "ignore runtime state"]);
        let mut parent = ChatSession::new(PARENT.into());
        parent.work_project_root = Some(fixture.source.clone());
        parent.thread.mode = "agent".into();
        parent.thread.model = "openai/gpt-4o".into();
        fixture
            .app
            .chat
            .sessions
            .write()
            .await
            .insert(PARENT.into(), Arc::new(Mutex::new(parent)));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let _runner = install_test_runner(Arc::new({
            let seen = seen.clone();
            move |gcx, mut messages, config| {
                let seen = seen.clone();
                Box::pin(async move {
                    let app = AppState::from_gcx(gcx).await;
                    seen.lock().await.push(messages.clone());
                    messages.push(ChatMessage::new("assistant".into(), "done".into()));
                    let chat = config.chat_id.as_deref().unwrap();
                    let session = app.chat.sessions.read().await.get(chat).cloned().unwrap();
                    let thread = session.lock().await.thread.clone();
                    crate::chat::trajectories::save_trajectory_as_with_intent_checked(
                        app.gcx.clone(),
                        &thread,
                        &messages,
                        crate::chat::types::TrajectoryCommitIntent::Required,
                    )
                    .await?;
                    crate::subchat::mirror_subchat_messages_into_session(&app, chat, &messages)
                        .await;
                    crate::subchat::finish_stateful_subchat_session(&app, chat, &config).await;
                    crate::chat::trajectories::flush_trajectory_for_session(app.clone(), session)
                        .await
                        .unwrap();
                    Ok(SubchatResult {
                        messages,
                        chat_id: config.chat_id,
                        metering: Default::default(),
                        aborted: false,
                    })
                })
            }
        }));
        let mut request = spawn_request(PARENT, BgAgentKind::Subagent);
        if isolated {
            request.worktree_mode = SpawnWorktreeMode::Isolated {
                auto_merge: true,
            };
        }
        let first = await_completion(
            spawn_background_agent(fixture.app.clone(), request)
                .await
                .unwrap(),
        )
        .await;
        let first = settled(&fixture.app, &first).await;
        assert_eq!(first.status, BgAgentStatus::Completed);
        if isolated {
            assert_eq!(
                first.merge_status.as_deref(),
                Some("merged"),
                "{:?}",
                first.error
            );
        }
        let (text, receipt) = send(&fixture.app, &first, "saved first follow-up").await;
        assert!(text.contains("finished (status completed)"), "{text}");
        assert!(text.contains("agent_lifecycle action=continue"), "{text}");
        assert!(!text.contains("queued"), "{text}");
        assert_eq!(receipt["outcome"], "followup_saved");
        assert_eq!(receipt["accepted"], true);
        send(&fixture.app, &first, "saved second follow-up").await;
        let pending = fixture.app.agents.get_any(&first.agent_id).await.unwrap();
        assert!(pending.pending_deliveries.is_empty());
        assert_eq!(pending.followups.len(), 2);
        let resumed = resume_agent(
            fixture.app.clone(),
            PARENT,
            &first.agent_id,
            None,
            pending.change_seq,
            Some("new continuation instruction".into()),
            false,
        )
        .await
        .unwrap();
        let done = settled(&fixture.app, &resumed).await;
        assert_eq!(done.status, BgAgentStatus::Completed, "{:?}", done.error);
        assert!(done.followups.is_empty());
        assert!(done.pending_deliveries.is_empty());
        let seen = seen.lock().await;
        assert_eq!(seen.len(), 2);
        let texts: Vec<_> = seen[1]
            .iter()
            .map(|message| message.content.content_text_only())
            .collect();
        let first_index = texts
            .iter()
            .position(|text| text.contains("saved first follow-up"))
            .expect("first follow-up delivered");
        let second_index = texts
            .iter()
            .position(|text| text.contains("saved second follow-up"))
            .expect("second follow-up delivered");
        let instruction = texts
            .iter()
            .position(|text| text == "new continuation instruction")
            .unwrap();
        assert!(first_index < second_index && second_index < instruction);
        assert_eq!(
            texts
                .iter()
                .filter(|text| text.contains("saved first follow-up"))
                .count(),
            1
        );
        let saved = crate::chat::trajectories::load_trajectory_for_chat(
            fixture.app.gcx.clone(),
            done.child_chat_id.as_deref().unwrap(),
        )
        .await
        .unwrap();
        assert!(saved.messages.iter().any(|message| message
            .content
            .content_text_only()
            .contains("saved first follow-up")));
        assert_eq!(
            fixture
                .app
                .agents
                .delivery_status(&done.agent_id, receipt["delivery_id"].as_str().unwrap())
                .await,
            DeliveryStatus::Consumed
        );
        if isolated {
            assert_ne!(done.worktree_id, first.worktree_id);
        }
    }
}

#[tokio::test]
async fn followup_queue_is_bounded_and_reports_drops() {
    let fixture = repo_fixture().await;
    let child = child(&fixture.app).await;
    fixture
        .app
        .agents
        .mark_completed(&child.agent_id, completion())
        .await
        .unwrap();
    let settings = refact_work_core::coordination_settings();
    let mut first_id = String::new();
    for index in 0..=settings.agent_followup_max_count {
        let (text, receipt) = send(&fixture.app, &child, &format!("note {index}")).await;
        if index == 0 {
            first_id = receipt["delivery_id"].as_str().unwrap().into();
        }
        if index == settings.agent_followup_max_count {
            assert!(text.contains("dropped 1 oldest"), "{text}");
            assert_eq!(receipt["dropped_followups"], json!([first_id]));
        }
    }
    let after = fixture.app.agents.get_any(&child.agent_id).await.unwrap();
    assert_eq!(after.followups.len(), settings.agent_followup_max_count);
    assert_eq!(
        fixture
            .app
            .agents
            .delivery_status(&child.agent_id, &first_id)
            .await,
        DeliveryStatus::Failed
    );
    let large = "🦀".repeat(settings.agent_message_max_chars);
    let mut dropped = false;
    for _ in 0..=settings.agent_followup_max_bytes / large.len() {
        let (text, receipt) = send(&fixture.app, &child, &large).await;
        dropped |= text.contains("dropped")
            && !receipt["dropped_followups"].as_array().unwrap().is_empty();
    }
    let after = fixture.app.agents.get_any(&child.agent_id).await.unwrap();
    let bytes: usize = after
        .followups
        .iter()
        .map(|delivery| serde_json::to_vec(delivery).unwrap().len())
        .sum();
    assert!(bytes <= settings.agent_followup_max_bytes);
    assert!(after.followups.len() < settings.agent_followup_max_count);
    assert!(dropped);
}

#[tokio::test]
async fn terminal_question_reply_is_refused_instead_of_claiming_delivery() {
    let fixture = repo_fixture().await;
    let child = child(&fixture.app).await;
    let (_, question) = fixture
        .app
        .agents
        .add_question(&child.agent_id, "May I?".into())
        .await
        .unwrap();
    fixture
        .app
        .agents
        .mark_completed(&child.agent_id, completion())
        .await
        .unwrap();
    let error = ToolAgentMessage {
        config_path: String::new(),
    }
    .tool_execute(
        context(&fixture.app).await,
        &"reply".into(),
        &HashMap::from([
            ("to".into(), json!(child.agent_id)),
            ("text".into(), json!("yes")),
            ("reply_to".into(), json!(question)),
        ]),
    )
    .await
    .unwrap_err();
    assert!(error.contains("withdrawn"), "{error}");
    assert!(fixture
        .app
        .agents
        .get_any(&child.agent_id)
        .await
        .unwrap()
        .followups
        .is_empty());
}
