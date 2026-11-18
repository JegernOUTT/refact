use futures::future::BoxFuture;
use serde_json::{json, Map, Value};

use super::mask::item;
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput, Plan, Preview};
use crate::runtime_settings::{CoordinationOverride, TrajectoryRuntimeSettings};

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "settings.get",
            summary: "Runtime and coordination settings with value, default, bounds, source and restart-only flag.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"keys":{"type":"array","items":{"type":"string"},"description":"Keys to read; every key when absent."},"scope":{"enum":["effective","global","project"],"description":"Which value to report; effective by default."}}}),
            output: || Some(json!({"type":"object","required":["scope","settings"],"properties":{"scope":{"type":"string"},"settings":{"type":"object","description":"Keyed by setting name.","additionalProperties":{"type":"object","required":["value","default","source"],"properties":{"value":{},"default":{},"source":{"enum":["default","global","project"]},"range":{"type":"string","description":"min..max, inclusive"},"allowed":{"type":"array"},"restart_only":{"type":"boolean","description":"present when the value applies only after a restart"}}}}}})),
            handler: get,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "settings.schema",
            summary: "Every setting key with type, default, bounds, restart-only flag and how a change is treated.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["keys"],"properties":{"keys":{"type":"object","description":"Keyed by setting name.","additionalProperties":{"type":"object","required":["type","default","treatment"],"properties":{"type":{"type":"string"},"default":{},"treatment":{"enum":["ordinary","guarded","locked"],"description":"guarded: a change is risky and shows a warning; locked: no script may change it"},"group":{"type":"string"},"range":{"type":"string"},"allowed":{"type":"array"},"restart_only":{"type":"boolean"},"global_only":{"type":"boolean","description":"engine-wide; no project scope"}}}}}})),
            handler: schema,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "settings.diff",
            summary: "Keys whose effective value differs from the default, with the global and project values.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["changed"],"properties":{"changed":{"type":"object","description":"Keyed by setting name.","additionalProperties":{"type":"object","required":["value","default","global"],"properties":{"value":{},"default":{},"global":{},"project":{}}}}}})),
            handler: diff,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "settings.set",
            summary: "Change settings (scope project or global) after the user approves; guarded keys make the change risky.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["values"],"properties":{"values":{"type":"object","minProperties":1,"description":"Maps setting keys to new values."},"scope":{"enum":["project","global"],"description":"project (default) writes .refact/coordination.yaml; global writes the engine settings file."}}}),
            output: change_output,
            handler: set,
            preview: Some(set_preview),
            timeout: None,
        },
        EngineFn {
            name: "settings.reset",
            summary: "Set the given keys (or every changed key) back to their defaults in one scope after the user approves.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"keys":{"type":"array","minItems":1,"items":{"type":"string"},"description":"Keys to reset; every key that differs from its default when absent."},"scope":{"enum":["project","global"],"description":"project (default) or global."}}}),
            output: change_output,
            handler: reset,
            preview: Some(reset_preview),
            timeout: None,
        },
        EngineFn {
            name: "settings.clear_project_override",
            summary: "Remove project overrides so the keys inherit the global values, after the user approves.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"keys":{"type":"array","minItems":1,"items":{"type":"string"},"description":"Keys to clear; every overridden key when absent."}}}),
            output: change_output,
            handler: clear,
            preview: Some(clear_preview),
            timeout: None,
        },
    ]
}

fn change_output() -> Option<Value> {
    Some(
        json!({"type":"object","required":["scope","applied","skipped"],"properties":{"scope":{"type":"string"},"applied":{"type":"array","items":{"type":"string"}},"skipped":{"type":"array","items":{"type":"object","properties":{"key":{"type":"string"},"reason":{"type":"string"}}}}}}),
    )
}

/// Settings no engine function may change.
pub const LOCKED: &[&str] = &[
    "python_tool_enabled",
    "python_engine_read_enabled",
    "python_engine_change_enabled",
    "python_engine_risky_enabled",
    "python_engine_change_with_auto_commands",
];

