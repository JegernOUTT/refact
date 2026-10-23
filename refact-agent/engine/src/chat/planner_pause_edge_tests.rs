use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use refact_buddy_core::snapshot::BuddySnapshot;
use refact_buddy_core::types::{BuddyRuntimeEvent, BuddySuggestion};
use refact_chat_api::{ChatCommand, CommandRequest};
use refact_runtime_api::BuddyEventSink;
use refact_tool_api::{MatchConfirmDeny, MatchConfirmDenyResult, ToolDesc, ToolSource, ToolSourceType};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::app_state::AppToolRegistry;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatToolCall, ChatToolFunction, ContextEnum};
use crate::chat::tools::{process_tool_calls_once, ToolStepOutcome};
use crate::chat::types::{EnqueueCommandOutcome, SessionState};
use crate::tasks::board_pause_tests::{fixture, pause, resume, Fixture};
use crate::tools::tools_description::Tool;

#[derive(Default)]
struct EventSink {
    events: Mutex<Vec<BuddyRuntimeEvent>>,
}

#[async_trait]
impl BuddyEventSink for EventSink {
    async fn enqueue_event(&self, event: BuddyRuntimeEvent) {
        self.events.lock().await.push(event);
    }

    async fn complete_event(&self, _dedupe_key: &str, _status: &str) {}

    async fn snapshot(&self) -> Option<BuddySnapshot> {
        None
    }

    async fn apply_chat_completion(&self, event: BuddyRuntimeEvent, _xp: u64, _mood: String) {
        self.enqueue_event(event).await;
    }

    async fn report_error(
        &self,
        _error_type: &str,
        _error_msg: &str,
        _source: Option<&str>,
        _chat_id: Option<&str>,
    ) {
    }

    async fn mark_chat_error(&self, event: BuddyRuntimeEvent) {
        self.enqueue_event(event).await;
    }

    async fn maybe_add_suggestion(&self, _suggestion: BuddySuggestion) {}

    async fn build_pulse_message(&self) -> Option<ChatMessage> {
        None
    }

    async fn render_runtime_event_fast(
        &self,
        _workflow_id: &str,
        _workflow_summary: &str,
        _status: &str,
    ) -> Option<(String, Option<String>)> {
        None
    }
}

struct CountingTool {
    calls: Arc<AtomicUsize>,
    needs_confirmation: bool,
}

#[async_trait]
impl Tool for CountingTool {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "pause_edge_tool".into(),
            experimental: false,
            allow_parallel: false,
            description: "Planner pause test tool".into(),
            input_schema: json!({"type": "object", "properties": {}}),
            output_schema: None,
            annotations: None,
            display_name: "Planner pause test tool".into(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<Mutex<AtCommandsContext>>,
        tool_call_id: &String,
        _args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ccx.lock().await.abort_flag.store(true, Ordering::SeqCst);
        let mut result = ChatMessage::new("tool".into(), "executed".into());
        result.tool_call_id = tool_call_id.clone();
        Ok((false, vec![ContextEnum::ChatMessage(result)]))
    }

    async fn match_against_confirm_deny(
        &self,
        _ccx: Arc<Mutex<AtCommandsContext>>,
        _args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        Ok(MatchConfirmDeny {
            result: if self.needs_confirmation {
                MatchConfirmDenyResult::CONFIRMATION
            } else {
                MatchConfirmDenyResult::PASS
            },
            command: "pause_edge_tool".into(),
            rule: "planner pause test approval".into(),
        })
    }
}

