use crate::contracts::{WorkOrigin, WorkPhase};
use crate::identity::{CheckoutKey, ProjectKey, RunKey, WorkKey};
use serde::{Deserialize, Serialize};
use std::fmt;
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

/// Where a disputed disagreement stands.
///
/// `Open` and `Stale` are deliberately distinct from `Adjudicated`: an open
/// record means nothing decisive was presented, a stale one means the ground
/// the arguments stood on moved. Neither is a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisagreementStatus {
    Open,
    Adjudicated,
    Stale,
    Withdrawn,
}

impl DisagreementStatus {
    pub const ALL: [DisagreementStatus; 4] = [
        DisagreementStatus::Open,
        DisagreementStatus::Adjudicated,
        DisagreementStatus::Stale,
        DisagreementStatus::Withdrawn,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            DisagreementStatus::Open => "open",
            DisagreementStatus::Adjudicated => "adjudicated",
            DisagreementStatus::Stale => "stale",
            DisagreementStatus::Withdrawn => "withdrawn",
        }
    }

    pub fn parse(raw: &str) -> Option<DisagreementStatus> {
        DisagreementStatus::ALL
            .into_iter()
            .find(|s| s.as_str() == raw)
    }

    pub fn is_open(&self) -> bool {
        matches!(self, DisagreementStatus::Open)
    }
}

/// What kind of thing a piece of evidence points at. `Evidence` is an id in the
/// evidence ledger; every other variant is a typed source reference for a
/// position argued where no ledger entry exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Evidence,
    File,
    Command,
    TestRun,
    Document,
    Trajectory,
}

impl EvidenceKind {
    pub const ALL: [EvidenceKind; 6] = [
        EvidenceKind::Evidence,
        EvidenceKind::File,
        EvidenceKind::Command,
        EvidenceKind::TestRun,
        EvidenceKind::Document,
        EvidenceKind::Trajectory,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceKind::Evidence => "evidence",
            EvidenceKind::File => "file",
            EvidenceKind::Command => "command",
            EvidenceKind::TestRun => "test_run",
            EvidenceKind::Document => "document",
            EvidenceKind::Trajectory => "trajectory",
        }
    }

    pub fn parse(raw: &str) -> Option<EvidenceKind> {
        EvidenceKind::ALL.into_iter().find(|k| k.as_str() == raw)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub kind: EvidenceKind,
    pub reference: String,
}

impl EvidenceRef {
    pub fn new(kind: EvidenceKind, reference: impl Into<String>) -> Self {
        EvidenceRef {
            kind,
            reference: reference.into(),
        }
    }
}

impl fmt::Display for EvidenceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind.as_str(), self.reference)
    }
}

/// One side of a disagreement, as its author argued it.
///
/// `subject_revision` is the revision of the disputed subject this side read.
/// Two sides arguing different revisions are not disagreeing about the same
/// thing, which is what makes stale detection possible at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisagreementPosition {
    pub position_id: String,
    pub author: WorkKey,
    pub run_generation: i64,
    pub statement: String,
    pub evidence: Vec<EvidenceRef>,
    pub asserted_at_ms: i64,
    pub subject_revision: String,
}

impl DisagreementPosition {
    pub fn cites(&self, evidence: &EvidenceRef) -> bool {
        self.evidence.contains(evidence)
    }
}

/// How an adjudicated disagreement was settled. It names the evidence that
/// decided it, never the author or the write order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adjudication {
    pub adjudicator: String,
    pub resolved_position: String,
    pub decisive_evidence: EvidenceRef,
    pub rationale: String,
    pub decided_at_ms: i64,
}

/// What an adjudicator concluded. `Unresolvable` is a legal outcome, not a
/// failure: without decisive evidence there is no winner to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdjudicationVerdict {
    Decided {
        position_id: String,
        decisive_evidence: EvidenceRef,
        rationale: String,
    },
    Unresolvable {
        reason: String,
    },
}

/// Why an adjudication carries no weight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdjudicationRefusal {
    NotOpen(DisagreementStatus),
    /// The subject moved under the arguments. The record is now stale and has
    /// to be re-argued rather than settled on what the sides used to read.
    Stale {
        asserted_against: String,
        current: String,
    },
    UnknownPosition(String),
    /// The cited evidence is not among what the winning side actually
    /// presented, so the decision rests on something unreadable.
    EvidenceNotPresented(String),
    /// An unresolvable outcome without a stated reason is a silent dismissal.
    MissingReason,
}

