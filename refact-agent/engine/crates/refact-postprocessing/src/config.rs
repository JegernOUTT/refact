#[derive(Debug, Clone)]
pub struct PostprocessingLimits {
    pub max_tool_budget_tokens: usize,
    pub max_per_file_budget_tokens: usize,
    pub max_line_length_chars: usize,
    pub tokens_for_text_percent: usize,
}

impl Default for PostprocessingLimits {
    fn default() -> Self {
        Self {
            max_tool_budget_tokens: 131_072,
            max_per_file_budget_tokens: 131_072,
            max_line_length_chars: 10_000,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_large_enough_for_modern_context_windows() {
        let d = PostprocessingLimits::default();
        assert_eq!(d.max_tool_budget_tokens, 131_072);
        assert_eq!(d.max_per_file_budget_tokens, 131_072);
        assert_eq!(d.max_line_length_chars, 10_000);
        assert_eq!(d.tokens_for_text_percent, 30);
    }

    #[test]
    fn install_postprocessing_limits_changes_what_limits_returns() {
        let guard = test_limits_lock();
        install_postprocessing_limits(PostprocessingLimits {
            max_tool_budget_tokens: 262_144,
            max_per_file_budget_tokens: 65_536,
            max_line_length_chars: 4_096,
            tokens_for_text_percent: 40,
        });
        let installed = limits();
        install_postprocessing_limits(PostprocessingLimits::default());
        drop(guard);

        assert_eq!(installed.max_tool_budget_tokens, 262_144);
        assert_eq!(installed.max_per_file_budget_tokens, 65_536);
        assert_eq!(installed.max_line_length_chars, 4_096);
        assert_eq!(installed.tokens_for_text_percent, 40);
        assert_eq!(limits().max_tool_budget_tokens, 131_072);
    }
}
