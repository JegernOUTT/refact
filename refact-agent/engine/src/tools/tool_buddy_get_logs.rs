use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const MAX_LINES: usize = 500;
const DEFAULT_LINES: usize = 50;
const MAX_LOG_TAIL_BYTES: u64 = 256 * 1024;


static REDACT_PATTERNS: &[&str] = &[
    r"sk-[a-zA-Z0-9]{20,}",
    r"Bearer [^\s]+",
    r"api[_-]?key[=:]\s*[^\s,]+",
    r"token[=:]\s*[^\s,]+",
];

fn redact_sensitive(line: &str) -> String {
    let mut result = line.to_string();
    for pat in REDACT_PATTERNS {
        if let Ok(re) = regex::Regex::new(pat) {
            result = re.replace_all(&result, "[REDACTED]").to_string();
        }
    }
    result
}

pub struct ToolBuddyGetLogs {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolBuddyGetLogs {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_get_logs".to_string(),
            display_name: "Buddy Get Logs".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Read recent refact-lsp log lines. Sensitive data (API keys, tokens) is redacted. Useful for diagnosing errors and understanding what the engine is doing.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "lines": {
                        "type": "number",
                        "description": "Number of recent lines to return. Default: 50, max: 500."
                    },
                    "filter": {
                        "type": "string",
                        "description": "Optional regex filter — only lines matching this pattern are returned."
                    },
                    "errors_only": {
                        "type": "boolean",
                        "description": "If true, only return ERROR and WARN lines. Default: false."
                    }
                },
                "required": []
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let lines_req = args
            .get("lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_LINES as u64) as usize;
        let line_limit = lines_req.min(MAX_LINES);
        let filter_pat = args
            .get("filter")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let errors_only = args
            .get("errors_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let filter_re = match &filter_pat {
            Some(pat) => {
                Some(regex::Regex::new(pat).map_err(|e| format!("invalid filter regex: {}", e))?)
            }
            None => None,
        };

        let gcx = ccx.lock().await.global_context.clone();
        let (logs_to_file, cache_dir) = {
            let lock = gcx.read().await;
            (lock.cmdline.logs_to_file.clone(), lock.cache_dir.clone())
        };

        let log_path = if !logs_to_file.is_empty() {
            std::path::PathBuf::from(&logs_to_file)
        } else {
            cache_dir.join("logs")
        };

        let log_content = read_log_content(&log_path).await?;

        let filtered: Vec<&str> = log_content
            .lines()
            .filter(|line| {
                if errors_only && !line.contains("ERROR") && !line.contains("WARN") {
                    return false;
                }
                if let Some(re) = &filter_re {
                    return re.is_match(line);
                }
                true
            })
            .collect();

        let tail: Vec<String> = filtered
            .iter()
            .rev()
            .take(line_limit)
            .rev()
            .map(|l| redact_sensitive(l))
            .collect();

        let output = if tail.is_empty() {
            "No log lines found matching the criteria.".to_string()
        } else {
            format!(
                "Log lines ({} shown, redacted):\n{}",
                tail.len(),
                tail.join("\n")
            )
        };

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(output),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

fn is_log_candidate(path: &std::path::Path) -> bool {
    let extension_is_log = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("log"))
        .unwrap_or(false);
    let filename_mentions_refact = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase().contains("refact"))
        .unwrap_or(false);
    extension_is_log || filename_mentions_refact
}

async fn read_bounded_log_tail(log_path: &std::path::Path) -> Result<String, String> {
    let mut file = tokio::fs::File::open(log_path)
        .await
        .map_err(|e| format!("failed to read log file {:?}: {}", log_path, e))?;
    let len = file
        .metadata()
        .await
        .map_err(|e| format!("failed to stat log file {:?}: {}", log_path, e))?
        .len();
    let start = len.saturating_sub(MAX_LOG_TAIL_BYTES);
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|e| format!("failed to seek log file {:?}: {}", log_path, e))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .await
        .map_err(|e| format!("failed to read log file {:?}: {}", log_path, e))?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        if let Some(pos) = text.find('\n') {
            text = text[pos + 1..].to_string();
        }
    }
    Ok(text)
}

async fn read_log_content(log_path: &std::path::Path) -> Result<String, String> {
    if log_path.is_file() {
        return read_bounded_log_tail(log_path).await;
    }

    if log_path.is_dir() {
        let mut entries = tokio::fs::read_dir(log_path)
            .await
            .map_err(|e| format!("failed to read logs dir {:?}: {}", log_path, e))?;

        let mut files: Vec<(std::path::PathBuf, std::time::SystemTime)> = vec![];
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if !is_log_candidate(&path) {
                continue;
            }
            if let Ok(meta) = tokio::fs::metadata(&path).await {
                if meta.is_file() {
                    if let Ok(modified) = meta.modified() {
                        files.push((path, modified));
                    }
                }
            }
        }

        files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        if let Some((newest, _)) = files.first() {
            return read_bounded_log_tail(newest).await;
        }
    }

    Err(format!("no log file found at {:?}", log_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buddy_get_logs_redaction() {
        let line = "connecting with api_key=sk-abc123defgh456ijklmn and Bearer mytoken123";
        let redacted = redact_sensitive(line);
        assert!(!redacted.contains("sk-abc123defgh456ijklmn"));
        assert!(!redacted.contains("mytoken123"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn test_buddy_get_logs_limit() {
        assert!(DEFAULT_LINES <= MAX_LINES);
        assert_eq!(501_usize.min(MAX_LINES), MAX_LINES);
    }
}
