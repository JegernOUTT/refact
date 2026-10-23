// Shared planner -> task-agent live message delivery.
//
// Every planner-side producer (agent_steer, planner_reply, task_broadcast,
// pause_agent, resume_agent) targets a single board card and has to answer the
// same questions: is the card still the card we validated, is its agent chat
// still the same chat, is there a live session, and did the message actually
// get delivered. This module owns that reserve/revalidate/deliver sequence and
// is the only place in the tools layer that touches the unified delivery API,
// so a signature change lands in one file.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, StatusUpdate};
use refact_core::chat_types::{DeliveryOutcome, PendingDelivery, PushMode};
use refact_runtime_api::{ChatSessionFacade, SessionState};

pub(crate) fn push_mode_schema() -> Value {
    PushMode::schema()
}

pub(crate) fn parse_push_mode(args: &HashMap<String, Value>) -> Result<PushMode, String> {
    PushMode::from_args(args)
}

pub(crate) fn push_mode_label(push: PushMode) -> &'static str {
    match push {
        PushMode::Preempt => "preempt",
        PushMode::Append => "append",
        PushMode::WhenIdle => "when_idle",
    }
}

pub(crate) fn planner_user_message(content: String) -> ChatMessage {
    ChatMessage {
        role: "user".to_string(),
        content: ChatContent::SimpleText(content),
        ..Default::default()
    }
}

pub(crate) fn delivery(
    messages: Vec<ChatMessage>,
    push: PushMode,
    source: &str,
    wake: bool,
) -> PendingDelivery {
    PendingDelivery::new(messages, push, source.to_string(), wake)
}

/// Attach a caller-chosen stable id so the delivery layer can drop a repeat of
/// the same logical message (a retried planner_reply, for example).
pub(crate) fn with_dedupe_id(mut delivery: PendingDelivery, id: String) -> PendingDelivery {
    delivery.id = id;
    delivery
}

pub(crate) fn single_message_delivery(
    content: String,
    push: PushMode,
    source: &str,
    wake: bool,
) -> PendingDelivery {
    delivery(vec![planner_user_message(content)], push, source, wake)
}

#[derive(Clone)]
pub(crate) struct CardTarget {
    pub card_id: String,
    pub title: String,
    pub chat_id: String,
}

impl CardTarget {
    pub fn from_card(card: &BoardCard, chat_id: String) -> Self {
        Self {
            card_id: card.id.clone(),
            title: card.title.clone(),
            chat_id,
        }
    }
}

/// What must still be true about the card at delivery time.
#[derive(Clone, Copy)]
pub(crate) struct CardGuard {
    /// Require the card to still be in the `doing` column.
    pub require_doing: bool,
    /// Probe the session first and report "no live session" instead of
    /// delivering into a chat that no longer exists.
    pub require_live_session: bool,
}

impl CardGuard {
    pub fn doing_with_live_session() -> Self {
        Self {
            require_doing: true,
            require_live_session: true,
        }
    }

    pub fn doing() -> Self {
        Self {
            require_doing: true,
            require_live_session: false,
        }
    }
}

/// Honest per-card result. `Skipped`/`NoLiveSession`/`Failed` all mean the
/// message did NOT reach the agent; only `Delivered` may be reported as success
/// and it still distinguishes a fresh delivery from a deduped repeat.
pub(crate) enum DeliveryReport {
    Delivered {
        outcome: DeliveryOutcome,
        prior_state: Option<SessionState>,
    },
    NoLiveSession,
    Skipped {
        reason: String,
    },
    Failed {
        error: String,
    },
}

impl DeliveryReport {
    /// True only when this call actually handed a new message to the agent.
    pub fn reached_agent(&self) -> bool {
        matches!(
            self,
            DeliveryReport::Delivered {
                outcome: DeliveryOutcome::Queued | DeliveryOutcome::Delivered,
                ..
            }
        )
    }

    /// Machine-readable receipt: acceptance by the delivery layer, never consumption.
    pub fn receipt(&self) -> serde_json::Value {
        let (outcome, reason, prior_state) = match self {
            DeliveryReport::Delivered {
                outcome,
                prior_state,
            } => (
                match outcome {
                    DeliveryOutcome::Queued => "queued",
                    DeliveryOutcome::Delivered => "delivered",
                    DeliveryOutcome::Duplicate => "duplicate",
                    DeliveryOutcome::Expired => "expired",
                    DeliveryOutcome::Superseded => "superseded",
                    DeliveryOutcome::Sealed => "sealed",
                },
                None,
                prior_state.map(|state| state.to_string()),
            ),
            DeliveryReport::NoLiveSession => ("no_live_session", None, None),
            DeliveryReport::Skipped { reason } => ("skipped", Some(reason.clone()), None),
            DeliveryReport::Failed { error } => ("failed", Some(error.clone()), None),
        };
        serde_json::json!({
            "outcome": outcome,
            "reached": self.reached_agent(),
            "reason": reason,
            "prior_state": prior_state,
        })
    }

