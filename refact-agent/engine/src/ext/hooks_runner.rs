use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock as ARwLock;
use serde::Serialize;

use crate::ext::config_dirs::get_ext_dirs;
use crate::ext::hooks::{HookConfig, HookEvent, load_hooks};
use crate::global_context::GlobalContext;

const HOOK_MAX_OUTPUT_BYTES: usize = 10 * 1024;
const HOOK_DEFAULT_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Serialize)]
pub struct HookPayload {
    pub hook_event_name: String,
    pub session_id: String,
    pub project_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_prompt: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug)]
pub enum HookResult {
    Success(String),
    Block(String),
    Warning(String),
    Timeout,
}

fn matcher_matches(matcher: &Option<String>, tool_name: Option<&str>) -> bool {
    match matcher {
        None => true,
        Some(pattern) => {
            if pattern.is_empty() {
                return true;
            }
            match tool_name {
                Some(name) => regex::Regex::new(pattern)
                    .map(|re| re.is_match(name))
                    .unwrap_or(false),
                None => false,
            }
        }
    }
}

pub async fn get_project_dir_string(gcx: Arc<ARwLock<GlobalContext>>) -> String {
    let dirs = crate::files_correction::get_project_dirs(gcx).await;
    dirs.into_iter()
        .next()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub async fn get_hooks_for_event(
    gcx: Arc<ARwLock<GlobalContext>>,
    event: HookEvent,
    tool_name: Option<&str>,
) -> Vec<HookConfig> {
    let ext_dirs = get_ext_dirs(gcx).await;
    let hooks = load_hooks(&ext_dirs).await;
    hooks
        .into_iter()
        .filter(|h| h.event == event)
        .filter(|h| matcher_matches(&h.matcher, tool_name))
        .collect()
}

pub async fn run_hooks(
    gcx: Arc<ARwLock<GlobalContext>>,
    event: HookEvent,
    payload: HookPayload,
) -> Vec<HookResult> {
    let tool_name = payload.tool_name.clone();
    let matching_hooks = get_hooks_for_event(gcx, event, tool_name.as_deref()).await;

    let mut results = Vec::new();
    for hook in &matching_hooks {
        results.push(run_single_hook(hook, &payload).await);
    }
    results
}

async fn run_single_hook(config: &HookConfig, payload: &HookPayload) -> HookResult {
    let payload_json = match serde_json::to_string(payload) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("hooks_runner: failed to serialize payload: {}", e);
            return HookResult::Warning(format!("Failed to serialize payload: {}", e));
        }
    };

    let timeout_secs = config.timeout.unwrap_or(HOOK_DEFAULT_TIMEOUT_SECS);
    let timeout = Duration::from_secs(timeout_secs);

    match tokio::time::timeout(timeout, run_hook_process(config, payload, &payload_json)).await {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!(
                "hooks_runner: hook timed out after {}s: {}",
                timeout_secs,
                config.command
            );
            HookResult::Timeout
        }
    }
}

fn make_hook_command(
    config: &HookConfig,
    payload: &HookPayload,
) -> tokio::process::Command {
    #[cfg(unix)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(&config.command);
        c
    };

    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/c").arg(&config.command);
        c
    };

    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("REFACT_PROJECT_DIR", &payload.project_dir)
        .env("REFACT_SESSION_ID", &payload.session_id)
        .env("REFACT_HOOK_EVENT", &payload.hook_event_name);

    cmd
}

async fn run_hook_process(
    config: &HookConfig,
    payload: &HookPayload,
    payload_json: &str,
) -> HookResult {
    let mut cmd = make_hook_command(config, payload);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "hooks_runner: failed to spawn '{}': {}",
                config.command,
                e
            );
            return HookResult::Warning(format!("Failed to spawn: {}", e));
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload_json.as_bytes()).await;
    }

    let output = match child.wait_with_output().await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(
                "hooks_runner: failed to wait for '{}': {}",
                config.command,
                e
            );
            return HookResult::Warning(format!("Failed to wait: {}", e));
        }
    };

    let stdout = truncate_bytes(&output.stdout, HOOK_MAX_OUTPUT_BYTES);
    let stderr = truncate_bytes(&output.stderr, HOOK_MAX_OUTPUT_BYTES);

    match output.status.code().unwrap_or(-1) {
        0 => HookResult::Success(stdout),
        2 => {
            let reason = if stderr.is_empty() { stdout } else { stderr };
            tracing::info!("hooks_runner: hook blocked action: {}", reason);
            HookResult::Block(reason)
        }
        code => {
            tracing::warn!(
                "hooks_runner: hook '{}' exited with code {}: {}",
                config.command,
                code,
                stderr
            );
            HookResult::Warning(stderr)
        }
    }
}

fn truncate_bytes(data: &[u8], max_bytes: usize) -> String {
    let slice = if data.len() > max_bytes {
        &data[..max_bytes]
    } else {
        data
    };
    String::from_utf8_lossy(slice).into_owned()
}

