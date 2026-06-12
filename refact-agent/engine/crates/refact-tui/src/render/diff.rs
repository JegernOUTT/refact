use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::wrapping::wrap_line;

#[derive(Clone, Copy)]
enum DiffKind {
    File,
    Hunk,
    Add,
    Delete,
    Context,
}

pub fn is_unified_diff(text: &str) -> bool {
    let mut has_file = false;
    let mut has_hunk = false;
    for line in text.lines() {
        if line.starts_with("diff --git ") || line.starts_with("--- ") || line.starts_with("+++ ") {
            has_file = true;
        }
        if line.starts_with("@@") {
            has_hunk = true;
        }
    }
    has_file && has_hunk
}

pub fn render_unified_diff(
    text: &str,
    width: Option<usize>,
    color_enabled: bool,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for line in text.lines() {
        let kind = classify(line);
        let style = style_for(kind, color_enabled);
        let prefix = prefix_for(kind);
        let content = if matches!(kind, DiffKind::Add | DiffKind::Delete) && line.len() > 1 {
            &line[1..]
        } else {
            line
        };
        let rendered = Line::from(vec![
            Span::styled(prefix.to_string(), style),
            Span::styled(content.to_string(), style),
        ]);
        out.extend(wrap_line(rendered, width));
    }
    if out.is_empty() {
        out.push(Line::default());
    }
    out
}

fn classify(line: &str) -> DiffKind {
    if line.starts_with("@@") {
        DiffKind::Hunk
    } else if line.starts_with("diff --git ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
    {
        DiffKind::File
    } else if line.starts_with('+') {
        DiffKind::Add
    } else if line.starts_with('-') {
        DiffKind::Delete
    } else {
        DiffKind::Context
    }
}

fn prefix_for(kind: DiffKind) -> &'static str {
    match kind {
        DiffKind::Add => "+ ",
        DiffKind::Delete => "- ",
        DiffKind::Hunk => "@ ",
        DiffKind::File => "  ",
        DiffKind::Context => "  ",
    }
}

fn style_for(kind: DiffKind, color_enabled: bool) -> Style {
    if !color_enabled {
        return match kind {
            DiffKind::File | DiffKind::Hunk => Style::default().add_modifier(Modifier::BOLD),
            _ => Style::default(),
        };
    }
    match kind {
        DiffKind::File => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        DiffKind::Hunk => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        DiffKind::Add => Style::default().fg(Color::Green),
        DiffKind::Delete => Style::default().fg(Color::Red),
        DiffKind::Context => Style::default().fg(Color::DarkGray),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::wrapping::line_to_plain;

    fn plain(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(line_to_plain).collect()
    }

    #[test]
    fn detects_unified_diff() {
        assert!(is_unified_diff("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new"));
        assert!(!is_unified_diff("+not enough"));
    }

    #[test]
    fn renders_diff_prefixes_and_styles() {
        let lines = render_unified_diff(
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n same",
            Some(80),
            true,
        );
        let rendered = plain(&lines);
        assert!(rendered.iter().any(|line| line == "- old"));
        assert!(rendered.iter().any(|line| line == "+ new"));
        let add = lines
            .iter()
            .find(|line| line_to_plain(line) == "+ new")
            .unwrap();
        assert_eq!(add.spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn no_color_diff_keeps_prefixes() {
        let lines = render_unified_diff("@@ -1 +1 @@\n-old\n+new", Some(80), false);
        assert!(plain(&lines).contains(&"- old".to_string()));
        assert!(plain(&lines).contains(&"+ new".to_string()));
    }
}
