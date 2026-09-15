//! A goal that is waiting on something names what it waits on, consumes no
//! autonomous turns while it waits, and wakes once when that subject settles.
//!
//! Waiting is not stalling and not passing: the wait ends either because the
//! subject settled or because its deadline expired, and expiry is reported as
//! its own outcome rather than quietly becoming a pass.

use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::chat::internal_roles::{self, EventSubkind};
use crate::chat::trajectories::maybe_save_trajectory_background_with_intent;
use crate::chat::types::*;

pub const GOAL_WAIT_SOURCE: &str = "chat.goal_monitor";

fn epoch_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn goal_wait_begin_event(wait: &GoalWait, at_ms: u64) -> ChatMessage {
    let subject = wait.subject.label();
    internal_roles::event(
        EventSubkind::GoalPursuit,
        GOAL_WAIT_SOURCE,
        json!({
            "kind": "waiting",
            "subject": subject,
            "deadline_ms": wait.deadline_ms,
            "at_ms": at_ms,
        }),
        format!("Goal pursuit waiting on {subject}; no goal rounds until it settles."),
    )
}

pub fn goal_wait_resume_event(wait: &GoalWait, resume: GoalWaitResume, at_ms: u64) -> ChatMessage {
    let subject = wait.subject.label();
    let (kind, text) = match resume {
        GoalWaitResume::Settled => (
            "wait_settled",
            format!("Goal pursuit resumed: {subject} settled."),
        ),
        GoalWaitResume::Expired => (
            "wait_expired",
            format!("Goal wait expired: {subject} did not settle before its deadline."),
        ),
        GoalWaitResume::Cancelled => ("wait_cancelled", format!("Goal wait cancelled: {subject}.")),
    };
    internal_roles::event(
        EventSubkind::GoalPursuit,
        GOAL_WAIT_SOURCE,
        json!({
            "kind": kind,
            "subject": subject,
            "resume": resume.label(),
            "at_ms": at_ms,
        }),
        text,
    )
}

/// The single-shot half of the resume. `goal_settle_wait` hands the wait back to
/// exactly one caller, so two wake sources racing on the same subject produce one
/// resume event and one wake.
pub fn settle_goal_wait_in_session(
    session: &mut ChatSession,
    subject: Option<&GoalWaitSubject>,
    resume: GoalWaitResume,
    at_ms: u64,
) -> Option<GoalWait> {
    let settled = session.goal_settle_wait(subject, at_ms)?;
    let wake = resume != GoalWaitResume::Cancelled;
    let delivery = PendingDelivery::new(
        vec![goal_wait_resume_event(&settled, resume, at_ms)],
        PushMode::Append,
        GOAL_WAIT_SOURCE,
        wake,
    );
    match session.enqueue_delivery(delivery) {
        Ok(DeliveryOutcome::Delivered) if wake => session.queue_notify.notify_one(),
        Err(error) => tracing::warn!(%error, "goal wait resume delivery rejected"),
        _ => {}
    }
    Some(settled)
}

/// Recheck on event, not on a timer: whoever observes the subject settling calls
/// this, and only the first observation wakes the goal.
pub async fn settle_goal_wait(
    app: AppState,
    chat_id: &str,
    subject: &GoalWaitSubject,
    resume: GoalWaitResume,
) -> bool {
    let session_arc = {
        let sessions = app.chat.sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return false;
    };
    let settled = {
        let mut session = session_arc.lock().await;
        settle_goal_wait_in_session(&mut session, Some(subject), resume, epoch_ms_now()).is_some()
    };
    if settled {
        resume_goal_wait_execution(&app, chat_id).await;
        maybe_save_trajectory_background_with_intent(
            app,
            session_arc,
            TrajectoryCommitIntent::Checkpoint,
        );
    }
    settled
}

/// A wait must not occupy scheduler capacity. This is the same park the
/// scheduler performs for every other registered wait; a run still draining
/// operations keeps its permit rather than blocking here.
pub fn park_goal_wait_execution(app: &AppState, chat_id: &str) -> bool {
    let Some(execution) = app.agent_executions.get(chat_id) else {
        return false;
    };
    match execution.park_now() {
        Ok(outcome) => matches!(
            outcome,
            crate::agents::execution::ParkOutcome::Parked
                | crate::agents::execution::ParkOutcome::AlreadyParked
        ),
        Err(error) => {
            tracing::debug!(%chat_id, %error, "goal wait could not release its permit");
            false
        }
    }
}

