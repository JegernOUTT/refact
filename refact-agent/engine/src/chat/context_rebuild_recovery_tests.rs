use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::call_validation::{ChatMessage, ChatToolCall, ChatToolFunction};
use crate::chat::context_rebuild::*;
use crate::chat::types::{ChatSession, CompressionPhase, CompressionReason, SessionState};
use refact_core::active_context::{active_context, make_reconstruction_report, ReconstructionMetadata};

fn msg(id: &str, role: &str, text: &str) -> ChatMessage {
    let mut m = ChatMessage::new(role.into(), text.into());
    m.message_id = id.into();
    m
}

fn call(id: &str, name: &str) -> ChatToolCall {
    ChatToolCall {
        id: id.into(),
        index: None,
        function: ChatToolFunction {
            arguments: "{}".into(),
            name: name.into(),
        },
        tool_type: "function".into(),
        extra_content: None,
        started_at_ms: None,
        completed_at_ms: None,
    }
}

fn assistant_calls(id: &str, text: &str, calls: Vec<ChatToolCall>) -> ChatMessage {
    let mut m = msg(id, "assistant", text);
    m.tool_calls = Some(calls);
    m
}

fn result(id: &str, call_id: &str, text: &str) -> ChatMessage {
    let mut m = msg(id, "tool", text);
    m.tool_call_id = call_id.into();
    m
}

fn with_privacy(mut m: ChatMessage) -> ChatMessage {
    m.extra.insert(
        "privacy".into(),
        serde_json::json!({"files":[{"path":"/p/a.rs","zone":"normal","attribution":"declared"}]}),
    );
    m
}

fn has_unanswered(messages: &[ChatMessage]) -> bool {
    let mut open = HashSet::new();
    for m in messages {
        for c in m.tool_calls.iter().flatten() {
            open.insert(c.id.clone());
        }
        if matches!(m.role.as_str(), "tool" | "diff" | "context_file") {
            open.remove(&m.tool_call_id);
        }
    }
    !open.is_empty()
}

fn field_shape() -> Vec<ChatMessage> {
    let mut v = vec![
        msg("u1", "user", "older question: what is X?"),
        with_privacy(assistant_calls(
            "a1",
            "resuming agent and reading file",
            vec![call("orphan", "agent_resume"), call("done", "cat")],
        )),
        with_privacy(result("r-done", "done", "file body\u{1b}[0m")),
        msg(
            "answer",
            "user",
            r#"{"type":"ask_questions_answer","tool_call_id":"older-question","answers":[{"id":"x","answer":"approved"}]}"#,
        ),
        msg("a2", "assistant", "X is the answer to the older question"),
    ];
    for i in 0..6 {
        v.push(msg(&format!("u{}", i + 2), "user", "next"));
        v.push(msg(&format!("g{i}"), "assistant", "newer generation"));
    }
    v
}

#[test]
fn abandoned_historical_call_gets_failed_unknown_result_and_raw_is_untouched() {
    let raw = field_shape();
    let raw_bytes = serde_json::to_string(&raw).unwrap();
    assert!(has_unanswered(&raw));
    let (normalized, abandoned) = normalize_rebuild_input(&raw, &HashSet::new());
    assert_eq!(serde_json::to_string(&raw).unwrap(), raw_bytes);
    assert!(!has_unanswered(&normalized));
    assert_eq!(abandoned.len(), 1);
    assert_eq!(abandoned[0].tool_call_id, "orphan");
    assert_eq!(abandoned[0].name, "agent_resume");
    assert_eq!(abandoned[0].assistant_message_id, "a1");
    assert_eq!(normalized.len(), raw.len() + 1);
    let synthetic = &normalized[3];
    assert_eq!(synthetic.role, "tool");
    assert_eq!(synthetic.tool_call_id, "orphan");
    assert_eq!(synthetic.tool_failed, Some(true));
    assert_eq!(
        synthetic.content.content_text_only(),
        ABANDONED_OUTCOME_TEXT
    );
    let text = ABANDONED_OUTCOME_TEXT.to_lowercase();
    assert!(text.contains("outcome unknown"));
    assert!(!text.contains("succeeded.") && !text.contains("was never executed"));
    assert_eq!(
        synthetic.extra[ABANDONED_RESULT_EXTRA]["outcome"],
        serde_json::json!("unknown")
    );
    let mut without = normalized.clone();
    without.remove(3);
    assert_eq!(serde_json::to_string(&without).unwrap(), raw_bytes);
    assert_eq!(normalized[1].tool_calls.as_ref().unwrap().len(), 2);
    assert!(normalized[2].content.content_text_only().contains('\u{1b}'));
    assert!(normalized[1].extra.contains_key("privacy"));
}

