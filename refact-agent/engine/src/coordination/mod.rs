//! Local work discovery, cross-project-root catalog, privacy-gated views and
//! the significant-change context capsule (M1-03 / M1-08 / M1-09).

#[cfg(test)]
mod agent_wait_authorization_tests;
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
#[cfg(test)]
mod deadlock_identity_tests;
#[cfg(test)]
mod deadlock_snapshot_tests;
pub mod decomposition;
pub mod disagreement;
#[cfg(test)]
mod disagreement_gate_tests;
pub mod error;
pub mod fact_ledger;
#[cfg(test)]
mod fact_ledger_retention_tests;
pub mod file_provenance;
pub mod history;
pub mod launch_intents;
pub mod leases;
pub mod merge_gate;
pub mod metrics;
pub mod metrics_view;
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
#[cfg(test)]
mod team_config_tests;
#[cfg(test)]
mod team_registry_tests;
pub mod teams;
#[cfg(test)]
mod teams_delegation_tests;
pub mod types;
#[cfg(all(test, feature = "perf-tests"))]
mod validation_perf_tests;
#[cfg(test)]
mod work_vocabulary_tests;

pub fn engine_instance_id() -> String {
    format!(
        "{}:{}",
        hostname::get()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    )
}
