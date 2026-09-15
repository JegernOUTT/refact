use crate::contracts::WorkPriority;
use crate::store::records::CONFIDENCE_CERTAIN_PER_MILLE;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoordinationSettings {
    pub enabled: bool,
    pub auto_inject_enabled: bool,
    /// Let the task planner read `agent:<id>` memories it does not own. Off by
    /// default; every grant is audited.
    pub memory_supervisor_override: bool,
    pub max_active_engine: usize,
    pub max_active_project: usize,
    pub max_active_root: usize,
    /// Base admission opportunities per eligible root, multiplied by its weight.
    pub root_fairness_quantum: usize,
    /// Operator-assigned weights by stable root chat ID; unlisted roots weigh 1.
    pub root_weights: BTreeMap<String, usize>,
    pub max_root_weight_entries: usize,
    /// Maximum consecutive aged resumes before an eligible fresh run gets a turn.
    pub max_resume_burst: usize,
    /// Ordering class a run gets when its launch context assigns none.
    pub default_priority: WorkPriority,
    /// Consecutive admission opportunities a more urgent class may take ahead
    /// of the turn fairness alone would have served. Past the bound the
    /// fair-share turn is taken whatever the classes are, so every class keeps
    /// a minimum share of one opportunity in every `max_priority_burst + 1`.
    pub max_priority_burst: usize,
    pub max_pending: usize,
    pub max_outstanding: usize,
    /// Parked runs kept resident. Beyond this bound the least valuable parked
    /// run is checkpointed and evicted, which is not cancellation: it keeps its
    /// identity, queue age and accounting, and resumes from the checkpoint.
    pub max_parked_residency: usize,
    #[serde(alias = "max_depth")]
    pub max_subchat_depth: usize,
    pub max_read_stamps: usize,
    pub max_write_records: usize,
    pub max_undo_entries_per_file: usize,
    pub max_undo_bytes: usize,
    pub overview_default_rows: usize,
    pub overview_max_rows: usize,
    pub max_work_views: usize,
    pub max_work_cursors: usize,
    pub max_work_history_rows: usize,
    pub max_work_history_bytes: usize,
    pub max_work_provenance_records: usize,
    pub max_work_provenance_bytes: usize,
    pub work_cursor_ttl_secs: u64,
    pub work_subscription_interval_ms: u64,
    pub work_stream_max_bytes: usize,
    pub work_stream_timeout_ms: u64,
    pub work_reconnect_max_ms: u64,
    pub inspect_max_paths: usize,
    /// How long an observed work-root existence probe is reused before the
    /// directory is stat'ed again. Bounds the cost of reporting removed
    /// checkouts: repeated projections inside this window probe nothing.
    pub work_root_presence_ttl_ms: u64,
    pub capsule_token_budget: usize,
    pub capsule_max_rows: usize,
    pub min_interval_secs: u64,
    pub debounce_secs: u64,
    pub recent_window_secs: u64,
    pub lock_timeout_secs: u64,
    pub lock_poll_interval_ms: u64,
    pub approval_timeout_secs: u64,
    pub max_pending_approvals: usize,
    pub max_pending_questions: usize,
    pub question_default_expiry_secs: u64,
    pub question_max_expiry_secs: u64,
    pub deadlock_detection_enabled: bool,
    pub deadlock_scan_interval_ms: u64,
    pub deadlock_max_nodes: usize,
    pub deadlock_max_edges: usize,
    pub deadlock_detection_attempts: usize,
    /// Reporting is always on; breaking a detected cycle by cancelling the
    /// refused waiter stays opt-in.
    pub deadlock_auto_cancel: bool,
    /// How long arriving questions are collected before the human is shown one
    /// batch instead of a stream of interruptions. Zero presents immediately.
    pub attention_batch_window_ms: u64,
    /// Beyond this many outstanding human-facing questions, low-priority ones
    /// are refused rather than queued.
    pub attention_max_outstanding_questions: usize,
    /// How long a low-stakes question waits before its documented default is
    /// applied.
    pub attention_default_timeout_secs: u64,
    pub delivery_default_ttl_secs: u64,
    pub delivery_max_ttl_secs: u64,
    pub delivery_max_pending: usize,
    pub delivery_status_retention: usize,
    pub agent_message_max_chars: usize,
    pub peer_grant_default_ttl_secs: u64,
    pub peer_grant_max_ttl_secs: u64,
    pub peer_grant_max_count: usize,
    pub broadcast_rate_limit: usize,
    pub broadcast_rate_window_secs: u64,
    pub mailbox_default_rows: usize,
    pub mailbox_max_rows: usize,
    pub board_max_reviewers: usize,
    pub board_claim_key_max_len: usize,
    pub board_allow_self_review: bool,
    /// Cards one derived dependency wave may submit at once. Waves order
    /// submission only; admission capacity still governs execution, so the
    /// overflow is reported as undispatched rather than refused.
    pub max_wave_width: usize,
    /// Stages one pipeline lane may declare. A deeper lane is refused, because
    /// the depth is declared by the caller rather than derived from the DAG.
    pub max_pipeline_depth: usize,
    pub agent_wait_default_ms: u64,
    pub agent_wait_max_ms: u64,
    pub agent_wait_max_agents: usize,
    pub subagent_default_steps: usize,
    pub subagent_max_steps: usize,
    pub launch_context_tokens: usize,
    pub launch_context_messages: usize,
    pub launch_context_entries: usize,
    pub launch_context_bytes: usize,
    pub launch_source_bytes: usize,
    pub agent_profile_max_count: usize,
    pub agent_profile_max_bytes: usize,
    /// Gap entries kept from one child's handoff-sufficiency declaration.
    pub handoff_report_max_entries: usize,
    /// Characters kept per declared handoff field before explicit truncation.
    pub handoff_report_max_chars: usize,
    /// Finished children scanned when aggregating the handoff metric.
    pub handoff_metric_max_records: usize,
    /// Child declarations required before the handoff finding states a verdict
    /// instead of `insufficient_data`.
    pub handoff_finding_min_samples: usize,
    /// Age of the newest-first finished children a handoff finding may count.
    pub handoff_finding_retention_ms: u64,
    /// Correction attempts a child gets when its final message misses the
    /// declared deliverable contract. `0` reports the miss without re-asking.
    pub deliverable_max_corrections: usize,
    /// Context tokens that must remain free to hold a full deliverable. Below
    /// it the contract reports a recoverable omission instead of truncating.
    pub deliverable_result_budget_tokens: usize,
    /// Upgrades a declared-intent `reject` path scope to canonical enforcement.
    pub strict_path_scope_default: bool,
    pub max_path_scope_entries: usize,
    /// Prior-run outcomes retained on a closed or continued child record.
    pub agent_closed_retained_max: usize,
    /// Milliseconds a coordination-store connection waits for a competing
    /// writer before reporting the database as busy.
    pub store_busy_timeout_ms: u64,
    /// The coordination store refuses to open on a detected network
    /// filesystem, where WAL locking is not dependable. Setting this accepts
    /// that risk and downgrades the refusal to a reported warning.
    pub store_allow_non_local_filesystem: bool,
    /// How often this engine rewrites its own presence record.
    pub presence_heartbeat_interval_ms: u64,
    /// After this long without a heartbeat another engine reads as
    /// `unreachable`; it is never read as dead on a stale heartbeat alone.
    pub presence_stale_after_ms: u64,
    /// How long an acquired lease stays authoritative without renewal.
    pub lease_ttl_ms: u64,
    /// How often the owner refreshes its lease. Renewal never changes the
    /// fencing generation.
    pub lease_renew_interval_ms: u64,
    /// How long a lease must already be expired before an explicit takeover is
    /// allowed at all. Engine presence still has to say the holder is gone.
    pub lease_takeover_grace_ms: u64,
    /// How long a merge waits for the target checkout's merge lease before it
    /// fails as retryable. A merge never blocks indefinitely.
    pub merge_lease_wait_ms: u64,
    /// How often a waiting merge retries the checkout's merge lease.
    pub merge_lease_poll_interval_ms: u64,
    pub store_max_engine_instances: usize,
    pub store_max_run_intents: usize,
    pub store_max_queued_work: usize,
    pub store_max_leases: usize,
    pub store_max_outbox_messages: usize,
    pub store_max_activity_observations: usize,
    pub store_max_decomposition_outcomes: usize,
    pub store_max_disagreements: usize,
    /// Open disagreements one project may hold at once. Filing past the cap is
    /// refused rather than silently evicting a dispute nobody settled.
    pub disagreement_max_open: usize,
    /// How long a settled disagreement is kept. An open one is never dropped by
    /// age, however old it is.
    pub disagreement_retention_ms: u64,
    /// Whether an open disagreement stops work from being reported complete.
    /// Dissent is a signal, not automatically a blocker, so this is off unless
    /// a project asks for it.
    pub disagreement_blocks_completion: bool,
    /// Claims one project's fact ledger may hold. Past it the least recently
    /// touched are evicted, deterministically.
    pub store_max_claims: usize,
    /// Claims one subject may carry. Recording past the cap is refused rather
    /// than evicting something another run established.
    pub claim_max_per_subject: usize,
    /// How long a settled claim is kept. A standing or contested claim is never
    /// dropped by age.
    pub claim_retention_ms: u64,
    /// How much one independent corroboration raises a claim's confidence.
    pub claim_corroboration_step_per_mille: usize,
    /// How sure both sides must be before a contradiction is filed as a
    /// disagreement. Below it both claims are still kept, unmerged.
    pub claim_contradiction_min_confidence_per_mille: usize,
    /// Causal events retained, both in the in-memory recorder and in the
    /// store. Beyond it the oldest events are evicted and any trace that
    /// needed them reports itself incomplete rather than guessing.
    pub store_max_causal_events: usize,
    /// Comparable runs a cohort needs before the decomposition advisor reports
    /// a figure at all. Below it the advisor refuses rather than averaging a
    /// handful of runs into something that reads like a law.
    pub decomposition_min_samples: usize,
    /// Named teams one catalog may hold.
    pub team_max_count: usize,
    /// Roles one team may define.
    pub team_role_max_count: usize,
    /// Seats one roster may hold at once.
    pub team_member_max_count: usize,
    /// Mission-log entries one team keeps. The log is append-only, so the bound
    /// is what stops it growing forever: the oldest entry is dropped and the
    /// drop is counted rather than hidden.
    pub team_mission_log_max_entries: usize,
    /// How long a roster seat stays its holder's without a heartbeat. Inside
    /// this window a member is still alive and cannot be reclaimed.
    pub team_heartbeat_ttl_ms: u64,
    /// How long a settled outbox message is kept before retention deletes it.
    /// Messages whose outcome is still unknown are never pruned by age.
    pub outbox_retention_secs: u64,
    /// How often the durable outbox prunes settled messages.
    pub outbox_prune_interval_secs: u64,
    /// How long a run intent that reached a terminal outcome is kept before
    /// reconciliation prunes it. A run whose outcome is unknown is never
    /// pruned by age, however old it is.
    pub run_intent_retention_secs: u64,
    /// Adapts outbound provider concurrency to observed rate-limit pressure.
    /// Disabling it leaves provider traffic ungoverned.
    pub provider_pressure_enabled: bool,
    /// Ceiling for one provider credential bucket; growth never exceeds it.
    pub provider_max_concurrency: usize,
    /// Floor for one bucket; halving never drops below it.
    pub provider_min_concurrency: usize,
    /// Concurrency a freshly observed bucket starts at.
    pub provider_initial_concurrency: usize,
    /// Slots a background surge may never occupy, so interactive traffic keeps
    /// a way through a saturated bucket.
    pub provider_interactive_reserved_slots: usize,
    /// Consecutive successes before concurrency grows by one.
    pub provider_aimd_increase_after_successes: usize,
    /// How long after a halving further rate-limit pressure is absorbed without
    /// halving again, so one burst cannot collapse a bucket to the floor.
    pub provider_aimd_decrease_cooldown_ms: u64,
    /// Attempts a retryable failure class gets before the original cause is
    /// surfaced.
    pub provider_retry_max_attempts: usize,
    pub provider_retry_base_backoff_ms: u64,
    pub provider_retry_max_backoff_ms: u64,
    /// Jitter span added to a computed backoff, in per-mille of that backoff.
    pub provider_retry_jitter_per_mille: usize,
    /// Consecutive retryable failures before the bucket's breaker opens.
    pub provider_breaker_failure_threshold: usize,
    pub provider_breaker_cooldown_ms: u64,
    /// Upper bound on an honoured `Retry-After`; a provider cannot park traffic
    /// for longer than this.
    pub provider_max_retry_after_ms: u64,
    pub provider_pressure_max_buckets: usize,
    /// Identical completed tool calls, or identical text-only replies, that
    /// must recur inside the examined window before a run is reported as
    /// repeating itself. Elapsed silence never substitutes for this evidence.
    pub no_progress_repeat_threshold: usize,
    /// Turns a run's material output must stay unchanged, while it is still
    /// generating, before it is reported as producing nothing.
    pub no_progress_unchanged_turns: usize,
    /// Most recent turns examined for repetition.
    pub no_progress_window_turns: usize,
}

