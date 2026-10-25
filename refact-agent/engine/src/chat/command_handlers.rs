use super::*;
use crate::chat::{browser_decisions, delivery, session, trajectories, types};
use crate::chat::browser_context;
use crate::call_validation::ContextFile;
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{ControlVerdict, HookPayload, get_project_dir_string, run_control_hooks};
use crate::ext::slash_expand::expand_slash_command;
use crate::ext::skills_context::{expand_skill_includes, SKILLS_CONTEXT_MARKER};
use crate::chat::generation::prepare_session_preamble_and_knowledge;
use crate::buddy::chat_reactions::{maybe_enqueue_chat_reaction, AcceptedUserMessage};
use refact_buddy_core::user_action::UserAction;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum QueueCommandOutcome {
    Continue,
    Finished,
}

struct QueueCommandContext {
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    request_id: String,
    priority: bool,
    native_guard: Option<refact_chat_api::NativeApprovalGuard>,
    supervision_guard: Option<refact_chat_api::SupervisionCommandGuard>,
}

pub(super) async fn handle_queue_command(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    request: CommandRequest,
) -> QueueCommandOutcome {
    let context = QueueCommandContext {
        app,
        session_arc,
        request_id: request.client_request_id,
        priority: request.priority,
        native_guard: request.native_approval_guard,
        supervision_guard: request.supervision_guard,
    };
    match request.command {
        ChatCommand::SetGoal {
            content,
            budget,
            criteria,
        } => Box::pin(handle_set_goal(&context, content, budget, criteria)).await,
        ChatCommand::SetGoalBudget { budget } => {
            Box::pin(handle_set_goal_budget(&context, budget)).await
        }
        ChatCommand::UpdateGoal { note } => Box::pin(handle_update_goal(&context, note)).await,
        ChatCommand::GoalControl { action } => {
            Box::pin(handle_goal_control(&context, action)).await
        }
        ChatCommand::UserMessage {
            content,
            attachments,
            context_files,
            suppress_auto_enrichment,
            client_message_id,
        } => {
            Box::pin(handle_user_message(
                &context,
                content,
                attachments,
                context_files,
                suppress_auto_enrichment,
                client_message_id,
            ))
            .await
        }
        ChatCommand::RetryFromIndex {
            index,
            content,
            attachments,
        } => {
            Box::pin(handle_retry_from_index(
                &context,
                index,
                content,
                attachments,
            ))
            .await
        }
        ChatCommand::SetParams { patch } => Box::pin(handle_set_params(&context, patch)).await,
        ChatCommand::Abort {} => Box::pin(handle_abort(&context)).await,
        ChatCommand::AnswerQuestions {
            tool_call_id,
            answers,
            asked_at_ms,
        } => {
            Box::pin(handle_answer_questions(
                &context,
                tool_call_id,
                answers,
                asked_at_ms,
            ))
            .await
        }
        ChatCommand::CleanBackgroundProcesses { include_services } => {
            Box::pin(handle_clean_background_processes(
                &context,
                include_services,
            ))
            .await
        }
        ChatCommand::ToolDecision {
            tool_call_id,
            accepted,
        } => Box::pin(handle_tool_decision(&context, tool_call_id, accepted)).await,
        ChatCommand::ToolDecisions { decisions } => {
            Box::pin(handle_tool_decisions(&context, decisions)).await
        }
        ChatCommand::IdeToolResult {
            tool_call_id,
            content,
            tool_failed,
        } => {
            Box::pin(handle_ide_tool_result(
                &context,
                tool_call_id,
                content,
                tool_failed,
            ))
            .await
        }
        ChatCommand::UpdateMessage {
            message_id,
            content,
            attachments,
            regenerate,
        } => {
            Box::pin(handle_update_message(
                &context,
                message_id,
                content,
                attachments,
                regenerate,
            ))
            .await
        }
        ChatCommand::RemoveMessage {
            message_id,
            regenerate,
        } => Box::pin(handle_remove_message(&context, message_id, regenerate)).await,
        ChatCommand::Regenerate {} => Box::pin(handle_regenerate(&context)).await,
        ChatCommand::RestoreMessages { messages } => {
            Box::pin(handle_restore_messages(&context, messages)).await
        }
        ChatCommand::BranchFromChat {
            source_chat_id,
            up_to_message_id,
        } => {
            Box::pin(handle_branch_from_chat(
                &context,
                source_chat_id,
                up_to_message_id,
            ))
            .await
        }
        ChatCommand::BrowserContextDecision {
            pending_message_id,
            include_actions,
            include_console,
            include_network,
            include_mutations,
            include_screenshot,
            last_n_actions,
            last_n_console,
            last_n_network,
        } => {
            Box::pin(handle_browser_context_decision(
                &context,
                pending_message_id,
                include_actions,
                include_console,
                include_network,
                include_mutations,
                include_screenshot,
                last_n_actions,
                last_n_console,
                last_n_network,
            ))
            .await
        }
        ChatCommand::DeliverMessages { delivery } => {
            Box::pin(handle_deliver_messages(&context, delivery)).await
        }
        ChatCommand::UpdatePendingDelivery {
            delivery_id,
            push,
            cancel,
        } => {
            Box::pin(handle_update_pending_delivery(
                &context,
                delivery_id,
                push,
                cancel,
            ))
            .await
        }
    }
}

