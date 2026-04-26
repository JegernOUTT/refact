use serde::{Deserialize, Serialize};

pub const MAX_BUFFER_SIZE: usize = 10000;
pub const SCROLL_DEBOUNCE_MS: f64 = 300.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecorderEvent {
    Navigation {
        url: String,
        title: String,
        timestamp: f64,
    },
    Click {
        selector: String,
        text: String,
        x: f64,
        y: f64,
        timestamp: f64,
    },
    Input {
        selector: String,
        value: String,
        masked: bool,
        timestamp: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        field_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        aria_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
    },
    Keypress {
        key: String,
        modifiers: Vec<String>,
        timestamp: f64,
    },
    Submit {
        selector: String,
        action: String,
        method: String,
        timestamp: f64,
    },
    Scroll {
        scroll_x: f64,
        scroll_y: f64,
        timestamp: f64,
    },
    MutationSummary {
        added: u32,
        removed: u32,
        changed: u32,
        timestamp: f64,
    },
    ToolbarAction {
        action: String,
        timestamp: f64,
    },
}

impl RecorderEvent {
    pub fn timestamp(&self) -> f64 {
        match self {
            RecorderEvent::Navigation { timestamp, .. } => *timestamp,
            RecorderEvent::Click { timestamp, .. } => *timestamp,
            RecorderEvent::Input { timestamp, .. } => *timestamp,
            RecorderEvent::Keypress { timestamp, .. } => *timestamp,
            RecorderEvent::Submit { timestamp, .. } => *timestamp,
            RecorderEvent::Scroll { timestamp, .. } => *timestamp,
            RecorderEvent::MutationSummary { timestamp, .. } => *timestamp,
            RecorderEvent::ToolbarAction { timestamp, .. } => *timestamp,
        }
    }

