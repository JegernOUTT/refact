use std::time::Instant;

use serde::Serialize;

use crate::app_state::AppState;
use super::types::{ChatCommand, ChatSession, CommandRequest, SessionState};

const SESSION_LIMIT: usize = 256;
const ENTRY_LIMIT: usize = 16_384;

#[derive(Default)]
pub(crate) struct RuntimeDiagnostics {
    pub queue_progress: Option<Instant>,
    pub chat_progress: Option<Instant>,
    pub accepted_at: Option<Instant>,
    pub accepted_count: u64,
    pub phase: Option<(&'static str, Instant)>,
}

impl RuntimeDiagnostics {
    pub fn accept(&mut self, now: Instant) {
        if self.accepted_at.is_none() {
            self.chat_progress = Some(now);
        }
        self.accepted_at.get_or_insert(now);
        self.accepted_count = self.accepted_count.saturating_add(1);
        self.phase.get_or_insert(("accepted", now));
    }

    pub fn phase(&mut self, phase: &'static str, now: Instant) {
        if self.accepted_at.is_some() {
            self.phase = Some((phase, now));
        }
    }

    pub fn clear_accepted(&mut self) {
        self.accepted_at = None;
        self.accepted_count = 0;
        self.phase = None;
    }

    pub fn first_output(&mut self, now: Instant) {
        self.chat_progress = Some(now);
        if let Some(started) = self.accepted_at {
            super::perf_diagnostics::record(
                super::perf_diagnostics::PerfComponent::MessageAcceptedToFirstDelta,
                None,
                super::perf_diagnostics::PerfOutcome::Success,
                now.saturating_duration_since(started)
                    .as_micros()
                    .try_into()
                    .unwrap_or(u64::MAX),
                None,
                None,
                None,
            );
        }
        self.clear_accepted();
    }
}

#[derive(Debug, Default, Serialize)]
pub struct PendingAggregate {
    pending_count: u64,
    oldest_pending_age_ms: Option<u64>,
    unknown_age_count: u64,
}

impl PendingAggregate {
    fn add(&mut self, age: Option<u64>, count: u64) {
        self.pending_count += count;
        if let Some(age) = age {
            self.oldest_pending_age_ms = Some(self.oldest_pending_age_ms.unwrap_or(0).max(age));
        } else {
            self.unknown_age_count += count;
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Coverage {
    sessions_total: Option<usize>,
    inspected: usize,
    skipped_busy: usize,
    skipped_budget: usize,
    partial: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct Progress {
    oldest_queue_progress_age_ms: Option<u64>,
    oldest_chat_progress_age_ms: Option<u64>,
    unknown_queue_progress_sessions: usize,
    unknown_chat_progress_sessions: usize,
}

#[derive(Debug, Serialize)]
pub struct Blocker {
    reason: &'static str,
    #[serde(flatten)]
    pending: PendingAggregate,
}

#[derive(Debug, Serialize)]
pub struct Phase {
    phase: &'static str,
    #[serde(flatten)]
    pending: PendingAggregate,
}

#[derive(Debug, Default, Serialize)]
pub struct QueueDiagnostics {
    sampled_at_ms: u64,
    coverage: Coverage,
    commands: PendingAggregate,
    deliveries: PendingAggregate,
    question_answers: PendingAggregate,
    blockers: Vec<Blocker>,
    progress: Progress,
    accepted_waiting_first_output: PendingAggregate,
    phases: Vec<Phase>,
}

fn age(now: Instant, at: Instant) -> u64 {
    now.saturating_duration_since(at)
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn blocker(session: &ChatSession) -> &'static str {
    if session.runner_lease.is_some() {
        "runner_owned"
    } else if session.runtime.state == SessionState::Generating {
        "generating"
    } else if session.runtime.state == SessionState::ExecutingTools {
        "executing_tools"
    } else if session.turn_depth > 0 {
        "turn_in_flight"
    } else if session
        .pending_mode_handoff
        .as_ref()
        .is_some_and(|marker| marker.get("http_transition").is_some())
    {
        "mode_transition"
    } else if super::context_rebuild::compression_attempt_active(session) {
        "compression"
    } else if session.runtime.interruption.is_some() {
        "interrupted"
    } else if session.planner_paused() {
        "planner_paused"
    } else if session.runtime.state == SessionState::WaitingIde {
        "waiting_ide"
    } else if session.runtime.state == SessionState::Paused {
        "tool_confirmation"
    } else if session.pending_browser_message.is_some() {
        "browser_context"
    } else {
        "ready"
    }
}

fn command_blocker(session: &ChatSession, request: &CommandRequest) -> &'static str {
    let reason = blocker(session);
    let allowed = match reason {
        "interrupted" | "planner_paused" => matches!(request.command, ChatCommand::Abort {}),
        "waiting_ide" => matches!(
            request.command,
            ChatCommand::Abort {} | ChatCommand::IdeToolResult { .. }
        ),
        "tool_confirmation" => matches!(
            request.command,
            ChatCommand::Abort {}
                | ChatCommand::ToolDecision { .. }
                | ChatCommand::ToolDecisions { .. }
                | ChatCommand::AnswerQuestions { .. }
                | ChatCommand::SubmitQuestionResponse { .. }
        ),
        "browser_context" => {
            request.priority || !matches!(request.command, ChatCommand::UserMessage { .. })
        }
        _ => false,
    };
    if allowed {
        "ready"
    } else {
        reason
    }
}

impl QueueDiagnostics {
    fn pending(&mut self, reason: &'static str, age: Option<u64>) {
        let index = match self.blockers.iter().position(|item| item.reason == reason) {
            Some(index) => index,
            None => {
                self.blockers.push(Blocker {
                    reason,
                    pending: PendingAggregate::default(),
                });
                self.blockers.len() - 1
            }
        };
        self.blockers[index].pending.add(age, 1);
    }

    fn inspect(&mut self, session: &ChatSession, now: Instant, remaining: &mut usize) -> bool {
        if session.closed {
            return true;
        }
        let mut scanned = 0;
        for request in session.command_queue.iter().take(*remaining) {
            let age = session
                .command_enqueued_at
                .get(&request.client_request_id)
                .map(|at| age(now, *at));
            self.commands.add(age, 1);
            if matches!(
                request.command,
                ChatCommand::AnswerQuestions { .. } | ChatCommand::SubmitQuestionResponse { .. }
            ) {
                self.question_answers.add(age, 1);
            }
            self.pending(command_blocker(session, request), age);
            scanned += 1;
        }
        *remaining -= scanned;
        let mut delivery_scanned = 0;
        for delivery in session
            .pending_deliveries
            .iter()
            .chain(session.runner_pending_deliveries.iter())
            .take(*remaining)
        {
            let age = (delivery.enqueued_at_ms > 0)
                .then(|| self.sampled_at_ms.saturating_sub(delivery.enqueued_at_ms));
            self.deliveries.add(age, 1);
            self.pending(
                if session.runner_lease.is_some() {
                    "runner_owned"
                } else {
                    "delivery_boundary"
                },
                age,
            );
            delivery_scanned += 1;
        }
        *remaining -= delivery_scanned;
        let diagnostics = &session.live_diagnostics;
        if let Some(at) = diagnostics.accepted_at {
            self.accepted_waiting_first_output
                .add(Some(age(now, at)), diagnostics.accepted_count);
            if let Some((phase, at)) = diagnostics.phase {
                let index = match self.phases.iter().position(|item| item.phase == phase) {
                    Some(index) => index,
                    None => {
                        self.phases.push(Phase {
                            phase,
                            pending: PendingAggregate::default(),
                        });
                        self.phases.len() - 1
                    }
                };
                self.phases[index]
                    .pending
                    .add(Some(age(now, at)), diagnostics.accepted_count);
            }
        }
        if !session.command_queue.is_empty()
            || !session.pending_deliveries.is_empty()
            || !session.runner_pending_deliveries.is_empty()
            || diagnostics.accepted_at.is_some()
        {
            for (at, oldest, unknown) in [
                (
                    diagnostics.queue_progress,
                    &mut self.progress.oldest_queue_progress_age_ms,
                    &mut self.progress.unknown_queue_progress_sessions,
                ),
                (
                    diagnostics.chat_progress,
                    &mut self.progress.oldest_chat_progress_age_ms,
                    &mut self.progress.unknown_chat_progress_sessions,
                ),
            ] {
                if let Some(at) = at {
                    *oldest = Some(oldest.unwrap_or(0).max(age(now, at)));
                } else {
                    *unknown += 1;
                }
            }
        }
        scanned == session.command_queue.len()
            && delivery_scanned
                == session
                    .pending_deliveries
                    .len()
                    .saturating_add(session.runner_pending_deliveries.len())
    }
}

pub fn sample(app: &AppState) -> QueueDiagnostics {
    let now = Instant::now();
    let mut result = QueueDiagnostics {
        sampled_at_ms: chrono::Utc::now().timestamp_millis().max(0) as u64,
        ..Default::default()
    };
    let handles = match app.chat.sessions.try_read() {
        Ok(sessions) => {
            result.coverage.sessions_total = Some(sessions.len());
            result.coverage.skipped_budget = sessions.len().saturating_sub(SESSION_LIMIT);
            sessions
                .values()
                .take(SESSION_LIMIT)
                .cloned()
                .collect::<Vec<_>>()
        }
        Err(_) => {
            result.coverage.partial = true;
            return result;
        }
    };
    let mut remaining = ENTRY_LIMIT;
    for handle in handles {
        if remaining == 0 {
            result.coverage.skipped_budget += 1;
            continue;
        }
        match handle.try_lock() {
            Ok(session) => {
                result.coverage.inspected += 1;
                if !result.inspect(&session, now, &mut remaining) {
                    result.coverage.skipped_budget += 1;
                }
            }
            Err(_) => result.coverage.skipped_busy += 1,
        }
    }
    result.coverage.partial =
        result.coverage.skipped_busy > 0 || result.coverage.skipped_budget > 0;
    result.blockers.sort_by_key(|item| item.reason);
    result.phases.sort_by_key(|item| item.phase);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use serde_json::json;
    use tokio::sync::Mutex;
    use crate::chat::types::DeltaOp;

    fn request(id: &str, command: serde_json::Value) -> CommandRequest {
        let mut value = command;
        value["client_request_id"] = json!(id);
        value["priority"] = json!(false);
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn known_unknown_ages_answers_subset_and_global_entry_budget() {
        let now = Instant::now();
        let mut session = ChatSession::new("secret-chat".into());
        session.command_queue.push_back(request(
            "secret-answer",
            json!({"type":"answer_questions","tool_call_id":"secret-tool","answers":[]}),
        ));
        session.command_queue.push_back(request(
            "secret-request",
            json!({"type":"user_message","content":"secret-prompt"}),
        ));
        session
            .command_enqueued_at
            .insert("secret-answer".into(), now - Duration::from_millis(500));
        session.runtime.state = SessionState::Paused;
        let mut result = QueueDiagnostics::default();
        let mut budget = 2;
        assert!(result.inspect(&session, now, &mut budget));
        assert_eq!(budget, 0);
        assert_eq!(result.commands.pending_count, 2);
        assert_eq!(result.commands.oldest_pending_age_ms, Some(500));
        assert_eq!(result.commands.unknown_age_count, 1);
        assert_eq!(result.question_answers.pending_count, 1);
        assert_eq!(
            result
                .blockers
                .iter()
                .find(|b| b.reason == "ready")
                .unwrap()
                .pending
                .pending_count,
            1
        );
        assert_eq!(
            result
                .blockers
                .iter()
                .find(|b| b.reason == "tool_confirmation")
                .unwrap()
                .pending
                .pending_count,
            1
        );
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["progress"]["unknown_queue_progress_sessions"], 1);
        fn aggregate_keys(value: &serde_json::Value) {
            match value {
                serde_json::Value::Object(fields) => {
                    for (key, child) in fields {
                        assert!(matches!(
                            key.as_str(),
                            "sampled_at_ms"
                                | "coverage"
                                | "sessions_total"
                                | "inspected"
                                | "skipped_busy"
                                | "skipped_budget"
                                | "partial"
                                | "commands"
                                | "deliveries"
                                | "question_answers"
                                | "blockers"
                                | "progress"
                                | "oldest_queue_progress_age_ms"
                                | "oldest_chat_progress_age_ms"
                                | "unknown_queue_progress_sessions"
                                | "unknown_chat_progress_sessions"
                                | "accepted_waiting_first_output"
                                | "phases"
                                | "reason"
                                | "phase"
                                | "pending_count"
                                | "oldest_pending_age_ms"
                                | "unknown_age_count"
                        ));
                        aggregate_keys(child);
                    }
                }
                serde_json::Value::Array(items) => items.iter().for_each(aggregate_keys),
                serde_json::Value::String(label) => {
                    assert!(matches!(label.as_str(), "ready" | "tool_confirmation"));
                }
                _ => {}
            }
        }
        aggregate_keys(&value);
        session
            .command_enqueued_at
            .insert("secret-request".into(), now - Duration::from_millis(100));
        let mut two_known = QueueDiagnostics::default();
        let mut budget = 2;
        assert!(two_known.inspect(&session, now, &mut budget));
        assert_eq!(two_known.commands.oldest_pending_age_ms, Some(500));
        let mut capped = QueueDiagnostics::default();
        let mut budget = 1;
        assert!(!capped.inspect(&session, now, &mut budget));
        assert_eq!(capped.commands.pending_count, 1);
        assert_eq!(budget, 0);
        session.closed = true;
        let mut closed = QueueDiagnostics::default();
        assert!(closed.inspect(&session, now, &mut budget));
        assert_eq!(closed.commands.pending_count, 0);
    }

    #[test]
    fn accepted_phases_ignore_metadata_preserve_oldest_and_clear_on_output_or_abort() {
        use super::super::perf_diagnostics::{
            install_test_recorder, MemoryPerfSink, PerfClock, PerfRecorder,
        };

        struct Clock;
        impl PerfClock for Clock {
            fn now_us(&self) -> u64 {
                0
            }
        }
        let sink = Arc::new(MemoryPerfSink::new());
        let _recorder =
            install_test_recorder(Arc::new(PerfRecorder::new(Arc::new(Clock), sink.clone())));
        let now = Instant::now();
        let mut session = ChatSession::new("private".into());
        session.live_diagnostics.chat_progress = Some(now - Duration::from_secs(3_600));
        session
            .live_diagnostics
            .accept(now - Duration::from_secs(20));
        session
            .live_diagnostics
            .accept(now - Duration::from_secs(10));
        assert_eq!(
            session.live_diagnostics.accepted_at,
            Some(now - Duration::from_secs(20))
        );
        assert_eq!(session.live_diagnostics.accepted_count, 2);
        assert_eq!(
            session.live_diagnostics.chat_progress,
            Some(now - Duration::from_secs(20))
        );
        session
            .live_diagnostics
            .phase("preamble", now - Duration::from_secs(5));
        let mut sampled = QueueDiagnostics::default();
        let mut budget = ENTRY_LIMIT;
        assert!(sampled.inspect(&session, now, &mut budget));
        assert_eq!(
            sampled.accepted_waiting_first_output.oldest_pending_age_ms,
            Some(20_000)
        );
        assert_eq!(sampled.phases[0].phase, "preamble");
        assert_eq!(sampled.phases[0].pending.oldest_pending_age_ms, Some(5_000));
        session.start_stream().unwrap();
        session.emit_stream_delta_at(
            vec![DeltaOp::SetUsage {
                usage: json!({}),
            }],
            now,
        );
        assert_eq!(session.live_diagnostics.accepted_count, 2);
        session.finish_stream(None);
        assert_eq!(session.live_diagnostics.accepted_count, 2);
        session.start_stream().unwrap();
        session.emit_stream_delta_at(
            vec![DeltaOp::AppendReasoning {
                text: "thinking".into(),
            }],
            now,
        );
        assert_eq!(session.live_diagnostics.accepted_count, 0);
        assert_eq!(session.live_diagnostics.chat_progress, Some(now));
        session.emit_stream_delta_at(
            vec![DeltaOp::AppendContent {
                text: "answer".into(),
            }],
            now + Duration::from_secs(1),
        );
        let accepted_events: Vec<_> = sink
            .events()
            .into_iter()
            .filter(|event| event.component == "message.accepted_to_first_delta")
            .collect();
        assert_eq!(accepted_events.len(), 1);
        assert_eq!(accepted_events[0].elapsed_us, 20_000_000);
        session.live_diagnostics.accept(now);
        session.abort_stream();
        assert!(session.live_diagnostics.accepted_at.is_none());
        session.live_diagnostics.accept(now);
        session.close_event_channel();
        assert!(session.live_diagnostics.accepted_at.is_none());
    }

    #[test]
    fn delivery_only_progress_starts_at_enqueue_and_does_not_include_prior_idle_time() {
        use crate::call_validation::ChatMessage;
        use crate::chat::types::{PendingDelivery, PushMode};

        let stale = Instant::now() - Duration::from_secs(3_600);
        let mut session = ChatSession::new("delivery-progress".into());
        session.live_diagnostics.queue_progress = Some(stale);
        session.start_stream().unwrap();
        let delivery = |id: &str| {
            PendingDelivery::with_id(
                id,
                vec![ChatMessage::new("user".into(), "private delivery".into())],
                PushMode::Append,
                "diagnostic-test",
                false,
            )
        };
        assert_eq!(
            session.enqueue_delivery(delivery("first")).unwrap(),
            crate::chat::types::DeliveryOutcome::Queued
        );
        let enqueued = session.live_diagnostics.queue_progress.unwrap();
        assert!(enqueued > stale);
        session.enqueue_delivery(delivery("second")).unwrap();
        assert_eq!(session.live_diagnostics.queue_progress, Some(enqueued));
        let mut sampled = QueueDiagnostics::default();
        let mut budget = ENTRY_LIMIT;
        assert!(sampled.inspect(&session, enqueued + Duration::from_secs(5), &mut budget));
        assert_eq!(sampled.progress.oldest_queue_progress_age_ms, Some(5_000));
        session.pending_deliveries.clear();
        session.live_diagnostics.queue_progress = Some(stale);
        session.restore_pending_deliveries(vec![delivery("restored")]);
        assert_eq!(session.pending_deliveries.len(), 1);
        assert!(session.live_diagnostics.queue_progress.is_none());
    }

    #[tokio::test]
    async fn sampling_disabled_history_and_held_locks_is_partial_without_waiting() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        app.gcx.performance_telemetry.set_enabled(false);
        let session = Arc::new(Mutex::new(ChatSession::new("secret".into())));
        session
            .lock()
            .await
            .command_queue
            .push_back(request("private-request", json!({"type":"regenerate"})));
        app.chat
            .sessions
            .write()
            .await
            .insert("secret".into(), session.clone());
        assert_eq!(sample(&app).commands.pending_count, 1);
        app.gcx.performance_telemetry.reset();
        assert_eq!(sample(&app).commands.pending_count, 1);
        let held = session.lock().await;
        let sampled = sample(&app);
        assert_eq!(sampled.coverage.sessions_total, Some(1));
        assert_eq!(sampled.coverage.skipped_busy, 1);
        assert!(sampled.coverage.partial);
        drop(held);
        let registry = app.chat.sessions.write().await;
        let sampled = sample(&app);
        assert_eq!(sampled.coverage.sessions_total, None);
        assert_eq!(sampled.coverage.inspected, 0);
        assert!(sampled.coverage.partial);
        drop(registry);
        assert!(!sample(&app).coverage.partial);
        let mut registry = app.chat.sessions.write().await;
        for index in 1..=SESSION_LIMIT {
            registry.insert(
                format!("private-{index}"),
                Arc::new(Mutex::new(ChatSession::new(format!("private-{index}")))),
            );
        }
        drop(registry);
        let capped = sample(&app);
        assert_eq!(capped.coverage.sessions_total, Some(SESSION_LIMIT + 1));
        assert_eq!(capped.coverage.inspected, SESSION_LIMIT);
        assert_eq!(capped.coverage.skipped_budget, 1);
        assert!(capped.coverage.partial);
    }
}