async fn handle_set_goal(
    context: &QueueCommandContext,
    content: String,
    budget: Option<GoalBudget>,
    criteria: Option<Vec<GoalCriterion>>,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let request_id = &context.request_id;
    let supervision_guard = context.supervision_guard.as_ref();
    let policy = crate::chat::verify_cmd::VerifyCommandPolicy::load(app.gcx.clone()).await;
    if let Err(error) = crate::chat::evidence::validate_goal_verify_commands(
        criteria.as_deref().unwrap_or_default(),
        &policy,
    ) {
        reject_guard(&mut *session_arc.lock().await, request_id, error);
        return QueueCommandOutcome::Continue;
    }
    let result = {
        let mut session = session_arc.lock().await;
        let chat_id = session.chat_id.clone();
        match crate::buddy::supervision_authority::with_command_guard(
            supervision_guard,
            &chat_id,
            || {
                Ok(handle_set_goal_command(
                    &mut session,
                    content,
                    budget,
                    criteria,
                ))
            },
        ) {
            Ok(result) => result,
            Err(error) => {
                reject_guard(&mut session, &request_id, error);
                return QueueCommandOutcome::Continue;
            }
        }
    };
    match result {
        Ok(_) => {
            maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await
        }
        Err(error) => {
            warn!("SetGoal command rejected: {}", error);
            let mut session = session_arc.lock().await;
            emit_goal_command_error(&mut session, error);
        }
    }
    QueueCommandOutcome::Finished
}

async fn handle_set_goal_budget(
    context: &QueueCommandContext,
    budget: GoalBudget,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let result = {
        let mut session = session_arc.lock().await;
        handle_set_goal_budget_command(&mut session, budget)
    };
    match result {
        Ok(_) => {
            maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await
        }
        Err(error) => {
            warn!("SetGoalBudget command rejected: {}", error);
            let mut session = session_arc.lock().await;
            emit_goal_command_error(&mut session, error);
        }
    }
    QueueCommandOutcome::Finished
}

async fn handle_update_goal(context: &QueueCommandContext, note: String) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let policy = crate::chat::verify_cmd::VerifyCommandPolicy::load(app.gcx.clone()).await;
    let result = {
        let mut session = session_arc.lock().await;
        handle_update_goal_command(&mut session, note).map(|result| {
            for advisory in crate::chat::evidence::unverifiable_goal_criteria(&session, &policy) {
                tracing::info!(chat_id = %session.chat_id, %advisory, "UpdateGoal appended with advisory");
            }
            result
        })
    };
    match result {
        Ok(_) => {
            maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await
        }
        Err(error) => {
            warn!("UpdateGoal command rejected: {}", error);
            let mut session = session_arc.lock().await;
            emit_goal_command_error(&mut session, error);
        }
    }
    QueueCommandOutcome::Finished
}

async fn handle_goal_control(context: &QueueCommandContext, action: String) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let request_id = &context.request_id;
    let supervision_guard = context.supervision_guard.as_ref();
    let result = {
        let mut session = session_arc.lock().await;
        let chat_id = session.chat_id.clone();
        match crate::buddy::supervision_authority::with_command_guard(
            supervision_guard,
            &chat_id,
            || Ok(handle_goal_control_command(&mut session, action)),
        ) {
            Ok(result) => result,
            Err(error) => {
                reject_guard(&mut session, &request_id, error);
                return QueueCommandOutcome::Continue;
            }
        }
    };
    match result {
        Ok(_) => {
            maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await
        }
        Err(error) => {
            warn!("GoalControl command rejected: {}", error);
            let mut session = session_arc.lock().await;
            emit_goal_command_error(&mut session, error);
        }
    }
    QueueCommandOutcome::Finished
}

async fn handle_retry_from_index(
    context: &QueueCommandContext,
    index: usize,
    content: serde_json::Value,
    attachments: Vec<serde_json::Value>,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let mut session = session_arc.lock().await;
    session.truncate_messages(index);
    let parsed_content = parse_content_with_attachments(&content, &attachments);
    let user_message = ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "user".to_string(),
        content: parsed_content,
        ..Default::default()
    };
    session.add_message(user_message);
    note_user_turn_resets_goal_pursuit(&mut session);
    drop(session);

    maybe_save_trajectory_background_with_intent(
        app.clone(),
        session_arc.clone(),
        TrajectoryCommitIntent::Checkpoint,
    );
    prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone()).await;
    if aborted_before_start_generation(&session_arc).await {
        return QueueCommandOutcome::Continue;
    }
    start_generation(app.clone(), session_arc.clone()).await;
    QueueCommandOutcome::Finished
}

async fn handle_set_params(
    context: &QueueCommandContext,
    patch: serde_json::Value,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    if !patch.is_object() {
        warn!("SetParams patch must be an object, ignoring");
        return QueueCommandOutcome::Continue;
    }
    let mode_switch_reason = patch
        .get("reason")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let (chat_id, thread_before) = {
        let session = session_arc.lock().await;
        (session.chat_id.clone(), session.thread.clone())
    };
    let worktree_update = match resolve_worktree_setparams_update(
        app.clone(),
        &chat_id,
        &thread_before,
        &patch,
    )
    .await
    {
        Ok(update) => update,
        Err(e) => {
            warn!("SetParams worktree update rejected: {}", e);
            let mut session = session_arc.lock().await;
            reject_guard(&mut session, &context.request_id, e.clone());
            let compression_phase = session.compression_phase;
            let compression_reason = session.compression_reason;
            let event = session.runtime_update_event(
                SessionState::Error,
                Some(e),
                false,
                compression_phase,
                compression_reason,
            );
            session.emit(event);
            session.set_runtime_state(SessionState::Idle, None);
            return QueueCommandOutcome::Continue;
        }
    };
    let mut session = session_arc.lock().await;
    let old_model = session.thread.model.clone();
    let old_mode = session.thread.mode.clone();
    let (mut changed, sanitized_patch) = apply_setparams_patch(&mut session.thread, &patch);
    let mode_changed = session.thread.mode != old_mode;
    let worktree_message = worktree_update
        .as_ref()
        .filter(|update| update.changed)
        .map(|update| match update.worktree.as_ref() {
            Some(worktree) => worktree_activation_message(worktree),
            None => worktree_disabled_message(update.previous_worktree.as_ref()),
        });
    if let Some(update) = worktree_update.clone() {
        session.thread.worktree = update.worktree;
        changed |= update.changed;
    }

    let title_in_patch = patch.get("title").and_then(|v| v.as_str());
    let is_gen_in_patch = patch.get("is_title_generated").and_then(|v| v.as_bool());
    if let Some(title) = title_in_patch {
        let is_generated = is_gen_in_patch.unwrap_or(false);
        session.set_title(title.to_string(), is_generated);
    } else if let Some(is_gen) = is_gen_in_patch {
        if session.thread.is_title_generated != is_gen {
            let title = session.thread.title.clone();
            session.set_title(title, is_gen);
            changed = true;
        }
    }
    let mut patch_for_chat_sse = sanitized_patch;
    if let Some(obj) = patch_for_chat_sse.as_object_mut() {
        obj.remove("title");
        obj.remove("is_title_generated");
        if let Some(update) = worktree_update {
            obj.insert("worktree".to_string(), update.sse_value);
        }
    }
    session.emit(ChatEvent::ThreadUpdated {
        params: patch_for_chat_sse,
    });
    if changed {
        session.increment_version();
        session.touch();
    }
    if mode_changed {
        add_mode_switch_event_and_plan_if_changed(
            app.clone(),
            &mut session,
            &old_mode,
            mode_switch_reason.as_deref(),
            "chat.session",
        )
        .await;
    }
    if mode_changed || session.thread.model != old_model {
        session.release_turn_only_state();
    }
    if let Some(message) = worktree_message {
        session.add_message(message);
    }
    drop(session);
    if changed {
        maybe_save_trajectory_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Required,
        )
        .await;
    }
    QueueCommandOutcome::Finished
}

