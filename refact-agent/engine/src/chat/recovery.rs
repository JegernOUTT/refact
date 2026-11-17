use futures::FutureExt;
use std::sync::{Arc, atomic::Ordering};
use tokio::sync::Mutex;
use refact_core::recovery::*;
use super::types::{ChatSession, SessionState};

pub fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

pub fn validate_policy_patch(patch: &serde_json::Value) -> Result<(), String> {
    if let Some(value) = patch.get("recovery_policy") {
        if !value.is_null() {
            let overrides: RecoveryPolicyOverrides = serde_json::from_value(value.clone())
                .map_err(|e| format!("Invalid recovery policy: {e}"))?;
            overrides.resolve(RecoveryPolicy::default())?;
        }
    }
    Ok(())
}

pub fn apply_model_patch(session: &mut ChatSession, model: &str) -> bool {
    let old = session.thread.model.clone();
    let (changed, patch) = super::queue::apply_setparams_patch(
        &mut session.thread,
        &serde_json::json!({"model": model}),
    );
    if session.thread.model != old {
        super::trajectory_ops::sanitize_messages_for_model_switch(&mut session.messages);
        session.release_turn_only_state();
        session.emit(super::types::ChatEvent::ThreadUpdated {
            params: patch,
        });
        session.mark_persisted_runtime_changed();
    }
    changed
}

pub async fn install_budget(session_arc: &Arc<Mutex<ChatSession>>) -> Result<(), String> {
    let (root, overrides, existing) = {
        let s = session_arc.lock().await;
        (
            s.work_project_root.clone(),
            s.thread.recovery_policy.clone().filter(|p| !p.is_empty()),
            s.recovery_budget.clone(),
        )
    };
    if existing.is_some() {
        return Ok(());
    }
    let (policy, mut source) =
        crate::runtime_settings::effective_recovery_policy(root.as_deref()).await?;
    let policy = if let Some(overrides) = overrides {
        source = RecoveryPolicySource::Thread;
        overrides.resolve(policy)?
    } else {
        policy
    };
    let mut session = session_arc.lock().await;
    session.recovery_policy_source = source;
    session.recovery_budget = Some(Arc::new(RecoveryBudget::new(policy)));
    Ok(())
}

pub fn exhaust_deadline(session: &mut ChatSession, budget: &Arc<RecoveryBudget>) {
    if !session
        .recovery_budget
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, budget))
        || session.abort_flag.load(Ordering::SeqCst)
        || session.runtime.recovery.as_ref().is_some_and(|record| {
            matches!(
                record.state,
                RecoveryState::NeedsAction | RecoveryState::Exhausted | RecoveryState::Cancelled
            )
        })
    {
        return;
    }
    budget.cancel();
    session.preserve_interrupted_recovery_output();
    let mut record = session
        .runtime
        .recovery
        .clone()
        .filter(|record| {
            matches!(
                record.state,
                RecoveryState::Waiting | RecoveryState::Attempting
            )
        })
        .unwrap_or_else(|| RecoveryRecord {
            operation_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            state: RecoveryState::Attempting,
            policy: budget.policy().clone(),
            policy_source: session.recovery_policy_source,
            manual: false,
            retries_used: budget.retries_used(),
            latest_error: RecoveryError {
                category: "Timeout".into(),
                title: "Recovery deadline exhausted".into(),
                explanation: "Task execution admission exceeded the recovery time budget".into(),
                suggested_action: "Retry when execution is available".into(),
                is_retryable: true,
                raw_error: "Recovery deadline exhausted".into(),
            },
            next_retry_at_ms: None,
            deadline_at_ms: Some(now_ms()),
            retry_not_before_ms: None,
            terminal_reason: None,
            attempts: Vec::new(),
        });
    record.state = RecoveryState::Exhausted;
    record.revision = record.revision.saturating_add(1);
    record.next_retry_at_ms = None;
    record.terminal_reason = Some("deadline_exhausted".into());
    session.update_recovery(Some(record));
    session.set_runtime_state(
        SessionState::Error,
        Some("Recovery deadline exhausted".into()),
    );
}

