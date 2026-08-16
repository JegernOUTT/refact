use std::sync::Arc;

use base64::Engine;
use headless_chrome::Tab;
use refact_core::image_policy::{resize_to_policy, ImageFormat, ImagePolicy};
use serde_json::json;

use crate::call_validation::{ChatContent, ChatMessage, MultimodalElement};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::global_context::GlobalContext;
use crate::integrations::browser_types::{
    ConsoleEntry, MutationSummaryEntry, NetworkEntry, RecorderEvent,
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

pub struct BrowserContextMessages {
    pub event: ChatMessage,
    pub screenshot: Option<ChatMessage>,
}

type ScreenshotCapture = dyn Fn(&Tab) -> Result<(String, String), String> + Send + Sync;

fn should_emit_context(snapshot: &BrowserContextSnapshot) -> bool {
    snapshot.page_changed
        || !snapshot.actions.is_empty()
        || !snapshot.console.is_empty()
        || !snapshot.network.is_empty()
        || !snapshot.mutations.is_empty()
}

pub fn format_browser_context(snapshot: &BrowserContextSnapshot) -> String {
    let mut out = format!(
        "[Browser Context]\nURL: {}\nTitle: {}",
        snapshot.url, snapshot.title
    );

    if !snapshot.actions.is_empty() {
        out.push_str("\n\n## User Actions (since last message)");
        for action in &snapshot.actions {
            out.push('\n');
            out.push_str(&format_action(action));
        }
    }

    if !snapshot.console.is_empty() {
        out.push_str("\n\n## Console (since last message)");
        for entry in &snapshot.console {
            out.push('\n');
            out.push_str(&format_console_entry(entry));
        }
    }

    if !snapshot.network.is_empty() {
        out.push_str("\n\n## Network (since last message)");
        for entry in &snapshot.network {
            out.push('\n');
            out.push_str(&format_network_entry(entry));
        }
    }

    if !snapshot.mutations.is_empty() {
        out.push_str("\n\n## DOM Changes (since last message)");
        let total_added: u32 = snapshot.mutations.iter().map(|m| m.added).sum();
        let total_removed: u32 = snapshot.mutations.iter().map(|m| m.removed).sum();
        let total_changed: u32 = snapshot.mutations.iter().map(|m| m.changed).sum();
        if total_added > 0 {
            out.push_str(&format!("\nAdded: {} elements", total_added));
        }
        if total_removed > 0 {
            out.push_str(&format!("\nRemoved: {} elements", total_removed));
        }
        if total_changed > 0 {
            out.push_str(&format!("\nChanged: {} elements", total_changed));
        }
    }

    out
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
        RecorderEvent::Click {
            selector,
            text,
            x,
            y,
            timestamp,
        } => {
            let label = if text.is_empty() {
                selector.clone()
            } else {
                format!("{} \"{}\"", selector, text)
            };
            format!(
                "[{}] click → {} (x:{}, y:{})",
                format_timestamp(*timestamp),
                label,
                *x as i32,
                *y as i32
            )
        }
        RecorderEvent::Input {
            selector,
            value,
            timestamp,
            tag,
            input_type,
            placeholder,
            ..
        } => {
            let field_desc = match (
                tag.as_deref(),
                input_type.as_deref(),
                placeholder.as_deref(),
            ) {
                (Some(t), Some(it), Some(ph)) => {
                    format!("<{}[type={}] placeholder=\"{}\">", t, it, ph)
                }
                (Some(t), Some(it), None) => format!("<{}[type={}]>", t, it),
                (Some(t), None, Some(ph)) => format!("<{} placeholder=\"{}\">", t, ph),
                _ => selector.clone(),
            };
            format!(
                "[{}] input → {} \"{}\"",
                format_timestamp(*timestamp),
                field_desc,
                value
            )
        }
        RecorderEvent::Keypress {
            key,
            modifiers,
            timestamp,
        } => {
            let mods = if modifiers.is_empty() {
                String::new()
            } else {
                format!("{}+", modifiers.join("+"))
            };
            format!(
                "[{}] keypress → {}{}",
                format_timestamp(*timestamp),
                mods,
                key
            )
        }
        RecorderEvent::Submit {
            selector,
            method,
            action,
            timestamp,
        } => {
            format!(
                "[{}] submit → {} {} {}",
                format_timestamp(*timestamp),
                selector,
                method,
                action
            )
        }
        RecorderEvent::Scroll {
            scroll_x,
            scroll_y,
            timestamp,
        } => {
            format!(
                "[{}] scroll → ({}, {})",
                format_timestamp(*timestamp),
                *scroll_x as i32,
                *scroll_y as i32
            )
        }
        RecorderEvent::MutationSummary {
            added,
            removed,
            changed,
            timestamp,
        } => {
            format!(
                "[{}] dom-change → +{} -{} ~{}",
                format_timestamp(*timestamp),
                added,
                removed,
                changed
            )
        }
        RecorderEvent::ToolbarAction { action, timestamp } => {
            format!("[{}] toolbar → {}", format_timestamp(*timestamp), action)
        }
    }
}

