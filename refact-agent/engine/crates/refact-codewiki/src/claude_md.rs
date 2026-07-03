use serde::{Deserialize, Serialize};

use crate::selection_scoring::{PageCandidate, PageKind};

pub const BEGIN_MARKER: &str = "<!-- BEGIN REPOWISE -->";
pub const END_MARKER: &str = "<!-- END REPOWISE -->";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaudeMdInput {
    pub repo_name: String,
    pub pages: Vec<PageCandidate>,
    pub health_callouts: Vec<String>,
    pub tech_stack: Vec<String>,
    pub indexed_commit: String,
}

pub fn render_claude_md(pages: &[PageCandidate]) -> String {
    generate_claude_md(&ClaudeMdInput {
        repo_name: "repository".to_string(),
        pages: pages.to_vec(),
        health_callouts: Vec::new(),
        tech_stack: Vec::new(),
        indexed_commit: "unknown".to_string(),
    })
}

pub fn generate_claude_md(input: &ClaudeMdInput) -> String {
    let mut lines = Vec::new();
    lines.push(BEGIN_MARKER.to_string());
    lines.push(format!(
        "## IMPORTANT: Codebase Intelligence Instructions for {}",
        md_escape(&input.repo_name)
    ));
    lines.push(String::new());
    lines.push(
        "> This repository is indexed by Repowise. Always verify generated context against source before editing."
            .to_string(),
    );
    lines.push(format!(
        "Indexed commit: `{}`",
        md_escape(&input.indexed_commit)
    ));
    lines.push(String::new());

    lines.push("### Overview".to_string());
    if input.pages.is_empty() {
        lines.push("- No deterministic wiki pages are selected yet.".to_string());
    } else {
        for page in input.pages.iter().take(5) {
            lines.push(format!(
                "- `{}` — {} page for {}",
                md_escape(&page_title(page)),
                page_kind_label(page.kind),
                md_escape(&page.paths.join(", "))
            ));
        }
    }
    lines.push(String::new());

    lines.push("### Selected Pages".to_string());
    if input.pages.is_empty() {
        lines.push("No pages were selected.".to_string());
    } else {
        lines.push("| Page | Kind | Score | Paths |".to_string());
        lines.push("|---|---|---:|---|".to_string());
        for page in input.pages.iter().take(30) {
            lines.push(format!(
                "| `{}` | {} | {:.3} | {} |",
                md_escape(&page_title(page)),
                page_kind_label(page.kind),
                page.score,
                md_escape(&page.paths.join(", "))
            ));
        }
    }
    lines.push(String::new());

    lines.push("### Code Health".to_string());
    if input.health_callouts.is_empty() {
        lines.push("- No health callouts were indexed.".to_string());
    } else {
        for callout in input.health_callouts.iter().take(20) {
            lines.push(format!("- {}", md_escape(callout)));
        }
    }
    lines.push(String::new());

    lines.push("### Tech Stack".to_string());
    if input.tech_stack.is_empty() {
        lines.push("- Unknown or not detected.".to_string());
    } else {
        for item in input.tech_stack.iter().take(30) {
            lines.push(format!("- {}", md_escape(item)));
        }
    }
    lines.push(String::new());

    lines.push("### Search".to_string());
    lines.push(
        "Use hybrid search for repository questions: BM25 keyword retrieval over selected page text fused with a caller-provided PageRank prior via Reciprocal Rank Fusion (k=60)."
            .to_string(),
    );
    lines.push(String::new());
    lines.push(END_MARKER.to_string());

    cap_marked_block(lines, 200)
}

pub fn splice_into_existing(existing: &str, generated_body: &str) -> String {
    let Some(begin_start) = existing.find(BEGIN_MARKER) else {
        return append_marked_block(existing, generated_body);
    };
    let search_after_begin = begin_start + BEGIN_MARKER.len();
    let Some(end_relative) = existing[search_after_begin..].find(END_MARKER) else {
        return append_marked_block(existing, generated_body);
    };
    let end_end = search_after_begin + end_relative + END_MARKER.len();

    let mut out = String::new();
    out.push_str(existing[..begin_start].trim_end());
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(generated_body.trim());
    let tail = existing[end_end..].trim_start_matches(|c| c == '\r' || c == '\n');
    if !tail.is_empty() {
        out.push_str("\n\n");
        out.push_str(tail);
    }
    out
}