#[test]
fn current_unresolved_group_still_blocks_even_after_events() {
    let mut raw = field_shape();
    raw.truncate(5);
    raw.push(assistant_calls("latest", "", vec![call("live", "shell")]));
    let mut note = crate::chat::internal_roles::event(
        crate::chat::internal_roles::EventSubkind::SystemNotice,
        "test",
        serde_json::json!({}),
        "notice",
    );
    note.message_id = "ev".into();
    raw.push(note);
    raw.push(msg("sys", "system", "system text"));
    let (normalized, abandoned) = normalize_rebuild_input(&raw, &HashSet::new());
    assert!(has_unanswered(&normalized));
    assert!(normalized.iter().all(|m| m.tool_call_id != "live"));
    assert_eq!(abandoned.len(), 1);
    assert_eq!(abandoned[0].tool_call_id, "orphan");
}

#[test]
fn trailing_user_message_is_not_supersession_evidence() {
    let raw = vec![
        msg("u", "user", "q"),
        assistant_calls("a", "", vec![call("c", "shell")]),
        msg("u2", "user", "still waiting"),
    ];
    let (normalized, abandoned) = normalize_rebuild_input(&raw, &HashSet::new());
    assert!(abandoned.is_empty());
    assert!(has_unanswered(&normalized));
}

#[test]
fn runtime_owned_calls_are_never_reconciled() {
    let raw = field_shape();
    let live: HashSet<String> = ["orphan".to_string()].into();
    let (normalized, abandoned) = normalize_rebuild_input(&raw, &live);
    assert!(abandoned.is_empty());
    assert!(has_unanswered(&normalized));
    let mut s = ChatSession::new("live-owner".into());
    s.pending_questions
        .push(refact_chat_api::PendingQuestionSet {
            tool_call_id: "orphan".into(),
            questions: vec![],
            asked_at_ms: 0,
            status: refact_chat_api::PendingQuestionStatus::Pending,
            answers: vec![],
            resolution_id: None,
            response_protocol_version: refact_chat_api::QUESTION_RESPONSE_PROTOCOL_VERSION,
            response: None,
        });
    assert!(live_tool_call_ids(&s).contains("orphan"));
    s.pending_questions[0].status = refact_chat_api::PendingQuestionStatus::Answered;
    assert!(!live_tool_call_ids(&s).contains("orphan"));
}

#[test]
fn synthetic_ids_are_collision_safe_and_payload_valid() {
    let mut raw = field_shape();
    raw[0].message_id = "abandoned-result-orphan".into();
    raw.push(msg("abandoned-result-orphan-1", "user", "collide"));
    raw.push(msg("tail", "assistant", "done"));
    let (normalized, abandoned) = normalize_rebuild_input(&raw, &HashSet::new());
    let ids: Vec<_> = normalized.iter().map(|m| m.message_id.clone()).collect();
    let unique: HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len());
    assert!(ids.contains(&"abandoned-result-orphan-2".to_string()));
    let notes = rebuild_notes(&normalized, &abandoned);
    let payload = with_abandoned_notes(normalized, notes);
    assert!(make_reconstruction_report(payload, ReconstructionMetadata::default()).is_ok());
}

