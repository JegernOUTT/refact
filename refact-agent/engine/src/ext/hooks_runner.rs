use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock as ARwLock;
use serde::Serialize;

use crate::ext::config_dirs::{get_ext_dirs, CommandSource, ExtDirs};
use crate::ext::hooks::{HookConfig, HookEvent, load_hooks};
use crate::global_context::GlobalContext;

const HOOK_MAX_OUTPUT_BYTES: usize = 10 * 1024;
const HOOK_DEFAULT_TIMEOUT_SECS: u64 = 30;
const HOOKS_CACHE_TTL: Duration = Duration::from_secs(5);
const MAX_CONCURRENT_HOOKS: usize = 5;

static HOOK_SEMAPHORE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();

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
#[allow(dead_code)]
pub enum HookResult {
    Success(String),
    Block(String),
    Warning(String),
    Timeout,
}

#[derive(Clone)]
struct CompiledHook {
    config: HookConfig,
    compiled_matcher: Option<regex::Regex>,
}

struct HooksCacheEntry {
    hooks: Vec<CompiledHook>,
    loaded_at: Instant,
    project_key: String,
}

static HOOKS_CACHE: OnceLock<tokio::sync::RwLock<Option<HooksCacheEntry>>> = OnceLock::new();

fn compile_hooks(hooks: Vec<HookConfig>) -> Vec<CompiledHook> {
    let mut result = Vec::new();
    for config in hooks {
        let compiled_matcher = match &config.matcher {
            None => None,
            Some(p) if p.is_empty() => None,
            Some(p) => match regex::Regex::new(p) {
                Ok(re) => Some(re),
                Err(e) => {
                    tracing::warn!("hooks_runner: invalid matcher regex '{}': {}", p, e);
                    continue;
                }
            },
        };
        result.push(CompiledHook {
            config,
            compiled_matcher,
        });
    }
    result
}

fn compiled_matcher_matches(compiled: Option<&regex::Regex>, tool_name: Option<&str>) -> bool {
    match compiled {
        None => true,
        Some(re) => match tool_name {
            Some(name) => re.is_match(name),
            None => false,
        },
    }
}

async fn get_compiled_hooks_from_ext_dirs(ext_dirs: &ExtDirs) -> Vec<CompiledHook> {
    let project_key = ext_dirs
        .project_dirs
        .first()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lock = HOOKS_CACHE.get_or_init(|| tokio::sync::RwLock::new(None));
    {
        let read = lock.read().await;
        if let Some(entry) = &*read {
            if entry.loaded_at.elapsed() < HOOKS_CACHE_TTL && entry.project_key == project_key {
                return entry.hooks.clone();
            }
        }
    }
    let raw_hooks = load_hooks(ext_dirs).await;
    let compiled = compile_hooks(raw_hooks);
    let mut write = lock.write().await;
    *write = Some(HooksCacheEntry {
        hooks: compiled.clone(),
        loaded_at: Instant::now(),
        project_key,
    });
    compiled
}

pub async fn get_project_dir_string(gcx: Arc<ARwLock<GlobalContext>>) -> String {
    let dirs = crate::files_correction::get_project_dirs(gcx).await;
    dirs.into_iter()
        .next()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn is_global_source(source: &CommandSource) -> bool {
    matches!(
        source,
        CommandSource::GlobalClaude | CommandSource::GlobalRefact
    )
}

pub async fn get_hooks_for_event(
    gcx: Arc<ARwLock<GlobalContext>>,
    event: HookEvent,
    tool_name: Option<&str>,
) -> Vec<HookConfig> {
    let ext_dirs = get_ext_dirs(gcx).await;
    let compiled_hooks = get_compiled_hooks_from_ext_dirs(&ext_dirs).await;
    compiled_hooks
        .into_iter()
        .filter(|h| is_global_source(&h.config.source)) // Trust gating: only global hooks
        .filter(|h| h.config.event == event)
        .filter(|h| compiled_matcher_matches(h.compiled_matcher.as_ref(), tool_name))
        .map(|h| h.config)
        .collect()
}

async fn run_single_hook_with_semaphore(config: &HookConfig, payload: &HookPayload) -> HookResult {
    let semaphore =
        HOOK_SEMAPHORE.get_or_init(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_HOOKS));
    let _permit = semaphore
        .acquire()
        .await
        .expect("hook semaphore should not be closed");
    run_single_hook(config, payload).await
}

