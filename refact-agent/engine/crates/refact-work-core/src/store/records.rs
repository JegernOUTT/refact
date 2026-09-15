use crate::contracts::{WorkOrigin, WorkPhase};
use crate::identity::{CheckoutKey, ProjectKey, RunKey, WorkKey};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentState {
    Persisted,
    Dispatched,
    Acknowledged,
    Terminal,
}

impl IntentState {
    pub const ALL: [IntentState; 4] = [
        IntentState::Persisted,
        IntentState::Dispatched,
        IntentState::Acknowledged,
        IntentState::Terminal,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            IntentState::Persisted => "persisted",
            IntentState::Dispatched => "dispatched",
            IntentState::Acknowledged => "acknowledged",
            IntentState::Terminal => "terminal",
        }
    }

    pub fn parse(raw: &str) -> Option<IntentState> {
        IntentState::ALL.into_iter().find(|s| s.as_str() == raw)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Pending,
    InFlight,
    Delivered,
    Failed,
    Expired,
}

impl DeliveryState {
    pub const ALL: [DeliveryState; 5] = [
        DeliveryState::Pending,
        DeliveryState::InFlight,
        DeliveryState::Delivered,
        DeliveryState::Failed,
        DeliveryState::Expired,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            DeliveryState::Pending => "pending",
            DeliveryState::InFlight => "in_flight",
            DeliveryState::Delivered => "delivered",
            DeliveryState::Failed => "failed",
            DeliveryState::Expired => "expired",
        }
    }

    pub fn parse(raw: &str) -> Option<DeliveryState> {
        DeliveryState::ALL.into_iter().find(|s| s.as_str() == raw)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessKind {
    Read,
    Write,
}

impl AccessKind {
    pub const ALL: [AccessKind; 2] = [AccessKind::Read, AccessKind::Write];

    pub fn as_str(&self) -> &'static str {
        match self {
            AccessKind::Read => "read",
            AccessKind::Write => "write",
        }
    }

    pub fn parse(raw: &str) -> Option<AccessKind> {
        AccessKind::ALL.into_iter().find(|a| a.as_str() == raw)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineInstanceRecord {
    pub project: ProjectKey,
    pub instance_id: String,
    pub pid: i64,
    pub boot_id: String,
    pub process_started_at_ms: i64,
    pub host_id: String,
    pub heartbeat_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunIntentRecord {
    pub project: ProjectKey,
    pub run: RunKey,
    pub generation: i64,
    pub state: IntentState,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedWorkRecord {
    pub project: ProjectKey,
    pub work: WorkKey,
    pub root: CheckoutKey,
    pub origin: WorkOrigin,
    pub phase: WorkPhase,
    pub enqueued_at_ms: i64,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub project: ProjectKey,
    pub lease_key: String,
    pub owner_token: String,
    pub fencing_generation: i64,
    pub acquired_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryOutboxRecord {
    pub project: ProjectKey,
    pub message_id: String,
    pub recipient: String,
    pub state: DeliveryState,
    pub attempts: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityObservationRecord {
    pub project: ProjectKey,
    pub record_id: String,
    pub work: WorkKey,
    pub path: String,
    pub access: AccessKind,
    pub content_hash: Option<String>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruneReport {
    pub engine_instances: usize,
    pub run_intents: usize,
    pub queued_work: usize,
    pub leases: usize,
    pub delivery_outbox: usize,
    pub activity_observations: usize,
}

impl PruneReport {
    pub fn total(&self) -> usize {
        self.engine_instances
            + self.run_intents
            + self.queued_work
            + self.leases
            + self.delivery_outbox
            + self.activity_observations
    }
}
