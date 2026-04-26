use serde::{Deserialize, Serialize};

use crate::integrations::browser_models::{BrowserLocator, BrowserStep};

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
        BrowserAction::NavigateTo { url, .. } => {
            Some(vec![BrowserStep::Navigate { url: url.clone() }])
        }
        BrowserAction::ScrollTo { selector, .. } => Some(vec![BrowserStep::ScrollTo {
            locator: BrowserLocator::css(selector),
        }]),
        BrowserAction::Screenshot { .. } => Some(vec![BrowserStep::Screenshot]),
        BrowserAction::Reload { .. } => Some(vec![BrowserStep::Reload]),
        BrowserAction::ClickAtElement { selector, .. } => Some(vec![BrowserStep::Click {
            locator: BrowserLocator::css(selector),
        }]),
        BrowserAction::PressKey { key, modifiers, .. } => Some(vec![BrowserStep::PressKey {
            key: key.clone(),
            modifiers: modifiers.clone(),
        }]),
        BrowserAction::TabLog { .. } => Some(vec![BrowserStep::TabLog]),
        BrowserAction::Eval { expression, .. } => Some(vec![BrowserStep::Eval {
            expression: expression.clone(),
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
    fn test_eval_with_logical_operators() {
        let action = parse_command(
            "eval 1 document.querySelector('button') && document.querySelector('button').textContent",
        )
        .unwrap();
        match action {
            BrowserAction::Eval { tab_id, expression } => {
                assert_eq!(tab_id, "1");
                assert!(expression.contains("&&"));
                assert!(expression.contains("textContent"));
            }
            _ => panic!("Expected Eval"),
        }
    }

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
    fn test_eval_string_literal() {
        let action = parse_command("eval 1 'hello'").unwrap();
        match action {
            BrowserAction::Eval { expression, .. } => {
                assert_eq!(expression, "'hello'");
            }
            _ => panic!("Expected Eval"),
        }
    }

    #[test]
    fn test_eval_unquoted_typeof() {
        let action = parse_command("eval 1 typeof document").unwrap();
        match action {
            BrowserAction::Eval { expression, .. } => {
                assert_eq!(expression, "typeof document");
            }
            _ => panic!("Expected Eval"),
        }
    }

    #[test]
    fn test_eval_window_scroll() {
        let action =
            parse_command("eval 1 window.scrollTo(0, document.body.scrollHeight)").unwrap();
        match action {
            BrowserAction::Eval { expression, .. } => {
                assert!(expression.contains("scrollTo"));
            }
            _ => panic!("Expected Eval"),
        }
    }

    #[test]
    fn test_eval_semicolon_expression() {
        let action = parse_command("eval 1 var x=1;x+1").unwrap();
        match action {
            BrowserAction::Eval { expression, .. } => {
                assert_eq!(expression, "var x=1;x+1");
            }
            _ => panic!("Expected Eval"),
        }
    }

    #[test]
    fn test_eval_vue_check() {
        let action =
            parse_command("eval 1 window.__VUE_DEVTOOLS_GLOBAL_HOOK__ !== undefined").unwrap();
        match action {
            BrowserAction::Eval { expression, .. } => {
                assert!(expression.contains("!=="));
            }
            _ => panic!("Expected Eval"),
        }
    }

    #[test]
    fn test_click_complex_css_selector_with_commas() {
        let action = parse_command(
            r#"click_at_element 1 button[data-testid="accept-all-cookies"], [id*="accept"], .accept-all"#,
        )
        .unwrap();
        match action {
            BrowserAction::ClickAtElement { selector, .. } => {
                assert!(selector.contains(","));
                assert!(selector.contains("accept-all"));
            }
            _ => panic!("Expected ClickAtElement"),
        }
    }

    #[test]
    fn test_click_id_selector_with_hash() {
        let action = parse_command("click_at_element 1 #onetrust-accept-btn-handler").unwrap();
        match action {
            BrowserAction::ClickAtElement { selector, .. } => {
                assert_eq!(selector, "#onetrust-accept-btn-handler");
            }
            _ => panic!("Expected ClickAtElement"),
        }
    }

    #[test]
    fn test_click_nth_child_selector() {
        let action = parse_command("click_at_element 2 tr:nth-child(1) td:nth-child(3)").unwrap();
        match action {
            BrowserAction::ClickAtElement { selector, .. } => {
                assert_eq!(selector, "tr:nth-child(1) td:nth-child(3)");
            }
            _ => panic!("Expected ClickAtElement"),
        }
    }

    #[test]
    fn test_html_id_selector() {
        let action = parse_command("html 1 #loginForm").unwrap();
        match action {
            BrowserAction::Html { selector, .. } => {
                assert_eq!(selector, "#loginForm");
            }
            _ => panic!("Expected Html"),
        }
    }

    #[test]
    fn test_html_id_with_hash() {
        let action = parse_command("html 1 #api").unwrap();
        match action {
            BrowserAction::Html { selector, .. } => {
                assert_eq!(selector, "#api");
            }
            _ => panic!("Expected Html"),
        }
    }

    #[test]
    fn test_scroll_to_id_selector() {
        let action = parse_command("scroll_to 1 #email").unwrap();
        match action {
            BrowserAction::ScrollTo { selector, .. } => {
                assert_eq!(selector, "#email");
            }
            _ => panic!("Expected ScrollTo"),
        }
    }

    #[test]
    fn test_scroll_to_complex_selector() {
        let action = parse_command(
            r#"scroll_to 1 a[href*="register"], a[href*="signup"], a[href*="create"]"#,
        )
        .unwrap();
        match action {
            BrowserAction::ScrollTo { selector, .. } => {
                assert!(selector.contains(","));
            }
            _ => panic!("Expected ScrollTo"),
        }
    }

    #[test]
    fn test_type_text_with_spaces() {
        let action = parse_command("type_text_at 1 Pearson Test Institute").unwrap();
        match action {
            BrowserAction::TypeText { text, .. } => {
                assert_eq!(text, "Pearson Test Institute");
            }
            _ => panic!("Expected TypeText"),
        }
    }

    #[test]
    fn test_styles_with_filter_separator() {
        let action = parse_command(r#"styles 1 [style*="aspect-ratio"] --filter color"#).unwrap();
        match action {
            BrowserAction::Styles {
                selector,
                property_filter,
                ..
            } => {
                assert!(selector.contains("aspect-ratio"));
                assert_eq!(property_filter, "color");
            }
            _ => panic!("Expected Styles"),
        }
    }

    #[test]
    fn test_styles_no_filter() {
        let action = parse_command("styles 1 body").unwrap();
        match action {
            BrowserAction::Styles {
                selector,
                property_filter,
                ..
            } => {
                assert_eq!(selector, "body");
                assert_eq!(property_filter, "");
            }
            _ => panic!("Expected Styles"),
        }
    }

    #[test]
    fn test_styles_descendant_selector_no_ambiguity() {
        let action = parse_command("styles 1 tr:nth-child(1) td:nth-child(3)").unwrap();
        match action {
            BrowserAction::Styles {
                selector,
                property_filter,
                ..
            } => {
                assert_eq!(selector, "tr:nth-child(1) td:nth-child(3)");
                assert_eq!(property_filter, "");
            }
            _ => panic!("Expected Styles"),
        }
    }

    #[test]
    fn test_styles_descendant_selector_with_filter() {
        let action = parse_command("styles 1 div.container p.text --filter margin").unwrap();
        match action {
            BrowserAction::Styles {
                selector,
                property_filter,
                ..
            } => {
                assert_eq!(selector, "div.container p.text");
                assert_eq!(property_filter, "margin");
            }
            _ => panic!("Expected Styles"),
        }
    }

    #[test]
    fn test_press_key_return_normalized_to_enter() {
        let action = parse_command("press_key 1 Return").unwrap();
        match action {
            BrowserAction::PressKey { key, .. } => {
                assert_eq!(key, "Enter");
            }
            _ => panic!("Expected PressKey"),
        }
    }

    #[test]
    fn test_press_key_with_modifiers() {
        let action = parse_command("press_key 1 a Ctrl,Shift").unwrap();
        match action {
            BrowserAction::PressKey { key, modifiers, .. } => {
                assert_eq!(key, "a");
                assert_eq!(modifiers, vec!["Ctrl", "Shift"]);
            }
            _ => panic!("Expected PressKey"),
        }
    }

    #[test]
    fn test_press_key_command_alias() {
        let action = parse_command("press_key 1 Tab Cmd").unwrap();
        match action {
            BrowserAction::PressKey { modifiers, .. } => {
                assert_eq!(modifiers, vec!["Meta"]);
            }
            _ => panic!("Expected PressKey"),
        }
    }

    #[test]
    fn test_wait_for_valid() {
        let action = parse_command("wait_for 1 3").unwrap();
        match action {
            BrowserAction::WaitFor { seconds, .. } => {
                assert!((seconds - 3.0).abs() < f64::EPSILON);
            }
            _ => panic!("Expected WaitFor"),
        }
    }

    #[test]
    fn test_wait_for_out_of_range() {
        assert!(parse_command("wait_for 1 0.1").is_err());
        assert!(parse_command("wait_for 1 20").is_err());
    }

    #[test]
    fn test_navigate_url_with_query_params() {
        let action = parse_command("navigate_to 1 https://example.com/path?q=1&r=2#frag").unwrap();
        match action {
            BrowserAction::NavigateTo { url, .. } => {
                assert!(url.contains("?q=1&r=2#frag"));
            }
            _ => panic!("Expected NavigateTo"),
        }
    }

    #[test]
    fn test_blank_lines_and_comments_skipped() {
        let results =
            parse_commands("open_tab 1 desktop\n\n// comment\n# another comment\nscreenshot 1");
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.is_ok()));
    }

    #[test]
    fn test_unknown_command_clear_error() {
        let err = parse_command("foobar 1").unwrap_err();
        assert!(err.contains("Unknown command"));
        assert!(err.contains("foobar"));
    }

    #[test]
    fn test_rest_after_tokens() {
        assert_eq!(
            rest_after_tokens("eval 1 hello world", 2),
            Some("hello world".to_string())
        );
        assert_eq!(rest_after_tokens("screenshot 1", 2), None);
        assert_eq!(rest_after_tokens("cmd", 1), None);
    }

    #[test]
    fn test_key_normalization() {
        assert_eq!(normalize_key_name("Return"), "Enter");
        assert_eq!(normalize_key_name("Esc"), "Escape");
        assert_eq!(normalize_key_name("Del"), "Delete");
        assert_eq!(normalize_key_name("Up"), "ArrowUp");
        assert_eq!(normalize_key_name("Enter"), "Enter");
        assert_eq!(normalize_key_name("Tab"), "Tab");
    }

    #[test]
    fn test_open_tab_missing_device() {
        let err = parse_command("open_tab 1").unwrap_err();
        assert!(err.contains("missing device type"));
    }

    #[test]
    fn test_click_at_point() {
        let action = parse_command("click_at_point 1 100.5 200.3").unwrap();
        match action {
            BrowserAction::ClickAtPoint { x, y, .. } => {
                assert!((x - 100.5).abs() < f64::EPSILON);
                assert!((y - 200.3).abs() < f64::EPSILON);
            }
            _ => panic!("Expected ClickAtPoint"),
        }
    }

    #[test]
    fn test_wait_for_selector_id() {
        let action = parse_command("wait_for_selector 1 #login-button").unwrap();
        match action {
            BrowserAction::WaitForSelector { tab_id, selector } => {
                assert_eq!(tab_id, "1");
                assert_eq!(selector, "#login-button");
            }
            _ => panic!("Expected WaitForSelector"),
        }
    }

    #[test]
    fn test_wait_for_selector_complex_css() {
        let action =
            parse_command("wait_for_selector 1 button[data-testid='submit'], .form-submit")
                .unwrap();
        match action {
            BrowserAction::WaitForSelector { selector, .. } => {
                assert!(selector.contains("data-testid"));
                assert!(selector.contains(","));
            }
            _ => panic!("Expected WaitForSelector"),
        }
    }

    #[test]
    fn test_wait_for_selector_missing_selector() {
        let err = parse_command("wait_for_selector 1").unwrap_err();
        assert!(err.contains("missing"));
    }

    #[test]
    fn test_wait_for_navigation() {
        let action = parse_command("wait_for_navigation 1").unwrap();
        match action {
            BrowserAction::WaitForNavigation { tab_id } => {
                assert_eq!(tab_id, "1");
            }
            _ => panic!("Expected WaitForNavigation"),
        }
    }

    #[test]
    fn test_wait_for_navigation_missing_tab() {
        let err = parse_command("wait_for_navigation").unwrap_err();
        assert!(err.contains("missing"));
    }

    #[test]
    fn test_list_tabs() {
        let action = parse_command("list_tabs").unwrap();
        assert!(matches!(action, BrowserAction::ListTabs));
    }

    #[test]
    fn test_close_tab() {
        let action = parse_command("close_tab 1").unwrap();
        match action {
            BrowserAction::CloseTab { tab_id } => assert_eq!(tab_id, "1"),
            _ => panic!("Expected CloseTab"),
        }
    }

    #[test]
    fn test_close_tab_missing_id() {
        let err = parse_command("close_tab").unwrap_err();
        assert!(err.contains("missing"));
    }

    #[test]
    fn test_parse_commands_with_new_commands() {
        let input = "open_tab 1 desktop\nnavigate_to 1 https://example.com\nwait_for_selector 1 #content\nscreenshot 1\nclose_tab 1";
        let results = parse_commands(input);
        assert_eq!(results.len(), 5);
        assert!(results.iter().all(|r| r.is_ok()));
    }

    #[test]
    fn test_list_tabs_in_multi_command() {
        let input = "open_tab 1 desktop\nlist_tabs\nscreenshot 1";
        let results = parse_commands(input);
        assert_eq!(results.len(), 3);
        assert!(matches!(
            results[1].as_ref().unwrap(),
            BrowserAction::ListTabs
        ));
    }

    #[test]
    fn test_heredoc_eval_multiline() {
        let input = "eval 1 <<EOF\n(function() {\n  return document.title;\n})()\nEOF";
        let results = parse_commands(input);
        assert_eq!(results.len(), 1);
        match results[0].as_ref().unwrap() {
            BrowserAction::Eval { expression, .. } => {
                assert!(expression.contains("document.title"));
                assert!(expression.contains('\n'));
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
    fn test_heredoc_mixed_with_regular() {
        let input = "open_tab 1 desktop\neval 1 <<JS\nvar x = 1;\nreturn x;\nJS\nscreenshot 1";
        let results = parse_commands(input);
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.is_ok()));
    }

    #[test]
    fn test_heredoc_not_triggered_by_js_shift() {
        let action = parse_command("eval 1 x << 5").unwrap();
        match action {
            BrowserAction::Eval { expression, .. } => {
                assert_eq!(expression, "x << 5");
            }
            _ => panic!("Expected Eval"),
        }
    }

    #[test]
    fn test_heredoc_not_triggered_without_space() {
        let action = parse_command("eval 1 x<<EOF").unwrap();
        match action {
            BrowserAction::Eval { expression, .. } => {
                assert_eq!(expression, "x<<EOF");
            }
            _ => panic!("Expected Eval"),
        }
    }

    #[test]
    fn test_screenshot_rejects_extra_args() {
        assert!(parse_command("screenshot 1 extra").is_err());
    }

    #[test]
    fn test_reload_rejects_extra_args() {
        assert!(parse_command("reload 1 now").is_err());
    }

    #[test]
    fn test_list_tabs_rejects_extra_args() {
        assert!(parse_command("list_tabs unexpected").is_err());
    }

    #[test]
    fn test_open_tab_rejects_extra_args() {
        assert!(parse_command("open_tab 1 desktop extra").is_err());
    }

    #[test]
    fn test_close_tab_rejects_extra_args() {
        assert!(parse_command("close_tab 1 force").is_err());
    }

    #[test]
    fn test_wait_for_navigation_rejects_extra_args() {
        assert!(parse_command("wait_for_navigation 1 please").is_err());
    }

    #[test]
    fn test_detect_heredoc_basic() {
        assert_eq!(detect_heredoc("eval 1 <<EOF"), Some(("eval 1", "EOF")));
    }

    #[test]
    fn test_detect_heredoc_no_space() {
        assert_eq!(detect_heredoc("eval 1 x<<EOF"), None);
    }

    #[test]
    fn test_detect_heredoc_short_marker() {
        assert_eq!(detect_heredoc("eval 1 <<X"), None); // marker too short
    }

    #[test]
    fn test_detect_heredoc_lowercase_marker() {
        assert_eq!(detect_heredoc("eval 1 <<eof"), None);
    }

    #[test]
    fn test_get_tab_id_navigate() {
        let action = parse_command("navigate_to 1 https://example.com").unwrap();
        assert_eq!(get_tab_id(&action), Some("1"));
    }

    #[test]
    fn test_get_tab_id_list_tabs() {
        let action = parse_command("list_tabs").unwrap();
        assert_eq!(get_tab_id(&action), None);
    }

    #[test]
    fn test_get_tab_id_screenshot() {
        let action = parse_command("screenshot 5").unwrap();
        assert_eq!(get_tab_id(&action), Some("5"));
    }

    #[test]
    fn test_convert_navigate() {
        let action = parse_command("navigate_to 1 https://example.com").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        assert_eq!(steps.len(), 1);
        assert!(matches!(&steps[0], BrowserStep::Navigate { url } if url == "https://example.com"));
    }

    #[test]
    fn test_convert_screenshot() {
        let action = parse_command("screenshot 1").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        assert_eq!(steps.len(), 1);
        assert!(matches!(&steps[0], BrowserStep::Screenshot));
    }

    #[test]
    fn test_convert_reload() {
        let action = parse_command("reload 1").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        assert_eq!(steps.len(), 1);
        assert!(matches!(&steps[0], BrowserStep::Reload));
    }

    #[test]
    fn test_convert_click_at_element() {
        let action = parse_command("click_at_element 1 #btn").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        assert_eq!(steps.len(), 1);
        assert!(matches!(&steps[0], BrowserStep::Click { .. }));
    }

    #[test]
    fn test_convert_scroll_to() {
        let action = parse_command("scroll_to 1 #main").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        assert_eq!(steps.len(), 1);
        assert!(matches!(&steps[0], BrowserStep::ScrollTo { .. }));
    }

    #[test]
    fn test_convert_press_key() {
        let action = parse_command("press_key 1 Enter").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        assert_eq!(steps.len(), 1);
        match &steps[0] {
            BrowserStep::PressKey { key, modifiers } => {
                assert_eq!(key, "Enter");
                assert!(modifiers.is_empty());
            }
            _ => panic!("Expected PressKey"),
        }
    }

    #[test]
    fn test_convert_press_key_with_modifiers() {
        let action = parse_command("press_key 1 a Ctrl,Shift").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        match &steps[0] {
            BrowserStep::PressKey { key, modifiers } => {
                assert_eq!(key, "a");
                assert_eq!(modifiers, &vec!["Ctrl".to_string(), "Shift".to_string()]);
            }
            _ => panic!("Expected PressKey"),
        }
    }

    #[test]
    fn test_convert_eval() {
        let action = parse_command("eval 1 document.title").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        assert!(
            matches!(&steps[0], BrowserStep::Eval { expression } if expression == "document.title")
        );
    }

    #[test]
    fn test_convert_tab_log() {
        let action = parse_command("tab_log 1").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        assert!(matches!(&steps[0], BrowserStep::TabLog));
    }

    #[test]
    fn test_convert_styles() {
        let action = parse_command("styles 1 body --filter color").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        match &steps[0] {
            BrowserStep::Styles {
                property_filter, ..
            } => {
                assert_eq!(property_filter.as_deref(), Some("color"));
            }
            _ => panic!("Expected Styles"),
        }
    }

    #[test]
    fn test_convert_styles_no_filter() {
        let action = parse_command("styles 1 body").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        match &steps[0] {
            BrowserStep::Styles {
                property_filter, ..
            } => {
                assert!(property_filter.is_none());
            }
            _ => panic!("Expected Styles"),
        }
    }

    #[test]
    fn test_convert_wait_for() {
        let action = parse_command("wait_for 1 3").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        assert!(
            matches!(&steps[0], BrowserStep::WaitSeconds { seconds } if (*seconds - 3.0).abs() < f64::EPSILON)
        );
    }

    #[test]
    fn test_convert_wait_for_selector() {
        let action = parse_command("wait_for_selector 1 #loaded").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        assert!(matches!(&steps[0], BrowserStep::WaitForSelector { .. }));
    }

    #[test]
    fn test_convert_wait_for_navigation() {
        let action = parse_command("wait_for_navigation 1").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        assert!(matches!(&steps[0], BrowserStep::WaitForNavigation { .. }));
    }

    #[test]
    fn test_convert_open_tab_returns_none() {
        let action = parse_command("open_tab 1 desktop").unwrap();
        assert!(to_browser_steps(&action).is_none());
    }

    #[test]
    fn test_convert_close_tab_returns_none() {
        let action = parse_command("close_tab 1").unwrap();
        assert!(to_browser_steps(&action).is_none());
    }

    #[test]
    fn test_convert_list_tabs_returns_none() {
        let action = parse_command("list_tabs").unwrap();
        assert!(to_browser_steps(&action).is_none());
    }

    #[test]
    fn test_convert_click_at_point_returns_none() {
        let action = parse_command("click_at_point 1 100 200").unwrap();
        assert!(to_browser_steps(&action).is_none());
    }

    #[test]
    fn test_convert_type_text_returns_none() {
        let action = parse_command("type_text_at 1 hello world").unwrap();
        assert!(to_browser_steps(&action).is_none());
    }

    #[test]
    fn test_convert_html_to_dom_snapshot() {
        let action = parse_command("html 1 #main").unwrap();
        let steps = to_browser_steps(&action).unwrap();
        assert_eq!(steps.len(), 1);
        match &steps[0] {
            BrowserStep::DomSnapshot {
                selector,
                max_chars,
            } => {
                assert_eq!(selector, "#main");
                assert_eq!(*max_chars, Some(3000));
            }
            _ => panic!("Expected DomSnapshot"),
        }
    }

    #[test]
    fn test_convert_click_preserves_complex_selector() {
        let action =
            parse_command(r#"click_at_element 1 button[data-testid="accept"], .accept-all"#)
                .unwrap();
        let steps = to_browser_steps(&action).unwrap();
        match &steps[0] {
            BrowserStep::Click { locator } => {
                let json = serde_json::to_value(locator).unwrap();
                assert_eq!(json["by"], "css");
                let val = json["value"].as_str().unwrap();
                assert!(val.contains(","));
                assert!(val.contains("accept-all"));
            }
            _ => panic!("Expected Click"),
        }
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
    fn test_parse_fill_field_simple() {
        let action = parse_command("fill_field 1 #email user@test.com").unwrap();
        match action {
            BrowserAction::FillField {
                tab_id,
                selector,
                text,
            } => {
                assert_eq!(tab_id, "1");
                assert_eq!(selector, "#email");
                assert_eq!(text, "user@test.com");
            }
            _ => panic!("Expected FillField"),
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
    fn test_parse_fill_field_quoted_descendant_selector() {
        let action =
            parse_command(r#"fill_field 2 "div.search input[type=text]" rust tutorial"#).unwrap();
        match action {
            BrowserAction::FillField {
                tab_id,
                selector,
                text,
            } => {
                assert_eq!(tab_id, "2");
                assert_eq!(selector, "div.search input[type=text]");
                assert_eq!(text, "rust tutorial");
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
