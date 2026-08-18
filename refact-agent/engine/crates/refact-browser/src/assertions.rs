use std::collections::BTreeMap;

use refact_integrations::browser_models::{BrowserExpectedText, LocatorRegex};
use regex::RegexBuilder;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextMatchKind {
    Exact,
    Contains,
}

pub fn normalize_text(value: &str) -> String {
    value
        .replace(['\u{200b}', '\u{00ad}'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn matches_text(
    received: &str,
    expected: &BrowserExpectedText,
    kind: TextMatchKind,
    ignore_case: bool,
) -> Result<bool, String> {
    match expected {
        BrowserExpectedText::Text(expected) => {
            let mut received = normalize_text(received);
            let mut expected = normalize_text(expected);
            if ignore_case {
                received = received.to_lowercase();
                expected = expected.to_lowercase();
            }
            Ok(match kind {
                TextMatchKind::Exact => received == expected,
                TextMatchKind::Contains => received.contains(&expected),
            })
        }
        BrowserExpectedText::Regex(regex) => regex_matches(received, regex, ignore_case),
    }
}

pub fn matches_text_list(
    received: &[String],
    expected: &[BrowserExpectedText],
    kind: TextMatchKind,
    ignore_case: bool,
) -> Result<bool, String> {
    match kind {
        TextMatchKind::Exact => {
            if received.len() != expected.len() {
                return Ok(false);
            }
            for (received, expected) in received.iter().zip(expected) {
                if !matches_text(received, expected, TextMatchKind::Exact, ignore_case)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        TextMatchKind::Contains => {
            let mut index = 0;
            for expected in expected {
                loop {
                    let Some(candidate) = received.get(index) else {
                        return Ok(false);
                    };
                    index += 1;
                    if matches_text(candidate, expected, TextMatchKind::Contains, ignore_case)? {
                        break;
                    }
                }
            }
            Ok(true)
        }
    }
}

fn regex_matches(
    received: &str,
    expected: &LocatorRegex,
    ignore_case: bool,
) -> Result<bool, String> {
    let flags = &expected.flags;
    let mut builder = RegexBuilder::new(&expected.source);
    builder.case_insensitive(ignore_case || flags.contains('i'));
    builder.multi_line(flags.contains('m'));
    builder.dot_matches_new_line(flags.contains('s'));
    builder
        .build()
        .map(|regex| regex.is_match(received))
        .map_err(|error| format!("Invalid expectation regex: {error}"))
}

pub fn matches_json_property(received: &Value, expected: &Value) -> bool {
    received == expected
}

pub fn matches_class_list(received: &str, expected: &str, ignore_case: bool) -> bool {
    let mut expected = expected
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut received = received
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if ignore_case {
        expected = expected
            .into_iter()
            .map(|value| value.to_lowercase())
            .collect();
        received = received
            .into_iter()
            .map(|value| value.to_lowercase())
            .collect();
    }
    expected
        .iter()
        .all(|class_name| received.contains(class_name))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ChildrenMode {
    #[default]
    Contain,
    Equal,
    DeepEqual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AriaValue {
    Text(String),
    Regex(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AriaTemplateNode {
    Role {
        role: String,
        name: Option<AriaValue>,
        attributes: BTreeMap<String, String>,
        properties: BTreeMap<String, String>,
        children: Vec<AriaTemplateNode>,
        mode: ChildrenMode,
    },
    Text(String),
}

pub fn match_aria_snapshot(expected: &str, actual: &str) -> Result<bool, String> {
    let expected = parse_aria_yaml(expected)?;
    let actual = parse_aria_yaml(actual)?;
    Ok(match_children(&expected, &actual, ChildrenMode::Contain))
}

pub fn aria_snapshot_diff(expected: &str, actual: &str) -> String {
    let expected_lines = expected.lines().collect::<Vec<_>>();
    let actual_lines = actual.lines().collect::<Vec<_>>();
    let mut common_prefix = 0;
    while common_prefix < expected_lines.len()
        && common_prefix < actual_lines.len()
        && expected_lines[common_prefix] == actual_lines[common_prefix]
    {
        common_prefix += 1;
    }
    let mut lines = vec!["--- expected".to_string(), "+++ received".to_string()];
    lines.extend(
        expected_lines[common_prefix..]
            .iter()
            .map(|line| format!("- {line}")),
    );
    lines.extend(
        actual_lines[common_prefix..]
            .iter()
            .map(|line| format!("+ {line}")),
    );
    lines.join("\n")
}

fn parse_aria_yaml(input: &str) -> Result<Vec<AriaTemplateNode>, String> {
    let value: serde_yaml::Value = serde_yaml::from_str(input)
        .map_err(|error| format!("Invalid ARIA snapshot YAML: {error}"))?;
    let sequence = value
        .as_sequence()
        .ok_or_else(|| "ARIA snapshot must be a YAML sequence".to_string())?;
    parse_sequence(sequence)
}

fn parse_sequence(sequence: &[serde_yaml::Value]) -> Result<Vec<AriaTemplateNode>, String> {
    let mut nodes = Vec::new();
    for item in sequence {
        match item {
            serde_yaml::Value::String(value) => nodes.push(parse_key(value, Vec::new())?),
            serde_yaml::Value::Mapping(mapping) => {
                for (key, value) in mapping {
                    let key = key
                        .as_str()
                        .ok_or_else(|| "ARIA snapshot keys must be strings".to_string())?;
                    if key.starts_with('/') {
                        let value = value.as_str().map(str::to_string).unwrap_or_else(|| {
                            serde_yaml::to_string(value)
                                .unwrap_or_default()
                                .trim()
                                .to_string()
                        });
                        nodes.push(AriaTemplateNode::Text(format!("{key}: {value}")));
                        continue;
                    }
                    let children = match value {
                        serde_yaml::Value::Null => Vec::new(),
                        serde_yaml::Value::Sequence(children) => parse_sequence(children)?,
                        serde_yaml::Value::String(text) => {
                            vec![AriaTemplateNode::Text(normalize_text(text))]
                        }
                        other => vec![AriaTemplateNode::Text(normalize_text(
                            &serde_yaml::to_string(other)
                                .map_err(|error| error.to_string())?
                                .trim()
                                .to_string(),
                        ))],
                    };
                    nodes.push(parse_key(key, children)?);
                }
            }
            other => nodes.push(AriaTemplateNode::Text(normalize_text(
                &serde_yaml::to_string(other)
                    .map_err(|error| error.to_string())?
                    .trim()
                    .to_string(),
            ))),
        }
    }
    Ok(nodes)
}

fn parse_key(key: &str, children: Vec<AriaTemplateNode>) -> Result<AriaTemplateNode, String> {
    if key == "text" {
        return Ok(AriaTemplateNode::Text(String::new()));
    }
    if key.starts_with('/') {
        return Ok(AriaTemplateNode::Text(key.to_string()));
    }
    let key = key.trim();
    let role_end = key.find(char::is_whitespace).unwrap_or(key.len());
    let role = key[..role_end].to_string();
    if role.is_empty() {
        return Err("ARIA role cannot be empty".to_string());
    }
    let mut rest = key[role_end..].trim();
    let name = if let Some(value) = rest.strip_prefix('"') {
        let end =
            find_unescaped(value, '"').ok_or_else(|| format!("Unterminated ARIA name in {key}"))?;
        rest = value[end + 1..].trim();
        Some(AriaValue::Text(normalize_text(&unescape_quoted(
            &value[..end],
        ))))
    } else if let Some(value) = rest.strip_prefix('/') {
        let end = find_unescaped(value, '/')
            .ok_or_else(|| format!("Unterminated ARIA regex in {key}"))?;
        rest = value[end + 1..].trim();
        Some(AriaValue::Regex(value[..end].to_string()))
    } else {
        None
    };
    let mut attributes = BTreeMap::new();
    while let Some(value) = rest.strip_prefix('[') {
        let end = value
            .find(']')
            .ok_or_else(|| format!("Unterminated ARIA attribute in {key}"))?;
        let attribute = &value[..end];
        let (name, value) = attribute.split_once('=').unwrap_or((attribute, "true"));
        attributes.insert(name.trim().to_string(), value.trim().to_string());
        rest = value_at(rest, end + 2).trim();
    }
    if !rest.is_empty() {
        return Err(format!("Unexpected ARIA key content: {rest}"));
    }
    let mut properties = BTreeMap::new();
    let mut mode = ChildrenMode::Contain;
    let mut actual_children = Vec::new();
    for child in children {
        let AriaTemplateNode::Text(text) = &child else {
            actual_children.push(child);
            continue;
        };
        let Some(property) = text.strip_prefix('/') else {
            actual_children.push(child);
            continue;
        };
        let (property_name, property_value) = property
            .split_once(':')
            .ok_or_else(|| format!("Invalid ARIA property: {text}"))?;
        if property_name == "children" {
            mode = match property_value.trim() {
                "contain" => ChildrenMode::Contain,
                "equal" => ChildrenMode::Equal,
                "deep-equal" => ChildrenMode::DeepEqual,
                other => return Err(format!("Unsupported ARIA children mode: {other}")),
            };
        } else {
            properties.insert(property_name.to_string(), normalize_text(property_value));
        }
    }
    Ok(AriaTemplateNode::Role {
        role,
        name,
        attributes,
        properties,
        children: actual_children,
        mode,
    })
}

fn value_at(value: &str, offset: usize) -> &str {
    &value[offset..]
}

fn find_unescaped(value: &str, target: char) -> Option<usize> {
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == target {
            return Some(index);
        }
    }
    None
}

fn unescape_quoted(value: &str) -> String {
    value.replace("\\\"", "\"").replace("\\\\", "\\")
}

fn match_children(
    expected: &[AriaTemplateNode],
    actual: &[AriaTemplateNode],
    mode: ChildrenMode,
) -> bool {
    if matches!(mode, ChildrenMode::Equal | ChildrenMode::DeepEqual) {
        let child_mode = if mode == ChildrenMode::DeepEqual {
            ChildrenMode::DeepEqual
        } else {
            ChildrenMode::Contain
        };
        return expected.len() == actual.len()
            && expected
                .iter()
                .zip(actual)
                .all(|(expected, actual)| match_node(expected, actual, child_mode));
    }
    let mut actual_index = 0;
    for expected_node in expected {
        let Some(offset) = actual[actual_index..]
            .iter()
            .position(|actual_node| match_node(expected_node, actual_node, mode))
        else {
            return false;
        };
        actual_index += offset + 1;
    }
    true
}

fn match_node(
    expected: &AriaTemplateNode,
    actual: &AriaTemplateNode,
    inherited: ChildrenMode,
) -> bool {
    match (expected, actual) {
        (AriaTemplateNode::Text(expected), AriaTemplateNode::Text(actual)) => {
            normalize_text(expected) == normalize_text(actual)
        }
        (
            AriaTemplateNode::Role {
                role: expected_role,
                name: expected_name,
                attributes: expected_attributes,
                properties: expected_properties,
                children: expected_children,
                mode,
            },
            AriaTemplateNode::Role {
                role: actual_role,
                name: actual_name,
                attributes: actual_attributes,
                properties: actual_properties,
                children: actual_children,
                ..
            },
        ) => {
            expected_role == actual_role
                && expected_name
                    .as_ref()
                    .is_none_or(|expected| match_aria_value(expected, actual_name.as_ref()))
                && expected_attributes
                    .iter()
                    .all(|(key, value)| actual_attributes.get(key) == Some(value))
                && expected_properties
                    .iter()
                    .all(|(key, value)| actual_properties.get(key) == Some(value))
                && match_children(
                    expected_children,
                    actual_children,
                    if *mode == ChildrenMode::Contain {
                        inherited
                    } else {
                        *mode
                    },
                )
        }
        _ => false,
    }
}

fn match_aria_value(expected: &AriaValue, actual: Option<&AriaValue>) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    let actual = match actual {
        AriaValue::Text(value) | AriaValue::Regex(value) => value,
    };
    match expected {
        AriaValue::Text(expected) => expected == actual,
        AriaValue::Regex(pattern) => RegexBuilder::new(pattern)
            .build()
            .is_ok_and(|regex| regex.is_match(actual)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> BrowserExpectedText {
        BrowserExpectedText::Text(value.to_string())
    }

    #[test]
    fn text_matching_normalizes_whitespace_and_supports_case_substring_and_regex() {
        assert!(matches_text(
            "  Hello\n world ",
            &text("Hello world"),
            TextMatchKind::Exact,
            false
        )
        .unwrap());
        assert!(
            matches_text("Hello WORLD", &text("world"), TextMatchKind::Contains, true).unwrap()
        );
        assert!(matches_text(
            "Order 42",
            &BrowserExpectedText::Regex(LocatorRegex {
                source: r"Order \d+".to_string(),
                flags: String::new()
            }),
            TextMatchKind::Exact,
            false,
        )
        .unwrap());
    }

    fn received(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn expected(values: &[&str]) -> Vec<BrowserExpectedText> {
        values.iter().map(|value| text(value)).collect()
    }

    #[test]
    fn have_text_lists_require_the_same_length_and_order() {
        let actual = received(&["Alpha", "Beta", "Gamma"]);
        assert!(matches_text_list(
            &actual,
            &expected(&["Alpha", "Beta", "Gamma"]),
            TextMatchKind::Exact,
            false
        )
        .unwrap());
        assert!(!matches_text_list(
            &actual,
            &expected(&["Alpha", "Beta"]),
            TextMatchKind::Exact,
            false
        )
        .unwrap());
        assert!(!matches_text_list(
            &actual,
            &expected(&["Beta", "Alpha", "Gamma"]),
            TextMatchKind::Exact,
            false
        )
        .unwrap());
        assert!(!matches_text_list(
            &actual,
            &expected(&["Alph", "Beta", "Gamma"]),
            TextMatchKind::Exact,
            false
        )
        .unwrap());
    }

    #[test]
    fn contain_text_lists_are_an_ordered_subset_of_substrings() {
        let actual = received(&["Alpha one", "Beta two", "Gamma three"]);
        assert!(matches_text_list(
            &actual,
            &expected(&["Beta"]),
            TextMatchKind::Contains,
            false
        )
        .unwrap());
        assert!(matches_text_list(
            &actual,
            &expected(&["Alpha", "Gamma"]),
            TextMatchKind::Contains,
            false
        )
        .unwrap());
        assert!(!matches_text_list(
            &actual,
            &expected(&["Gamma", "Alpha"]),
            TextMatchKind::Contains,
            false
        )
        .unwrap());
        assert!(!matches_text_list(
            &actual,
            &expected(&["Alpha", "Delta"]),
            TextMatchKind::Contains,
            false
        )
        .unwrap());
        assert!(
            matches_text_list(&actual, &expected(&["BETA"]), TextMatchKind::Contains, true)
                .unwrap()
        );
        assert!(matches_text_list(&actual, &[], TextMatchKind::Contains, false).unwrap());
    }

    #[test]
    fn text_lists_accept_regex_entries_alongside_plain_strings() {
        let actual = received(&["Order 42", "Order 7"]);
        assert!(matches_text_list(
            &actual,
            &[
                BrowserExpectedText::Regex(LocatorRegex {
                    source: r"^Order \d+$".to_string(),
                    flags: String::new()
                }),
                text("Order 7"),
            ],
            TextMatchKind::Exact,
            false,
        )
        .unwrap());
    }

    #[test]
    fn class_and_json_matching_preserve_token_and_value_semantics() {
        assert!(matches_class_list("button primary large", "PRIMARY", true));
        assert!(matches_class_list(
            "button primary large",
            "large primary",
            false
        ));
        assert!(!matches_class_list(
            "button primary-large",
            "primary",
            false
        ));
        assert!(matches_json_property(
            &serde_json::json!({"ready": true}),
            &serde_json::json!({"ready": true})
        ));
        assert!(!matches_json_property(
            &serde_json::json!(["one", "two"]),
            &serde_json::json!(["two", "one"])
        ));
    }

    #[test]
    fn aria_template_uses_name_and_property_wildcards_and_ordered_containment() {
        let actual = r#"- navigation "Primary":
  - link "Guide":
    - /url: /guide
  - button "Save"
  - button "Cancel""#;
        let expected = r#"- navigation:
  - link:
    - /url: /guide
  - button "Cancel""#;
        assert!(match_aria_snapshot(expected, actual).unwrap());
    }

    #[test]
    fn aria_template_equal_children_rejects_extra_children_and_diff_keeps_actual() {
        let actual = "- list:\n  - listitem \"One\"\n  - listitem \"Two\"";
        let expected = "- list:\n  - /children: equal\n  - listitem";
        assert!(!match_aria_snapshot(expected, actual).unwrap());
        let diff = aria_snapshot_diff(expected, actual);
        assert!(diff.contains("--- expected"));
        assert!(diff.contains("+   - listitem \"Two\""));
    }

    #[test]
    fn aria_template_supports_regex_names_and_escaped_quotes() {
        assert!(match_aria_snapshot(r#"- button /Save \d+/"#, r#"- button "Save 42""#).unwrap());
        assert!(match_aria_snapshot(
            r#"- button "Save \"draft\"""#,
            r#"- button "Save \"draft\"""#
        )
        .unwrap());
    }
}