fn pending_attention_questions(
    app: &AppState,
    session: &ChatSession,
) -> Vec<(String, Vec<String>)> {
    let budget = app.attention();
    session
        .pending_questions
        .iter()
        .filter(|set| set.status == PendingQuestionStatus::Pending)
        .map(|set| {
            let waiter = format!("{}:{}", session.chat_id, set.tool_call_id);
            let question_ids = budget.questions_for_waiter(&waiter);
            (waiter, question_ids)
        })
        .collect()
}

fn leave_attention_questions(app: &AppState, questions: Vec<(String, Vec<String>)>) {
    let budget = app.attention();
    let now = Utc::now();
    for (waiter, question_ids) in questions {
        for question_id in question_ids {
            if let Err(error) = budget.leave(&question_id, &waiter, now) {
                tracing::debug!(%error, %question_id, %waiter, "attention waiter already left");
            }
        }
    }
}

async fn handle_abort(context: &QueueCommandContext) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let mut session = session_arc.lock().await;
    session.suppress_delivery_wakes();
    session.abort_stream();
    if session.planner_paused() || session.planner_pause_pending_tools {
        session.planner_pause_pending_tools = false;
        session.clear_pending_tool_calls_for_interruption();
    }
    let attention_questions = pending_attention_questions(&app, &session);
    let questions_cancelled = session.cancel_pending_questions();
    let browser_cancelled =
        browser_decisions::cancel_pending_browser_message(&mut session.pending_browser_message)
            .is_some();
    if browser_cancelled {
        session.mark_persisted_runtime_changed();
    }
    let goal_stopped = session.stop_goal_on_manual_abort();
    drop(session);
    leave_attention_questions(&app, attention_questions);
    if goal_stopped
        || questions_cancelled
        || browser_cancelled
        || session_arc.lock().await.trajectory_dirty
    {
        maybe_save_trajectory_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Required,
        )
        .await;
    }
    QueueCommandOutcome::Finished
}

async fn handle_answer_questions(
    context: &QueueCommandContext,
    tool_call_id: String,
    answers: Vec<QuestionAnswer>,
    asked_at_ms: Option<u64>,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let request_id = &context.request_id;
    let supervision_guard = context.supervision_guard.as_ref();
    let content = answer_questions_content(&tool_call_id, &answers);
    let resolution_id = request_id.clone();
    let timeout_default_notice = request_id
        .strip_prefix("attention-timeout:")
        .and_then(|question_id| app.questions().get(question_id))
        .and_then(|question| question.default_answer);
    let mut staged_message_id: Option<String> = None;
    let mut attention_questions = Vec::new();
    let resolved = {
        let mut session = session_arc.lock().await;
        let chat_id = session.chat_id.clone();
        match crate::buddy::supervision_authority::with_command_guard(
            supervision_guard,
            &chat_id,
            || {
                session.answer_pending_questions_at(
                    &tool_call_id,
                    answers,
                    resolution_id.clone(),
                    asked_at_ms,
                )
            },
        ) {
            Ok(resolved) => {
                if resolved {
                    if !request_id.starts_with("attention-timeout:") {
                        let waiter = format!("{chat_id}:{tool_call_id}");
                        let question_ids = app.attention().questions_for_waiter(&waiter);
                        attention_questions.push((waiter, question_ids));
                    }
                    if let (Some(question_id), Some(default_answer)) = (
                        request_id.strip_prefix("attention-timeout:"),
                        timeout_default_notice,
                    ) {
                        session.add_message(crate::chat::internal_roles::event(
                            crate::chat::internal_roles::EventSubkind::SystemNotice,
                            "chat.attention_timeout",
                            serde_json::json!({ "question_id": question_id }),
                            format!("No answer before the deadline; applied the documented default: {default_answer}"),
                        ));
                    }
                    let message = user_message_with_client_message_id(
                        ChatContent::SimpleText(content),
                        Vec::new(),
                        Some(request_id.clone()),
                    );
                    staged_message_id = Some(message.message_id.clone());
                    session.add_message(message);
                }
                resolved
                    || session.pending_questions.iter().any(|set| {
                        set.tool_call_id == tool_call_id
                            && set.status == PendingQuestionStatus::Answered
                            && set.resolution_id.as_deref() == Some(resolution_id.as_str())
                    })
            }
            Err(error) => {
                reject_question_answers(&mut session, &resolution_id, error);
                return QueueCommandOutcome::Continue;
            }
        }
    };
    if resolved {
        if let Err(error) = trajectories::try_save_trajectory_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Required,
        )
        .await
        {
            let mut session = session_arc.lock().await;
            let rolled_back = session.rollback_answered_questions(
                &tool_call_id,
                &resolution_id,
                staged_message_id.as_deref(),
            );
            let error = if rolled_back {
                format!(
                "failed to persist question answers: {error}; the answer was not applied, please answer again"
            )
            } else {
                format!("failed to persist question answers: {error}")
            };
            release_answer_questions_failure(&mut session, &resolution_id, error, rolled_back);
            return QueueCommandOutcome::Continue;
        }
        leave_attention_questions(&app, attention_questions);
        match crate::agents::approval::answer_from_user(
            &app,
            &session_arc,
            &tool_call_id,
            &resolution_id,
        )
        .await
        {
            Ok(true) => return QueueCommandOutcome::Continue,
            Ok(false) => {}
            Err(error) => {
                let mut session = session_arc.lock().await;
                reject_question_answers(&mut session, &resolution_id, error);
                return QueueCommandOutcome::Continue;
            }
        }
        {
            let mut session = session_arc.lock().await;
            let chat_id = session.chat_id.clone();
            if let Err(error) = crate::buddy::supervision_authority::with_command_guard(
                supervision_guard,
                &chat_id,
                || {
                    note_user_turn_resets_goal_pursuit(&mut session);
                    session.set_runtime_state(SessionState::Generating, None);
                    Ok(())
                },
            ) {
                reject_guard(&mut session, &resolution_id, error);
                return QueueCommandOutcome::Continue;
            }
        }
        prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone()).await;
        {
            let session = session_arc.lock().await;
            if !session.user_interrupt_flag.load(Ordering::SeqCst) {
                session.abort_flag.store(false, Ordering::SeqCst);
            }
        }
        if aborted_before_start_generation(&session_arc).await {
            return QueueCommandOutcome::Continue;
        }
        start_generation(app.clone(), session_arc.clone()).await;
    } else {
        let mut session = session_arc.lock().await;
        if session.runtime.state == SessionState::Generating {
            let state = next_state_after_released_answer(&session);
            session.set_runtime_state(state, None);
        }
        session.queue_notify.notify_one();
    }
    QueueCommandOutcome::Finished
}