pub async fn within_budget<T>(
    session_arc: &Arc<Mutex<ChatSession>>,
    future: impl std::future::Future<Output = T>,
) -> Option<T> {
    let (budget, notify) = {
        let session = session_arc.lock().await;
        (
            session.recovery_budget.clone(),
            session.abort_notify.clone(),
        )
    };
    let Some(budget) = budget else {
        return Some(future.await);
    };
    let cancelled = async {
        loop {
            let aborted = notify.notified();
            tokio::pin!(aborted);
            aborted.as_mut().enable();
            if session_arc.lock().await.abort_flag.load(Ordering::SeqCst) {
                return;
            }
            aborted.await;
        }
    };
    if budget.check().is_err() {
        if budget.deadline_remaining().is_zero() {
            exhaust_deadline(&mut *session_arc.lock().await, &budget);
        }
        return None;
    }
    let remaining = budget.deadline_remaining();
    tokio::select! {
        biased;
        _ = cancelled => None,
        _ = async {
            if remaining == std::time::Duration::MAX {
                std::future::pending::<()>().await;
            } else {
                tokio::time::sleep(remaining).await;
            }
        } => {
            exhaust_deadline(&mut *session_arc.lock().await, &budget);
            None
        },
        output = future => Some(output),
    }
}

pub async fn handle_failure(
    app: &crate::app_state::AppState,
    session_arc: &Arc<Mutex<ChatSession>>,
    error: &super::stream_core::LlmStreamError,
) -> bool {
    if let Some(budget) = session_arc.lock().await.recovery_budget.as_ref() {
        budget.start_recovery();
    }
    let Some(Ok(operation)) = within_budget(
        session_arc,
        super::session_runtime::begin_board_operation(app, session_arc),
    )
    .await
    else {
        return false;
    };
    let (policy, source) = {
        let s = session_arc.lock().await;
        let Some(budget) = &s.recovery_budget else {
            return false;
        };
        (budget.policy().clone(), s.recovery_policy_source)
    };
    let info = refact_core::retry_policy::user_error_info(
        refact_core::retry_policy::classify_user_error(&error.message),
    );
    let diagnostic = RecoveryError {
        category: if error.is_cooldown() {
            "ProviderCooldown".into()
        } else {
            format!("{:?}", info.category)
        },
        title: if error.is_cooldown() {
            "Provider cooldown"
        } else {
            info.title
        }
        .into(),
        explanation: if error.is_cooldown() {
            "The provider is temporarily unavailable and has requested a cooldown."
        } else {
            info.explanation
        }
        .into(),
        suggested_action: if error.is_cooldown() {
            "Wait for the provider cooldown to expire."
        } else {
            info.suggested_action
        }
        .into(),
        is_retryable: error.is_cooldown() || info.is_retryable,
        raw_error: super::diagnostics::safe_provider_error_diagnostic(&error.message),
    };
    let (budget, notify, retry_notify, next_at, run_id) = {
        let mut s = session_arc.lock().await;
        if s.abort_flag.load(Ordering::SeqCst) || error.retry_decision().is_user_cancelled() {
            return false;
        }
        let budget = s
            .recovery_budget
            .clone()
            .unwrap_or_else(|| Arc::new(RecoveryBudget::new(policy.clone())));
        budget.start_recovery();
        s.recovery_budget = Some(budget.clone());
        let now = now_ms();
        let mut record = s
            .runtime
            .recovery
            .clone()
            .filter(|r| matches!(r.state, RecoveryState::Attempting | RecoveryState::Waiting))
            .unwrap_or_else(|| RecoveryRecord {
                operation_id: uuid::Uuid::new_v4().to_string(),
                run_id: uuid::Uuid::new_v4().to_string(),
                revision: 0,
                state: RecoveryState::Attempting,
                policy,
                policy_source: source,
                manual: false,
                retries_used: 0,
                latest_error: diagnostic.clone(),
                next_retry_at_ms: None,
                retry_not_before_ms: None,
                deadline_at_ms: Some(
                    now.saturating_add(
                        budget
                            .deadline_remaining()
                            .as_millis()
                            .min(u64::MAX as u128) as u64,
                    ),
                ),
                terminal_reason: None,
                attempts: Vec::new(),
            });
        record.revision = record.revision.saturating_add(1);
        record.retries_used = budget.retries_used();
        record.latest_error = diagnostic.clone();
        let no_new_send = record
            .attempts
            .iter()
            .rev()
            .find(|attempt| attempt.outcome != "admission_deferred")
            .is_some_and(|attempt| attempt.retry_number == record.retries_used);
        record.attempts.push(RecoveryAttempt {
            at_ms: now,
            retry_number: record.retries_used,
            error: Some(diagnostic),
            outcome: if error.is_cooldown() {
                "admission_deferred".into()
            } else {
                "failed".into()
            },
        });
        if record.attempts.len() > 101 {
            record.attempts.remove(0);
        }
        record.retry_not_before_ms = error.retry_not_before_ms();
        let eligible = (error.is_cooldown() || (!no_new_send && budget.sends_used() > 0))
            && (record.manual
                || (record.policy.mode == RecoveryMode::Transient
                    && error.retry_decision().is_retryable_transient()));
        let exhausted = budget.check().is_err() || budget.sends_used() > record.policy.max_retries;
        record.state = if exhausted {
            RecoveryState::Exhausted
        } else if eligible {
            RecoveryState::Waiting
        } else {
            RecoveryState::NeedsAction
        };
        let next = now
            .saturating_add(record.policy.delay_ms)
            .max(record.retry_not_before_ms.unwrap_or(0));
        record.next_retry_at_ms = (record.state == RecoveryState::Waiting).then_some(next);
        if exhausted {
            record.terminal_reason = Some(
                if budget.deadline_remaining().is_zero() {
                    "deadline_exhausted"
                } else {
                    "retry_limit_exhausted"
                }
                .into(),
            );
        }
        let run_id = record.run_id.clone();
        let waiting = record.state == RecoveryState::Waiting;
        s.preserve_interrupted_recovery_output();
        let mut message =
            super::diagnostics::make_ui_only_error_message(&record.latest_error.raw_error);
        message.extra.insert(
            "error_info".into(),
            serde_json::to_value(&record.latest_error).expect("recovery error is serializable"),
        );
        message.extra.insert(
            "recovery_operation_id".into(),
            serde_json::json!(record.operation_id),
        );
        message
            .extra
            .insert("recovery_run_id".into(), serde_json::json!(record.run_id));
        s.append_error_message(message);
        s.update_recovery(Some(record));
        s.set_runtime_state(
            if waiting {
                SessionState::Generating
            } else {
                SessionState::Error
            },
            if waiting {
                None
            } else {
                Some(info.title.into())
            },
        );
        if !waiting {
            return false;
        }
        (
            budget,
            s.abort_notify.clone(),
            s.recovery_notify.clone(),
            next,
            run_id,
        )
    };
    super::trajectories::maybe_save_trajectory_background_with_intent(
        app.clone(),
        session_arc.clone(),
        super::types::TrajectoryCommitIntent::Checkpoint,
    );
    drop(operation);
    wait_for_retry(session_arc, budget, notify, retry_notify, next_at, run_id).await
}

