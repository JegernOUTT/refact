pub mod diff;
pub mod highlight;
pub mod markdown;
pub mod wrapping;

pub use diff::{is_unified_diff, render_unified_diff};
pub use markdown::{render_markdown, render_markdown_with_options, MarkdownRenderer, RenderOptions};

pub fn color_enabled_from_env() -> bool {
    std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM")
            .map(|term| term != "dumb")
            .unwrap_or(true)
}