/// Keys whose change cannot weaken a boundary, each with the reason. Every other key is guarded.
pub const ORDINARY: &[(&str, &str)] = &[
    (
        "internal_traces_keep_per_folder",
        "retention of internal trace files",
    ),
    (
        "internal_trace_prune_interval_secs",
        "how often old traces are pruned",
    ),
    (
        "buddy_conversations_keep",
        "retention of buddy conversation files",
    ),
    (
        "buddy_conversations_prune_interval_secs",
        "how often buddy files are pruned",
    ),
    (
        "buddy_conversations_prune_min_age_secs",
        "minimum age before buddy files are pruned",
    ),
    (
        "session_cleanup_interval_secs",
        "how often idle sessions are checked",
    ),
    ("max_queue_size", "queued messages per chat"),
    ("recent_request_ids_capacity", "duplicate-request memory"),
    ("max_images_per_message", "images kept in one message"),
    ("max_file_size", "largest file the editor tools read"),
    (
        "auto_enrichment_total_token_cap",
        "size of automatic context",
    ),
    (
        "auto_enrichment_card_token_cap",
        "size of one automatic context card",
    ),
    (
        "auto_enrichment_knowledge_top_n",
        "knowledge items in automatic context",
    ),
    (
        "trajectory_watcher_self_write_enabled",
        "whether the watcher reacts to its own writes",
    ),
    (
        "pp_max_tool_budget_tokens",
        "context budget of tool results",
    ),
    (
        "pp_max_per_file_budget_tokens",
        "context budget of one file",
    ),
    ("pp_max_line_length_chars", "line length in context"),
    ("tool_max_line_chars", "line length in tool output"),
    ("pp_tokens_for_text_percent", "share of context for text"),
    ("git_intel_max_commits", "history depth of git statistics"),
    (
        "git_intel_deep_walk_limit",
        "history depth of deep git walks",
    ),
    (
        "git_intel_max_files_per_commit_cochange",
        "files per commit in co-change statistics",
    ),
    (
        "git_intel_max_files_per_commit_entropy",
        "files per commit in entropy statistics",
    ),
    (
        "codegraph_dead_code_max_results",
        "rows of the dead code report",
    ),
    (
        "codegraph_exec_flow_max_nodes",
        "nodes of the execution flow report",
    ),
    ("cat_max_input_paths", "paths per cat call"),
    ("cat_max_lines", "lines per cat call"),
    ("cat_max_file_bytes", "bytes per file in cat"),
    ("cat_max_expanded_files", "files a cat glob expands to"),
    ("get_logs_max_tail_bytes", "log tail size"),
    ("planner_qna_question_limit", "length of planner questions"),
    ("planner_qna_answer_limit", "length of planner answers"),
    (
        "hist_search_preview_chars",
        "preview length of history search",
    ),
    ("agent_diff_max_output_bytes", "size of agent diffs"),
    (
        "process_subscribe_preview_bytes",
        "preview size of process output",
    ),
    (
        "plan_checkpoint_delta_threshold",
        "when plan checkpoints are written",
    ),
    (
        "plan_projection_max_chars",
        "size of the plan shown to the model",
    ),
    (
        "verification_evidence_output_cap_bytes",
        "size of kept verification output",
    ),
    (
        "verification_evidence_cache_max_entries",
        "verification cache entries",
    ),
    (
        "verification_evidence_cache_max_bytes",
        "verification cache size",
    ),
    (
        "verification_evidence_cache_ttl_secs",
        "verification cache lifetime",
    ),
    ("root_fairness_quantum", "scheduling share between chats"),
    ("root_weights", "scheduling weights between chats"),
    ("max_root_weight_entries", "number of scheduling weights"),
    ("max_resume_burst", "resumes per scheduling round"),
    ("default_priority", "default work priority"),
    ("max_priority_burst", "high-priority items per round"),
    ("max_parked_residency", "parked work items kept"),
    ("max_read_stamps", "read records kept for edit checks"),
    ("max_write_records", "write records kept"),
    ("max_undo_entries_per_file", "undo history per file"),
    ("max_undo_bytes", "undo history size"),
    ("overview_default_rows", "rows of work overviews"),
    ("overview_max_rows", "largest work overview"),
    ("max_work_views", "work views kept"),
    ("max_work_cursors", "work cursors kept"),
    ("max_work_history_rows", "work history rows kept"),
    ("max_work_history_bytes", "work history size"),
    ("max_work_provenance_records", "provenance records kept"),
    ("max_work_provenance_bytes", "provenance size"),
    ("work_cursor_ttl_secs", "lifetime of work cursors"),
    ("work_subscription_interval_ms", "work update interval"),
    ("work_stream_timeout_ms", "work stream timeout"),
    ("work_reconnect_max_ms", "work stream reconnect delay"),
    ("inspect_max_paths", "paths per inspection"),
    ("capsule_token_budget", "size of context capsules"),
    ("capsule_max_rows", "rows of context capsules"),
    (
        "min_interval_secs",
        "minimum interval of background refreshes",
    ),
    ("debounce_secs", "debounce of background refreshes"),
    ("recent_window_secs", "what counts as recent"),
    (
        "work_root_presence_ttl_ms",
        "presence lifetime of work roots",
    ),
    ("lock_timeout_secs", "how long a lock is awaited"),
    ("lock_poll_interval_ms", "lock polling interval"),
    (
        "question_default_expiry_secs",
        "default expiry of agent questions",
    ),
    ("deadlock_max_nodes", "size of the deadlock graph"),
    ("deadlock_max_edges", "size of the deadlock graph"),
    ("attention_batch_window_ms", "batching of attention notices"),
    (
        "attention_max_outstanding_questions",
        "open attention questions",
    ),
    (
        "delivery_default_ttl_secs",
        "default lifetime of deliveries",
    ),
    ("delivery_max_ttl_secs", "longest lifetime of deliveries"),
    ("delivery_status_retention", "delivery statuses kept"),
    ("agent_followup_max_bytes", "size of agent follow-ups"),
    ("agent_message_max_chars", "length of agent messages"),
    ("broadcast_rate_limit", "broadcasts per window"),
    ("broadcast_rate_window_secs", "broadcast window"),
    ("mailbox_default_rows", "rows of mailbox listings"),
    ("mailbox_max_rows", "largest mailbox listing"),
    ("board_claim_key_max_len", "length of board claim keys"),
    ("overlap_ordering_enabled", "ordering of overlapping work"),
    ("agent_wait_default_ms", "default wait for agents"),
    ("launch_context_tokens", "size of agent launch context"),
    (
        "launch_context_messages",
        "messages of agent launch context",
    ),
    ("launch_context_entries", "entries of agent launch context"),
    ("launch_context_bytes", "bytes of agent launch context"),
    ("launch_source_bytes", "bytes of agent launch sources"),
    ("agent_profile_max_count", "agent profiles kept"),
    ("agent_profile_max_bytes", "size of agent profiles"),
    ("handoff_report_max_entries", "entries of handoff reports"),
    ("handoff_report_max_chars", "length of handoff reports"),
    ("handoff_metric_max_records", "handoff metrics kept"),
    (
        "handoff_finding_min_samples",
        "samples before a handoff finding",
    ),
    (
        "handoff_finding_retention_ms",
        "retention of handoff findings",
    ),
    (
        "deliverable_result_budget_tokens",
        "size of deliverable results",
    ),
    ("max_path_scope_entries", "entries of a path scope"),
    ("agent_closed_retained_max", "closed agents kept"),
    ("agent_terminal_retained_max", "finished agents kept"),
    ("presence_heartbeat_interval_ms", "presence heartbeat"),
    ("presence_stale_after_ms", "when presence goes stale"),
    ("lease_renew_interval_ms", "lease renewal"),
    ("merge_lease_wait_ms", "wait for the merge lease"),
    ("merge_lease_poll_interval_ms", "merge lease polling"),
    ("store_busy_timeout_ms", "coordination store busy wait"),
    ("store_max_engine_instances", "engine instances tracked"),
    ("store_max_run_intents", "run intents kept"),
    ("store_max_queued_work", "queued work kept"),
    ("store_max_leases", "leases kept"),
    ("store_max_outbox_messages", "outbox messages kept"),
    (
        "store_max_activity_observations",
        "activity observations kept",
    ),
    (
        "store_max_decomposition_outcomes",
        "decomposition outcomes kept",
    ),
    ("store_max_disagreements", "disagreements kept"),
    ("disagreement_retention_ms", "retention of disagreements"),
    ("store_max_claims", "claims kept"),
    ("claim_max_per_subject", "claims per subject"),
    ("claim_retention_ms", "retention of claims"),
    (
        "claim_corroboration_step_per_mille",
        "claim confidence step",
    ),
    (
        "claim_contradiction_min_confidence_per_mille",
        "claim contradiction threshold",
    ),
    ("store_max_causal_events", "causal events kept"),
    (
        "decomposition_min_samples",
        "samples before decomposition advice",
    ),
    ("team_role_max_count", "roles per team"),
    ("team_mission_log_max_entries", "team mission log size"),
    ("team_heartbeat_ttl_ms", "team heartbeat lifetime"),
    ("outbox_retention_secs", "outbox retention"),
    ("outbox_prune_interval_secs", "outbox pruning"),
    ("run_intent_retention_secs", "run intent retention"),
    (
        "provider_aimd_increase_after_successes",
        "provider throttling step",
    ),
    (
        "provider_aimd_decrease_cooldown_ms",
        "provider throttling cooldown",
    ),
    ("provider_retry_jitter_per_mille", "provider retry jitter"),
    (
        "provider_pressure_max_buckets",
        "provider throttling buckets",
    ),
    ("metrics_max_series", "metric series kept"),
    ("metrics_max_explanations", "metric explanations kept"),
    ("metrics_max_label_chars", "metric label length"),
    (
        "event_channel_capacity",
        "restart only, so refused anyway; event buffer size",
    ),
    (
        "trajectory_writer_enabled",
        "restart only, so refused anyway; chat file writer",
    ),
    (
        "trajectory_index_coordinator_enabled",
        "restart only, so refused anyway; chat indexing",
    ),
    (
        "tool_catalog_snapshots_enabled",
        "restart only, so refused anyway; tool list caching",
    ),
];