#[test]
fn unknown_outcome_notice_is_deterministic_and_survives_repeated_rebuilds() {
    let raw = field_shape();
    let (normalized, abandoned) = normalize_rebuild_input(&raw, &HashSet::new());
    let notes = rebuild_notes(&normalized, &abandoned);
    assert_eq!(notes.len(), 1);
    let note_bytes = serde_json::to_string(&notes[0]).unwrap();
    let analyzer_output = vec![msg(
        "cont",
        "user",
        "## Summary\n\nanalyzer dropped everything",
    )];
    let first = with_abandoned_notes(analyzer_output, notes);
    assert_eq!(first.len(), 2);
    assert!(is_abandoned_note(&first[0]));
    assert!(first[0]
        .content
        .content_text_only()
        .contains("Outcome unknown"));
    let report = make_reconstruction_report(first, ReconstructionMetadata::default()).unwrap();

    let mut stored = raw.clone();
    stored.push(report);
    stored.push(msg("later-u", "user", "continue"));
    stored.push(msg("later-a", "assistant", "ok"));
    let restored: Vec<ChatMessage> =
        serde_json::from_str(&serde_json::to_string(&stored).unwrap()).unwrap();
    let second_input = active_context(&restored).unwrap().messages;
    let (second_norm, second_abandoned) = normalize_rebuild_input(&second_input, &HashSet::new());
    assert!(second_abandoned.is_empty());
    let carried = rebuild_notes(&second_norm, &second_abandoned);
    assert_eq!(carried.len(), 1);
    assert_eq!(serde_json::to_string(&carried[0]).unwrap(), note_bytes);
    let second = with_abandoned_notes(vec![msg("cont2", "user", "summary 2")], carried);
    assert_eq!(second.iter().filter(|m| is_abandoned_note(m)).count(), 1);
    assert!(make_reconstruction_report(second, ReconstructionMetadata::default()).is_ok());
    assert_eq!(
        serde_json::to_string(&restored[..raw.len()]).unwrap(),
        serde_json::to_string(&raw).unwrap()
    );
}

#[test]
fn already_noted_calls_are_not_noted_twice_and_analyzer_copies_are_deduplicated() {
    let raw = field_shape();
    let (normalized, abandoned) = normalize_rebuild_input(&raw, &HashSet::new());
    let note = rebuild_notes(&normalized, &abandoned).pop().unwrap();
    let mut source = normalized.clone();
    source.push(note.clone());
    let notes = rebuild_notes(&source, &abandoned);
    assert_eq!(notes.len(), 1);
    let out = with_abandoned_notes(vec![note.clone(), msg("c", "user", "s")], notes);
    assert_eq!(out.iter().filter(|m| is_abandoned_note(m)).count(), 1);
}

#[test]
fn many_abandoned_calls_produce_one_bounded_notice() {
    let mut raw = vec![msg("u", "user", "q")];
    let calls = (0..100).map(|i| call(&format!("c{i}"), "shell")).collect();
    raw.push(assistant_calls("a", "", calls));
    raw.push(msg("a2", "assistant", "moved on"));
    let (_, abandoned) = normalize_rebuild_input(&raw, &HashSet::new());
    assert_eq!(abandoned.len(), 100);
    let note = abandoned_calls_note(&abandoned).unwrap();
    let payload = &note.extra["event"]["payload"];
    assert_eq!(payload["count"], 100);
    assert_eq!(payload["truncated"], true);
    assert_eq!(payload["tool_calls"].as_array().unwrap().len(), 32);
    assert!(note.content.content_text_only().contains("and 68 more"));
}

