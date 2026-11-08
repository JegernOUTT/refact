use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
            RecorderEvent::Navigation {
                timestamp, ..
            } => *timestamp,
            RecorderEvent::Click {
                timestamp, ..
            } => *timestamp,
            RecorderEvent::Input {
                timestamp, ..
            } => *timestamp,
            RecorderEvent::Keypress {
                timestamp, ..
            } => *timestamp,
            RecorderEvent::Submit {
                timestamp, ..
            } => *timestamp,
            RecorderEvent::Scroll {
                timestamp, ..
            } => *timestamp,
            RecorderEvent::MutationSummary {
                timestamp, ..
            } => *timestamp,
            RecorderEvent::ToolbarAction {
                timestamp, ..
            } => *timestamp,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct NetworkEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_attribution: Option<String>,
    pub timestamp: f64,
    pub method: String,
    pub url: String,
    pub resource_type: String,
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub request_headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub response_headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<NetworkTiming>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoded_data_length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_text: Option<String>,
    #[serde(default)]
    pub from_service_worker: bool,
    #[serde(default)]
    pub is_navigation_request: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkTiming {
    pub start_time: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_end: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MutationSummaryEntry {
    pub timestamp: f64,
    pub added: u32,
    pub removed: u32,
    pub changed: u32,
    pub descriptions: Vec<String>,
}

pub const MASKED_VALUE: &str = "********";

pub fn apply_password_masking(event: &RecorderEvent) -> RecorderEvent {
    match event {
        RecorderEvent::Input {
            selector,
            value: _,
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
                    value: MASKED_VALUE.to_string(),
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
    enforce_buffer_limit_for(buffer, &mut [cursor]);
}

pub fn enforce_buffer_limit_for<T>(buffer: &mut Vec<T>, cursors: &mut [&mut usize]) {
    if buffer.len() <= MAX_BUFFER_SIZE {
        return;
    }
    let excess = buffer.len() - MAX_BUFFER_SIZE;
    buffer.drain(..excess);
    for cursor in cursors {
        **cursor = cursor.saturating_sub(excess);
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
            RecorderEvent::Input {
                value,
                masked,
                ..
            } => {
                assert_eq!(value, "********");
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
            RecorderEvent::Input {
                value,
                masked,
                ..
            } => {
                assert_eq!(value, "user@test.com");
                assert!(!masked);
            }
            _ => panic!("Wrong variant"),
        }
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
    fn test_buffer_cursor_saturating_sub() {
        let mut buffer: Vec<u32> = (0..10500).collect();
        let mut cursor = 100usize;
        enforce_buffer_limit(&mut buffer, &mut cursor);
        assert_eq!(buffer.len(), MAX_BUFFER_SIZE);
        assert_eq!(cursor, 0);
    }

    #[test]
    fn test_flush_buffer_since_incremental() {
        let mut buffer = vec![NetworkEntry {
            timestamp: 1.0,
            method: "GET".to_string(),
            url: "https://example.com".to_string(),
            resource_type: "Document".to_string(),
            status: Some(200),
            ..NetworkEntry::default()
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
            ..NetworkEntry::default()
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
                assert_eq!(value, "********");
                assert_eq!(tag.as_deref(), Some("input"));
                assert_eq!(input_type.as_deref(), Some("password"));
                assert_eq!(field_name.as_deref(), Some("password"));
                assert_eq!(placeholder.as_deref(), Some("Enter password"));
            }
            _ => panic!("Wrong variant"),
        }
    }
}