    pub fn short_status(&self) -> String {
        match self {
            DeliveryReport::Delivered { outcome, .. } => match outcome {
                DeliveryOutcome::Queued => "queued".to_string(),
                DeliveryOutcome::Delivered => "delivered".to_string(),
                DeliveryOutcome::Duplicate => {
                    "not delivered (duplicate of an already-sent message)".to_string()
                }
                DeliveryOutcome::Expired => {
                    "not delivered (its deadline passed before a boundary opened)".to_string()
                }
                DeliveryOutcome::Superseded => {
                    "not delivered (replaced by a newer message with the same key)".to_string()
                }
                DeliveryOutcome::Sealed => "not delivered (the chat is sealed)".to_string(),
            },
            DeliveryReport::NoLiveSession => "not delivered (no live agent session)".to_string(),
            DeliveryReport::Skipped { reason } => format!("not delivered ({})", reason),
            DeliveryReport::Failed { error } => format!("not delivered (failed: {})", error),
        }
    }

    pub fn describe(&self, action: &str) -> String {
        match self {
            DeliveryReport::Delivered {
                outcome,
                prior_state,
            } => {
                let state = prior_state
                    .map(|state| state.to_string())
                    .unwrap_or_else(|| "unavailable".to_string());
                // Exhaustive on purpose: a new dropped outcome must not fall
                // through into the phrasing reserved for a delivery that landed.
                match outcome {
                    DeliveryOutcome::Duplicate => format!(
                        "{action} message was NOT delivered: the delivery layer recognised it as a duplicate of an already-sent message."
                    ),
                    DeliveryOutcome::Expired => format!(
                        "{action} message was NOT delivered: its deadline passed before a boundary opened."
                    ),
                    DeliveryOutcome::Superseded => format!(
                        "{action} message was NOT delivered: a newer message with the same key replaced it."
                    ),
                    DeliveryOutcome::Sealed => format!(
                        "{action} message was NOT delivered: the chat is sealed."
                    ),
                    DeliveryOutcome::Queued | DeliveryOutcome::Delivered => format!(
                        "{action} message {}; prior agent state: {state}",
                        self.short_status()
                    ),
                }
            }
            DeliveryReport::NoLiveSession => format!(
                "{} message was NOT delivered: no live agent session was found.",
                action
            ),
            DeliveryReport::Skipped { reason } => {
                format!("{} message was NOT delivered: {}.", action, reason)
            }
            DeliveryReport::Failed { error } => {
                format!(
                    "{} message was NOT delivered: delivery failed: {}",
                    action, error
                )
            }
        }
    }
}

fn stale_card_reason(
    card: Option<&BoardCard>,
    card_id: &str,
    expected_chat_id: &str,
    require_doing: bool,
) -> Option<String> {
    let Some(card) = card else {
        return Some(format!("card {} no longer exists", card_id));
    };
    if require_doing && card.column != "doing" {
        return Some(format!(
            "card {} is now in column '{}'",
            card_id, card.column
        ));
    }
    if card.agent_chat_id.as_deref() != Some(expected_chat_id) {
        return Some(format!(
            "agent_chat_id changed from '{}' to '{}'",
            expected_chat_id,
            card.agent_chat_id.as_deref().unwrap_or("none")
        ));
    }
    None
}

async fn card_is_stale(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    target: &CardTarget,
    require_doing: bool,
) -> Result<Option<String>, String> {
    let board = storage::load_board(gcx, task_id).await?;
    Ok(stale_card_reason(
        board.get_card(&target.card_id),
        &target.card_id,
        &target.chat_id,
        require_doing,
    ))
}