impl fmt::Display for AdjudicationRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdjudicationRefusal::NotOpen(status) => write!(
                f,
                "this disagreement is {} and is not open for adjudication",
                status.as_str()
            ),
            AdjudicationRefusal::Stale {
                asserted_against,
                current,
            } => write!(
                f,
                "the subject moved from `{asserted_against}` to `{current}` after the positions \
                 were asserted; the disagreement is stale and must be re-argued"
            ),
            AdjudicationRefusal::UnknownPosition(id) => {
                write!(f, "no position `{id}` was filed on this disagreement")
            }
            AdjudicationRefusal::EvidenceNotPresented(id) => write!(
                f,
                "position `{id}` never presented the evidence cited as decisive"
            ),
            AdjudicationRefusal::MissingReason => {
                write!(f, "an unresolvable disagreement needs a stated reason")
            }
        }
    }
}

/// Why a disagreement could not be filed at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilingRefusal {
    EmptyClaim,
    /// One position is an assertion, not a disagreement.
    NotTwoSided(usize),
    DuplicatePosition(String),
    EmptyPositionId,
}

impl fmt::Display for FilingRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilingRefusal::EmptyClaim => write!(f, "a disagreement needs the claim in dispute"),
            FilingRefusal::NotTwoSided(count) => write!(
                f,
                "a disagreement needs at least two positions, {count} given"
            ),
            FilingRefusal::DuplicatePosition(id) => {
                write!(f, "position id `{id}` is used twice")
            }
            FilingRefusal::EmptyPositionId => write!(f, "every position needs an id"),
        }
    }
}

/// A recorded disagreement: the claim, every position argued on it, and how it
/// was settled if it was.
///
/// Positions are append-only by construction — nothing here removes or rewrites
/// one — so the rejected side of an adjudicated record stays readable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisagreementRecord {
    pub project: ProjectKey,
    pub record_id: String,
    pub claim: String,
    /// What the claim is about: a path, a spec id, a document reference.
    pub subject: String,
    pub positions: Vec<DisagreementPosition>,
    pub status: DisagreementStatus,
    pub adjudication: Option<Adjudication>,
    /// Why this record is still open when nothing decisive was presented.
    pub open_reason: Option<String>,
    pub filed_at_ms: i64,
    pub updated_at_ms: i64,
}

impl DisagreementRecord {
    pub fn validate(&self) -> Result<(), FilingRefusal> {
        if self.claim.trim().is_empty() {
            return Err(FilingRefusal::EmptyClaim);
        }
        if self.positions.len() < 2 {
            return Err(FilingRefusal::NotTwoSided(self.positions.len()));
        }
        let mut seen: Vec<&str> = Vec::with_capacity(self.positions.len());
        for position in &self.positions {
            if position.position_id.trim().is_empty() {
                return Err(FilingRefusal::EmptyPositionId);
            }
            if seen.contains(&position.position_id.as_str()) {
                return Err(FilingRefusal::DuplicatePosition(
                    position.position_id.clone(),
                ));
            }
            seen.push(&position.position_id);
        }
        Ok(())
    }

    pub fn position(&self, position_id: &str) -> Option<&DisagreementPosition> {
        self.positions
            .iter()
            .find(|position| position.position_id == position_id)
    }

    /// Positions an adjudication did not resolve to. They are kept, never
    /// deleted, so a rejected argument can still be read back.
    pub fn rejected_positions(&self) -> Vec<&DisagreementPosition> {
        let Some(adjudication) = &self.adjudication else {
            return Vec::new();
        };
        self.positions
            .iter()
            .filter(|position| position.position_id != adjudication.resolved_position)
            .collect()
    }

    /// The revision every side argued against, when they agree on one.
    pub fn asserted_revision(&self) -> Option<&str> {
        let first = self.positions.first()?;
        self.positions
            .iter()
            .all(|position| position.subject_revision == first.subject_revision)
            .then_some(first.subject_revision.as_str())
    }

    pub fn is_stale_against(&self, current_revision: &str) -> bool {
        self.positions
            .iter()
            .any(|position| position.subject_revision != current_revision)
    }

    pub fn withdraw(&mut self, at_ms: i64) {
        self.status = DisagreementStatus::Withdrawn;
        self.updated_at_ms = at_ms;
    }

