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
    // Use the shared Buddy secret redactor first (covers ghp_/glpat-/Authorization/password/
    // aws keys/etc.), then apply the local log-specific patterns on top so nothing regresses.
    let mut result = crate::buddy::actor::redact_sensitive(line);
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

        let gcx = ccx.lock().await.app.gcx.clone();
        let (logs_to_file, cache_dir) = {
            let lock = gcx.clone();
            (lock.cmdline.logs_to_file.clone(), lock.cache_dir.clone())
        };

        let log_path = if !logs_to_file.is_empty() {
            std::path::PathBuf::from(&logs_to_file)
        } else {
            resolve_log_dir(&cache_dir)
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

pub fn is_log_candidate(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".log")
        || lower.contains("refact")
        || lower.starts_with("rustbinary")
        || (lower.starts_with("worker-") && lower.ends_with(".log"))
}

fn dir_has_log_candidate(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            let path = entry.path();
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(is_log_candidate)
                    .unwrap_or(false)
        })
}

fn dir_has_worker_log(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            let path = entry.path();
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| {
                        let lower = name.to_ascii_lowercase();
                        lower.starts_with("worker-") && lower.ends_with(".log")
                    })
                    .unwrap_or(false)
        })
}

/// Selection priority within a log dir: daemon worker logs beat classic rustbinary logs, which
/// beat any other candidate. Prevents a fresher unrelated `.log` from shadowing a real log.
fn log_tier(path: &std::path::Path) -> u8 {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.starts_with("worker-") {
        3
    } else if name.starts_with("rustbinary") {
        2
    } else {
        1
    }
}

pub fn resolve_log_dir(cache_dir: &std::path::Path) -> std::path::PathBuf {
    let daemon = cache_dir.join("daemon").join("logs");
    // Prefer the daemon worker-log layout only when it actually contains a worker-*.log, so a
    // stray non-worker file in daemon/logs does not shadow the classic engine log. If the
    // daemon dir has candidates but the classic dir is empty, still prefer daemon.
    if dir_has_worker_log(&daemon)
        || (dir_has_log_candidate(&daemon) && !dir_has_log_candidate(&cache_dir.join("logs")))
    {
        daemon
    } else {
        cache_dir.join("logs")
    }
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
            let is_candidate = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(is_log_candidate)
                .unwrap_or(false);
            if !is_candidate {
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

        files.sort_by(|a, b| {
            log_tier(&b.0)
                .cmp(&log_tier(&a.0))
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.0.cmp(&b.0))
        });

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

    #[tokio::test]
    async fn classic_rustbinary_layout_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let logs_dir = dir.path().join("logs");
        tokio::fs::create_dir_all(&logs_dir).await.unwrap();
        tokio::fs::write(logs_dir.join("rustbinary.2026-07-01"), "classic tail")
            .await
            .unwrap();

        let resolved = resolve_log_dir(dir.path());
        let content = read_log_content(&resolved).await.unwrap();

        assert_eq!(resolved, logs_dir);
        assert!(is_log_candidate("rustbinary.2026-07-01"));
        assert!(content.contains("classic tail"));
    }

    #[tokio::test]
    async fn daemon_worker_layout_is_found_and_preferred() {
        let dir = tempfile::tempdir().unwrap();
        let classic_dir = dir.path().join("logs");
        let daemon_dir = dir.path().join("daemon").join("logs");
        tokio::fs::create_dir_all(&classic_dir).await.unwrap();
        tokio::fs::create_dir_all(&daemon_dir).await.unwrap();
        tokio::fs::write(classic_dir.join("rustbinary.2026-07-01"), "classic")
            .await
            .unwrap();
        tokio::fs::write(daemon_dir.join("worker-1.log"), "daemon tail")
            .await
            .unwrap();

        let resolved = resolve_log_dir(dir.path());
        let content = read_log_content(&resolved).await.unwrap();

        assert_eq!(resolved, daemon_dir);
        assert!(is_log_candidate("worker-1.log"));
        assert!(content.contains("daemon tail"));
    }

    #[tokio::test]
    async fn large_file_tail_does_not_read_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rustbinary.2026-07-01");
        let content = format!(
            "old-start\n{}\nrecent-tail",
            "x".repeat(MAX_LOG_TAIL_BYTES as usize + 1024)
        );
        tokio::fs::write(&path, content).await.unwrap();

        let tail = read_log_content(&path).await.unwrap();

        assert!(tail.len() <= MAX_LOG_TAIL_BYTES as usize);
        assert!(!tail.contains("old-start"));
        assert!(tail.contains("recent-tail"));
    }
}