pub async fn wait_for_retry(
    session_arc: &Arc<Mutex<ChatSession>>,
    budget: Arc<RecoveryBudget>,
    notify: Arc<tokio::sync::Notify>,
    retry_notify: Arc<tokio::sync::Notify>,
    next_at: u64,
    run_id: String,
) -> bool {
    let aborted = notify.notified();
    tokio::pin!(aborted);
    aborted.as_mut().enable();
    {
        let s = session_arc.lock().await;
        if s.abort_flag.load(Ordering::SeqCst) || budget.check().is_err() {
            return false;
        }
    }
    let wait = std::time::Duration::from_millis(next_at.saturating_sub(now_ms()));
    let elapsed = tokio::select! {
        biased;
        _ = &mut aborted => false,
        _ = tokio::time::sleep(budget.deadline_remaining()) => false,
        _ = tokio::time::sleep(wait) => true,
        _ = retry_notify.notified() => true,
    };
    let mut s = session_arc.lock().await;
    while retry_notify.notified().now_or_never().is_some() {}
    if s.abort_flag.load(Ordering::SeqCst) {
        return false;
    }
    let Some(mut record) = s
        .runtime
        .recovery
        .clone()
        .filter(|r| r.run_id == run_id && r.state == RecoveryState::Waiting)
    else {
        return false;
    };
    record.revision = record.revision.saturating_add(1);
    record.next_retry_at_ms = None;
    let ready =
        elapsed && budget.check().is_ok() && record.retry_not_before_ms.unwrap_or(0) <= now_ms();
    record.state = if ready {
        RecoveryState::Attempting
    } else {
        RecoveryState::Exhausted
    };
    if !ready {
        record.terminal_reason = Some("deadline_exhausted".into());
    }
    s.update_recovery(Some(record));
    s.set_runtime_state(
        if ready {
            SessionState::Idle
        } else {
            SessionState::Error
        },
        None,
    );
    ready
}

