use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ChatLimits {
    pub max_queue_size: usize,
    pub event_channel_capacity: usize,
    pub recent_request_ids_capacity: usize,
    pub max_images_per_message: usize,
    pub max_parallel_tools: usize,
    pub max_included_files: usize,
    pub max_file_size: usize,
}

impl Default for ChatLimits {
    fn default() -> Self {
        Self {
            max_queue_size: 100,
            event_channel_capacity: 4096,
            recent_request_ids_capacity: 100,
            max_images_per_message: 5,
            max_parallel_tools: 16,
            max_included_files: 15,
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
            stream_idle: Duration::from_secs(60 * 60),
            stream_total: Duration::from_secs(60 * 60),
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

pub static CHAT_CONFIG: std::sync::LazyLock<ChatConfig> = std::sync::LazyLock::new(ChatConfig::new);

pub fn limits() -> &'static ChatLimits {
    &CHAT_CONFIG.limits
}

pub fn timeouts() -> &'static ChatTimeouts {
    &CHAT_CONFIG.timeouts
}

pub fn tokens() -> &'static TokenDefaults {
    &CHAT_CONFIG.tokens
}

pub fn presentation() -> &'static PresentationLimits {
    &CHAT_CONFIG.presentation
}