fn format_console_entry(entry: &ConsoleEntry) -> String {
    format!(
        "[{}] [{}] {}",
        format_timestamp(entry.timestamp),
        entry.level,
        entry.text
    )
}

fn format_network_entry(entry: &NetworkEntry) -> String {
    let status_str = entry
        .status
        .map(|s| format!(" → {}", s))
        .unwrap_or_default();
    format!(
        "[{}] {} {}{}",
        format_timestamp(entry.timestamp),
        entry.method,
        entry.url,
        status_str
    )
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

pub async fn get_browser_context_for_chat(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<BrowserContextSnapshot> {
    let (_, runtime_arc) = crate::integrations::browser_runtime::find_runtime_by_chat_id(
        crate::app_state::AppState::from_gcx(gcx).await,
        chat_id,
    )
    .await?;
    let rt = runtime_arc.lock().await;

    if !rt.is_connected {
        return None;
    }

    let (url, title) = if let Some(tab) = rt.get_active_tab() {
        let url = tab.get_url();
        let title = tab.get_title().unwrap_or_default();
        (url, title)
    } else {
        (String::new(), String::new())
    };

    let actions = rt.action_buffer[rt.last_send_action_cursor..].to_vec();
    let console = rt.console_buffer[rt.last_send_console_cursor..].to_vec();
    let network = rt.network_buffer[rt.last_send_network_cursor..].to_vec();
    let mutations = rt.mutation_summary[rt.last_send_mutation_cursor..].to_vec();

    let total_bytes = compute_context_size(&actions, &console, &network, &mutations);

    let page_changed = rt.page_changed(&url);

    let snapshot = BrowserContextSnapshot {
        url,
        title,
        actions,
        console,
        network,
        mutations,
        total_bytes,
        page_changed,
    };
    if !should_emit_context(&snapshot) {
        return None;
    }

    Some(snapshot)
}

pub async fn commit_browser_cursors(gcx: Arc<GlobalContext>, chat_id: &str) {
    if let Some((_, runtime_arc)) = crate::integrations::browser_runtime::find_runtime_by_chat_id(
        crate::app_state::AppState::from_gcx(gcx).await,
        chat_id,
    )
    .await
    {
        let mut rt = runtime_arc.lock().await;
        let url = rt.get_active_tab().map(|tab| tab.get_url());
        rt.commit_cursors(url.as_deref());
    }
}

async fn image_policy_for_chat(gcx: Arc<GlobalContext>, chat_id: &str) -> ImagePolicy {
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let model_id = {
        let sessions = app.chat.sessions.read().await;
        let session = sessions.get(chat_id).cloned();
        drop(sessions);
        let Some(session) = session else {
            return ImagePolicy::browser_capture();
        };
        let model_id = session.lock().await.thread.model.clone();
        model_id
    };
    let Ok(caps) = crate::global_context::try_load_caps_quickly_if_not_present(gcx, 0).await else {
        return ImagePolicy::browser_capture();
    };
    crate::caps::resolve_chat_model(caps, &model_id)
        .map(|model| ImagePolicy::for_model(&model.base).with_format(ImageFormat::Webp, Some(80)))
        .unwrap_or_else(|_| ImagePolicy::browser_capture())
}

async fn capture_browser_screenshot_with(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    attach_screenshot: bool,
    page_changed: bool,
    capture: &ScreenshotCapture,
) -> Option<ChatMessage> {
    if !attach_screenshot || !page_changed {
        return None;
    }
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let (_, runtime_arc) =
        crate::integrations::browser_runtime::find_runtime_by_chat_id(app, chat_id).await?;
    let policy = image_policy_for_chat(gcx, chat_id).await;
    let rt = runtime_arc.lock().await;
    let tab = rt.get_active_tab()?;
    capture_screenshot_message_with(true, true, &policy, &|| capture(&tab))
}

pub async fn capture_browser_screenshot(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<ChatMessage> {
    capture_browser_screenshot_with(
        gcx,
        chat_id,
        true,
        true,
        &crate::integrations::browser_controller::capture_viewport_screenshot_png,
    )
    .await
}

fn screenshot_message(
    capture: (String, String),
    policy: &ImagePolicy,
) -> Result<ChatMessage, String> {
    let (encoded, mime) = capture;
    let raw = base64::prelude::BASE64_STANDARD
        .decode(encoded)
        .map_err(|error| format!("Base64 decode failed: {error}"))?;
    let (resized, resized_mime) = resize_to_policy(&raw, &mime, policy)?;
    Ok(ChatMessage {
        role: "user".to_string(),
        content: ChatContent::Multimodal(vec![MultimodalElement::new(
            resized_mime,
            base64::prelude::BASE64_STANDARD.encode(resized),
        )?]),
        ..Default::default()
    })
}

fn capture_screenshot_message_with(
    attach_screenshot: bool,
    page_changed: bool,
    policy: &ImagePolicy,
    capture: &dyn Fn() -> Result<(String, String), String>,
) -> Option<ChatMessage> {
    if !attach_screenshot || !page_changed {
        return None;
    }
    screenshot_message(capture().ok()?, policy).ok()
}

pub async fn maybe_insert_browser_context(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    has_browser_meta: bool,
    attach_screenshot_on_send: bool,
) -> Option<(BrowserContextMessages, bool)> {
    if !has_browser_meta {
        return None;
    }

    let snapshot = get_browser_context_for_chat(gcx.clone(), chat_id).await?;

    if snapshot.total_bytes > OVERSIZE_THRESHOLD {
        return Some((make_context_messages(&snapshot, None), true));
    }

    let screenshot = capture_browser_screenshot_with(
        gcx.clone(),
        chat_id,
        attach_screenshot_on_send,
        snapshot.page_changed,
        &crate::integrations::browser_controller::capture_viewport_screenshot_png,
    )
    .await;

    commit_browser_cursors(gcx, chat_id).await;

    Some((make_context_messages(&snapshot, screenshot), false))
}

pub fn apply_decision_to_snapshot(
    snapshot: &mut BrowserContextSnapshot,
    include_actions: bool,
    include_console: bool,
    include_network: bool,
    include_mutations: bool,
    last_n_actions: Option<usize>,
    last_n_console: Option<usize>,
    last_n_network: Option<usize>,
) {
    if !include_actions {
        snapshot.actions.clear();
    } else if let Some(n) = last_n_actions {
        if snapshot.actions.len() > n {
            let start = snapshot.actions.len() - n;
            snapshot.actions = snapshot.actions[start..].to_vec();
        }
    }
    if !include_console {
        snapshot.console.clear();
    } else if let Some(n) = last_n_console {
        if snapshot.console.len() > n {
            let start = snapshot.console.len() - n;
            snapshot.console = snapshot.console[start..].to_vec();
        }
    }
    if !include_network {
        snapshot.network.clear();
    } else if let Some(n) = last_n_network {
        if snapshot.network.len() > n {
            let start = snapshot.network.len() - n;
            snapshot.network = snapshot.network[start..].to_vec();
        }
    }
    if !include_mutations {
        snapshot.mutations.clear();
    }
    snapshot.total_bytes = compute_context_size(
        &snapshot.actions,
        &snapshot.console,
        &snapshot.network,
        &snapshot.mutations,
    );
}

pub fn make_context_messages(
    snapshot: &BrowserContextSnapshot,
    screenshot: Option<ChatMessage>,
) -> BrowserContextMessages {
    let text = format_browser_context(snapshot);
    BrowserContextMessages {
        event: event(
            EventSubkind::SystemNotice,
            "chat.browser_context",
            json!({
                "url": snapshot.url,
                "title": snapshot.title,
                "actions": snapshot.actions.len(),
                "console": snapshot.console.len(),
                "network": snapshot.network.len(),
                "mutations": snapshot.mutations.len(),
                "total_bytes": snapshot.total_bytes,
                "page_changed": snapshot.page_changed,
            }),
            text,
        ),
        screenshot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::image_policy::ImageFormat;

    fn empty_snapshot(page_changed: bool) -> BrowserContextSnapshot {
        BrowserContextSnapshot {
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            actions: vec![],
            console: vec![],
            network: vec![],
            mutations: vec![],
            total_bytes: 8,
            page_changed,
        }
    }

    fn tiny_png() -> Vec<u8> {
        let image = image::DynamicImage::new_rgba8(2, 2);
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

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
                    tag: None,
                    input_type: None,
                    field_name: None,
                    placeholder: None,
                    aria_label: None,
                    role: None,
                },
            ],
            console: vec![ConsoleEntry {
                timestamp: 45246000.0,
                level: "error".to_string(),
                text: "Uncaught TypeError: Cannot read property 'x' of null".to_string(),
            }],
            network: vec![NetworkEntry {
                timestamp: 45245000.0,
                method: "GET".to_string(),
                url: "https://example.com".to_string(),
                resource_type: "Document".to_string(),
                status: Some(200),
                ..NetworkEntry::default()
            }],
            mutations: vec![MutationSummaryEntry {
                timestamp: 45247000.0,
                added: 3,
                removed: 1,
                changed: 2,
                descriptions: vec![],
            }],
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
            ..NetworkEntry::default()
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
            ..NetworkEntry::default()
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

        let messages = make_context_messages(&snapshot, None);
        let msg = messages.event;
        assert!(messages.screenshot.is_none());
        assert_eq!(msg.role, "event");
        let event = msg.extra.get("event").unwrap();
        assert_eq!(event["subkind"], "system_notice");
        assert_eq!(event["source"], "chat.browser_context");
        match &msg.content {
            ChatContent::SimpleText(text) => {
                assert!(text.contains("[Browser Context]"));
                assert!(text.contains("https://example.com"));
            }
            _ => panic!("Expected SimpleText"),
        }
    }

    #[test]
    fn navigation_only_snapshot_produces_context_message() {
        let snapshot = empty_snapshot(true);
        assert!(should_emit_context(&snapshot));
        let messages = make_context_messages(&snapshot, None);

        assert_eq!(messages.event.role, "event");
        assert_eq!(
            messages.event.extra["event"]["payload"]["page_changed"],
            true
        );
        assert!(messages
            .event
            .content
            .content_text_only()
            .contains("URL: https://example.com"));
    }

    #[test]
    fn toggle_off_does_not_capture_and_produces_text_only_context() {
        let calls = std::cell::Cell::new(0);
        let screenshot =
            capture_screenshot_message_with(false, true, &ImagePolicy::browser_capture(), &|| {
                calls.set(calls.get() + 1);
                Ok((String::new(), "image/png".to_string()))
            });
        let messages = make_context_messages(&empty_snapshot(true), None);

        assert_eq!(calls.get(), 0);
        assert!(screenshot.is_none());
        assert!(matches!(messages.event.content, ChatContent::SimpleText(_)));
        assert!(messages.screenshot.is_none());
    }

    #[test]
    fn unchanged_page_does_not_capture_screenshot() {
        let calls = std::cell::Cell::new(0);

        let screenshot =
            capture_screenshot_message_with(true, false, &ImagePolicy::browser_capture(), &|| {
                calls.set(calls.get() + 1);
                Ok((String::new(), "image/png".to_string()))
            });

        assert_eq!(calls.get(), 0);
        assert!(screenshot.is_none());
    }

    #[test]
    fn enabled_page_change_uses_mock_capture() {
        let calls = std::cell::Cell::new(0);
        let encoded = base64::prelude::BASE64_STANDARD.encode(tiny_png());

        let screenshot = capture_screenshot_message_with(
            true,
            true,
            &ImagePolicy::default().with_format(ImageFormat::Png, None),
            &|| {
                calls.set(calls.get() + 1);
                Ok((encoded.clone(), "image/png".to_string()))
            },
        );

        assert_eq!(calls.get(), 1);
        assert!(
            screenshot.is_some_and(|message| matches!(message.content, ChatContent::Multimodal(_)))
        );
    }

    #[test]
    fn unchanged_empty_snapshot_is_suppressed() {
        assert!(!should_emit_context(&empty_snapshot(false)));
    }

    #[test]
    fn screenshot_message_resizes_and_uses_capture_format() {
        let encoded = base64::prelude::BASE64_STANDARD.encode(tiny_png());
        let policy = ImagePolicy::default().with_format(ImageFormat::Webp, Some(80));

        let message = screenshot_message((encoded, "image/png".to_string()), &policy).unwrap();

        assert_eq!(message.role, "user");
        let ChatContent::Multimodal(elements) = message.content else {
            panic!("expected multimodal screenshot");
        };
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].m_type, "image/webp");
        let bytes = base64::prelude::BASE64_STANDARD
            .decode(&elements[0].m_content)
            .unwrap();
        assert!(image::load_from_memory(&bytes).is_ok());
    }

    #[test]
    fn screenshot_survives_provider_bound_linearize() {
        use refact_llm::adapter::{AdapterSettings, LlmWireAdapter};
        use refact_llm::adapters::openai_chat::OpenAiChatAdapter;
        use refact_llm::canonical::LlmRequest;

        let encoded = base64::prelude::BASE64_STANDARD.encode(tiny_png());
        let screenshot = screenshot_message(
            (encoded, "image/png".to_string()),
            &ImagePolicy::default().with_format(ImageFormat::Png, None),
        )
        .unwrap();
        let context = make_context_messages(&empty_snapshot(true), Some(screenshot));
        let linearized = crate::chat::linearize::apply_summarization_linearize(vec![
            context.event,
            context.screenshot.unwrap(),
            ChatMessage::new("user".to_string(), "inspect this page".to_string()),
        ]);
        let request = LlmRequest::new("gpt-4.1".to_string(), linearized);
        let settings = AdapterSettings {
            api_key: String::new(),
            auth_token: String::new(),
            endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
            extra_headers: Default::default(),
            model_name: "gpt-4.1".to_string(),
            supports_tools: true,
            supports_reasoning: false,
            reasoning_type: None,
            supports_temperature: true,
            supports_max_completion_tokens: false,
            eof_is_done: false,
            supports_web_search: false,
            supports_cache_control: false,
        };

        let body = OpenAiChatAdapter
            .build_http(&refact_privacy::testing::cleared(request), &settings)
            .unwrap()
            .body;
        let serialized = body["messages"].to_string();

        assert!(serialized.contains("<event subkind=\\\"system_notice\\\""));
        assert!(serialized.contains("data:image/png;base64,"));
    }

    #[test]
    fn screenshot_survives_anthropic_wire_lowering() {
        use refact_llm::adapter::{AdapterSettings, LlmWireAdapter};
        use refact_llm::adapters::anthropic::AnthropicAdapter;
        use refact_llm::canonical::LlmRequest;

        let encoded = base64::prelude::BASE64_STANDARD.encode(tiny_png());
        let screenshot = screenshot_message(
            (encoded, "image/png".to_string()),
            &ImagePolicy::default().with_format(ImageFormat::Png, None),
        )
        .unwrap();
        let context = make_context_messages(&empty_snapshot(true), Some(screenshot));
        let linearized = crate::chat::linearize::apply_summarization_linearize(vec![
            context.event,
            context.screenshot.unwrap(),
            ChatMessage::new("user".to_string(), "inspect this page".to_string()),
        ]);
        let request = LlmRequest::new("claude-sonnet-4-5".to_string(), linearized);
        let settings = AdapterSettings {
            api_key: String::new(),
            auth_token: String::new(),
            endpoint: "https://api.anthropic.com/v1/messages".to_string(),
            extra_headers: Default::default(),
            model_name: "claude-sonnet-4-5".to_string(),
            supports_tools: true,
            supports_reasoning: false,
            reasoning_type: None,
            supports_temperature: true,
            supports_max_completion_tokens: false,
            eof_is_done: false,
            supports_web_search: false,
            supports_cache_control: false,
        };

        let body = AnthropicAdapter
            .build_http(&refact_privacy::testing::cleared(request), &settings)
            .unwrap()
            .body;
        let serialized = body["messages"].to_string();

        assert!(serialized.contains("<event subkind=\\\"system_notice\\\""));
        assert!(serialized.contains("\"type\":\"image\""));
        assert!(serialized.contains("\"media_type\":\"image/png\""));
    }

    #[test]
    fn test_oversize_detection() {
        let large_text = "x".repeat(200);
        let actions: Vec<RecorderEvent> = (0..1000)
            .map(|i| RecorderEvent::Input {
                selector: format!("#field-{}", i),
                value: large_text.clone(),
                masked: false,
                timestamp: i as f64 * 1000.0,
                tag: None,
                input_type: None,
                field_name: None,
                placeholder: None,
                aria_label: None,
                role: None,
            })
            .collect();

        let size = compute_context_size(&actions, &[], &[], &[]);
        assert!(size > OVERSIZE_THRESHOLD);
    }

    #[test]
    fn test_format_browser_context_sections_present() {
        let snapshot = BrowserContextSnapshot {
            url: "https://example.com".to_string(),
            title: "Test".to_string(),
            actions: vec![RecorderEvent::Navigation {
                url: "https://example.com".to_string(),
                title: "Test".to_string(),
                timestamp: 1000.0,
            }],
            console: vec![ConsoleEntry {
                timestamp: 1000.0,
                level: "warn".to_string(),
                text: "test log".to_string(),
            }],
            network: vec![NetworkEntry {
                timestamp: 1000.0,
                method: "GET".to_string(),
                url: "https://example.com".to_string(),
                resource_type: "Document".to_string(),
                status: Some(200),
                ..NetworkEntry::default()
            }],
            mutations: vec![MutationSummaryEntry {
                timestamp: 1000.0,
                added: 1,
                removed: 0,
                changed: 2,
                descriptions: vec![],
            }],
            total_bytes: 100,
            page_changed: false,
        };
        let out = format_browser_context(&snapshot);
        assert!(out.contains("[Browser Context]"));
        assert!(out.contains("## User Actions (since last message)"));
        assert!(out.contains("navigate →"));
        assert!(out.contains("## Console (since last message)"));
        assert!(out.contains("[warn] test log"));
        assert!(out.contains("## Network (since last message)"));
        assert!(out.contains("GET https://example.com → 200"));
        assert!(out.contains("## DOM Changes (since last message)"));
        assert!(out.contains("Added: 1 elements"));
        assert!(out.contains("Changed: 2 elements"));
        assert!(!out.contains("Removed:"));
    }
}
