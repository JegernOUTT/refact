//! Local work discovery, cross-project-root catalog, privacy-gated views and
//! the significant-change context capsule (M1-03 / M1-08 / M1-09).

pub(crate) mod board_bootstrap;
pub mod board_index;
pub(crate) mod board_rows;
pub(crate) mod bounds;
pub mod broadcast;
pub mod capsule;
mod capsule_render;
#[cfg(test)]
mod capsule_tests;
pub mod catalog;
pub mod causal;
pub mod causal_view;
pub mod deadlock;
pub mod decomposition;
pub mod disagreement;
pub mod error;
pub mod fact_ledger;
pub mod file_provenance;
pub mod history;
pub mod leases;
pub mod merge_gate;
pub mod metrics;
pub mod outbox;
pub mod paging;
pub mod peer_authority;
pub mod presence;
pub mod privacy_guard;
pub mod provenance;
pub mod reconcile;
pub mod roots;
pub mod scope;
pub mod service;
#[cfg(test)]
mod service_tests;
pub mod teams;
pub mod types;
#[cfg(test)]
mod work_vocabulary_tests;
