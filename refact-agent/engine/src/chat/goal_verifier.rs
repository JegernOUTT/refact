use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ChatMeta, ChatUsage, SamplingParameters};
use crate::chat::internal_roles::{self, EventSubkind};
use crate::chat::prepare::{prepare_chat_passthrough, ChatPrepareOptions, ToolChoice};
use crate::chat::stream_core::{
    run_llm_stream, ChoiceFinal, LlmStreamOutcome, StreamCollector, StreamRunParams,
    ABORT_ERROR_MESSAGE,
};
use crate::chat::types::*;
use crate::constants::CHAT_TOP_N;
use crate::llm::params::CacheControl;
use crate::scratchpad_abstract::HasTokenizerAndEot;
use refact_tool_api::ToolDesc;

const GOAL_VERIFIER_SOURCE: &str = "chat.goal_verifier";
const INCONCLUSIVE_GAP: &str = "verification inconclusive; continue";
const GOAL_VERIFIER_MAX_TOKENS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalVerdict {
    Met,
    Unmet(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalVerifierReply {
    pub verdict: GoalVerdict,
    pub verifier_reply: String,
    pub tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalVerificationBegin {
    Started,
    NoActiveGoal,
    BudgetExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalVerificationApplyOutcome {
    Finalized,
    Rearmed,
    NoGoal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalCompletionGateOutcome {
    Passthrough,
    Finalized,
    Rearmed,
    BudgetExhausted(GoalStatus),
    Aborted,
}

fn epoch_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn should_verify_goal_on_done(session: &ChatSession) -> bool {
    session.goal.as_ref().is_some_and(|goal| {
        goal.active
            && matches!(
                goal.status,
                GoalStatus::Active | GoalStatus::BudgetExhausted | GoalStatus::NoProgress
            )
    })
}

pub fn begin_goal_verification_if_needed(session: &mut ChatSession) -> GoalVerificationBegin {
    let Some(goal) = session.goal.as_ref() else {
        return GoalVerificationBegin::NoActiveGoal;
    };
    if !goal.active {
        return GoalVerificationBegin::NoActiveGoal;
    }
    if matches!(
        goal.status,
        GoalStatus::BudgetExhausted | GoalStatus::NoProgress
    ) {
        return GoalVerificationBegin::BudgetExhausted;
    }
    if goal.status != GoalStatus::Active {
        return GoalVerificationBegin::NoActiveGoal;
    }
    if goal.goal_budget_exhausted() {
        return GoalVerificationBegin::BudgetExhausted;
    }
    session.goal_set_status(GoalStatus::Verifying);
    GoalVerificationBegin::Started
}

pub async fn verify_goal_before_completion(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    trigger: &str,
) -> GoalCompletionGateOutcome {
    let begin = {
        let mut session = session_arc.lock().await;
        begin_goal_verification_if_needed(&mut session)
    };
    match begin {
        GoalVerificationBegin::Started => {}
        GoalVerificationBegin::BudgetExhausted => {
            let mut session = session_arc.lock().await;
            let status = apply_goal_budget_exhausted_terminal(&mut session, trigger)
                .unwrap_or(GoalStatus::BudgetExhausted);
            return GoalCompletionGateOutcome::BudgetExhausted(status);
        }
        GoalVerificationBegin::NoActiveGoal => return GoalCompletionGateOutcome::Passthrough,
    }
    {
        let session = session_arc.lock().await;
        if session.user_interrupt_flag.load(Ordering::SeqCst) {
            drop(session);
            abort_goal_verification(session_arc.clone()).await;
            return GoalCompletionGateOutcome::Aborted;
        }
    }

    let reply = match run_goal_verifier(app, session_arc.clone()).await {
        Ok(reply) => reply,
        Err(error) => {
            let aborted = {
                let session = session_arc.lock().await;
                session.user_interrupt_flag.load(Ordering::SeqCst) || error == ABORT_ERROR_MESSAGE
            };
            if aborted {
                abort_goal_verification(session_arc.clone()).await;
                return GoalCompletionGateOutcome::Aborted;
            }
            GoalVerifierReply {
                verdict: GoalVerdict::Unmet(vec![INCONCLUSIVE_GAP.to_string()]),
                verifier_reply: format!("verification failed: {error}"),
                tokens: 0,
            }
        }
    };

    {
        let session = session_arc.lock().await;
        if session.user_interrupt_flag.load(Ordering::SeqCst) {
            drop(session);
            abort_goal_verification(session_arc.clone()).await;
            return GoalCompletionGateOutcome::Aborted;
        }
    }

    let mut session = session_arc.lock().await;
    match apply_goal_verdict(&mut session, trigger, reply) {
        GoalVerificationApplyOutcome::Finalized => GoalCompletionGateOutcome::Finalized,
        GoalVerificationApplyOutcome::Rearmed => GoalCompletionGateOutcome::Rearmed,
        GoalVerificationApplyOutcome::NoGoal => GoalCompletionGateOutcome::Passthrough,
    }
}

fn apply_goal_budget_exhausted_terminal(
    session: &mut ChatSession,
    trigger: &str,
) -> Option<GoalStatus> {
    let at_ms = epoch_ms_now();
    let goal = session.goal.as_ref()?;
    let status = if matches!(
        goal.status,
        GoalStatus::BudgetExhausted | GoalStatus::NoProgress
    ) {
        goal.status
    } else {
        goal.goal_budget_exhaustion_status_at(at_ms)
            .unwrap_or(GoalStatus::BudgetExhausted)
    };
    let kind = terminal_goal_kind(status);
    let already_recorded = session.goal.as_ref().is_some_and(|goal| {
        goal.events
            .iter()
            .any(|event| event.kind == "goal_pursuit" && event.text.contains(kind))
    });
    session.goal_set_status(status);
    if !already_recorded {
        let message = goal_budget_exhausted_terminal_event(status, trigger, at_ms);
        session.goal_push_event(GoalEvent {
            at_ms,
            kind: "goal_pursuit".to_string(),
            text: message.content.content_text_only(),
        });
        session.add_message(message);
    }
    session.set_runtime_state(SessionState::Completed, None);
    Some(status)
}

fn terminal_goal_kind(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::NoProgress => "no_progress",
        _ => "budget_exhausted",
    }
}

fn goal_budget_exhausted_terminal_event(
    status: GoalStatus,
    trigger: &str,
    at_ms: u64,
) -> ChatMessage {
    let kind = terminal_goal_kind(status);
    internal_roles::event(
        EventSubkind::GoalPursuit,
        GOAL_VERIFIER_SOURCE,
        json!({
            "kind": kind,
            "trigger": trigger,
            "at_ms": at_ms,
        }),
        format!("Goal pursuit stopped before completion: {kind}."),
    )
}

async fn abort_goal_verification(session_arc: Arc<AMutex<ChatSession>>) {
    let mut session = session_arc.lock().await;
    if session.goal_status == Some(GoalStatus::Verifying) {
        session.goal_set_status(GoalStatus::Active);
    }
    session.set_runtime_state(SessionState::Idle, None);
}

pub fn goal_verification_message(goal_text: &str) -> ChatMessage {
    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "user".to_string(),
        content: ChatContent::SimpleText(format!(
            "[goal verification]\n\nSynthesized goal:\n{goal_text}\n\nAcceptance criteria:\nUse every explicit criterion in the synthesized goal above.\n\nJudge ONLY whether the work so far FULLY satisfies the goal and every criterion. Respond EXACTLY: first line `GOAL: MET` or `GOAL: UNMET`; if UNMET, a bullet list of unmet requirements; call NO tools."
        )),
        extra: serde_json::Map::from_iter([(
            "goal_verification".to_string(),
            json!({"hidden": true}),
        )]),
        ..Default::default()
    }
}

pub fn parse_goal_verdict(answer: &str, tool_called: bool) -> GoalVerdict {
    if tool_called {
        return inconclusive_verdict();
    }
    let mut lines = answer
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let Some(first) = lines.next() else {
        return inconclusive_verdict();
    };
    if first.eq_ignore_ascii_case("GOAL: MET") {
        return GoalVerdict::Met;
    }
    if first.eq_ignore_ascii_case("GOAL: UNMET") {
        let gaps = lines
            .map(|line| line.trim_start_matches(['-', '*', '•', ' ']).trim())
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if gaps.is_empty() {
            inconclusive_verdict()
        } else {
            GoalVerdict::Unmet(gaps)
        }
    } else {
        inconclusive_verdict()
    }
}

fn inconclusive_verdict() -> GoalVerdict {
    GoalVerdict::Unmet(vec![INCONCLUSIVE_GAP.to_string()])
}

struct GoalVerifierCollector;

impl StreamCollector for GoalVerifierCollector {
    fn on_delta_ops(&mut self, _choice_idx: usize, _ops: Vec<DeltaOp>) {}
    fn on_usage(&mut self, _usage: &ChatUsage) {}
    fn on_finish(&mut self, _choice_idx: usize, _finish_reason: Option<String>) {}
}

struct GoalVerifierPrepareInputs {
    thread: ThreadParams,
    messages: Vec<ChatMessage>,
    chat_id: String,
    tools: Vec<ToolDesc>,
    sampling_parameters: SamplingParameters,
    options: ChatPrepareOptions,
}

fn goal_text_for_verifier(session: &ChatSession) -> Result<String, String> {
    crate::chat::goal_role::synthesize_current_goal(session)
        .or_else(|| session.goal.as_ref().map(|goal| goal.content.clone()))
        .ok_or_else(|| "no active goal to verify".to_string())
}

fn verifier_sampling_parameters(thread: &ThreadParams) -> SamplingParameters {
    SamplingParameters {
        max_new_tokens: GOAL_VERIFIER_MAX_TOKENS,
        temperature: Some(0.0),
        boost_reasoning: false,
        reasoning_effort: thread
            .reasoning_effort
            .as_ref()
            .and_then(|s| match s.as_str() {
                "low" => Some(crate::call_validation::ReasoningEffort::Low),
                "medium" => Some(crate::call_validation::ReasoningEffort::Medium),
                "high" => Some(crate::call_validation::ReasoningEffort::High),
                "xhigh" => Some(crate::call_validation::ReasoningEffort::XHigh),
                "max" => Some(crate::call_validation::ReasoningEffort::Max),
                _ => None,
            }),
        thinking_budget: thread.thinking_budget,
        ..Default::default()
    }
}

fn goal_verifier_prepare_inputs_from_parts(
    mut thread: ThreadParams,
    mut messages: Vec<ChatMessage>,
    goal_text: String,
    chat_id: String,
    tools: Vec<ToolDesc>,
) -> GoalVerifierPrepareInputs {
    messages.push(goal_verification_message(&goal_text));
    thread.max_tokens = Some(GOAL_VERIFIER_MAX_TOKENS);
    thread.temperature = Some(0.0);
    let sampling_parameters = verifier_sampling_parameters(&thread);
    let options = ChatPrepareOptions {
        prepend_system_prompt: false,
        allow_at_commands: false,
        allow_tool_prerun: false,
        supports_tools: true,
        tool_choice: Some(ToolChoice::None),
        parallel_tool_calls: Some(false),
        cache_control: CacheControl::Ephemeral,
        frozen_request_prefix: thread.frozen_request_prefix.clone(),
    };
    GoalVerifierPrepareInputs {
        thread,
        messages,
        chat_id,
        tools,
        sampling_parameters,
        options,
    }
}

#[cfg(test)]
fn goal_verifier_prepare_inputs(
    session: &ChatSession,
    tools: Vec<ToolDesc>,
) -> Result<GoalVerifierPrepareInputs, String> {
    Ok(goal_verifier_prepare_inputs_from_parts(
        session.thread.clone(),
        session.messages.clone(),
        goal_text_for_verifier(session)?,
        session.chat_id.clone(),
        tools,
    ))
}

async fn verifier_cancel_requested(session_arc: &Arc<AMutex<ChatSession>>) -> bool {
    let session = session_arc.lock().await;
    session.user_interrupt_flag.load(Ordering::SeqCst)
}

pub async fn run_goal_verifier(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> Result<GoalVerifierReply, String> {
    let gcx = app.gcx.clone();
    let (thread, messages, goal_text, chat_id, abort_notify) = {
        let session = session_arc.lock().await;
        let goal_text = goal_text_for_verifier(&session)?;
        (
            session.thread.clone(),
            session.messages.clone(),
            goal_text,
            session.chat_id.clone(),
            session.abort_notify.clone(),
        )
    };

    if verifier_cancel_requested(&session_arc).await {
        return Err(ABORT_ERROR_MESSAGE.to_string());
    }

    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.message)?;
    let model_rec = crate::caps::resolve_chat_model(caps, &thread.model)?;
    let tools_for_gen = app
        .tool_registry
        .get_tools_index_for_mode(&thread.mode, Some(&model_rec.base.id))
        .await;
    let prepare_inputs = goal_verifier_prepare_inputs_from_parts(
        thread,
        messages,
        goal_text,
        chat_id,
        tools_for_gen.tools,
    );
    let model_n_ctx = if model_rec.base.n_ctx > 0 {
        model_rec.base.n_ctx
    } else {
        crate::chat::config::tokens().default_n_ctx
    };
    let effective_n_ctx = match prepare_inputs.thread.context_tokens_cap {
        Some(cap) if cap > 0 => cap.min(model_n_ctx),
        _ => model_n_ctx,
    };
    let tokenizer_arc = crate::tokens::cached_tokenizer(gcx.clone(), &model_rec.base).await?;
    let t = HasTokenizerAndEot::new(tokenizer_arc);
    let ccx = AtCommandsContext::new_from_app(
        app.clone(),
        effective_n_ctx,
        CHAT_TOP_N,
        false,
        prepare_inputs.messages.clone(),
        prepare_inputs.chat_id.clone(),
        prepare_inputs.thread.root_chat_id.clone(),
        model_rec.base.id.clone(),
        prepare_inputs.thread.task_meta.clone(),
        prepare_inputs.thread.worktree.clone(),
    )
    .await;
    let meta = ChatMeta {
        chat_id: prepare_inputs.chat_id,
        chat_mode: prepare_inputs.thread.mode.clone(),
        chat_remote: false,
        current_config_file: String::new(),
        context_tokens_cap: prepare_inputs.thread.context_tokens_cap,
        include_project_info: prepare_inputs.thread.include_project_info,
        request_attempt_id: Uuid::new_v4().to_string(),
        worktree: prepare_inputs.thread.worktree.clone(),
    };
    let mut sampling_parameters = prepare_inputs.sampling_parameters;
    let mut options = prepare_inputs.options;
    options.supports_tools = model_rec.supports_tools;
    let prepared = prepare_chat_passthrough(
        gcx,
        Arc::new(AMutex::new(ccx)),
        &t,
        prepare_inputs.messages,
        &prepare_inputs.thread,
        &model_rec.base.id,
        &prepare_inputs.thread.mode,
        prepare_inputs.tools,
        &meta,
        &mut sampling_parameters,
        &options,
    )
    .await?;

    if verifier_cancel_requested(&session_arc).await {
        return Err(ABORT_ERROR_MESSAGE.to_string());
    }

    let verifier_abort_flag = Arc::new(AtomicBool::new(false));
    let params = StreamRunParams {
        llm_request: prepared.llm_request,
        model_rec: model_rec.base.clone(),
        chat_id: None,
        allow_websocket: true,
        abort_flag: Some(verifier_abort_flag),
        abort_notify: Some(abort_notify),
        supports_tools: model_rec.supports_tools,
        supports_reasoning: model_rec.has_reasoning_support(),
        reasoning_type: model_rec.reasoning_type_string(),
        supports_temperature: model_rec.supports_temperature,
    };
    let mut collector = GoalVerifierCollector;
    let choices = run_llm_stream(app, params, &mut collector)
        .await
        .map_err(|error| error.message)
        .and_then(|outcome| match outcome {
            LlmStreamOutcome::Choices(choices) => Ok(choices),
            LlmStreamOutcome::PausedForCacheGuard => {
                Err("verification paused by cache guard".to_string())
            }
        })?;
    let choice = choices.into_iter().next().unwrap_or_default();
    Ok(goal_verifier_reply_from_choice(choice))
}

fn goal_verifier_reply_from_choice(choice: ChoiceFinal) -> GoalVerifierReply {
    let tool_called = !choice.tool_calls_raw.is_empty();
    let verdict = parse_goal_verdict(&choice.content, tool_called);
    let tokens = choice
        .usage
        .as_ref()
        .map(|usage| usage.total_tokens as u64)
        .unwrap_or(0);
    GoalVerifierReply {
        verdict,
        verifier_reply: choice.content,
        tokens,
    }
}

pub fn apply_goal_verdict(
    session: &mut ChatSession,
    trigger: &str,
    reply: GoalVerifierReply,
) -> GoalVerificationApplyOutcome {
    if session.goal.is_none() {
        return GoalVerificationApplyOutcome::NoGoal;
    }

    let at_ms = epoch_ms_now();
    session.goal_record_verifier_attempt(reply.tokens);
    let (verdict_text, gaps, event_kind, event_text) = match &reply.verdict {
        GoalVerdict::Met => (
            "met".to_string(),
            Vec::new(),
            "verified".to_string(),
            "Goal verification passed.".to_string(),
        ),
        GoalVerdict::Unmet(gaps) => (
            "unmet".to_string(),
            gaps.clone(),
            "verification_gaps".to_string(),
            format!("Goal verification found gaps:\n{}", gaps.join("\n")),
        ),
    };
    session.goal_push_attempt(GoalAttempt {
        at_ms,
        trigger: trigger.to_string(),
        verdict: verdict_text,
        gaps: gaps.clone(),
        verifier_reply: reply.verifier_reply,
    });
    session.add_message(internal_roles::event(
        EventSubkind::GoalPursuit,
        GOAL_VERIFIER_SOURCE,
        json!({"kind": event_kind, "at_ms": at_ms, "gaps": gaps}),
        event_text,
    ));

    match reply.verdict {
        GoalVerdict::Met => {
            session.goal_set_status(GoalStatus::Completed);
            session.set_runtime_state(SessionState::Completed, None);
            GoalVerificationApplyOutcome::Finalized
        }
        GoalVerdict::Unmet(_) => {
            session.goal_set_status(GoalStatus::Active);
            session.set_runtime_state(SessionState::Idle, None);
            let _ = session.enqueue_priority_command(CommandRequest {
                client_request_id: format!("goal-verifier-regenerate-{}", Uuid::new_v4()),
                priority: true,
                command: ChatCommand::Regenerate {},
            });
            GoalVerificationApplyOutcome::Rearmed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_chat_api::FrozenRequestPrefix;
    use refact_tool_api::{ToolSource, ToolSourceType};
    use serde_json::json;

    fn session_with_goal() -> ChatSession {
        let mut session = ChatSession::new("goal-verifier-test".to_string());
        session.install_goal(
            "agent",
            "ship feature\n- tests pass",
            true,
            GoalBudget::default(),
        );
        session.set_runtime_state(SessionState::ExecutingTools, None);
        session
    }

    fn finite_goal_budget() -> GoalBudget {
        GoalBudget {
            max_turns: Some(3),
            max_minutes: None,
            max_tokens: None,
            cooldown_ms: 1_500,
            no_progress_token_threshold: 50,
            no_progress_turns: Some(2),
        }
    }

    fn use_finite_goal_budget(session: &mut ChatSession) -> GoalBudget {
        let budget = finite_goal_budget();
        session.goal.as_mut().unwrap().budget = budget.clone();
        budget
    }

    fn tool_desc(name: &str) -> ToolDesc {
        ToolDesc {
            name: name.to_string(),
            display_name: name.to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: format!("{name} tool"),
            input_schema: json!({"type": "object", "properties": {}}),
            output_schema: None,
            annotations: None,
        }
    }

    fn drain_runtime_states(session: &mut ChatSession) -> Vec<SessionState> {
        let mut rx = session.subscribe();
        session.set_runtime_state(SessionState::ExecutingTools, None);
        session.goal_set_status(GoalStatus::Verifying);
        apply_goal_verdict(
            session,
            "task_done",
            GoalVerifierReply {
                verdict: GoalVerdict::Met,
                verifier_reply: "GOAL: MET".to_string(),
                tokens: 7,
            },
        );
        let mut states = Vec::new();
        while let Ok(raw) = rx.try_recv() {
            let envelope: EventEnvelope = serde_json::from_str(&raw).unwrap();
            if let ChatEvent::RuntimeUpdated { state, .. } = envelope.event {
                states.push(state);
            }
        }
        states
    }

    #[test]
    fn goal_verification_message_contains_required_contract() {
        let message = goal_verification_message("Do the thing");
        let text = message.content.content_text_only();
        assert_eq!(message.role, "user");
        assert!(text.contains("[goal verification]"));
        assert!(text.contains("Do the thing"));
        assert!(text.contains("GOAL: MET"));
        assert!(text.contains("GOAL: UNMET"));
        assert!(text.contains("call NO tools"));
    }

    #[test]
    fn parse_goal_verdict_met() {
        assert_eq!(parse_goal_verdict("GOAL: MET", false), GoalVerdict::Met);
    }

    #[test]
    fn parse_goal_verdict_unmet_bullets() {
        assert_eq!(
            parse_goal_verdict("GOAL: UNMET\n- missing tests\n* docs", false),
            GoalVerdict::Unmet(vec!["missing tests".to_string(), "docs".to_string()])
        );
    }

    #[test]
    fn parse_goal_verdict_tool_call_or_empty_is_unmet() {
        assert_eq!(
            parse_goal_verdict("GOAL: MET", true),
            GoalVerdict::Unmet(vec![INCONCLUSIVE_GAP.to_string()])
        );
        assert_eq!(
            parse_goal_verdict("", false),
            GoalVerdict::Unmet(vec![INCONCLUSIVE_GAP.to_string()])
        );
        assert_eq!(
            parse_goal_verdict("maybe", false),
            GoalVerdict::Unmet(vec![INCONCLUSIVE_GAP.to_string()])
        );
    }

    #[test]
    fn begin_goal_verification_sets_verifying_without_completed() {
        let mut session = session_with_goal();
        let mut rx = session.subscribe();

        let begin = begin_goal_verification_if_needed(&mut session);

        assert_eq!(begin, GoalVerificationBegin::Started);
        assert_eq!(session.goal_status, Some(GoalStatus::Verifying));
        let mut saw_verifying = false;
        while let Ok(raw) = rx.try_recv() {
            let envelope: EventEnvelope = serde_json::from_str(&raw).unwrap();
            if let ChatEvent::RuntimeUpdated {
                goal_status, state, ..
            } = envelope.event
            {
                assert_ne!(state, SessionState::Completed);
                if goal_status == Some(GoalStatus::Verifying) {
                    saw_verifying = true;
                }
            }
        }
        assert!(saw_verifying);
    }

    #[test]
    fn begin_goal_verification_skips_budget_exhausted_and_no_goal() {
        let mut no_goal = ChatSession::new("no-goal".to_string());
        assert_eq!(
            begin_goal_verification_if_needed(&mut no_goal),
            GoalVerificationBegin::NoActiveGoal
        );

        let mut exhausted = session_with_goal();
        let budget = use_finite_goal_budget(&mut exhausted);
        exhausted.goal.as_mut().unwrap().progress.turns_used = budget.max_turns.unwrap();
        assert_eq!(
            begin_goal_verification_if_needed(&mut exhausted),
            GoalVerificationBegin::BudgetExhausted
        );
    }

    #[tokio::test]
    async fn budget_exhausted_finish_records_terminal_status() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let mut session = session_with_goal();
        let budget = use_finite_goal_budget(&mut session);
        session.goal.as_mut().unwrap().progress.turns_used = budget.max_turns.unwrap();
        assert!(should_verify_goal_on_done(&session));
        let session_arc = Arc::new(AMutex::new(session));

        let outcome = verify_goal_before_completion(app, session_arc.clone(), "task_done").await;

        assert_eq!(
            outcome,
            GoalCompletionGateOutcome::BudgetExhausted(GoalStatus::BudgetExhausted)
        );
        let session = session_arc.lock().await;
        let goal = session.goal.as_ref().unwrap();
        assert_eq!(goal.status, GoalStatus::BudgetExhausted);
        assert!(goal.attempts.is_empty());
        assert!(goal.events.iter().any(|event| {
            event.kind == "goal_pursuit" && event.text.contains("budget_exhausted")
        }));
        assert!(!session.messages.iter().any(|message| {
            message
                .extra
                .get("event")
                .and_then(|event| event.get("payload"))
                .and_then(|payload| payload.get("kind"))
                .and_then(|kind| kind.as_str())
                == Some("verified")
        }));
    }

    #[tokio::test]
    async fn no_progress_exhausted_finish_records_terminal_status() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let mut session = session_with_goal();
        let budget = use_finite_goal_budget(&mut session);
        session.goal.as_mut().unwrap().progress.no_progress_turns =
            budget.no_progress_turns.unwrap();
        assert!(should_verify_goal_on_done(&session));
        let session_arc = Arc::new(AMutex::new(session));

        let outcome = verify_goal_before_completion(app, session_arc.clone(), "task_done").await;

        assert_eq!(
            outcome,
            GoalCompletionGateOutcome::BudgetExhausted(GoalStatus::NoProgress)
        );
        let session = session_arc.lock().await;
        let goal = session.goal.as_ref().unwrap();
        assert_eq!(goal.status, GoalStatus::NoProgress);
        assert!(goal
            .events
            .iter()
            .any(|event| { event.kind == "goal_pursuit" && event.text.contains("no_progress") }));
    }

    #[test]
    fn verifier_goal_text_uses_synthesized_goal_with_deltas() {
        let mut session = session_with_goal();
        session.add_message(internal_roles::goal_delta(
            "tool.update_goal",
            json!({"seq": 1}),
            "also update docs",
        ));

        let inputs = goal_verifier_prepare_inputs(&session, vec![]).unwrap();
        let verifier_text = inputs.messages.last().unwrap().content.content_text_only();

        assert!(verifier_text.contains("ship feature"));
        assert!(verifier_text.contains("## Goal updates"));
        assert!(verifier_text.contains("also update docs"));
        assert_eq!(
            goal_text_for_verifier(&session).unwrap(),
            crate::chat::goal_role::synthesize_current_goal(&session).unwrap()
        );
    }

    #[tokio::test]
    async fn verifier_cancel_uses_user_interrupt_not_existing_finish_abort() {
        let session_arc = Arc::new(AMutex::new(session_with_goal()));
        {
            let session = session_arc.lock().await;
            session.abort_flag.store(true, Ordering::SeqCst);
        }

        assert!(!verifier_cancel_requested(&session_arc).await);
        {
            let session = session_arc.lock().await;
            assert!(session.abort_flag.load(Ordering::SeqCst));
            session.user_interrupt_flag.store(true, Ordering::SeqCst);
        }
        assert!(verifier_cancel_requested(&session_arc).await);
    }

    #[test]
    fn verifier_prepare_inputs_preserve_parent_cache_and_messages() {
        let mut session = session_with_goal();
        session.thread.frozen_request_prefix = Some(FrozenRequestPrefix {
            schema_version: 1,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            system_prompt: Some("cached system".to_string()),
            tools_canonical: Some(json!([{"name": "finish"}])),
        });
        session.add_message(ChatMessage {
            message_id: "user-1".to_string(),
            role: "user".to_string(),
            content: ChatContent::SimpleText("please ship".to_string()),
            ..Default::default()
        });
        session.add_message(ChatMessage {
            message_id: "assistant-1".to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("working".to_string()),
            thinking_blocks: Some(vec![json!({"type": "thinking", "signature": "sig-1"})]),
            ..Default::default()
        });
        let original_messages = session.messages.clone();
        let tools = vec![tool_desc("task_done")];

        let inputs = goal_verifier_prepare_inputs(&session, tools.clone()).unwrap();

        assert_eq!(inputs.messages.len(), original_messages.len() + 1);
        for (left, right) in inputs.messages.iter().zip(original_messages.iter()) {
            assert_eq!(left.message_id, right.message_id);
            assert_eq!(left.role, right.role);
            assert_eq!(
                left.content.content_text_only(),
                right.content.content_text_only()
            );
            assert_eq!(left.thinking_blocks, right.thinking_blocks);
        }
        let appended = inputs.messages.last().unwrap();
        assert_eq!(appended.role, "user");
        assert!(appended.extra.contains_key("goal_verification"));
        assert_eq!(
            inputs.options.frozen_request_prefix,
            session.thread.frozen_request_prefix
        );
        assert!(!inputs.options.prepend_system_prompt);
        assert!(!inputs.options.allow_at_commands);
        assert!(!inputs.options.allow_tool_prerun);
        assert!(matches!(inputs.options.tool_choice, Some(ToolChoice::None)));
        assert_eq!(inputs.options.parallel_tool_calls, Some(false));
        assert_eq!(
            serde_json::to_value(&inputs.tools).unwrap(),
            serde_json::to_value(&tools).unwrap()
        );
    }

    #[test]
    fn apply_goal_verdict_met_finalizes_completed_and_records_attempt() {
        let mut session = session_with_goal();
        session.goal_set_status(GoalStatus::Verifying);

        let outcome = apply_goal_verdict(
            &mut session,
            "task_done",
            GoalVerifierReply {
                verdict: GoalVerdict::Met,
                verifier_reply: "GOAL: MET".to_string(),
                tokens: 11,
            },
        );

        assert_eq!(outcome, GoalVerificationApplyOutcome::Finalized);
        assert_eq!(session.runtime.state, SessionState::Completed);
        let goal = session.goal.as_ref().unwrap();
        assert_eq!(goal.status, GoalStatus::Completed);
        assert_eq!(goal.progress.turns_used, 1);
        assert_eq!(goal.progress.tokens_used, 11);
        assert_eq!(goal.attempts.len(), 1);
        assert_eq!(goal.attempts[0].verifier_reply, "GOAL: MET");
        assert!(session.messages.iter().any(|message| {
            message
                .extra
                .get("event")
                .and_then(|event| event.get("payload"))
                == Some(&json!({"kind": "verified", "at_ms": goal.attempts[0].at_ms, "gaps": []}))
        }));
    }

    #[test]
    fn apply_goal_verdict_unmet_rearms_and_enqueues_regenerate() {
        let mut session = session_with_goal();
        session.goal_set_status(GoalStatus::Verifying);

        let outcome = apply_goal_verdict(
            &mut session,
            "task_done",
            GoalVerifierReply {
                verdict: GoalVerdict::Unmet(vec!["missing test".to_string()]),
                verifier_reply: "GOAL: UNMET\n- missing test".to_string(),
                tokens: 13,
            },
        );

        assert_eq!(outcome, GoalVerificationApplyOutcome::Rearmed);
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
        assert_eq!(
            session.goal.as_ref().unwrap().attempts[0].gaps,
            vec!["missing test"]
        );
        assert_eq!(
            session.goal.as_ref().unwrap().attempts[0].verifier_reply,
            "GOAL: UNMET\n- missing test"
        );
        assert!(session
            .command_queue
            .iter()
            .any(|request| matches!(request.command, ChatCommand::Regenerate {})));
    }

    #[test]
    fn no_completed_status_emitted_before_met_verdict() {
        let mut session = session_with_goal();
        let states = drain_runtime_states(&mut session);
        let completed_idx = states
            .iter()
            .position(|state| *state == SessionState::Completed)
            .unwrap();
        assert!(states[..completed_idx]
            .iter()
            .all(|state| *state != SessionState::Completed));
        assert!(states
            .iter()
            .any(|state| *state == SessionState::ExecutingTools));
    }

    #[test]
    fn finish_with_no_goal_is_passthrough() {
        let mut session = ChatSession::new("plain".to_string());
        assert_eq!(
            begin_goal_verification_if_needed(&mut session),
            GoalVerificationBegin::NoActiveGoal
        );
        assert!(!should_verify_goal_on_done(&session));
    }
}