/// Group of every guarded key that is in no listed group.
pub const OTHER_GUARDED: (&str, &str) = (
    "other",
    "a setting that is not known to be harmless changes; check its meaning in settings.schema",
);

/// Groups of guarded keys: (group, keys, what a change may weaken).
pub const GUARDED: &[(&str, &[&str], &str)] = &[
    (
        "script budgets",
        &[
            "python_max_run_ms",
            "python_max_compute_ms",
            "python_max_memory_bytes",
            "python_max_nested_calls",
            "python_max_parallel_calls",
            "python_max_live_sessions",
            "python_disk_quota_bytes",
            "python_max_state_bytes",
            "python_max_result_chars",
            "python_max_catalog_chars",
            "python_max_nested_result_bytes",
            "python_max_llm_calls",
            "python_max_llm_tokens",
        ],
        "scripts may run longer, use more memory and disk, or make more tool and model calls",
    ),
    (
        "approval timeout",
        &["approval_timeout_secs"],
        "pending approvals wait a different time before they expire",
    ),
    (
        "review and scope",
        &[
            "board_allow_self_review",
            "strict_path_scope_default",
            "disagreement_blocks_completion",
            "deliverable_max_corrections",
        ],
        "agents may review their own work, write outside a strict path scope, finish with open disagreements or get more correction rounds",
    ),
    (
        "peer grants",
        &[
            "peer_grant_default_ttl_secs",
            "peer_grant_max_ttl_secs",
            "peer_grant_max_count",
        ],
        "agents may hold more or longer-lived grants over each other",
    ),
    (
        "deadlock handling",
        &["deadlock_auto_cancel", "deadlock_detection_enabled"],
        "deadlocks may go undetected, or the engine may cancel agents on its own to break one",
    ),
    (
        "coordination store",
        &["store_allow_non_local_filesystem"],
        "the coordination store may live on a filesystem where locking is unreliable",
    ),
    (
        "automatic recovery",
        &[
            "recovery_auto_retry_enabled",
            "recovery_max_retries",
            "recovery_delay_ms",
            "recovery_total_timeout_ms",
        ],
        "failed turns may be retried automatically, spending more model calls",
    ),
    (
        "coordination",
        &["enabled", "auto_inject_enabled"],
        "agent coordination, including its disagreement and review gates, or the coordination notes injected into chats, may be switched off",
    ),
    (
        "stuck-run and deadlock detection",
        &[
            "no_progress_repeat_threshold",
            "no_progress_unchanged_turns",
            "no_progress_window_turns",
            "deadlock_scan_interval_ms",
            "deadlock_detection_attempts",
        ],
        "looping agents may keep spending without being reported, or deadlocks may be found much later",
    ),
    (
        "audit and review",
        &["metrics_enabled", "disagreement_max_open", "board_max_reviewers"],
        "the decision log may stop recording, or fewer reviewers and disagreements may be allowed",
    ),
    (
        "coordination leases",
        &["lease_takeover_grace_ms", "lease_ttl_ms"],
        "another engine may take over work leases almost at once",
    ),
    (
        "private memories",
        &["memory_supervisor_override"],
        "the planner may read the private memories of other agents",
    ),
    (
        "agent limits",
        &[
            "max_subchat_depth",
            "subagent_max_steps",
            "subagent_default_steps",
            "goal_sub_goal_max_depth",
            "goal_sub_goal_max_fan_out",
            "goal_auto_sub_goal_lane_cap",
            "max_pipeline_depth",
            "team_max_count",
        ],
        "agents may nest deeper, run more steps or start more agents and sub-goals",
    ),
    (
        "provider spend",
        &[
            "provider_retry_max_attempts",
            "provider_max_concurrency",
            "provider_pressure_enabled",
            "provider_min_concurrency",
            "provider_initial_concurrency",
            "provider_interactive_reserved_slots",
            "provider_breaker_failure_threshold",
            "provider_breaker_cooldown_ms",
            "provider_retry_base_backoff_ms",
            "provider_retry_max_backoff_ms",
            "provider_max_retry_after_ms",
        ],
        "model requests may be retried more often, run more at once or keep hitting a failing provider, spending more",
    ),
    (
        "waits and timeouts",
        &[
            "question_max_expiry_secs",
            "attention_default_timeout_secs",
            "stream_total_timeout_secs",
            "session_idle_timeout_secs",
            "verification_evidence_timeout_secs",
            "stream_idle_timeout_secs",
        ],
        "questions, model streams, sessions or verification commands may run or wait much longer",
    ),
];

