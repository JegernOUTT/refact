use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ChatLimits {
    pub max_queue_size: usize,
    pub event_channel_capacity: usize,
    pub recent_request_ids_capacity: usize,
    pub max_images_per_message: usize,
    pub max_parallel_tools: usize,
    pub max_file_size: usize,
}

impl Default for ChatLimits {
    fn default() -> Self {
        Self {
            max_queue_size: 100,
            event_channel_capacity: 4096,
            recent_request_ids_capacity: 100,
            max_images_per_message: 50,
            max_parallel_tools: usize::MAX,
            max_file_size: 40_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatTimeouts {
    pub session_idle: Duration,
    pub session_cleanup_interval: Duration,
    pub stream_idle: Duration,
    pub stream_total: Duration,
    pub stream_heartbeat: Duration,
    pub watcher_debounce: Duration,
    pub watcher_idle: Duration,
    pub watcher_poll: Duration,
}

impl Default for ChatTimeouts {
    fn default() -> Self {
        Self {
            session_idle: Duration::from_secs(30 * 60),
            session_cleanup_interval: Duration::from_secs(5 * 60),
            stream_idle: Duration::from_secs(5 * 60),
            stream_total: Duration::from_secs(30 * 60),
            stream_heartbeat: Duration::from_secs(2),
            watcher_debounce: Duration::from_millis(200),
            watcher_idle: Duration::from_secs(60),
            watcher_poll: Duration::from_millis(50),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TokenDefaults {
    pub min_budget_tokens: usize,
    pub default_n_ctx: usize,
}

impl Default for TokenDefaults {
    fn default() -> Self {
        Self {
            min_budget_tokens: 1024,
            default_n_ctx: 32000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PresentationLimits {
    pub preview_chars: usize,
}

impl Default for PresentationLimits {
    fn default() -> Self {
        Self { preview_chars: 120 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChatConfig {
    pub limits: ChatLimits,
    pub timeouts: ChatTimeouts,
    pub tokens: TokenDefaults,
    pub presentation: PresentationLimits,
}

impl ChatConfig {
    pub fn new() -> Self {
        Self::default()
    }
}

pub static CHAT_CONFIG: std::sync::LazyLock<std::sync::RwLock<ChatConfig>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(ChatConfig::new()));

pub fn limits() -> ChatLimits {
    CHAT_CONFIG
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .limits
        .clone()
}

pub fn timeouts() -> ChatTimeouts {
    CHAT_CONFIG
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .timeouts
        .clone()
}

pub fn tokens() -> TokenDefaults {
    CHAT_CONFIG
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .tokens
        .clone()
}

pub fn presentation() -> PresentationLimits {
    CHAT_CONFIG
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .presentation
        .clone()
}

pub fn install_runtime_config(config: ChatConfig) {
    *CHAT_CONFIG
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = config;
}

pub fn apply_live_limits(limits: ChatLimits) {
    let mut config = CHAT_CONFIG
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    config.limits.max_queue_size = limits.max_queue_size;
    config.limits.recent_request_ids_capacity = limits.recent_request_ids_capacity;
    config.limits.max_images_per_message = limits.max_images_per_message;
    config.limits.max_parallel_tools = limits.max_parallel_tools;
    config.limits.max_file_size = limits.max_file_size;
}
