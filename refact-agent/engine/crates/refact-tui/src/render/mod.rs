use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use textwrap::Options as WrapOptions;

use crate::vendored::decoded_text_merge::DecodedTextMerge;

#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownRenderer {
    width: Option<usize>,
}

impl MarkdownRenderer {
    pub fn new(width: Option<usize>) -> Self {
        Self { width }
    }

    pub fn render(&self, source: &str) -> Vec<Line<'static>> {
        render_markdown(source, self.width)
    }
}

pub fn render_markdown(source: &str, width: Option<usize>) -> Vec<Line<'static>> {
    let mut parser_options = Options::empty();
    parser_options.insert(Options::ENABLE_TABLES);
    parser_options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(source, parser_options).into_offset_iter();
    let mut out = Vec::new();
    let mut current = Vec::new();
    let mut style_stack = vec![Style::default()];
    let mut list_depth = 0usize;
    let mut in_code_block = false;
    let mut code_block_lang = String::new();

    for (event, _) in DecodedTextMerge::new(parser) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    flush_line(&mut out, &mut current, width);
                    let style = Style::default().add_modifier(Modifier::BOLD);
                    style_stack.push(style);
                    current.push(Span::styled(
                        format!("{} ", heading_marker(level as u8)),
                        style,
                    ));
                }
                Tag::Emphasis => push_merged_style(&mut style_stack, Modifier::ITALIC, None),
                Tag::Strong => push_merged_style(&mut style_stack, Modifier::BOLD, None),
                Tag::Strikethrough => {
                    push_merged_style(&mut style_stack, Modifier::CROSSED_OUT, None)
                }
                Tag::CodeBlock(kind) => {
                    flush_line(&mut out, &mut current, width);
                    in_code_block = true;
                    code_block_lang = match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    let label = if code_block_lang.is_empty() {
                        "code".to_string()
                    } else {
                        format!("code · {code_block_lang}")
                    };
                    out.push(Line::from(Span::styled(
                        label,
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                Tag::BlockQuote => {
                    flush_line(&mut out, &mut current, width);
                    current.push(Span::styled("▍ ", Style::default().fg(Color::Green)));
                    push_merged_style(&mut style_stack, Modifier::empty(), Some(Color::Green));
                }
                Tag::List(_) => {
                    flush_line(&mut out, &mut current, width);
                    list_depth = list_depth.saturating_add(1);
                }
                Tag::Item => {
                    flush_line(&mut out, &mut current, width);
                    current.push(Span::styled(
                        format!("{}• ", "  ".repeat(list_depth.saturating_sub(1))),
                        Style::default().fg(Color::Blue),
                    ));
                }
                Tag::Link { .. } => {
                    push_merged_style(&mut style_stack, Modifier::UNDERLINED, Some(Color::Cyan))
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => {
                    let _ = style_stack.pop();
                    flush_line(&mut out, &mut current, width);
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                    let _ = style_stack.pop();
                }
                TagEnd::CodeBlock => {
                    flush_line(&mut out, &mut current, width);
                    in_code_block = false;
                    code_block_lang.clear();
                }
                TagEnd::BlockQuote => {
                    let _ = style_stack.pop();
                    flush_line(&mut out, &mut current, width);
                }
                TagEnd::List(_) => {
                    flush_line(&mut out, &mut current, width);
                    list_depth = list_depth.saturating_sub(1);
                }
                TagEnd::Item => flush_line(&mut out, &mut current, width),
                TagEnd::Paragraph => flush_line(&mut out, &mut current, width),
                _ => {}
            },
            Event::Text(text) => {
                let style = current_style(&style_stack);
                if in_code_block {
                    for (idx, line) in text.split('\n').enumerate() {
                        if idx > 0 {
                            flush_line(&mut out, &mut current, width);
                        }
                        if !line.is_empty() {
                            current.push(Span::styled(
                                format!("  {line}"),
                                Style::default().fg(Color::Yellow),
                            ));
                        }
                    }
                } else {
                    current.push(Span::styled(text.to_string(), style));
                }
            }
            Event::Code(code) => current.push(Span::styled(
                code.to_string(),
                current_style(&style_stack).fg(Color::Yellow),
            )),
            Event::SoftBreak => current.push(Span::raw(" ")),
            Event::HardBreak => flush_line(&mut out, &mut current, width),
            Event::Rule => {
                flush_line(&mut out, &mut current, width);
                out.push(Line::from(Span::styled(
                    "─".repeat(width.unwrap_or(24).min(80)),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                current.push(Span::styled(
                    html.to_string(),
                    current_style(&style_stack).fg(Color::DarkGray),
                ));
            }
            _ => {}
        }
    }
    flush_line(&mut out, &mut current, width);
    out
}

fn heading_marker(level: u8) -> &'static str {
    match level {
        1 => "#",
        2 => "##",
        3 => "###",
        4 => "####",
        5 => "#####",
        _ => "######",
    }
}

fn push_merged_style(stack: &mut Vec<Style>, modifier: Modifier, color: Option<Color>) {
    let mut style = current_style(stack).add_modifier(modifier);
    if let Some(color) = color {
        style = style.fg(color);
    }
    stack.push(style);
}

fn current_style(stack: &[Style]) -> Style {
    stack.last().copied().unwrap_or_default()
}

fn flush_line(
    out: &mut Vec<Line<'static>>,
    current: &mut Vec<Span<'static>>,
    width: Option<usize>,
) {
    if current.is_empty() {
        return;
    }
    let line = Line::from(std::mem::take(current));
    out.extend(wrap_line(line, width));
}

fn wrap_line(line: Line<'static>, width: Option<usize>) -> Vec<Line<'static>> {
    let Some(width) = width.filter(|w| *w > 0) else {
        return vec![line];
    };
    let plain = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    if unicode_width::UnicodeWidthStr::width(plain.as_str()) <= width {
        return vec![line];
    }
    let style = line
        .spans
        .first()
        .map(|span| span.style)
        .unwrap_or(line.style);
    textwrap::wrap(&plain, WrapOptions::new(width))
        .into_iter()
        .map(|wrapped| Line::from(Span::styled(wrapped.into_owned(), style)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn renders_markdown_headings_and_code() {
        let lines = render_markdown(
            "# Title\nhello `world`\n\n```rust\nfn main() {}\n```",
            Some(80),
        );
        let rendered = text(&lines);
        assert!(rendered.iter().any(|line| line.contains("# Title")));
        assert!(rendered.iter().any(|line| line.contains("hello world")));
        assert!(rendered.iter().any(|line| line.contains("code · rust")));
        assert!(rendered.iter().any(|line| line.contains("fn main")));
    }

    #[test]
    fn wraps_long_markdown() {
        let lines = render_markdown("alpha beta gamma delta", Some(8));
        assert!(lines.len() > 1);
    }
}
