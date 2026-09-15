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
    /// Persisted and never dispatched: redelivering it repeats a transport
    /// attempt that provably never reached the recipient.
    Pending,
    /// Dispatched with no outcome recorded afterwards. The external effect is
    /// unknown, so it is surfaced for inspection instead of being replayed.
    InFlight,
    Delivered,
    Consumed,
    Failed,
    Expired,
    Superseded,
    Cancelled,
}

impl DeliveryState {
    pub const ALL: [DeliveryState; 8] = [
        DeliveryState::Pending,
        DeliveryState::InFlight,
        DeliveryState::Delivered,
        DeliveryState::Consumed,
        DeliveryState::Failed,
        DeliveryState::Expired,
        DeliveryState::Superseded,
        DeliveryState::Cancelled,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            DeliveryState::Pending => "pending",
            DeliveryState::InFlight => "in_flight",
            DeliveryState::Delivered => "delivered",
            DeliveryState::Consumed => "consumed",
            DeliveryState::Failed => "failed",
            DeliveryState::Expired => "expired",
            DeliveryState::Superseded => "superseded",
            DeliveryState::Cancelled => "cancelled",
        }
    }

    pub fn parse(raw: &str) -> Option<DeliveryState> {
        DeliveryState::ALL.into_iter().find(|s| s.as_str() == raw)
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self, DeliveryState::Pending | DeliveryState::InFlight)
    }

    pub fn is_unknown_outcome(&self) -> bool {
        matches!(self, DeliveryState::InFlight)
    }

    pub fn is_redeliverable(&self) -> bool {
        matches!(self, DeliveryState::Pending)
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

/// What an engine instance declares about itself in its own record.
///
/// `Idle` means the engine advertises no running work; `Closed` is written by a
/// clean shutdown so a reader never has to guess whether the process crashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceMark {
    Active,
    Idle,
    Closed,
}

impl PresenceMark {
    pub const ALL: [PresenceMark; 3] = [
        PresenceMark::Active,
        PresenceMark::Idle,
        PresenceMark::Closed,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            PresenceMark::Active => "active",
            PresenceMark::Idle => "idle",
            PresenceMark::Closed => "closed",
        }
    }

    pub fn parse(raw: &str) -> Option<PresenceMark> {
        PresenceMark::ALL.into_iter().find(|m| m.as_str() == raw)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineInstanceRecord {
    pub project: ProjectKey,
    pub instance_id: String,
    pub pid: i64,
    pub boot_id: String,
    /// Milliseconds since boot at which this PID started; `-1` when the
    /// platform could not tell us.
    pub process_started_at_ms: i64,
    pub host_id: String,
    pub heartbeat_at_ms: i64,
    pub presence_mark: PresenceMark,
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
    /// Engine instance behind `owner_token`, so a stale holder can be looked up
    /// in engine presence instead of being guessed at.
    pub holder_instance: String,
    pub owner_token: String,
    pub fencing_generation: i64,
    pub acquired_at_ms: i64,
    pub expires_at_ms: i64,
}

impl LeaseRecord {
    pub fn is_expired(&self, at_ms: i64) -> bool {
        at_ms >= self.expires_at_ms
    }

    pub fn authority(&self) -> LeaseAuthority {
        LeaseAuthority {
            lease_key: self.lease_key.clone(),
            owner_token: self.owner_token.clone(),
            fencing_generation: self.fencing_generation,
        }
    }
}

/// What a writer presents to prove it still owns the work it is writing about.
/// A write carrying a generation the lease no longer has is rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseAuthority {
    pub lease_key: String,
    pub owner_token: String,
    pub fencing_generation: i64,
}

/// The outcome of an atomic acquisition attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseAcquisition {
    Acquired(LeaseRecord),
    /// Someone else's row is in the way. Expiry alone does not authorize taking
    /// it: that is an explicit, presence-checked takeover.
    Held(LeaseRecord),
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
    /// Logical file identity: the first canonical path observed for this
    /// physical file, so every spelling and hard link collapses onto one key.
    pub file_key: String,
    /// Physical file identity when the platform exposes one.
    pub file_alias: Option<String>,
    pub instance_id: String,
    pub tool_call: Option<String>,
    pub content_hash_before: Option<String>,
    pub checkout: Option<String>,
}

/// What one finished run actually cost, and whether it ran alone.
///
/// `sibling_count` counts this run plus the runs that overlapped it under the
/// same parent: `1` is a single agent, anything above that is fan-out. Nothing
/// here is an estimate — every field is measured or left at what the runner
/// reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecompositionOutcomeRecord {
    pub project: ProjectKey,
    pub record_id: String,
    pub work: WorkKey,
    /// What kind of work this was, so only comparable runs are compared.
    pub work_class: String,
    pub fan_out_group: Option<String>,
    pub sibling_count: i64,
    pub wall_clock_ms: i64,
    pub tokens: i64,
    pub turns: i64,
    pub tool_calls: i64,
    pub succeeded: bool,
    /// The work had to be run again rather than landing on the first attempt.
    pub reworked: bool,
    pub recorded_at_ms: i64,
    /// Paths this run declared, kept so a model-facing surface can be filtered
    /// against the destination privacy policy.
    pub paths: Vec<String>,
}

impl DecompositionOutcomeRecord {
    pub fn fanned_out(&self) -> bool {
        self.sibling_count > 1
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruneReport {
    pub engine_instances: usize,
    pub run_intents: usize,
    pub queued_work: usize,
    pub leases: usize,
    pub delivery_outbox: usize,
    pub activity_observations: usize,
    pub decomposition_outcomes: usize,
}

impl PruneReport {
    pub fn total(&self) -> usize {
        self.engine_instances
            + self.run_intents
            + self.queued_work
            + self.leases
            + self.delivery_outbox
            + self.activity_observations
            + self.decomposition_outcomes
    }
}
