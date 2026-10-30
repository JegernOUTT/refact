use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Mutex,
};
use std::time::Duration;
use tokio::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryMode {
    Off,
    Transient,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPolicy {
    pub mode: RecoveryMode,
    pub max_retries: u32,
    pub delay_ms: u64,
    pub total_timeout_ms: u64,
}
impl Default for RecoveryPolicy {
    fn default() -> Self {
        Self {
            mode: RecoveryMode::Transient,
            max_retries: 2,
            delay_ms: 10_000,
            total_timeout_ms: 600_000,
        }
    }
}
impl RecoveryPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_retries > 100
            || self.delay_ms > 3_600_000
            || !(1_000..=86_400_000).contains(&self.total_timeout_ms)
        {
            return Err("Recovery policy exceeds allowed bounds".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPolicyOverrides {
    pub mode: Option<RecoveryMode>,
    pub max_retries: Option<u32>,
    pub delay_ms: Option<u64>,
    pub total_timeout_ms: Option<u64>,
}
impl RecoveryPolicyOverrides {
    pub fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.max_retries.is_none()
            && self.delay_ms.is_none()
            && self.total_timeout_ms.is_none()
    }
    pub fn resolve(&self, mut inherited: RecoveryPolicy) -> Result<RecoveryPolicy, String> {
        if let Some(v) = self.mode {
            inherited.mode = v;
        }
        if let Some(v) = self.max_retries {
            inherited.max_retries = v;
        }
        if let Some(v) = self.delay_ms {
            inherited.delay_ms = v;
        }
        if let Some(v) = self.total_timeout_ms {
            inherited.total_timeout_ms = v;
        }
        inherited.validate()?;
        Ok(inherited)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryPolicySource {
    Global,
    Project,
    Thread,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryState {
    Waiting,
    Attempting,
    NeedsAction,
    Exhausted,
    Cancelled,
    Succeeded,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryError {
    pub category: String,
    pub title: String,
    pub explanation: String,
    pub suggested_action: String,
    pub is_retryable: bool,
    pub raw_error: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryAttempt {
    pub at_ms: u64,
    pub retry_number: u32,
    pub error: Option<RecoveryError>,
    pub outcome: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryRecord {
    pub operation_id: String,
    pub run_id: String,
    pub revision: u64,
    pub state: RecoveryState,
    pub policy: RecoveryPolicy,
    pub policy_source: RecoveryPolicySource,
    pub manual: bool,
    pub retries_used: u32,
    pub latest_error: RecoveryError,
    pub next_retry_at_ms: Option<u64>,
    pub deadline_at_ms: Option<u64>,
    #[serde(default)]
    pub retry_not_before_ms: Option<u64>,
    pub terminal_reason: Option<String>,
    pub attempts: Vec<RecoveryAttempt>,
}
impl RecoveryRecord {
    pub fn holds_goal(&self) -> bool {
        self.state != RecoveryState::Succeeded
    }
    pub fn interrupt_on_reload(&mut self) {
        if matches!(
            self.state,
            RecoveryState::Waiting | RecoveryState::Attempting
        ) {
            self.state = RecoveryState::NeedsAction;
            self.revision = self.revision.saturating_add(1);
            self.next_retry_at_ms = None;
            self.terminal_reason = Some("interrupted".into());
        }
        if self.policy.validate().is_err() {
            self.state = RecoveryState::NeedsAction;
            self.terminal_reason = Some("invalid_persisted_policy".into());
            self.next_retry_at_ms = None;
        }
        self.attempts.truncate(101);
    }
}

#[derive(Debug)]
pub struct RecoveryBudget {
    policy: RecoveryPolicy,
    sends: AtomicU32,
    cancelled: AtomicBool,
    deadline: Mutex<Option<Instant>>,
}
impl RecoveryBudget {
    pub fn new(policy: RecoveryPolicy) -> Self {
        Self {
            policy,
            sends: AtomicU32::new(0),
            cancelled: AtomicBool::new(false),
            deadline: Mutex::new(None),
        }
    }
    pub fn new_manual(policy: RecoveryPolicy) -> Self {
        let budget = Self::new(policy);
        budget.sends.store(1, Ordering::SeqCst);
        budget.start_recovery();
        budget
    }
    pub fn start_recovery(&self) {
        let mut deadline = self.deadline.lock().unwrap_or_else(|e| e.into_inner());
        if deadline.is_none() {
            *deadline = Some(Instant::now() + Duration::from_millis(self.policy.total_timeout_ms));
        }
    }
    pub fn deadline_remaining(&self) -> Duration {
        self.deadline
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|d| d.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::MAX)
    }
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Err("Recovery cancelled".into());
        }
        if self.deadline_remaining().is_zero() {
            return Err("Recovery deadline exhausted".into());
        }
        Ok(())
    }
    pub fn reserve_send(&self) -> Result<u32, String> {
        self.check()?;
        self.sends
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n <= self.policy.max_retries).then(|| n + 1)
            })
            .map_err(|_| "Recovery retries exhausted".into())
    }
    pub fn policy(&self) -> &RecoveryPolicy {
        &self.policy
    }
    pub fn sends_used(&self) -> u32 {
        self.sends.load(Ordering::SeqCst)
    }
    pub fn retries_used(&self) -> u32 {
        self.sends_used().saturating_sub(1)
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_batch_counts_only_new_provider_sends() {
        let policy = RecoveryPolicy {
            max_retries: 1,
            ..Default::default()
        };
        let budget = RecoveryBudget::new_manual(policy);
        assert_eq!(budget.reserve_send(), Ok(1));
        assert_eq!(budget.retries_used(), 1);
        assert_eq!(
            budget.reserve_send(),
            Err("Recovery retries exhausted".into())
        );
    }

    #[test]
    fn cancelled_budget_refuses_provider_send_without_consuming_slot() {
        let budget = RecoveryBudget::new(RecoveryPolicy::default());
        assert_eq!(budget.reserve_send(), Ok(0));
        budget.start_recovery();
        budget.cancel();
        assert_eq!(budget.reserve_send(), Err("Recovery cancelled".into()));
        assert_eq!(budget.sends_used(), 1);
    }
}