    pub fn is_scroll(&self) -> bool {
        matches!(self, RecorderEvent::Scroll { .. })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsoleEntry {
    pub timestamp: f64,
    pub level: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkEntry {
    pub timestamp: f64,
    pub method: String,
    pub url: String,
    pub resource_type: String,
    pub status: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MutationSummaryEntry {
    pub timestamp: f64,
    pub added: u32,
    pub removed: u32,
    pub changed: u32,
    pub descriptions: Vec<String>,
}

pub fn apply_password_masking(event: &RecorderEvent) -> RecorderEvent {
    match event {
        RecorderEvent::Input {
            selector,
            value,
            masked,
            timestamp,
            tag,
            input_type,
            field_name,
            placeholder,
            aria_label,
            role,
        } => {
            if *masked {
                RecorderEvent::Input {
                    selector: selector.clone(),
                    value: "*".repeat(value.len()),
                    masked: true,
                    timestamp: *timestamp,
                    tag: tag.clone(),
                    input_type: input_type.clone(),
                    field_name: field_name.clone(),
                    placeholder: placeholder.clone(),
                    aria_label: aria_label.clone(),
                    role: role.clone(),
                }
            } else {
                event.clone()
            }
        }
        _ => event.clone(),
    }
}

pub fn enforce_buffer_limit<T>(buffer: &mut Vec<T>, cursor: &mut usize) {
    if buffer.len() > MAX_BUFFER_SIZE {
        let excess = buffer.len() - MAX_BUFFER_SIZE;
        buffer.drain(..excess);
        *cursor = cursor.saturating_sub(excess);
    }
}

pub fn flush_buffer_since<T: Clone>(buffer: &[T], cursor: &mut usize) -> Vec<T> {
    let start = (*cursor).min(buffer.len());
    let items = buffer[start..].to_vec();
    *cursor = buffer.len();
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recorder_event_navigation_parse() {
        let json = r#"{"type":"navigation","url":"https://example.com","title":"Example","timestamp":1000.0}"#;
        let event: RecorderEvent = serde_json::from_str(json).unwrap();
        assert!(
            matches!(event, RecorderEvent::Navigation { ref url, .. } if url == "https://example.com")
        );
        assert_eq!(event.timestamp(), 1000.0);
    }

    #[test]
    fn test_recorder_event_click_parse() {
        let json = r##"{"type":"click","selector":"#btn","text":"Submit","x":100.0,"y":200.0,"timestamp":1001.0}"##;
        let event: RecorderEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, RecorderEvent::Click { x, y, .. } if x == 100.0 && y == 200.0));
    }

    #[test]
    fn test_recorder_event_input_parse() {
        let json = r##"{"type":"input","selector":"#email","value":"user@test.com","masked":false,"timestamp":1002.0}"##;
        let event: RecorderEvent = serde_json::from_str(json).unwrap();
        match event {
            RecorderEvent::Input { value, masked, .. } => {
                assert_eq!(value, "user@test.com");
                assert!(!masked);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_recorder_event_input_masked_parse() {
        let json = r##"{"type":"input","selector":"#pass","value":"secret123","masked":true,"timestamp":1003.0}"##;
        let event: RecorderEvent = serde_json::from_str(json).unwrap();
        match event {
            RecorderEvent::Input { masked, .. } => assert!(masked),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_recorder_event_keypress_parse() {
        let json = r#"{"type":"keypress","key":"Enter","modifiers":["Ctrl"],"timestamp":1004.0}"#;
        let event: RecorderEvent = serde_json::from_str(json).unwrap();
        match event {
            RecorderEvent::Keypress { key, modifiers, .. } => {
                assert_eq!(key, "Enter");
                assert_eq!(modifiers, vec!["Ctrl"]);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_recorder_event_submit_parse() {
        let json = r##"{"type":"submit","selector":"#form","action":"/login","method":"POST","timestamp":1005.0}"##;
        let event: RecorderEvent = serde_json::from_str(json).unwrap();
        match event {
            RecorderEvent::Submit { action, method, .. } => {
                assert_eq!(action, "/login");
                assert_eq!(method, "POST");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_recorder_event_scroll_parse() {
        let json = r#"{"type":"scroll","scroll_x":0.0,"scroll_y":500.0,"timestamp":1006.0}"#;
        let event: RecorderEvent = serde_json::from_str(json).unwrap();
        assert!(event.is_scroll());
        assert_eq!(event.timestamp(), 1006.0);
    }

    #[test]
    fn test_recorder_event_mutation_summary_parse() {
        let json =
            r#"{"type":"mutation_summary","added":3,"removed":1,"changed":2,"timestamp":1007.0}"#;
        let event: RecorderEvent = serde_json::from_str(json).unwrap();
        match event {
            RecorderEvent::MutationSummary {
                added,
                removed,
                changed,
                ..
            } => {
                assert_eq!(added, 3);
                assert_eq!(removed, 1);
                assert_eq!(changed, 2);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_recorder_event_roundtrip() {
        let event = RecorderEvent::Click {
            selector: "button.submit".to_string(),
            text: "Go".to_string(),
            x: 42.5,
            y: 99.1,
            timestamp: 2000.0,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: RecorderEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    #[test]
    fn test_password_masking_masks_input() {
        let event = RecorderEvent::Input {
            selector: "#password".to_string(),
            value: "secret123".to_string(),
            masked: true,
            timestamp: 1000.0,
            tag: None,
            input_type: None,
            field_name: None,
            placeholder: None,
            aria_label: None,
            role: None,
        };
        let masked = apply_password_masking(&event);
        match masked {
            RecorderEvent::Input { value, masked, .. } => {
                assert_eq!(value, "*********");
                assert!(masked);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_password_masking_skips_non_masked() {
        let event = RecorderEvent::Input {
            selector: "#email".to_string(),
            value: "user@test.com".to_string(),
            masked: false,
            timestamp: 1000.0,
            tag: None,
            input_type: None,
            field_name: None,
            placeholder: None,
            aria_label: None,
            role: None,
        };
        let result = apply_password_masking(&event);
        match result {
            RecorderEvent::Input { value, masked, .. } => {
                assert_eq!(value, "user@test.com");
                assert!(!masked);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_password_masking_non_input_passthrough() {
        let event = RecorderEvent::Click {
            selector: "#btn".to_string(),
            text: "OK".to_string(),
            x: 10.0,
            y: 20.0,
            timestamp: 1000.0,
        };
        let result = apply_password_masking(&event);
        assert_eq!(result, event);
    }

    #[test]
    fn test_buffer_max_size_enforcement() {
        let mut buffer: Vec<u32> = (0..10500).collect();
        let mut cursor = 9000usize;
        enforce_buffer_limit(&mut buffer, &mut cursor);
        assert_eq!(buffer.len(), MAX_BUFFER_SIZE);
        assert_eq!(cursor, 8500);
        assert_eq!(buffer[0], 500);
    }

    #[test]
    fn test_buffer_max_size_no_op_when_under() {
        let mut buffer: Vec<u32> = (0..100).collect();
        let mut cursor = 50usize;
        enforce_buffer_limit(&mut buffer, &mut cursor);
        assert_eq!(buffer.len(), 100);
        assert_eq!(cursor, 50);
    }

    #[test]
    fn test_buffer_cursor_saturating_sub() {
        let mut buffer: Vec<u32> = (0..10500).collect();
        let mut cursor = 100usize;
        enforce_buffer_limit(&mut buffer, &mut cursor);
        assert_eq!(buffer.len(), MAX_BUFFER_SIZE);
        assert_eq!(cursor, 0);
    }

    #[test]
    fn test_flush_buffer_since_basic() {
        let buffer = vec![
            ConsoleEntry {
                timestamp: 1.0,
                level: "log".to_string(),
                text: "hello".to_string(),
            },
            ConsoleEntry {
                timestamp: 2.0,
                level: "warn".to_string(),
                text: "warning".to_string(),
            },
        ];
        let mut cursor = 0usize;
        let flushed = flush_buffer_since(&buffer, &mut cursor);
        assert_eq!(flushed.len(), 2);
        assert_eq!(cursor, 2);

        let flushed2 = flush_buffer_since(&buffer, &mut cursor);
        assert_eq!(flushed2.len(), 0);
    }

    #[test]
    fn test_flush_buffer_since_incremental() {
        let mut buffer = vec![NetworkEntry {
            timestamp: 1.0,
            method: "GET".to_string(),
            url: "https://example.com".to_string(),
            resource_type: "Document".to_string(),
            status: Some(200),
        }];
        let mut cursor = 0usize;
        let flushed = flush_buffer_since(&buffer, &mut cursor);
        assert_eq!(flushed.len(), 1);

        buffer.push(NetworkEntry {
            timestamp: 2.0,
            method: "POST".to_string(),
            url: "https://api.example.com".to_string(),
            resource_type: "XHR".to_string(),
            status: Some(201),
        });
        let flushed2 = flush_buffer_since(&buffer, &mut cursor);
        assert_eq!(flushed2.len(), 1);
        assert_eq!(flushed2[0].method, "POST");
    }

    #[test]
    fn test_flush_buffer_since_stale_cursor_does_not_panic() {
        let buffer: Vec<u32> = vec![1, 2, 3];
        let mut cursor = 10usize;
        let flushed = flush_buffer_since(&buffer, &mut cursor);
        assert_eq!(flushed.len(), 0);
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_flush_buffer_since_cursor_exactly_at_len() {
        let buffer: Vec<u32> = vec![1, 2, 3];
        let mut cursor = 3usize;
        let flushed = flush_buffer_since(&buffer, &mut cursor);
        assert_eq!(flushed.len(), 0);
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_console_entry_serde_roundtrip() {
        let entry = ConsoleEntry {
            timestamp: 100.0,
            level: "error".to_string(),
            text: "Uncaught TypeError".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ConsoleEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, parsed);
    }

    #[test]
    fn test_network_entry_serde_roundtrip() {
        let entry = NetworkEntry {
            timestamp: 200.0,
            method: "POST".to_string(),
            url: "https://api.example.com/data".to_string(),
            resource_type: "Fetch".to_string(),
            status: Some(404),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: NetworkEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, parsed);
    }

    #[test]
    fn test_network_entry_serde_no_status() {
        let entry = NetworkEntry {
            timestamp: 300.0,
            method: "GET".to_string(),
            url: "https://example.com".to_string(),
            resource_type: "Document".to_string(),
            status: None,
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert!(json["status"].is_null());
        let parsed: NetworkEntry = serde_json::from_value(json).unwrap();
        assert!(parsed.status.is_none());
    }

    #[test]
    fn test_input_enriched_metadata_parse() {
        let json = r##"{"type":"input","selector":"#email","value":"user@test.com","masked":false,"timestamp":1000.0,"tag":"input","input_type":"email","field_name":"email","placeholder":"Enter email","aria_label":"Email Address","role":"textbox"}"##;
        let event: RecorderEvent = serde_json::from_str(json).unwrap();
        match event {
            RecorderEvent::Input {
                tag,
                input_type,
                field_name,
                placeholder,
                aria_label,
                role,
                ..
            } => {
                assert_eq!(tag.as_deref(), Some("input"));
                assert_eq!(input_type.as_deref(), Some("email"));
                assert_eq!(field_name.as_deref(), Some("email"));
                assert_eq!(placeholder.as_deref(), Some("Enter email"));
                assert_eq!(aria_label.as_deref(), Some("Email Address"));
                assert_eq!(role.as_deref(), Some("textbox"));
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_input_without_enriched_metadata_backwards_compat() {
        let json = r##"{"type":"input","selector":"#email","value":"user@test.com","masked":false,"timestamp":1000.0}"##;
        let event: RecorderEvent = serde_json::from_str(json).unwrap();
        match event {
            RecorderEvent::Input {
                tag,
                input_type,
                field_name,
                placeholder,
                aria_label,
                role,
                ..
            } => {
                assert!(tag.is_none());
                assert!(input_type.is_none());
                assert!(field_name.is_none());
                assert!(placeholder.is_none());
                assert!(aria_label.is_none());
                assert!(role.is_none());
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_password_masking_preserves_metadata() {
        let event = RecorderEvent::Input {
            selector: "#pass".to_string(),
            value: "secret".to_string(),
            masked: true,
            timestamp: 1000.0,
            tag: Some("input".to_string()),
            input_type: Some("password".to_string()),
            field_name: Some("password".to_string()),
            placeholder: Some("Enter password".to_string()),
            aria_label: None,
            role: None,
        };
        let masked = apply_password_masking(&event);
        match masked {
            RecorderEvent::Input {
                value,
                tag,
                input_type,
                field_name,
                placeholder,
                ..
            } => {
                assert_eq!(value, "******");
                assert_eq!(tag.as_deref(), Some("input"));
                assert_eq!(input_type.as_deref(), Some("password"));
                assert_eq!(field_name.as_deref(), Some("password"));
                assert_eq!(placeholder.as_deref(), Some("Enter password"));
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_input_enriched_metadata_roundtrip() {
        let event = RecorderEvent::Input {
            selector: "#search".to_string(),
            value: "query".to_string(),
            masked: false,
            timestamp: 2000.0,
            tag: Some("input".to_string()),
            input_type: Some("search".to_string()),
            field_name: Some("q".to_string()),
            placeholder: Some("Search...".to_string()),
            aria_label: Some("Search".to_string()),
            role: Some("searchbox".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: RecorderEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    #[test]
    fn test_input_enriched_skips_none_in_serialization() {
        let event = RecorderEvent::Input {
            selector: "#field".to_string(),
            value: "val".to_string(),
            masked: false,
            timestamp: 1000.0,
            tag: None,
            input_type: None,
            field_name: None,
            placeholder: None,
            aria_label: None,
            role: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("tag"));
        assert!(!json.contains("input_type"));
        assert!(!json.contains("field_name"));
    }

    #[test]
    fn test_mutation_summary_entry_serde_roundtrip() {
        let entry = MutationSummaryEntry {
            timestamp: 999.0,
            added: 5,
            removed: 2,
            changed: 3,
            descriptions: vec!["childList changed on #app".to_string()],
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: MutationSummaryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, parsed);
    }
}