/// The guarded group of `key`: a listed group, or the catch-all for every key that is not ordinary.
pub fn guarded_group(key: &str) -> Option<(&'static str, &'static str)> {
    if LOCKED.contains(&key) || ORDINARY.iter().any(|(name, _)| *name == key) {
        return None;
    }
    Some(
        GUARDED
            .iter()
            .find(|(_, keys, _)| keys.contains(&key))
            .map_or(OTHER_GUARDED, |(group, _, warning)| (*group, *warning)),
    )
}

fn treatment(key: &str) -> &'static str {
    if LOCKED.contains(&key) {
        "locked"
    } else if guarded_group(key).is_some() {
        "guarded"
    } else {
        "ordinary"
    }
}

#[derive(Clone)]
struct KeyInfo {
    name: String,
    coordination: bool,
    value_type: &'static str,
    minimum: Option<u64>,
    maximum: Option<u64>,
    allowed: Option<&'static [&'static str]>,
    restart_only: bool,
}

fn keys() -> Vec<KeyInfo> {
    let prefix = format!("{}.", crate::runtime_settings::COORDINATION_SETTINGS_KEY);
    crate::http::routers::v1::settings_config::trajectory_settings_fields()
        .into_iter()
        .map(|field| {
            let (name, coordination) = match field.name.strip_prefix(&prefix) {
                Some(name) => (name.to_string(), true),
                None => (field.name.clone(), false),
            };
            KeyInfo {
                name,
                coordination,
                value_type: field.value_type,
                minimum: field.minimum,
                maximum: field.maximum,
                allowed: field.allowed,
                restart_only: field.apply_mode == "restart_required",
            }
        })
        .collect()
}