/// Revalidate the card, optionally probe for a live session, revalidate once
/// more to close the window opened by that probe, then deliver.
pub(crate) async fn deliver_to_card(
    gcx: Arc<GlobalContext>,
    facade: Arc<dyn ChatSessionFacade>,
    task_id: &str,
    target: &CardTarget,
    guard: CardGuard,
    pending: PendingDelivery,
) -> DeliveryReport {
    match card_is_stale(gcx.clone(), task_id, target, guard.require_doing).await {
        Ok(Some(reason)) => return DeliveryReport::Skipped { reason },
        Ok(None) => {}
        Err(error) => {
            return DeliveryReport::Failed {
                error: format!("could not re-read the board: {}", error),
            }
        }
    }

    let mut prior_state = None;
    if guard.require_live_session {
        match facade.session_state(&target.chat_id).await {
            Ok(None) => return DeliveryReport::NoLiveSession,
            Ok(state) => prior_state = state,
            Err(error) => {
                return DeliveryReport::Failed {
                    error: format!("could not read the agent session state: {}", error),
                }
            }
        }

        match card_is_stale(gcx, task_id, target, guard.require_doing).await {
            Ok(Some(reason)) => return DeliveryReport::Skipped { reason },
            Ok(None) => {}
            Err(error) => {
                return DeliveryReport::Failed {
                    error: format!("could not re-read the board: {}", error),
                }
            }
        }
    } else if let Ok(state) = facade.session_state(&target.chat_id).await {
        prior_state = state;
    }

    match facade.deliver_messages(&target.chat_id, pending).await {
        Ok(outcome) => DeliveryReport::Delivered {
            outcome,
            prior_state,
        },
        Err(error) => DeliveryReport::Failed { error },
    }
}

const CARD_SKIP_PREFIX: &str = "planner_delivery_skip:";

/// Append a status update only while the card still matches the target,
/// reserving it for this send. `Ok(Some(reason))` means the card moved on and
/// nothing was written.
pub(crate) async fn reserve_card_status(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    target: &CardTarget,
    status_message: String,
    require_doing: bool,
    touch_heartbeat: bool,
) -> Result<Option<String>, String> {
    let card_id = target.card_id.clone();
    let chat_id = target.chat_id.clone();
    let timestamp = Utc::now().to_rfc3339();
    let result = storage::update_board_atomic(gcx, task_id, move |board| {
        if let Some(reason) =
            stale_card_reason(board.get_card(&card_id), &card_id, &chat_id, require_doing)
        {
            return Err(format!("{}{}", CARD_SKIP_PREFIX, reason));
        }
        let card = board
            .get_card_mut(&card_id)
            .ok_or_else(|| format!("{}card {} no longer exists", CARD_SKIP_PREFIX, card_id))?;
        if touch_heartbeat {
            card.last_heartbeat_at = Some(timestamp.clone());
        }
        card.status_updates.push(StatusUpdate {
            timestamp: timestamp.clone(),
            message: status_message.clone(),
        });
        Ok(())
    })
    .await;

    match result {
        Ok((_, ())) => Ok(None),
        Err(error) => match error.strip_prefix(CARD_SKIP_PREFIX) {
            Some(reason) => Ok(Some(reason.to_string())),
            None => Err(error),
        },
    }
}