async fn handle_clean_background_processes(
    context: &QueueCommandContext,
    include_services: bool,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let chat_id = {
        let session = session_arc.lock().await;
        session.chat_id.clone()
    };
    match session::clean_background_processes_for_chat(app.clone(), &chat_id, include_services)
        .await
    {
        Ok(killed) => {
            let mut session = session_arc.lock().await;
            session.add_background_process_cleanup_notice(killed.len());
            drop(session);
            maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await;
        }
        Err(error) => {
            warn!("CleanBackgroundProcesses failed: {}", error);
            let mut session = session_arc.lock().await;
            let compression_phase = session.compression_phase;
            let compression_reason = session.compression_reason;
            let event = session.runtime_update_event(
                SessionState::Error,
                Some(error),
                false,
                compression_phase,
                compression_reason,
            );
            session.emit(event);
            session.set_runtime_state(SessionState::Idle, None);
        }
    }
    QueueCommandOutcome::Finished
}

async fn handle_tool_decision(
    context: &QueueCommandContext,
    tool_call_id: String,
    accepted: bool,
) -> QueueCommandOutcome {
    let request_id = &context.request_id;
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let native_guard = context.native_guard.as_ref();
    let decisions = vec![ToolDecisionItem {
        tool_call_id: tool_call_id.clone(),
        accepted,
    }];
    super::handle_tool_decisions_guarded(
        app.clone(),
        session_arc.clone(),
        &decisions,
        native_guard,
        &request_id,
    )
    .await;
    QueueCommandOutcome::Finished
}

async fn handle_tool_decisions(
    context: &QueueCommandContext,
    decisions: Vec<ToolDecisionItem>,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let request_id = &context.request_id;
    let native_guard = context.native_guard.as_ref();
    super::handle_tool_decisions_guarded(
        app.clone(),
        session_arc.clone(),
        &decisions,
        native_guard,
        &request_id,
    )
    .await;
    QueueCommandOutcome::Finished
}

async fn handle_ide_tool_result(
    context: &QueueCommandContext,
    tool_call_id: String,
    content: String,
    tool_failed: bool,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let mut session = session_arc.lock().await;
    let completed = session.record_ide_tool_result(tool_call_id, content, tool_failed);
    if completed {
        session.release_turn_only_state();
        session.set_runtime_state(SessionState::Idle, None);
    }
    drop(session);
    if !completed {
        return QueueCommandOutcome::Continue;
    }
    if aborted_before_start_generation(&session_arc).await {
        return QueueCommandOutcome::Continue;
    }
    start_generation(app.clone(), session_arc.clone()).await;
    QueueCommandOutcome::Finished
}

async fn handle_update_message(
    context: &QueueCommandContext,
    message_id: String,
    content: serde_json::Value,
    attachments: Vec<serde_json::Value>,
    regenerate: bool,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let mut session = session_arc.lock().await;
    if session.runtime.state == SessionState::Generating {
        session.abort_stream();
    }
    let parsed_content = parse_content_with_attachments(&content, &attachments);
    if let Some(idx) = session
        .messages
        .iter()
        .position(|m| m.message_id == message_id)
    {
        let mut updated_msg = session.messages[idx].clone();
        updated_msg.content = parsed_content;
        session.update_message(&message_id, updated_msg);
        if regenerate && idx + 1 < session.messages.len() {
            session.truncate_messages(idx + 1);
            session.set_runtime_state(SessionState::Idle, None);
            session.reactivate_goal_stopped_by_manual_abort();
            drop(session);
            maybe_save_trajectory_background_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Checkpoint,
            );
            prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone()).await;
            if aborted_before_start_generation(&session_arc).await {
                return QueueCommandOutcome::Continue;
            }
            start_generation(app.clone(), session_arc.clone()).await;
        }
    }
    QueueCommandOutcome::Finished
}