fn lookup_key(all: &[KeyInfo], name: &str) -> Result<KeyInfo, EngineError> {
    all.iter().find(|k| k.name == name).cloned().ok_or_else(|| {
        let mut close: Vec<(f64, &str)> = all
            .iter()
            .map(|k| (strsim::jaro_winkler(name, &k.name), k.name.as_str()))
            .filter(|(score, _)| *score >= 0.8)
            .collect();
        close.sort_by(|a, b| b.0.total_cmp(&a.0));
        let hint = if close.is_empty() {
            "settings.schema lists every key".to_string()
        } else {
            format!(
                "did you mean {}?",
                close
                    .iter()
                    .take(3)
                    .map(|(_, n)| *n)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        EngineError::Type(format!("unknown setting '{name}'; {hint}"))
    })
}

fn field<'v>(root: &'v Value, key: &KeyInfo) -> Option<&'v Value> {
    if key.coordination {
        root.get(crate::runtime_settings::COORDINATION_SETTINGS_KEY)?
            .get(&key.name)
    } else {
        root.get(&key.name)
    }
}

struct Snapshot {
    defaults: Value,
    global: TrajectoryRuntimeSettings,
    global_json: Value,
    global_raw: Value,
    live: Value,
    project: CoordinationOverride,
    effective_coordination: Value,
}

fn failed(error: impl std::fmt::Display) -> EngineError {
    EngineError::Failed(error.to_string())
}

async fn snapshot(ctx: &EngineCtx) -> Result<Snapshot, EngineError> {
    let path = crate::runtime_settings::settings_path(&ctx.app.paths.config_dir);
    let global = crate::runtime_settings::load_from_path(&path)
        .await
        .map_err(failed)?;
    let global_raw = match tokio::fs::read_to_string(&path).await {
        Ok(text) if !text.trim().is_empty() => {
            serde_yaml::from_str::<Value>(&text).map_err(failed)?
        }
        Ok(_) => json!({}),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(failed(error)),
    };
    let project = match &ctx.project_root {
        Some(root) => crate::runtime_settings::load_project_coordination_override(
            &crate::runtime_settings::project_coordination_path(root),
        )
        .await
        .map_err(failed)?,
        None => CoordinationOverride::new(),
    };
    let live = crate::runtime_settings::current();
    let effective_coordination = serde_json::to_value(
        crate::runtime_settings::resolve_coordination(&live.coordination, &project)
            .map_err(failed)?,
    )
    .map_err(failed)?;
    Ok(Snapshot {
        defaults: serde_json::to_value(TrajectoryRuntimeSettings::default()).map_err(failed)?,
        global_json: serde_json::to_value(&global).map_err(failed)?,
        global,
        global_raw,
        live: serde_json::to_value(&live).map_err(failed)?,
        project,
        effective_coordination,
    })
}

impl Snapshot {
    fn default_value(&self, key: &KeyInfo) -> Value {
        field(&self.defaults, key).cloned().unwrap_or(Value::Null)
    }
    fn global_value(&self, key: &KeyInfo) -> Value {
        field(&self.global_json, key)
            .cloned()
            .unwrap_or(Value::Null)
    }
    fn project_value(&self, key: &KeyInfo) -> Option<Value> {
        key.coordination
            .then(|| self.project.get(&key.name).cloned())
            .flatten()
    }
    fn effective(&self, key: &KeyInfo) -> Value {
        if key.coordination {
            self.effective_coordination.get(&key.name).cloned()
        } else {
            self.live.get(&key.name).cloned()
        }
        .unwrap_or(Value::Null)
    }
    fn source(&self, key: &KeyInfo) -> &'static str {
        if self.project_value(key).is_some() {
            "project"
        } else if field(&self.global_raw, key).is_some() {
            "global"
        } else {
            "default"
        }
    }
}

fn wanted_keys(all: &[KeyInfo], args: &Value) -> Result<Vec<KeyInfo>, EngineError> {
    match args.get("keys").and_then(Value::as_array) {
        Some(names) => names
            .iter()
            .map(|name| lookup_key(all, name.as_str().unwrap_or_default()))
            .collect(),
        None => Ok(all.to_vec()),
    }
}

