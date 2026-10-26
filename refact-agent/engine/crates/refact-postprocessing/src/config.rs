#[derive(Debug, Clone)]
pub struct PostprocessingLimits {
    pub max_tool_budget_tokens: usize,
    pub max_per_file_budget_tokens: usize,
    pub max_line_length_chars: usize,
    pub tool_max_line_chars: usize,
    pub tokens_for_text_percent: usize,
}

impl Default for PostprocessingLimits {
    fn default() -> Self {
        Self {
            max_tool_budget_tokens: 131_072,
            max_per_file_budget_tokens: 131_072,
            max_line_length_chars: 10_000,
            tool_max_line_chars: 2_000,
            tokens_for_text_percent: 30,
        }
    }
}

pub static POSTPROCESSING_LIMITS: std::sync::LazyLock<std::sync::RwLock<PostprocessingLimits>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(PostprocessingLimits::default()));

pub fn limits() -> PostprocessingLimits {
    POSTPROCESSING_LIMITS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub fn install_postprocessing_limits(limits: PostprocessingLimits) {
    *POSTPROCESSING_LIMITS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = limits;
}

#[cfg(test)]
pub(crate) static TEST_LIMITS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn test_limits_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LIMITS_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