async fn handle_remove_message(
    context: &QueueCommandContext,
    message_id: String,
    regenerate: bool,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let mut session = session_arc.lock().await;
    if session.runtime.state == SessionState::Generating {
        session.abort_stream();
    }
    if let Some(idx) = session.remove_message(&message_id) {
        if regenerate && idx < session.messages.len() {
            session.truncate_messages(idx);
            session.set_runtime_state(SessionState::Idle, None);
            session.reactivate_goal_stopped_by_manual_abort();
            drop(session);
            maybe_save_trajectory_background_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Checkpoint,
            );
            prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone()).await;
            if aborted_before_start_generation(&session_arc).await {
                return QueueCommandOutcome::Continue;
            }
            start_generation(app.clone(), session_arc.clone()).await;
        }
    }
    QueueCommandOutcome::Finished
}

async fn handle_regenerate(context: &QueueCommandContext) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    {
        let mut session = session_arc.lock().await;
        session.reactivate_goal_stopped_by_manual_abort();
    }
    maybe_save_trajectory_background_with_intent(
        app.clone(),
        session_arc.clone(),
        TrajectoryCommitIntent::Checkpoint,
    );
    prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone()).await;
    if aborted_before_start_generation(&session_arc).await {
        return QueueCommandOutcome::Continue;
    }
    start_generation(app.clone(), session_arc.clone()).await;
    QueueCommandOutcome::Finished
}

async fn handle_restore_messages(
    context: &QueueCommandContext,
    messages: Vec<serde_json::Value>,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let mut session = session_arc.lock().await;
    session.release_turn_only_state();
    for msg_value in messages {
        if let Ok(msg) = serde_json::from_value::<ChatMessage>(msg_value) {
            if !is_allowed_for_restore(&msg) {
                continue;
            }
            let sanitized = sanitize_message_for_restore(&msg);
            session.add_message(sanitized);
        }
    }
    drop(session);
    maybe_save_trajectory_with_intent(
        app.clone(),
        session_arc.clone(),
        TrajectoryCommitIntent::Required,
    )
    .await;
    QueueCommandOutcome::Finished
}

async fn handle_branch_from_chat(
    context: &QueueCommandContext,
    source_chat_id: String,
    up_to_message_id: String,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    if let Err(e) = trajectories::validate_trajectory_id(&source_chat_id) {
        warn!("BranchFromChat: invalid source_chat_id: {}", e);
        return QueueCommandOutcome::Continue;
    }

    let sessions = app.chat.sessions.clone();

    let source_session_arc =
        session::get_or_create_session_with_trajectory(app.clone(), &sessions, &source_chat_id)
            .await;

    let (messages_to_copy, root_id, source_worktree) = {
        let source_session = source_session_arc.lock().await;
        let mut msgs = Vec::new();
        let mut found = false;
        for m in &source_session.messages {
            if is_allowed_for_branch(m) {
                msgs.push(sanitize_message_for_branch(m));
            }
            if m.message_id == up_to_message_id {
                found = true;
                break;
            }
        }
        if !found {
            warn!(
                "BranchFromChat: up_to_message_id '{}' not found in source chat",
                up_to_message_id
            );
            return QueueCommandOutcome::Continue;
        }
        crate::chat::trajectory_ops::demote_goal_ownership_for_branch(&mut msgs);
        let root = source_session
            .thread
            .root_chat_id
            .clone()
            .unwrap_or_else(|| source_chat_id.clone());
        (msgs, root, source_session.thread.worktree.clone())
    };

    let mut session = session_arc.lock().await;
    session.thread.parent_id = Some(source_chat_id.clone());
    session.thread.link_type = Some("branch".to_string());
    session.thread.root_chat_id = Some(root_id);

    for msg in messages_to_copy {
        session.add_message(msg);
    }
    let target_thread = session.thread.clone();
    drop(session);
    if let Some(worktree) = source_worktree.as_ref() {
        if let Some(validated) =
            add_thread_worktree_reference(app.clone(), &target_thread.id, &target_thread, worktree)
                .await
        {
            let mut session = session_arc.lock().await;
            session.thread.worktree = Some(validated);
        }
    }
    maybe_save_trajectory_with_intent(
        app.clone(),
        session_arc.clone(),
        TrajectoryCommitIntent::Required,
    )
    .await;
    QueueCommandOutcome::Finished
}

async fn handle_browser_context_decision(
    context: &QueueCommandContext,
    pending_message_id: String,
    include_actions: bool,
    include_console: bool,
    include_network: bool,
    include_mutations: bool,
    include_screenshot: bool,
    last_n_actions: Option<usize>,
    last_n_console: Option<usize>,
    last_n_network: Option<usize>,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let request_id = &context.request_id;
    let native_guard = context.native_guard.as_ref();
    let root = native_guard_root(&app, native_guard).await;
    let pending = {
        let mut session = session_arc.lock().await;
        if !consume_native_guard(&mut session, native_guard, root.as_deref(), &request_id) {
            return QueueCommandOutcome::Continue;
        }
        browser_decisions::take_matching_browser_message(
            &mut session.pending_browser_message,
            &pending_message_id,
        )
    };
    let pending = match pending {
        Ok(pending) => pending,
        Err(error) => {
            warn!(?error, "BrowserContextDecision: stale response rejected");
            let mut session = session_arc.lock().await;
            session.emit(ChatEvent::Ack {
                client_request_id: request_id.clone(),
                accepted: false,
                result: Some(
                    serde_json::json!({"error": "browser context decision is no longer current"}),
                ),
            });
            let snapshot = session.snapshot();
            session.emit(snapshot);
            return QueueCommandOutcome::Continue;
        }
    };

    let browser_chat_id = {
        let session = session_arc.lock().await;
        session.chat_id.clone()
    };

    let snapshot =
        browser_context::get_browser_context_for_chat(app.gcx.clone(), &browser_chat_id).await;
    let screenshot = if include_screenshot
        && snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.page_changed)
    {
        browser_context::capture_browser_screenshot(app.gcx.clone(), &browser_chat_id).await
    } else {
        None
    };

    {
        let mut session = session_arc.lock().await;

        if let Some(mut snap) = snapshot {
            browser_context::apply_decision_to_snapshot(
                &mut snap,
                include_actions,
                include_console,
                include_network,
                include_mutations,
                last_n_actions,
                last_n_console,
                last_n_network,
            );
            let ctx_messages = browser_context::make_context_messages(&snap, screenshot);
            session.add_message(ctx_messages.event);
            if let Some(screenshot) = ctx_messages.screenshot {
                session.add_message(screenshot);
            }
        }

        if let Some(skill_msg) = pending.skill_context_msg {
            session.add_message(skill_msg);
        }
        if !pending.context_files.is_empty() {
            apply_manual_context_files(&mut session, &pending.context_files);
        }

        if pending.suppress_auto_enrichment && pending.context_files.is_empty() {
            session.suppress_auto_enrichment_for_next_turn = true;
        }

        let parsed_content = parse_content_with_attachments(&pending.content, &pending.attachments);
        let user_message = user_message_with_client_message_id(
            parsed_content,
            pending.checkpoints,
            pending.client_message_id,
        );
        session.add_message(user_message);
        note_user_turn_resets_goal_pursuit(&mut session);

        if let Some(ref skill_name) = pending.skill_activation_name {
            session.set_active_skill(skill_name.clone());
        }
        session.set_runtime_state(SessionState::Idle, None);
    }

    browser_context::commit_browser_cursors(app.gcx.clone(), &browser_chat_id).await;
    maybe_save_trajectory_background_with_intent(
        app.clone(),
        session_arc.clone(),
        TrajectoryCommitIntent::Checkpoint,
    );
    prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone()).await;
    if aborted_before_start_generation(&session_arc).await {
        return QueueCommandOutcome::Continue;
    }
    start_generation(app.clone(), session_arc.clone()).await;
    QueueCommandOutcome::Finished
}

