use serde::{Deserialize, Serialize};

use crate::browser_models::{BrowserLocator, BrowserScreenshotOptions, BrowserStep};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DeviceType {
    Desktop,
    Mobile,
    Tablet,
}

impl std::fmt::Display for DeviceType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            DeviceType::Desktop => write!(f, "desktop"),
            DeviceType::Mobile => write!(f, "mobile"),
            DeviceType::Tablet => write!(f, "tablet"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum BrowserAction {
    OpenTab {
        tab_id: String,
        device: DeviceType,
    },
    NavigateTo {
        tab_id: String,
        url: String,
    },
    ScrollTo {
        tab_id: String,
        selector: String,
    },
    Screenshot {
        tab_id: String,
    },
    Html {
        tab_id: String,
        selector: String,
    },
    Reload {
        tab_id: String,
    },
    ClickAtElement {
        tab_id: String,
        selector: String,
    },
    ClickAtPoint {
        tab_id: String,
        x: f64,
        y: f64,
    },
    TypeText {
        tab_id: String,
        text: String,
    },
    FillField {
        tab_id: String,
        selector: String,
        text: String,
    },
    PressKey {
        tab_id: String,
        key: String,
        modifiers: Vec<String>,
    },
    TabLog {
        tab_id: String,
    },
    Eval {
        tab_id: String,
        expression: String,
    },
    Styles {
        tab_id: String,
        selector: String,
        property_filter: String,
    },
    WaitFor {
        tab_id: String,
        seconds: f64,
    },
    WaitForSelector {
        tab_id: String,
        selector: String,
    },
    WaitForNavigation {
        tab_id: String,
    },
    ListTabs,
    CloseTab {
        tab_id: String,
    },
}

pub fn normalize_key_name(key: &str) -> &str {
    match key {
        "Return" => "Enter",
        "Esc" => "Escape",
        "Del" => "Delete",
        "BS" => "Backspace",
        "Up" => "ArrowUp",
        "Down" => "ArrowDown",
        "Left" => "ArrowLeft",
        "Right" => "ArrowRight",
        other => other,
    }
}

fn rest_after_tokens(line: &str, n: usize) -> Option<String> {
    let mut pos = 0;
    let bytes = line.as_bytes();
    let len = bytes.len();

    for _ in 0..n {
        while pos < len && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= len {
            return None;
        }
        while pos < len && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
    }
    while pos < len && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    if pos >= len {
        return None;
    }
    Some(line[pos..].to_string())
}

fn first_tokens(line: &str, n: usize) -> Vec<String> {
    line.split_whitespace()
        .take(n)
        .map(|s| s.to_string())
        .collect()
}

fn parse_device(s: &str) -> Result<DeviceType, String> {
    match s {
        "desktop" => Ok(DeviceType::Desktop),
        "mobile" => Ok(DeviceType::Mobile),
        "tablet" => Ok(DeviceType::Tablet),
        other => Err(format!(
            "Unknown device type: '{}'. Use: desktop, mobile, tablet",
            other
        )),
    }
}

fn parse_modifier(m: &str) -> Result<String, String> {
    match m.trim() {
        "Alt" => Ok("Alt".into()),
        "Ctrl" | "Control" => Ok("Ctrl".into()),
        "Meta" | "Cmd" | "Command" => Ok("Meta".into()),
        "Shift" => Ok("Shift".into()),
        other => Err(format!(
            "Unknown modifier: '{}'. Use: Alt, Ctrl, Meta, Shift",
            other
        )),
    }
}

fn detect_heredoc(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_end();
    for (i, _) in line.match_indices("<<") {
        if i > 0 && !line.as_bytes()[i - 1].is_ascii_whitespace() {
            continue;
        }
        let after = line[i + 2..].trim();
        if after.len() >= 2
            && after
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            let prefix = line[..i].trim();
            if !prefix.is_empty() {
                return Some((prefix, after));
            }
        }
    }
    None
}

fn reject_extra_tokens(line: &str, max_tokens: usize, cmd: &str) -> Result<(), String> {
    let count = line.split_whitespace().count();
    if count > max_tokens {
        Err(format!("{}: too many arguments", cmd))
    } else {
        Ok(())
    }
}

