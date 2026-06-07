use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use refact_buddy_core::conductor::{
    ConductorGoal, ConductorWakeReason, GoalBudgetSpent, GoalBudgetWakeBuckets, GoalLedger,
    GoalStatus,
};
use refact_buddy_core::conductor_store::{list_goal_ledgers, mutate_goal_ledger, MissingGoalBehavior};
use refact_chat_api::MessageOrigin;

use crate::global_context::SharedGlobalContext;

use super::budget::aggregate_goal_spent;

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
        let status = ledger.status.unwrap_or_default();
        if status.is_terminal() || status == GoalStatus::Paused {
            return;
        }
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

pub async fn refresh_conductor_wake_targets(gcx: SharedGlobalContext) -> ConductorWakeTargets {
    let Some(project_root) = conductor_project_root(gcx.clone()).await else {
        return install_conductor_wake_targets(gcx, ConductorWakeTargets::default()).await;
    };
    refresh_conductor_wake_targets_for_project(gcx, &project_root).await
}

pub async fn refresh_conductor_wake_targets_for_project(
    gcx: SharedGlobalContext,
    project_root: &Path,
) -> ConductorWakeTargets {
    let ledgers = match list_goal_ledgers(project_root).await {
        Ok(ledgers) => ledgers,
        Err(error) => {
            tracing::warn!(
                "conductor wake target refresh failed for {}: {}",
                project_root.display(),
                error
            );
            Vec::new()
        }
    };
    let targets = ConductorWakeTargets::from_goal_ledgers(
        ledgers
            .into_iter()
            .map(|stored| (stored.goal_id, stored.ledger)),
    );
    install_conductor_wake_targets(gcx, targets).await
}

pub async fn enqueue_task_wake(
    gcx: SharedGlobalContext,
    task_id: &str,
    reason: ConductorWakeReason,
) -> bool {
    let targets = active_or_refreshed_targets(gcx.clone()).await;
    enqueue_target_goals(gcx, targets.goals_for_task(task_id), reason).await
}

pub async fn enqueue_chat_wake(
    gcx: SharedGlobalContext,
    chat_id: &str,
    reason: ConductorWakeReason,
) -> bool {
    enqueue_chat_or_task_wake(gcx, chat_id, None, reason).await
}

pub async fn enqueue_chat_or_task_wake(
    gcx: SharedGlobalContext,
    chat_id: &str,
    task_id: Option<&str>,
    reason: ConductorWakeReason,
) -> bool {
    let targets = active_or_refreshed_targets(gcx.clone()).await;
    let mut goal_ids = targets.goals_for_chat(chat_id);
    if let Some(task_id) = task_id {
        for goal_id in targets.goals_for_task(task_id) {
            push_unique(&mut goal_ids, goal_id);
        }
    }
    enqueue_target_goals(gcx, goal_ids, reason).await
}

pub async fn enqueue_all_wake(gcx: SharedGlobalContext, reason: ConductorWakeReason) -> bool {
    let targets = active_or_refreshed_targets(gcx.clone()).await;
    enqueue_target_goals(gcx, targets.goal_ids(), reason).await
}