async fn handle_deliver_messages(
    context: &QueueCommandContext,
    delivery: PendingDelivery,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    // Normally routed straight into the pending queue by
    // `chat::delivery`; reaching the command loop means a producer
    // pushed it as a plain command, so honour it here too.
    let wake = delivery.wake;
    let outcome = {
        let mut session = session_arc.lock().await;
        session.enqueue_delivery(delivery)
    };
    match outcome {
        Ok(DeliveryOutcome::Delivered) => {
            note_delivered_messages_reset_goal_pursuit(&session_arc).await;
            maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await;
            if wake && !aborted_before_start_generation(&session_arc).await {
                start_generation(app.clone(), session_arc.clone()).await;
            }
        }
        Ok(_) => {
            maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await;
        }
        Err(error) => {
            warn!("DeliverMessages rejected: {}", error);
            let mut session = session_arc.lock().await;
            emit_goal_command_error(&mut session, error);
        }
    }
    QueueCommandOutcome::Finished
}

async fn handle_update_pending_delivery(
    context: &QueueCommandContext,
    delivery_id: String,
    push: Option<PushMode>,
    cancel: bool,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let chat_id = {
        let session = session_arc.lock().await;
        session.chat_id.clone()
    };
    if let Err(error) =
        delivery::update_pending_delivery_in_chat(app.clone(), &chat_id, &delivery_id, push, cancel)
            .await
    {
        warn!("UpdatePendingDelivery rejected: {}", error);
    }
    QueueCommandOutcome::Finished
}

struct PreparedUserMessage {
    content: serde_json::Value,
    attachments: Vec<serde_json::Value>,
    context_files: Vec<serde_json::Value>,
    suppress_auto_enrichment: bool,
    client_message_id: Option<String>,
    checkpoints: Vec<crate::git::checkpoints::Checkpoint>,
    skill_activation_name: Option<String>,
    skill_context_msg: Option<ChatMessage>,
    browser_chat_id: String,
    browser_ctx_result: Option<(browser_context::BrowserContextMessages, bool)>,
    additional_messages: Vec<CommandRequest>,
}

async fn handle_user_message(
    context: &QueueCommandContext,
    content: serde_json::Value,
    attachments: Vec<serde_json::Value>,
    context_files: Vec<serde_json::Value>,
    suppress_auto_enrichment: bool,
    client_message_id: Option<String>,
) -> QueueCommandOutcome {
    let chat_id = context.session_arc.lock().await.chat_id.clone();
    if let Some(reason) = crate::agents::seal::seal_reason(&context.app, &chat_id).await {
        let mut session = context.session_arc.lock().await;
        reject_guard(&mut session, &context.request_id, reason.message());
        return QueueCommandOutcome::Continue;
    }
    let prepared = Box::pin(prepare_user_message(
        context,
        content,
        attachments,
        context_files,
        suppress_auto_enrichment,
        client_message_id,
    ))
    .await;
    let Some(prepared) = prepared else {
        return QueueCommandOutcome::Continue;
    };
    if !Box::pin(handle_oversized_browser_context(context, &prepared)).await {
        return QueueCommandOutcome::Continue;
    }
    if !Box::pin(add_user_messages(context, prepared)).await {
        return QueueCommandOutcome::Continue;
    }
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone()).await;
    if aborted_before_start_generation(&session_arc).await {
        return QueueCommandOutcome::Continue;
    }
    start_generation(app.clone(), session_arc.clone()).await;
    QueueCommandOutcome::Finished
}