fn range(key: &KeyInfo) -> Option<String> {
    match (key.minimum, key.maximum) {
        (Some(min), Some(max)) => Some(format!("{min}..{max}")),
        _ => None,
    }
}

/// Compact per-key entry: the kernel converts at most a few thousand JSON nodes per result.
fn compact(mut entry: Map<String, Value>, key: &KeyInfo) -> Value {
    if let Some(range) = range(key) {
        entry.insert("range".into(), json!(range));
    }
    if let Some(allowed) = key.allowed {
        entry.insert("allowed".into(), json!(allowed));
    }
    if key.restart_only {
        entry.insert("restart_only".into(), json!(true));
    }
    Value::Object(entry)
}

fn get<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let all = keys();
        let wanted = wanted_keys(&all, args)?;
        let scope = args
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or("effective");
        let snap = snapshot(ctx).await?;
        let mut settings = Map::new();
        for key in &wanted {
            let value = match scope {
                "global" => snap.global_value(key),
                "project" => snap.project_value(key).unwrap_or(Value::Null),
                _ => snap.effective(key),
            };
            let mut entry = Map::new();
            entry.insert("value".into(), value);
            entry.insert("default".into(), snap.default_value(key));
            entry.insert("source".into(), json!(snap.source(key)));
            settings.insert(key.name.clone(), compact(entry, key));
        }
        let text = format!("{} settings ({scope})", settings.len());
        Ok(EngineOutput::new(
            json!({"scope":scope,"settings":settings}),
            text,
        ))
    })
}

fn schema<'a>(
    _ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let defaults =
            serde_json::to_value(TrajectoryRuntimeSettings::default()).map_err(failed)?;
        let mut all = Map::new();
        for key in keys() {
            let mut entry = Map::new();
            entry.insert("type".into(), json!(key.value_type));
            entry.insert(
                "default".into(),
                field(&defaults, &key).cloned().unwrap_or(Value::Null),
            );
            entry.insert("treatment".into(), json!(treatment(&key.name)));
            if let Some(group) = guarded_group(&key.name) {
                entry.insert("group".into(), json!(group.0));
            }
            if !key.coordination {
                entry.insert("global_only".into(), json!(true));
            }
            all.insert(key.name.clone(), compact(entry, &key));
        }
        let text = format!("{} setting keys", all.len());
        Ok(EngineOutput::new(json!({"keys":all}), text))
    })
}

fn diff<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let snap = snapshot(ctx).await?;
        let mut changed = Map::new();
        for key in keys()
            .iter()
            .filter(|key| snap.effective(key) != snap.default_value(key))
        {
            let mut row = json!({
                "value": snap.effective(key),
                "default": snap.default_value(key),
                "global": snap.global_value(key),
            });
            if let Some(project) = snap.project_value(key) {
                row["project"] = project;
            }
            changed.insert(key.name.clone(), row);
        }
        let text = format!("{} settings differ from their defaults", changed.len());
        Ok(EngineOutput::new(json!({"changed":changed}), text))
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Project,
    Global,
}

impl Scope {
    fn parse(args: &Value) -> Self {
        match args.get("scope").and_then(Value::as_str) {
            Some("global") => Self::Global,
            _ => Self::Project,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
        }
    }
}

#[derive(Clone)]
enum Apply {
    Project(std::path::PathBuf, CoordinationOverride),
    Global(TrajectoryRuntimeSettings),
}

#[derive(Clone)]
struct SettingsPlan {
    scope: Scope,
    /// (key, old, new); `None` old: not set, `None` new: removed.
    changes: Vec<(String, Option<Value>, Option<Value>)>,
    skipped: Vec<Value>,
    apply: Apply,
}

/// Refuses keys no engine function may change, before anything is asked.
fn check_changeable(key: &KeyInfo, scope: Scope) -> Result<(), EngineError> {
    if LOCKED.contains(&key.name.as_str()) {
        return Err(EngineError::Permission(format!(
            "{} is locked: it switches the python tool or engine functions and can only be changed by a person in Settings",
            key.name
        )));
    }
    if key.restart_only {
        return Err(EngineError::Permission(format!(
            "{} applies only after an engine restart and cannot be changed from a script; change it in Settings and restart",
            key.name
        )));
    }
    if scope == Scope::Project && !key.coordination {
        return Err(EngineError::Type(format!(
            "{} is an engine-wide setting; use scope=\"global\"",
            key.name
        )));
    }
    Ok(())
}

fn project_root(ctx: &EngineCtx) -> Result<std::path::PathBuf, EngineError> {
    ctx.project_root
        .clone()
        .ok_or_else(|| EngineError::Type("no project is open; use scope=\"global\"".into()))
}

fn excluded(key: &KeyInfo) -> Option<&'static str> {
    if LOCKED.contains(&key.name.as_str()) {
        Some("locked")
    } else if key.restart_only {
        Some("restart only")
    } else {
        None
    }
}