#[test]
fn routine_skips_are_nonfatal_and_do_not_mark_current_version() {
    for skip in [
        SKIP_SESSION_CHANGED,
        SKIP_CHANGED_DURING_HOOK,
        SKIP_NOT_IDLE,
        SKIP_OTHER_PENDING,
        SKIP_ALREADY_ATTEMPTED,
        SKIP_PENDING_TOOLS,
    ] {
        let mut s = ChatSession::new("skip".into());
        s.add_message(msg("u", "user", "keep"));
        let before = serde_json::to_value(&s.messages).unwrap();
        assert_eq!(
            record_rebuild_failure(&mut s, skip),
            CompactionOutcome::NothingToCompact,
            "{skip}"
        );
        assert_eq!(serde_json::to_value(&s.messages).unwrap(), before);
        assert_ne!(s.runtime.state, SessionState::Error);
        assert!(s.runtime.error.is_none());
        assert!(s.last_rebuild_attempt_version.is_none());
        assert_eq!(s.compression_phase, Some(CompressionPhase::Skipped));
        let expected = if skip == SKIP_PENDING_TOOLS {
            CompressionReason::PendingToolCalls
        } else {
            CompressionReason::SourceChanged
        };
        assert_eq!(s.compression_reason, Some(expected));
    }
}

#[test]
fn genuine_failure_stops_with_redacted_detail() {
    let mut s = ChatSession::new("fail".into());
    let secret = "sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789";
    let error = format!("provider returned 401: invalid key {secret}");
    assert_eq!(
        record_rebuild_failure(&mut s, &error),
        CompactionOutcome::LlmUnavailable
    );
    assert_eq!(s.runtime.state, SessionState::Error);
    let runtime_error = s.runtime.error.clone().unwrap();
    assert!(runtime_error.starts_with("Context rebuild failed: "));
    assert!(runtime_error.contains("401"), "{runtime_error}");
    assert!(!runtime_error.contains(secret), "{runtime_error}");
    let banner = s.messages.last().unwrap().content.content_text_only();
    assert!(!banner.contains(secret));
    assert_eq!(s.last_rebuild_attempt_version, Some(s.trajectory_version));
    assert!(!is_routine_rebuild_skip(&error));
    assert!(!is_routine_rebuild_skip("disk failed"));
}

fn average_symbol_budget(messages: &[ChatMessage], cap: usize) -> usize {
    let symbols: usize = messages
        .iter()
        .map(refact_agentic::mode_transition::message_symbols)
        .sum();
    let tokens: usize = messages
        .iter()
        .map(|m| crate::tokens::count_text_tokens(None, &m.content.content_text_only()).unwrap())
        .sum();
    cap * symbols / tokens
}

#[test]
fn token_cap_converts_to_symbols_and_reserves_low_density_controls() {
    let dense = "漢".repeat(30_000);
    let mut plan = msg("plan", "plan", &"p ".repeat(20_000));
    plan.extra.insert(
        "plan".into(),
        serde_json::json!({"mode":"agent","version":1,"created_at_ms":0,"supersedes":null}),
    );
    let messages = vec![msg("u", "user", &dense), plan.clone()];
    let plan_symbols = refact_agentic::mode_transition::message_symbols(&plan);
    let plan_tokens =
        crate::tokens::count_text_tokens(None, &plan.content.content_text_only()).unwrap();
    let cap = plan_tokens + 2_000;
    assert!(average_symbol_budget(&messages, cap) < plan_symbols);
    let budget = token_cap_to_symbol_budget(&messages, cap, None);
    assert!(budget > plan_symbols, "{budget} <= {plan_symbols}");
    let total: usize = messages
        .iter()
        .map(refact_agentic::mode_transition::message_symbols)
        .sum();
    assert!(budget < total);
    let source_only = vec![msg("u", "user", &"a".repeat(70_000))];
    let tokens = crate::tokens::count_text_tokens(None, &"a".repeat(70_000)).unwrap();
    let converted = token_cap_to_symbol_budget(&source_only, tokens / 2, None);
    assert!(
        converted > tokens / 2,
        "symbols must exceed tokens for ASCII"
    );
    assert!(converted < 70_000);
    let huge = token_cap_to_symbol_budget(&source_only, usize::MAX / 2, None);
    assert!(huge < 70_000, "budget can never be a no-op");
}