async fn prepare_user_message(
    context: &QueueCommandContext,
    mut content: serde_json::Value,
    attachments: Vec<serde_json::Value>,
    context_files: Vec<serde_json::Value>,
    suppress_auto_enrichment: bool,
    client_message_id: Option<String>,
) -> Option<PreparedUserMessage> {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let priority = context.priority;
    let mut skill_activation_info = None;
    if let Some(text) = content.as_str() {
        match expand_slash_command(app.clone(), text).await {
            Ok(Some(expanded)) => {
                skill_activation_info = expanded.skill_to_activate;
                content = serde_json::Value::String(expanded.expanded_text);
                let mut session = session_arc.lock().await;
                session.active_command = ActiveCommandContext {
                    name: expanded.source_command,
                    allowed_tools: expanded.allowed_tools,
                    model_override: expanded.model_override,
                    context_fork: expanded.context_fork,
                };
            }
            Ok(None) => {
                let mut session = session_arc.lock().await;
                if session.thread.active_skill.is_none() {
                    session.active_command = ActiveCommandContext::default();
                }
            }
            Err(e) => {
                warn!("slash command expansion error: {}", e);
                let mut session = session_arc.lock().await;
                if session.thread.active_skill.is_none() {
                    session.active_command = ActiveCommandContext::default();
                }
            }
        }
    }

    let skill_activation_name: Option<String> =
        skill_activation_info.as_ref().map(|i| i.name.clone());
    let skill_context_msg = if let Some(info) = skill_activation_info {
        let body = expand_skill_includes(&info.body, &info.skill_dir).await;
        let line_count = body.lines().count().max(1);
        Some(ChatMessage {
            message_id: Uuid::new_v4().to_string(),
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(vec![ContextFile {
                file_name: format!("skill://{}", info.name),
                file_content: body,
                line1: 1,
                line2: line_count,
                file_rev: None,
                symbols: vec![],
                gradient_type: 0,
                usefulness: 95.0,
                skip_pp: true,
            }]),
            tool_call_id: SKILLS_CONTEXT_MARKER.to_string(),
            ..Default::default()
        })
    } else {
        None
    };

    let additional_messages = if !priority {
        let mut session = session_arc.lock().await;
        let msgs = drain_non_priority_user_messages(&mut session.command_queue);
        if !msgs.is_empty() {
            for message in &msgs {
                session.record_command_queue_wait(&message.client_request_id);
            }
            session.emit_queue_update();
        }
        msgs
    } else {
        Vec::new()
    };

    let (checkpoints_enabled, chat_id, latest_checkpoint, worktree) = {
        let session = session_arc.lock().await;
        (
            session.thread.checkpoints_enabled,
            session.chat_id.clone(),
            find_latest_checkpoint(&session),
            session.thread.worktree.clone(),
        )
    };

    let checkpoints = if checkpoints_enabled {
        create_checkpoint_async(
            app.clone(),
            latest_checkpoint.as_ref(),
            &chat_id,
            worktree.as_ref(),
        )
        .await
    } else {
        Vec::new()
    };

    let (has_browser_meta, attach_screenshot_on_send, browser_chat_id) = {
        let session = session_arc.lock().await;
        let bm = session.thread.browser_meta.as_ref();
        (
            bm.is_some(),
            bm.map_or(false, |m| m.attach_screenshot_on_send),
            session.chat_id.clone(),
        )
    };

    let browser_ctx_result = browser_context::maybe_insert_browser_context(
        app.gcx.clone(),
        &browser_chat_id,
        has_browser_meta,
        attach_screenshot_on_send,
    )
    .await;

    let (session_id_for_hook, project_dir_for_hook) = {
        let session = session_arc.lock().await;
        let sid = session.chat_id.clone();
        drop(session);
        let pd = get_project_dir_string(app.clone()).await;
        (sid, pd)
    };
    let prompt_text = match &content {
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    let prompt_payload = HookPayload::new(
        &HookEvent::UserPromptSubmit,
        session_id_for_hook.clone(),
        project_dir_for_hook.clone(),
    )
    .with_user_prompt(prompt_text);
    let prompt_verdict =
        run_control_hooks(app.clone(), HookEvent::UserPromptSubmit, prompt_payload).await;
    if let ControlVerdict::Deny(reason) = prompt_verdict {
        let mut session = session_arc.lock().await;
        let compression_phase = session.compression_phase;
        let compression_reason = session.compression_reason;
        let event = session.runtime_update_event(
            types::SessionState::Error,
            Some(format!("Message blocked by hook: {}", reason)),
            false,
            compression_phase,
            compression_reason,
        );
        session.emit(event);
        session.set_runtime_state(types::SessionState::Idle, None);
        return None;
    }

    let additional_messages = {
        let mut approved = Vec::new();
        for additional in additional_messages {
            let text = if let ChatCommand::UserMessage { ref content, .. } = additional.command {
                match content {
                    serde_json::Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                }
            } else {
                approved.push(additional);
                continue;
            };
            let add_verdict = run_control_hooks(
                app.clone(),
                HookEvent::UserPromptSubmit,
                HookPayload::new(
                    &HookEvent::UserPromptSubmit,
                    session_id_for_hook.clone(),
                    project_dir_for_hook.clone(),
                )
                .with_user_prompt(text),
            )
            .await;
            if !matches!(add_verdict, ControlVerdict::Deny(_)) {
                approved.push(additional);
            }
        }
        approved
    };
    Some(PreparedUserMessage {
        content,
        attachments,
        context_files,
        suppress_auto_enrichment,
        client_message_id,
        checkpoints,
        skill_activation_name,
        skill_context_msg,
        browser_chat_id,
        browser_ctx_result,
        additional_messages,
    })
}

async fn handle_oversized_browser_context(
    context: &QueueCommandContext,
    prepared: &PreparedUserMessage,
) -> bool {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let content = &prepared.content;
    let attachments = &prepared.attachments;
    let context_files = &prepared.context_files;
    let suppress_auto_enrichment = prepared.suppress_auto_enrichment;
    let client_message_id = &prepared.client_message_id;
    let checkpoints = &prepared.checkpoints;
    let skill_activation_name = &prepared.skill_activation_name;
    let skill_context_msg = &prepared.skill_context_msg;
    let browser_chat_id = &prepared.browser_chat_id;
    let browser_ctx_result = &prepared.browser_ctx_result;
    let is_oversize = browser_ctx_result
        .as_ref()
        .map_or(false, |(_, oversize)| *oversize);

    if is_oversize {
        if let Some((_, true)) = browser_ctx_result {
            let snapshot =
                browser_context::get_browser_context_for_chat(app.gcx.clone(), &browser_chat_id)
                    .await;
            if let Some(ref snap) = snapshot {
                let action_bytes = serde_json::to_string(&snap.actions)
                    .unwrap_or_default()
                    .len();
                let console_bytes = serde_json::to_string(&snap.console)
                    .unwrap_or_default()
                    .len();
                let network_bytes = serde_json::to_string(&snap.network)
                    .unwrap_or_default()
                    .len();
                let mutation_bytes = serde_json::to_string(&snap.mutations)
                    .unwrap_or_default()
                    .len();
                let pending_message_id = Uuid::new_v4().to_string();
                let mut session = session_arc.lock().await;
                session.pending_browser_message = Some(PendingBrowserMessage {
                    pending_message_id: pending_message_id.clone(),
                    content: content.clone(),
                    attachments: attachments.clone(),
                    client_message_id: client_message_id.clone(),
                    checkpoints: checkpoints.clone(),
                    context_files: context_files.clone(),
                    suppress_auto_enrichment,
                    skill_activation_name: skill_activation_name.clone(),
                    skill_context_msg: skill_context_msg.clone(),
                });
                session.mark_persisted_runtime_changed();
                drop(session);
                if let Err(error) = trajectories::try_save_trajectory_with_intent(
                    app.clone(),
                    session_arc.clone(),
                    TrajectoryCommitIntent::Required,
                )
                .await
                {
                    let mut session = session_arc.lock().await;
                    session.pending_browser_message = None;
                    session.mark_persisted_runtime_changed();
                    session.set_runtime_state(
                    SessionState::Error,
                    Some(format!("Could not save the browser-context decision: {error}. Resend your message to try again.")),
                );
                    return false;
                }
                let mut session = session_arc.lock().await;
                session.emit(ChatEvent::BrowserContextOversize {
                    total_bytes: action_bytes + console_bytes + network_bytes + mutation_bytes,
                    action_count: snap.actions.len(),
                    action_bytes,
                    console_count: snap.console.len(),
                    console_bytes,
                    network_count: snap.network.len(),
                    network_bytes,
                    mutation_bytes,
                    pending_message_id: pending_message_id.clone(),
                });
                session.set_runtime_state(SessionState::WaitingUserInput, None);
            }
        }
        return false;
    }
    true
}

async fn add_user_messages(context: &QueueCommandContext, prepared: PreparedUserMessage) -> bool {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let PreparedUserMessage {
        content,
        attachments,
        context_files,
        suppress_auto_enrichment,
        client_message_id,
        checkpoints,
        skill_activation_name,
        skill_context_msg,
        browser_chat_id: _,
        browser_ctx_result,
        additional_messages,
    } = prepared;
    let request_id = &context.request_id;
    let mut accepted_user_messages = Vec::new();
    let questions_cancelled;
    let attention_questions;
    {
        let mut session = session_arc.lock().await;
        attention_questions = pending_attention_questions(&app, &session);
        questions_cancelled = session.cancel_pending_questions();

        if let Some((ctx_messages, _)) = browser_ctx_result {
            session.add_message(ctx_messages.event);
            if let Some(screenshot) = ctx_messages.screenshot {
                session.add_message(screenshot);
            }
        }

        if let Some(skill_msg) = skill_context_msg {
            session.add_message(skill_msg);
        }

        if !context_files.is_empty() {
            apply_manual_context_files(&mut session, &context_files);
        }

        let parsed_content = parse_content_with_attachments(&content, &attachments);
        accepted_user_messages.push(AcceptedUserMessage {
            chat_id: session.chat_id.clone(),
            thread: session.thread.clone(),
            content: parsed_content.clone(),
        });
        let user_message = user_message_with_client_message_id(
            parsed_content.clone(),
            checkpoints,
            client_message_id,
        );
        session.add_message(user_message);
        note_user_turn_resets_goal_pursuit(&mut session);
        if session.messages.iter().filter(|m| m.role == "user").count() == 1 {
            let chat_id = session.chat_id.clone();
            let first_user_text_preview = parsed_content
                .content_text_only()
                .chars()
                .take(80)
                .collect();
            drop(session);
            push_user_activity(
                app.clone(),
                UserAction::ChatStarted {
                    chat_id,
                    first_user_text_preview,
                    ts: Utc::now(),
                },
            )
            .await;
            session = session_arc.lock().await;
        }

        if suppress_auto_enrichment && context_files.is_empty() {
            session.suppress_auto_enrichment_for_next_turn = true;
        }

        if let Some(ref skill_name) = skill_activation_name {
            session.set_active_skill(skill_name.clone());
        }

        for additional in additional_messages {
            if let ChatCommand::UserMessage {
                content: add_content,
                attachments: add_attachments,
                context_files: add_ctx_files,
                suppress_auto_enrichment: _,
                client_message_id: add_client_message_id,
            } = additional.command
            {
                if !add_ctx_files.is_empty() {
                    apply_manual_context_files(&mut session, &add_ctx_files);
                }
                let add_parsed = parse_content_with_attachments(&add_content, &add_attachments);
                accepted_user_messages.push(AcceptedUserMessage {
                    chat_id: session.chat_id.clone(),
                    thread: session.thread.clone(),
                    content: add_parsed.clone(),
                });
                let add_message = user_message_with_client_message_id(
                    add_parsed,
                    Vec::new(),
                    add_client_message_id,
                );
                session.add_message(add_message);
                note_user_turn_resets_goal_pursuit(&mut session);
            }
        }
    }

    leave_attention_questions(&app, attention_questions);
    for accepted_user_message in accepted_user_messages {
        let _ = maybe_enqueue_chat_reaction(app.clone(), accepted_user_message).await;
    }

    if questions_cancelled {
        if let Err(error) = trajectories::try_save_trajectory_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Required,
        )
        .await
        {
            let mut session = session_arc.lock().await;
            let error = format!("failed to persist cancelled question answers: {error}");
            session.emit(ChatEvent::Ack {
                client_request_id: request_id.clone(),
                accepted: false,
                result: Some(serde_json::json!({ "error": error.clone() })),
            });
            session.set_runtime_state(SessionState::Error, Some(error));
            session.queue_notify.notify_one();
            return false;
        }
    } else {
        maybe_save_trajectory_background_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Checkpoint,
        );
    }
    true
}