impl Default for CoordinationSettings {
    fn default() -> Self {
        CoordinationSettings {
            enabled: true,
            auto_inject_enabled: true,
            memory_supervisor_override: false,
            max_active_engine: 32,
            max_active_project: 16,
            max_active_root: 16,
            root_fairness_quantum: 1,
            root_weights: BTreeMap::new(),
            max_root_weight_entries: 4096,
            max_resume_burst: 4,
            default_priority: WorkPriority::Normal,
            max_priority_burst: 4,
            max_pending: 1024,
            max_outstanding: 4096,
            max_parked_residency: 256,
            max_subchat_depth: 5,
            max_read_stamps: 16384,
            max_write_records: 65536,
            max_undo_entries_per_file: 128,
            max_undo_bytes: 128 * 1024 * 1024,
            overview_default_rows: 50,
            overview_max_rows: 500,
            max_work_views: 4096,
            max_work_cursors: 16384,
            max_work_history_rows: 16384,
            max_work_history_bytes: 64 * 1024 * 1024,
            max_work_provenance_records: 16384,
            max_work_provenance_bytes: 4 * 1024 * 1024,
            work_cursor_ttl_secs: 300,
            work_subscription_interval_ms: 1000,
            work_stream_max_bytes: 16 * 1024 * 1024,
            work_stream_timeout_ms: 60_000,
            work_reconnect_max_ms: 10_000,
            inspect_max_paths: 200,
            work_root_presence_ttl_ms: 60_000,
            capsule_token_budget: 1600,
            capsule_max_rows: 24,
            min_interval_secs: 60,
            debounce_secs: 5,
            recent_window_secs: 3600,
            lock_timeout_secs: 30,
            lock_poll_interval_ms: 25,
            approval_timeout_secs: 1800,
            max_pending_approvals: 256,
            max_pending_questions: 256,
            question_default_expiry_secs: 1800,
            question_max_expiry_secs: 604_800,
            deadlock_detection_enabled: true,
            deadlock_scan_interval_ms: 5_000,
            deadlock_max_nodes: 4096,
            deadlock_max_edges: 16384,
            deadlock_detection_attempts: 3,
            deadlock_auto_cancel: false,
            attention_batch_window_ms: 2_000,
            attention_max_outstanding_questions: 32,
            attention_default_timeout_secs: 900,
            delivery_default_ttl_secs: 86_400,
            delivery_max_ttl_secs: 604_800,
            delivery_max_pending: 4096,
            delivery_status_retention: 16_384,
            agent_message_max_chars: 65_536,
            peer_grant_default_ttl_secs: 3600,
            peer_grant_max_ttl_secs: 604_800,
            peer_grant_max_count: 256,
            broadcast_rate_limit: 32,
            broadcast_rate_window_secs: 60,
            mailbox_default_rows: 20,
            mailbox_max_rows: 200,
            board_max_reviewers: 256,
            board_claim_key_max_len: 256,
            board_allow_self_review: false,
            max_wave_width: 8,
            max_pipeline_depth: 16,
            agent_wait_default_ms: 60_000,
            agent_wait_max_ms: 30 * 60 * 1000,
            agent_wait_max_agents: 128,
            subagent_default_steps: 256,
            subagent_max_steps: 4096,
            launch_context_tokens: 24_000,
            launch_context_messages: 1024,
            launch_context_entries: 256,
            launch_context_bytes: 16 * 1024 * 1024,
            launch_source_bytes: 4 * 1024 * 1024,
            agent_profile_max_count: 256,
            agent_profile_max_bytes: 64 * 1024,
            handoff_report_max_entries: 16,
            handoff_report_max_chars: 512,
            handoff_metric_max_records: 512,
            handoff_finding_min_samples: 5,
            handoff_finding_retention_ms: 7 * 24 * 60 * 60 * 1_000,
            deliverable_max_corrections: 1,
            deliverable_result_budget_tokens: 1_024,
            strict_path_scope_default: false,
            max_path_scope_entries: 256,
            agent_closed_retained_max: 64,
            store_busy_timeout_ms: 5_000,
            store_allow_non_local_filesystem: false,
            presence_heartbeat_interval_ms: 15_000,
            presence_stale_after_ms: 120_000,
            lease_ttl_ms: 120_000,
            lease_renew_interval_ms: 30_000,
            lease_takeover_grace_ms: 60_000,
            merge_lease_wait_ms: 300_000,
            merge_lease_poll_interval_ms: 250,
            store_max_engine_instances: 512,
            store_max_run_intents: 65_536,
            store_max_queued_work: 65_536,
            store_max_leases: 16_384,
            store_max_outbox_messages: 65_536,
            store_max_activity_observations: 262_144,
            store_max_decomposition_outcomes: 4_096,
            store_max_disagreements: 4_096,
            disagreement_max_open: 64,
            disagreement_retention_ms: 30 * 24 * 60 * 60 * 1_000,
            disagreement_blocks_completion: false,
            store_max_claims: 4_096,
            claim_max_per_subject: 64,
            claim_retention_ms: 30 * 24 * 60 * 60 * 1_000,
            claim_corroboration_step_per_mille: 100,
            claim_contradiction_min_confidence_per_mille: 250,
            store_max_causal_events: 16_384,
            team_max_count: 32,
            team_role_max_count: 16,
            team_member_max_count: 64,
            team_mission_log_max_entries: 512,
            team_heartbeat_ttl_ms: 120_000,
            decomposition_min_samples: 5,
            outbox_retention_secs: 604_800,
            outbox_prune_interval_secs: 3_600,
            run_intent_retention_secs: 604_800,
            provider_pressure_enabled: true,
            provider_max_concurrency: 16,
            provider_min_concurrency: 1,
            provider_initial_concurrency: 4,
            provider_interactive_reserved_slots: 1,
            provider_aimd_increase_after_successes: 8,
            provider_aimd_decrease_cooldown_ms: 1_000,
            provider_retry_max_attempts: 4,
            provider_retry_base_backoff_ms: 500,
            provider_retry_max_backoff_ms: 60_000,
            provider_retry_jitter_per_mille: 250,
            provider_breaker_failure_threshold: 8,
            provider_breaker_cooldown_ms: 30_000,
            provider_max_retry_after_ms: 300_000,
            provider_pressure_max_buckets: 4_096,
            no_progress_repeat_threshold: 3,
            no_progress_unchanged_turns: 12,
            no_progress_window_turns: 24,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsViolation {
    pub field: String,
    pub message: String,
}

impl fmt::Display for SettingsViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsError {
    pub violations: Vec<SettingsViolation>,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let joined = self
            .violations
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        write!(f, "invalid coordination settings: {joined}")
    }
}

impl std::error::Error for SettingsError {}

fn at_least(out: &mut Vec<SettingsViolation>, field: &str, value: u64, minimum: u64) {
    if value < minimum {
        out.push(SettingsViolation {
            field: field.to_string(),
            message: format!("must be at least {minimum}, got {value}"),
        });
    }
}

fn not_greater(
    out: &mut Vec<SettingsViolation>,
    field: &str,
    value: u64,
    other_field: &str,
    other: u64,
) {
    if value > other {
        out.push(SettingsViolation {
            field: field.to_string(),
            message: format!("must not exceed {other_field} ({other}), got {value}"),
        });
    }
}

impl CoordinationSettings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        let mut v = Vec::new();
        at_least(
            &mut v,
            "max_undo_entries_per_file",
            self.max_undo_entries_per_file as u64,
            1,
        );
        at_least(&mut v, "max_undo_bytes", self.max_undo_bytes as u64, 1);

        at_least(
            &mut v,
            "work_stream_max_bytes",
            self.work_stream_max_bytes as u64,
            1,
        );
        at_least(
            &mut v,
            "work_stream_timeout_ms",
            self.work_stream_timeout_ms,
            1,
        );
        at_least(
            &mut v,
            "work_reconnect_max_ms",
            self.work_reconnect_max_ms,
            1,
        );
        for (field, value) in [
            ("launch_context_tokens", self.launch_context_tokens as u64),
            (
                "launch_context_messages",
                self.launch_context_messages as u64,
            ),
            ("launch_context_entries", self.launch_context_entries as u64),
            ("launch_context_bytes", self.launch_context_bytes as u64),
            ("launch_source_bytes", self.launch_source_bytes as u64),
            (
                "agent_profile_max_count",
                self.agent_profile_max_count as u64,
            ),
            (
                "agent_profile_max_bytes",
                self.agent_profile_max_bytes as u64,
            ),
            (
                "handoff_report_max_entries",
                self.handoff_report_max_entries as u64,
            ),
            (
                "handoff_report_max_chars",
                self.handoff_report_max_chars as u64,
            ),
            (
                "handoff_metric_max_records",
                self.handoff_metric_max_records as u64,
            ),
            (
                "handoff_finding_min_samples",
                self.handoff_finding_min_samples as u64,
            ),
            (
                "handoff_finding_retention_ms",
                self.handoff_finding_retention_ms,
            ),
            ("max_path_scope_entries", self.max_path_scope_entries as u64),
            ("max_work_views", self.max_work_views as u64),
            ("max_work_cursors", self.max_work_cursors as u64),
            ("max_work_history_rows", self.max_work_history_rows as u64),
            ("max_work_history_bytes", self.max_work_history_bytes as u64),
            (
                "max_work_provenance_records",
                self.max_work_provenance_records as u64,
            ),
            (
                "max_work_provenance_bytes",
                self.max_work_provenance_bytes as u64,
            ),
            ("work_cursor_ttl_secs", self.work_cursor_ttl_secs),
            (
                "work_subscription_interval_ms",
                self.work_subscription_interval_ms,
            ),
            ("inspect_max_paths", self.inspect_max_paths as u64),
            ("max_wave_width", self.max_wave_width as u64),
            ("max_pipeline_depth", self.max_pipeline_depth as u64),
        ] {
            at_least(&mut v, field, value, 1);
        }
        at_least(
            &mut v,
            "max_active_engine",
            self.max_active_engine as u64,
            1,
        );
        at_least(
            &mut v,
            "max_active_project",
            self.max_active_project as u64,
            1,
        );
        at_least(&mut v, "max_active_root", self.max_active_root as u64, 1);
        at_least(
            &mut v,
            "root_fairness_quantum",
            self.root_fairness_quantum as u64,
            1,
        );
        at_least(&mut v, "max_resume_burst", self.max_resume_burst as u64, 1);
        at_least(
            &mut v,
            "max_priority_burst",
            self.max_priority_burst as u64,
            1,
        );
        at_least(
            &mut v,
            "max_root_weight_entries",
            self.max_root_weight_entries as u64,
            1,
        );
        not_greater(
            &mut v,
            "root_weights",
            self.root_weights.len() as u64,
            "max_root_weight_entries",
            self.max_root_weight_entries as u64,
        );
        for (root, weight) in &self.root_weights {
            if root.trim().is_empty() || root.trim() != root || root.chars().any(char::is_control) {
                v.push(SettingsViolation {
                    field: "root_weights".into(),
                    message: "root IDs must be nonempty, trimmed and contain no control characters"
                        .into(),
                });
            }
            at_least(&mut v, "root_weights", *weight as u64, 1);
        }
        at_least(&mut v, "max_pending", self.max_pending as u64, 1);
        at_least(&mut v, "max_outstanding", self.max_outstanding as u64, 1);
        at_least(
            &mut v,
            "max_parked_residency",
            self.max_parked_residency as u64,
            1,
        );
        not_greater(
            &mut v,
            "max_parked_residency",
            self.max_parked_residency as u64,
            "max_outstanding",
            self.max_outstanding as u64,
        );
        at_least(
            &mut v,
            "max_subchat_depth",
            self.max_subchat_depth as u64,
            1,
        );
        at_least(&mut v, "max_read_stamps", self.max_read_stamps as u64, 1);
        at_least(
            &mut v,
            "max_write_records",
            self.max_write_records as u64,
            1,
        );
        at_least(
            &mut v,
            "overview_default_rows",
            self.overview_default_rows as u64,
            1,
        );
        at_least(
            &mut v,
            "overview_max_rows",
            self.overview_max_rows as u64,
            1,
        );
        at_least(
            &mut v,
            "capsule_token_budget",
            self.capsule_token_budget as u64,
            1600,
        );
        at_least(&mut v, "capsule_max_rows", self.capsule_max_rows as u64, 1);
        at_least(&mut v, "recent_window_secs", self.recent_window_secs, 1);
        at_least(
            &mut v,
            "work_root_presence_ttl_ms",
            self.work_root_presence_ttl_ms,
            1,
        );
        at_least(&mut v, "lock_timeout_secs", self.lock_timeout_secs, 1);
        at_least(
            &mut v,
            "approval_timeout_secs",
            self.approval_timeout_secs,
            1,
        );
        for (field, value) in [
            ("delivery_default_ttl_secs", self.delivery_default_ttl_secs),
            ("delivery_max_ttl_secs", self.delivery_max_ttl_secs),
            ("agent_wait_default_ms", self.agent_wait_default_ms),
            ("agent_wait_max_ms", self.agent_wait_max_ms),
            (
                "peer_grant_default_ttl_secs",
                self.peer_grant_default_ttl_secs,
            ),
            ("peer_grant_max_ttl_secs", self.peer_grant_max_ttl_secs),
            (
                "broadcast_rate_window_secs",
                self.broadcast_rate_window_secs,
            ),
        ] {
            at_least(&mut v, field, value, 1);
        }
        for (field, value) in [
            ("delivery_max_pending", self.delivery_max_pending),
            ("delivery_status_retention", self.delivery_status_retention),
            ("agent_message_max_chars", self.agent_message_max_chars),
            ("peer_grant_max_count", self.peer_grant_max_count),
            ("broadcast_rate_limit", self.broadcast_rate_limit),
            ("mailbox_default_rows", self.mailbox_default_rows),
            ("mailbox_max_rows", self.mailbox_max_rows),
            ("board_max_reviewers", self.board_max_reviewers),
            ("board_claim_key_max_len", self.board_claim_key_max_len),
            ("agent_wait_max_agents", self.agent_wait_max_agents),
            ("agent_closed_retained_max", self.agent_closed_retained_max),
            (
                "store_max_engine_instances",
                self.store_max_engine_instances,
            ),
            ("store_max_run_intents", self.store_max_run_intents),
            ("store_max_queued_work", self.store_max_queued_work),
            ("store_max_leases", self.store_max_leases),
            ("store_max_outbox_messages", self.store_max_outbox_messages),
            (
                "store_max_activity_observations",
                self.store_max_activity_observations,
            ),
            (
                "store_max_decomposition_outcomes",
                self.store_max_decomposition_outcomes,
            ),
            ("store_max_disagreements", self.store_max_disagreements),
            ("disagreement_max_open", self.disagreement_max_open),
            ("store_max_claims", self.store_max_claims),
            ("claim_max_per_subject", self.claim_max_per_subject),
            (
                "claim_corroboration_step_per_mille",
                self.claim_corroboration_step_per_mille,
            ),
            ("store_max_causal_events", self.store_max_causal_events),
            ("decomposition_min_samples", self.decomposition_min_samples),
            ("team_max_count", self.team_max_count),
            ("team_role_max_count", self.team_role_max_count),
            ("team_member_max_count", self.team_member_max_count),
            (
                "team_mission_log_max_entries",
                self.team_mission_log_max_entries,
            ),
        ] {
            at_least(&mut v, field, value as u64, 1);
        }
        at_least(
            &mut v,
            "team_heartbeat_ttl_ms",
            self.team_heartbeat_ttl_ms,
            1,
        );
        at_least(
            &mut v,
            "disagreement_retention_ms",
            self.disagreement_retention_ms,
            1,
        );
        at_least(&mut v, "claim_retention_ms", self.claim_retention_ms, 1);
        not_greater(
            &mut v,
            "claim_contradiction_min_confidence_per_mille",
            self.claim_contradiction_min_confidence_per_mille as u64,
            "full certainty",
            CONFIDENCE_CERTAIN_PER_MILLE as u64,
        );
        not_greater(
            &mut v,
            "claim_corroboration_step_per_mille",
            self.claim_corroboration_step_per_mille as u64,
            "full certainty",
            CONFIDENCE_CERTAIN_PER_MILLE as u64,
        );
        at_least(
            &mut v,
            "store_busy_timeout_ms",
            self.store_busy_timeout_ms,
            1,
        );
        at_least(
            &mut v,
            "presence_heartbeat_interval_ms",
            self.presence_heartbeat_interval_ms,
            1,
        );
        at_least(
            &mut v,
            "outbox_retention_secs",
            self.outbox_retention_secs,
            1,
        );
        at_least(
            &mut v,
            "presence_stale_after_ms",
            self.presence_stale_after_ms,
            1,
        );
        not_greater(
            &mut v,
            "presence_heartbeat_interval_ms",
            self.presence_heartbeat_interval_ms,
            "presence_stale_after_ms",
            self.presence_stale_after_ms,
        );
        at_least(
            &mut v,
            "outbox_prune_interval_secs",
            self.outbox_prune_interval_secs,
            1,
        );
        at_least(
            &mut v,
            "run_intent_retention_secs",
            self.run_intent_retention_secs,
            1,
        );
        at_least(
            &mut v,
            "no_progress_repeat_threshold",
            self.no_progress_repeat_threshold as u64,
            2,
        );
        at_least(
            &mut v,
            "no_progress_unchanged_turns",
            self.no_progress_unchanged_turns as u64,
            2,
        );
        at_least(
            &mut v,
            "no_progress_window_turns",
            self.no_progress_window_turns as u64,
            2,
        );
        not_greater(
            &mut v,
            "no_progress_repeat_threshold",
            self.no_progress_repeat_threshold as u64,
            "no_progress_window_turns",
            self.no_progress_window_turns as u64,
        );
        not_greater(
            &mut v,
            "no_progress_unchanged_turns",
            self.no_progress_unchanged_turns as u64,
            "no_progress_window_turns",
            self.no_progress_window_turns as u64,
        );
        at_least(&mut v, "lease_ttl_ms", self.lease_ttl_ms, 1);
        at_least(
            &mut v,
            "lease_renew_interval_ms",
            self.lease_renew_interval_ms,
            1,
        );
        not_greater(
            &mut v,
            "lease_renew_interval_ms",
            self.lease_renew_interval_ms,
            "lease_ttl_ms",
            self.lease_ttl_ms,
        );
        at_least(&mut v, "merge_lease_wait_ms", self.merge_lease_wait_ms, 1);
        at_least(
            &mut v,
            "merge_lease_poll_interval_ms",
            self.merge_lease_poll_interval_ms,
            1,
        );
        not_greater(
            &mut v,
            "merge_lease_poll_interval_ms",
            self.merge_lease_poll_interval_ms,
            "merge_lease_wait_ms",
            self.merge_lease_wait_ms,
        );
        at_least(
            &mut v,
            "subagent_max_steps",
            self.subagent_max_steps as u64,
            1,
        );
        at_least(
            &mut v,
            "subagent_default_steps",
            self.subagent_default_steps as u64,
            1,
        );
        not_greater(
            &mut v,
            "subagent_default_steps",
            self.subagent_default_steps as u64,
            "subagent_max_steps",
            self.subagent_max_steps as u64,
        );
        at_least(
            &mut v,
            "max_pending_approvals",
            self.max_pending_approvals as u64,
            1,
        );
        at_least(
            &mut v,
            "max_pending_questions",
            self.max_pending_questions as u64,
            1,
        );
        at_least(
            &mut v,
            "question_default_expiry_secs",
            self.question_default_expiry_secs,
            1,
        );
        at_least(
            &mut v,
            "question_max_expiry_secs",
            self.question_max_expiry_secs,
            1,
        );
        not_greater(
            &mut v,
            "question_default_expiry_secs",
            self.question_default_expiry_secs,
            "question_max_expiry_secs",
            self.question_max_expiry_secs,
        );
        not_greater(
            &mut v,
            "question_max_expiry_secs",
            self.question_max_expiry_secs,
            "question_absolute_max_expiry_secs",
            31_536_000,
        );
        at_least(
            &mut v,
            "attention_max_outstanding_questions",
            self.attention_max_outstanding_questions as u64,
            1,
        );
        at_least(
            &mut v,
            "attention_default_timeout_secs",
            self.attention_default_timeout_secs,
            1,
        );
        not_greater(
            &mut v,
            "attention_default_timeout_secs",
            self.attention_default_timeout_secs,
            "question_max_expiry_secs",
            self.question_max_expiry_secs,
        );
        at_least(
            &mut v,
            "lock_poll_interval_ms",
            self.lock_poll_interval_ms,
            1,
        );
        at_least(
            &mut v,
            "deadlock_scan_interval_ms",
            self.deadlock_scan_interval_ms,
            1,
        );
        for (field, value) in [
            ("deadlock_max_nodes", self.deadlock_max_nodes),
            ("deadlock_max_edges", self.deadlock_max_edges),
            (
                "deadlock_detection_attempts",
                self.deadlock_detection_attempts,
            ),
        ] {
            at_least(&mut v, field, value as u64, 1);
        }

        not_greater(
            &mut v,
            "max_active_project",
            self.max_active_project as u64,
            "max_active_engine",
            self.max_active_engine as u64,
        );
        not_greater(
            &mut v,
            "max_active_root",
            self.max_active_root as u64,
            "max_active_engine",
            self.max_active_engine as u64,
        );
        not_greater(
            &mut v,
            "max_pending",
            self.max_pending as u64,
            "max_outstanding",
            self.max_outstanding as u64,
        );
        not_greater(
            &mut v,
            "overview_default_rows",
            self.overview_default_rows as u64,
            "overview_max_rows",
            self.overview_max_rows as u64,
        );
        not_greater(
            &mut v,
            "debounce_secs",
            self.debounce_secs,
            "min_interval_secs",
            self.min_interval_secs,
        );
        not_greater(
            &mut v,
            "lock_poll_interval_ms",
            self.lock_poll_interval_ms,
            "lock_timeout_secs_in_millis",
            self.lock_timeout_secs.saturating_mul(1000),
        );
        not_greater(
            &mut v,
            "delivery_default_ttl_secs",
            self.delivery_default_ttl_secs,
            "delivery_max_ttl_secs",
            self.delivery_max_ttl_secs,
        );
        not_greater(
            &mut v,
            "agent_wait_default_ms",
            self.agent_wait_default_ms,
            "agent_wait_max_ms",
            self.agent_wait_max_ms,
        );
        not_greater(
            &mut v,
            "peer_grant_default_ttl_secs",
            self.peer_grant_default_ttl_secs,
            "peer_grant_max_ttl_secs",
            self.peer_grant_max_ttl_secs,
        );
        not_greater(
            &mut v,
            "mailbox_default_rows",
            self.mailbox_default_rows as u64,
            "mailbox_max_rows",
            self.mailbox_max_rows as u64,
        );

        for (field, value) in [
            (
                "provider_max_concurrency",
                self.provider_max_concurrency as u64,
            ),
            (
                "provider_min_concurrency",
                self.provider_min_concurrency as u64,
            ),
            (
                "provider_initial_concurrency",
                self.provider_initial_concurrency as u64,
            ),
            (
                "provider_aimd_increase_after_successes",
                self.provider_aimd_increase_after_successes as u64,
            ),
            (
                "provider_retry_max_attempts",
                self.provider_retry_max_attempts as u64,
            ),
            (
                "provider_retry_base_backoff_ms",
                self.provider_retry_base_backoff_ms,
            ),
            (
                "provider_retry_max_backoff_ms",
                self.provider_retry_max_backoff_ms,
            ),
            (
                "provider_breaker_failure_threshold",
                self.provider_breaker_failure_threshold as u64,
            ),
            (
                "provider_breaker_cooldown_ms",
                self.provider_breaker_cooldown_ms,
            ),
            (
                "provider_max_retry_after_ms",
                self.provider_max_retry_after_ms,
            ),
            (
                "provider_pressure_max_buckets",
                self.provider_pressure_max_buckets as u64,
            ),
        ] {
            at_least(&mut v, field, value, 1);
        }
        not_greater(
            &mut v,
            "provider_min_concurrency",
            self.provider_min_concurrency as u64,
            "provider_initial_concurrency",
            self.provider_initial_concurrency as u64,
        );
        not_greater(
            &mut v,
            "provider_initial_concurrency",
            self.provider_initial_concurrency as u64,
            "provider_max_concurrency",
            self.provider_max_concurrency as u64,
        );
        not_greater(
            &mut v,
            "provider_interactive_reserved_slots",
            self.provider_interactive_reserved_slots as u64,
            "provider_max_concurrency_minus_one",
            self.provider_max_concurrency.saturating_sub(1) as u64,
        );
        not_greater(
            &mut v,
            "provider_retry_base_backoff_ms",
            self.provider_retry_base_backoff_ms,
            "provider_retry_max_backoff_ms",
            self.provider_retry_max_backoff_ms,
        );
        not_greater(
            &mut v,
            "provider_retry_jitter_per_mille",
            self.provider_retry_jitter_per_mille as u64,
            "provider_retry_jitter_per_mille_max",
            1000,
        );

        if v.is_empty() {
            Ok(())
        } else {
            Err(SettingsError { violations: v })
        }
    }