/// Append a follow-up status update (delivered / failed) for a card that is
/// still bound to the same agent chat. Silently does nothing when the card has
/// since been rebound, so late writes never land on a different agent's card.
pub(crate) async fn record_card_status(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    target: &CardTarget,
    status_message: String,
) -> Result<(), String> {
    let card_id = target.card_id.clone();
    let chat_id = target.chat_id.clone();
    let timestamp = Utc::now().to_rfc3339();
    storage::update_board_atomic(gcx, task_id, move |board| {
        let card = board
            .get_card_mut(&card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        if card.agent_chat_id.as_deref() != Some(chat_id.as_str()) {
            return Ok(());
        }
        card.status_updates.push(StatusUpdate {
            timestamp: timestamp.clone(),
            message: status_message.clone(),
        });
        Ok(())
    })
    .await
    .map(|_| ())
}

pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use refact_chat_api::ChatCommand;
    use refact_runtime_api::{
        ChatSessionSnapshot, ChatSessionUpdate, CreateSessionRequest, RuntimeTrajectorySnapshot,
    };
    use serde_json::json;
    use std::sync::Mutex;

    struct StateFacade {
        state: Result<Option<SessionState>, String>,
        probes: Mutex<Vec<String>>,
        queued: Mutex<Vec<(String, ChatCommand)>>,
    }

    impl StateFacade {
        fn new(state: Result<Option<SessionState>, String>) -> Self {
            Self {
                state,
                probes: Mutex::new(vec![]),
                queued: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl ChatSessionFacade for StateFacade {
        async fn session_snapshot(&self, _chat_id: &str) -> Result<ChatSessionSnapshot, String> {
            unreachable!()
        }

        async fn update_session(
            &self,
            _chat_id: &str,
            _update: ChatSessionUpdate,
        ) -> Result<(), String> {
            unreachable!()
        }

        async fn create_session(&self, _request: CreateSessionRequest) -> Result<(), String> {
            unreachable!()
        }

        async fn push_command(&self, chat_id: &str, command: ChatCommand) -> Result<(), String> {
            self.queued
                .lock()
                .unwrap()
                .push((chat_id.to_string(), command));
            Ok(())
        }

        async fn session_state(&self, chat_id: &str) -> Result<Option<SessionState>, String> {
            assert!(self.queued.lock().unwrap().is_empty());
            self.probes.lock().unwrap().push(chat_id.to_string());
            self.state.clone()
        }

        async fn maybe_save_session(&self, _chat_id: &str) -> Result<(), String> {
            unreachable!()
        }

        async fn save_trajectory_snapshot(
            &self,
            _snapshot: RuntimeTrajectorySnapshot,
        ) -> Result<(), String> {
            unreachable!()
        }
    }

    async fn delivery_fixture() -> (tempfile::TempDir, Arc<GlobalContext>, String, CardTarget) {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let task = storage::create_task(gcx.clone(), "Planner delivery state")
            .await
            .unwrap();
        let card = test_card("doing", Some("chat-1"));
        let target = CardTarget::from_card(&card, "chat-1".to_string());
        storage::save_board(
            gcx.clone(),
            &task.id,
            &crate::tasks::types::TaskBoard {
                cards: vec![card],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        (temp, gcx, task.id, target)
    }

    fn assert_queued(facade: &StateFacade, target: &CardTarget) {
        assert_eq!(*facade.probes.lock().unwrap(), vec![target.chat_id.clone()]);
        let queued = facade.queued.lock().unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].0, target.chat_id);
        let ChatCommand::DeliverMessages { delivery } = &queued[0].1 else {
            panic!("expected a queued delivery");
        };
        assert_eq!(delivery.push, PushMode::Append);
        assert_eq!(delivery.source, "agent_steer");
        assert!(delivery.wake);
        assert_eq!(delivery.messages.len(), 1);
        assert_eq!(
            delivery.messages[0].content.content_text_only(),
            "Steer message"
        );
    }

    #[tokio::test]
    async fn steer_receipt_reports_prior_state_without_requiring_a_live_session() {
        let (_temp, gcx, task_id, target) = delivery_fixture().await;
        let facade = Arc::new(StateFacade::new(Ok(Some(SessionState::Generating))));
        let report = deliver_to_card(
            gcx,
            facade.clone(),
            &task_id,
            &target,
            CardGuard::doing(),
            single_message_delivery(
                "Steer message".to_string(),
                PushMode::Append,
                "agent_steer",
                true,
            ),
        )
        .await;

        assert!(
            matches!(
                report,
                DeliveryReport::Delivered {
                    outcome: DeliveryOutcome::Queued,
                    prior_state: Some(SessionState::Generating),
                }
            ),
            "{}",
            report.describe("Steer")
        );
        assert!(report
            .describe("Steer")
            .contains("prior agent state: generating"));
        assert_eq!(report.receipt()["prior_state"], "generating");
        assert_queued(&facade, &target);
    }

    #[tokio::test]
    async fn steer_to_session_without_state_still_queues() {
        let (_temp, gcx, task_id, target) = delivery_fixture().await;
        for state in [Ok(None), Err("state unavailable".to_string())] {
            let facade = Arc::new(StateFacade::new(state));
            let report = deliver_to_card(
                gcx.clone(),
                facade.clone(),
                &task_id,
                &target,
                CardGuard::doing(),
                single_message_delivery(
                    "Steer message".to_string(),
                    PushMode::Append,
                    "agent_steer",
                    true,
                ),
            )
            .await;

            assert!(matches!(
                report,
                DeliveryReport::Delivered {
                    outcome: DeliveryOutcome::Queued,
                    prior_state: None,
                }
            ));
            assert!(report
                .describe("Steer")
                .contains("prior agent state: unavailable"));
            assert!(report.reached_agent());
            assert_queued(&facade, &target);
        }
    }

    #[tokio::test]
    async fn required_live_session_without_state_is_not_delivered() {
        let (_temp, gcx, task_id, target) = delivery_fixture().await;
        let facade = Arc::new(StateFacade::new(Ok(None)));
        let report = deliver_to_card(
            gcx,
            facade.clone(),
            &task_id,
            &target,
            CardGuard::doing_with_live_session(),
            single_message_delivery("Reply".to_string(), PushMode::Append, "planner_reply", true),
        )
        .await;

        assert!(matches!(report, DeliveryReport::NoLiveSession));
        assert_eq!(*facade.probes.lock().unwrap(), vec![target.chat_id]);
        assert!(facade.queued.lock().unwrap().is_empty());
    }

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn push_mode_defaults_to_append_when_absent() {
        assert_eq!(parse_push_mode(&args(&[])).unwrap(), PushMode::Append);
    }

    #[test]
    fn push_mode_null_is_treated_as_absent() {
        let args = args(&[("push", Value::Null)]);
        assert_eq!(parse_push_mode(&args).unwrap(), PushMode::Append);
    }

    #[test]
    fn push_mode_parses_all_variants_explicitly() {
        for (raw, expected) in [
            ("preempt", PushMode::Preempt),
            ("append", PushMode::Append),
            ("when_idle", PushMode::WhenIdle),
        ] {
            let args = args(&[("push", json!(raw))]);
            assert_eq!(parse_push_mode(&args).unwrap(), expected);
            assert_eq!(push_mode_label(expected), raw);
        }
    }

    fn test_card(column: &str, agent_chat_id: Option<&str>) -> BoardCard {
        BoardCard {
            id: "T-1".to_string(),
            title: "Card".to_string(),
            column: column.to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: Some("agent-1".to_string()),
            agent_chat_id: agent_chat_id.map(str::to_string),
            status_updates: vec![],
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            last_heartbeat_at: None,
            paused_at: None,
            paused_by: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
            attention: None,
            deliverable: None,
            verification: Vec::new(),
        }
    }

    #[test]
    fn stale_card_reason_reports_missing_column_and_rebound_chat() {
        let mut card = test_card("doing", Some("chat-1"));

        assert!(stale_card_reason(None, "T-1", "chat-1", true)
            .unwrap()
            .contains("no longer exists"));
        assert!(stale_card_reason(Some(&card), "T-1", "chat-1", true).is_none());

        card.column = "done".to_string();
        assert!(stale_card_reason(Some(&card), "T-1", "chat-1", true)
            .unwrap()
            .contains("now in column 'done'"));
        assert!(
            stale_card_reason(Some(&card), "T-1", "chat-1", false).is_none(),
            "column must be ignored when require_doing is false"
        );

        card.column = "doing".to_string();
        card.agent_chat_id = Some("chat-2".to_string());
        assert!(stale_card_reason(Some(&card), "T-1", "chat-1", true)
            .unwrap()
            .contains("agent_chat_id changed from 'chat-1' to 'chat-2'"));
    }

    #[test]
    fn dropped_outcomes_are_never_reported_as_reaching_the_agent() {
        for (outcome, wire) in [
            (DeliveryOutcome::Duplicate, "duplicate"),
            (DeliveryOutcome::Expired, "expired"),
            (DeliveryOutcome::Superseded, "superseded"),
            (DeliveryOutcome::Sealed, "sealed"),
        ] {
            let report = DeliveryReport::Delivered {
                outcome,
                prior_state: Some(SessionState::Idle),
            };
            assert!(!report.reached_agent(), "{wire}");
            assert!(report.short_status().starts_with("not delivered"), "{wire}");
            assert!(report.describe("Steer").contains("NOT delivered"), "{wire}");
            let receipt = report.receipt();
            assert_eq!(receipt["outcome"], wire);
            assert_eq!(receipt["reached"], false, "{wire}");
        }

        for (outcome, wire) in [
            (DeliveryOutcome::Queued, "queued"),
            (DeliveryOutcome::Delivered, "delivered"),
        ] {
            let report = DeliveryReport::Delivered {
                outcome,
                prior_state: None,
            };
            assert!(report.reached_agent(), "{wire}");
            assert!(
                !report.short_status().starts_with("not delivered"),
                "{wire}"
            );
            assert_eq!(report.receipt()["outcome"], wire);
        }
    }

    #[test]
    fn non_delivered_reports_never_claim_success() {
        let reports = [
            DeliveryReport::NoLiveSession,
            DeliveryReport::Skipped {
                reason: "card T-1 is now in column 'done'".to_string(),
            },
            DeliveryReport::Failed {
                error: "queue unavailable".to_string(),
            },
        ];
        for report in reports {
            assert!(!report.reached_agent());
            assert!(report.short_status().starts_with("not delivered"));
            assert!(report.describe("Pause").contains("NOT delivered"));
        }
    }
}