pub fn parse_command(line: &str) -> Result<BrowserAction, String> {
    let trimmed = line.trim();

    if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
        return Err(String::new());
    }

    let tokens = first_tokens(trimmed, 4);
    if tokens.is_empty() {
        return Err("Empty command".to_string());
    }

    let cmd = tokens[0].as_str();
    let tab_id = tokens.get(1).cloned();

    match cmd {
        "open_tab" => {
            reject_extra_tokens(trimmed, 3, "open_tab")?;
            let tab_id = tab_id.ok_or("open_tab: missing <tab_id>")?;
            let device_str = tokens
                .get(2)
                .ok_or("open_tab: missing device type (desktop|mobile|tablet)")?;
            Ok(BrowserAction::OpenTab {
                tab_id,
                device: parse_device(device_str)?,
            })
        }

        "navigate_to" => {
            let tab_id = tab_id.ok_or("navigate_to: missing <tab_id>")?;
            let url = rest_after_tokens(trimmed, 2).ok_or("navigate_to: missing <url>")?;
            Ok(BrowserAction::NavigateTo { tab_id, url })
        }

        "scroll_to" => {
            let tab_id = tab_id.ok_or("scroll_to: missing <tab_id>")?;
            let selector = rest_after_tokens(trimmed, 2).ok_or("scroll_to: missing <selector>")?;
            Ok(BrowserAction::ScrollTo { tab_id, selector })
        }

        "screenshot" => {
            reject_extra_tokens(trimmed, 2, "screenshot")?;
            let tab_id = tab_id.ok_or("screenshot: missing <tab_id>")?;
            Ok(BrowserAction::Screenshot { tab_id })
        }

        "html" => {
            let tab_id = tab_id.ok_or("html: missing <tab_id>")?;
            let selector = rest_after_tokens(trimmed, 2).ok_or("html: missing <selector>")?;
            Ok(BrowserAction::Html { tab_id, selector })
        }

        "reload" => {
            reject_extra_tokens(trimmed, 2, "reload")?;
            let tab_id = tab_id.ok_or("reload: missing <tab_id>")?;
            Ok(BrowserAction::Reload { tab_id })
        }

        "click_at_element" => {
            let tab_id = tab_id.ok_or("click_at_element: missing <tab_id>")?;
            let selector =
                rest_after_tokens(trimmed, 2).ok_or("click_at_element: missing <selector>")?;
            Ok(BrowserAction::ClickAtElement { tab_id, selector })
        }

        "click_at_point" => {
            reject_extra_tokens(trimmed, 4, "click_at_point")?;
            let tab_id = tab_id.ok_or("click_at_point: missing <tab_id>")?;
            let x_str = tokens.get(2).ok_or("click_at_point: missing <x>")?;
            let y_str = tokens.get(3).ok_or("click_at_point: missing <y>")?;
            let x = x_str
                .parse::<f64>()
                .map_err(|e| format!("click_at_point: invalid x: {}", e))?;
            let y = y_str
                .parse::<f64>()
                .map_err(|e| format!("click_at_point: invalid y: {}", e))?;
            Ok(BrowserAction::ClickAtPoint { tab_id, x, y })
        }

        "type_text_at" => {
            let tab_id = tab_id.ok_or("type_text_at: missing <tab_id>")?;
            let text = rest_after_tokens(trimmed, 2).ok_or("type_text_at: missing <text>")?;
            Ok(BrowserAction::TypeText { tab_id, text })
        }

        "fill_field" => {
            let tab_id = tab_id.ok_or("fill_field: missing <tab_id>")?;

            let after_tab_id =
                rest_after_tokens(trimmed, 2).ok_or("fill_field: missing <selector> <text>")?;
            let (selector, text) = if after_tab_id.starts_with('"') {
                let bytes = after_tab_id.as_bytes();
                let mut sel = String::new();
                let mut i = 1;
                let mut closed = false;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        sel.push(bytes[i + 1] as char);
                        i += 2;
                    } else if bytes[i] == b'"' {
                        closed = true;
                        i += 1;
                        break;
                    } else {
                        sel.push(bytes[i] as char);
                        i += 1;
                    }
                }
                if !closed {
                    return Err("fill_field: unclosed quote in selector".to_string());
                }
                let rest = after_tab_id[i..].trim().to_string();
                if rest.is_empty() {
                    return Err("fill_field: missing <text> after quoted selector".to_string());
                }
                (sel, rest)
            } else {
                let selector = tokens
                    .get(2)
                    .ok_or("fill_field: missing <selector>")?
                    .clone();
                let text = rest_after_tokens(trimmed, 3).ok_or("fill_field: missing <text>")?;
                (selector, text)
            };
            Ok(BrowserAction::FillField {
                tab_id,
                selector,
                text,
            })
        }

        "press_key" => {
            reject_extra_tokens(trimmed, 4, "press_key")?;
            let tab_id = tab_id.ok_or("press_key: missing <tab_id>")?;
            let key_raw = tokens.get(2).ok_or("press_key: missing <key>")?;
            let key = normalize_key_name(key_raw).to_string();
            let modifiers = match tokens.get(3) {
                Some(mods) => mods
                    .split(',')
                    .map(|m| parse_modifier(m))
                    .collect::<Result<Vec<_>, _>>()?,
                None => vec![],
            };
            Ok(BrowserAction::PressKey {
                tab_id,
                key,
                modifiers,
            })
        }

        "tab_log" => {
            reject_extra_tokens(trimmed, 2, "tab_log")?;
            let tab_id = tab_id.ok_or("tab_log: missing <tab_id>")?;
            Ok(BrowserAction::TabLog { tab_id })
        }

        "eval" => {
            let tab_id = tab_id.ok_or("eval: missing <tab_id>")?;
            let expression = rest_after_tokens(trimmed, 2).ok_or("eval: missing <expression>")?;
            Ok(BrowserAction::Eval { tab_id, expression })
        }

        "styles" => {
            let tab_id = tab_id.ok_or("styles: missing <tab_id>")?;
            let rest = rest_after_tokens(trimmed, 2).ok_or("styles: missing <selector>")?;

            let (selector, property_filter) = if let Some(pos) = rest.find(" --filter ") {
                (rest[..pos].to_string(), rest[pos + 10..].trim().to_string())
            } else if rest.ends_with(" --filter") {
                (rest[..rest.len() - 9].trim().to_string(), String::new())
            } else {
                (rest, String::new())
            };
            Ok(BrowserAction::Styles {
                tab_id,
                selector,
                property_filter,
            })
        }

        "wait_for" => {
            reject_extra_tokens(trimmed, 3, "wait_for")?;
            let tab_id = tab_id.ok_or("wait_for: missing <tab_id>")?;
            let secs_str = tokens.get(2).ok_or("wait_for: missing <seconds> (1-10)")?;
            let seconds = secs_str
                .parse::<f64>()
                .map_err(|e| format!("wait_for: invalid seconds: {}", e))?;
            if !(0.5..=10.0).contains(&seconds) {
                return Err(format!(
                    "wait_for: seconds should be between 0.5 and 10, got {}",
                    seconds
                ));
            }
            Ok(BrowserAction::WaitFor { tab_id, seconds })
        }

        "wait_for_selector" => {
            let tab_id = tab_id.ok_or("wait_for_selector: missing <tab_id>")?;
            let selector = rest_after_tokens(trimmed, 2)
                .ok_or("wait_for_selector: missing <element_selector>")?;
            Ok(BrowserAction::WaitForSelector { tab_id, selector })
        }

        "wait_for_navigation" => {
            reject_extra_tokens(trimmed, 2, "wait_for_navigation")?;
            let tab_id = tab_id.ok_or("wait_for_navigation: missing <tab_id>")?;
            Ok(BrowserAction::WaitForNavigation { tab_id })
        }

        "list_tabs" => {
            reject_extra_tokens(trimmed, 1, "list_tabs")?;
            Ok(BrowserAction::ListTabs)
        }

        "close_tab" => {
            reject_extra_tokens(trimmed, 2, "close_tab")?;
            let tab_id = tab_id.ok_or("close_tab: missing <tab_id>")?;
            Ok(BrowserAction::CloseTab { tab_id })
        }

        other => Err(format!(
            "Unknown command: '{}'. Available: open_tab, navigate_to, screenshot, \
             html, scroll_to, reload, click_at_element, click_at_point, type_text_at, \
             press_key, tab_log, eval, styles, wait_for, wait_for_selector, \
             wait_for_navigation, list_tabs, close_tab",
            other
        )),
    }
}

