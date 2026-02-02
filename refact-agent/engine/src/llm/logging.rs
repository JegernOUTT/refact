use serde_json::Value;

const REDACTED: &str = "[REDACTED]";
const MAX_CONTENT_LOG_LENGTH: usize = 500;

fn safe_truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    let mut end = max_len.min(s.len());
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    &s[..end]
}

pub fn sanitize_request_for_logging(body: &Value) -> Value {
    let mut sanitized = body.clone();
    if let Some(obj) = sanitized.as_object_mut() {
        if let Some(messages) = obj.get_mut("messages") {
            sanitize_messages(messages);
        }
        if let Some(input) = obj.get_mut("input") {
            sanitize_input(input);
        }
        if let Some(instructions) = obj.get_mut("instructions") {
            truncate_string(instructions);
        }
        if let Some(system) = obj.get_mut("system") {
            truncate_string(system);
        }
    }
    sanitized
}

fn sanitize_messages(messages: &mut Value) {
    if let Some(arr) = messages.as_array_mut() {
        for msg in arr {
            if let Some(obj) = msg.as_object_mut() {
                if let Some(content) = obj.get_mut("content") {
                    truncate_content(content);
                }
            }
        }
    }
}

fn sanitize_input(input: &mut Value) {
    match input {
        Value::String(s) => {
            if s.len() > MAX_CONTENT_LOG_LENGTH {
                let truncated = safe_truncate(s, MAX_CONTENT_LOG_LENGTH);
                let remaining = s.len() - truncated.len();
                *s = format!("{}...[truncated {} chars]", truncated, remaining);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                if let Some(obj) = item.as_object_mut() {
                    if let Some(content) = obj.get_mut("content") {
                        truncate_content(content);
                    }
                }
            }
        }
        _ => {}
    }
}

fn truncate_content(content: &mut Value) {
    match content {
        Value::String(s) => {
            if s.len() > MAX_CONTENT_LOG_LENGTH {
                let truncated = safe_truncate(s, MAX_CONTENT_LOG_LENGTH);
                let remaining = s.len() - truncated.len();
                *s = format!("{}...[truncated {} chars]", truncated, remaining);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                if let Some(obj) = item.as_object_mut() {
                    if obj.get("type").and_then(|t| t.as_str()) == Some("image_url") {
                        if let Some(image_url) = obj.get_mut("image_url") {
                            if let Some(url_obj) = image_url.as_object_mut() {
                                if let Some(url) = url_obj.get_mut("url") {
                                    *url = Value::String(REDACTED.to_string());
                                }
                            }
                        }
                    }
                    if obj.get("type").and_then(|t| t.as_str()) == Some("image") {
                        if let Some(source) = obj.get_mut("source") {
                            if let Some(source_obj) = source.as_object_mut() {
                                if let Some(data) = source_obj.get_mut("data") {
                                    *data = Value::String(REDACTED.to_string());
                                }
                            }
                        }
                    }
                    if let Some(text) = obj.get_mut("text") {
                        truncate_string(text);
                    }
                }
            }
        }
        _ => {}
    }
}

fn truncate_string(value: &mut Value) {
    if let Value::String(s) = value {
        if s.len() > MAX_CONTENT_LOG_LENGTH {
            let truncated = safe_truncate(s, MAX_CONTENT_LOG_LENGTH);
            let remaining = s.len() - truncated.len();
            *s = format!("{}...[truncated {} chars]", truncated, remaining);
        }
    }
}

pub fn sanitize_headers_for_logging(headers: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let name_str = name.to_string().to_lowercase();
            let value_str = if name_str.contains("authorization")
                || name_str.contains("api-key")
                || name_str.contains("x-api-key")
            {
                REDACTED.to_string()
            } else {
                value.to_str().unwrap_or(REDACTED).to_string()
            };
            (name_str, value_str)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
    use serde_json::json;

    #[test]
    fn test_sanitize_request_truncates_content() {
        let long_content = "a".repeat(1000);
        let body = json!({
            "messages": [{"role": "user", "content": long_content}]
        });
        let sanitized = sanitize_request_for_logging(&body);
        let content = sanitized["messages"][0]["content"].as_str().unwrap();
        assert!(content.len() < 600);
        assert!(content.contains("[truncated"));
    }

    #[test]
    fn test_sanitize_request_redacts_images() {
        let body = json!({
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,SECRETDATA"}}
                ]
            }]
        });
        let sanitized = sanitize_request_for_logging(&body);
        let url = &sanitized["messages"][0]["content"][0]["image_url"]["url"];
        assert_eq!(url, "[REDACTED]");
    }

    #[test]
    fn test_sanitize_headers_redacts_auth() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer sk-secret123"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let sanitized = sanitize_headers_for_logging(&headers);
        let auth = sanitized.iter().find(|(k, _)| k == "authorization").unwrap();
        assert_eq!(auth.1, "[REDACTED]");

        let ct = sanitized.iter().find(|(k, _)| k == "content-type").unwrap();
        assert_eq!(ct.1, "application/json");
    }

    #[test]
    fn test_safe_truncate_ascii() {
        assert_eq!(safe_truncate("hello", 10), "hello");
        assert_eq!(safe_truncate("hello world", 5), "hello");
    }

    #[test]
    fn test_safe_truncate_utf8_no_panic() {
        let chinese = "你好世界这是一个很长的中文字符串";
        let result = safe_truncate(chinese, 10);
        assert!(result.len() <= 10);
        assert!(result.is_char_boundary(result.len()));

        let emoji = "👋🌍🎉✨🚀💻🔥⭐🎯🏆";
        let result = safe_truncate(emoji, 8);
        assert!(result.len() <= 8);
    }

    #[test]
    fn test_sanitize_request_utf8_content() {
        let body = json!({
            "messages": [{"role": "user", "content": "这是一个很长的中文消息，需要被截断处理，确保不会在UTF-8字符边界处崩溃。".repeat(20)}]
        });
        let sanitized = sanitize_request_for_logging(&body);
        assert!(sanitized["messages"][0]["content"].as_str().is_some());
    }
}
