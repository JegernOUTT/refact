use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::integrations::browser_types::{
    RecorderEvent, ConsoleEntry, NetworkEntry, MutationSummaryEntry,
};

const OVERSIZE_THRESHOLD: usize = 100 * 1024;

pub struct BrowserContextSnapshot {
    pub url: String,
    pub title: String,
    pub actions: Vec<RecorderEvent>,
    pub console: Vec<ConsoleEntry>,
    pub network: Vec<NetworkEntry>,
    pub mutations: Vec<MutationSummaryEntry>,
    pub total_bytes: usize,
    pub page_changed: bool,
}

pub struct OversizeInfo {
    pub total_bytes: usize,
    pub action_count: usize,
    pub console_count: usize,
    pub network_count: usize,
}

pub fn format_browser_context(snapshot: &BrowserContextSnapshot) -> String {
    let mut parts = Vec::new();

    parts.push(format!("[Browser Context]\nURL: {}\nTitle: {}", snapshot.url, snapshot.title));

    if !snapshot.actions.is_empty() {
        let mut lines = Vec::new();
        lines.push("\n## User Actions (since last message)".to_string());
        for action in &snapshot.actions {
            lines.push(format_action(action));
        }
        parts.push(lines.join("\n"));
    }

    if !snapshot.console.is_empty() {
        let mut lines = Vec::new();
        lines.push("\n## Console (since last message)".to_string());
        for entry in &snapshot.console {
            lines.push(format_console_entry(entry));
        }
        parts.push(lines.join("\n"));
    }

    if !snapshot.network.is_empty() {
        let mut lines = Vec::new();
        lines.push("\n## Network (since last message)".to_string());
        for entry in &snapshot.network {
            lines.push(format_network_entry(entry));
        }
        parts.push(lines.join("\n"));
    }

    if !snapshot.mutations.is_empty() {
        let mut lines = Vec::new();
        lines.push("\n## DOM Changes (since last message)".to_string());
        let total_added: u32 = snapshot.mutations.iter().map(|m| m.added).sum();
        let total_removed: u32 = snapshot.mutations.iter().map(|m| m.removed).sum();
        let total_changed: u32 = snapshot.mutations.iter().map(|m| m.changed).sum();
        if total_added > 0 {
            lines.push(format!("Added: {} elements", total_added));
        }
        if total_removed > 0 {
            lines.push(format!("Removed: {} elements", total_removed));
        }
        if total_changed > 0 {
            lines.push(format!("Changed: {} elements", total_changed));
        }
        parts.push(lines.join("\n"));
    }

    parts.join("\n")
}

