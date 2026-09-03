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
    )
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