pub async fn enqueue_budget_threshold_wakes_for_project(
    gcx: SharedGlobalContext,
    project_root: &Path,
) -> bool {
    let ledgers = match list_goal_ledgers(project_root).await {
        Ok(ledgers) => ledgers,
        Err(error) => {
            tracing::warn!(
                "conductor budget wake refresh failed for {}: {}",
                project_root.display(),
                error
            );
            return false;
        }
    };
    let targets = ConductorWakeTargets::from_goal_ledgers(
        ledgers
            .iter()
            .map(|stored| (stored.goal_id.clone(), stored.ledger.clone())),
    );
    install_conductor_wake_targets(gcx.clone(), targets).await;

    let mut enqueued = false;
    for stored in ledgers {
        if ledger_is_inactive(&stored.ledger) {
            continue;
        }
        let mut goal = ConductorGoal::from_ledger(stored.goal_id.clone(), stored.ledger.clone());
        let spent = match aggregate_goal_spent(gcx.clone(), &goal).await {
            Ok(spent) => spent,
            Err(error) => {
                tracing::warn!(
                    goal_id = %goal.id,
                    error = %error,
                    "conductor budget wake aggregation failed; enqueuing fail-closed budget wake"
                );
                if enqueue_goal_wake_after_target_refresh(
                    gcx.clone(),
                    &goal.id,
                    ConductorWakeReason::Budget,
                )
                .await
                {
                    enqueued = true;
                }
                continue;
            }
        };
        goal.spent = GoalBudgetSpent {
            no_progress_wakes: goal.ledger.no_progress_wakes,
            ..spent
        };
        let current = monotonic_budget_wake_buckets(
            &goal.ledger.budget_wake_buckets,
            &budget_wake_buckets(&goal),
        );
        if current == goal.ledger.budget_wake_buckets {
            continue;
        }
        if !enqueue_goal_wake_after_target_refresh(
            gcx.clone(),
            &goal.id,
            ConductorWakeReason::Budget,
        )
        .await
        {
            continue;
        }
        match persist_budget_wake_buckets(project_root, &goal.id, current).await {
            Ok(true) => enqueued = true,
            Ok(false) => {}
            Err(error) => tracing::warn!(
                "conductor budget wake bucket persist failed for {}: {}",
                goal.id,
                error
            ),
        }
    }
    enqueued
}

pub async fn enqueue_goal_wake(
    gcx: SharedGlobalContext,
    goal_id: &str,
    reason: ConductorWakeReason,
) -> bool {
    if !refresh_conductor_wake_targets(gcx.clone())
        .await
        .contains_goal(goal_id)
    {
        return false;
    }
    enqueue_target_goals(gcx, vec![goal_id.to_string()], reason).await
}

pub async fn enqueue_goal_wake_after_target_refresh(
    gcx: SharedGlobalContext,
    goal_id: &str,
    reason: ConductorWakeReason,
) -> bool {
    let targets = gcx.conductor_wake_targets.lock().await.clone();
    if !targets.contains_goal(goal_id) {
        return false;
    }
    enqueue_target_goals(gcx, vec![goal_id.to_string()], reason).await
}

pub async fn enqueue_goal_wake_if_idle(
    gcx: SharedGlobalContext,
    goal_id: &str,
    reason: ConductorWakeReason,
) -> bool {
    if !refresh_conductor_wake_targets(gcx.clone())
        .await
        .contains_goal(goal_id)
    {
        return false;
    }
    let now = Utc::now();
    let mut bus = gcx.conductor_wake_bus.lock().await;
    if bus
        .mailbox(goal_id)
        .map(|mailbox| mailbox.in_flight)
        .unwrap_or(false)
    {
        return false;
    }
    bus.enqueue_goal(goal_id, reason, now)
}

pub async fn record_owned_chat_message(
    gcx: SharedGlobalContext,
    chat_id: &str,
    origin: Option<MessageOrigin>,
) -> bool {
    record_owned_chat_or_task_message(gcx, chat_id, None, origin).await
}

pub async fn record_owned_chat_or_task_message(
    gcx: SharedGlobalContext,
    chat_id: &str,
    task_id: Option<&str>,
    origin: Option<MessageOrigin>,
) -> bool {
    if !message_origin_sets_human_yield(origin) {
        return false;
    }
    let targets = active_or_refreshed_targets(gcx.clone()).await;
    let mut goal_ids = targets.goals_for_chat(chat_id);
    if let Some(task_id) = task_id {
        for goal_id in targets.goals_for_task(task_id) {
            push_unique(&mut goal_ids, goal_id);
        }
    }
    if goal_ids.is_empty() {
        return false;
    }
    let now = Utc::now();
    let mut bus = gcx.conductor_wake_bus.lock().await;
    let mut recorded = false;
    for goal_id in goal_ids {
        recorded |= bus.record_owned_chat_message(goal_id, origin, now);
    }
    recorded
}

