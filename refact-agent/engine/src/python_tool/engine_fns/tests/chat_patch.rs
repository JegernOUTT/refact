use super::chats_risky::{install, message, until};
use super::*;
use crate::chat::types::ChatSession;
use refact_chat_api::PythonEngineClass as Class;

const CALL: &str = "call-chat-patch";
const EDIT_FIRST: &str =
    "await engine('chat.patch', ops=[{'op':'edit','message_id':'cp-m0','text':'edited question'}])";

/// This chat in the middle of a turn that a typed question started: its last message is the
/// assistant message holding the running `python` call `CALL`.
async fn running_turn(app: &AppState) {
    install(app).await;
    let session = session(app).await;
    let mut s = session.lock().await;
    s.thread.model = "model".into();
    let mut call = message("cp-m3", "assistant", "");
    call.tool_calls = Some(vec![serde_json::from_value(json!({
        "id": CALL,
        "type": "function",
        "function": {"name": "python", "arguments": "{}"}
    }))
    .unwrap()]);
    s.messages = vec![
        message("cp-m0", "user", "first question"),
        message("cp-m1", "assistant", "first answer"),
        message("cp-m2", "user", "second question"),
        call,
    ];
    s.runtime.state = refact_chat_api::SessionState::ExecutingTools;
    s.turn_depth = 1;
}

/// Runs `code` as the running `python` call of this chat's turn.
fn spawn_call(app: &AppState, code: &str) -> tokio::task::JoinHandle<ChatMessage> {
    let app = app.clone();
    let code = code.to_string();
    tokio::spawn(async move {
        let ccx = Arc::new(tokio::sync::Mutex::new(
            crate::at_commands::at_commands::AtCommandsContext::new_from_app(
                app.clone(),
                16384,
                20,
                false,
                vec![],
                "python-test".into(),
                None,
                "model".into(),
                None,
                None,
            )
            .await,
        ));
        let (_, messages) = tokio::time::timeout(
            Duration::from_secs(60),
            crate::tools::tool_python::ToolPython::default().tool_execute(
                ccx,
                &CALL.to_string(),
                &HashMap::from([("code".into(), json!(code))]),
            ),
        )
        .await
        .expect("python run exceeded its budget")
        .unwrap();
        match messages.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(m) => m,
            _ => panic!("expected a message"),
        }
    })
}