pub async fn resume_goal_wait_execution(app: &AppState, chat_id: &str) {
    let Some(execution) = app.agent_executions.get(chat_id) else {
        return;
    };
    if let Err(error) = execution.resume().await {
        tracing::debug!(%chat_id, %error, "goal wait could not reacquire its permit");
    }
}

pub async fn begin_goal_wait_park(app: AppState, session_arc: Arc<AMutex<ChatSession>>) {
    let chat_id = {
        let session = session_arc.lock().await;
        if session.goal_wait().is_none() {
            return;
        }
        session.chat_id.clone()
    };
    park_goal_wait_execution(&app, &chat_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_chat_api::GoalBudget;

    fn waiting_session(subject: GoalWaitSubject, deadline_ms: Option<u64>) -> ChatSession {
        let mut session = ChatSession::new("goal-wait-test".to_string());
        session.install_goal("agent", "ship it", true, GoalBudget::default());
        assert!(session.goal_begin_wait(GoalWait::new(subject, deadline_ms), 1_000));
        session
    }

    fn resume_event_count(session: &ChatSession, kind: &str) -> usize {
        session
            .messages
            .iter()
            .chain(
                session
                    .pending_deliveries
                    .iter()
                    .flat_map(|delivery| delivery.messages.iter()),
            )
            .filter(|message| {
                message
                    .extra
                    .get("event")
                    .and_then(|event| event.get("payload"))
                    .and_then(|payload| payload.get("kind"))
                    .and_then(|value| value.as_str())
                    == Some(kind)
            })
            .count()
    }

    /// Two wake sources racing on the same wait — a process exit and the
    /// deadline sweep — must resume the goal once, not twice.
    #[test]
    fn a_settled_subject_wakes_the_goal_exactly_once() {
        let subject = GoalWaitSubject::Process {
            process_id: "proc-7".to_string(),
        };
        let mut session = waiting_session(subject.clone(), Some(5_000));

        let first = settle_goal_wait_in_session(
            &mut session,
            Some(&subject),
            GoalWaitResume::Settled,
            2_000,
        );
        let second =
            settle_goal_wait_in_session(&mut session, None, GoalWaitResume::Expired, 5_000);

        assert!(first.is_some());
        assert!(second.is_none(), "the losing wake source must find nothing");
        assert!(session.goal_wait().is_none());
        assert_eq!(resume_event_count(&session, "wait_settled"), 1);
        assert_eq!(resume_event_count(&session, "wait_expired"), 0);
    }

    /// A wake for a different subject is not this goal's wake.
    #[test]
    fn a_wake_for_another_subject_leaves_the_wait_standing() {
        let subject = GoalWaitSubject::Child {
            agent_id: "bgagent-1".to_string(),
        };
        let mut session = waiting_session(subject, None);

        let settled = settle_goal_wait_in_session(
            &mut session,
            Some(&GoalWaitSubject::Child {
                agent_id: "bgagent-2".to_string(),
            }),
            GoalWaitResume::Settled,
            2_000,
        );

        assert!(settled.is_none());
        assert!(session.goal_wait().is_some());
        assert_eq!(resume_event_count(&session, "wait_settled"), 0);
    }

    #[test]
    fn a_wait_is_installed_once_and_survives_a_projection_rebuild() {
        let subject = GoalWaitSubject::Event {
            name: "ci-run-42".to_string(),
        };
        let mut session = waiting_session(subject.clone(), None);

        assert!(
            !session.goal_begin_wait(GoalWait::new(subject.clone(), None), 2_000),
            "a live wait keeps its identity"
        );
        assert_eq!(session.goal_wait().unwrap().since_ms, 1_000);

        session.rebuild_goal_projection_from_messages();
        assert_eq!(session.goal_wait().unwrap().subject, subject);

        let replayed = refact_chat_api::reduce_goal_ledger(&session.goal_ledger).unwrap();
        assert_eq!(replayed.wait.unwrap().subject, subject);
    }

    #[test]
    fn a_user_message_abandons_the_wait() {
        let mut session = waiting_session(
            GoalWaitSubject::Process {
                process_id: "proc-1".to_string(),
            },
            None,
        );

        assert!(session.goal_reset_no_progress());

        assert!(session.goal_wait().is_none());
        let replayed = refact_chat_api::reduce_goal_ledger(&session.goal_ledger).unwrap();
        assert!(replayed.wait.is_none());
    }
}
