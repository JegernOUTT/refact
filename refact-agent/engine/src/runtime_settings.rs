use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

const INTERNAL_TRACES_KEEP_PER_FOLDER_DEFAULT: usize = 200;
const INTERNAL_TRACE_PRUNE_INTERVAL_SECS_DEFAULT: u64 = 3600;
const BUDDY_CONVERSATIONS_KEEP_DEFAULT: usize = 500;
const BUDDY_CONVERSATIONS_PRUNE_INTERVAL_SECS_DEFAULT: u64 = 3600;
const BUDDY_CONVERSATIONS_PRUNE_MIN_AGE_SECS_DEFAULT: u64 = 86_400;
const SESSION_IDLE_TIMEOUT_SECS_DEFAULT: u64 = 30 * 60;
const SESSION_CLEANUP_INTERVAL_SECS_DEFAULT: u64 = 5 * 60;
const STREAM_IDLE_TIMEOUT_SECS_DEFAULT: u64 = 5 * 60;
const STREAM_TOTAL_TIMEOUT_SECS_DEFAULT: u64 = 30 * 60;
const MAX_QUEUE_SIZE_DEFAULT: usize = 100;
const EVENT_CHANNEL_CAPACITY_DEFAULT: usize = 4096;
const RECENT_REQUEST_IDS_CAPACITY_DEFAULT: usize = 100;
const MAX_IMAGES_PER_MESSAGE_DEFAULT: usize = 50;
const MAX_FILE_SIZE_DEFAULT: usize = 40_000;
const AUTO_ENRICHMENT_TOTAL_TOKEN_CAP_DEFAULT: usize = 1600;
const AUTO_ENRICHMENT_CARD_TOKEN_CAP_DEFAULT: usize = 480;
const KNOWLEDGE_TOP_N_DEFAULT: usize = 3;
const TRAJECTORY_TOP_N_DEFAULT: usize = 2;
const PP_MAX_TOOL_BUDGET_TOKENS_DEFAULT: usize = 131_072;
const PP_MAX_PER_FILE_BUDGET_TOKENS_DEFAULT: usize = 131_072;
const PP_MAX_LINE_LENGTH_CHARS_DEFAULT: usize = 10_000;
const PP_TOKENS_FOR_TEXT_PERCENT_DEFAULT: usize = 30;
const GIT_INTEL_MAX_COMMITS_DEFAULT: usize = 5_000;
const GIT_INTEL_DEEP_WALK_LIMIT_DEFAULT: usize = 50_000;
const GIT_INTEL_MAX_FILES_PER_COMMIT_COCHANGE_DEFAULT: usize = 1_000;
const GIT_INTEL_MAX_FILES_PER_COMMIT_ENTROPY_DEFAULT: usize = 200;
const CODEGRAPH_DEAD_CODE_MAX_RESULTS_DEFAULT: usize = 5_000;
const CODEGRAPH_EXEC_FLOW_MAX_NODES_DEFAULT: usize = 5_000;
const VECDB_TRAJECTORY_SPLIT_BYTES_DEFAULT: usize = 16_384;
const CAT_MAX_INPUT_PATHS_DEFAULT: usize = 512;
const CAT_MAX_LINES_DEFAULT: usize = 5_000;
const CAT_MAX_FILE_BYTES_DEFAULT: usize = 8_388_608;
const CAT_MAX_EXPANDED_FILES_DEFAULT: usize = 2_048;
const GET_LOGS_MAX_TAIL_BYTES_DEFAULT: usize = 1_048_576;
const PLANNER_QNA_QUESTION_LIMIT_DEFAULT: usize = 8_000;
const PLANNER_QNA_ANSWER_LIMIT_DEFAULT: usize = 8_000;
const HIST_SEARCH_PREVIEW_CHARS_DEFAULT: usize = 2_000;
const AGENT_DIFF_MAX_OUTPUT_BYTES_DEFAULT: usize = 4_194_304;
const PROCESS_SUBSCRIBE_PREVIEW_BYTES_DEFAULT: usize = 2_000;
const REVIEW_DIFF_CHAR_CAP_DEFAULT: usize = 400_000;
const REVIEW_MAX_DIFF_PATCH_BYTES_DEFAULT: usize = 4_194_304;

pub use refact_work_core::CoordinationSettings;

/// Single source of truth for every coordination bound.
///
/// `validate` walks this table generically and the HTTP layer publishes it
/// verbatim, so engine enforcement and the ranges the GUI offers cannot drift.
/// Defaults live in `refact_work_core`; only the admissible range lives here.
pub struct CoordinationFieldBound {
    pub name: &'static str,
    pub value_type: &'static str,
    pub minimum: Option<u64>,
    pub maximum: Option<u64>,
}

const fn coordination_bool(name: &'static str) -> CoordinationFieldBound {
    CoordinationFieldBound {
        name,
        value_type: "boolean",
        minimum: None,
        maximum: None,
    }
}

const fn coordination_integer(
    name: &'static str,
    minimum: u64,
    maximum: u64,
) -> CoordinationFieldBound {
    CoordinationFieldBound {
        name,
        value_type: "integer",
        minimum: Some(minimum),
        maximum: Some(maximum),
    }
}

