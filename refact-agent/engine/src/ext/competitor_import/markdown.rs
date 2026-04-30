use serde_yaml::{Mapping, Value};

use crate::ext::slash_commands::parse_frontmatter_and_body;

pub fn parse_markdown_frontmatter(content: &str) -> (Value, String) {
    parse_frontmatter_and_body(content)
}

pub fn frontmatter_mapping(frontmatter: &Value) -> Mapping {
    frontmatter.as_mapping().cloned().unwrap_or_default()
}

pub fn yaml_string(frontmatter: &Value, key: &str) -> String {
    frontmatter
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

pub fn yaml_string_any(frontmatter: &Value, keys: &[&str]) -> String {
    keys.iter()
        .map(|key| yaml_string(frontmatter, key))
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

pub fn yaml_string_list(frontmatter: &Value, key: &str) -> Vec<String> {
    match frontmatter.get(key) {
        Some(Value::Sequence(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect(),
        Some(Value::String(value)) => split_tool_list(value),
        _ => Vec::new(),
    }
}

pub fn yaml_string_list_any(frontmatter: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .map(|key| yaml_string_list(frontmatter, key))
        .find(|values| !values.is_empty())
        .unwrap_or_default()
}

pub fn first_useful_line_or_heading(markdown: &str) -> Option<String> {
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "---" || trimmed.starts_with("```") {
            continue;
        }
        let value = if trimmed.starts_with('#') {
            trimmed.trim_start_matches('#').trim()
        } else {
            trimmed.trim_start_matches(['-', '*']).trim()
        };
        let value = strip_inline_markdown(value);
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

pub fn sanitize_skill_id(input: &str) -> String {
    sanitize_identifier(input)
}

pub fn sanitize_command_name(input: &str) -> String {
    sanitize_identifier(input)
}

pub fn sanitize_subagent_id(input: &str) -> String {
    sanitize_identifier(input)
}

pub fn set_yaml_string(mapping: &mut Mapping, key: &str, value: &str) {
    if !value.trim().is_empty() {
        mapping.insert(
            Value::String(key.to_string()),
            Value::String(value.trim().to_string()),
        );
    }
}

pub fn set_yaml_string_list(mapping: &mut Mapping, key: &str, values: &[String]) {
    if !values.is_empty() {
        mapping.insert(
            Value::String(key.to_string()),
            Value::Sequence(
                values
                    .iter()
                    .map(|value| Value::String(value.clone()))
                    .collect(),
            ),
        );
    }
}

pub fn render_markdown_with_frontmatter(
    frontmatter: &Mapping,
    body: &str,
) -> Result<String, String> {
    if frontmatter.is_empty() {
        return Ok(body.to_string());
    }
    let yaml = serde_yaml::to_string(frontmatter).map_err(|err| err.to_string())?;
    let body = body.trim_start_matches(['\n', '\r']);
    Ok(format!("---\n{}\n---\n{}", yaml.trim_end(), body))
}

fn split_tool_list(value: &str) -> Vec<String> {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn sanitize_identifier(input: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in input.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch == '_' {
            Some('_')
        } else if ch == '-'
            || ch == '.'
            || ch == '/'
            || ch == '\\'
            || ch == ':'
            || ch.is_ascii_whitespace()
        {
            Some('-')
        } else {
            None
        };
        let Some(ch) = mapped else {
            continue;
        };
        if ch == '-' {
            if out.is_empty() || prev_dash {
                continue;
            }
            prev_dash = true;
            out.push(ch);
        } else {
            prev_dash = false;
            out.push(ch);
        }
    }
    out.trim_matches(['-', '_']).to_string()
}

fn strip_inline_markdown(value: &str) -> String {
    value
        .trim_matches(['`', '*', '_'])
        .trim()
        .chars()
        .take(160)
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn useful_line_prefers_heading_text() {
        let markdown = "\n# Review Code\nDetails";

        assert_eq!(
            first_useful_line_or_heading(markdown),
            Some("Review Code".to_string())
        );
    }

    #[test]
    fn sanitize_identifier_normalizes_names() {
        assert_eq!(sanitize_skill_id("My Skill.v2"), "my-skill-v2");
        assert_eq!(sanitize_command_name("docs/review"), "docs-review");
        assert_eq!(sanitize_subagent_id("Research Agent"), "research-agent");
    }

    #[test]
    fn render_frontmatter_roundtrip() {
        let mut mapping = Mapping::new();
        set_yaml_string(&mut mapping, "description", "Review code");
        set_yaml_string_list(&mut mapping, "allowed-tools", &["cat".to_string()]);

        let rendered = render_markdown_with_frontmatter(&mapping, "Body").unwrap();
        let (frontmatter, body) = parse_markdown_frontmatter(&rendered);

        assert_eq!(yaml_string(&frontmatter, "description"), "Review code");
        assert_eq!(yaml_string_list(&frontmatter, "allowed-tools"), vec!["cat"]);
        assert_eq!(body, "Body");
    }
}