/// Builds and validates the new settings for `values` in `scope`.
async fn plan_values(
    ctx: &EngineCtx,
    scope: Scope,
    values: Vec<(KeyInfo, Option<Value>)>,
    skipped: Vec<Value>,
) -> Result<SettingsPlan, EngineError> {
    let snap = snapshot(ctx).await?;
    let mut changes = Vec::new();
    let apply = match scope {
        Scope::Project => {
            let root = project_root(ctx)?;
            super::common::inside_project(
                &root,
                &crate::runtime_settings::project_coordination_path(&root),
                &ctx.app.gcx.config_dir,
            )?;
            let mut overrides = snap.project.clone();
            for (key, new) in &values {
                let old = overrides.get(&key.name).cloned();
                if old == *new {
                    continue;
                }
                match new {
                    Some(value) => overrides.insert(key.name.clone(), value.clone()),
                    None => overrides.remove(&key.name),
                };
                let shown = old.clone().or_else(|| {
                    let from = if field(&snap.global_raw, key).is_some() {
                        "global"
                    } else {
                        "default"
                    };
                    Some(in_force(&snap.global_value(key), from))
                });
                changes.push((key.name.clone(), shown, new.clone()));
            }
            let overrides = crate::runtime_settings::normalize_coordination_override(overrides)
                .map_err(EngineError::Type)?;
            crate::runtime_settings::resolve_coordination(&snap.global.coordination, &overrides)
                .map_err(EngineError::Type)?;
            Apply::Project(root, overrides)
        }
        Scope::Global => {
            let mut rendered = snap.global_json.clone();
            for (key, new) in &values {
                let new = new.clone().unwrap_or_else(|| snap.default_value(key));
                let old = Some(match field(&snap.global_raw, key) {
                    Some(_) => snap.global_value(key),
                    None => in_force(&snap.global_value(key), "default"),
                });
                if snap.global_value(key) == new {
                    continue;
                }
                let slot = if key.coordination {
                    &mut rendered[crate::runtime_settings::COORDINATION_SETTINGS_KEY][&key.name]
                } else {
                    &mut rendered[&key.name]
                };
                *slot = new.clone();
                changes.push((key.name.clone(), old, Some(new)));
            }
            let settings: TrajectoryRuntimeSettings = serde_json::from_value(rendered)
                .map_err(|e| EngineError::Type(format!("invalid setting value: {e}")))?;
            crate::runtime_settings::validate(&settings).map_err(EngineError::Type)?;
            crate::runtime_settings::resolve_coordination(&settings.coordination, &snap.project)
                .map_err(|e| {
                    EngineError::Type(format!(
                        "the project overrides would no longer be valid: {e}"
                    ))
                })?;
            Apply::Global(settings)
        }
    };
    Ok(SettingsPlan {
        scope,
        changes,
        skipped,
        apply,
    })
}

async fn plan_set(ctx: &EngineCtx, args: &Value) -> Result<SettingsPlan, EngineError> {
    let scope = Scope::parse(args);
    let all = keys();
    let mut values = Vec::new();
    for (name, value) in args["values"].as_object().cloned().unwrap_or_default() {
        let key = lookup_key(&all, &name)?;
        check_changeable(&key, scope)?;
        if value.is_null() {
            return Err(EngineError::Type(format!(
                "{name}: null is not a value; use settings.reset or settings.clear_project_override"
            )));
        }
        values.push((key, Some(value)));
    }
    plan_values(ctx, scope, values, Vec::new()).await
}

async fn plan_reset(ctx: &EngineCtx, args: &Value) -> Result<SettingsPlan, EngineError> {
    let scope = Scope::parse(args);
    let all = keys();
    let snap = snapshot(ctx).await?;
    let mut values = Vec::new();
    let mut skipped = Vec::new();
    if args.get("keys").is_some() {
        for key in wanted_keys(&all, args)? {
            check_changeable(&key, scope)?;
            let default = snap.default_value(&key);
            values.push((key, Some(default)));
        }
    } else {
        for key in all
            .iter()
            .filter(|k| scope == Scope::Global || k.coordination)
        {
            let current = match scope {
                Scope::Project => snap.project_value(key),
                Scope::Global => Some(snap.global_value(key)),
            };
            if current
                .as_ref()
                .is_none_or(|v| *v == snap.default_value(key))
            {
                continue;
            }
            if let Some(reason) = excluded(key) {
                skipped.push(json!({"key":key.name,"reason":reason}));
                continue;
            }
            values.push((key.clone(), Some(snap.default_value(key))));
        }
    }
    plan_values(ctx, scope, values, skipped).await
}