static COORDINATION_FIELD_BOUNDS: &[CoordinationFieldBound] = &[
    coordination_bool("enabled"),
    coordination_bool("auto_inject_enabled"),
    coordination_integer("max_active_engine", 1, 4_096),
    coordination_integer("max_active_project", 1, 4_096),
    coordination_integer("max_active_root", 1, 4_096),
    coordination_integer("root_fairness_quantum", 1, 1_024),
    CoordinationFieldBound {
        name: "root_weights",
        value_type: "integer_map",
        minimum: Some(1),
        maximum: Some(1_024),
    },
    coordination_integer("max_root_weight_entries", 1, 1_000_000),
    coordination_integer("max_resume_burst", 1, 1_024),
    coordination_integer("max_pending", 1, 1_000_000),
    coordination_integer("max_outstanding", 1, 4_000_000),
    coordination_integer("max_subchat_depth", 1, 64),
    coordination_integer("max_read_stamps", 64, 1_048_576),
    coordination_integer("max_write_records", 64, 4_194_304),
    coordination_integer("max_undo_entries_per_file", 1, 100_000),
    coordination_integer("max_undo_bytes", 1024, 1_073_741_824),
    coordination_integer("overview_default_rows", 1, 10_000),
    coordination_integer("overview_max_rows", 1, 10_000),
    coordination_integer("max_work_views", 1, 1_000_000),
    coordination_integer("max_work_cursors", 1, 1_000_000),
    coordination_integer("max_work_history_rows", 1, 1_000_000),
    coordination_integer("max_work_history_bytes", 1024, 1_073_741_824),
    coordination_integer("max_work_provenance_records", 1, 1_000_000),
    coordination_integer("max_work_provenance_bytes", 1024, 268_435_456),
    coordination_integer("work_cursor_ttl_secs", 1, 86_400),
    coordination_integer("work_subscription_interval_ms", 100, 60_000),
    coordination_integer("work_stream_max_bytes", 1024, 268_435_456),
    coordination_integer("work_stream_timeout_ms", 30_000, 3_600_000),
    coordination_integer("work_reconnect_max_ms", 100, 60_000),
    coordination_integer("inspect_max_paths", 1, 10_000),
    coordination_integer("capsule_token_budget", 1600, 128_000),
    coordination_integer("capsule_max_rows", 1, 1_000),
    coordination_integer("min_interval_secs", 0, 86_400),
    coordination_integer("debounce_secs", 0, 3_600),
    coordination_integer("recent_window_secs", 60, 2_592_000),
    coordination_integer("lock_timeout_secs", 1, 3_600),
    coordination_integer("lock_poll_interval_ms", 1, 60_000),
    coordination_integer("approval_timeout_secs", 1, 604_800),
    coordination_integer("max_pending_approvals", 1, 100_000),
    coordination_integer("delivery_default_ttl_secs", 1, 2_592_000),
    coordination_integer("delivery_max_ttl_secs", 1, 2_592_000),
    coordination_integer("delivery_max_pending", 1, 1_000_000),
    coordination_integer("delivery_status_retention", 1, 4_000_000),
    coordination_integer("agent_message_max_chars", 1, 4_000_000),
    coordination_integer("peer_grant_default_ttl_secs", 1, 2_592_000),
    coordination_integer("peer_grant_max_ttl_secs", 1, 2_592_000),
    coordination_integer("board_max_reviewers", 1, 100_000),
    coordination_integer("board_claim_key_max_len", 1, 100_000),
    coordination_bool("board_allow_self_review"),
    coordination_integer("agent_wait_default_ms", 1, 3_600_000),
    coordination_integer("agent_wait_max_ms", 1, 86_400_000),
    coordination_integer("agent_wait_max_agents", 1, 10_000),
    coordination_integer("subagent_max_steps", 1, 100_000),
    coordination_integer("subagent_default_steps", 1, 100_000),
    coordination_integer("launch_context_tokens", 1, 1_000_000),
    coordination_integer("launch_context_messages", 1, 100_000),
    coordination_integer("launch_context_entries", 1, 100_000),
    coordination_integer("launch_context_bytes", 1, 1_073_741_824),
    coordination_integer("launch_source_bytes", 1, 1_073_741_824),
    coordination_integer("agent_profile_max_count", 1, 100_000),
    coordination_integer("agent_profile_max_bytes", 1_024, 16_777_216),
];

pub fn coordination_field_bounds() -> &'static [CoordinationFieldBound] {
    COORDINATION_FIELD_BOUNDS
}

/// Dotted names the GUI addresses coordination leaves by, e.g.
/// `coordination.max_active_engine`.
pub const COORDINATION_SETTINGS_KEY: &str = "coordination";