pub fn first_block_reason(results: &[HookResult]) -> Option<String> {
    for r in results {
        if let HookResult::Block(reason) = r {
            return Some(reason.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_payload(event: &str, tool_name: Option<&str>) -> HookPayload {
        HookPayload {
            hook_event_name: event.to_string(),
            session_id: "test-session".to_string(),
            project_dir: "/tmp".to_string(),
            tool_name: tool_name.map(|s| s.to_string()),
            tool_input: None,
            tool_output: None,
            user_prompt: None,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn test_payload_serialization_minimal() {
        let payload = make_payload("PreToolUse", None);
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("PreToolUse"));
        assert!(json.contains("test-session"));
        assert!(!json.contains("tool_name"));
        assert!(!json.contains("tool_output"));
        assert!(!json.contains("user_prompt"));
    }

    #[test]
    fn test_payload_serialization_with_tool() {
        let mut payload = make_payload("PreToolUse", Some("shell"));
        payload.tool_input = Some(serde_json::json!({"cmd": "ls"}));
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("shell"));
        assert!(json.contains("tool_input"));
        assert!(json.contains("cmd"));
    }

    #[test]
    fn test_payload_extra_flattened() {
        let mut payload = make_payload("Stop", None);
        payload.extra.insert(
            "finish_reason".to_string(),
            serde_json::json!("stop"),
        );
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("finish_reason"));
        assert!(json.contains("\"stop\""));
    }

    #[test]
    fn test_matcher_matches_none_matches_all() {
        assert!(matcher_matches(&None, Some("shell")));
        assert!(matcher_matches(&None, Some("cat")));
        assert!(matcher_matches(&None, None));
    }

    #[test]
    fn test_matcher_matches_empty_pattern_matches_all() {
        assert!(matcher_matches(&Some("".to_string()), Some("shell")));
        assert!(matcher_matches(&Some("".to_string()), None));
    }

    #[test]
    fn test_matcher_matches_pattern_with_tool_name() {
        assert!(matcher_matches(
            &Some("Bash|shell".to_string()),
            Some("shell")
        ));
        assert!(matcher_matches(
            &Some("Bash|shell".to_string()),
            Some("Bash")
        ));
        assert!(!matcher_matches(
            &Some("Bash|shell".to_string()),
            Some("cat")
        ));
    }

    #[test]
    fn test_matcher_matches_pattern_without_tool_name_returns_false() {
        assert!(!matcher_matches(&Some("shell".to_string()), None));
    }

    #[test]
    fn test_matcher_invalid_regex_returns_false() {
        assert!(!matcher_matches(&Some("[invalid".to_string()), Some("shell")));
    }

    #[tokio::test]
    async fn test_run_single_hook_success() {
        let config = crate::ext::hooks::HookConfig {
            event: HookEvent::PreToolUse,
            matcher: None,
            command: "echo success_output".to_string(),
            timeout: Some(5),
            source: crate::ext::config_dirs::CommandSource::GlobalRefact,
        };
        let payload = make_payload("PreToolUse", Some("shell"));
        let result = run_single_hook(&config, &payload).await;
        match result {
            HookResult::Success(out) => assert!(out.contains("success_output")),
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_run_single_hook_exit_2_blocks() {
        let config = crate::ext::hooks::HookConfig {
            event: HookEvent::PreToolUse,
            matcher: None,
            command: "sh -c 'echo block_reason >&2; exit 2'".to_string(),
            timeout: Some(5),
            source: crate::ext::config_dirs::CommandSource::GlobalRefact,
        };
        let payload = make_payload("PreToolUse", Some("shell"));
        let result = run_single_hook(&config, &payload).await;
        match result {
            HookResult::Block(reason) => assert!(reason.contains("block_reason")),
            other => panic!("Expected Block, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_run_single_hook_nonzero_exit_warning() {
        let config = crate::ext::hooks::HookConfig {
            event: HookEvent::PostToolUse,
            matcher: None,
            command: "sh -c 'echo warn_output >&2; exit 1'".to_string(),
            timeout: Some(5),
            source: crate::ext::config_dirs::CommandSource::GlobalRefact,
        };
        let payload = make_payload("PostToolUse", Some("cat"));
        let result = run_single_hook(&config, &payload).await;
        match result {
            HookResult::Warning(msg) => assert!(msg.contains("warn_output")),
            other => panic!("Expected Warning, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_run_single_hook_timeout() {
        let config = crate::ext::hooks::HookConfig {
            event: HookEvent::SessionStart,
            matcher: None,
            command: "sleep 60".to_string(),
            timeout: Some(1),
            source: crate::ext::config_dirs::CommandSource::GlobalRefact,
        };
        let payload = make_payload("SessionStart", None);
        let result = run_single_hook(&config, &payload).await;
        assert!(matches!(result, HookResult::Timeout));
    }

    #[test]
    fn test_first_block_reason_found() {
        let results = vec![
            HookResult::Success("ok".to_string()),
            HookResult::Block("blocked".to_string()),
            HookResult::Warning("warn".to_string()),
        ];
        assert_eq!(
            first_block_reason(&results),
            Some("blocked".to_string())
        );
    }

    #[test]
    fn test_first_block_reason_not_found() {
        let results = vec![
            HookResult::Success("ok".to_string()),
            HookResult::Warning("warn".to_string()),
        ];
        assert_eq!(first_block_reason(&results), None);
    }

    #[test]
    fn test_first_block_reason_empty() {
        let results: Vec<HookResult> = vec![];
        assert_eq!(first_block_reason(&results), None);
    }

    #[test]
    fn test_truncate_bytes_no_truncation() {
        let data = b"hello world";
        assert_eq!(truncate_bytes(data, 100), "hello world");
    }

    #[test]
    fn test_truncate_bytes_truncates() {
        let data = b"hello world";
        let result = truncate_bytes(data, 5);
        assert_eq!(result, "hello");
    }
}