fn append_marked_block(existing: &str, generated_body: &str) -> String {
    if existing.trim().is_empty() {
        return format!("{}\n", generated_body.trim());
    }

    let mut out = existing.trim_end().to_string();
    out.push_str("\n\n");
    out.push_str(generated_body.trim());
    out.push('\n');
    out
}

fn cap_marked_block(mut lines: Vec<String>, max_lines: usize) -> String {
    if lines.len() > max_lines {
        lines.truncate(max_lines.saturating_sub(1));
        if !matches!(lines.last(), Some(line) if line == END_MARKER) {
            lines.push(END_MARKER.to_string());
        }
    }
    let mut rendered = lines.join("\n");
    rendered.push('\n');
    rendered
}

fn page_title(page: &PageCandidate) -> String {
    page.id
        .split_once(':')
        .map(|(_, title)| title)
        .unwrap_or(page.id.as_str())
        .to_string()
}

fn page_kind_label(kind: PageKind) -> &'static str {
    match kind {
        PageKind::File => "file",
        PageKind::Module => "module",
        PageKind::Scc => "scc",
        PageKind::ApiContract => "api contract",
        PageKind::Infra => "infra",
    }
}

fn md_escape(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(id: &str, kind: PageKind, score: f64, paths: &[&str]) -> PageCandidate {
        PageCandidate {
            id: id.to_string(),
            kind,
            score,
            paths: paths.iter().map(|path| (*path).to_string()).collect(),
        }
    }

    #[test]
    fn generate_claude_md_includes_core_fields_and_markers() {
        let input = ClaudeMdInput {
            repo_name: "demo-repo".to_string(),
            pages: vec![page(
                "file:src/auth.rs",
                PageKind::File,
                0.91,
                &["src/auth.rs"],
            )],
            health_callouts: vec!["auth has high churn".to_string()],
            tech_stack: vec!["Rust".to_string()],
            indexed_commit: "abc123".to_string(),
        };

        let md = generate_claude_md(&input);
        assert!(md.contains(BEGIN_MARKER));
        assert!(md.contains(END_MARKER));
        assert!(md.contains("demo-repo"));
        assert!(md.contains("src/auth.rs"));
        assert!(md.contains("abc123"));
    }

    #[test]
    fn claude_md_renders_fixture() {
        let pages = vec![
            page("file:src/auth.rs", PageKind::File, 1.0, &["src/auth.rs"]),
            page(
                "module:src",
                PageKind::Module,
                0.8,
                &["src/auth.rs", "src/lib.rs"],
            ),
        ];

        let md = render_claude_md(&pages);

        assert!(!md.trim().is_empty());
        assert!(md.contains("src/auth.rs"));
        assert!(md.contains("module:src") || md.contains("src"));
    }

    #[test]
    fn splice_into_existing_preserves_user_text_and_replaces_inside() {
        let existing = "user before\n<!-- BEGIN REPOWISE -->\nold generated\n<!-- END REPOWISE -->\nuser after";
        let generated = "<!-- BEGIN REPOWISE -->\nnew generated\n<!-- END REPOWISE -->\n";

        let spliced = splice_into_existing(existing, generated);
        assert!(spliced.contains("user before"));
        assert!(spliced.contains("user after"));
        assert!(spliced.contains("new generated"));
        assert!(!spliced.contains("old generated"));
    }

    #[test]
    fn splice_into_existing_appends_when_markers_absent() {
        let spliced = splice_into_existing(
            "user only",
            "<!-- BEGIN REPOWISE -->\nblock\n<!-- END REPOWISE -->",
        );
        assert!(spliced.starts_with("user only"));
        assert!(spliced.contains("block"));
    }
}