pub fn coordination_field_path(name: &str) -> String {
    format!("{COORDINATION_SETTINGS_KEY}.{name}")
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct TrajectoryRuntimeSettings {
    pub internal_traces_keep_per_folder: usize,
    pub internal_trace_prune_interval_secs: u64,
    pub buddy_conversations_keep: usize,
    pub buddy_conversations_prune_interval_secs: u64,
    pub buddy_conversations_prune_min_age_secs: u64,
    pub session_idle_timeout_secs: u64,
    pub session_cleanup_interval_secs: u64,
    pub stream_idle_timeout_secs: u64,
    pub stream_total_timeout_secs: u64,
    pub max_queue_size: usize,
    pub event_channel_capacity: usize,
    pub recent_request_ids_capacity: usize,
    pub max_parallel_tools: Option<usize>,
    pub max_images_per_message: usize,
    pub max_file_size: usize,
    pub auto_enrichment_total_token_cap: usize,
    pub auto_enrichment_card_token_cap: usize,
    pub auto_enrichment_knowledge_top_n: usize,
    pub auto_enrichment_trajectory_top_n: usize,
    pub trajectory_writer_enabled: bool,
    pub trajectory_index_coordinator_enabled: bool,
    pub trajectory_watcher_self_write_enabled: bool,
    pub tool_catalog_snapshots_enabled: bool,
    pub vecdb_path_coalescing_enabled: bool,
    pub pp_max_tool_budget_tokens: usize,
    pub pp_max_per_file_budget_tokens: usize,
    pub pp_max_line_length_chars: usize,
    pub pp_tokens_for_text_percent: usize,
    pub git_intel_max_commits: usize,
    pub git_intel_deep_walk_limit: usize,
    pub git_intel_max_files_per_commit_cochange: usize,
    pub git_intel_max_files_per_commit_entropy: usize,
    pub codegraph_dead_code_max_results: usize,
    pub codegraph_exec_flow_max_nodes: usize,
    pub vecdb_trajectory_split_bytes: usize,
    pub cat_max_input_paths: usize,
    pub cat_max_lines: usize,
    pub cat_max_file_bytes: usize,
    pub cat_max_expanded_files: usize,
    pub get_logs_max_tail_bytes: usize,
    pub planner_qna_question_limit: usize,
    pub planner_qna_answer_limit: usize,
    pub hist_search_preview_chars: usize,
    pub agent_diff_max_output_bytes: usize,
    pub process_subscribe_preview_bytes: usize,
    pub review_diff_char_cap: usize,
    pub review_max_diff_patch_bytes: usize,
    pub coordination: CoordinationSettings,
}

impl Default for TrajectoryRuntimeSettings {
    fn default() -> Self {
        Self {
            internal_traces_keep_per_folder: INTERNAL_TRACES_KEEP_PER_FOLDER_DEFAULT,
            internal_trace_prune_interval_secs: INTERNAL_TRACE_PRUNE_INTERVAL_SECS_DEFAULT,
            buddy_conversations_keep: BUDDY_CONVERSATIONS_KEEP_DEFAULT,
            buddy_conversations_prune_interval_secs:
                BUDDY_CONVERSATIONS_PRUNE_INTERVAL_SECS_DEFAULT,
            buddy_conversations_prune_min_age_secs: BUDDY_CONVERSATIONS_PRUNE_MIN_AGE_SECS_DEFAULT,
            session_idle_timeout_secs: SESSION_IDLE_TIMEOUT_SECS_DEFAULT,
            session_cleanup_interval_secs: SESSION_CLEANUP_INTERVAL_SECS_DEFAULT,
            stream_idle_timeout_secs: STREAM_IDLE_TIMEOUT_SECS_DEFAULT,
            stream_total_timeout_secs: STREAM_TOTAL_TIMEOUT_SECS_DEFAULT,
            max_queue_size: MAX_QUEUE_SIZE_DEFAULT,
            event_channel_capacity: EVENT_CHANNEL_CAPACITY_DEFAULT,
            recent_request_ids_capacity: RECENT_REQUEST_IDS_CAPACITY_DEFAULT,
            max_parallel_tools: None,
            max_images_per_message: MAX_IMAGES_PER_MESSAGE_DEFAULT,
            max_file_size: MAX_FILE_SIZE_DEFAULT,
            auto_enrichment_total_token_cap: AUTO_ENRICHMENT_TOTAL_TOKEN_CAP_DEFAULT,
            auto_enrichment_card_token_cap: AUTO_ENRICHMENT_CARD_TOKEN_CAP_DEFAULT,
            auto_enrichment_knowledge_top_n: KNOWLEDGE_TOP_N_DEFAULT,
            auto_enrichment_trajectory_top_n: TRAJECTORY_TOP_N_DEFAULT,
            trajectory_writer_enabled: true,
            trajectory_index_coordinator_enabled: true,
            trajectory_watcher_self_write_enabled: true,
            tool_catalog_snapshots_enabled: true,
            vecdb_path_coalescing_enabled: true,
            pp_max_tool_budget_tokens: PP_MAX_TOOL_BUDGET_TOKENS_DEFAULT,
            pp_max_per_file_budget_tokens: PP_MAX_PER_FILE_BUDGET_TOKENS_DEFAULT,
            pp_max_line_length_chars: PP_MAX_LINE_LENGTH_CHARS_DEFAULT,
            pp_tokens_for_text_percent: PP_TOKENS_FOR_TEXT_PERCENT_DEFAULT,
            git_intel_max_commits: GIT_INTEL_MAX_COMMITS_DEFAULT,
            git_intel_deep_walk_limit: GIT_INTEL_DEEP_WALK_LIMIT_DEFAULT,
            git_intel_max_files_per_commit_cochange:
                GIT_INTEL_MAX_FILES_PER_COMMIT_COCHANGE_DEFAULT,
            git_intel_max_files_per_commit_entropy: GIT_INTEL_MAX_FILES_PER_COMMIT_ENTROPY_DEFAULT,
            codegraph_dead_code_max_results: CODEGRAPH_DEAD_CODE_MAX_RESULTS_DEFAULT,
            codegraph_exec_flow_max_nodes: CODEGRAPH_EXEC_FLOW_MAX_NODES_DEFAULT,
            vecdb_trajectory_split_bytes: VECDB_TRAJECTORY_SPLIT_BYTES_DEFAULT,
            cat_max_input_paths: CAT_MAX_INPUT_PATHS_DEFAULT,
            cat_max_lines: CAT_MAX_LINES_DEFAULT,
            cat_max_file_bytes: CAT_MAX_FILE_BYTES_DEFAULT,
            cat_max_expanded_files: CAT_MAX_EXPANDED_FILES_DEFAULT,
            get_logs_max_tail_bytes: GET_LOGS_MAX_TAIL_BYTES_DEFAULT,
            planner_qna_question_limit: PLANNER_QNA_QUESTION_LIMIT_DEFAULT,
            planner_qna_answer_limit: PLANNER_QNA_ANSWER_LIMIT_DEFAULT,
            hist_search_preview_chars: HIST_SEARCH_PREVIEW_CHARS_DEFAULT,
            agent_diff_max_output_bytes: AGENT_DIFF_MAX_OUTPUT_BYTES_DEFAULT,
            process_subscribe_preview_bytes: PROCESS_SUBSCRIBE_PREVIEW_BYTES_DEFAULT,
            review_diff_char_cap: REVIEW_DIFF_CHAR_CAP_DEFAULT,
            review_max_diff_patch_bytes: REVIEW_MAX_DIFF_PATCH_BYTES_DEFAULT,
            coordination: CoordinationSettings::default(),
        }
    }
}

static ACTIVE_SETTINGS: LazyLock<RwLock<TrajectoryRuntimeSettings>> =
    LazyLock::new(|| RwLock::new(TrajectoryRuntimeSettings::default()));

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join("trajectory-settings.yaml")
}

pub async fn load_from_path(path: &Path) -> Result<TrajectoryRuntimeSettings, String> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => serde_yaml::from_str(&contents)
            .map_err(|error| format!("invalid trajectory settings in {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(TrajectoryRuntimeSettings::default())
        }
        Err(error) => Err(format!(
            "cannot read trajectory settings {}: {error}",
            path.display()
        )),
    }
}