    /// Settle this disagreement on the evidence presented.
    ///
    /// A verdict naming evidence the winning side never presented is refused; a
    /// subject that moved makes the record stale instead of adjudicated; and an
    /// unresolvable verdict leaves it open with its reason. Nothing here
    /// touches a position, so the losing argument survives the decision.
    pub fn resolve(
        &mut self,
        adjudicator: &str,
        verdict: AdjudicationVerdict,
        current_revision: &str,
        at_ms: i64,
    ) -> Result<DisagreementStatus, AdjudicationRefusal> {
        if !self.status.is_open() {
            return Err(AdjudicationRefusal::NotOpen(self.status));
        }
        if self.is_stale_against(current_revision) {
            let asserted_against = self
                .asserted_revision()
                .unwrap_or("differing revisions")
                .to_string();
            self.status = DisagreementStatus::Stale;
            self.updated_at_ms = at_ms;
            return Err(AdjudicationRefusal::Stale {
                asserted_against,
                current: current_revision.to_string(),
            });
        }
        match verdict {
            AdjudicationVerdict::Unresolvable { reason } => {
                let reason = reason.trim().to_string();
                if reason.is_empty() {
                    return Err(AdjudicationRefusal::MissingReason);
                }
                self.open_reason = Some(reason);
                self.updated_at_ms = at_ms;
                Ok(DisagreementStatus::Open)
            }
            AdjudicationVerdict::Decided {
                position_id,
                decisive_evidence,
                rationale,
            } => {
                let position = self
                    .position(&position_id)
                    .ok_or_else(|| AdjudicationRefusal::UnknownPosition(position_id.clone()))?;
                if !position.cites(&decisive_evidence) {
                    return Err(AdjudicationRefusal::EvidenceNotPresented(position_id));
                }
                self.adjudication = Some(Adjudication {
                    adjudicator: adjudicator.to_string(),
                    resolved_position: position_id,
                    decisive_evidence,
                    rationale,
                    decided_at_ms: at_ms,
                });
                self.open_reason = None;
                self.status = DisagreementStatus::Adjudicated;
                self.updated_at_ms = at_ms;
                Ok(DisagreementStatus::Adjudicated)
            }
        }
    }
}

/// Which boundary produced a causal event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CausalKind {
    Delivery,
    Spawn,
    Completion,
    Question,
    Evidence,
    Decision,
}

impl CausalKind {
    pub const ALL: [CausalKind; 6] = [
        CausalKind::Delivery,
        CausalKind::Spawn,
        CausalKind::Completion,
        CausalKind::Question,
        CausalKind::Evidence,
        CausalKind::Decision,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            CausalKind::Delivery => "delivery",
            CausalKind::Spawn => "spawn",
            CausalKind::Completion => "completion",
            CausalKind::Question => "question",
            CausalKind::Evidence => "evidence",
            CausalKind::Decision => "decision",
        }
    }

    pub fn parse(raw: &str) -> Option<CausalKind> {
        CausalKind::ALL.into_iter().find(|k| k.as_str() == raw)
    }
}

/// One cause of one event. `Unknown` is what a producer records when it does
/// not know what caused the event; a parent is never guessed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CausalCause {
    Event(String),
    Unknown,
}

impl CausalCause {
    pub fn event_id(&self) -> Option<&str> {
        match self {
            CausalCause::Event(id) => Some(id.as_str()),
            CausalCause::Unknown => None,
        }
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, CausalCause::Unknown)
    }
}

/// One node of the causal DAG: what happened, where, and which events caused
/// it. Causality is a DAG — an event may carry several causes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CausalEventRecord {
    pub project: ProjectKey,
    pub event_id: String,
    /// Monotonic within one recorder, so events written in the same
    /// millisecond still have a total order.
    pub seq: i64,
    pub kind: CausalKind,
    pub work: WorkKey,
    /// Chat or agent the event happened in.
    pub scope: String,
    /// Empty on a record written before causality existed, which reads as a
    /// single unknown cause rather than as an uncaused event.
    #[serde(default)]
    pub causes: Vec<CausalCause>,
    pub recorded_at_ms: i64,
    /// Paths this event touched, kept only so a model-facing surface can be
    /// filtered against the destination privacy policy.
    #[serde(default)]
    pub paths: Vec<String>,
}

impl CausalEventRecord {
    pub fn causes_or_unknown(&self) -> Vec<CausalCause> {
        if self.causes.is_empty() {
            vec![CausalCause::Unknown]
        } else {
            self.causes.clone()
        }
    }

    pub fn has_unknown_cause(&self) -> bool {
        self.causes.is_empty() || self.causes.iter().any(CausalCause::is_unknown)
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
    pub disagreements: usize,
    pub causal_events: usize,
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
            + self.disagreements
            + self.causal_events
    }
}
