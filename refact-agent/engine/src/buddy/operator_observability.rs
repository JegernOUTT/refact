//! On-demand, content-free diagnostics. No sampler, journal, rollout, or evidence expiry.
//! Missing product retention policy is deliberately represented as a refusal, not a TTL.
use serde::Serialize;

pub const MAX_DIAGNOSTIC_ROWS_PER_TABLE: usize = 10_000;
pub const MAX_COMPACTION_ROWS: usize = 10_000;

/// Explicit opt-in new-work backpressure, never a hard terminal-write disk quota.
/// Generic documents remain native-owner governed and are counted, not deleted.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapacityPolicy {
    pub max_rows_per_table: usize,
    pub max_database_bytes: u64,
}

/// Stable user-visible global budget, independent of the namespace registry.
pub const GLOBAL_DOCUMENT_BUDGET_MULTIPLIER: usize = 64;

/// Native installation only, inactive unless explicitly authorized by the operator.
/// All namespaces share the global admission budget; native terminal credits are prepaid.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentGrowthPolicy {
    pub max_rows_per_namespace: usize,
    pub replace_editorial_aggregate: bool,
}
impl Default for DocumentGrowthPolicy {
    fn default() -> Self {
        Self {
            max_rows_per_namespace: 4096,
            replace_editorial_aggregate: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicySnapshot {
    pub epoch: u64,
    pub policy: Option<DocumentGrowthPolicy>,
}
impl PolicySnapshot {
    pub fn version(&self) -> String {
        format!("document-policy:{}", self.epoch)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminalReservationSummary {
    pub retained_entries: u64,
    pub outstanding_slots: u64,
    pub grandfathered_entries: u64,
    pub policy_control_entries: u64,
    pub retained_ledger_is_charged: bool,
    pub covered_producers: &'static [&'static str],
    pub unclassified_writes_exempt: bool,
    pub hard_database_quota: bool,
}

/// Explicit registry of current ordinary owners. Unknown namespaces use the SAME
/// ordinary class and global budget, never a new independent budget.
pub const DOCUMENT_ADMISSION_NAMESPACES: &[&str] = &[
    "improvement_specs",
    "improvement_candidates",
    "resource_draft",
    "prepared_draft",
    "legacy_receipt",
    "legacy_activity",
    "prepared_action",
    "action_grant",
    "improvement_attempts",
    "policy_control_draft",
    "policy_control_prepared",
    "policy_control_audit",
    "work_action_origin",
    "work_action_bindings",
];
/// No namespace-wide exemption exists; these names require native owned reservations.
pub const DOCUMENT_RECOVERY_EXEMPTIONS: &[&str] = &[];
pub const DOCUMENT_RESERVED_NAMESPACES: &[&str] = &[
    "improvement_proofs",
    "improvement_fixed_runs",
    "improvement_comparisons",
    "resource_receipt",
    "improvement_installation",
    "improvement_host_release",
    "improvement_native_canary",
    "improvement_artifacts",
    "improvement_artifact_cleanup",
    "supervision_mandates",
    "supervision_links",
];
pub fn document_namespace_class(namespace: &str) -> &'static str {
    if namespace == "editorial" {
        "current_aggregate_with_material_history"
    } else if DOCUMENT_RESERVED_NAMESPACES.contains(&namespace) {
        "native_reserved_or_ordinary"
    } else {
        "ordinary_admission"
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DocumentGrowthSummary {
    pub snapshot: PolicySnapshot,
    pub terminal_reservations: TerminalReservationSummary,

    pub policy: Option<DocumentGrowthPolicy>,
    pub admission_namespaces: &'static [&'static str],
    pub recovery_exemptions: &'static [&'static str],
    pub global_document_bound: Option<usize>,
    pub charged_document_units: usize,
    pub reserved_namespaces: &'static [&'static str],
    pub unclassified_namespaces_may_grow: bool,
    pub editorial_frozen_through_revision: Option<u64>,
    /// At most one counter-only slot between material receipt transitions.
    /// Frozen history and terminal/feedback transitions are additional, not bounded here.
    pub editorial_mutable_slots: usize,
    pub editorial_selection_admission: bool,
    pub sequence_allocator_growth_bounded: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct TableSummary {
    pub table: &'static str,
    pub sampled_rows: u64,
    pub payload_bytes_lower_bound: u64,
    pub truncated: bool,
    /// None means there is no total row cap, not unlimited safe capacity.
    pub enforced_row_capacity: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoreSummary {
    pub new_work_admission_policy: Option<CapacityPolicy>,
    pub document_growth: DocumentGrowthSummary,
    pub generic_document_growth_is_native_owner_governed: bool,
    pub schema_version: u32,
    pub tables: Vec<TableSummary>,
    pub record_byte_limit: usize,
    pub page_limit: usize,
    pub checkpoint_evidence_limit: usize,
    pub scan_rows_per_table: usize,
    pub database_allocated_bytes: u64,
    pub database_reusable_bytes: u64,
    pub effective_database_byte_capacity: u64,
    /// WAL and explicit backups are not included in database page capacity.
    pub database_capacity_excludes_wal_and_backups: bool,
    pub active_runs: u64,
    pub interrupted_workflow_outcomes_lower_bound: u64,
    /// Counts below are lower bounds when executions are truncated.
    pub active_executions_lower_bound: u64,
    pub unknown_executions_lower_bound: u64,
    /// Conservative protection: every payload row, including every historical document,
    /// receipt, decision, pin and material revision. No reference graph is guessed.
    pub protected_payload_rows_lower_bound: u64,
    pub evidence_deletion_supported: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RetentionPreview {
    pub policy: &'static str,
    pub evidence_deletion_supported: bool,
    pub protected_payload_rows_lower_bound: u64,
    pub redundant_sequence_rows_in_batch: usize,
    pub sequence_high_watermark: u64,
    pub max_batch_rows: usize,
    pub requires_native_policy_for_evidence_deletion: bool,
}

/// In-process preparation only; deliberately neither Deserialize nor a bearer grant.
/// HTTP exposes preview only until a native exact-action retention policy exists.
pub struct PreparedSequenceCompaction {
    pub(crate) project: refact_buddy_core::operator_types::ProjectIdentity,
    pub(crate) watermark: u64,
    pub(crate) cutoff: u64,
    pub(crate) rows: usize,
}
