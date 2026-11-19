use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::tools::tool_python::tests::{call, execute, fixture, with_tools};
use refact_chat_api::{
    ChatCommand, ChatEvent, CommandRequest, EventEnvelope, PythonApprovalDecisionKind,
    PythonRunControlAction,
};
use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;
use tokio::sync::broadcast;

const CHAT: &str = "python-test";

#[tokio::test]
async fn preempting_an_attached_python_turn_does_not_deliver_or_wake_later() {
    use crate::at_commands::at_commands::AtCommandsContext;
    use crate::call_validation::ContextEnum;
    use crate::tools::tool_python::ToolPython;
    use crate::tools::tools_description::Tool;
    use axum::{routing::post, Json, Router};
    use refact_chat_api::{PythonRunUpdate, SessionState};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use tokio::sync::{Mutex, Notify, Semaphore};
    use tokio_util::task::AbortOnDropHandle;

    const SENDER: &str = "python-preempt-sender";
    const MODEL: &str = "test/python-preempt";
    const OLD: &str = "preempted-python";
    const URGENT: &str = "urgent replacement request";
    const REPLY: &str = "urgent reply received";
    const LATE: &str = "old run must not land";

    struct LocalWait(Arc<Notify>, Arc<Semaphore>);
    #[async_trait::async_trait]
    impl Tool for LocalWait {
        async fn tool_execute(
            &mut self,
            _: Arc<Mutex<AtCommandsContext>>,
            id: &String,
            _: &HashMap<String, Value>,
        ) -> Result<(bool, Vec<ContextEnum>), String> {
            self.0.notify_one();
            self.1.acquire().await.unwrap().forget();
            let mut message = ChatMessage::new("tool".into(), LATE.into());
            message.tool_call_id = id.clone();
            Ok((false, vec![ContextEnum::ChatMessage(message)]))
        }

        fn tool_description(&self) -> refact_tool_api::ToolDesc {
            let mut desc = ToolPython::default().tool_description();
            desc.name = "local_wait".into();
            desc.input_schema = json!({"type":"object","additionalProperties":false});
            desc
        }
    }

    let (_dir, mut app, _) = fixture().await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Semaphore::new(0));
    app.tool_registry = Arc::new(
        crate::app_state::AppToolRegistry::with_fixture_tool_factory(app.gcx.clone(), {
            let (entered, release) = (entered.clone(), release.clone());
            Arc::new(move || {
                vec![
                    Box::new(ToolPython::default()),
                    Box::new(LocalWait(entered.clone(), release.clone())),
                ]
            })
        }),
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let router = Router::new().route(
        "/v1/chat/completions",
        post({
            let requests = requests.clone();
            move |Json(body): Json<Value>| {
                let requests = requests.clone();
                async move {
                    let mut requests = requests.lock().await;
                    requests.push(body);
                    let (delta, finish) = match requests.len() {
                        1 => {
                            let mut tool_call = serde_json::to_value(call(
                                OLD,
                                "r = await local_wait()\nr['text']",
                                false,
                            ))
                            .unwrap();
                            tool_call["index"] = json!(0);
                            (
                                json!({"role":"assistant","tool_calls":[tool_call]}),
                                "tool_calls",
                            )
                        }
                        2 => (json!({"role":"assistant","content":REPLY}), "stop"),
                        _ => (
                            json!({"role":"assistant","content":"unexpected extra wake"}),
                            "stop",
                        ),
                    };
                    let chunk = json!({"choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                    let terminal =
                        json!({"choices":[{"index":0,"delta":{},"finish_reason":finish}]});
                    (
                        [("content-type", "text/event-stream")],
                        format!("data: {chunk}\n\ndata: {terminal}\n\ndata: [DONE]\n\n"),
                    )
                }
            }
        }),
    );
    let _server = AbortOnDropHandle::new(tokio::spawn(
        axum::Server::from_tcp(listener)
            .unwrap()
            .serve(router.into_make_service()),
    ));

    *app.gcx.privacy_policy_load.write().unwrap() = refact_privacy::PolicyLoad::default();
    let (project_root, _) = crate::caps::project_model_defaults_state(app.gcx.clone()).await;
    let mut caps = crate::caps::CodeAssistantCaps::default();
    caps.chat_models.insert(
        MODEL.into(),
        Arc::new(crate::caps::ChatModelRecord {
            base: crate::caps::BaseModelRecord {
                id: MODEL.into(),
                name: "python-preempt".into(),
                endpoint: format!("http://{address}/v1/chat/completions"),
                wire_format: crate::llm::WireFormat::OpenaiChatCompletions,
                tokenizer: "fake".into(),
                n_ctx: 8192,
                enabled: true,
                ..Default::default()
            },
            supports_agent: true,
            supports_tools: true,
            ..Default::default()
        }),
    );
    caps.defaults.chat_default_model = MODEL.into();
    {
        let mut state = app.model.caps.write().await;
        state.caps = Some(Arc::new(caps));
        state.loaded_project_root = Some(project_root.clone());
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let target = app.chat.sessions.read().await[CHAT].clone();
    let mut sender = crate::chat::types::ChatSession::new(SENDER.into());
    let mut target_events = {
        let mut target = target.lock().await;
        for chat in [&mut *target, &mut sender] {
            chat.thread.mode = "agent".into();
            chat.thread.model = MODEL.into();
            chat.work_project_root = project_root.clone();
            chat.thread.include_project_info = false;
            chat.thread.checkpoints_enabled = false;
            chat.add_message(ChatMessage::new("system".into(), "system".into()));
        }
        target.add_message(ChatMessage::new("user".into(), "start the old run".into()));
        target.subscribe()
    };
    let mut target_log = target_events.resubscribe();
    let mut sender_events = sender.subscribe();
    let typed = serde_json::from_value(json!({
        "client_request_id":"typed", "type":"user_message", "content":"send urgent"
    }))
    .unwrap();
    crate::chat::queue::latch_turn_origin(&mut sender, &typed);
    let sender_thread = sender.thread.clone();
    app.chat
        .sessions
        .write()
        .await
        .insert(SENDER.into(), Arc::new(Mutex::new(sender)));

    let old_turn = AbortOnDropHandle::new(tokio::spawn(crate::chat::start_generation(
        app.clone(),
        target.clone(),
    )));
    tokio::time::timeout(Duration::from_secs(10), entered.notified())
        .await
        .expect("the old turn never reached the nested wait");
    let jobs = app.runtime.tool_jobs.clone();
    let old = jobs
        .list(Some(CHAT))
        .into_iter()
        .find(|(job, _)| job.receipt.binding.tool_call_id == OLD)
        .unwrap()
        .0;
    assert_eq!(old.status, crate::tool_jobs::JobStatus::Running);
    assert_eq!(
        target.lock().await.runtime.state,
        SessionState::ExecutingTools
    );

    let script = format!(
        "(await engine('chats.send', chat_id={CHAT:?}, text={URGENT:?}, push='preempt'))['data']"
    );
    let sender_run = AbortOnDropHandle::new(tokio::spawn({
        let app = app.clone();
        async move {
            crate::chat::tools::execute_tools(
                app,
                &[call("send-preempt", &script, false)],
                &[],
                &sender_thread,
                "agent",
                Some(MODEL),
                Default::default(),
            )
            .await
            .0
        }
    }));
    let approval = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let raw = sender_events.recv().await.unwrap();
            let envelope: EventEnvelope = serde_json::from_str(&raw).unwrap();
            assert_eq!(envelope.chat_id, SENDER);
            if let ChatEvent::PythonApprovalRequested {
                request,
            } = envelope.event
            {
                break request;
            }
        }
    })
    .await
    .expect("the sender did not request approval");
    assert_eq!(approval.tool_name, "engine.chats.send");
    assert!(approval.needs_person);
    app.python_tool
        .approvals
        .decide(
            &app,
            SENDER,
            &approval.request_id,
            PythonApprovalDecisionKind::AllowOnce,
        )
        .await
        .unwrap();
    let messages = tokio::time::timeout(Duration::from_secs(10), sender_run)
        .await
        .unwrap()
        .unwrap();
    let [sent] = messages.as_slice() else {
        panic!("expected one sender result: {messages:?}")
    };
    assert_eq!(sent.extra["python"]["status"], "completed", "{sent:?}");
    let result: Value = serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(sent.extra["python"]["run_dir"].as_str().unwrap())
                .join("result.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(result["chat_id"], CHAT);
    assert_eq!(result["outcome"], "delivered");
    assert_eq!(result["queued"], false);
    assert_eq!(result["opened"], false);

    next_chat_event(
        &mut target_events,
        "the urgent reply",
        |event| match event {
            ChatEvent::MessageAdded {
                message, ..
            } if message.role == "assistant" && message.content.content_text_only() == REPLY => {
                Some(())
            }
            _ => None,
        },
    )
    .await;
    tokio::time::timeout(Duration::from_secs(10), old_turn)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        jobs.get(CHAT, &old.receipt.job_id).unwrap().status,
        crate::tool_jobs::JobStatus::CancelRequested,
        "the original attachment must request cancellation while the nested call is still held",
    );
    release.add_permits(1);
    let terminal = tokio::time::timeout(
        Duration::from_secs(10),
        jobs.wait_terminal(CHAT, &old.receipt.job_id),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(terminal.status, crate::tool_jobs::JobStatus::Unknown);
    let result = terminal.result.unwrap();
    assert_eq!(result.messages[0].extra["python"]["status"], "cancelled");
    assert!(jobs.background_terminal_refs().is_empty());
    tokio::time::timeout(Duration::from_secs(10), jobs.shutdown())
        .await
        .unwrap();
    app.python_tool.registry.shutdown_all().await;

    let target = target.lock().await;
    assert_eq!(
        target
            .messages
            .iter()
            .filter(|m| m.role == "user" && m.content.content_text_only() == URGENT)
            .count(),
        1
    );
    assert_eq!(
        target
            .messages
            .iter()
            .filter(|m| m.role == "assistant" && m.content.content_text_only() == REPLY)
            .count(),
        1
    );
    assert!(target.messages.iter().all(|m| completion_job(m).is_none()
        && m.extra.get("tool_job").is_none()
        && !m.content.content_text_only().contains(LATE)));
    assert!(target
        .pending_deliveries
        .iter()
        .all(|delivery| delivery.deferred_job.is_none()));
    assert!(target.delivery_wake_sources.is_empty());
    drop(target);
    let requests = requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| {
            message["role"] == "user"
                && (message["content"] == URGENT
                    || message["content"]
                        .as_array()
                        .is_some_and(|parts| parts.iter().any(|part| part["text"] == URGENT)))
        }));
    loop {
        let raw = match target_log.try_recv() {
            Ok(raw) => raw,
            Err(broadcast::error::TryRecvError::Empty) => break,
            Err(error) => panic!("target event log not available: {error}"),
        };
        let event = serde_json::from_str::<EventEnvelope>(&raw).unwrap().event;
        assert!(
            !matches!(event, ChatEvent::PythonRunUpdate { tool_call_id, update: PythonRunUpdate::Detached {}, .. } if tool_call_id == OLD)
        );
    }
}

