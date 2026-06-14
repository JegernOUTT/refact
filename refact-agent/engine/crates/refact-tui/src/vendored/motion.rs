// Adapted from openai/codex codex-rs/tui, Apache-2.0.

pub fn reduced_motion_from_env() -> bool {
    std::env::var_os("REFACT_TUI_REDUCED_MOTION").is_some()
        || std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM")
            .map(|term| term == "dumb")
            .unwrap_or(false)
}

pub fn frame<'a>(frames: &'a [&'a str], tick: u64) -> &'a str {
    if frames.is_empty() {
        return "";
    }
    frames[tick as usize % frames.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_wraps() {
        assert_eq!(frame(&["a", "b", "c"], 4), "b");
        assert_eq!(frame(&[], 4), "");
    }
}
