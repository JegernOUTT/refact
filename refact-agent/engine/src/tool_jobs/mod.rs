//! Runtime-owned tool invocations. Admission is deliberately separate from authorization:
//! callers must finish confirmation, hooks, argument revalidation and acquire a lifecycle
//! lease before moving a prepared invocation here. A receipt is not execution success.
mod runtime;
pub mod scope;
pub use scope::{ExecutionLease, ScopeChangeError, ScopeChangeGuard, ScopeCoordinator, ScopeKey};

pub use runtime::{
    JobBinding, JobEvent, JobReceipt, JobResult, JobRuntime, JobSnapshot, JobStatus, Submission,
    RESULT_NOT_KEPT, RESULT_RETENTION_BYTES, TERMINAL_RECORD_CAP,
};