async fn configure_fixture_confirmations(app: &AppState) {
    let mut registry =
        crate::yaml_configs::customization_registry::get_project_registry(app.gcx.clone())
            .await
            .unwrap();
    registry.modes.get_mut("agent").unwrap().tool_confirm.rules = vec![
        crate::yaml_configs::customization_types::ToolConfirmRule {
            match_pattern: "fixture".into(),
            action: "ask".into(),
        },
        crate::yaml_configs::customization_types::ToolConfirmRule {
            match_pattern: "serial_fixture".into(),
            action: "deny".into(),
        },
    ];
    let root = crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .remove(0);
    app.gcx
        .project_registry_cache
        .write()
        .unwrap()
        .insert(root, registry);
    app.gcx.tool_catalog_generations.advance_customization();
}

#[tokio::test]
async fn parallel_artifact_deny_approval_and_session_reuse() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    configure_fixture_confirmations(&app).await;
    let script = "import asyncio\nimport json\nfrom pathlib import Path\ntry:\n    await serial_fixture()\nexcept PermissionError:\n    denied = True\nresults = await asyncio.gather(fixture(), fixture(), fixture())\nPath(ARTIFACTS_DIR + '/out.json').write_text(json.dumps(results))\nanswer = 42\nanswer";
    let task = {
        let app = app.clone();
        tokio::spawn(async move { execute(app, &[call("e2e", script, false)]).await })
    };
    tokio::time::timeout(Duration::from_secs(20), async {
        while !task.is_finished() {
            let session = app.chat.sessions.read().await[CHAT].clone();
            let requests = session.lock().await.runtime.python_approvals.clone();
            for request in requests {
                let command = CommandRequest {
                    native_approval_guard: None,
                    supervision_guard: None,
                    client_request_id: uuid::Uuid::new_v4().to_string(),
                    priority: false,
                    command: ChatCommand::PythonApprovalDecision {
                        request_id: request.request_id,
                        decision: PythonApprovalDecisionKind::AllowOnce,
                    },
                };
                crate::chat::handle_v1_chat_command(
                    axum::extract::State(app.clone()),
                    axum::extract::Path(CHAT.into()),
                    serde_json::to_vec(&command).unwrap().into(),
                )
                .await
                .unwrap();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let messages = task.await.unwrap();
    assert_eq!(messages[0].tool_failed, Some(false), "{:?}", messages[0]);
    let workspace = super::workspace::PythonWorkspace::for_chat(&app, CHAT)
        .await
        .unwrap();
    assert!(workspace.artifacts_dir().join("out.json").exists());
    let messages = execute(app.clone(), &[call("reuse", "answer", false)]).await;
    assert!(messages[0].content.content_text_only().contains("42"));
}

async fn next_chat_event<T>(
    events: &mut broadcast::Receiver<Arc<String>>,
    expected: &str,
    select: impl Fn(ChatEvent) -> Option<T>,
) -> T {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let raw = events.recv().await.expect("chat event subscription lost");
            let envelope: EventEnvelope = serde_json::from_str(&raw).unwrap();
            assert_eq!(envelope.chat_id, CHAT);
            if let Some(value) = select(envelope.event) {
                return value;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("chat did not emit {expected}"))
}

fn completion_job(message: &ChatMessage) -> Option<&str> {
    let meta = message.extra.get("tool_job_result");
    if meta.is_some()
        || message
            .extra
            .get("event")
            .is_some_and(|event| event["source"] == "tool_job.completed")
    {
        Some(
            meta.and_then(|meta| meta["job_id"].as_str())
                .expect("chat completion must identify its job"),
        )
    } else {
        None
    }
}

#[tokio::test]
async fn detached_python_delivers_once_without_polling_and_public_stop_delivers_cancelled() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    configure_fixture_confirmations(&app).await;

    let (project_root, _) = crate::caps::project_model_defaults_state(app.gcx.clone()).await;
    let mut caps = crate::caps::CodeAssistantCaps::default();
    caps.chat_models.insert(
        "model".into(),
        Arc::new(crate::caps::ChatModelRecord {
            base: crate::caps::BaseModelRecord {
                id: "model".into(),
                ..Default::default()
            },
            supports_agent: true,
            supports_tools: true,
            ..Default::default()
        }),
    );
    {
        let mut state = app.gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.loaded_project_root = Some(project_root.clone());
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let session = app.chat.sessions.read().await[CHAT].clone();
    let mut events = {
        let mut session = session.lock().await;
        session.thread.mode = "agent".into();
        session.thread.model = "model".into();
        session.work_project_root = project_root;
        session
            .queue_processor_running
            .store(true, Ordering::SeqCst);
        session.subscribe()
    };
    app.runtime
        .tool_jobs
        .set_test_inline_budget(Some(Duration::from_millis(10)));
    let mut chat_log = events.resubscribe();
    let mut jobs: Vec<String> = Vec::new();
    for (call_id, stop) in [("detach-complete", false), ("detach-stop", true)] {
        let tool_call = call(
            call_id,
            "r = await fixture()\nr['data']['answer'] + 1",
            false,
        );
        session.lock().await.add_message(ChatMessage {
            role: "assistant".into(),
            tool_calls: Some(vec![tool_call.clone()]),
            ..Default::default()
        });
        let messages = execute(app.clone(), &[tool_call]).await;
        let [receipt] = messages.as_slice() else {
            panic!("expected one detach receipt, got {messages:?}");
        };
        assert_eq!(receipt.role, "tool");
        assert_eq!(receipt.tool_call_id, call_id);
        assert_eq!(receipt.tool_failed, None);
        let meta = &receipt.extra["tool_job"];
        assert_eq!(meta["status"], "accepted");
        let job = meta["job_id"].as_str().unwrap();
        assert!(!job.is_empty() && !jobs.iter().any(|seen| seen.as_str() == job));
        session.lock().await.add_message(receipt.clone());

        let request =
            next_chat_event(
                &mut events,
                "the pending fixture approval",
                |event| match event {
                    ChatEvent::PythonApprovalRequested {
                        request,
                    } if request.tool_call_id == call_id => Some(request),
                    _ => None,
                },
            )
            .await;
        assert_eq!(request.tool_name, "fixture");
        {
            let locked = session.lock().await;
            assert!(locked.runtime.python_approvals.iter().any(|pending| {
                pending.request_id == request.request_id && pending.run_id == request.run_id
            }));
            assert!(!locked
                .messages
                .iter()
                .any(|message| completion_job(message) == Some(job)));
        }
        let command = CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: uuid::Uuid::new_v4().to_string(),
            priority: false,
            command: if stop {
                ChatCommand::PythonRunControl {
                    tool_call_id: call_id.into(),
                    action: PythonRunControlAction::Stop,
                }
            } else {
                ChatCommand::PythonApprovalDecision {
                    request_id: request.request_id,
                    decision: PythonApprovalDecisionKind::AllowOnce,
                }
            },
        };
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            crate::chat::handle_v1_chat_command(
                axum::extract::State(app.clone()),
                axum::extract::Path(CHAT.into()),
                serde_json::to_vec(&command).unwrap().into(),
            ),
        )
        .await
        .expect("Python command did not return")
        .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let terminal =
            next_chat_event(
                &mut events,
                "the detached Python result",
                |event| match event {
                    ChatEvent::MessageAdded {
                        message, ..
                    } if completion_job(&message) == Some(job) => Some(message),
                    _ => None,
                },
            )
            .await;
        assert_eq!(terminal.role, "event");
        assert_eq!(terminal.extra["event"]["source"], "tool_job.completed");
        assert_eq!(terminal.extra["tool_job_result"]["tool_call_id"], call_id);
        assert_eq!(terminal.extra["tool_job_result"]["tool_name"], "python");
        assert_eq!(terminal.tool_failed, Some(stop));
        let python = &terminal.extra["python"];
        assert_eq!(python["run_id"], request.run_id);
        assert_eq!(python["background"], true);
        assert_eq!(
            python["status"],
            if stop { "cancelled" } else { "completed" }
        );
        if stop {
            assert_eq!(python["error"]["type"], "CancelledError");
            assert_eq!(python["result_preview"], "");
        } else {
            assert_eq!(python["result_preview"], "43");
        }
        jobs.push(job.to_owned());
    }

    tokio::time::timeout(Duration::from_secs(10), app.runtime.tool_jobs.shutdown())
        .await
        .expect("tool jobs and completion bridge did not shut down");
    app.python_tool.registry.shutdown_all().await;
    let mut emitted = Vec::new();
    loop {
        let raw = match chat_log.try_recv() {
            Ok(raw) => raw,
            Err(broadcast::error::TryRecvError::Empty) => break,
            Err(error) => panic!("chat event subscription lost: {error}"),
        };
        let envelope: EventEnvelope = serde_json::from_str(&raw).unwrap();
        if let ChatEvent::MessageAdded {
            message, ..
        } = envelope.event
        {
            if let Some(job) = completion_job(&message) {
                emitted.push(job.to_owned());
            }
        }
    }
    assert_eq!(emitted, jobs);
    let locked = session.lock().await;
    let landed: Vec<_> = locked.messages.iter().filter_map(completion_job).collect();
    assert_eq!(landed, jobs);
}
