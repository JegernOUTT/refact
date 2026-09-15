pub mod approval;
pub mod cap_stop;
pub use cap_stop::{CapKind, CapScope, CapStop};
pub use approval::{
    ApprovalDecision, ApprovalError, ApprovalRegistry, PendingApproval, ExecutionApproval,
};
pub mod contracts;
pub mod identity;
pub mod scheduler;
pub mod settings;
pub mod store;
pub mod waitgraph;

pub use contracts::{ActivityState, WorkOrigin, WorkPhase, WorkPriority, WorkSummary, WorkerProjection};
pub use identity::{CheckoutKey, ProjectKey, RunKey, RuntimeKind, WorkKey, WorkScope};
pub use scheduler::{
    AdmissionError, AdmissionRequest, AdmissionScheduler, AdmissionSchedulerHandle, Permit,
    SchedulerSnapshot, Ticket, TicketId, TicketSnapshot, TicketState,
};
pub use settings::{
    coordination_settings, install_coordination_settings, CoordinationSettings, SettingsError,
    SettingsViolation,
};
pub use store::filesystem::{classify_mounts, classify_path, FilesystemKind};
pub use store::records::{
    now_ms, AccessKind, ActivityObservationRecord, Adjudication, AdjudicationRefusal,
    AdjudicationVerdict, CausalCause, CausalEventRecord, CausalKind, ClaimAssessment,
    ClaimCorroboration, ClaimPolicy, ClaimProvenance, ClaimRecord, ClaimRefusal, ClaimStatus,
    ClaimSubject, ClaimSubjectKind, ClaimVerdict, DecompositionOutcomeRecord, DeliveryOutboxRecord,
    DeliveryState, DisagreementPosition, DisagreementRecord, DisagreementStatus,
    EngineInstanceRecord, EvidenceKind, EvidenceRef, FilingRefusal, IntentState, LeaseAcquisition,
    LeaseAuthority, LeaseRecord, PruneReport, QueuedWorkRecord, RunIntentRecord,
    CONFIDENCE_CERTAIN_PER_MILLE,
};
pub use store::schema::SCHEMA_VERSION;
pub use store::{
    default_store_path, ClaimFiling, CoordinationStore, DisagreementFiling, ProjectAccess,
    StoreError, StoreWarning,
};
pub use waitgraph::{
    detect, detect_stable, DeadlockReport, Detection, RunFacts, WaitEdge, WaitGraph,
    WaitGraphBounds, WaitHandle, WaitKind, WaitNode, WaitRegistry, WaitTarget, DEADLOCK_CODE,
};