pub fn current() -> TrajectoryRuntimeSettings {
    ACTIVE_SETTINGS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Push the live crate-backed limits into every consuming crate.
///
/// Called from both `install_startup` and `install_live`: none of these limits
/// size a boot-time allocation, so all of them apply immediately.
fn install_crate_limits(settings: &TrajectoryRuntimeSettings) {
    refact_postprocessing::config::install_postprocessing_limits(
        refact_postprocessing::config::PostprocessingLimits {
            max_tool_budget_tokens: settings.pp_max_tool_budget_tokens,
            max_per_file_budget_tokens: settings.pp_max_per_file_budget_tokens,
            max_line_length_chars: settings.pp_max_line_length_chars,
            tokens_for_text_percent: settings.pp_tokens_for_text_percent,
        },
    );
    refact_git_intel::config::install_git_intel_limits(refact_git_intel::config::GitIntelLimits {
        max_commits: settings.git_intel_max_commits,
        deep_walk_limit: settings.git_intel_deep_walk_limit,
        max_files_per_commit_for_cochange: settings.git_intel_max_files_per_commit_cochange,
        max_files_per_commit_for_entropy: settings.git_intel_max_files_per_commit_entropy,
    });
    refact_codegraph::config::install_codegraph_limits(refact_codegraph::config::CodegraphLimits {
        dead_code_max_results: settings.codegraph_dead_code_max_results,
        exec_flow_max_nodes: settings.codegraph_exec_flow_max_nodes,
    });
    refact_vecdb::vdb_thread::install_vecdb_trajectory_split_bytes(
        settings.vecdb_trajectory_split_bytes,
    );
    if let Err(error) =
        refact_work_core::install_coordination_settings(settings.coordination.clone())
    {
        tracing::warn!("keeping previous coordination settings: {error}");
    }
}

pub fn install_startup(settings: TrajectoryRuntimeSettings) {
    install_crate_limits(&settings);
    refact_vecdb::vdb_thread::install_vecdb_path_coalescing_setting(
        settings.vecdb_path_coalescing_enabled,
    );
    refact_chat_history::config::install_runtime_config(refact_chat_history::config::ChatConfig {
        limits: refact_chat_history::config::ChatLimits {
            max_queue_size: settings.max_queue_size,
            event_channel_capacity: settings.event_channel_capacity,
            recent_request_ids_capacity: settings.recent_request_ids_capacity,
            max_images_per_message: settings.max_images_per_message,
            max_parallel_tools: settings.max_parallel_tools.unwrap_or(usize::MAX),
            max_file_size: settings.max_file_size,
        },
        ..Default::default()
    });
    refact_chat_api::install_runtime_timeouts(refact_chat_api::RuntimeChatTimeouts {
        max_queue_size: settings.max_queue_size,
        session_idle: Duration::from_secs(settings.session_idle_timeout_secs),
        session_cleanup_interval: Duration::from_secs(settings.session_cleanup_interval_secs),
        stream_idle: Duration::from_secs(settings.stream_idle_timeout_secs),
        stream_total: Duration::from_secs(settings.stream_total_timeout_secs),
    });
    *ACTIVE_SETTINGS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = settings;
}

pub fn install_live(settings: &TrajectoryRuntimeSettings) {
    let active_before = current();
    install_crate_limits(settings);
    refact_chat_history::config::apply_live_limits(refact_chat_history::config::ChatLimits {
        max_queue_size: settings.max_queue_size,
        event_channel_capacity: active_before.event_channel_capacity,
        recent_request_ids_capacity: settings.recent_request_ids_capacity,
        max_images_per_message: settings.max_images_per_message,
        max_parallel_tools: settings.max_parallel_tools.unwrap_or(usize::MAX),
        max_file_size: settings.max_file_size,
    });
    refact_chat_api::install_runtime_timeouts(refact_chat_api::RuntimeChatTimeouts {
        max_queue_size: settings.max_queue_size,
        session_idle: Duration::from_secs(settings.session_idle_timeout_secs),
        session_cleanup_interval: Duration::from_secs(settings.session_cleanup_interval_secs),
        stream_idle: Duration::from_secs(settings.stream_idle_timeout_secs),
        stream_total: Duration::from_secs(settings.stream_total_timeout_secs),
    });
    let mut active = ACTIVE_SETTINGS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let restart_required = (
        active.event_channel_capacity,
        active.trajectory_writer_enabled,
        active.trajectory_index_coordinator_enabled,
        active.tool_catalog_snapshots_enabled,
        active.vecdb_path_coalescing_enabled,
    );
    *active = settings.clone();
    active.event_channel_capacity = restart_required.0;
    active.trajectory_writer_enabled = restart_required.1;
    active.trajectory_index_coordinator_enabled = restart_required.2;
    active.tool_catalog_snapshots_enabled = restart_required.3;
    active.vecdb_path_coalescing_enabled = restart_required.4;
}

#[cfg(test)]
pub fn reset_for_test() {
    install_startup(TrajectoryRuntimeSettings::default());
}

pub fn validate(settings: &TrajectoryRuntimeSettings) -> Result<(), String> {
    validate_usize(
        "internal_traces_keep_per_folder",
        settings.internal_traces_keep_per_folder,
        10,
        10_000,
    )?;
    validate_u64(
        "internal_trace_prune_interval_secs",
        settings.internal_trace_prune_interval_secs,
        60,
        86_400,
    )?;
    validate_usize(
        "buddy_conversations_keep",
        settings.buddy_conversations_keep,
        10,
        100_000,
    )?;
    validate_u64(
        "buddy_conversations_prune_interval_secs",
        settings.buddy_conversations_prune_interval_secs,
        60,
        86_400,
    )?;
    validate_u64(
        "buddy_conversations_prune_min_age_secs",
        settings.buddy_conversations_prune_min_age_secs,
        60,
        365 * 86_400,
    )?;
    validate_u64(
        "session_idle_timeout_secs",
        settings.session_idle_timeout_secs,
        60,
        86_400,
    )?;
    validate_u64(
        "session_cleanup_interval_secs",
        settings.session_cleanup_interval_secs,
        10,
        86_400,
    )?;
    validate_u64(
        "stream_idle_timeout_secs",
        settings.stream_idle_timeout_secs,
        10,
        86_400,
    )?;
    validate_u64(
        "stream_total_timeout_secs",
        settings.stream_total_timeout_secs,
        60,
        172_800,
    )?;
    validate_usize("max_queue_size", settings.max_queue_size, 1, 10_000)?;
    validate_usize(
        "event_channel_capacity",
        settings.event_channel_capacity,
        16,
        1_000_000,
    )?;
    validate_usize(
        "recent_request_ids_capacity",
        settings.recent_request_ids_capacity,
        1,
        100_000,
    )?;
    if let Some(value) = settings.max_parallel_tools {
        validate_usize("max_parallel_tools", value, 1, 10_000)?;
    }
    validate_usize(
        "max_images_per_message",
        settings.max_images_per_message,
        1,
        1_000,
    )?;
    validate_usize("max_file_size", settings.max_file_size, 1_024, 50_000_000)?;
    validate_usize(
        "auto_enrichment_total_token_cap",
        settings.auto_enrichment_total_token_cap,
        64,
        32_000,
    )?;
    validate_usize(
        "auto_enrichment_card_token_cap",
        settings.auto_enrichment_card_token_cap,
        32,
        16_000,
    )?;
    if settings.auto_enrichment_card_token_cap > settings.auto_enrichment_total_token_cap {
        return Err(
            "auto_enrichment_card_token_cap must not exceed auto_enrichment_total_token_cap"
                .to_string(),
        );
    }
    validate_usize(
        "auto_enrichment_knowledge_top_n",
        settings.auto_enrichment_knowledge_top_n,
        1,
        20,
    )?;
    validate_usize(
        "auto_enrichment_trajectory_top_n",
        settings.auto_enrichment_trajectory_top_n,
        1,
        20,
    )?;
    validate_usize(
        "pp_max_tool_budget_tokens",
        settings.pp_max_tool_budget_tokens,
        4_096,
        2_000_000,
    )?;
    validate_usize(
        "pp_max_per_file_budget_tokens",
        settings.pp_max_per_file_budget_tokens,
        1_024,
        2_000_000,
    )?;
    validate_usize(
        "pp_max_line_length_chars",
        settings.pp_max_line_length_chars,
        80,
        1_000_000,
    )?;
    validate_usize(
        "pp_tokens_for_text_percent",
        settings.pp_tokens_for_text_percent,
        1,
        100,
    )?;
    validate_usize(
        "git_intel_max_commits",
        settings.git_intel_max_commits,
        100,
        1_000_000,
    )?;
    validate_usize(
        "git_intel_deep_walk_limit",
        settings.git_intel_deep_walk_limit,
        100,
        5_000_000,
    )?;
    validate_usize(
        "git_intel_max_files_per_commit_cochange",
        settings.git_intel_max_files_per_commit_cochange,
        10,
        100_000,
    )?;
    validate_usize(
        "git_intel_max_files_per_commit_entropy",
        settings.git_intel_max_files_per_commit_entropy,
        5,
        100_000,
    )?;
    validate_usize(
        "codegraph_dead_code_max_results",
        settings.codegraph_dead_code_max_results,
        50,
        1_000_000,
    )?;
    validate_usize(
        "codegraph_exec_flow_max_nodes",
        settings.codegraph_exec_flow_max_nodes,
        50,
        1_000_000,
    )?;
    validate_usize(
        "vecdb_trajectory_split_bytes",
        settings.vecdb_trajectory_split_bytes,
        256,
        1_048_576,
    )?;
    validate_usize(
        "cat_max_input_paths",
        settings.cat_max_input_paths,
        1,
        100_000,
    )?;
    validate_usize("cat_max_lines", settings.cat_max_lines, 100, 1_000_000)?;
    validate_usize(
        "cat_max_file_bytes",
        settings.cat_max_file_bytes,
        4_096,
        268_435_456,
    )?;
    validate_usize(
        "cat_max_expanded_files",
        settings.cat_max_expanded_files,
        1,
        100_000,
    )?;
    validate_usize(
        "get_logs_max_tail_bytes",
        settings.get_logs_max_tail_bytes,
        4_096,
        268_435_456,
    )?;
    validate_usize(
        "planner_qna_question_limit",
        settings.planner_qna_question_limit,
        200,
        1_000_000,
    )?;
    validate_usize(
        "planner_qna_answer_limit",
        settings.planner_qna_answer_limit,
        200,
        1_000_000,
    )?;
    validate_usize(
        "hist_search_preview_chars",
        settings.hist_search_preview_chars,
        100,
        100_000,
    )?;
    validate_usize(
        "agent_diff_max_output_bytes",
        settings.agent_diff_max_output_bytes,
        4_096,
        268_435_456,
    )?;
    validate_usize(
        "process_subscribe_preview_bytes",
        settings.process_subscribe_preview_bytes,
        50,
        100_000,
    )?;
    validate_usize(
        "review_diff_char_cap",
        settings.review_diff_char_cap,
        4_096,
        8_388_608,
    )?;
    validate_usize(
        "review_max_diff_patch_bytes",
        settings.review_max_diff_patch_bytes,
        4_096,
        268_435_456,
    )?;
    validate_coordination(&settings.coordination)
}

/// Range-check every coordination leaf against the published bounds table, then
/// hand the value to the crate for the invariants only it can express.
pub fn validate_coordination(coordination: &CoordinationSettings) -> Result<(), String> {
    let rendered = serde_json::to_value(coordination)
        .map_err(|error| format!("cannot inspect coordination settings: {error}"))?;
    let object = rendered
        .as_object()
        .ok_or_else(|| "coordination settings must serialize as an object".to_string())?;
    for bound in coordination_field_bounds() {
        let value = object.get(bound.name).ok_or_else(|| {
            format!(
                "{} is bounded but absent from the coordination settings",
                coordination_field_path(bound.name)
            )
        })?;
        if bound.value_type == "integer_map" {
            validate_coordination_integer_map(bound, value)?;
            continue;
        }
        let (minimum, maximum) = match (bound.minimum, bound.maximum) {
            (Some(minimum), Some(maximum)) => (minimum, maximum),
            _ => {
                if !value.is_boolean() {
                    return Err(format!(
                        "{} must be true or false",
                        coordination_field_path(bound.name)
                    ));
                }
                continue;
            }
        };
        let number = value.as_u64().ok_or_else(|| {
            format!(
                "{} must be a whole number",
                coordination_field_path(bound.name)
            )
        })?;
        validate_u64(
            &coordination_field_path(bound.name),
            number,
            minimum,
            maximum,
        )?;
    }
    let bounded: std::collections::BTreeSet<&str> = coordination_field_bounds()
        .iter()
        .map(|bound| bound.name)
        .collect();
    if let Some(missing) = object.keys().find(|name| !bounded.contains(name.as_str())) {
        return Err(format!(
            "{} has no published range; add it to the coordination bounds table",
            coordination_field_path(missing)
        ));
    }
    coordination
        .validate()
        .map_err(|error| format!("coordination: {error}"))
}

/// A sparse per-project override: only the coordination leaves the project
/// actually pins are present, everything else stays inherited.
pub type CoordinationOverride = std::collections::BTreeMap<String, serde_json::Value>;

fn validate_coordination_integer_map(
    bound: &CoordinationFieldBound,
    value: &serde_json::Value,
) -> Result<(), String> {
    let map = value.as_object().ok_or_else(|| {
        format!(
            "{} must map root IDs to whole numbers",
            coordination_field_path(bound.name)
        )
    })?;
    for number in map.values() {
        let number = number.as_u64().ok_or_else(|| {
            format!(
                "{} weights must be whole numbers",
                coordination_field_path(bound.name)
            )
        })?;
        validate_u64(
            &coordination_field_path(bound.name),
            number,
            bound.minimum.unwrap_or(1),
            bound.maximum.unwrap_or(u64::MAX),
        )?;
    }
    Ok(())
}

/// Where a resolved coordination value came from.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationValueSource {
    Global,
    Project,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CoordinationEffectiveField {
    pub name: String,
    pub value: serde_json::Value,
    pub global_value: serde_json::Value,
    pub source: CoordinationValueSource,
    pub inherited: bool,
}

pub fn project_coordination_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact").join("coordination.yaml")
}