pub fn parse_commands(commands_str: &str) -> Vec<Result<BrowserAction, String>> {
    let lines: Vec<&str> = commands_str.lines().collect();
    let mut results = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        if let Some((prefix, marker)) = detect_heredoc(trimmed) {
            let mut body_lines = Vec::new();
            i += 1;
            let mut found_end = false;
            while i < lines.len() {
                if lines[i].trim() == marker {
                    found_end = true;
                    i += 1;
                    break;
                }
                body_lines.push(lines[i]);
                i += 1;
            }
            if !found_end {
                results.push(Err(format!(
                    "Unterminated heredoc: expected closing '{}' marker",
                    marker
                )));
            } else {
                let body = body_lines.join("\n");
                let full_command = format!("{} {}", prefix, body);
                results.push(parse_command(&full_command));
            }
        } else {
            results.push(parse_command(trimmed));
            i += 1;
        }
    }

    results
}

pub fn get_tab_id(action: &BrowserAction) -> Option<&str> {
    match action {
        BrowserAction::OpenTab { tab_id, .. }
        | BrowserAction::NavigateTo { tab_id, .. }
        | BrowserAction::ScrollTo { tab_id, .. }
        | BrowserAction::Screenshot { tab_id, .. }
        | BrowserAction::Html { tab_id, .. }
        | BrowserAction::Reload { tab_id, .. }
        | BrowserAction::ClickAtElement { tab_id, .. }
        | BrowserAction::ClickAtPoint { tab_id, .. }
        | BrowserAction::TypeText { tab_id, .. }
        | BrowserAction::FillField { tab_id, .. }
        | BrowserAction::PressKey { tab_id, .. }
        | BrowserAction::TabLog { tab_id, .. }
        | BrowserAction::Eval { tab_id, .. }
        | BrowserAction::Styles { tab_id, .. }
        | BrowserAction::WaitFor { tab_id, .. }
        | BrowserAction::WaitForSelector { tab_id, .. }
        | BrowserAction::WaitForNavigation { tab_id, .. }
        | BrowserAction::CloseTab { tab_id, .. } => Some(tab_id.as_str()),
        BrowserAction::ListTabs => None,
    }
}