async fn run_hooks_from_list(hooks: &[HookConfig], payload: &HookPayload) -> Vec<HookResult> {
    let futs: Vec<_> = hooks
        .iter()
        .map(|hook| run_single_hook_with_semaphore(hook, payload))
        .collect();
    futures::future::join_all(futs).await
}

pub async fn run_hooks(
    gcx: Arc<ARwLock<GlobalContext>>,
    event: HookEvent,
    payload: HookPayload,
) -> Vec<HookResult> {
    let tool_name = payload.tool_name.clone();
    let matching_hooks = get_hooks_for_event(gcx, event, tool_name.as_deref()).await;
    run_hooks_from_list(&matching_hooks, &payload).await
}

async fn read_bounded<R: tokio::io::AsyncRead + Unpin>(mut reader: R, max_bytes: usize) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                let to_store = (max_bytes.saturating_sub(buf.len())).min(n);
                if to_store > 0 {
                    buf.extend_from_slice(&chunk[..to_store]);
                }
            }
            Err(_) => break,
        }
    }
    buf
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
    let timeout_dur = Duration::from_secs(timeout_secs);

    let mut cmd = make_hook_command(config, payload);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("hooks_runner: failed to spawn '{}': {}", config.command, e);
            return HookResult::Warning(format!("Failed to spawn: {}", e));
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload_json.as_bytes()).await;
        drop(stdin);
    }

    let stdout_task = child
        .stdout
        .take()
        .map(|out| tokio::spawn(read_bounded(out, HOOK_MAX_OUTPUT_BYTES)));
    let stderr_task = child
        .stderr
        .take()
        .map(|err| tokio::spawn(read_bounded(err, HOOK_MAX_OUTPUT_BYTES)));

    match tokio::time::timeout(timeout_dur, child.wait()).await {
        Ok(Ok(status)) => {
            let stdout_bytes = match stdout_task {
                Some(t) => t.await.unwrap_or_default(),
                None => Vec::new(),
            };
            let stderr_bytes = match stderr_task {
                Some(t) => t.await.unwrap_or_default(),
                None => Vec::new(),
            };
            let stdout = String::from_utf8_lossy(&stdout_bytes).into_owned();
            let stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();
            match status.code().unwrap_or(-1) {
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
        Ok(Err(e)) => {
            tracing::warn!(
                "hooks_runner: failed to wait for '{}': {}",
                config.command,
                e
            );
            if let Some(t) = stdout_task {
                t.abort();
            }
            if let Some(t) = stderr_task {
                t.abort();
            }
            HookResult::Warning(format!("Failed to wait: {}", e))
        }
        Err(_) => {
            tracing::warn!(
                "hooks_runner: hook timed out after {}s, killing process: {}",
                timeout_secs,
                config.command
            );
            let _ = child.kill().await;
            let _ = child.wait().await;
            if let Some(t) = stdout_task {
                t.abort();
            }
            if let Some(t) = stderr_task {
                t.abort();
            }
            HookResult::Timeout
        }
    }
}

fn make_hook_command(config: &HookConfig, payload: &HookPayload) -> tokio::process::Command {
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

    fn make_hook_config(event: HookEvent, matcher: Option<&str>, command: &str) -> HookConfig {
        crate::ext::hooks::HookConfig {
            event,
            matcher: matcher.map(|s| s.to_string()),
            command: command.to_string(),
            timeout: Some(5),
            source: crate::ext::config_dirs::CommandSource::GlobalRefact,
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
        payload
            .extra
            .insert("finish_reason".to_string(), serde_json::json!("stop"));
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("finish_reason"));
        assert!(json.contains("\"stop\""));
    }

    #[test]
    fn test_compiled_matcher_matches_none_matches_all() {
        assert!(compiled_matcher_matches(None, Some("shell")));
        assert!(compiled_matcher_matches(None, Some("cat")));
        assert!(compiled_matcher_matches(None, None));
    }

    #[test]
    fn test_compiled_matcher_matches_pattern_with_tool_name() {
        let re = regex::Regex::new("Bash|shell").unwrap();
        assert!(compiled_matcher_matches(Some(&re), Some("shell")));
        assert!(compiled_matcher_matches(Some(&re), Some("Bash")));
        assert!(!compiled_matcher_matches(Some(&re), Some("cat")));
    }

    #[test]
    fn test_compiled_matcher_matches_pattern_without_tool_name_returns_false() {
        let re = regex::Regex::new("shell").unwrap();
        assert!(!compiled_matcher_matches(Some(&re), None));
    }

    #[test]
    fn test_compile_hooks_skips_invalid_regex() {
        let configs = vec![
            make_hook_config(HookEvent::PreToolUse, Some("[invalid"), "cmd1"),
            make_hook_config(HookEvent::PreToolUse, Some("shell"), "cmd2"),
        ];
        let compiled = compile_hooks(configs);
        assert_eq!(compiled.len(), 1);
        assert_eq!(compiled[0].config.command, "cmd2");
    }

    #[test]
    fn test_compile_hooks_none_matcher_becomes_match_all() {
        let configs = vec![make_hook_config(HookEvent::SessionStart, None, "cmd")];
        let compiled = compile_hooks(configs);
        assert_eq!(compiled.len(), 1);
        assert!(compiled[0].compiled_matcher.is_none());
        assert!(compiled_matcher_matches(
            compiled[0].compiled_matcher.as_ref(),
            Some("anything")
        ));
        assert!(compiled_matcher_matches(
            compiled[0].compiled_matcher.as_ref(),
            None
        ));
    }

    #[test]
    fn test_compile_hooks_empty_matcher_becomes_match_all() {
        let configs = vec![make_hook_config(HookEvent::SessionStart, Some(""), "cmd")];
        let compiled = compile_hooks(configs);
        assert_eq!(compiled.len(), 1);
        assert!(compiled[0].compiled_matcher.is_none());
    }

    #[test]
    fn test_hooks_cache_returns_same_result() {
        let configs = vec![
            make_hook_config(HookEvent::PreToolUse, Some("shell|bash"), "cmd1"),
            make_hook_config(HookEvent::PostToolUse, None, "cmd2"),
        ];
        let compiled1 = compile_hooks(configs.clone());
        let compiled2 = compile_hooks(configs);
        assert_eq!(compiled1.len(), compiled2.len());
        assert_eq!(compiled1.len(), 2);
        assert!(compiled_matcher_matches(
            compiled1[0].compiled_matcher.as_ref(),
            Some("shell")
        ));
        assert!(compiled_matcher_matches(
            compiled2[0].compiled_matcher.as_ref(),
            Some("shell")
        ));
        assert!(!compiled_matcher_matches(
            compiled1[0].compiled_matcher.as_ref(),
            Some("cat")
        ));
        assert!(!compiled_matcher_matches(
            compiled2[0].compiled_matcher.as_ref(),
            Some("cat")
        ));
        assert!(compiled_matcher_matches(
            compiled1[1].compiled_matcher.as_ref(),
            None
        ));
        assert!(compiled_matcher_matches(
            compiled2[1].compiled_matcher.as_ref(),
            None
        ));
    }

    #[tokio::test]
    async fn test_run_single_hook_success() {
        let config = make_hook_config(HookEvent::PreToolUse, None, "echo success_output");
        let payload = make_payload("PreToolUse", Some("shell"));
        let result = run_single_hook(&config, &payload).await;
        match result {
            HookResult::Success(out) => assert!(out.contains("success_output")),
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_run_single_hook_exit_2_blocks() {
        let config = make_hook_config(
            HookEvent::PreToolUse,
            None,
            "sh -c 'echo block_reason >&2; exit 2'",
        );
        let payload = make_payload("PreToolUse", Some("shell"));
        let result = run_single_hook(&config, &payload).await;
        match result {
            HookResult::Block(reason) => assert!(reason.contains("block_reason")),
            other => panic!("Expected Block, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_run_single_hook_nonzero_exit_warning() {
        let config = make_hook_config(
            HookEvent::PostToolUse,
            None,
            "sh -c 'echo warn_output >&2; exit 1'",
        );
        let payload = make_payload("PostToolUse", Some("cat"));
        let result = run_single_hook(&config, &payload).await;
        match result {
            HookResult::Warning(msg) => assert!(msg.contains("warn_output")),
            other => panic!("Expected Warning, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_run_single_hook_timeout() {
        let mut config = make_hook_config(HookEvent::SessionStart, None, "sleep 60");
        config.timeout = Some(1);
        let payload = make_payload("SessionStart", None);
        let result = run_single_hook(&config, &payload).await;
        assert!(matches!(result, HookResult::Timeout));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_hook_timeout_kills_process() {
        let tmpdir = tempfile::tempdir().unwrap();
        let pid_file = tmpdir.path().join("pid.txt");
        let pid_path_str = pid_file.to_str().unwrap().to_string();
        let cmd = format!("echo $$ > '{}'; sleep 60", pid_path_str);
        let config = crate::ext::hooks::HookConfig {
            event: HookEvent::SessionStart,
            matcher: None,
            command: cmd,
            timeout: Some(2),
            source: crate::ext::config_dirs::CommandSource::GlobalRefact,
        };
        let payload = make_payload("SessionStart", None);
        let result = run_single_hook(&config, &payload).await;
        assert!(matches!(result, HookResult::Timeout));

        let pid_str = std::fs::read_to_string(&pid_file)
            .expect("PID file should have been written by the child process");
        let pid: u32 = pid_str.trim().parse().expect("PID should be numeric");

        let kill_output = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .expect("kill command should execute");
        assert!(
            !kill_output.status.success(),
            "Process {} should have been killed and reaped after timeout, but kill -0 succeeded",
            pid
        );
    }

    #[test]
    fn test_first_block_reason_found() {
        let results = vec![
            HookResult::Success("ok".to_string()),
            HookResult::Block("blocked".to_string()),
            HookResult::Warning("warn".to_string()),
        ];
        assert_eq!(first_block_reason(&results), Some("blocked".to_string()));
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
    fn test_is_global_source() {
        use std::path::PathBuf;
        assert!(is_global_source(
            &crate::ext::config_dirs::CommandSource::GlobalClaude
        ));
        assert!(is_global_source(
            &crate::ext::config_dirs::CommandSource::GlobalRefact
        ));
        assert!(!is_global_source(
            &crate::ext::config_dirs::CommandSource::ProjectClaude(PathBuf::from("/p"))
        ));
        assert!(!is_global_source(
            &crate::ext::config_dirs::CommandSource::ProjectRefact(PathBuf::from("/p"))
        ));
    }

    #[test]
    fn test_project_hooks_skipped_by_default() {
        use std::path::PathBuf;
        let source =
            crate::ext::config_dirs::CommandSource::ProjectRefact(PathBuf::from("/project"));
        assert!(!is_global_source(&source));
    }

    #[test]
    fn test_global_hooks_still_run() {
        let source = crate::ext::config_dirs::CommandSource::GlobalRefact;
        assert!(is_global_source(&source));
    }

    #[tokio::test]
    async fn test_hooks_cache_project_separation() {
        use std::path::PathBuf;
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();

        tokio::fs::write(
            tmp1.path().join("hooks.yaml"),
            "hooks:\n  SessionStart:\n    - hooks:\n        - type: command\n          command: cmd_project_a",
        ).await.unwrap();

        tokio::fs::write(
            tmp2.path().join("hooks.yaml"),
            "hooks:\n  SessionEnd:\n    - hooks:\n        - type: command\n          command: cmd_project_b",
        ).await.unwrap();

        let ext_dirs1 = ExtDirs {
            global_dirs: vec![tmp1.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![PathBuf::from("/project/a")],
        };
        let ext_dirs2 = ExtDirs {
            global_dirs: vec![tmp2.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![PathBuf::from("/project/b")],
        };

        let hooks1 = get_compiled_hooks_from_ext_dirs(&ext_dirs1).await;
        let hooks2 = get_compiled_hooks_from_ext_dirs(&ext_dirs2).await;

        assert!(hooks1.iter().any(|h| h.config.command == "cmd_project_a"));
        assert!(!hooks1.iter().any(|h| h.config.command == "cmd_project_b"));
        assert!(hooks2.iter().any(|h| h.config.command == "cmd_project_b"));
        assert!(!hooks2.iter().any(|h| h.config.command == "cmd_project_a"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_run_single_hook_large_output_truncated() {
        let config = make_hook_config(HookEvent::PreToolUse, None, "yes | head -c 102400");
        let payload = make_payload("PreToolUse", Some("shell"));
        let result = run_single_hook(&config, &payload).await;
        match result {
            HookResult::Success(out) => {
                assert!(
                    out.len() <= HOOK_MAX_OUTPUT_BYTES,
                    "output should be bounded: {} > {}",
                    out.len(),
                    HOOK_MAX_OUTPUT_BYTES
                );
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_run_single_hook_large_stderr_truncated() {
        let config = make_hook_config(
            HookEvent::PreToolUse,
            None,
            "yes | head -c 102400 >&2; exit 2",
        );
        let payload = make_payload("PreToolUse", Some("shell"));
        let result = run_single_hook(&config, &payload).await;
        match result {
            HookResult::Block(reason) => {
                assert!(
                    reason.len() <= HOOK_MAX_OUTPUT_BYTES,
                    "stderr should be bounded: {} > {}",
                    reason.len(),
                    HOOK_MAX_OUTPUT_BYTES
                );
            }
            other => panic!("Expected Block, got {:?}", other),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_run_hooks_parallel() {
        let hooks = vec![
            make_hook_config(HookEvent::PreToolUse, None, "sleep 1"),
            make_hook_config(HookEvent::PreToolUse, None, "sleep 1"),
            make_hook_config(HookEvent::PreToolUse, None, "sleep 1"),
        ];
        let payload = make_payload("PreToolUse", None);
        let start = std::time::Instant::now();
        let results = run_hooks_from_list(&hooks, &payload).await;
        let elapsed = start.elapsed();
        assert_eq!(results.len(), 3);
        assert!(
            elapsed.as_secs() < 2,
            "Hooks should run in parallel (sum=3s, parallel~1s), took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_run_hooks_parallel_block_propagates() {
        let hooks = vec![
            make_hook_config(HookEvent::PreToolUse, None, "echo success"),
            make_hook_config(
                HookEvent::PreToolUse,
                None,
                "sh -c 'echo blocked >&2; exit 2'",
            ),
            make_hook_config(HookEvent::PreToolUse, None, "echo success2"),
        ];
        let payload = make_payload("PreToolUse", None);
        let results = run_hooks_from_list(&hooks, &payload).await;
        assert_eq!(results.len(), 3);
        let block_reason = first_block_reason(&results);
        assert!(
            block_reason.is_some(),
            "Expected a Block result from one of the parallel hooks"
        );
        assert!(block_reason.unwrap().contains("blocked"));
    }

    #[test]
    fn test_subagent_stop_event_fires() {
        let configs = vec![make_hook_config(
            HookEvent::SubagentStop,
            None,
            "echo subagent_done",
        )];
        let compiled = compile_hooks(configs);
        assert_eq!(compiled.len(), 1);
        assert_eq!(compiled[0].config.event, HookEvent::SubagentStop);
        assert!(compiled_matcher_matches(
            compiled[0].compiled_matcher.as_ref(),
            None
        ));
    }

    #[test]
    fn test_notification_event_fires() {
        let configs = vec![make_hook_config(
            HookEvent::Notification,
            None,
            "echo notify",
        )];
        let compiled = compile_hooks(configs);
        assert_eq!(compiled.len(), 1);
        assert_eq!(compiled[0].config.event, HookEvent::Notification);
        assert!(compiled_matcher_matches(
            compiled[0].compiled_matcher.as_ref(),
            None
        ));
    }
}