pub async fn coordination_for_project(project_root: &Path) -> Result<CoordinationSettings, String> {
    let overrides = load_project_coordination_override(&project_coordination_path(project_root))
        .await
        .map_err(|_| "project coordination settings unavailable")?;
    resolve_coordination(&current().coordination, &overrides)
        .map_err(|_| "project coordination settings invalid".to_string())
}

pub async fn load_project_coordination_override(
    path: &Path,
) -> Result<CoordinationOverride, String> {
    let raw = match tokio::fs::read_to_string(path).await {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CoordinationOverride::new())
        }
        Err(error) => {
            return Err(format!(
                "cannot read coordination overrides {}: {error}",
                path.display()
            ))
        }
    };
    if raw.trim().is_empty() {
        return Ok(CoordinationOverride::new());
    }
    let parsed: CoordinationOverride = serde_yaml::from_str(&raw).map_err(|error| {
        format!(
            "invalid coordination overrides in {}: {error}",
            path.display()
        )
    })?;
    normalize_coordination_override(parsed)
}

/// Accept only published coordination leaves, each already inside its range, so
/// a project file can never widen a bound the global surface enforces.
pub fn normalize_coordination_override(
    raw: CoordinationOverride,
) -> Result<CoordinationOverride, String> {
    let mut normalized = CoordinationOverride::new();
    for (name, value) in raw {
        let bound = coordination_field_bounds()
            .iter()
            .find(|bound| bound.name == name)
            .ok_or_else(|| format!("unknown coordination setting '{name}'"))?;
        if bound.value_type == "integer_map" {
            validate_coordination_integer_map(bound, &value)?;
            normalized.insert(name, value);
            continue;
        }
        match (bound.minimum, bound.maximum) {
            (Some(minimum), Some(maximum)) => {
                let number = value.as_u64().ok_or_else(|| {
                    format!(
                        "{} must be a whole number",
                        coordination_field_path(bound.name)
                    )
                })?;
                validate_u64(
                    &coordination_field_path(bound.name),
                    number,
                    minimum,
                    maximum,
                )?;
            }
            _ => {
                if !value.is_boolean() {
                    return Err(format!(
                        "{} must be true or false",
                        coordination_field_path(bound.name)
                    ));
                }
            }
        }
        normalized.insert(name, value);
    }
    Ok(normalized)
}

