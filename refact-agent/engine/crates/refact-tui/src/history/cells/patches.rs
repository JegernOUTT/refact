use super::*;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::diff_model::FileChange;
use crate::render::create_diff_summary;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiffCell {
    text: String,
}

impl DiffCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl HistoryCell for DiffCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Diff
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = vec![role_line(
            "diff",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )];
        lines.extend(diff_summary_header_lines(&self.text, width));
        lines.extend(render_unified_diff(
            &self.text,
            Some(width.saturating_sub(2).max(8)),
            color_enabled_from_env(),
        ));
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiffToolCell {
    card: ToolCard,
    selected: bool,
}

impl DiffToolCell {
    pub fn new(card: ToolCard, selected: bool) -> Self {
        Self { card, selected }
    }
}

impl HistoryCell for DiffToolCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Diff
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let source = diff_source(&self.card);
        let stats = diff_file_stats(&source);
        let mut lines = vec![role_line(
            if self.selected {
                "diff selected"
            } else {
                "diff"
            },
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )];
        lines.push(tool_summary_line(
            &self.card,
            diff_summary(&stats),
            self.card
                .duration_ms
                .map(format_duration)
                .unwrap_or_default(),
        ));
        lines.extend(subchat_lines(&self.card, width));
        lines.extend(diff_summary_header_lines(&source, width));
        for stat in &stats {
            lines.push(Line::from(vec![
                Span::styled("Δ ", Style::default().fg(Color::Blue)),
                Span::raw(stat.path.clone()),
                Span::styled(
                    format!(" +{}", stat.added),
                    Style::default().fg(Color::Green),
                ),
                Span::styled(
                    format!(" -{}", stat.deleted),
                    Style::default().fg(Color::Red),
                ),
            ]));
        }
        if self.card.expanded {
            if is_unified_diff(&source) {
                lines.extend(
                    render_unified_diff(
                        &source,
                        Some(width.saturating_sub(2).max(8)),
                        color_enabled_from_env(),
                    )
                    .into_iter()
                    .take(EXPANDED_OUTPUT_LINES),
                );
            } else {
                lines.extend(output_lines(&source, width, EXPANDED_OUTPUT_LINES, false));
            }
        }
        finish(lines)
    }

    fn is_final(&self) -> bool {
        self.card.status != ToolStatus::Running
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.card, self.selected))
    }
}

fn diff_summary_header_lines(source: &str, width: usize) -> Vec<Line<'static>> {
    let changes = file_changes_from_unified_diff(source);
    if changes.is_empty() {
        return Vec::new();
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    create_diff_summary(&changes, &cwd, width)
        .into_iter()
        .filter(|line| !line_to_plain(line).starts_with("    "))
        .collect()
}

fn file_changes_from_unified_diff(source: &str) -> HashMap<PathBuf, FileChange> {
    let mut changes = HashMap::new();
    if !is_unified_diff(source) {
        return changes;
    }

    let mut current_path = None::<PathBuf>;
    let mut current_change = None::<ParsedFileChange>;
    let mut current_lines = Vec::<String>::new();
    for line in source.lines() {
        if let Some(path) = diff_git_pathbuf(line).or_else(|| plus_file_pathbuf(line)) {
            if let Some(path) = current_path.replace(path) {
                changes.insert(
                    path,
                    file_change_from_parsed(&current_lines, current_change.take()),
                );
            }
            current_lines.clear();
            current_change = None;
        }
        if current_path.is_some() {
            current_change = current_change.or_else(|| parsed_file_change(line));
            current_lines.push(line.to_string());
        }
    }
    if let Some(path) = current_path {
        changes.insert(
            path,
            file_change_from_parsed(&current_lines, current_change),
        );
    }
    changes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParsedFileChange {
    Add,
    Delete,
}

fn parsed_file_change(line: &str) -> Option<ParsedFileChange> {
    let line = line.trim();
    if line.starts_with("new file mode ") {
        Some(ParsedFileChange::Add)
    } else if line.starts_with("deleted file mode ") {
        Some(ParsedFileChange::Delete)
    } else {
        None
    }
}

fn file_change_from_parsed(lines: &[String], parsed: Option<ParsedFileChange>) -> FileChange {
    let unified_diff = lines.join("\n");
    match parsed {
        Some(ParsedFileChange::Add) => FileChange::Add {
            content: added_file_content(lines),
        },
        Some(ParsedFileChange::Delete) => FileChange::Delete {
            content: deleted_file_content(lines),
        },
        _ => FileChange::Update {
            unified_diff,
            move_path: None,
        },
    }
}

fn added_file_content(lines: &[String]) -> String {
    lines
        .iter()
        .filter_map(|line| line.strip_prefix('+'))
        .filter(|line| !line.starts_with("++"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn deleted_file_content(lines: &[String]) -> String {
    lines
        .iter()
        .filter_map(|line| line.strip_prefix('-'))
        .filter(|line| !line.starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn line_to_plain(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

fn diff_git_pathbuf(line: &str) -> Option<PathBuf> {
    let rest = line.strip_prefix("diff --git ")?;
    rest.split_whitespace()
        .nth(1)
        .map(|path| path.trim_start_matches("b/"))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}
fn plus_file_pathbuf(line: &str) -> Option<PathBuf> {
    let path = line
        .strip_prefix("+++ ")?
        .trim()
        .split_whitespace()
        .next()?;
    if path == "/dev/null" {
        return None;
    }
    Some(PathBuf::from(path.trim_start_matches("b/"))).filter(|path| !path.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::test_support::{text, tool_card};
    use serde_json::json;

    #[test]
    fn diff_cell_reuses_unified_diff_renderer() {
        let cell = DiffCell::new("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new");
        let rendered = text(&cell.render(80));
        assert!(rendered.contains("diff\n"));
        assert!(rendered.contains("• Edited x (+1 -1)"));
        assert!(rendered.contains("-old"));
        assert!(rendered.contains("+new"));
    }

    #[test]
    fn diff_cell_snapshot_reuses_unified_diff_renderer() {
        let card = tool_card(
            "apply_patch",
            json!({}),
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new",
        );
        let cell = cell_from_tool_card(card, false);
        assert_eq!(cell.kind(), HistoryCellKind::Diff);
        let rendered = text(&cell.render(80));
        assert!(rendered.contains("diff\n▾ ✅ 1 file · +1 -1 · 1.2s"));
        assert!(rendered.contains("• Edited x (+1 -1)"));
        assert!(rendered.contains("Δ x +1 -1"));
        assert!(rendered.contains("-old"));
        assert!(rendered.contains("+new"));
    }
}