#[tokio::test]
async fn fresh_live_agents_are_reserved_before_source_size_budget_cap() {
    let history = vec![msg("request", "user", &"continue work ".repeat(40))];
    let mut live = crate::chat::internal_roles::event(
        crate::chat::internal_roles::EventSubkind::SystemNotice,
        refact_agentic::mode_transition::LIVE_AGENTS_EVENT_SOURCE,
        serde_json::json!({"agents": [{"agent_id": "bgagent-exact", "task": "retain task detail ".repeat(500)}]}),
        "current live subagent state",
    );
    live.message_id = "live-state".into();
    let mandatory_symbols = refact_agentic::mode_transition::message_symbols(&live)
        + live.extra["event"].to_string().chars().count();
    assert!(token_cap_to_symbol_budget(&history, 20_000, None) < mandatory_symbols);
    let input =
        crate::agentic::mode_transition::replace_live_agents_context(&history, Some(live.clone()));
    let total_symbols = token_cap_to_symbol_budget(&input, 20_000, None);
    assert!(total_symbols > mandatory_symbols);
    let rebuilt = refact_agentic::mode_transition::assemble_reconstruction_with_mandatory(
        &input,
        &refact_agentic::mode_transition::ParsedDecisions::default(),
        "agent",
        &[],
        refact_agentic::mode_transition::TransitionContextBudget {
            previous_symbols: total_symbols * 2,
            total_symbols,
            files_symbols: total_symbols / 2,
            messages_symbols: total_symbols / 2,
            max_images: 0,
        },
        true,
        &Default::default(),
        std::slice::from_ref(&live),
    )
    .await
    .unwrap();
    let carried = rebuilt
        .iter()
        .find(|message| refact_agentic::mode_transition::is_live_agents_snapshot(message))
        .unwrap();
    assert_eq!(carried.extra, live.extra);
}

#[test]
fn token_cap_scales_down_when_characters_cost_multiple_tokens() {
    let text = "🦀".repeat(10_000);
    let tokens = crate::tokens::count_text_tokens(None, &text).unwrap();
    let symbols = text.chars().count();
    assert!(tokens > symbols);
    let cap = tokens / 4;
    let budget = token_cap_to_symbol_budget(&[msg("u", "user", &text)], cap, None);
    assert_eq!(budget, cap * symbols / tokens);
    assert!(budget < cap);
}

#[tokio::test]
async fn preview_rejects_historical_calls_still_owned_by_the_runtime() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let mut s = ChatSession::new("preview-owned".into());
    for message in field_shape() {
        s.add_message(message);
    }
    let live_ids = HashSet::from(["orphan".to_string()]);
    let preview = preview_manual_context_rebuild(
        gcx,
        SessionState::Idle,
        false,
        &s.messages,
        &live_ids,
        &s.thread,
        None,
    )
    .await;
    assert!(!preview.eligible);
    assert!(preview.reason.unwrap().contains("completed tool results"));
    assert!(preview.resolved_model.is_none());
}

#[tokio::test]
async fn failed_persistence_is_fatal_redacted_and_never_publishes_a_report() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let mut s = ChatSession::new("persist-failure".into());
    s.add_message(msg("u", "user", "original context"));
    let before = serde_json::to_value(&s.messages).unwrap();
    s.active_compression_attempt = Some(1);
    status(&mut s, CompressionPhase::Running, None);
    let source = RebuildSource::capture(&s).unwrap();
    let thread = s.thread.clone();
    let session = Arc::new(Mutex::new(s));
    let report = make_reconstruction_report(
        vec![msg("rebuilt", "user", "summary")],
        ReconstructionMetadata::default(),
    )
    .unwrap();
    let secret = "sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789";
    let error = commit_report_then_observe(
        gcx,
        &session,
        report,
        &source,
        1,
        &thread,
        &std::sync::atomic::AtomicBool::new(false),
        "manual",
        1,
        |_| async { Err(format!("disk failed: {secret}")) },
    )
    .await
    .unwrap_err();
    let mut s = session.lock().await;
    assert_eq!(serde_json::to_value(&s.messages).unwrap(), before);
    assert_eq!(s.compression_phase, Some(CompressionPhase::Failed));
    assert_eq!(
        record_rebuild_failure(&mut s, &error),
        CompactionOutcome::LlmUnavailable
    );
    assert_eq!(s.runtime.state, SessionState::Error);
    assert_eq!(s.compression_phase, Some(CompressionPhase::Failed));
    assert!(s.runtime.error.as_ref().unwrap().contains("disk failed"));
    assert!(!s.runtime.error.as_ref().unwrap().contains(secret));
    assert!(!s.messages.iter().any(|m| m.role == "compression_report"));
}