fn format_timestamp(ts: f64) -> String {
    let total_secs = (ts / 1000.0) as u64;
    let hours = (total_secs / 3600) % 24;
    let minutes = (total_secs / 60) % 60;
    let seconds = total_secs % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

fn format_action(action: &RecorderEvent) -> String {
    match action {
        RecorderEvent::Navigation { url, timestamp, .. } => {
            format!("[{}] navigate → {}", format_timestamp(*timestamp), url)
        }
        RecorderEvent::Click { selector, text, x, y, timestamp } => {
            let label = if text.is_empty() { selector.clone() } else { format!("{} \"{}\"", selector, text) };
            format!("[{}] click → {} (x:{}, y:{})", format_timestamp(*timestamp), label, *x as i32, *y as i32)
        }
        RecorderEvent::Input { selector, value, timestamp, .. } => {
            format!("[{}] input → {} \"{}\"", format_timestamp(*timestamp), selector, value)
        }
        RecorderEvent::Keypress { key, modifiers, timestamp } => {
            let mods = if modifiers.is_empty() { String::new() } else { format!("{}+", modifiers.join("+")) };
            format!("[{}] keypress → {}{}", format_timestamp(*timestamp), mods, key)
        }
        RecorderEvent::Submit { selector, method, action, timestamp } => {
            format!("[{}] submit → {} {} {}", format_timestamp(*timestamp), selector, method, action)
        }
        RecorderEvent::Scroll { scroll_x, scroll_y, timestamp } => {
            format!("[{}] scroll → ({}, {})", format_timestamp(*timestamp), *scroll_x as i32, *scroll_y as i32)
        }
        RecorderEvent::MutationSummary { added, removed, changed, timestamp } => {
            format!("[{}] dom-change → +{} -{} ~{}", format_timestamp(*timestamp), added, removed, changed)
        }
    }
}

fn format_console_entry(entry: &ConsoleEntry) -> String {
    format!("[{}] [{}] {}", format_timestamp(entry.timestamp), entry.level, entry.text)
}

fn format_network_entry(entry: &NetworkEntry) -> String {
    let status_str = entry.status.map(|s| format!(" → {}", s)).unwrap_or_default();
    format!("[{}] {} {}{}", format_timestamp(entry.timestamp), entry.method, entry.url, status_str)
}

pub fn compute_context_size(
    actions: &[RecorderEvent],
    console: &[ConsoleEntry],
    network: &[NetworkEntry],
    mutations: &[MutationSummaryEntry],
) -> usize {
    serde_json::to_string(actions).unwrap_or_default().len()
        + serde_json::to_string(console).unwrap_or_default().len()
        + serde_json::to_string(network).unwrap_or_default().len()
        + serde_json::to_string(mutations).unwrap_or_default().len()
}

pub fn cap_to_n(snapshot: &mut BrowserContextSnapshot, n: usize) {
    if snapshot.actions.len() > n {
        let start = snapshot.actions.len() - n;
        snapshot.actions = snapshot.actions[start..].to_vec();
    }
    if snapshot.console.len() > n {
        let start = snapshot.console.len() - n;
        snapshot.console = snapshot.console[start..].to_vec();
    }
    if snapshot.network.len() > n {
        let start = snapshot.network.len() - n;
        snapshot.network = snapshot.network[start..].to_vec();
    }
    snapshot.total_bytes = compute_context_size(
        &snapshot.actions,
        &snapshot.console,
        &snapshot.network,
        &snapshot.mutations,
    );
}

pub async fn get_browser_context_for_chat(
    gcx: Arc<ARwLock<GlobalContext>>,
    chat_id: &str,
) -> Option<BrowserContextSnapshot> {
    let (_, runtime_arc) = crate::integrations::browser_runtime::find_runtime_by_chat_id(gcx, chat_id).await?;
    let rt = runtime_arc.lock().await;

    if !rt.is_connected {
        return None;
    }

    let url = String::new();
    let title = String::new();

    let actions = rt.action_buffer[rt.last_send_action_cursor..].to_vec();
    let console = rt.console_buffer[rt.last_send_console_cursor..].to_vec();
    let network = rt.network_buffer[rt.last_send_network_cursor..].to_vec();
    let mutations = rt.mutation_summary[rt.last_send_mutation_cursor..].to_vec();

    let total_bytes = compute_context_size(&actions, &console, &network, &mutations);

    let page_changed = rt.page_changed();

    if actions.is_empty() && console.is_empty() && network.is_empty() && mutations.is_empty() {
        return None;
    }

    Some(BrowserContextSnapshot {
        url,
        title,
        actions,
        console,
        network,
        mutations,
        total_bytes,
        page_changed,
    })
}

pub async fn commit_browser_cursors(
    gcx: Arc<ARwLock<GlobalContext>>,
    chat_id: &str,
) {
    if let Some((_, runtime_arc)) = crate::integrations::browser_runtime::find_runtime_by_chat_id(gcx, chat_id).await {
        let mut rt = runtime_arc.lock().await;
        rt.commit_cursors();
    }
}

pub async fn maybe_insert_browser_context(
    gcx: Arc<ARwLock<GlobalContext>>,
    chat_id: &str,
    has_browser_meta: bool,
    attach_screenshot_on_send: bool,
) -> Option<(ChatMessage, Option<OversizeInfo>)> {
    if !has_browser_meta {
        return None;
    }

    let snapshot = get_browser_context_for_chat(gcx.clone(), chat_id).await?;

    if snapshot.total_bytes > OVERSIZE_THRESHOLD {
        let oversize = OversizeInfo {
            total_bytes: snapshot.total_bytes,
            action_count: snapshot.actions.len(),
            console_count: snapshot.console.len(),
            network_count: snapshot.network.len(),
        };
        return Some((make_context_message(&snapshot, false), Some(oversize)));
    }

    commit_browser_cursors(gcx, chat_id).await;

    Some((make_context_message(&snapshot, attach_screenshot_on_send && snapshot.page_changed), None))
}

fn make_context_message(snapshot: &BrowserContextSnapshot, _attach_screenshot: bool) -> ChatMessage {
    let text = format_browser_context(snapshot);
    ChatMessage {
        message_id: uuid::Uuid::new_v4().to_string(),
        role: "user".to_string(),
        content: ChatContent::SimpleText(text),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_browser_context_full() {
        let snapshot = BrowserContextSnapshot {
            url: "https://example.com".to_string(),
            title: "Example Page".to_string(),
            actions: vec![
                RecorderEvent::Navigation {
                    url: "https://example.com".to_string(),
                    title: "Example Page".to_string(),
                    timestamp: 45245000.0,
                },
                RecorderEvent::Click {
                    selector: "button.submit".to_string(),
                    text: "Submit Form".to_string(),
                    x: 200.0,
                    y: 350.0,
                    timestamp: 45248000.0,
                },
                RecorderEvent::Input {
                    selector: "input#email".to_string(),
                    value: "user@example.com".to_string(),
                    masked: false,
                    timestamp: 45250000.0,
                },
            ],
            console: vec![
                ConsoleEntry {
                    timestamp: 45246000.0,
                    level: "error".to_string(),
                    text: "Uncaught TypeError: Cannot read property 'x' of null".to_string(),
                },
            ],
            network: vec![
                NetworkEntry {
                    timestamp: 45245000.0,
                    method: "GET".to_string(),
                    url: "https://example.com".to_string(),
                    resource_type: "Document".to_string(),
                    status: Some(200),
                },
            ],
            mutations: vec![
                MutationSummaryEntry {
                    timestamp: 45247000.0,
                    added: 3,
                    removed: 1,
                    changed: 2,
                    descriptions: vec![],
                },
            ],
            total_bytes: 500,
            page_changed: true,
        };

        let formatted = format_browser_context(&snapshot);
        assert!(formatted.contains("[Browser Context]"));
        assert!(formatted.contains("URL: https://example.com"));
        assert!(formatted.contains("Title: Example Page"));
        assert!(formatted.contains("## User Actions (since last message)"));
        assert!(formatted.contains("navigate → https://example.com"));
        assert!(formatted.contains("click → button.submit \"Submit Form\""));
        assert!(formatted.contains("input → input#email \"user@example.com\""));
        assert!(formatted.contains("## Console (since last message)"));
        assert!(formatted.contains("[error] Uncaught TypeError"));
        assert!(formatted.contains("## Network (since last message)"));
        assert!(formatted.contains("GET https://example.com → 200"));
        assert!(formatted.contains("## DOM Changes (since last message)"));
        assert!(formatted.contains("Added: 3 elements"));
        assert!(formatted.contains("Removed: 1 elements"));
        assert!(formatted.contains("Changed: 2 elements"));
    }

    #[test]
    fn test_format_browser_context_empty_sections() {
        let snapshot = BrowserContextSnapshot {
            url: "https://test.com".to_string(),
            title: "Test".to_string(),
            actions: vec![],
            console: vec![],
            network: vec![],
            mutations: vec![],
            total_bytes: 0,
            page_changed: false,
        };

        let formatted = format_browser_context(&snapshot);
        assert!(formatted.contains("[Browser Context]"));
        assert!(formatted.contains("URL: https://test.com"));
        assert!(!formatted.contains("## User Actions"));
        assert!(!formatted.contains("## Console"));
        assert!(!formatted.contains("## Network"));
        assert!(!formatted.contains("## DOM Changes"));
    }

    #[test]
    fn test_format_timestamp() {
        assert_eq!(format_timestamp(0.0), "00:00:00");
        assert_eq!(format_timestamp(45245000.0), "12:34:05");
        assert_eq!(format_timestamp(3661000.0), "01:01:01");
    }

    #[test]
    fn test_format_action_navigation() {
        let action = RecorderEvent::Navigation {
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            timestamp: 1000.0,
        };
        let formatted = format_action(&action);
        assert!(formatted.contains("navigate →"));
        assert!(formatted.contains("https://example.com"));
    }

    #[test]
    fn test_format_action_click_with_text() {
        let action = RecorderEvent::Click {
            selector: "#btn".to_string(),
            text: "OK".to_string(),
            x: 10.0,
            y: 20.0,
            timestamp: 1000.0,
        };
        let formatted = format_action(&action);
        assert!(formatted.contains("click →"));
        assert!(formatted.contains("#btn \"OK\""));
        assert!(formatted.contains("(x:10, y:20)"));
    }

    #[test]
    fn test_format_action_click_no_text() {
        let action = RecorderEvent::Click {
            selector: "#btn".to_string(),
            text: String::new(),
            x: 10.0,
            y: 20.0,
            timestamp: 1000.0,
        };
        let formatted = format_action(&action);
        assert!(formatted.contains("click → #btn (x:10, y:20)"));
    }

    #[test]
    fn test_format_action_submit() {
        let action = RecorderEvent::Submit {
            selector: "form#login".to_string(),
            method: "POST".to_string(),
            action: "/api/login".to_string(),
            timestamp: 1000.0,
        };
        let formatted = format_action(&action);
        assert!(formatted.contains("submit →"));
        assert!(formatted.contains("POST"));
        assert!(formatted.contains("/api/login"));
    }

    #[test]
    fn test_format_action_keypress_with_modifiers() {
        let action = RecorderEvent::Keypress {
            key: "Enter".to_string(),
            modifiers: vec!["Ctrl".to_string(), "Shift".to_string()],
            timestamp: 1000.0,
        };
        let formatted = format_action(&action);
        assert!(formatted.contains("keypress → Ctrl+Shift+Enter"));
    }

    #[test]
    fn test_format_action_scroll() {
        let action = RecorderEvent::Scroll {
            scroll_x: 0.0,
            scroll_y: 500.0,
            timestamp: 1000.0,
        };
        let formatted = format_action(&action);
        assert!(formatted.contains("scroll → (0, 500)"));
    }

    #[test]
    fn test_format_console_entry() {
        let entry = ConsoleEntry {
            timestamp: 1000.0,
            level: "error".to_string(),
            text: "Something failed".to_string(),
        };
        let formatted = format_console_entry(&entry);
        assert!(formatted.contains("[error]"));
        assert!(formatted.contains("Something failed"));
    }

    #[test]
    fn test_format_network_entry_with_status() {
        let entry = NetworkEntry {
            timestamp: 1000.0,
            method: "GET".to_string(),
            url: "https://api.com/data".to_string(),
            resource_type: "Fetch".to_string(),
            status: Some(200),
        };
        let formatted = format_network_entry(&entry);
        assert!(formatted.contains("GET https://api.com/data → 200"));
    }

    #[test]
    fn test_format_network_entry_no_status() {
        let entry = NetworkEntry {
            timestamp: 1000.0,
            method: "POST".to_string(),
            url: "https://api.com/data".to_string(),
            resource_type: "XHR".to_string(),
            status: None,
        };
        let formatted = format_network_entry(&entry);
        assert!(formatted.contains("POST https://api.com/data"));
        assert!(!formatted.contains("→"));
    }

    #[test]
    fn test_compute_context_size_empty() {
        let size = compute_context_size(&[], &[], &[], &[]);
        assert_eq!(size, 8);
    }

    #[test]
    fn test_compute_context_size_with_data() {
        let actions = vec![RecorderEvent::Click {
            selector: "#btn".to_string(),
            text: "OK".to_string(),
            x: 10.0,
            y: 20.0,
            timestamp: 1000.0,
        }];
        let size = compute_context_size(&actions, &[], &[], &[]);
        assert!(size > 8);
    }

    #[test]
    fn test_cap_to_n_reduces_buffers() {
        let mut snapshot = BrowserContextSnapshot {
            url: "https://example.com".to_string(),
            title: "Test".to_string(),
            actions: (0..100).map(|i| RecorderEvent::Click {
                selector: format!("#btn-{}", i),
                text: format!("Button {}", i),
                x: i as f64,
                y: 0.0,
                timestamp: i as f64 * 1000.0,
            }).collect(),
            console: (0..50).map(|i| ConsoleEntry {
                timestamp: i as f64 * 1000.0,
                level: "log".to_string(),
                text: format!("Log {}", i),
            }).collect(),
            network: (0..30).map(|i| NetworkEntry {
                timestamp: i as f64 * 1000.0,
                method: "GET".to_string(),
                url: format!("https://api.com/{}", i),
                resource_type: "Fetch".to_string(),
                status: Some(200),
            }).collect(),
            mutations: vec![],
            total_bytes: 0,
            page_changed: false,
        };

        cap_to_n(&mut snapshot, 10);
        assert_eq!(snapshot.actions.len(), 10);
        assert_eq!(snapshot.console.len(), 10);
        assert_eq!(snapshot.network.len(), 10);
    }

    #[test]
    fn test_cap_to_n_no_op_when_under_limit() {
        let mut snapshot = BrowserContextSnapshot {
            url: "https://example.com".to_string(),
            title: "Test".to_string(),
            actions: vec![RecorderEvent::Click {
                selector: "#btn".to_string(),
                text: "OK".to_string(),
                x: 10.0,
                y: 20.0,
                timestamp: 1000.0,
            }],
            console: vec![],
            network: vec![],
            mutations: vec![],
            total_bytes: 100,
            page_changed: false,
        };

        cap_to_n(&mut snapshot, 10);
        assert_eq!(snapshot.actions.len(), 1);
    }

    #[test]
    fn test_make_context_message_role_and_content() {
        let snapshot = BrowserContextSnapshot {
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            actions: vec![RecorderEvent::Navigation {
                url: "https://example.com".to_string(),
                title: "Example".to_string(),
                timestamp: 1000.0,
            }],
            console: vec![],
            network: vec![],
            mutations: vec![],
            total_bytes: 50,
            page_changed: false,
        };

        let msg = make_context_message(&snapshot, false);
        assert_eq!(msg.role, "user");
        match &msg.content {
            ChatContent::SimpleText(text) => {
                assert!(text.contains("[Browser Context]"));
                assert!(text.contains("https://example.com"));
            }
            _ => panic!("Expected SimpleText"),
        }
    }

    #[test]
    fn test_oversize_detection() {
        let large_text = "x".repeat(200);
        let actions: Vec<RecorderEvent> = (0..1000).map(|i| RecorderEvent::Input {
            selector: format!("#field-{}", i),
            value: large_text.clone(),
            masked: false,
            timestamp: i as f64 * 1000.0,
        }).collect();

        let size = compute_context_size(&actions, &[], &[], &[]);
        assert!(size > OVERSIZE_THRESHOLD);
    }
}
