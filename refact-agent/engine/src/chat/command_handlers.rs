use super::*;
use crate::buddy::chat_reactions::{maybe_enqueue_chat_reaction, AcceptedUserMessage};
use crate::call_validation::ContextFile;
use crate::chat::browser_context;
use crate::chat::generation::prepare_session_preamble_and_knowledge;
use crate::chat::{browser_decisions, delivery, session, trajectories, types};
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{get_project_dir_string, run_control_hooks, ControlVerdict, HookPayload};
use crate::ext::skills_context::{expand_skill_includes, SKILLS_CONTEXT_MARKER};
use crate::ext::slash_expand::expand_slash_command;
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
    board_operation: AMutex<Option<crate::chat::session_runtime::BoardOperation>>,
}

impl QueueCommandContext {
    async fn release_board_operation(&self) {
        self.board_operation.lock().await.take();
    }
}

fn companion_command_allowed(command: &ChatCommand) -> bool {
    crate::buddy::companion::command_allowed(command)
}

#[cfg(test)]
mod companion_command_gate_tests {
    use super::*;

    #[test]
    fn companion_rejects_settings_and_history_mutation() {
        for value in [
            serde_json::json!({"type":"set_params","patch":{"tool_use":"agent"}}),
            serde_json::json!({"type":"restore_messages","messages":[]}),
            serde_json::json!({"type":"regenerate"}),
        ] {
            let command: ChatCommand = serde_json::from_value(value).unwrap();
            assert!(!companion_command_allowed(&command));
        }
    }
}

