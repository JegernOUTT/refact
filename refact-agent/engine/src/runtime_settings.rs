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
            trajectory_writer_enabled: false,
            trajectory_index_coordinator_enabled: false,
            trajectory_watcher_self_write_enabled: false,
            tool_catalog_snapshots_enabled: false,
            vecdb_path_coalescing_enabled: false,
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

pub fn install_startup(settings: TrajectoryRuntimeSettings) {
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
        active.trajectory_watcher_self_write_enabled,
        active.tool_catalog_snapshots_enabled,
        active.vecdb_path_coalescing_enabled,
    );
    *active = settings.clone();
    active.event_channel_capacity = restart_required.0;
    active.trajectory_writer_enabled = restart_required.1;
    active.trajectory_index_coordinator_enabled = restart_required.2;
    active.trajectory_watcher_self_write_enabled = restart_required.3;
    active.tool_catalog_snapshots_enabled = restart_required.4;
    active.vecdb_path_coalescing_enabled = restart_required.5;
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