async fn parked_fixture(needs_confirmation: bool) -> (Fixture, Arc<AtomicUsize>, Arc<EventSink>) {
    let mut f = fixture(SessionState::Idle).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let tool_calls = calls.clone();
    f.app.tool_registry = Arc::new(AppToolRegistry::with_fixture_tool_factory(
        f.app.gcx.clone(),
        Arc::new(move || {
            vec![Box::new(CountingTool {
                calls: tool_calls.clone(),
                needs_confirmation,
            })]
        }),
    ));
    let events = Arc::new(EventSink::default());
    f.app.buddy_event_sink = events.clone();
    {
        let mut session = f.agent.lock().await;
        session.thread.title = "Paused batch".into();
        session.thread.mode = "agent".into();
        session.add_message(ChatMessage {
            role: "assistant".into(),
            tool_calls: Some(vec![ChatToolCall {
                id: "parked-tool".into(),
                index: None,
                function: ChatToolFunction {
                    name: "pause_edge_tool".into(),
                    arguments: "{}".into(),
                },
                tool_type: "function".into(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            }]),
            ..Default::default()
        });
    }
    pause(&f).await;
    assert!(matches!(
        process_tool_calls_once(f.app.clone(), f.agent.clone(), "agent", None).await,
        ToolStepOutcome::Stop
    ));
    assert!(f.agent.lock().await.planner_pause_pending_tools);
    (f, calls, events)
}

#[tokio::test]
async fn queued_abort_on_paused_agent_cancels_parked_batch() {
    let (f, calls, _) = parked_fixture(false).await;
    let running = {
        let mut session = f.agent.lock().await;
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert_eq!(
            session.enqueue_accepted_priority_command(CommandRequest {
                client_request_id: "queued-abort".into(),
                priority: true,
                command: ChatCommand::Abort {},
                native_approval_guard: None,
                supervision_guard: None,
            }),
            EnqueueCommandOutcome::Accepted
        );
        assert!(!session.abort_flag.load(Ordering::SeqCst));
        session.queue_processor_running.clone()
    };
    let worker = tokio::spawn(crate::chat::process_command_queue(
        f.app.clone(),
        f.agent.clone(),
        running.clone(),
    ));
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let session = f.agent.lock().await;
            if session.command_queue.is_empty() && session.abort_flag.load(Ordering::SeqCst) {
                break;
            }
            drop(session);
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("queued Abort did not drain");
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(30), async {
        while running.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    running.store(true, Ordering::SeqCst);
    assert!(f.agent.lock().await.planner_paused());

    resume(&f).await;

    let replay_queued = {
        let session = f.agent.lock().await;
        assert!(!session.planner_paused());
        let replay_queued = session.planner_pause_pending_tools;
        if replay_queued {
            session.abort_flag.store(false, Ordering::SeqCst);
            session.user_interrupt_flag.store(false, Ordering::SeqCst);
        }
        replay_queued
    };
    if replay_queued {
        tokio::time::timeout(
            Duration::from_secs(30),
            crate::chat::start_generation(f.app.clone(), f.agent.clone()),
        )
        .await
        .expect("parked tool replay did not stop");
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "aborted tool batch replayed"
    );
    let session = f.agent.lock().await;
    assert!(!session.planner_pause_pending_tools);
    assert!(session.messages.iter().all(|message| message
        .tool_calls
        .as_ref()
        .is_none_or(|calls| calls.iter().all(|call| call.id != "parked-tool"))));
}

#[tokio::test]
async fn resumed_batch_needing_confirmation_emits_waiting_for_approval() {
    let (f, calls, events) = parked_fixture(true).await;
    resume(&f).await;
    f.agent.lock().await.thread.task_meta = None;

    tokio::time::timeout(
        Duration::from_secs(30),
        crate::chat::start_generation(f.app.clone(), f.agent.clone()),
    )
    .await
    .expect("confirmation replay did not stop");

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let session = f.agent.lock().await;
    assert_eq!(session.runtime.state, SessionState::Paused);
    assert!(session.runtime.paused);
    assert!(!session.planner_pause_pending_tools);
    assert_eq!(session.runtime.pause_reasons.len(), 1);
    let reason = &session.runtime.pause_reasons[0];
    assert_eq!(reason.tool_call_id, "parked-tool");
    assert_eq!(reason.tool_name, "pause_edge_tool");
    assert_eq!(reason.reason_type, "confirmation");
    assert_eq!(reason.rule, "planner pause test approval");
    let events = events.events.lock().await;
    let approval_events: Vec<_> = events
        .iter()
        .filter(|event| event.signal_type == "tool_confirmation")
        .collect();
    assert_eq!(
        approval_events.len(),
        1,
        "missing Waiting for approval event"
    );
    let event = approval_events[0];
    assert_eq!(event.title, "Waiting for approval: Paused batch");
    assert_eq!(event.status, "paused");
    assert_eq!(event.chat_id.as_deref(), Some(session.chat_id.as_str()));
    assert!(event.persistent);
}