async fn approve_call(
    app: &AppState,
    code: &str,
) -> (refact_chat_api::PythonApprovalRequest, Value) {
    let task = spawn_call(app, code);
    let request = next_request(app, &task).await;
    decide(app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    (request, result(&finish(task).await))
}

/// Runs a call that must be refused before any card; returns the Python error.
async fn refused_call(app: &AppState, code: &str) -> (String, String) {
    let m = finish(spawn_call(app, code)).await;
    assert!(
        session(app)
            .await
            .lock()
            .await
            .runtime
            .python_approvals
            .is_empty(),
        "a card was raised"
    );
    let error = &m.extra["python"]["error"];
    (
        error["type"]
            .as_str()
            .unwrap_or_else(|| panic!("not refused: {}", m.content.content_text_only()))
            .to_string(),
        error["message"].as_str().unwrap_or_default().to_string(),
    )
}

/// The rest of the turn: the result of the call and a closing answer; then the queue runs.
async fn end_turn(app: &AppState) {
    let session = session(app).await;
    {
        let mut s = session.lock().await;
        let mut result = message("cp-m4", "tool", "python output");
        result.tool_call_id = CALL.into();
        s.messages.push(result);
        s.messages.push(message("cp-m5", "assistant", "done"));
        s.runtime.state = refact_chat_api::SessionState::Idle;
        s.turn_depth = 0;
        s.queue_notify.notify_one();
    }
    until(&session, |s| {
        s.command_queue.is_empty() && s.commands_taken == 0
    })
    .await;
}

fn texts(s: &ChatSession) -> Vec<(String, String)> {
    s.messages
        .iter()
        .map(|m| (m.message_id.clone(), m.content.content_text_only()))
        .collect()
}

#[tokio::test]
async fn the_patch_is_queued_and_applied_after_the_turn() {
    let (_dir, app, _) = fixture().await;
    running_turn(&app).await;
    let (request, out) = approve_call(&app, &format!("({EDIT_FIRST})['data']")).await;
    let card = request.engine.unwrap();
    assert_eq!(card.title, "rewrite this chat's history");
    assert_eq!(card.scope.as_deref(), Some("this chat, after this turn"));
    assert_eq!(card.class, Class::Destructive);
    assert_eq!(card.items[0].old, Some(json!("first question")));
    assert_eq!(card.items[0].new, Some(json!("edited question")));
    assert_eq!(out["applies"], "after_this_turn", "{out}");
    let request_id = out["request_id"].as_str().unwrap().to_string();
    assert!(request_id.starts_with("engine-"), "{out}");
    {
        let session = session(&app).await;
        let s = session.lock().await;
        assert_eq!(s.messages[0].content.content_text_only(), "first question");
        assert_eq!(s.command_queue.len(), 1);
        assert_eq!(
            serde_json::to_value(&s.command_queue[0].command).unwrap()["type"],
            "patch_messages"
        );
    }
    end_turn(&app).await;
    let session = session(&app).await;
    let s = session.lock().await;
    assert_eq!(
        texts(&s),
        [
            ("cp-m0", "edited question"),
            ("cp-m1", "first answer"),
            ("cp-m2", "second question"),
            ("cp-m3", ""),
            ("cp-m4", "python output"),
            ("cp-m5", "done"),
        ]
        .map(|(id, text)| (id.to_string(), text.to_string()))
    );
    assert_eq!(
        s.messages[0].extra[refact_chat_api::DELIVERY_EXTRA_KEY]["source"],
        "engine.chat.patch"
    );
}

#[derive(Clone, Copy, Debug)]
enum Between {
    Edit,
    Turn,
    Compaction,
    Handoff,
}

/// K5: a change between the approval and the end of the turn drops the patch visibly: nothing is
/// applied, the chat shows why, and its request is acknowledged as not accepted.
#[tokio::test]
async fn a_change_before_the_patch_applies_drops_it_visibly() {
    for (between, reason) in [
        (
            Between::Edit,
            "the messages before the turn that queued it changed",
        ),
        (
            Between::Turn,
            "another turn of this chat started after the turn that queued it",
        ),
        (
            Between::Compaction,
            "this chat was compacted after it was queued",
        ),
        (Between::Handoff, "a mode handoff of this chat is pending"),
    ] {
        let (_dir, app, _) = fixture().await;
        running_turn(&app).await;
        let mut events = session(&app).await.lock().await.subscribe();
        let (_, out) = approve_call(&app, &format!("({EDIT_FIRST})['data']")).await;
        let request_id = out["request_id"].as_str().unwrap().to_string();
        match between {
            Between::Edit => {
                session(&app).await.lock().await.messages[0] =
                    message("cp-m0", "user", "moved question");
            }
            Between::Turn => {
                tokio::time::sleep(Duration::from_millis(5)).await;
                latch(
                    &app,
                    "typed-again",
                    json!({"type":"user_message","content":"again"}),
                )
                .await;
            }
            Between::Compaction => {
                let mut report = message("cp-r", "compression_report", "summary");
                report.extra.insert(
                    "compression_report".into(),
                    json!({"kind": "chat_compression_report"}),
                );
                session(&app).await.lock().await.messages.push(report);
            }
            Between::Handoff => {
                session(&app).await.lock().await.pending_mode_handoff =
                    Some(json!({"to": "agent"}));
            }
        }
        end_turn(&app).await;
        let session = session(&app).await;
        let s = session.lock().await;
        assert_ne!(
            s.messages[0].content.content_text_only(),
            "edited question",
            "{between:?}: applied"
        );
        let shown = format!("chat.patch was not applied: {reason}");
        assert!(
            s.messages
                .iter()
                .any(|m| m.role == "error" && m.content.content_text_only() == shown),
            "{between:?}: {:?}",
            texts(&s)
        );
        let mut refused = false;
        while let Ok(event) = events.try_recv() {
            let event: Value = serde_json::from_str(&event).unwrap();
            refused |= event["type"] == "ack"
                && event["client_request_id"] == request_id
                && event["accepted"] == false;
        }
        assert!(refused, "{between:?}: no refusal ack");
    }
}

#[tokio::test]
async fn messages_of_the_running_turn_and_moving_anchors_are_refused() {
    let (_dir, app, _) = fixture().await;
    running_turn(&app).await;
    let (kind, text) = refused_call(
        &app,
        "await engine('chat.patch', ops=[{'op':'edit','message_id':'cp-m3','text':'x'}])",
    )
    .await;
    assert_eq!(kind, "PermissionError", "{text}");
    assert!(text.contains("running turn"), "{text}");
    let (kind, text) = refused_call(
        &app,
        "await engine('chat.patch', ops=[{'op':'remove','message_id':'cp-m2','through':'cp-m3'}])",
    )
    .await;
    assert_eq!(kind, "PermissionError", "{text}");
    for anchor in ["start", "end"] {
        let (kind, text) = refused_call(
            &app,
            &format!("await engine('chat.patch', ops=[{{'op':'insert','after':'{anchor}','role':'user','text':'x'}}])"),
        )
        .await;
        assert_eq!(kind, "TypeError", "{anchor}: {text}");
    }
    assert!(session(&app).await.lock().await.command_queue.is_empty());
}

#[tokio::test]
async fn deny_queues_nothing() {
    let (_dir, app, _) = fixture().await;
    running_turn(&app).await;
    let before = texts(&*session(&app).await.lock().await);
    let task = spawn_call(&app, EDIT_FIRST);
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    let session = session(&app).await;
    let s = session.lock().await;
    assert!(s.command_queue.is_empty());
    assert_eq!(texts(&s), before);
}

#[tokio::test]
async fn child_and_agent_chats_are_refused() {
    for (agent, refusal) in [(false, "agent runner"), (true, "background agent chat")] {
        let (_dir, app, ccx) = fixture().await;
        running_turn(&app).await;
        if agent {
            ccx.lock().await.background_agent_id = Some("bgagent-x".into());
        } else {
            ccx.lock().await.subchat_depth = 1;
        }
        let m = run_ccx(
            ccx,
            &format!("try:\n    {EDIT_FIRST}\n    r = 'ran'\nexcept PermissionError as e:\n    r = str(e)\nr"),
        )
        .await;
        assert!(
            result(&m).as_str().unwrap().contains(refusal),
            "{}",
            result(&m)
        );
        assert!(session(&app).await.lock().await.command_queue.is_empty());
    }
}

/// D4: the command route refuses a patch command at decode time, before anything is queued.
#[tokio::test]
async fn the_command_route_refuses_patch_messages() {
    let (_dir, app, _) = fixture().await;
    let body = json!({
        "client_request_id": "route-patch",
        "type": "patch_messages",
        "patch": {"ops": [], "prefix": "h1-x", "tail": 0, "turn_at_ms": 0, "items": "x"}
    });
    let refusal = crate::chat::handle_v1_chat_command(
        axum::extract::State(app.clone()),
        axum::extract::Path("python-test".to_string()),
        hyper::body::Bytes::from(body.to_string()),
    )
    .await
    .err()
    .expect("the route accepted a patch command");
    assert_eq!(refusal.status_code, axum::http::StatusCode::BAD_REQUEST);
    assert!(session(&app).await.lock().await.command_queue.is_empty());
}