#[tokio::test]
async fn queued_rebuild_skip_emits_truthful_unapplied_notice_without_requeueing() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let mut s = ChatSession::new("queued-pending".into());
    s.add_message(msg("u", "user", "task"));
    s.add_message(assistant_calls("a", "", vec![call("pending", "shell")]));
    request_rebuild(
        &mut s,
        PendingContextRebuild {
            model: None,
            trigger: "tool-requested".into(),
        },
    )
    .unwrap();
    let session = Arc::new(Mutex::new(s));
    assert!(!drain_pending(gcx, &session).await.unwrap());
    let mut s = session.lock().await;
    assert!(s.pending_context_rebuild.is_none());
    assert_ne!(s.runtime.state, SessionState::Error);
    assert_eq!(s.messages.last().unwrap().message_id, "a");
    assert_eq!(s.pending_deliveries_for_snapshot().len(), 1);
    s.add_message(result("r", "pending", "completed"));
    s.drain_pending_deliveries();
    let notice = s.messages.last().unwrap();
    assert_eq!(notice.extra["event"]["payload"]["applied"], false);
    assert_eq!(
        notice.extra["event"]["payload"]["trigger"],
        "tool-requested"
    );
    assert!(notice
        .content
        .content_text_only()
        .contains("was not applied"));
}

#[test]
fn queued_hook_veto_stays_nonfatal_and_redacts_its_notice() {
    let mut s = ChatSession::new("queued-veto".into());
    status(
        &mut s,
        CompressionPhase::Skipped,
        Some(CompressionReason::PreCompactDenied),
    );
    let secret = "sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789";
    record_requested_rebuild_skip(
        &mut s,
        "tool-requested",
        &format!("blocked by PreCompact hook: policy denied {secret}"),
    )
    .unwrap();
    assert_ne!(s.runtime.state, SessionState::Error);
    assert!(s.runtime.error.is_none());
    assert_eq!(
        s.compression_reason,
        Some(CompressionReason::PreCompactDenied)
    );
    let notice = s.messages.last().unwrap();
    assert_eq!(notice.extra["event"]["payload"]["applied"], false);
    assert!(!serde_json::to_string(notice).unwrap().contains(secret));
    assert!(record_requested_rebuild_skip(&mut s, "tool-requested", "disk failed").is_err());
}

#[tokio::test]
async fn apply_blocks_runtime_owned_question_but_proceeds_past_abandoned_call() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    for owned in [true, false] {
        let mut s = ChatSession::new(format!("apply-{owned}"));
        for m in field_shape() {
            s.add_message(m);
        }
        if owned {
            s.pending_questions
                .push(refact_chat_api::PendingQuestionSet {
                    tool_call_id: "orphan".into(),
                    questions: vec![],
                    asked_at_ms: 0,
                    status: refact_chat_api::PendingQuestionStatus::Pending,
                    answers: vec![],
                    resolution_id: None,
                    response_protocol_version: refact_chat_api::QUESTION_RESPONSE_PROTOCOL_VERSION,
                    response: None,
                });
        }
        let before = serde_json::to_string(&s.messages).unwrap();
        let session = Arc::new(Mutex::new(s));
        let error = rebuild_session(
            gcx.clone(),
            &session,
            PendingContextRebuild {
                model: None,
                trigger: "manual".into(),
            },
            None,
            true,
        )
        .await
        .unwrap_err();
        assert_eq!(error == SKIP_PENDING_TOOLS, owned, "{error}");
        let s = session.lock().await;
        assert_eq!(serde_json::to_string(&s.messages).unwrap(), before);
        assert!(!compression_attempt_active(&s));
    }
}