pub fn try_drain_due_conductor_wakes(
    gcx: &SharedGlobalContext,
    now: DateTime<Utc>,
) -> Vec<DueConductorWake> {
    let Ok(mut bus) = gcx.conductor_wake_bus.try_lock() else {
        return Vec::new();
    };
    bus.drain_due(now)
}

pub async fn complete_conductor_wake(gcx: &SharedGlobalContext, goal_id: &str) -> bool {
    let mut bus = gcx.conductor_wake_bus.lock().await;
    bus.complete_goal(goal_id)
}

async fn conductor_project_root(gcx: SharedGlobalContext) -> Option<PathBuf> {
    crate::files_correction::get_project_dirs(gcx)
        .await
        .into_iter()
        .next()
}

async fn active_or_refreshed_targets(gcx: SharedGlobalContext) -> ConductorWakeTargets {
    let targets = gcx.conductor_wake_targets.lock().await.clone();
    if !targets.is_empty() {
        return targets;
    }
    refresh_conductor_wake_targets(gcx).await
}

async fn install_conductor_wake_targets(
    gcx: SharedGlobalContext,
    targets: ConductorWakeTargets,
) -> ConductorWakeTargets {
    {
        let mut target_guard = gcx.conductor_wake_targets.lock().await;
        *target_guard = targets.clone();
    }
    {
        let mut bus = gcx.conductor_wake_bus.lock().await;
        bus.reconcile_targets(&targets);
    }
    targets
}

async fn enqueue_target_goals(
    gcx: SharedGlobalContext,
    goal_ids: Vec<String>,
    reason: ConductorWakeReason,
) -> bool {
    if goal_ids.is_empty() {
        return false;
    }
    let now = Utc::now();
    let mut bus = gcx.conductor_wake_bus.lock().await;
    let mut enqueued = false;
    for goal_id in goal_ids {
        enqueued |= bus.enqueue_goal(goal_id, reason, now);
    }
    enqueued
}