async fn plan_clear(ctx: &EngineCtx, args: &Value) -> Result<SettingsPlan, EngineError> {
    let all = keys();
    let snap = snapshot(ctx).await?;
    let mut values = Vec::new();
    let mut skipped = Vec::new();
    if args.get("keys").is_some() {
        for key in wanted_keys(&all, args)? {
            check_changeable(&key, Scope::Project)?;
            if !snap.project.contains_key(&key.name) {
                skipped.push(json!({"key":key.name,"reason":"no project override"}));
                continue;
            }
            values.push((key, None));
        }
    } else {
        for name in snap.project.keys() {
            let key = lookup_key(&all, name)?;
            if let Some(reason) = excluded(&key) {
                skipped.push(json!({"key":key.name,"reason":reason}));
                continue;
            }
            values.push((key, None));
        }
    }
    plan_values(ctx, Scope::Project, values, skipped).await
}

/// The value in force when no override of this scope exists, marked with where it comes from.
fn in_force(value: &Value, from: &str) -> Value {
    let text = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    json!(format!("{text} ({from})"))
}

/// The state a settings plan is computed from: the global file and the project overrides.
fn plan_state(snap: &Snapshot) -> Value {
    json!({"global": snap.global_json, "project": snap.project})
}

fn preview_of(plan: SettingsPlan, state: Value, title: &str) -> Result<Preview, EngineError> {
    if plan.changes.is_empty() {
        return Ok(Preview {
            title: title.into(),
            items: Vec::new(),
            scope: Some(plan.scope.as_str().into()),
            class: EngineClass::Change,
            warning: None,
            plan: Plan::nothing(),
        });
    }
    let mut groups: Vec<(&str, &str)> = Vec::new();
    for (key, _, _) in &plan.changes {
        if let Some(group) = guarded_group(key) {
            if !groups.iter().any(|g| g.0 == group.0) {
                groups.push(group);
            }
        }
    }
    let warning = (!groups.is_empty()).then(|| {
        format!(
            "This changes guarded settings, so {}.",
            groups.iter().map(|g| g.1).collect::<Vec<_>>().join("; ")
        )
    });
    Ok(Preview {
        title: title.into(),
        items: plan
            .changes
            .iter()
            .map(|(key, old, new)| item(key, old.clone(), new.clone()))
            .collect(),
        scope: Some(plan.scope.as_str().into()),
        class: if groups.is_empty() {
            EngineClass::Change
        } else {
            EngineClass::Risky
        },
        warning,
        plan: Plan::new(state, plan),
    })
}

/// Applies exactly the approved plan, or fails when the settings moved since the preview.
async fn apply(ctx: &EngineCtx, function: &str) -> Result<EngineOutput, EngineError> {
    let current = plan_state(&snapshot(ctx).await?);
    let plan: SettingsPlan = ctx.take_plan(function, &current)?;
    match &plan.apply {
        Apply::Project(root, overrides) => {
            super::common::inside_project(
                root,
                &crate::runtime_settings::project_coordination_path(root),
                &ctx.app.gcx.config_dir,
            )?;
            crate::http::routers::v1::settings_config::persist_project_coordination(
                root, overrides,
            )
            .await
            .map_err(|e| EngineError::Failed(e.message))?;
        }
        Apply::Global(settings) => {
            crate::http::routers::v1::settings_config::persist_trajectory_settings(
                &ctx.app, settings,
            )
            .await
            .map_err(|e| EngineError::Failed(e.message))?;
        }
    }
    let applied: Vec<&str> = plan.changes.iter().map(|(k, _, _)| k.as_str()).collect();
    let mut text = format!(
        "changed {} setting{} ({})",
        applied.len(),
        if applied.len() == 1 { "" } else { "s" },
        plan.scope.as_str()
    );
    let python: Vec<&str> = applied
        .iter()
        .copied()
        .filter(|k| k.starts_with("python_"))
        .collect();
    if !python.is_empty() {
        text.push_str(&format!(
            "; {} applies from the next script run, this run keeps the limits it started with",
            python.join(", ")
        ));
    }
    let mut data = Map::new();
    data.insert("scope".into(), json!(plan.scope.as_str()));
    data.insert("applied".into(), json!(applied));
    data.insert("skipped".into(), json!(plan.skipped));
    Ok(EngineOutput::new(Value::Object(data), text))
}

async fn preview_with(
    ctx: &EngineCtx,
    plan: SettingsPlan,
    title: &str,
) -> Result<Preview, EngineError> {
    let state = plan_state(&snapshot(ctx).await?);
    preview_of(plan, state, title)
}

fn set_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        preview_with(ctx, plan_set(ctx, args).await?, "change engine settings").await
    })
}
fn set<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move { apply(ctx, "settings.set").await })
}
fn reset_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        preview_with(
            ctx,
            plan_reset(ctx, args).await?,
            "reset engine settings to their defaults",
        )
        .await
    })
}
fn reset<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move { apply(ctx, "settings.reset").await })
}
fn clear_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        preview_with(
            ctx,
            plan_clear(ctx, args).await?,
            "remove project setting overrides",
        )
        .await
    })
}
fn clear<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move { apply(ctx, "settings.clear_project_override").await })
}