/// Merge a sparse project override onto the global coordination settings.
pub fn resolve_coordination(
    global: &CoordinationSettings,
    overrides: &CoordinationOverride,
) -> Result<CoordinationSettings, String> {
    let overrides = normalize_coordination_override(overrides.clone())?;
    let mut rendered = serde_json::to_value(global)
        .map_err(|error| format!("cannot inspect coordination settings: {error}"))?;
    let object = rendered
        .as_object_mut()
        .ok_or_else(|| "coordination settings must serialize as an object".to_string())?;
    for (name, value) in overrides {
        object.insert(name, value);
    }
    let resolved: CoordinationSettings = serde_json::from_value(rendered)
        .map_err(|error| format!("invalid coordination overrides: {error}"))?;
    validate_coordination(&resolved)?;
    Ok(resolved)
}

/// The resolved view the Settings surface renders: every leaf, its effective
/// value, the inherited global value, and whether the project pinned it.
pub fn coordination_effective_view(
    global: &CoordinationSettings,
    overrides: &CoordinationOverride,
) -> Result<Vec<CoordinationEffectiveField>, String> {
    let resolved = resolve_coordination(global, overrides)?;
    let resolved_json = serde_json::to_value(&resolved)
        .map_err(|error| format!("cannot inspect coordination settings: {error}"))?;
    let global_json = serde_json::to_value(global)
        .map_err(|error| format!("cannot inspect coordination settings: {error}"))?;
    Ok(coordination_field_bounds()
        .iter()
        .map(|bound| {
            let overridden = overrides.contains_key(bound.name);
            CoordinationEffectiveField {
                name: bound.name.to_string(),
                value: resolved_json
                    .get(bound.name)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
                global_value: global_json
                    .get(bound.name)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
                source: if overridden {
                    CoordinationValueSource::Project
                } else {
                    CoordinationValueSource::Global
                },
                inherited: !overridden,
            }
        })
        .collect())
}

fn validate_usize(name: &str, value: usize, minimum: usize, maximum: usize) -> Result<(), String> {
    if !(minimum..=maximum).contains(&value) {
        return Err(format!(
            "{name} must be between {minimum} and {maximum}; got {value}"
        ));
    }
    Ok(())
}