pub(super) async fn handle_queue_command(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    request: CommandRequest,
) -> QueueCommandOutcome {
    let _native_mutation = {
        let mut session = session_arc.lock().await;
        match super::super::session::NativeMutationGuard::enter_locked(&session_arc, &mut session) {
            Ok(guard) => guard,
            Err(error) => {
                reject_guard(&mut session, &request.client_request_id, error);
                return QueueCommandOutcome::Continue;
            }
        }
    };
    {
        let mut session = session_arc.lock().await;
        if crate::buddy::companion::is_companion(&session.thread)
            && !companion_command_allowed(&request.command)
        {
            reject_guard(&mut session, &request.client_request_id, "Companion conversations cannot change execution settings or restore another session".into());
            return QueueCommandOutcome::Continue;
        }
    }
    let board_operation = if matches!(
        &request.command,
        ChatCommand::BrowserContextDecision { .. }
            | ChatCommand::UserMessage { .. }
            | ChatCommand::RetryFromIndex { .. }
            | ChatCommand::Regenerate {}
            | ChatCommand::RecoveryRetry { .. }
            | ChatCommand::AnswerQuestions { .. }
            | ChatCommand::SubmitQuestionResponse { .. }
            | ChatCommand::IdeToolResult { .. }
            | ChatCommand::UpdateMessage {
                regenerate: true,
                ..
            }
            | ChatCommand::RemoveMessage {
                regenerate: true,
                ..
            }
    ) {
        match super::super::session_runtime::begin_board_operation(&app, &session_arc).await {
            Ok(operation) => operation,
            Err(reason) => {
                session_arc.lock().await.emit(ChatEvent::Ack {
                    client_request_id: request.client_request_id,
                    accepted: false,
                    result: Some(serde_json::json!({ "error": reason.recovery_message() })),
                });
                return QueueCommandOutcome::Continue;
            }
        }
    } else {
        None
    };
    let context = QueueCommandContext {
        app,
        session_arc,
        request_id: request.client_request_id,
        priority: request.priority,
        native_guard: request.native_approval_guard,
        supervision_guard: request.supervision_guard,
        board_operation: AMutex::new(board_operation),
    };
    if matches!(
        &request.command,
        ChatCommand::SetGoal { .. }
            | ChatCommand::UpdateGoal { .. }
            | ChatCommand::SetGoalBudget { .. }
            | ChatCommand::GoalControl { .. }
    ) {
        if let Err(error) = crate::tasks::goal::hydrate_session_task_goal_for_mutation(
            context.app.gcx.clone(),
            &context.session_arc,
        )
        .await
        {
            reject_guard(
                &mut *context.session_arc.lock().await,
                &context.request_id,
                error,
            );
            return QueueCommandOutcome::Continue;
        }
    }
    match request.command {
        ChatCommand::RecoveryRetry {
            run_id,
            expected_revision,
            options,
            model,
        } => {
            if let Some(model) = &model {
                let resolution = match crate::global_context::try_load_caps_quickly_if_not_present(
                    context.app.gcx.clone(),
                    0,
                )
                .await
                {
                    Ok(caps) => crate::caps::resolve_chat_model(caps, model).map(|_| ()),
                    Err(error) => Err(error.message),
                };
                if let Err(error) = resolution {
                    reject_guard(
                        &mut *context.session_arc.lock().await,
                        &context.request_id,
                        error,
                    );
                    return QueueCommandOutcome::Continue;
                }
            }
            let chat_id = context.session_arc.lock().await.chat_id.clone();
            if let Some(reason) = crate::agents::seal::seal_reason(&context.app, &chat_id).await {
                reject_guard(
                    &mut *context.session_arc.lock().await,
                    &context.request_id,
                    reason.message(),
                );
                return QueueCommandOutcome::Continue;
            }
            let mut session = context.session_arc.lock().await;
            let valid = options.validate().is_ok()
                && options.max_retries > 0
                && session.runtime.recovery.as_ref().is_some_and(|r| {
                    r.run_id == run_id
                        && r.revision == expected_revision
                        && matches!(
                            r.state,
                            refact_core::recovery::RecoveryState::NeedsAction
                                | refact_core::recovery::RecoveryState::Exhausted
                                | refact_core::recovery::RecoveryState::Cancelled
                        )
                });
            if !valid {
                reject_guard(
                    &mut session,
                    &context.request_id,
                    "Stale recovery revision or invalid retry options".into(),
                );
                return QueueCommandOutcome::Continue;
            }
            if session.planner_paused()
                || session.goal_status.as_ref().is_some_and(|status| {
                    matches!(status, refact_chat_api::GoalStatus::Paused)
                        || (*status == refact_chat_api::GoalStatus::Stopped
                            && !session.goal_stopped_by_abort
                            && !session
                                .goal
                                .as_ref()
                                .is_some_and(|g| g.stop_reason.as_deref() == Some("manual_abort")))
                })
            {
                reject_guard(
                    &mut session,
                    &context.request_id,
                    "Resume the paused or stopped goal before retrying".into(),
                );
                return QueueCommandOutcome::Continue;
            }
            session.reactivate_goal_stopped_by_manual_abort();
            session.archive_recovery();
            let mut record = session.runtime.recovery.clone().unwrap();
            record.run_id = uuid::Uuid::new_v4().to_string();
            record.revision += 1;
            record.policy = options.clone();
            record.policy_source = refact_core::recovery::RecoveryPolicySource::Thread;
            record.attempts.clear();
            record.manual = true;
            record.retries_used = 0;
            record.state = refact_core::recovery::RecoveryState::Attempting;
            record.terminal_reason = None;
            record.deadline_at_ms =
                Some(crate::chat::recovery::now_ms().saturating_add(options.total_timeout_ms));
            record.next_retry_at_ms = None;
            if let Some(model) = model {
                let old_provider =
                    crate::stats::event::split_model_provider(&session.thread.model).0;
                let new_provider = crate::stats::event::split_model_provider(&model).0;
                crate::chat::recovery::apply_model_patch(&mut session, &model);
                if old_provider != new_provider {
                    record.retry_not_before_ms = None;
                }
            }
            session.recovery_budget = Some(std::sync::Arc::new(
                refact_core::recovery::RecoveryBudget::new_manual(options),
            ));
            session.abort_flag.store(false, Ordering::SeqCst);
            session.user_interrupt_flag.store(false, Ordering::SeqCst);
            let next_at = record.retry_not_before_ms.unwrap_or(0);
            let should_wait = next_at > crate::chat::recovery::now_ms();
            if should_wait {
                record.state = refact_core::recovery::RecoveryState::Waiting;
                record.next_retry_at_ms = Some(next_at);
            }
            let run = record.run_id.clone();
            let budget = session.recovery_budget.clone().unwrap();
            let notify = session.abort_notify.clone();
            let retry_notify = session.recovery_notify.clone();
            session.update_recovery(Some(record));
            session.set_runtime_state(
                if should_wait {
                    SessionState::Generating
                } else {
                    SessionState::Idle
                },
                None,
            );
            let owned_run = run.clone();
            drop(session);
            maybe_save_trajectory_with_intent(
                context.app.clone(),
                context.session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await;
            let stopped_before_wait = {
                let session = context.session_arc.lock().await;
                session.abort_flag.load(Ordering::SeqCst)
                    || !session.runtime.recovery.as_ref().is_some_and(|r| {
                        r.run_id == owned_run
                            && matches!(
                                r.state,
                                refact_core::recovery::RecoveryState::Waiting
                                    | refact_core::recovery::RecoveryState::Attempting
                            )
                    })
            };
            context.release_board_operation().await;
            let ready = !stopped_before_wait
                && (!should_wait
                    || crate::chat::recovery::wait_for_retry(
                        &context.session_arc,
                        budget.clone(),
                        notify,
                        retry_notify,
                        next_at,
                        run,
                    )
                    .await);
            if !ready {
                {
                    let mut session = context.session_arc.lock().await;
                    if session
                        .runtime
                        .recovery
                        .as_ref()
                        .is_some_and(|r| r.run_id == owned_run)
                        && session
                            .recovery_budget
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &budget))
                    {
                        session.recovery_budget = None;
                        session.abort_flag.store(false, Ordering::SeqCst);
                        session.user_interrupt_flag.store(false, Ordering::SeqCst);
                        session.queue_notify.notify_one();
                    }
                }
                maybe_save_trajectory_with_intent(
                    context.app.clone(),
                    context.session_arc.clone(),
                    TrajectoryCommitIntent::Required,
                )
                .await;
                return QueueCommandOutcome::Finished;
            }
            start_generation(context.app.clone(), context.session_arc.clone()).await;
            QueueCommandOutcome::Finished
        }
        ChatCommand::RecoveryRetryNow {
            ..
        } => QueueCommandOutcome::Continue,
        ChatCommand::SetGoal {
            content,
            budget,
            criteria,
            plan,
        } => Box::pin(handle_set_goal(&context, content, budget, criteria, plan)).await,
        ChatCommand::SetGoalBudget {
            budget,
        } => Box::pin(handle_set_goal_budget(&context, budget)).await,
        ChatCommand::UpdateGoal {
            note,
            plan,
        } => Box::pin(handle_update_goal(&context, note, plan)).await,
        ChatCommand::GoalControl {
            action,
        } => Box::pin(handle_goal_control(&context, action)).await,
        ChatCommand::UserMessage {
            content,
            attachments,
            context_files,
            suppress_auto_enrichment,
            client_message_id,
            app_context_ids,
        } => {
            Box::pin(handle_user_message(
                &context,
                content,
                attachments,
                context_files,
                suppress_auto_enrichment,
                client_message_id,
                app_context_ids,
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
        ChatCommand::SetParams {
            patch,
        } => Box::pin(handle_set_params(&context, patch)).await,
        ChatCommand::SetAppWorkspace {
            expected_revision,
            workspace,
        } => {
            match crate::chat::app_workspace::set_app_workspace_in_session(
                context.app.clone(),
                context.session_arc.clone(),
                expected_revision,
                workspace,
            )
            .await
            {
                Ok(stored) => context.session_arc.lock().await.emit(ChatEvent::Ack {
                    client_request_id: context.request_id.clone(),
                    accepted: true,
                    result: Some(serde_json::json!({"revision": stored.revision})),
                }),
                Err(error) => context.session_arc.lock().await.emit(ChatEvent::Ack {
                    client_request_id: context.request_id.clone(),
                    accepted: false,
                    result: Some(error.to_json()),
                }),
            }
            QueueCommandOutcome::Finished
        }
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
                None,
            ))
            .await
        }
        ChatCommand::SubmitQuestionResponse {
            tool_call_id,
            asked_at_ms,
            response,
        } => {
            if context.supervision_guard.is_some() {
                let mut session = context.session_arc.lock().await;
                reject_question_answers(
                    &mut session,
                    &context.request_id,
                    "structured question responses require an explicit user response".into(),
                );
                return QueueCommandOutcome::Continue;
            }
            Box::pin(handle_answer_questions(
                &context,
                tool_call_id,
                Vec::new(),
                Some(asked_at_ms),
                Some(response),
            ))
            .await
        }
        ChatCommand::CleanBackgroundProcesses {
            include_services,
        } => {
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
        ChatCommand::ToolDecisions {
            decisions,
        } => Box::pin(handle_tool_decisions(&context, decisions)).await,
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
        ChatCommand::RestoreMessages {
            messages,
        } => Box::pin(handle_restore_messages(&context, messages)).await,
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
        ChatCommand::DeliverMessages {
            delivery,
        } => Box::pin(handle_deliver_messages(&context, delivery)).await,
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
    plan: Option<refact_chat_api::GoalPlanInstall>,
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
                let budget = budget.map(|budget| {
                    if supervision_guard.is_some() {
                        GoalBudget {
                            supervised: true,
                            ..budget
                        }
                    } else {
                        budget.without_hard_limits()
                    }
                });
                Ok(handle_set_goal_command(
                    &mut session,
                    content,
                    budget,
                    criteria,
                    plan,
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
            if let Err(error) =
                crate::tasks::goal::commit_session_task_goal(app.gcx.clone(), &session_arc).await
            {
                emit_goal_command_error(&mut *session_arc.lock().await, error);
                return QueueCommandOutcome::Finished;
            }
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
            if let Err(error) =
                crate::tasks::goal::commit_session_task_goal(app.gcx.clone(), &session_arc).await
            {
                emit_goal_command_error(&mut *session_arc.lock().await, error);
                return QueueCommandOutcome::Finished;
            }
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

async fn handle_update_goal(
    context: &QueueCommandContext,
    note: Option<String>,
    plan: Option<refact_chat_api::GoalPlanMutation>,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let policy = crate::chat::verify_cmd::VerifyCommandPolicy::load(app.gcx.clone()).await;
    let result = {
        let mut session = session_arc.lock().await;
        handle_update_goal_command(&mut session, note, plan).map(|result| {
            for advisory in crate::chat::evidence::unverifiable_goal_criteria(&session, &policy) {
                tracing::info!(chat_id = %session.chat_id, %advisory, "UpdateGoal appended with advisory");
            }
            result
        })
    };
    match result {
        Ok(_) => {
            if let Err(error) =
                crate::tasks::goal::commit_session_task_goal(app.gcx.clone(), &session_arc).await
            {
                emit_goal_command_error(&mut *session_arc.lock().await, error);
                return QueueCommandOutcome::Finished;
            }
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
            if let Err(error) =
                crate::tasks::goal::commit_session_task_goal(app.gcx.clone(), &session_arc).await
            {
                emit_goal_command_error(&mut *session_arc.lock().await, error);
                return QueueCommandOutcome::Finished;
            }
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
    {
        let mut session = session_arc.lock().await;
        if session.runtime.recovery.is_some() {
            session.archive_recovery();
            session.update_recovery(None);
        }
        session.recovery_budget = None;
    }
    context.release_board_operation().await;
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
    if let Err(error) = crate::chat::recovery::validate_policy_patch(&patch) {
        reject_guard(&mut *session_arc.lock().await, &context.request_id, error);
        return QueueCommandOutcome::Continue;
    }
    let mode_switch_reason = patch
        .get("reason")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let scope_patch = crate::chat::worktree_lifecycle::is_scope_patch(&patch);
    let _scope_transition = if scope_patch {
        Some(crate::chat::worktree_lifecycle::transition_lock(&session_arc).await)
    } else {
        None
    };
    let (chat_id, thread_before) = {
        let session = session_arc.lock().await;
        (session.chat_id.clone(), session.thread.clone())
    };
    if let Err(error) =
        check_worktree_setparams_authority(&app, &chat_id, &thread_before, &patch).await
    {
        let mut session = session_arc.lock().await;
        reject_guard(&mut session, &context.request_id, error);
        return QueueCommandOutcome::Continue;
    }
    {
        let mut session = session_arc.lock().await;
        if crate::chat::worktree_lifecycle::changes_attachment(&session.thread, &patch) {
            if let Err(error) = crate::chat::worktree_lifecycle::check_idle_attachment(&session) {
                reject_guard(&mut session, &context.request_id, error);
                return QueueCommandOutcome::Continue;
            }
        }
    }
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
            session.emit(ChatEvent::Ack {
                client_request_id: context.request_id.clone(),
                accepted: false,
                result: Some(serde_json::json!({"error": e})),
            });
            return QueueCommandOutcome::Continue;
        }
    };
    let mut session = session_arc.lock().await;
    if scope_patch {
        if let Err(error) =
            crate::chat::worktree_lifecycle::check_unchanged_attachment(&session, &thread_before)
        {
            reject_guard(&mut session, &context.request_id, error);
            return QueueCommandOutcome::Continue;
        }
    }
    if crate::chat::worktree_lifecycle::changes_attachment(&session.thread, &patch) {
        if let Err(error) = crate::chat::worktree_lifecycle::check_idle_attachment(&session) {
            reject_guard(&mut session, &context.request_id, error);
            return QueueCommandOutcome::Continue;
        }
    }
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
    if session.thread.model != old_model {
        crate::chat::trajectory_ops::sanitize_messages_for_model_switch(&mut session.messages);
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
    response: Option<refact_chat_api::QuestionResponse>,
) -> QueueCommandOutcome {
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    let request_id = &context.request_id;
    let supervision_guard = context.supervision_guard.as_ref();
    let content = match &response {
        Some(response) => serde_json::json!({
            "type": "ask_questions_answer", "tool_call_id": tool_call_id,
            "response_protocol_version": 2, "answers": response.answers,
            "overall_comments": response.overall_comments,
            "skipped_meaning": "Skipped questions are not answered or approved. Decide how to proceed; do not silently apply a default or repeat the same question."
        }).to_string(),
        None => answer_questions_content(&tool_call_id, &answers),
    };
    let resolution_id = request_id.clone();
    let timeout_default_notice = request_id
        .strip_prefix("attention-timeout:")
        .and_then(|question_id| app.questions().get(question_id))
        .and_then(|question| question.default_answer);
    let mut staged_message_id: Option<String> = None;
    let mut staged_at = None;
    let mut attention_questions = Vec::new();
    let resolved = {
        let mut session = session_arc.lock().await;
        let chat_id = session.chat_id.clone();
        match crate::buddy::supervision_authority::with_command_guard(
            supervision_guard,
            &chat_id,
            || match response {
                Some(response) => session.submit_question_response(
                    &tool_call_id,
                    asked_at_ms.ok_or("question instance timestamp required")?,
                    response,
                    resolution_id.clone(),
                ),
                None => session.answer_pending_questions_at(
                    &tool_call_id,
                    answers,
                    resolution_id.clone(),
                    asked_at_ms,
                ),
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
                    staged_at = Some(std::time::Instant::now());
                    session.add_message(message);
                    crate::buddy::decision_receipts::transition(
                        &mut session,
                        &resolution_id,
                        "applied",
                        None,
                    );
                }
                resolved
                    || session.pending_questions.iter().any(|set| {
                        set.tool_call_id == tool_call_id
                            && set.status == PendingQuestionStatus::Answered
                            && set.resolution_id.as_deref() == Some(resolution_id.as_str())
                    })
            }
            Err(error) => {
                crate::buddy::decision_receipts::transition(
                    &mut session,
                    &resolution_id,
                    "rejected",
                    Some(error.clone()),
                );
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
            if let Ok(Some(mut receipt)) =
                crate::buddy::decision_receipts::find(&session, &resolution_id)
            {
                receipt.status = "unknown".into();
                receipt.reason =
                    Some("Answer persistence failed; application is not acknowledged".into());
                session.increment_version();
                receipt.version = session.trajectory_version;
                crate::buddy::decision_receipts::replace(&mut session, receipt);
            }
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
                crate::buddy::decision_receipts::transition(
                    &mut session,
                    &resolution_id,
                    "rejected",
                    Some(error.clone()),
                );
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
                    if let Some(staged_at) = staged_at {
                        session.live_diagnostics.accept(staged_at);
                    }
                    session.set_runtime_state(SessionState::Generating, None);
                    Ok(())
                },
            ) {
                reject_guard(&mut session, &resolution_id, error);
                return QueueCommandOutcome::Continue;
            }
        }
        context.release_board_operation().await;
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
    context.release_board_operation().await;
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
            context.release_board_operation().await;
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
            context.release_board_operation().await;
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
        if context.request_id.starts_with("delivery-wake-")
            && session
                .runtime
                .recovery
                .as_ref()
                .is_some_and(|r| r.holds_goal())
        {
            session.suppress_delivery_wakes();
            return QueueCommandOutcome::Continue;
        }
        session.reactivate_goal_stopped_by_manual_abort();
    }
    maybe_save_trajectory_background_with_intent(
        app.clone(),
        session_arc.clone(),
        TrajectoryCommitIntent::Checkpoint,
    );
    {
        let mut session = session_arc.lock().await;
        if session.runtime.recovery.is_some() {
            session.archive_recovery();
            session.update_recovery(None);
        }
        session.recovery_budget = None;
    }
    context.release_board_operation().await;
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
    let _transition = match crate::chat::worktree_lifecycle::try_transition_lock(&session_arc) {
        Ok(transition) => transition,
        Err(error) => {
            reject_guard(&mut *session_arc.lock().await, &context.request_id, error);
            return QueueCommandOutcome::Continue;
        }
    };
    let (before, version, requests, aborted, interrupted) = {
        let session = session_arc.lock().await;
        if let Err(error) = crate::chat::worktree_lifecycle::check_idle_attachment(&session) {
            drop(session);
            reject_guard(&mut *session_arc.lock().await, &context.request_id, error);
            return QueueCommandOutcome::Continue;
        }
        (
            session.thread.clone(),
            session.trajectory_version,
            session.recent_request_ids.clone(),
            session
                .abort_flag
                .load(std::sync::atomic::Ordering::Acquire),
            session
                .user_interrupt_flag
                .load(std::sync::atomic::Ordering::Acquire),
        )
    };

    let sessions = app.chat.sessions.clone();

    let source_session_arc = match session::get_or_create_session_with_trajectory(
        app.clone(),
        &sessions,
        &source_chat_id,
    )
    .await
    {
        Ok(source) => source,
        Err(error) => {
            reject_guard(
                &mut *session_arc.lock().await,
                &context.request_id,
                error.to_string(),
            );
            return QueueCommandOutcome::Continue;
        }
    };

    if Arc::ptr_eq(&source_session_arc, &session_arc) {
        reject_guard(
            &mut *session_arc.lock().await,
            &context.request_id,
            "Cannot branch a chat into itself".into(),
        );
        return QueueCommandOutcome::Continue;
    }
    let _source_transition =
        match crate::chat::worktree_lifecycle::try_transition_lock(&source_session_arc) {
            Ok(transition) => transition,
            Err(error) => {
                reject_guard(&mut *session_arc.lock().await, &context.request_id, error);
                return QueueCommandOutcome::Continue;
            }
        };
    let (messages_to_copy, root_id, source_worktree, source_before) = {
        let source_session = source_session_arc.lock().await;
        if crate::buddy::companion::is_companion(&source_session.thread) {
            drop(source_session);
            reject_guard(
                &mut *session_arc.lock().await,
                &context.request_id,
                "Cannot branch a source-bound companion conversation".into(),
            );
            return QueueCommandOutcome::Continue;
        }
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
        (
            msgs,
            root,
            source_session.thread.worktree.clone(),
            source_session.thread.clone(),
        )
    };

    let mut target_thread = before.clone();
    target_thread.parent_id = Some(source_chat_id.clone());
    target_thread.link_type = Some("branch".to_string());
    target_thread.root_chat_id = Some(root_id);
    let patch = serde_json::json!({
        "parent_id": target_thread.parent_id,
        "link_type": target_thread.link_type,
        "root_chat_id": target_thread.root_chat_id,
        "worktree": source_worktree.as_ref().or(before.worktree.as_ref()),
    });
    if let Err(error) = check_worktree_setparams_authority(&app, &before.id, &before, &patch).await
    {
        reject_guard(&mut *session_arc.lock().await, &context.request_id, error);
        return QueueCommandOutcome::Continue;
    }
    {
        let session = session_arc.lock().await;
        let sessions = app.chat.sessions.try_read();
        if !sessions.as_ref().is_ok_and(|sessions| {
            sessions
                .get(&before.id)
                .is_some_and(|current| Arc::ptr_eq(current, &session_arc))
        }) || crate::chat::worktree_lifecycle::check_unchanged_attachment(&session, &before)
            .is_err()
            || session.trajectory_version != version
        {
            drop(sessions);
            drop(session);
            reject_guard(
                &mut *session_arc.lock().await,
                &context.request_id,
                "Branch target changed before checkout authorization; retry".into(),
            );
            return QueueCommandOutcome::Continue;
        }
    }
    {
        let source = source_session_arc.lock().await;
        let sessions = app.chat.sessions.try_read();
        if !sessions.as_ref().is_ok_and(|sessions| {
            sessions
                .get(&source_chat_id)
                .is_some_and(|current| Arc::ptr_eq(current, &source_session_arc))
        }) || crate::chat::worktree_lifecycle::check_unchanged_attachment(
            &source,
            &source_before,
        )
        .is_err()
        {
            drop(sessions);
            drop(source);
            reject_guard(
                &mut *session_arc.lock().await,
                &context.request_id,
                "Branch source checkout changed; retain current scope and retry".into(),
            );
            return QueueCommandOutcome::Continue;
        }
    }
    if let Some(worktree) = source_worktree.as_ref() {
        if worktree.kind == "task_agent" && before.worktree.as_ref() != Some(worktree) {
            reject_guard(
                &mut *session_arc.lock().await,
                &context.request_id,
                "Task-agent checkouts cannot be attached through branching".into(),
            );
            return QueueCommandOutcome::Continue;
        }
        let Some(validated) =
            add_thread_worktree_reference(app.clone(), &target_thread.id, &target_thread, worktree)
                .await
        else {
            reject_guard(
                &mut *session_arc.lock().await,
                &context.request_id,
                "Branch checkout is unavailable; retain the current checkout and retry".into(),
            );
            return QueueCommandOutcome::Continue;
        };
        target_thread.worktree = Some(validated);
    }
    {
        let source = source_session_arc.lock().await;
        let sessions = app.chat.sessions.try_read();
        if !sessions.as_ref().is_ok_and(|sessions| {
            sessions
                .get(&source_chat_id)
                .is_some_and(|current| Arc::ptr_eq(current, &source_session_arc))
        }) || crate::chat::worktree_lifecycle::check_unchanged_attachment(
            &source,
            &source_before,
        )
        .is_err()
        {
            drop(sessions);
            drop(source);
            reject_guard(
                &mut *session_arc.lock().await,
                &context.request_id,
                "Branch source no longer current; checkout retained".into(),
            );
            return QueueCommandOutcome::Continue;
        }
    }
    {
        let _app_workspace_lease =
            crate::chat::app_workspace::AppWorkspaceMutationLease::acquire(session_arc.clone())
                .await;
        let mut session = session_arc.lock().await;
        let sessions = app.chat.sessions.try_read();
        if !sessions.as_ref().is_ok_and(|sessions| {
            sessions
                .get(&before.id)
                .is_some_and(|current| Arc::ptr_eq(current, &session_arc))
        }) {
            reject_guard(
                &mut session,
                &context.request_id,
                "Branch target session changed or registry busy; retry".into(),
            );
            return QueueCommandOutcome::Continue;
        }
        if let Err(error) = crate::chat::worktree_lifecycle::check_idle_attachment(&session)
            .and_then(|_| {
                crate::chat::worktree_lifecycle::check_unchanged_attachment(&session, &before)
            })
            .and_then(|_| {
                if session.trajectory_version != version
                    || session.recent_request_ids != requests
                    || session
                        .abort_flag
                        .load(std::sync::atomic::Ordering::Acquire)
                        != aborted
                    || session
                        .user_interrupt_flag
                        .load(std::sync::atomic::Ordering::Acquire)
                        != interrupted
                {
                    Err("Branch canceled because the target chat changed".into())
                } else {
                    Ok(())
                }
            })
        {
            reject_guard(&mut session, &context.request_id, error);
            return QueueCommandOutcome::Continue;
        }
        session.thread.parent_id = target_thread.parent_id;
        session.thread.link_type = target_thread.link_type;
        session.thread.root_chat_id = target_thread.root_chat_id;
        session.thread.worktree = target_thread.worktree;
        if let Some(forked) = session
            .thread
            .app_workspace
            .is_none()
            .then(|| {
                crate::chat::app_workspace::fork_app_workspace_for_new_chat(
                    source_before.app_workspace.as_ref(),
                )
            })
            .flatten()
        {
            crate::chat::app_workspace::install_forked_app_workspace(&mut session, forked);
        }
        for msg in messages_to_copy {
            session.add_message(msg);
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

        if let Some(target) = session
            .queued_user_merge_targets
            .remove(&pending.pending_message_id)
        {
            session
                .queued_user_merge_targets
                .insert(request_id.clone(), target);
        }
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
            session.add_queued_user_context(request_id, ctx_messages.event);
            if let Some(screenshot) = ctx_messages.screenshot {
                session.add_queued_user_context(request_id, screenshot);
            }
        }

        if let Some(skill_msg) = pending.skill_context_msg {
            session.add_queued_user_context(request_id, skill_msg);
        }
        if !pending.context_files.is_empty() {
            apply_manual_context_files(&mut session, &pending.context_files, request_id);
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
        session.add_or_merge_queued_user_message(request_id, user_message);
        session.live_diagnostics.accept(std::time::Instant::now());
        note_user_turn_resets_goal_pursuit(&mut session);

        if let Some(ref skill_name) = pending.skill_activation_name {
            session.set_active_skill(skill_name.clone());
        }
        session.set_runtime_state(SessionState::Idle, None);
        crate::buddy::decision_receipts::transition(&mut session, &request_id, "applied", None);
    }

    if trajectories::flush_trajectory_for_session(app.clone(), session_arc.clone())
        .await
        .is_err()
    {
        return QueueCommandOutcome::Continue;
    }
    browser_context::commit_browser_cursors(app.gcx.clone(), &browser_chat_id).await;
    maybe_save_trajectory_background_with_intent(
        app.clone(),
        session_arc.clone(),
        TrajectoryCommitIntent::Checkpoint,
    );
    {
        let mut session = session_arc.lock().await;
        if session.runtime.recovery.is_some() {
            session.archive_recovery();
            session.update_recovery(None);
        }
        session.recovery_budget = None;
    }
    context.release_board_operation().await;
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
            if let Err(error) =
                crate::tasks::goal::commit_session_task_goal(app.gcx.clone(), &session_arc).await
            {
                emit_goal_command_error(&mut *session_arc.lock().await, error);
                return QueueCommandOutcome::Finished;
            }
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
    app_context_ids: Vec<String>,
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
    app_context_ids: Vec<String>,
) -> QueueCommandOutcome {
    let is_companion = {
        let session = context.session_arc.lock().await;
        crate::buddy::companion::is_companion(&session.thread)
    };
    let chat_id = context.session_arc.lock().await.chat_id.clone();
    if is_companion {
        if !attachments.is_empty() || !context_files.is_empty() || !app_context_ids.is_empty() {
            crate::chat::app_context::release(&chat_id, &context.request_id);
            reject_guard(
                &mut *context.session_arc.lock().await,
                &context.request_id,
                "Companion questions must be text-only".into(),
            );
            return QueueCommandOutcome::Continue;
        }
        let Some(text) = content.as_str() else {
            reject_guard(
                &mut *context.session_arc.lock().await,
                &context.request_id,
                "Companion questions must be text-only".into(),
            );
            return QueueCommandOutcome::Continue;
        };
        let (chat_id, thread, history, version) = {
            let session = context.session_arc.lock().await;
            (
                session.chat_id.clone(),
                session.thread.clone(),
                session.messages.clone(),
                session.trajectory_version,
            )
        };
        let mut message = user_message_with_client_message_id(
            parse_content_with_attachments(&serde_json::json!(text), &[]),
            Vec::new(),
            client_message_id,
        );
        if let Err(error) = crate::buddy::companion::stamp_question_provenance(
            context.app.gcx.clone(),
            &chat_id,
            &thread,
            &history,
            &mut message,
        )
        .await
        {
            reject_guard(
                &mut *context.session_arc.lock().await,
                &context.request_id,
                error,
            );
            return QueueCommandOutcome::Continue;
        }
        let message_id = message.message_id.clone();
        {
            let mut session = context.session_arc.lock().await;
            if session.closed
                || session.trajectory_version != version
                || !crate::buddy::companion::is_companion(&session.thread)
                || serde_json::to_value(&session.thread).ok() != serde_json::to_value(&thread).ok()
            {
                reject_guard(
                    &mut session,
                    &context.request_id,
                    "Companion changed while preparing the question; retry".into(),
                );
                return QueueCommandOutcome::Continue;
            }
            session.add_message(message);
        }
        if let Err(error) = trajectories::try_save_trajectory_with_intent(
            context.app.clone(),
            context.session_arc.clone(),
            TrajectoryCommitIntent::Required,
        )
        .await
        {
            let mut session = context.session_arc.lock().await;
            session.remove_message(&message_id);
            reject_guard(
                &mut session,
                &context.request_id,
                format!("Could not persist companion question: {error}"),
            );
            return QueueCommandOutcome::Continue;
        }
        context
            .session_arc
            .lock()
            .await
            .live_diagnostics
            .accept(std::time::Instant::now());
        context.release_board_operation().await;
        start_generation(context.app.clone(), context.session_arc.clone()).await;
        return QueueCommandOutcome::Finished;
    }
    if let Some(reason) = crate::agents::seal::seal_reason(&context.app, &chat_id).await {
        crate::chat::app_context::release(&chat_id, &context.request_id);
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
        app_context_ids,
    ))
    .await;
    let Some(prepared) = prepared else {
        crate::chat::app_context::release(&chat_id, &context.request_id);
        return QueueCommandOutcome::Continue;
    };
    if !prepared.app_context_ids.is_empty()
        && prepared
            .browser_ctx_result
            .as_ref()
            .is_some_and(|(_, oversized)| *oversized)
    {
        crate::chat::app_context::release(&chat_id, &context.request_id);
        reject_guard(
            &mut *context.session_arc.lock().await,
            &context.request_id,
            "App context cannot be sent while browser context is oversized; detach browser context and resend".into(),
        );
        return QueueCommandOutcome::Continue;
    }
    if !Box::pin(handle_oversized_browser_context(context, &prepared)).await {
        return QueueCommandOutcome::Continue;
    }
    if !Box::pin(add_user_messages(context, prepared)).await {
        return QueueCommandOutcome::Continue;
    }
    let app = context.app.clone();
    let session_arc = context.session_arc.clone();
    {
        let mut session = session_arc.lock().await;
        if session.runtime.recovery.is_some() {
            session.archive_recovery();
            session.update_recovery(None);
        }
        session.recovery_budget = None;
    }
    context.release_board_operation().await;
    prepare_session_preamble_and_knowledge(app.clone(), session_arc.clone()).await;
    if aborted_before_start_generation(&session_arc).await {
        return QueueCommandOutcome::Continue;
    }
    start_generation(app.clone(), session_arc.clone()).await;
    QueueCommandOutcome::Finished
}

fn drain_user_batch_after_browser_context(
    session: &mut ChatSession,
    priority: bool,
    browser_oversized: bool,
) -> Vec<CommandRequest> {
    if priority || browser_oversized {
        return Vec::new();
    }
    let messages = drain_non_priority_user_messages(&mut session.command_queue);
    if !messages.is_empty() {
        for message in &messages {
            session.record_command_queue_wait(&message.client_request_id);
        }
        session.emit_queue_update();
    }
    messages
}

async fn prepare_user_message(
    context: &QueueCommandContext,
    mut content: serde_json::Value,
    attachments: Vec<serde_json::Value>,
    context_files: Vec<serde_json::Value>,
    suppress_auto_enrichment: bool,
    client_message_id: Option<String>,
    app_context_ids: Vec<String>,
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

    let additional_messages = drain_user_batch_after_browser_context(
        &mut *session_arc.lock().await,
        priority,
        browser_ctx_result
            .as_ref()
            .is_some_and(|(_, oversized)| *oversized),
    );

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
        session
            .queued_user_merge_targets
            .remove(&context.request_id);
        for additional in &additional_messages {
            session
                .queued_user_merge_targets
                .remove(&additional.client_request_id);
            crate::chat::app_context::release(&session.chat_id, &additional.client_request_id);
        }
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
            let text = if let ChatCommand::UserMessage {
                ref content, ..
            } = additional.command
            {
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
            } else {
                session_arc
                    .lock()
                    .await
                    .queued_user_merge_targets
                    .remove(&additional.client_request_id);
                let chat_id = session_arc.lock().await.chat_id.clone();
                crate::chat::app_context::release(&chat_id, &additional.client_request_id);
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
        app_context_ids,
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
                if let Some(target) = session
                    .queued_user_merge_targets
                    .remove(&context.request_id)
                {
                    session
                        .queued_user_merge_targets
                        .insert(pending_message_id.clone(), target);
                }
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
        app_context_ids,
        checkpoints,
        skill_activation_name,
        skill_context_msg,
        browser_chat_id: _,
        browser_ctx_result,
        additional_messages,
    } = prepared;
    let request_id = &context.request_id;
    let chat_id_for_context = session_arc.lock().await.chat_id.clone();
    let app_context_msgs = match crate::chat::app_context::messages_for(
        &chat_id_for_context,
        request_id,
        &app_context_ids,
    ) {
        Ok(messages) => messages,
        Err(error) => {
            crate::chat::app_context::release(&chat_id_for_context, request_id);
            reject_guard(&mut *session_arc.lock().await, request_id, error);
            return false;
        }
    };
    let mut accepted_user_messages = Vec::new();
    let questions_cancelled;
    let attention_questions;
    {
        let mut session = session_arc.lock().await;
        attention_questions = pending_attention_questions(&app, &session);
        questions_cancelled = session.cancel_pending_questions();

        if let Some((ctx_messages, _)) = browser_ctx_result {
            session.add_queued_user_context(request_id, ctx_messages.event);
            if let Some(screenshot) = ctx_messages.screenshot {
                session.add_queued_user_context(request_id, screenshot);
            }
        }

        if let Some(skill_msg) = skill_context_msg {
            session.add_queued_user_context(request_id, skill_msg);
        }

        if !context_files.is_empty() {
            apply_manual_context_files(&mut session, &context_files, request_id);
        }

        let parsed_content = parse_content_with_attachments(&content, &attachments);
        accepted_user_messages.push(AcceptedUserMessage {
            chat_id: session.chat_id.clone(),
            thread: session.thread.clone(),
            content: parsed_content.clone(),
        });
        let mut user_message = user_message_with_client_message_id(
            parsed_content.clone(),
            checkpoints,
            client_message_id,
        );
        for message in app_context_msgs {
            if let Ok(records) =
                refact_privacy::records_from_messages(std::slice::from_ref(&message))
            {
                crate::privacy::records::merge_records(
                    &mut user_message,
                    records.into_iter().map(|(_, record)| record),
                );
            }
            session.add_queued_user_context(request_id, message);
        }
        session.add_or_merge_queued_user_message(request_id, user_message);
        crate::chat::app_context::commit(&session.chat_id, request_id);
        session.live_diagnostics.accept(std::time::Instant::now());
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
                suppress_auto_enrichment: add_suppress_auto_enrichment,
                client_message_id: add_client_message_id,
                app_context_ids: add_app_context_ids,
            } = additional.command
            {
                let add_context_msgs = match crate::chat::app_context::messages_for(
                    &session.chat_id,
                    &additional.client_request_id,
                    &add_app_context_ids,
                ) {
                    Ok(messages) => messages,
                    Err(error) => {
                        crate::chat::app_context::release(
                            &session.chat_id,
                            &additional.client_request_id,
                        );
                        reject_guard(&mut session, &additional.client_request_id, error);
                        continue;
                    }
                };
                if !add_ctx_files.is_empty() {
                    apply_manual_context_files(
                        &mut session,
                        &add_ctx_files,
                        &additional.client_request_id,
                    );
                }
                if add_suppress_auto_enrichment && add_ctx_files.is_empty() {
                    session.suppress_auto_enrichment_for_next_turn = true;
                }
                let add_parsed = parse_content_with_attachments(&add_content, &add_attachments);
                accepted_user_messages.push(AcceptedUserMessage {
                    chat_id: session.chat_id.clone(),
                    thread: session.thread.clone(),
                    content: add_parsed.clone(),
                });
                let mut add_message = user_message_with_client_message_id(
                    add_parsed,
                    Vec::new(),
                    add_client_message_id,
                );
                for message in add_context_msgs {
                    if let Ok(records) =
                        refact_privacy::records_from_messages(std::slice::from_ref(&message))
                    {
                        crate::privacy::records::merge_records(
                            &mut add_message,
                            records.into_iter().map(|(_, record)| record),
                        );
                    }
                    session.add_queued_user_context(&additional.client_request_id, message);
                }
                session
                    .add_or_merge_queued_user_message(&additional.client_request_id, add_message);
                crate::chat::app_context::commit(&session.chat_id, &additional.client_request_id);
                session.live_diagnostics.accept(std::time::Instant::now());
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

#[cfg(test)]
mod answer_replay_tests {
    use super::*;

    #[tokio::test]
    async fn replayed_answer_preserves_original_accepted_diagnostics_without_new_message() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        gcx.documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .push(dir.path().to_path_buf());
        let app = AppState::from_gcx(gcx).await;
        let mut session = ChatSession::new("answer-replay-diagnostics".into());
        session.thread.mode = "chat".into();
        session.thread.checkpoints_enabled = false;
        session.thread.include_project_info = false;
        session.thread.auto_enrichment_enabled = Some(false);
        session.add_message(ChatMessage {
            message_id: "system".into(),
            role: "system".into(),
            content: ChatContent::SimpleText("Test system prompt".into()),
            ..Default::default()
        });
        let answers = vec![QuestionAnswer {
            id: "question".into(),
            answer: serde_json::json!("original answer"),
        }];
        let answer_content = answer_questions_content("ask-replay", &answers);
        session.add_message(user_message_with_client_message_id(
            ChatContent::SimpleText(answer_content.clone()),
            Vec::new(),
            Some("answer-request".into()),
        ));
        session
            .pending_questions
            .push(refact_chat_api::PendingQuestionSet {
                tool_call_id: "ask-replay".into(),
                questions: vec![refact_chat_api::PendingQuestion {
                    id: "question".into(),
                    question_type: "text".into(),
                    text: "What should be used?".into(),
                    options: None,
                }],
                asked_at_ms: 123,
                status: PendingQuestionStatus::Answered,
                answers: answers.clone(),
                resolution_id: Some("answer-request".into()),
                response_protocol_version: 1,
                response: None,
            });
        let original_accepted_at = std::time::Instant::now() - std::time::Duration::from_secs(30);
        session.live_diagnostics.accept(original_accepted_at);
        let queued: CommandRequest = serde_json::from_value(serde_json::json!({
            "client_request_id": "next-user", "type": "user_message",
            "content": "next turn", "priority": false
        }))
        .unwrap();
        session.command_queue.push_back(queued);
        let session_arc = Arc::new(AMutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert("answer-replay-diagnostics".into(), session_arc.clone());
        let context = QueueCommandContext {
            app,
            session_arc: session_arc.clone(),
            request_id: "answer-request".into(),
            priority: true,
            native_guard: None,
            supervision_guard: None,
            board_operation: AMutex::new(None),
        };

        let outcome =
            handle_answer_questions(&context, "ask-replay".into(), answers, Some(123), None).await;

        assert!(matches!(outcome, QueueCommandOutcome::Continue));
        let session = session_arc.lock().await;
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert!(session.delivery_wake_sources.contains("queue-boundary"));
        assert!(session.draft_message.is_none());
        assert_eq!(session.command_queue.len(), 1);
        let users: Vec<_> = session
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .collect();
        assert_eq!(users.len(), 1, "replay added another answer message");
        assert_eq!(users[0].content.content_text_only(), answer_content);
        assert_eq!(session.live_diagnostics.accepted_count, 1);
        assert_eq!(
            session.live_diagnostics.accepted_at,
            Some(original_accepted_at)
        );
        assert_eq!(
            session.live_diagnostics.phase.map(|(phase, _)| phase),
            Some("request_preparing"),
            "replay did not reach the continuation after persistence"
        );
    }
}

#[cfg(test)]
mod queued_browser_tests {
    use super::*;

    #[tokio::test]
    async fn queued_browser_decision_rebinds_pending_merge_without_new_user() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        gcx.documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .push(dir.path().to_path_buf());
        let app = AppState::from_gcx(gcx).await;
        let mut session = ChatSession::new("queued-browser-decision".into());
        session.thread.checkpoints_enabled = false;
        session.add_message(ChatMessage {
            message_id: "original".into(),
            role: "user".into(),
            content: ChatContent::SimpleText("first".into()),
            ..Default::default()
        });
        session
            .queued_user_merge_targets
            .insert("pending-browser".into(), "original".into());
        session.queued_user_restart_target = Some("original".into());
        session.pending_browser_message = Some(PendingBrowserMessage {
            pending_message_id: "pending-browser".into(),
            content: serde_json::json!("second"),
            attachments: vec![],
            client_message_id: None,
            checkpoints: vec![],
            context_files: vec![],
            suppress_auto_enrichment: false,
            skill_activation_name: None,
            skill_context_msg: None,
        });
        let image_data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIW2Nk+M/wHwAF/gL+3MxZ5wAAAABJRU5ErkJggg==";
        assert!(crate::call_validation::MultimodalElement::new(
            "image/png".into(),
            image_data.into()
        )
        .is_ok());
        let third: CommandRequest = serde_json::from_value(serde_json::json!({
            "client_request_id":"third", "type":"user_message", "priority":false,
            "content":"third", "client_message_id":"third-client",
            "attachments":[{"image_url":{"url":format!("data:image/png;base64,{image_data}")}}],
            "context_files":[{"file_name":"third.rs", "file_content":"third context", "line1":1, "line2":1}]
        })).unwrap();
        session.command_queue.push_back(third.clone());
        session
            .queued_user_merge_targets
            .insert("third".into(), "original".into());
        assert!(drain_user_batch_after_browser_context(&mut session, false, true).is_empty());
        assert_eq!(
            session.command_queue.front().unwrap().client_request_id,
            "third"
        );
        assert_eq!(
            serde_json::to_value(&session.command_queue[0]).unwrap(),
            serde_json::to_value(&third).unwrap()
        );
        session.set_planner_pause("test hold".into(), None);
        let session_arc = Arc::new(AMutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert("queued-browser-decision".into(), session_arc.clone());
        let context = QueueCommandContext {
            app,
            session_arc: session_arc.clone(),
            request_id: "decision-request".into(),
            priority: true,
            native_guard: None,
            supervision_guard: None,
            board_operation: AMutex::new(None),
        };
        handle_browser_context_decision(
            &context,
            "pending-browser".into(),
            false,
            false,
            false,
            false,
            false,
            None,
            None,
            None,
        )
        .await;
        let session = session_arc.lock().await;
        assert!(session.pending_browser_message.is_none());
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "first\nsecond"
        );
        assert_eq!(
            session
                .queued_user_merge_targets
                .get("third")
                .map(String::as_str),
            Some("original")
        );
        assert!(
            session.draft_message.is_none(),
            "decision bypassed planner hold"
        );
        drop(session);
        let third_context = QueueCommandContext {
            request_id: "third".into(),
            priority: false,
            ..context
        };
        let third = session_arc.lock().await.command_queue.pop_front().unwrap();
        let ChatCommand::UserMessage {
            content,
            attachments,
            context_files,
            suppress_auto_enrichment,
            client_message_id,
            ..
        } = third.command
        else {
            panic!("third request changed")
        };
        let prepared = prepare_user_message(
            &third_context,
            content,
            attachments,
            context_files,
            suppress_auto_enrichment,
            client_message_id,
            Vec::new(),
        )
        .await
        .unwrap();
        assert!(add_user_messages(&third_context, prepared).await);
        let session = session_arc.lock().await;
        let users: Vec<_> = session
            .messages
            .iter()
            .filter(|message| message.role == "user")
            .collect();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].message_id, "original");
        assert_eq!(users[0].content.content_text_only(), "first\nsecond\nthird");
        let ChatContent::Multimodal(parts) = &users[0].content else {
            panic!("third attachment lost")
        };
        assert!(parts
            .iter()
            .any(|part| part.m_type == "image/png" && part.m_content == image_data));
        assert!(session.messages.iter().any(|message| matches!(&message.content, ChatContent::ContextFiles(files) if files.iter().any(|file| file.file_content == "third context"))));
        assert!(session.queued_user_merge_targets.is_empty());
    }

    #[tokio::test]
    async fn queued_browser_context_and_screenshot_precede_merged_user() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let mut session = ChatSession::new("queued-browser-merge".into());
        session.thread.checkpoints_enabled = false;
        session.add_message(ChatMessage {
            message_id: "original".into(),
            role: "user".into(),
            content: ChatContent::SimpleText("first".into()),
            ..Default::default()
        });
        session.start_stream().unwrap();
        let request: CommandRequest = serde_json::from_value(serde_json::json!({"client_request_id":"queued", "type":"user_message", "content":"second", "priority":false})).unwrap();
        assert_eq!(
            session.enqueue_command(request),
            types::EnqueueCommandOutcome::Accepted
        );
        let session_arc = Arc::new(AMutex::new(session));
        let context = QueueCommandContext {
            app,
            session_arc: session_arc.clone(),
            request_id: "queued".into(),
            priority: false,
            native_guard: None,
            supervision_guard: None,
            board_operation: AMutex::new(None),
        };
        let prepared = PreparedUserMessage {
            content: serde_json::json!("second"),
            attachments: vec![],
            context_files: vec![],
            suppress_auto_enrichment: false,
            client_message_id: None,
            app_context_ids: Vec::new(),
            checkpoints: vec![],
            skill_activation_name: None,
            skill_context_msg: None,
            browser_chat_id: "queued-browser-merge".into(),
            additional_messages: vec![],
            browser_ctx_result: Some((
                browser_context::BrowserContextMessages {
                    event: ChatMessage {
                        message_id: "browser-event".into(),
                        role: "event".into(),
                        content: ChatContent::SimpleText("browser context".into()),
                        ..Default::default()
                    },
                    screenshot: Some(ChatMessage {
                        message_id: "browser-image".into(),
                        role: "user".into(),
                        content: ChatContent::Multimodal(vec![
                            crate::call_validation::MultimodalElement {
                                m_type: "image/png".into(),
                                m_content: "image-bytes".into(),
                            },
                        ]),
                        ..Default::default()
                    }),
                },
                false,
            )),
        };
        assert!(add_user_messages(&context, prepared).await);
        let session = session_arc.lock().await;
        assert_eq!(
            session
                .messages
                .iter()
                .map(|message| message.message_id.as_str())
                .collect::<Vec<_>>(),
            ["browser-event", "browser-image", "original"]
        );
        assert_eq!(
            session.messages[2].content.content_text_only(),
            "first\nsecond"
        );
        let ChatContent::Multimodal(parts) = &session.messages[1].content else {
            panic!("browser attachment lost")
        };
        assert_eq!(parts[0].m_content, "image-bytes");
        assert!(session.queued_user_merge_targets.is_empty());
    }
}
