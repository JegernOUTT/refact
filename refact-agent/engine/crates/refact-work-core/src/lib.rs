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
pub mod waitgraph;

pub use contracts::{ActivityState, WorkOrigin, WorkPhase, WorkSummary, WorkerProjection};
pub use identity::{CheckoutKey, ProjectKey, RunKey, RuntimeKind, WorkKey, WorkScope};
pub use scheduler::{
    AdmissionError, AdmissionRequest, AdmissionScheduler, AdmissionSchedulerHandle, Permit,
    SchedulerSnapshot, Ticket, TicketId, TicketSnapshot, TicketState,
};
pub use settings::{
    coordination_settings, install_coordination_settings, CoordinationSettings, SettingsError,
    SettingsViolation,
};
pub use waitgraph::{
    detect, detect_stable, DeadlockReport, Detection, RunFacts, WaitEdge, WaitGraph,
    WaitGraphBounds, WaitHandle, WaitKind, WaitNode, WaitRegistry, WaitTarget, DEADLOCK_CODE,
};