fn validate_u64(name: &str, value: u64, minimum: u64, maximum: u64) -> Result<(), String> {
    if !(minimum..=maximum).contains(&value) {
        return Err(format!(
            "{name} must be between {minimum} and {maximum}; got {value}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Every crate-backed limit, tweaked away from its default so a stale
    /// installer cannot pass by accident.
    fn tweaked() -> TrajectoryRuntimeSettings {
        TrajectoryRuntimeSettings {
            pp_max_tool_budget_tokens: 262_144,
            pp_max_per_file_budget_tokens: 65_536,
            pp_max_line_length_chars: 4_096,
            pp_tokens_for_text_percent: 42,
            git_intel_max_commits: 1_234,
            git_intel_deep_walk_limit: 23_456,
            git_intel_max_files_per_commit_cochange: 321,
            git_intel_max_files_per_commit_entropy: 77,
            codegraph_dead_code_max_results: 654,
            codegraph_exec_flow_max_nodes: 987,
            vecdb_trajectory_split_bytes: 4_096,
            coordination: CoordinationSettings {
                max_active_engine: 21,
                max_pending: 512,
                lock_timeout_secs: 17,
                ..CoordinationSettings::default()
            },
            ..TrajectoryRuntimeSettings::default()
        }
    }

    fn assert_crate_limits_match(settings: &TrajectoryRuntimeSettings) {
        let pp = refact_postprocessing::config::limits();
        assert_eq!(
            pp.max_tool_budget_tokens,
            settings.pp_max_tool_budget_tokens
        );
        assert_eq!(
            pp.max_per_file_budget_tokens,
            settings.pp_max_per_file_budget_tokens
        );
        assert_eq!(pp.max_line_length_chars, settings.pp_max_line_length_chars);
        assert_eq!(
            pp.tokens_for_text_percent,
            settings.pp_tokens_for_text_percent
        );

        let git = refact_git_intel::config::limits();
        assert_eq!(git.max_commits, settings.git_intel_max_commits);
        assert_eq!(git.deep_walk_limit, settings.git_intel_deep_walk_limit);
        assert_eq!(
            git.max_files_per_commit_for_cochange,
            settings.git_intel_max_files_per_commit_cochange
        );
        assert_eq!(
            git.max_files_per_commit_for_entropy,
            settings.git_intel_max_files_per_commit_entropy
        );

        let cg = refact_codegraph::config::limits();
        assert_eq!(
            cg.dead_code_max_results,
            settings.codegraph_dead_code_max_results
        );
        assert_eq!(
            cg.exec_flow_max_nodes,
            settings.codegraph_exec_flow_max_nodes
        );
    }

    #[test]
    #[serial(runtime_settings)]
    fn install_startup_propagates_every_crate_backed_limit() {
        let settings = tweaked();
        assert!(validate(&settings).is_ok());
        install_startup(settings.clone());
        assert_crate_limits_match(&settings);
        assert_eq!(
            refact_vecdb::vdb_thread::vecdb_trajectory_split_bytes(),
            settings.vecdb_trajectory_split_bytes
        );
        assert_eq!(
            current().pp_max_tool_budget_tokens,
            settings.pp_max_tool_budget_tokens
        );
        reset_for_test();
    }

    #[test]
    #[serial(runtime_settings)]
    fn install_live_propagates_every_crate_backed_limit_without_restart() {
        reset_for_test();
        let settings = tweaked();
        install_live(&settings);
        assert_crate_limits_match(&settings);
        assert_eq!(
            refact_vecdb::vdb_thread::vecdb_trajectory_split_bytes(),
            settings.vecdb_trajectory_split_bytes
        );
        // None of the new limits are carved out as restart-required, so the
        // active snapshot must reflect them immediately.
        assert_eq!(current(), settings);
        reset_for_test();
    }

    #[test]
    fn old_yaml_without_new_keys_loads_with_new_defaults() {
        let legacy = "\
internal_traces_keep_per_folder: 25
session_idle_timeout_secs: 120
max_queue_size: 7
trajectory_writer_enabled: false
";
        let loaded: TrajectoryRuntimeSettings = serde_yaml::from_str(legacy).unwrap();
        let defaults = TrajectoryRuntimeSettings::default();
        assert_eq!(loaded.internal_traces_keep_per_folder, 25);
        assert_eq!(loaded.session_idle_timeout_secs, 120);
        assert_eq!(loaded.max_queue_size, 7);
        assert!(!loaded.trajectory_writer_enabled);
        assert_eq!(
            loaded.pp_max_tool_budget_tokens,
            defaults.pp_max_tool_budget_tokens
        );
        assert_eq!(loaded.git_intel_max_commits, defaults.git_intel_max_commits);
        assert_eq!(
            loaded.codegraph_exec_flow_max_nodes,
            defaults.codegraph_exec_flow_max_nodes
        );
        assert_eq!(
            loaded.vecdb_trajectory_split_bytes,
            defaults.vecdb_trajectory_split_bytes
        );
        assert_eq!(loaded.review_diff_char_cap, defaults.review_diff_char_cap);
        assert_eq!(loaded.cat_max_input_paths, defaults.cat_max_input_paths);
        assert!(validate(&loaded).is_ok());
    }

    #[test]
    fn coordination_defaults_are_generous_and_survive_a_yaml_round_trip() {
        let defaults = TrajectoryRuntimeSettings::default().coordination;
        assert!(defaults.enabled);
        assert!(defaults.auto_inject_enabled);
        assert_eq!(defaults.max_active_engine, 32);
        assert_eq!(defaults.max_active_project, 16);
        assert_eq!(defaults.max_active_root, 16);
        assert_eq!(defaults.max_pending, 1_024);
        assert_eq!(defaults.max_outstanding, 4_096);
        assert_eq!(defaults.max_subchat_depth, 5);
        assert_eq!(defaults.max_read_stamps, 16_384);
        assert_eq!(defaults.max_write_records, 65_536);
        assert_eq!(defaults.overview_default_rows, 50);
        assert_eq!(defaults.overview_max_rows, 500);
        assert_eq!(defaults.capsule_token_budget, 1_600);
        assert_eq!(defaults.capsule_max_rows, 24);
        assert_eq!(defaults.min_interval_secs, 60);
        assert_eq!(defaults.debounce_secs, 5);
        assert_eq!(defaults.recent_window_secs, 3_600);
        assert_eq!(defaults.lock_timeout_secs, 30);
        assert_eq!(defaults.lock_poll_interval_ms, 25);

        let yaml = serde_yaml::to_string(&tweaked()).unwrap();
        let reloaded: TrajectoryRuntimeSettings = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(reloaded.coordination, tweaked().coordination);
    }

    #[test]
    fn yaml_without_a_coordination_block_keeps_every_coordination_default() {
        let legacy = "max_queue_size: 7\n";
        let loaded: TrajectoryRuntimeSettings = serde_yaml::from_str(legacy).unwrap();
        assert_eq!(loaded.max_queue_size, 7);
        assert_eq!(
            loaded.coordination,
            TrajectoryRuntimeSettings::default().coordination
        );
        assert!(validate(&loaded).is_ok());
    }

    #[test]
    fn a_partial_coordination_block_fills_the_rest_from_defaults() {
        let partial = "coordination:\n  max_active_engine: 48\n  debounce_secs: 3\n";
        let loaded: TrajectoryRuntimeSettings = serde_yaml::from_str(partial).unwrap();
        let defaults = TrajectoryRuntimeSettings::default().coordination;
        assert_eq!(loaded.coordination.max_active_engine, 48);
        assert_eq!(loaded.coordination.debounce_secs, 3);
        assert_eq!(loaded.coordination.max_pending, defaults.max_pending);
        assert_eq!(
            loaded.coordination.lock_poll_interval_ms,
            defaults.lock_poll_interval_ms
        );
        assert!(validate(&loaded).is_ok());
    }

    #[test]
    fn the_bounds_table_covers_exactly_the_serialized_coordination_leaves() {
        let rendered = serde_json::to_value(CoordinationSettings::default()).unwrap();
        let serialized: std::collections::BTreeSet<String> =
            rendered.as_object().unwrap().keys().cloned().collect();
        let bounded: std::collections::BTreeSet<String> = coordination_field_bounds()
            .iter()
            .map(|bound| bound.name.to_string())
            .collect();
        assert_eq!(bounded, serialized);
        for bound in coordination_field_bounds() {
            match (bound.minimum, bound.maximum) {
                (Some(minimum), Some(maximum)) => {
                    assert!(matches!(bound.value_type, "integer" | "integer_map"));
                    assert!(minimum <= maximum, "{} has an empty range", bound.name);
                }
                _ => assert_eq!(bound.value_type, "boolean"),
            }
        }
    }

    #[test]
    fn every_default_sits_inside_its_published_range() {
        let rendered = serde_json::to_value(CoordinationSettings::default()).unwrap();
        for bound in coordination_field_bounds() {
            if bound.value_type == "integer_map" {
                validate_coordination_integer_map(bound, &rendered[bound.name]).unwrap();
                continue;
            }
            let (minimum, maximum) = match (bound.minimum, bound.maximum) {
                (Some(minimum), Some(maximum)) => (minimum, maximum),
                _ => continue,
            };
            let value = rendered[bound.name].as_u64().unwrap();
            assert!(
                (minimum..=maximum).contains(&value),
                "{} default {value} is outside {minimum}..={maximum}",
                bound.name
            );
        }
    }

    #[test]
    fn out_of_range_and_invariant_violating_coordination_values_are_rejected() {
        let mut settings = TrajectoryRuntimeSettings::default();
        settings.coordination.max_subchat_depth = 0;
        let error = validate(&settings).unwrap_err();
        assert!(
            error.contains("coordination.max_subchat_depth"),
            "unexpected error: {error}"
        );

        let mut relational = TrajectoryRuntimeSettings::default();
        relational.coordination.max_active_project = 64;
        relational.coordination.max_active_engine = 32;
        let error = validate(&relational).unwrap_err();
        assert!(
            error.contains("max_active_project"),
            "unexpected error: {error}"
        );

        let mut too_big = TrajectoryRuntimeSettings::default();
        too_big.coordination.capsule_max_rows = 100_000;
        assert!(validate(&too_big).is_err());
    }

    #[test]
    fn a_sparse_project_override_resolves_over_the_global_settings() {
        let global = CoordinationSettings {
            max_active_engine: 32,
            capsule_max_rows: 24,
            ..CoordinationSettings::default()
        };
        let mut overrides = CoordinationOverride::new();
        overrides.insert("capsule_max_rows".to_string(), serde_json::json!(8));
        overrides.insert("auto_inject_enabled".to_string(), serde_json::json!(false));

        let resolved = resolve_coordination(&global, &overrides).unwrap();
        assert_eq!(resolved.capsule_max_rows, 8);
        assert!(!resolved.auto_inject_enabled);
        assert_eq!(resolved.max_active_engine, global.max_active_engine);
        assert_eq!(resolved.max_pending, global.max_pending);

        let view = coordination_effective_view(&global, &overrides).unwrap();
        let capsule = view
            .iter()
            .find(|field| field.name == "capsule_max_rows")
            .unwrap();
        assert_eq!(capsule.value, serde_json::json!(8));
        assert_eq!(capsule.global_value, serde_json::json!(24));
        assert_eq!(capsule.source, CoordinationValueSource::Project);
        assert!(!capsule.inherited);

        let engine = view
            .iter()
            .find(|field| field.name == "max_active_engine")
            .unwrap();
        assert_eq!(engine.source, CoordinationValueSource::Global);
        assert!(engine.inherited);
        assert_eq!(view.len(), coordination_field_bounds().len());
    }

    #[test]
    fn an_empty_project_override_inherits_every_global_value() {
        let global = CoordinationSettings {
            max_pending: 77,
            ..CoordinationSettings::default()
        };
        let overrides = CoordinationOverride::new();
        assert_eq!(resolve_coordination(&global, &overrides).unwrap(), global);
        assert!(coordination_effective_view(&global, &overrides)
            .unwrap()
            .iter()
            .all(|field| field.inherited));
    }

    #[test]
    fn project_overrides_cannot_introduce_unknown_or_out_of_range_values() {
        let mut unknown = CoordinationOverride::new();
        unknown.insert("not_a_setting".to_string(), serde_json::json!(1));
        let error = normalize_coordination_override(unknown).unwrap_err();
        assert!(error.contains("not_a_setting"), "unexpected error: {error}");

        let mut widened = CoordinationOverride::new();
        widened.insert(
            "max_active_engine".to_string(),
            serde_json::json!(1_000_000),
        );
        let error = normalize_coordination_override(widened).unwrap_err();
        assert!(
            error.contains("coordination.max_active_engine"),
            "unexpected error: {error}"
        );

        let mut wrong_type = CoordinationOverride::new();
        wrong_type.insert("enabled".to_string(), serde_json::json!(3));
        assert!(normalize_coordination_override(wrong_type).is_err());

        let mut breaks_invariant = CoordinationOverride::new();
        breaks_invariant.insert("overview_default_rows".to_string(), serde_json::json!(900));
        let error =
            resolve_coordination(&CoordinationSettings::default(), &breaks_invariant).unwrap_err();
        assert!(
            error.contains("overview_default_rows"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn root_weights_are_validated_in_global_and_sparse_project_settings() {
        let mut settings = CoordinationSettings::default();
        settings.root_weights.insert("root".into(), 2);
        validate_coordination(&settings).unwrap();
        let overrides =
            CoordinationOverride::from([("root_weights".into(), serde_json::json!({"other": 3}))]);
        let resolved = resolve_coordination(&settings, &overrides).unwrap();
        assert_eq!(resolved.root_quantum("other"), 3);
        assert_eq!(resolved.root_quantum("root"), 1);
        for value in [
            serde_json::json!({"root": 0}),
            serde_json::json!({"root": 1025}),
            serde_json::json!([]),
            serde_json::json!({"root": "2"}),
        ] {
            assert!(
                normalize_coordination_override(CoordinationOverride::from([(
                    "root_weights".into(),
                    value
                )]))
                .is_err()
            );
        }
    }

    #[tokio::test]
    async fn a_missing_or_empty_project_override_file_reads_as_no_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let path = project_coordination_path(dir.path());
        assert!(load_project_coordination_override(&path)
            .await
            .unwrap()
            .is_empty());

        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, "").await.unwrap();
        assert!(load_project_coordination_override(&path)
            .await
            .unwrap()
            .is_empty());

        tokio::fs::write(&path, "capsule_max_rows: 8\n")
            .await
            .unwrap();
        let loaded = load_project_coordination_override(&path).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["capsule_max_rows"], serde_json::json!(8));

        tokio::fs::write(&path, "capsule_max_rows: 100000\n")
            .await
            .unwrap();
        assert!(load_project_coordination_override(&path).await.is_err());
    }

    #[test]
    #[serial(runtime_settings)]
    fn coordination_settings_reach_the_crate_snapshot_live() {
        reset_for_test();
        assert_eq!(
            refact_work_core::coordination_settings(),
            CoordinationSettings::default()
        );
        let settings = tweaked();
        install_live(&settings);
        assert_eq!(
            refact_work_core::coordination_settings(),
            settings.coordination
        );
        assert_eq!(current().coordination, settings.coordination);
        reset_for_test();
        assert_eq!(
            refact_work_core::coordination_settings(),
            CoordinationSettings::default()
        );
    }

    #[test]
    fn empty_yaml_yields_defaults_and_full_round_trip_is_stable() {
        let empty: TrajectoryRuntimeSettings = serde_yaml::from_str("{}").unwrap();
        assert_eq!(empty, TrajectoryRuntimeSettings::default());
        let yaml = serde_yaml::to_string(&tweaked()).unwrap();
        assert_eq!(
            serde_yaml::from_str::<TrajectoryRuntimeSettings>(&yaml).unwrap(),
            tweaked()
        );
    }
}