    pub fn validated(self) -> Result<Self, SettingsError> {
        self.validate()?;
        Ok(self)
    }

    pub fn overview_rows(&self, requested: Option<usize>) -> usize {
        let rows = requested.unwrap_or(self.overview_default_rows);
        rows.clamp(1, self.overview_max_rows.max(1))
    }

    pub fn root_quantum(&self, root_id: &str) -> usize {
        self.root_fairness_quantum
            .saturating_mul(self.root_weights.get(root_id).copied().unwrap_or(1))
    }

    pub fn lock_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.lock_timeout_secs)
    }

    pub fn lock_poll_interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.lock_poll_interval_ms)
    }

    pub fn recent_window(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.recent_window_secs)
    }

    pub fn attention_batch_window(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.attention_batch_window_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_and_wait_defaults_are_generous_and_consistent() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.delivery_default_ttl_secs <= settings.delivery_max_ttl_secs);
        assert!(settings.delivery_max_pending >= 1024);
        assert!(settings.delivery_status_retention >= settings.delivery_max_pending);
        assert!(settings.agent_wait_default_ms <= settings.agent_wait_max_ms);
        assert!(settings.agent_wait_max_agents >= 32);
    }

    #[test]
    fn delivery_and_wait_bounds_reject_zero_and_inverted_values() {
        for mutate in [
            |settings: &mut CoordinationSettings| settings.delivery_max_pending = 0,
            |settings: &mut CoordinationSettings| settings.delivery_status_retention = 0,
            |settings: &mut CoordinationSettings| settings.agent_message_max_chars = 0,
            |settings: &mut CoordinationSettings| settings.agent_wait_max_agents = 0,
            |settings: &mut CoordinationSettings| settings.agent_closed_retained_max = 0,
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
        let mut settings = CoordinationSettings::default();
        settings.delivery_default_ttl_secs = settings.delivery_max_ttl_secs + 1;
        assert!(settings.validate().is_err());
        let mut settings = CoordinationSettings::default();
        settings.agent_wait_default_ms = settings.agent_wait_max_ms + 1;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn parked_residency_bound_defaults_usable_and_rejects_invalid_values() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.max_parked_residency >= 1);
        assert!(settings.max_parked_residency <= settings.max_outstanding);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.max_parked_residency = 0,
            |settings: &mut CoordinationSettings| {
                settings.max_parked_residency = settings.max_outstanding + 1
            },
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn priority_bounds_default_usable_and_reject_an_unusable_burst() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert_eq!(settings.default_priority, WorkPriority::Normal);
        assert!(settings.max_priority_burst >= 1);
        let mut settings = CoordinationSettings::default();
        settings.max_priority_burst = 0;
        assert!(settings.validate().is_err());
        let rendered = serde_json::to_value(CoordinationSettings::default()).unwrap();
        assert_eq!(rendered["default_priority"], "normal");
        let parsed: CoordinationSettings =
            serde_json::from_value(serde_json::json!({"default_priority": "user_blocking"}))
                .unwrap();
        assert_eq!(parsed.default_priority, WorkPriority::UserBlocking);
        assert!(
            serde_json::from_value::<CoordinationSettings>(
                serde_json::json!({"default_priority": "urgent"})
            )
            .is_err(),
            "a class outside the ladder is refused, not coerced"
        );
    }

    #[test]
    fn path_scope_bounds_default_declared_and_reject_zero_entries() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(!settings.strict_path_scope_default);
        assert!(settings.max_path_scope_entries >= 1);
        let mut settings = CoordinationSettings::default();
        settings.max_path_scope_entries = 0;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn question_bounds_default_usable_and_reject_invalid_values() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.max_pending_questions >= 1);
        assert!(settings.question_default_expiry_secs <= settings.question_max_expiry_secs);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.max_pending_questions = 0,
            |settings: &mut CoordinationSettings| settings.question_default_expiry_secs = 0,
            |settings: &mut CoordinationSettings| settings.question_max_expiry_secs = 0,
            |settings: &mut CoordinationSettings| {
                settings.question_default_expiry_secs = settings.question_max_expiry_secs + 1
            },
            |settings: &mut CoordinationSettings| settings.question_max_expiry_secs = 31_536_001,
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn deadlock_bounds_default_usable_and_reject_zero() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.deadlock_detection_enabled);
        assert!(!settings.deadlock_auto_cancel);
        assert!(settings.deadlock_scan_interval_ms >= 1);
        assert!(settings.deadlock_max_nodes >= 1);
        assert!(settings.deadlock_max_edges >= settings.deadlock_max_nodes);
        assert!(settings.deadlock_detection_attempts >= 2);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.deadlock_scan_interval_ms = 0,
            |settings: &mut CoordinationSettings| settings.deadlock_max_nodes = 0,
            |settings: &mut CoordinationSettings| settings.deadlock_max_edges = 0,
            |settings: &mut CoordinationSettings| settings.deadlock_detection_attempts = 0,
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn attention_bounds_default_generous_and_reject_invalid_values() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.attention_max_outstanding_questions >= 1);
        assert!(settings.attention_default_timeout_secs <= settings.question_max_expiry_secs);
        assert_eq!(
            settings.attention_batch_window(),
            std::time::Duration::from_millis(settings.attention_batch_window_ms)
        );
        let mut immediate = CoordinationSettings::default();
        immediate.attention_batch_window_ms = 0;
        immediate.validate().unwrap();
        for mutate in [
            |settings: &mut CoordinationSettings| settings.attention_max_outstanding_questions = 0,
            |settings: &mut CoordinationSettings| settings.attention_default_timeout_secs = 0,
            |settings: &mut CoordinationSettings| {
                settings.attention_default_timeout_secs = settings.question_max_expiry_secs + 1
            },
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn peer_mail_bounds_default_usable_and_reject_invalid_values() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.peer_grant_max_count >= 1);
        assert!(settings.broadcast_rate_limit >= 1);
        assert!(settings.broadcast_rate_window_secs >= 1);
        assert!(settings.mailbox_default_rows <= settings.mailbox_max_rows);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.peer_grant_max_count = 0,
            |settings: &mut CoordinationSettings| settings.broadcast_rate_limit = 0,
            |settings: &mut CoordinationSettings| settings.broadcast_rate_window_secs = 0,
            |settings: &mut CoordinationSettings| settings.mailbox_default_rows = 0,
            |settings: &mut CoordinationSettings| settings.mailbox_max_rows = 0,
            |settings: &mut CoordinationSettings| {
                settings.mailbox_default_rows = settings.mailbox_max_rows + 1
            },
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn handoff_bounds_default_usable_and_reject_zero() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.handoff_report_max_entries >= 1);
        assert!(settings.handoff_report_max_chars >= 64);
        assert!(settings.handoff_metric_max_records >= 1);
        assert!(
            settings.handoff_finding_min_samples >= 2,
            "one complaint is not a trend"
        );
        assert!(settings.handoff_finding_retention_ms >= 60_000);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.handoff_report_max_entries = 0,
            |settings: &mut CoordinationSettings| settings.handoff_report_max_chars = 0,
            |settings: &mut CoordinationSettings| settings.handoff_metric_max_records = 0,
            |settings: &mut CoordinationSettings| settings.handoff_finding_min_samples = 0,
            |settings: &mut CoordinationSettings| settings.handoff_finding_retention_ms = 0,
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn store_bounds_default_generous_and_reject_zero() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(!settings.store_allow_non_local_filesystem);
        assert!(settings.store_busy_timeout_ms >= 1_000);
        assert!(settings.store_max_engine_instances >= 1);
        assert!(settings.store_max_run_intents >= 1024);
        assert!(settings.store_max_queued_work >= 1024);
        assert!(settings.store_max_leases >= 1024);
        assert!(settings.store_max_outbox_messages >= 1024);
        assert!(settings.store_max_activity_observations >= 1024);
        assert!(settings.outbox_retention_secs >= 86_400);
        assert!(settings.outbox_prune_interval_secs >= 1);
        assert!(settings.run_intent_retention_secs >= 86_400);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.run_intent_retention_secs = 0,
            |settings: &mut CoordinationSettings| settings.outbox_retention_secs = 0,
            |settings: &mut CoordinationSettings| settings.outbox_prune_interval_secs = 0,
            |settings: &mut CoordinationSettings| settings.store_busy_timeout_ms = 0,
            |settings: &mut CoordinationSettings| settings.store_max_engine_instances = 0,
            |settings: &mut CoordinationSettings| settings.store_max_run_intents = 0,
            |settings: &mut CoordinationSettings| settings.store_max_queued_work = 0,
            |settings: &mut CoordinationSettings| settings.store_max_leases = 0,
            |settings: &mut CoordinationSettings| settings.store_max_outbox_messages = 0,
            |settings: &mut CoordinationSettings| settings.store_max_activity_observations = 0,
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn decomposition_bounds_default_usable_and_reject_zero() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.store_max_decomposition_outcomes >= 1024);
        assert!(settings.decomposition_min_samples >= 2);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.store_max_decomposition_outcomes = 0,
            |settings: &mut CoordinationSettings| settings.decomposition_min_samples = 0,
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn disagreement_bounds_default_usable_and_reject_zero() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(
            !settings.disagreement_blocks_completion,
            "dissent must not block completion unless a project asks for it"
        );
        assert!(settings.store_max_disagreements >= 1024);
        assert!(settings.disagreement_max_open >= 1);
        assert!(settings.disagreement_retention_ms >= 86_400_000);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.store_max_disagreements = 0,
            |settings: &mut CoordinationSettings| settings.disagreement_max_open = 0,
            |settings: &mut CoordinationSettings| settings.disagreement_retention_ms = 0,
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn no_progress_bounds_default_generous_and_reject_invalid_values() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.no_progress_repeat_threshold >= 3);
        assert!(settings.no_progress_unchanged_turns >= 8);
        assert!(settings.no_progress_window_turns >= settings.no_progress_unchanged_turns);
        assert!(settings.no_progress_window_turns >= settings.no_progress_repeat_threshold);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.no_progress_repeat_threshold = 0,
            |settings: &mut CoordinationSettings| settings.no_progress_repeat_threshold = 1,
            |settings: &mut CoordinationSettings| settings.no_progress_unchanged_turns = 1,
            |settings: &mut CoordinationSettings| settings.no_progress_window_turns = 1,
            |settings: &mut CoordinationSettings| settings.no_progress_window_turns = 2,
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn presence_bounds_default_generous_and_reject_invalid_values() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.presence_heartbeat_interval_ms >= 5_000);
        assert!(settings.presence_stale_after_ms >= settings.presence_heartbeat_interval_ms * 3);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.presence_heartbeat_interval_ms = 0,
            |settings: &mut CoordinationSettings| settings.presence_stale_after_ms = 0,
            |settings: &mut CoordinationSettings| {
                settings.presence_heartbeat_interval_ms = settings.presence_stale_after_ms + 1
            },
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn lease_bounds_default_generous_and_reject_invalid_values() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.lease_ttl_ms >= 60_000);
        assert!(settings.lease_renew_interval_ms <= settings.lease_ttl_ms / 2);
        let mut immediate = CoordinationSettings::default();
        immediate.lease_takeover_grace_ms = 0;
        immediate.validate().unwrap();
        for mutate in [
            |settings: &mut CoordinationSettings| settings.lease_ttl_ms = 0,
            |settings: &mut CoordinationSettings| settings.lease_renew_interval_ms = 0,
            |settings: &mut CoordinationSettings| {
                settings.lease_renew_interval_ms = settings.lease_ttl_ms + 1
            },
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn merge_lease_bounds_default_generous_and_reject_invalid_values() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.merge_lease_wait_ms >= 60_000);
        assert!(settings.merge_lease_poll_interval_ms <= settings.merge_lease_wait_ms);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.merge_lease_wait_ms = 0,
            |settings: &mut CoordinationSettings| settings.merge_lease_poll_interval_ms = 0,
            |settings: &mut CoordinationSettings| {
                settings.merge_lease_poll_interval_ms = settings.merge_lease_wait_ms + 1
            },
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn provider_pressure_bounds_default_generous_and_reject_invalid_values() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.provider_pressure_enabled);
        assert!(settings.provider_min_concurrency <= settings.provider_initial_concurrency);
        assert!(settings.provider_initial_concurrency <= settings.provider_max_concurrency);
        assert!(settings.provider_interactive_reserved_slots < settings.provider_max_concurrency);
        assert!(settings.provider_retry_base_backoff_ms <= settings.provider_retry_max_backoff_ms);
        assert!(settings.provider_retry_jitter_per_mille <= 1000);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.provider_max_concurrency = 0,
            |settings: &mut CoordinationSettings| settings.provider_min_concurrency = 0,
            |settings: &mut CoordinationSettings| settings.provider_initial_concurrency = 0,
            |settings: &mut CoordinationSettings| settings.provider_retry_max_attempts = 0,
            |settings: &mut CoordinationSettings| settings.provider_breaker_failure_threshold = 0,
            |settings: &mut CoordinationSettings| settings.provider_breaker_cooldown_ms = 0,
            |settings: &mut CoordinationSettings| settings.provider_max_retry_after_ms = 0,
            |settings: &mut CoordinationSettings| settings.provider_pressure_max_buckets = 0,
            |settings: &mut CoordinationSettings| {
                settings.provider_min_concurrency = settings.provider_initial_concurrency + 1
            },
            |settings: &mut CoordinationSettings| {
                settings.provider_initial_concurrency = settings.provider_max_concurrency + 1
            },
            |settings: &mut CoordinationSettings| {
                settings.provider_interactive_reserved_slots = settings.provider_max_concurrency
            },
            |settings: &mut CoordinationSettings| {
                settings.provider_retry_base_backoff_ms = settings.provider_retry_max_backoff_ms + 1
            },
            |settings: &mut CoordinationSettings| settings.provider_retry_jitter_per_mille = 1001,
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn agent_profile_bounds_default_usable_and_reject_zero() {
        let settings = CoordinationSettings::default();
        settings.validate().unwrap();
        assert!(settings.agent_profile_max_count >= 1);
        assert!(settings.agent_profile_max_bytes >= 1024);
        for mutate in [
            |settings: &mut CoordinationSettings| settings.agent_profile_max_count = 0,
            |settings: &mut CoordinationSettings| settings.agent_profile_max_bytes = 0,
        ] {
            let mut settings = CoordinationSettings::default();
            mutate(&mut settings);
            assert!(settings.validate().is_err());
        }
    }
}

fn global_slot() -> &'static RwLock<CoordinationSettings> {
    static SLOT: OnceLock<RwLock<CoordinationSettings>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(CoordinationSettings::default()))
}

pub fn install_coordination_settings(
    settings: CoordinationSettings,
) -> Result<CoordinationSettings, SettingsError> {
    settings.validate()?;
    let mut guard = global_slot()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = settings;
    Ok(guard.clone())
}

pub fn coordination_settings() -> CoordinationSettings {
    global_slot()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}