pub fn to_browser_steps(action: &BrowserAction) -> Option<Vec<BrowserStep>> {
    match action {
        BrowserAction::NavigateTo { url, .. } => Some(vec![BrowserStep::Navigate {
            url: url.clone(),
            timeout_ms: None,
        }]),
        BrowserAction::ScrollTo { selector, .. } => Some(vec![BrowserStep::ScrollTo {
            locator: BrowserLocator::css(selector),
        }]),
        BrowserAction::Screenshot { .. } => Some(vec![BrowserStep::Screenshot {
            options: BrowserScreenshotOptions::default(),
        }]),
        BrowserAction::Reload { .. } => Some(vec![BrowserStep::Reload]),
        BrowserAction::ClickAtElement { selector, .. } => Some(vec![BrowserStep::Click {
            locator: BrowserLocator::css(selector),
            timeout_ms: None,
        }]),
        BrowserAction::PressKey { key, modifiers, .. } => Some(vec![BrowserStep::PressKey {
            key: key.clone(),
            modifiers: modifiers.clone(),
        }]),
        BrowserAction::TabLog { .. } => Some(vec![BrowserStep::TabLog]),
        BrowserAction::Eval { expression, .. } => Some(vec![BrowserStep::Eval {
            expression: expression.clone(),
            timeout_ms: None,
        }]),
        BrowserAction::Styles {
            selector,
            property_filter,
            ..
        } => Some(vec![BrowserStep::Styles {
            locator: BrowserLocator::css(selector),
            property_filter: if property_filter.is_empty() {
                None
            } else {
                Some(property_filter.clone())
            },
        }]),
        BrowserAction::WaitFor { seconds, .. } => {
            Some(vec![BrowserStep::WaitSeconds { seconds: *seconds }])
        }
        BrowserAction::WaitForSelector { selector, .. } => {
            Some(vec![BrowserStep::WaitForSelector {
                locator: BrowserLocator::css(selector),
                state: None,
                timeout_ms: None,
            }])
        }
        BrowserAction::WaitForNavigation { .. } => {
            Some(vec![BrowserStep::WaitForNavigation { timeout_ms: None }])
        }
        BrowserAction::Html { selector, .. } => Some(vec![BrowserStep::DomSnapshot {
            selector: selector.clone(),
            max_chars: Some(3000),
        }]),
        BrowserAction::FillField { selector, text, .. } => Some(vec![BrowserStep::Fill {
            locator: BrowserLocator::css(selector),
            text: text.clone(),
            clear_first: true,
            verify: true,
        }]),

        BrowserAction::OpenTab { .. }
        | BrowserAction::CloseTab { .. }
        | BrowserAction::ListTabs
        | BrowserAction::ClickAtPoint { .. }
        | BrowserAction::TypeText { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eval_preserves_quotes() {
        let action =
            parse_command(r#"eval 1 "document.querySelectorAll('button').length""#).unwrap();
        match action {
            BrowserAction::Eval { expression, .. } => {
                assert_eq!(
                    expression,
                    r#""document.querySelectorAll('button').length""#
                );
            }
            _ => panic!("Expected Eval"),
        }
    }

    #[test]
    fn test_heredoc_type_text_multiline() {
        let input = "type_text_at 1 <<END\nLine one\nLine two\nEND";
        let results = parse_commands(input);
        assert_eq!(results.len(), 1);
        match results[0].as_ref().unwrap() {
            BrowserAction::TypeText { text, .. } => {
                assert_eq!(text, "Line one\nLine two");
            }
            _ => panic!("Expected TypeText"),
        }
    }

    #[test]
    fn test_heredoc_unterminated() {
        let input = "eval 1 <<EOF\nsome code\nno closing marker";
        let results = parse_commands(input);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
        assert!(results[0].as_ref().unwrap_err().contains("Unterminated"));
    }

    #[test]
    fn test_convert_wait_selector_preserves_complex_css() {
        let action = parse_command("wait_for_selector 1 tr:nth-child(1) td:nth-child(3)").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        match &steps[0] {
            BrowserStep::WaitForSelector { locator, .. } => {
                let json = serde_json::to_value(locator).unwrap();
                assert_eq!(
                    json["value"].as_str().unwrap(),
                    "tr:nth-child(1) td:nth-child(3)"
                );
            }
            _ => panic!("Expected WaitForSelector"),
        }
    }

    #[test]
    fn test_parse_fill_field_quoted_selector() {
        let action = parse_command(r#"fill_field 1 "form input[name=q]" hello world"#).unwrap();
        match action {
            BrowserAction::FillField {
                tab_id,
                selector,
                text,
            } => {
                assert_eq!(tab_id, "1");
                assert_eq!(selector, "form input[name=q]");
                assert_eq!(text, "hello world");
            }
            _ => panic!("Expected FillField"),
        }
    }

    #[test]
    fn test_parse_fill_field_unclosed_quote_error() {
        let result = parse_command(r#"fill_field 1 "unclosed selector text"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unclosed quote"));
    }

    #[test]
    fn test_parse_fill_field_missing_text_after_quoted_selector() {
        let result = parse_command(r#"fill_field 1 "input[name=q]""#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing <text>"));
    }

    #[test]
    fn test_fill_field_converts_to_fill_step() {
        let action = parse_command("fill_field 1 [name=q] hello").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        match &steps[0] {
            BrowserStep::Fill {
                locator,
                text,
                clear_first,
                verify,
            } => {
                let json = serde_json::to_value(locator).unwrap();
                assert_eq!(json["by"], "css");
                assert_eq!(json["value"], "[name=q]");
                assert_eq!(text, "hello");
                assert!(*clear_first);
                assert!(*verify);
            }
            _ => panic!("Expected Fill"),
        }
    }
}