pub fn succeeded(session: &mut ChatSession) {
    if let Some(mut record) = session.runtime.recovery.clone() {
        if matches!(
            record.state,
            RecoveryState::Waiting | RecoveryState::Attempting
        ) {
            record.retries_used = session
                .recovery_budget
                .as_ref()
                .map(|b| b.retries_used())
                .unwrap_or(record.retries_used);
            record.attempts.push(RecoveryAttempt {
                at_ms: now_ms(),
                retry_number: record.retries_used,
                error: None,
                outcome: "succeeded".into(),
            });
            if record.attempts.len() > 101 {
                record.attempts.remove(0);
            }
            record.state = RecoveryState::Succeeded;
            record.revision = record.revision.saturating_add(1);
            record.next_retry_at_ms = None;
            record.terminal_reason = Some("succeeded".into());
            session.update_recovery(Some(record));
        }
    }
    session.recovery_budget = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(policy: RecoveryPolicy) -> RecoveryRecord {
        RecoveryRecord {
            operation_id: "operation".into(),
            run_id: "run".into(),
            revision: 7,
            state: RecoveryState::Waiting,
            policy,
            policy_source: RecoveryPolicySource::Thread,
            manual: true,
            retries_used: 0,
            latest_error: RecoveryError {
                category: "AuthenticationFailed".into(),
                title: "Authentication failed".into(),
                explanation: "Expired token".into(),
                suggested_action: "Sign in".into(),
                is_retryable: false,
                raw_error: "redacted".into(),
            },
            next_retry_at_ms: Some(now_ms() + 60_000),
            retry_not_before_ms: None,
            deadline_at_ms: Some(now_ms() + 120_000),
            terminal_reason: None,
            attempts: Vec::new(),
        }
    }

    fn stamped_value(record: &RecoveryRecord) -> serde_json::Value {
        let mut value = serde_json::to_value(record).unwrap();
        for attempt in value["attempts"].as_array_mut().unwrap() {
            attempt["error"] = serde_json::Value::Null;
        }
        value
    }

    #[tokio::test(start_paused = true)]
    async fn abort_waiting_recovery_preserves_partial_as_inert_artifact() {
        let policy = RecoveryPolicy::default();
        let mut session = ChatSession::new("recovery-stop".into());
        let budget = Arc::new(RecoveryBudget::new_manual(policy.clone()));
        session.recovery_budget = Some(budget.clone());
        session.runtime.recovery = Some(record(policy));
        session
            .start_stream()
            .expect("draft stream must exist before Stop");
        session.draft_message.as_mut().unwrap().content =
            crate::call_validation::ChatContent::SimpleText("accepted partial".into());
        let notify = session.abort_notify.clone();
        let retry = session.recovery_notify.clone();
        let session = Arc::new(Mutex::new(session));
        let mut waiter = Box::pin(wait_for_retry(
            &session,
            budget.clone(),
            notify,
            retry,
            now_ms() + 60_000,
            "run".into(),
        ));
        assert!(matches!(
            futures::poll!(&mut waiter),
            std::task::Poll::Pending
        ));
        session.lock().await.apply_priority_abort();
        assert!(!waiter.await);
        let session = session.lock().await;
        let archived = session
            .messages
            .iter()
            .find(|m| m.content.content_text_only() == "accepted partial")
            .expect("Stop must preserve partial output");
        assert!(super::super::diagnostics::is_ui_only_message(archived));
        assert!(
            super::super::diagnostics::filter_ui_only_messages(vec![archived.clone()]).is_empty()
        );
        assert_eq!(
            session.runtime.recovery.as_ref().unwrap().state,
            RecoveryState::Cancelled
        );
        assert_eq!(budget.reserve_send(), Err("Recovery cancelled".into()));
    }

    #[tokio::test(start_paused = true)]
    async fn retry_wait_refuses_stale_run_identity() {
        let policy = RecoveryPolicy::default();
        let budget = Arc::new(RecoveryBudget::new_manual(policy.clone()));
        let mut session = ChatSession::new("recovery-stale".into());
        session.runtime.recovery = Some(record(policy));
        session.runtime.recovery.as_mut().unwrap().run_id = "replacement".into();
        let notify = session.abort_notify.clone();
        let retry = session.recovery_notify.clone();
        let session = Arc::new(Mutex::new(session));
        assert!(!wait_for_retry(&session, budget, notify, retry, now_ms(), "run".into()).await);
        let session = session.lock().await;
        assert_eq!(
            session.runtime.recovery.as_ref().unwrap().run_id,
            "replacement"
        );
        assert_eq!(
            session.runtime.recovery.as_ref().unwrap().state,
            RecoveryState::Waiting
        );
        assert_eq!(session.runtime.recovery.as_ref().unwrap().revision, 7);
    }
    async fn failure_test_app() -> crate::app_state::AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::app_state::AppState::from_gcx(gcx).await
    }

    fn stream_error(message: &str) -> super::super::stream_core::LlmStreamError {
        super::super::stream_core::LlmStreamError {
            message: message.into(),
            partial_output_emitted: true,
            pressure_failure: None,
            admission_refusal: None,
        }
    }

    #[tokio::test]
    async fn handle_failure_preserves_partial_before_linked_error() {
        let app = failure_test_app().await;
        let mut session = ChatSession::new("failure-partial".into());
        session.recovery_budget = Some(Arc::new(RecoveryBudget::new(RecoveryPolicy {
            mode: RecoveryMode::Off,
            ..RecoveryPolicy::default()
        })));
        session.start_stream().unwrap();
        session.draft_message.as_mut().unwrap().content =
            crate::call_validation::ChatContent::SimpleText("accepted partial".into());
        let session = Arc::new(Mutex::new(session));
        let error = stream_error("LLM error (401 Unauthorized): expired token");
        assert!(!handle_failure(&app, &session, &error).await);
        let session = session.lock().await;
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].role, "assistant");
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "accepted partial"
        );
        assert!(super::super::diagnostics::is_ui_only_message(
            &session.messages[0]
        ));
        let record = session.runtime.recovery.as_ref().unwrap();
        assert_eq!(record.state, RecoveryState::NeedsAction);
        assert_eq!(record.attempts.len(), 1);
        let message = &session.messages[1];
        assert_eq!(message.role, "error");
        assert_eq!(message.extra["recovery_operation_id"], record.operation_id);
        assert_eq!(message.extra["recovery_run_id"], record.run_id);
        assert_eq!(
            message.extra["error_info"],
            serde_json::to_value(&record.latest_error).unwrap()
        );
        assert_eq!(message.extra["recovery_record"], stamped_value(record));
        assert!(session.draft_message.is_none());
    }

    #[tokio::test]
    async fn handle_failure_collapses_only_same_operation_and_refreshes_run() {
        let app = failure_test_app().await;
        let policy = RecoveryPolicy {
            mode: RecoveryMode::Off,
            ..RecoveryPolicy::default()
        };
        let mut session = ChatSession::new("failure-repeat".into());
        session.recovery_budget = Some(Arc::new(RecoveryBudget::new(policy.clone())));
        let mut recovery = record(policy);
        recovery.state = RecoveryState::Attempting;
        recovery.manual = false;
        session.runtime.recovery = Some(recovery);
        let session = Arc::new(Mutex::new(session));
        let error = stream_error("LLM error (401 Unauthorized): expired token");
        assert!(!handle_failure(&app, &session, &error).await);
        let first_id = {
            let mut session = session.lock().await;
            assert_eq!(session.messages.len(), 1);
            let id = session.messages[0].message_id.clone();
            let recovery = session.runtime.recovery.as_mut().unwrap();
            recovery.state = RecoveryState::Attempting;
            recovery.run_id = "manual-retry-run".into();
            id
        };
        assert!(!handle_failure(&app, &session, &error).await);
        {
            let mut session = session.lock().await;
            assert_eq!(session.messages.len(), 1);
            assert_eq!(session.messages[0].message_id, first_id);
            assert_eq!(session.messages[0].extra["repeat_count"], 2);
            assert_eq!(
                session.messages[0].extra["recovery_run_id"],
                "manual-retry-run"
            );
            assert_eq!(
                session.messages[0].extra["recovery_record"]["run_id"],
                "manual-retry-run"
            );
            assert_eq!(session.runtime.recovery.as_ref().unwrap().attempts.len(), 2);
            session.runtime.recovery = None;
        }
        assert!(!handle_failure(&app, &session, &error).await);
        let session = session.lock().await;
        assert_eq!(session.messages.len(), 2);
        assert_ne!(session.messages[1].message_id, first_id);
        assert_ne!(
            session.messages[1].extra["recovery_operation_id"],
            "operation"
        );
        assert_eq!(session.messages[0].extra["repeat_count"], 2);
        assert_eq!(
            session.messages[1].extra["recovery_record"],
            stamped_value(session.runtime.recovery.as_ref().unwrap())
        );
    }

    #[test]
    fn recovery_updates_stamp_latest_error_and_archive_without_duplicates() {
        let mut session = ChatSession::new("recovery-stamping".into());
        let mut recovery = record(RecoveryPolicy::default());
        assert!(session.messages.is_empty());
        session.update_recovery(Some(recovery.clone()));
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].content.content_text_only(), "redacted");
        assert_eq!(
            session.messages[0].extra["recovery_operation_id"],
            serde_json::json!(recovery.operation_id)
        );
        recovery.state = RecoveryState::Exhausted;
        recovery.revision += 1;
        session.update_recovery(Some(recovery.clone()));
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].extra["recovery_record"]["state"],
            "exhausted"
        );
        recovery.state = RecoveryState::Succeeded;
        recovery.revision += 1;
        session.update_recovery(Some(recovery.clone()));
        assert_eq!(
            session.messages[0].extra["recovery_record"]["state"],
            "succeeded"
        );
        session.archive_recovery();
        assert_eq!(session.messages.len(), 1);
        let mut empty = ChatSession::new("successful".into());
        empty.update_recovery(Some(recovery));
        empty.archive_recovery();
        assert!(empty.messages.is_empty());
    }

    #[test]
    fn stamped_record_drops_attempt_diagnostics_but_runtime_record_keeps_them() {
        let mut session = ChatSession::new("recovery-stamp-size".into());
        let mut recovery = record(RecoveryPolicy::default());
        recovery.retries_used = 2;
        recovery.attempts = (0..2)
            .map(|retry_number| RecoveryAttempt {
                at_ms: 10 + retry_number as u64,
                retry_number,
                error: Some(RecoveryError {
                    raw_error: "x".repeat(2_000),
                    ..recovery.latest_error.clone()
                }),
                outcome: "failed".into(),
            })
            .collect();
        session.update_recovery(Some(recovery.clone()));
        recovery.revision += 1;
        recovery.state = RecoveryState::Exhausted;
        session.update_recovery(Some(recovery.clone()));

        assert_eq!(session.messages.len(), 1);
        let stamped = &session.messages[0].extra["recovery_record"];
        assert_eq!(stamped, &stamped_value(&recovery));
        assert_eq!(stamped["latest_error"]["raw_error"], "redacted");
        let runtime = session.runtime.recovery.as_ref().unwrap();
        assert_eq!(
            serde_json::to_value(runtime).unwrap(),
            serde_json::to_value(&recovery).unwrap()
        );
        assert!(runtime.attempts.iter().all(|attempt| attempt
            .error
            .as_ref()
            .is_some_and(|error| error.raw_error.len() == 2_000)));
    }

    #[test]
    fn recovery_archive_fallback_precedes_new_user_and_interrupts_waiting() {
        let mut session = ChatSession::new("recovery-archive".into());
        session.runtime.recovery = Some(record(RecoveryPolicy::default()));
        assert!(session.messages.is_empty());
        session.archive_recovery();
        session.add_message(crate::call_validation::ChatMessage::new(
            "user".into(),
            "next".into(),
        ));
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].role, "error");
        assert_eq!(
            session.messages[0].extra["recovery_record"]["state"],
            "cancelled"
        );
        assert_eq!(
            session.messages[0].extra["recovery_record"]["terminal_reason"],
            "superseded"
        );
        assert_eq!(session.messages[1].role, "user");
    }
}
