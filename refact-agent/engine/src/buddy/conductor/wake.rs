use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use refact_buddy_core::conductor::{ConductorWakeReason, GoalLedger};
use refact_chat_api::MessageOrigin;

pub const DEFAULT_WAKE_DEBOUNCE_SECS: i64 = 5;
pub const DEFAULT_HUMAN_YIELD_GRACE_SECS: i64 = 30;
pub const DEFAULT_GLOBAL_IN_FLIGHT_LIMIT: usize = 2;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConductorWakeTargets {
    goal_ids: Vec<String>,
    task_to_goals: HashMap<String, Vec<String>>,
    chat_to_goals: HashMap<String, Vec<String>>,
}

impl ConductorWakeTargets {
    pub fn from_goal_ledgers<I, S>(ledgers: I) -> Self
    where
        I: IntoIterator<Item = (S, GoalLedger)>,
        S: Into<String>,
    {
        let mut targets = Self::default();
        for (goal_id, ledger) in ledgers {
            targets.register_goal_ledger(goal_id, &ledger);
        }
        targets
    }

    pub fn register_goal_ledger(&mut self, goal_id: impl Into<String>, ledger: &GoalLedger) {
        let goal_id = normalized_goal_id(goal_id.into());
        if goal_id.is_empty() {
            return;
        }
        push_unique(&mut self.goal_ids, goal_id.clone());
        if let Some(task_id) = ledger.planner_task_id.as_deref() {
            self.register_task_goal(task_id, &goal_id);
        }
        for task_id in &ledger.task_ids {
            self.register_task_goal(task_id, &goal_id);
        }
        for chat_id in &ledger.chat_ids {
            self.register_chat_goal(chat_id, &goal_id);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.goal_ids.is_empty()
    }

    pub fn contains_goal(&self, goal_id: &str) -> bool {
        let goal_id = normalized_goal_id(goal_id.to_string());
        !goal_id.is_empty() && self.goal_ids.contains(&goal_id)
    }

    pub fn goal_ids(&self) -> Vec<String> {
        self.goal_ids.clone()
    }

    pub fn goals_for_task(&self, task_id: &str) -> Vec<String> {
        normalized_lookup(&self.task_to_goals, task_id)
    }

    pub fn goals_for_chat(&self, chat_id: &str) -> Vec<String> {
        normalized_lookup(&self.chat_to_goals, chat_id)
    }

    fn register_task_goal(&mut self, task_id: &str, goal_id: &str) {
        let task_id = normalized_goal_id(task_id.to_string());
        if task_id.is_empty() {
            return;
        }
        push_unique(
            self.task_to_goals.entry(task_id).or_default(),
            goal_id.to_string(),
        );
    }

    fn register_chat_goal(&mut self, chat_id: &str, goal_id: &str) {
        let chat_id = normalized_goal_id(chat_id.to_string());
        if chat_id.is_empty() {
            return;
        }
        push_unique(
            self.chat_to_goals.entry(chat_id).or_default(),
            goal_id.to_string(),
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeMailbox {
    pub reasons: Vec<ConductorWakeReason>,
    pub earliest_run_at: Option<DateTime<Utc>>,
    pub in_flight: bool,
    pub yield_until: Option<DateTime<Utc>>,
}

impl Default for WakeMailbox {
    fn default() -> Self {
        Self {
            reasons: Vec::new(),
            earliest_run_at: None,
            in_flight: false,
            yield_until: None,
        }
    }
}

impl WakeMailbox {
    pub fn enqueue(&mut self, reason: ConductorWakeReason, now: DateTime<Utc>, debounce: Duration) {
        if !self.reasons.contains(&reason) {
            self.reasons.push(reason);
        }
        let next_run_at = now + debounce;
        if self
            .earliest_run_at
            .map(|existing| next_run_at < existing)
            .unwrap_or(true)
        {
            self.earliest_run_at = Some(next_run_at);
        }
    }

    pub fn mark_yield(&mut self, now: DateTime<Utc>, grace: Duration) {
        let next_yield_until = now + grace;
        if self
            .yield_until
            .map(|existing| next_yield_until > existing)
            .unwrap_or(true)
        {
            self.yield_until = Some(next_yield_until);
        }
    }

    pub fn is_yielding(&self, now: DateTime<Utc>) -> bool {
        self.yield_until
            .map(|yield_until| now < yield_until)
            .unwrap_or(false)
    }

    fn is_due(&self, now: DateTime<Utc>) -> bool {
        !self.in_flight
            && !self.reasons.is_empty()
            && !self.is_yielding(now)
            && self
                .earliest_run_at
                .map(|earliest_run_at| now >= earliest_run_at)
                .unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueConductorWake {
    pub goal_id: String,
    pub reasons: Vec<ConductorWakeReason>,
    pub due_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ConductorWakeBus {
    mailboxes: HashMap<String, WakeMailbox>,
    debounce: Duration,
    human_yield_grace: Duration,
    global_in_flight_limit: usize,
}

impl Default for ConductorWakeBus {
    fn default() -> Self {
        Self::new()
    }
}

impl ConductorWakeBus {
    pub fn new() -> Self {
        Self::with_limits(
            Duration::seconds(DEFAULT_WAKE_DEBOUNCE_SECS),
            Duration::seconds(DEFAULT_HUMAN_YIELD_GRACE_SECS),
            DEFAULT_GLOBAL_IN_FLIGHT_LIMIT,
        )
    }

    pub fn with_limits(
        debounce: Duration,
        human_yield_grace: Duration,
        global_in_flight_limit: usize,
    ) -> Self {
        Self {
            mailboxes: HashMap::new(),
            debounce,
            human_yield_grace,
            global_in_flight_limit,
        }
    }

    pub fn mailbox(&self, goal_id: &str) -> Option<&WakeMailbox> {
        self.mailboxes.get(goal_id)
    }

    pub fn has_mailboxes(&self) -> bool {
        !self.mailboxes.is_empty()
    }

    pub fn reconcile_targets(&mut self, targets: &ConductorWakeTargets) {
        self.mailboxes
            .retain(|goal_id, _| targets.contains_goal(goal_id));
    }

    pub fn enqueue_goal(
        &mut self,
        goal_id: impl Into<String>,
        reason: ConductorWakeReason,
        now: DateTime<Utc>,
    ) -> bool {
        let goal_id = normalized_goal_id(goal_id.into());
        if goal_id.is_empty() {
            return false;
        }
        self.mailboxes
            .entry(goal_id)
            .or_default()
            .enqueue(reason, now, self.debounce);
        true
    }

    pub fn record_owned_chat_message(
        &mut self,
        goal_id: impl Into<String>,
        origin: Option<MessageOrigin>,
        now: DateTime<Utc>,
    ) -> bool {
        if !message_origin_sets_human_yield(origin) {
            return false;
        }
        let goal_id = normalized_goal_id(goal_id.into());
        if goal_id.is_empty() {
            return false;
        }
        let mailbox = self.mailboxes.entry(goal_id).or_default();
        mailbox.enqueue(ConductorWakeReason::HumanSteering, now, self.debounce);
        mailbox.mark_yield(now, self.human_yield_grace);
        true
    }

    pub fn is_goal_yielding(&self, goal_id: &str, now: DateTime<Utc>) -> bool {
        self.mailboxes
            .get(goal_id)
            .map(|mailbox| mailbox.is_yielding(now))
            .unwrap_or(false)
    }

    pub fn in_flight_count(&self) -> usize {
        self.mailboxes
            .values()
            .filter(|mailbox| mailbox.in_flight)
            .count()
    }

    pub fn drain_due(&mut self, now: DateTime<Utc>) -> Vec<DueConductorWake> {
        let mut capacity = self
            .global_in_flight_limit
            .saturating_sub(self.in_flight_count());
        if capacity == 0 {
            return Vec::new();
        }

        let mut due_goal_ids = self
            .mailboxes
            .iter()
            .filter_map(|(goal_id, mailbox)| {
                mailbox
                    .is_due(now)
                    .then(|| (mailbox.earliest_run_at.unwrap(), goal_id.clone()))
            })
            .collect::<Vec<_>>();
        due_goal_ids.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));

        let mut due = Vec::new();
        for (due_at, goal_id) in due_goal_ids {
            if capacity == 0 {
                break;
            }
            let Some(mailbox) = self.mailboxes.get_mut(&goal_id) else {
                continue;
            };
            if !mailbox.is_due(now) {
                continue;
            }
            let reasons = std::mem::take(&mut mailbox.reasons);
            mailbox.earliest_run_at = None;
            mailbox.in_flight = true;
            due.push(DueConductorWake {
                goal_id,
                reasons,
                due_at,
            });
            capacity -= 1;
        }
        due
    }

    pub fn complete_goal(&mut self, goal_id: &str) -> bool {
        let Some(mailbox) = self.mailboxes.get_mut(goal_id) else {
            return false;
        };
        if !mailbox.in_flight {
            return false;
        }
        mailbox.in_flight = false;
        true
    }
}

pub fn message_origin_sets_human_yield(origin: Option<MessageOrigin>) -> bool {
    !matches!(origin, Some(MessageOrigin::Conductor))
}

fn normalized_lookup(map: &HashMap<String, Vec<String>>, key: &str) -> Vec<String> {
    let key = normalized_goal_id(key.to_string());
    if key.is_empty() {
        return Vec::new();
    }
    map.get(&key).cloned().unwrap_or_default()
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn normalized_goal_id(goal_id: String) -> String {
    goal_id.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    fn test_bus() -> ConductorWakeBus {
        ConductorWakeBus::with_limits(Duration::seconds(5), Duration::seconds(30), 2)
    }

    #[test]
    fn wake_targets_map_goal_ledgers_to_task_and_chat_goals() {
        let targets = ConductorWakeTargets::from_goal_ledgers(vec![
            (
                "goal-1",
                GoalLedger {
                    planner_task_id: Some("task-1".to_string()),
                    task_ids: vec!["task-1".to_string(), "task-2".to_string()],
                    chat_ids: vec!["chat-1".to_string()],
                    ..Default::default()
                },
            ),
            (
                "goal-2",
                GoalLedger {
                    task_ids: vec!["task-2".to_string()],
                    chat_ids: vec!["chat-1".to_string(), "chat-2".to_string()],
                    ..Default::default()
                },
            ),
        ]);

        assert_eq!(targets.goal_ids(), vec!["goal-1", "goal-2"]);
        assert_eq!(targets.goals_for_task("task-1"), vec!["goal-1"]);
        assert_eq!(targets.goals_for_task("task-2"), vec!["goal-1", "goal-2"]);
        assert_eq!(targets.goals_for_chat("chat-1"), vec!["goal-1", "goal-2"]);
        assert_eq!(targets.goals_for_chat("chat-2"), vec!["goal-2"]);
        assert!(targets.goals_for_task("missing").is_empty());
    }

    #[test]
    fn wake_bus_reconcile_targets_drops_stale_mailboxes() {
        let mut bus = test_bus();
        let now = ts(50);
        bus.enqueue_goal("goal-1", ConductorWakeReason::Manual, now);
        bus.enqueue_goal("stale-goal", ConductorWakeReason::Manual, now);
        let targets = ConductorWakeTargets::from_goal_ledgers(vec![
            ("goal-1", GoalLedger::default()),
        ]);

        bus.reconcile_targets(&targets);

        assert!(bus.mailbox("goal-1").is_some());
        assert!(bus.mailbox("stale-goal").is_none());
    }

    #[test]
    fn coalesces_reasons_and_keeps_first_due_time() {
        let mut bus = test_bus();
        let now = ts(100);

        assert!(bus.enqueue_goal("goal-1", ConductorWakeReason::TaskBoard, now));
        assert!(bus.enqueue_goal(
            "goal-1",
            ConductorWakeReason::Heartbeat,
            now + Duration::seconds(1),
        ));
        assert!(bus.enqueue_goal(
            "goal-1",
            ConductorWakeReason::TaskBoard,
            now + Duration::seconds(2),
        ));

        let mailbox = bus.mailbox("goal-1").unwrap();
        assert_eq!(
            mailbox.reasons,
            vec![
                ConductorWakeReason::TaskBoard,
                ConductorWakeReason::Heartbeat,
            ]
        );
        assert_eq!(mailbox.earliest_run_at, Some(now + Duration::seconds(5)));
        assert!(bus.drain_due(now + Duration::seconds(4)).is_empty());

        let due = bus.drain_due(now + Duration::seconds(5));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].goal_id, "goal-1");
        assert_eq!(
            due[0].reasons,
            vec![
                ConductorWakeReason::TaskBoard,
                ConductorWakeReason::Heartbeat,
            ]
        );
    }

    #[test]
    fn in_flight_goal_is_not_drained_twice() {
        let mut bus = test_bus();
        let now = ts(200);

        bus.enqueue_goal("goal-1", ConductorWakeReason::Manual, now);
        let first = bus.drain_due(now + Duration::seconds(5));
        assert_eq!(first.len(), 1);
        assert_eq!(bus.in_flight_count(), 1);

        bus.enqueue_goal(
            "goal-1",
            ConductorWakeReason::TaskBoard,
            now + Duration::seconds(6),
        );
        assert!(bus.drain_due(now + Duration::seconds(11)).is_empty());

        assert!(bus.complete_goal("goal-1"));
        let second = bus.drain_due(now + Duration::seconds(11));
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].reasons, vec![ConductorWakeReason::TaskBoard]);
    }

    #[test]
    fn global_cap_limits_due_drain() {
        let mut bus = test_bus();
        let now = ts(300);
        for goal_id in ["goal-1", "goal-2", "goal-3"] {
            bus.enqueue_goal(goal_id, ConductorWakeReason::Manual, now);
        }

        let first = bus.drain_due(now + Duration::seconds(5));
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].goal_id, "goal-1");
        assert_eq!(first[1].goal_id, "goal-2");
        assert!(bus.drain_due(now + Duration::seconds(5)).is_empty());

        assert!(bus.complete_goal("goal-1"));
        let second = bus.drain_due(now + Duration::seconds(5));
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].goal_id, "goal-3");
    }

    #[test]
    fn due_drain_waits_for_debounce() {
        let mut bus = test_bus();
        let now = ts(400);

        bus.enqueue_goal("goal-1", ConductorWakeReason::Heartbeat, now);

        assert!(bus.drain_due(now + Duration::seconds(4)).is_empty());
        assert_eq!(bus.drain_due(now + Duration::seconds(5)).len(), 1);
    }

    #[test]
    fn human_origin_message_yields_and_later_drains_human_steering() {
        let mut bus = test_bus();
        let now = ts(500);

        assert!(bus.record_owned_chat_message("goal-1", Some(MessageOrigin::Human), now));
        assert!(bus.is_goal_yielding("goal-1", now + Duration::seconds(29)));
        assert!(bus.drain_due(now + Duration::seconds(29)).is_empty());

        let due = bus.drain_due(now + Duration::seconds(30));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].reasons, vec![ConductorWakeReason::HumanSteering]);
    }

    #[test]
    fn conductor_origin_message_does_not_yield() {
        let mut bus = test_bus();
        let now = ts(600);

        assert!(!bus.record_owned_chat_message("goal-1", Some(MessageOrigin::Conductor), now));

        assert!(bus.mailbox("goal-1").is_none());
        assert!(!bus.is_goal_yielding("goal-1", now));
    }

    #[test]
    fn legacy_missing_origin_defaults_to_human_yield() {
        let mut bus = test_bus();
        let now = ts(700);

        assert!(bus.record_owned_chat_message("goal-1", None, now));

        let mailbox = bus.mailbox("goal-1").unwrap();
        assert_eq!(mailbox.reasons, vec![ConductorWakeReason::HumanSteering]);
        assert_eq!(mailbox.yield_until, Some(now + Duration::seconds(30)));
    }
}
