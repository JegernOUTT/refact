//! Unified automation definitions and durable journal. Execution lives in callers.
mod catalog;
mod journal;
mod types;
pub use catalog::{AutomationCatalog, ProjectAutomationLease};
pub use journal::{journal_path, legacy_job_digest, AutomationJournal};
pub use types::*;
#[cfg(test)]
mod tests;
