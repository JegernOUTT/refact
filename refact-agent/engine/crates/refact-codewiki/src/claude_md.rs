use serde::{Deserialize, Serialize};

use crate::wiki::WikiEntry;

pub const BEGIN_MARKER: &str = "<!-- BEGIN REPOWISE -->";
pub const END_MARKER: &str = "<!-- END REPOWISE -->";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaudeMdInput {
    pub repo_name: String,
    pub top_modules: Vec<(String, f64, String)>,
    pub health_callouts: Vec<String>,
    pub tech_stack: Vec<String>,
    pub indexed_commit: String,
    pub wiki: Vec<WikiEntry>,
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
    let overview: Vec<&WikiEntry> = input
        .wiki
        .iter()
        .filter(|entry| !entry.summary.trim().is_empty())
        .take(5)
        .collect();
    if overview.is_empty() {
        lines.push("- No wiki summaries are indexed yet.".to_string());
    } else {
        for entry in overview {
            lines.push(format!(
                "- `{}` — {}",
                md_escape(&entry.module),
                one_line(&entry.summary, 180)
            ));
        }
    }
    lines.push(String::new());

    lines.push("### Top Modules".to_string());
    if input.top_modules.is_empty() {
        lines.push("No central modules were identified.".to_string());
    } else {
        lines.push("| Module | Centrality | Owner |".to_string());
        lines.push("|---|---:|---|".to_string());
        for (module, centrality, owner) in input.top_modules.iter().take(20) {
            let owner = if owner.trim().is_empty() {
                "—"
            } else {
                owner
            };
            lines.push(format!(
                "| `{}` | {:.3} | {} |",
                md_escape(module),
                centrality,
                md_escape(owner)
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
        "Use hybrid search for repository questions: BM25 keyword retrieval over module summaries fused with a freshness/PageRank-style prior via Reciprocal Rank Fusion (k=60)."
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

fn md_escape(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

fn one_line(text: &str, max_chars: usize) -> String {
    let mut s = md_escape(text);
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max_chars {
        return s;
    }

    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(module: &str, summary: &str) -> WikiEntry {
        WikiEntry {
            module: module.to_string(),
            summary: summary.to_string(),
            source_hash: 0,
            freshness: 1.0,
        }
    }

    #[test]
    fn generate_claude_md_includes_core_fields_and_markers() {
        let input = ClaudeMdInput {
            repo_name: "demo-repo".to_string(),
            top_modules: vec![("auth".to_string(), 0.91, "alice".to_string())],
            health_callouts: vec!["auth has high churn".to_string()],
            tech_stack: vec!["Rust".to_string()],
            indexed_commit: "abc123".to_string(),
            wiki: vec![entry("auth", "Handles login token validation.")],
        };

        let md = generate_claude_md(&input);
        assert!(md.contains(BEGIN_MARKER));
        assert!(md.contains(END_MARKER));
        assert!(md.contains("demo-repo"));
        assert!(md.contains("auth"));
        assert!(md.contains("abc123"));
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