async fn persist_budget_wake_buckets(
    project_root: &Path,
    goal_id: &str,
    buckets: GoalBudgetWakeBuckets,
) -> Result<bool, String> {
    let (_, persisted) = mutate_goal_ledger(
        project_root,
        goal_id,
        MissingGoalBehavior::RequireExisting,
        |ledger| {
            if ledger_is_inactive(ledger) {
                return Ok(false);
            }
            let next = monotonic_budget_wake_buckets(&ledger.budget_wake_buckets, &buckets);
            if next == ledger.budget_wake_buckets {
                return Ok(false);
            }
            ledger.budget_wake_buckets = next;
            Ok(true)
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(persisted)
}

fn ledger_is_inactive(ledger: &GoalLedger) -> bool {
    let status = ledger.status.unwrap_or_default();
    status.is_terminal() || status == GoalStatus::Paused
}

fn monotonic_budget_wake_buckets(
    previous: &GoalBudgetWakeBuckets,
    current: &GoalBudgetWakeBuckets,
) -> GoalBudgetWakeBuckets {
    GoalBudgetWakeBuckets {
        wall_clock_secs: previous.wall_clock_secs.max(current.wall_clock_secs),
        no_progress_wakes: previous.no_progress_wakes.max(current.no_progress_wakes),
        total_tokens: previous.total_tokens.max(current.total_tokens),
        usd: previous.usd.max(current.usd),
    }
}

fn budget_wake_buckets(goal: &ConductorGoal) -> GoalBudgetWakeBuckets {
    GoalBudgetWakeBuckets {
        wall_clock_secs: bucket_u64(goal.spent.elapsed_secs, goal.budget.wall_clock_secs),
        no_progress_wakes: bucket_u32(goal.spent.no_progress_wakes, goal.budget.no_progress_wakes),
        total_tokens: bucket_u64(goal.spent.total_tokens, goal.budget.total_tokens),
        usd: bucket_f64(goal.spent.usd, goal.budget.usd),
    }
}

fn bucket_u32(spent: u32, limit: Option<u32>) -> u8 {
    bucket_u64(u64::from(spent), limit.map(u64::from))
}

fn bucket_u64(spent: u64, limit: Option<u64>) -> u8 {
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return 0;
    };
    bucket_percent((spent as f64 / limit as f64) * 100.0)
}

fn bucket_f64(spent: Option<f64>, limit: Option<f64>) -> u8 {
    let (Some(spent), Some(limit)) = (spent, limit) else {
        return 0;
    };
    if limit <= 0.0 {
        return 0;
    }
    bucket_percent((spent / limit) * 100.0)
}

fn bucket_percent(percent: f64) -> u8 {
    if percent >= 100.0 {
        100
    } else if percent >= 90.0 {
        90
    } else if percent >= 80.0 {
        80
    } else {
        0
    }
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

    fn test_goal(no_progress_wakes: u32, stored_bucket: u8) -> GoalLedger {
        GoalLedger {
            budget: Some(refact_buddy_core::conductor::GoalBudget {
                wall_clock_secs: Some(3600),
                no_progress_wakes: Some(10),
                total_tokens: Some(100),
                ..Default::default()
            }),
            budget_wake_buckets: GoalBudgetWakeBuckets {
                no_progress_wakes: stored_bucket,
                ..Default::default()
            },
            no_progress_wakes,
            task_ids: vec!["task-1".to_string()],
            ..Default::default()
        }
    }

    async fn save_task_meta(root: &Path, task_id: &str, goal_id: &str) {
        let task_dir = root.join(".refact/tasks").join(task_id);
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let now = Utc::now().to_rfc3339();
        let meta = crate::tasks::types::TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: task_id.to_string(),
            status: crate::tasks::types::TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 0,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 0,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
            conductor: Some(crate::tasks::types::TaskConductorLink {
                goal_id: goal_id.to_string(),
                role: crate::tasks::types::TaskConductorRole::Planner,
            }),
        };
        tokio::fs::write(
            task_dir.join("meta.yaml"),
            serde_yaml::to_string(&meta).unwrap(),
        )
        .await
        .unwrap();
    }

    async fn write_invalid_budget_trajectory(root: &Path, task_id: &str) {
        let dir = root
            .join(".refact/tasks")
            .join(task_id)
            .join("trajectories/planner");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("bad.json"), "{not-json")
            .await
            .unwrap();
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
        let targets =
            ConductorWakeTargets::from_goal_ledgers(vec![("goal-1", GoalLedger::default())]);

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

    #[tokio::test]
    async fn runtime_target_refresh_loads_ledgers_and_task_hook_enqueues() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().to_path_buf(),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "goal-1",
            &GoalLedger {
                task_ids: vec!["task-1".to_string()],
                chat_ids: vec!["chat-1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let targets = refresh_conductor_wake_targets(gcx.clone()).await;
        assert_eq!(targets.goals_for_task("task-1"), vec!["goal-1"]);
        assert!(enqueue_task_wake(gcx.clone(), "task-1", ConductorWakeReason::TaskBoard).await);

        let bus = gcx.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-1").unwrap();
        assert_eq!(mailbox.reasons, vec![ConductorWakeReason::TaskBoard]);
    }

    #[tokio::test]
    async fn idle_active_goal_does_not_receive_budget_wake_below_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().to_path_buf(),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "goal-idle",
            &test_goal(0, 0),
        )
        .await
        .unwrap();

        refresh_conductor_wake_targets_for_project(gcx.clone(), dir.path()).await;
        assert!(!enqueue_budget_threshold_wakes_for_project(gcx.clone(), dir.path()).await);

        let bus = gcx.conductor_wake_bus.lock().await;
        assert!(bus.mailbox("goal-idle").is_none());
    }

    #[tokio::test]
    async fn budget_threshold_crossing_enqueues_exactly_one_budget_wake() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().to_path_buf(),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "goal-budget",
            &test_goal(8, 0),
        )
        .await
        .unwrap();

        refresh_conductor_wake_targets_for_project(gcx.clone(), dir.path()).await;
        assert!(enqueue_budget_threshold_wakes_for_project(gcx.clone(), dir.path()).await);
        assert!(!enqueue_budget_threshold_wakes_for_project(gcx.clone(), dir.path()).await);

        let bus = gcx.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-budget").unwrap();
        assert_eq!(mailbox.reasons, vec![ConductorWakeReason::Budget]);
        let ledger =
            refact_buddy_core::conductor_store::load_goal_ledger(dir.path(), "goal-budget")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(ledger.budget_wake_buckets.no_progress_wakes, 80);
    }

    #[tokio::test]
    async fn budget_threshold_crossing_refreshes_targets_without_pre_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().to_path_buf(),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "goal-no-pre-refresh",
            &test_goal(8, 0),
        )
        .await
        .unwrap();

        assert!(enqueue_budget_threshold_wakes_for_project(gcx.clone(), dir.path()).await);

        let targets = gcx.conductor_wake_targets.lock().await.clone();
        assert!(targets.contains_goal("goal-no-pre-refresh"));
        let bus = gcx.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-no-pre-refresh").unwrap();
        assert_eq!(mailbox.reasons, vec![ConductorWakeReason::Budget]);
        let ledger =
            refact_buddy_core::conductor_store::load_goal_ledger(dir.path(), "goal-no-pre-refresh")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(ledger.budget_wake_buckets.no_progress_wakes, 80);
    }

    #[tokio::test]
    async fn stale_targets_do_not_persist_crossed_threshold_without_wake() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().to_path_buf(),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "stale-goal",
            &GoalLedger::default(),
        )
        .await
        .unwrap();
        refresh_conductor_wake_targets_for_project(gcx.clone(), dir.path()).await;
        refact_buddy_core::conductor_store::remove_goal_ledger(dir.path(), "stale-goal")
            .await
            .unwrap();
        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "goal-fresh-budget",
            &test_goal(8, 0),
        )
        .await
        .unwrap();

        assert!(enqueue_budget_threshold_wakes_for_project(gcx.clone(), dir.path()).await);

        assert!(gcx
            .conductor_wake_bus
            .lock()
            .await
            .mailbox("goal-fresh-budget")
            .is_some());
        let ledger =
            refact_buddy_core::conductor_store::load_goal_ledger(dir.path(), "goal-fresh-budget")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(ledger.budget_wake_buckets.no_progress_wakes, 80);
    }

    #[tokio::test]
    async fn budget_aggregation_failure_does_not_mutate_stored_buckets() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().to_path_buf(),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        save_task_meta(dir.path(), "task-bad-budget", "goal-bad-budget").await;
        write_invalid_budget_trajectory(dir.path(), "task-bad-budget").await;
        let mut ledger = test_goal(8, 0);
        ledger.task_ids = vec!["task-bad-budget".to_string()];
        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "goal-bad-budget",
            &ledger,
        )
        .await
        .unwrap();

        assert!(enqueue_budget_threshold_wakes_for_project(gcx.clone(), dir.path()).await);

        let bus = gcx.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-bad-budget").unwrap();
        assert_eq!(mailbox.reasons, vec![ConductorWakeReason::Budget]);
        drop(bus);
        let ledger =
            refact_buddy_core::conductor_store::load_goal_ledger(dir.path(), "goal-bad-budget")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(ledger.budget_wake_buckets, GoalBudgetWakeBuckets::default());
    }

    #[tokio::test]
    async fn budget_wake_bucket_persistence_is_monotonic() {
        let dir = tempfile::tempdir().unwrap();
        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "goal-monotonic",
            &GoalLedger {
                budget_wake_buckets: GoalBudgetWakeBuckets {
                    wall_clock_secs: 90,
                    no_progress_wakes: 80,
                    total_tokens: 0,
                    usd: 100,
                },
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert!(persist_budget_wake_buckets(
            dir.path(),
            "goal-monotonic",
            GoalBudgetWakeBuckets {
                wall_clock_secs: 80,
                no_progress_wakes: 100,
                total_tokens: 90,
                usd: 0,
            },
        )
        .await
        .unwrap());

        let ledger =
            refact_buddy_core::conductor_store::load_goal_ledger(dir.path(), "goal-monotonic")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            ledger.budget_wake_buckets,
            GoalBudgetWakeBuckets {
                wall_clock_secs: 90,
                no_progress_wakes: 100,
                total_tokens: 90,
                usd: 100,
            }
        );
    }

    #[tokio::test]
    async fn budget_bucket_persistence_skips_paused_and_terminal_ledgers() {
        let dir = tempfile::tempdir().unwrap();
        for (goal_id, status) in [
            ("goal-paused", GoalStatus::Paused),
            ("goal-done", GoalStatus::Done),
        ] {
            refact_buddy_core::conductor_store::save_goal_ledger(
                dir.path(),
                goal_id,
                &GoalLedger {
                    status: Some(status),
                    budget_wake_buckets: GoalBudgetWakeBuckets {
                        no_progress_wakes: 80,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await
            .unwrap();

            assert!(!persist_budget_wake_buckets(
                dir.path(),
                goal_id,
                GoalBudgetWakeBuckets {
                    no_progress_wakes: 100,
                    ..Default::default()
                },
            )
            .await
            .unwrap());

            let ledger = refact_buddy_core::conductor_store::load_goal_ledger(dir.path(), goal_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(ledger.budget_wake_buckets.no_progress_wakes, 80);
        }
    }

    #[tokio::test]
    async fn restart_does_not_reenqueue_unchanged_budget_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().to_path_buf(),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "goal-restart",
            &test_goal(8, 80),
        )
        .await
        .unwrap();

        refresh_conductor_wake_targets_for_project(gcx.clone(), dir.path()).await;
        assert!(!enqueue_budget_threshold_wakes_for_project(gcx.clone(), dir.path()).await);

        let bus = gcx.conductor_wake_bus.lock().await;
        assert!(bus.mailbox("goal-restart").is_none());
    }

    #[tokio::test]
    async fn direct_goal_wake_requires_active_target_unless_target_refresh_bypass_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().to_path_buf(),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "goal-terminal",
            &GoalLedger {
                status: Some(GoalStatus::Done),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert!(
            !enqueue_goal_wake_if_idle(gcx.clone(), "goal-terminal", ConductorWakeReason::Manual)
                .await
        );
        assert!(gcx
            .conductor_wake_bus
            .lock()
            .await
            .mailbox("goal-terminal")
            .is_none());

        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "goal-created",
            &GoalLedger::default(),
        )
        .await
        .unwrap();
        refresh_conductor_wake_targets(gcx.clone()).await;
        assert!(
            enqueue_goal_wake_after_target_refresh(
                gcx.clone(),
                "goal-created",
                ConductorWakeReason::GoalCreated,
            )
            .await
        );
        let bus = gcx.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-created").unwrap();
        assert_eq!(mailbox.reasons, vec![ConductorWakeReason::GoalCreated]);
    }

    #[test]
    fn wake_targets_exclude_terminal_and_paused_goals() {
        let targets = ConductorWakeTargets::from_goal_ledgers(vec![
            (
                "goal-running",
                GoalLedger {
                    status: Some(GoalStatus::Active),
                    ..Default::default()
                },
            ),
            (
                "goal-paused",
                GoalLedger {
                    status: Some(GoalStatus::Paused),
                    ..Default::default()
                },
            ),
            (
                "goal-done",
                GoalLedger {
                    status: Some(GoalStatus::Done),
                    ..Default::default()
                },
            ),
        ]);

        assert_eq!(targets.goal_ids(), vec!["goal-running".to_string()]);
        assert!(!targets.contains_goal("goal-paused"));
        assert!(!targets.contains_goal("goal-done"));
    }

    #[tokio::test]
    async fn chat_lifecycle_hook_falls_back_to_task_target() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().to_path_buf(),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "goal-1",
            &GoalLedger {
                task_ids: vec!["task-1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        refresh_conductor_wake_targets(gcx.clone()).await;
        assert!(
            enqueue_chat_or_task_wake(
                gcx.clone(),
                "unregistered-chat",
                Some("task-1"),
                ConductorWakeReason::ChatLifecycle,
            )
            .await
        );

        let bus = gcx.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-1").unwrap();
        assert_eq!(mailbox.reasons, vec![ConductorWakeReason::ChatLifecycle]);
    }

    #[tokio::test]
    async fn human_chat_hook_yields_owned_task_goal_but_conductor_origin_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().to_path_buf(),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        refact_buddy_core::conductor_store::save_goal_ledger(
            dir.path(),
            "goal-1",
            &GoalLedger {
                task_ids: vec!["task-1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        refresh_conductor_wake_targets(gcx.clone()).await;
        assert!(
            record_owned_chat_or_task_message(
                gcx.clone(),
                "agent-chat-1",
                Some("task-1"),
                Some(MessageOrigin::Human),
            )
            .await
        );
        assert!(
            !record_owned_chat_or_task_message(
                gcx.clone(),
                "agent-chat-1",
                Some("task-1"),
                Some(MessageOrigin::Conductor),
            )
            .await
        );

        let bus = gcx.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-1").unwrap();
        assert_eq!(mailbox.reasons, vec![ConductorWakeReason::HumanSteering]);
        assert!(mailbox.yield_until.is_some());
    }

    #[tokio::test]
    async fn complete_conductor_wake_waits_for_locked_bus_and_clears_in_flight() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let now = ts(750);
        {
            let mut bus = gcx.conductor_wake_bus.lock().await;
            bus.enqueue_goal("goal-1", ConductorWakeReason::Manual, now);
            let due = bus.drain_due(now + Duration::seconds(5));
            assert_eq!(due.len(), 1);
            assert!(bus.mailbox("goal-1").unwrap().in_flight);
        }

        let guard = gcx.conductor_wake_bus.lock().await;
        let gcx_for_completion = gcx.clone();
        let completion =
            tokio::spawn(
                async move { complete_conductor_wake(&gcx_for_completion, "goal-1").await },
            );

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), async {
                while !completion.is_finished() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .is_err()
        );
        assert!(guard.mailbox("goal-1").unwrap().in_flight);
        drop(guard);

        assert!(completion.await.unwrap());
        {
            let mut bus = gcx.conductor_wake_bus.lock().await;
            assert!(!bus.mailbox("goal-1").unwrap().in_flight);
            bus.enqueue_goal(
                "goal-1",
                ConductorWakeReason::TaskBoard,
                now + Duration::seconds(6),
            );
            let due = bus.drain_due(now + Duration::seconds(11));
            assert_eq!(due.len(), 1);
            assert_eq!(due[0].goal_id, "goal-1");
            assert_eq!(due[0].reasons, vec![ConductorWakeReason::TaskBoard]);
        }
    }

    #[tokio::test]
    async fn try_drain_due_returns_empty_when_bus_is_locked() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let _guard = gcx.conductor_wake_bus.try_lock().unwrap();

        let due = try_drain_due_conductor_wakes(&gcx, ts(800));

        assert!(due.is_empty());
    }
}
