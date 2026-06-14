use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use crate::app_state::AppState;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::exec::{
    sanitize_short_description, ExecOutputStream, ExecOwnerMeta, ExecRawOutput, ExecReadResult,
    ExecSpawnRequest, ExecStatus,
};
use crate::files_correction::get_active_project_path;
use crate::scheduler::types::{CommandSpec, Job};

const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 300;
const MAX_COMMAND_TIMEOUT_SECS: u64 = 3600;
const COMMAND_TRANSCRIPT_MAX_BYTES: usize = 2 * 1024 * 1024;

pub struct CommandRunResult {
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

pub async fn run_command(app: &AppState, job: &Job, cmd: &CommandSpec) -> CommandRunResult {
    match run_command_inner(app, job, cmd).await {
        Ok(result) => result,
        Err(message) => CommandRunResult {
            status: "error".to_string(),
            stdout: String::new(),
            stderr: message,
            exit_code: None,
        },
    }
}

async fn run_command_inner(
    app: &AppState,
    job: &Job,
    cmd: &CommandSpec,
) -> Result<CommandRunResult, String> {
    let command = command_from_argv(&cmd.argv)?;
    let project_root = get_active_project_path(app.gcx.clone()).await;
    let cwd = resolve_cwd(project_root.as_deref(), cmd.cwd.as_deref())?;
    let owner = ExecOwnerMeta {
        chat_id: job.chat_id().map(str::to_string),
        tool_call_id: Some(format!("cron:{}", job.id)),
        service_name: None,
        workspace: project_root.clone(),
    };
    let timeout_secs = cmd
        .timeout_secs
        .unwrap_or(DEFAULT_COMMAND_TIMEOUT_SECS)
        .min(MAX_COMMAND_TIMEOUT_SECS);
    let mut request = ExecSpawnRequest::foreground(command)
        .with_timeout(Duration::from_secs(timeout_secs))
        .with_owner(owner)
        .with_transcript_limit(COMMAND_TRANSCRIPT_MAX_BYTES)
        .with_short_description(sanitize_short_description(&job.description))
        .with_tty(false);
    if let Some(cwd) = cwd {
        request = request.with_cwd(cwd);
    }
    request = request.with_env_map(command_env(cmd));
    let result = app.runtime.exec_registry.spawn(request).await?;
    let process_id = result.snapshot.meta.process_id.clone();
    let read = app.runtime.exec_registry.read(&process_id, 0, None).await;
    let raw_output = app
        .runtime
        .exec_registry
        .read_raw_capture(&process_id)
        .await;
    let (mut stdout, mut stderr) = collect_foreground_output(&read, raw_output.as_ref());
    if read.is_truncated {
        stderr.push_str(&format!(
            "Output was truncated by exec transcript limits ({} bytes kept, {} bytes dropped, {} chunks truncated).\n",
            read.current_bytes, read.dropped_bytes, read.truncated_chunks
        ));
    }
    if let Some(raw_output) = raw_output.as_ref() {
        if raw_output.is_truncated() {
            stderr.push_str(&format!(
                "Raw foreground capture reached limits (stdout: {}/{} bytes kept, {} bytes elided; stderr: {}/{} bytes kept, {} bytes elided).\n",
                raw_output.stdout_captured_bytes,
                raw_output.stdout_max_bytes,
                raw_output.stdout_elided_bytes,
                raw_output.stderr_captured_bytes,
                raw_output.stderr_max_bytes,
                raw_output.stderr_elided_bytes
            ));
        }
    }
    let exit_code = exec_exit_code(&result.snapshot.status);
    let status = if command_status_is_success(&result.snapshot.status) {
        "fired"
    } else {
        append_status_summary(&mut stderr, &result.snapshot.status, timeout_secs);
        "error"
    };
    Ok(CommandRunResult {
        status: status.to_string(),
        stdout: std::mem::take(&mut stdout),
        stderr,
        exit_code,
    })
}

pub fn command_output_message(task: &Job, result: &CommandRunResult) -> ChatMessage {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "scheduler_command".to_string(),
        json!({
            "task_id": task.id,
            "status": result.status,
            "exit_code": result.exit_code,
        }),
    );
    ChatMessage {
        role: "plain_text".to_string(),
        content: ChatContent::SimpleText(result.stdout.clone()),
        extra,
        ..Default::default()
    }
}

pub fn command_error_notice_message(task: &Job, result: &CommandRunResult) -> ChatMessage {
    event(
        EventSubkind::SystemNotice,
        "scheduler.cron",
        json!({
            "task_id": task.id,
            "action_kind": "command",
            "status": result.status,
            "exit_code": result.exit_code,
        }),
        command_error_summary(result),
    )
}

pub fn command_error_summary(result: &CommandRunResult) -> String {
    let stderr = result.stderr.trim();
    if stderr.is_empty() {
        return "Scheduled command failed".to_string();
    }
    format!("Scheduled command failed: {}", first_line(stderr))
}

fn command_from_argv(argv: &[String]) -> Result<String, String> {
    if argv.is_empty() {
        return Err("command argv is empty".to_string());
    }
    if argv.iter().any(|part| part.trim().is_empty()) {
        return Err("command argv contains an empty argument".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        Ok(argv
            .iter()
            .map(|part| powershell_escape(part))
            .collect::<Vec<_>>()
            .join(" "))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(shell_words::join(argv))
    }
}

fn command_env(cmd: &CommandSpec) -> HashMap<String, String> {
    cmd.env
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect::<HashMap<_, _>>()
}

fn resolve_cwd(project_root: Option<&Path>, cwd: Option<&str>) -> Result<Option<PathBuf>, String> {
    let Some(cwd) = cwd.map(str::trim).filter(|cwd| !cwd.is_empty()) else {
        return Ok(project_root.map(Path::to_path_buf));
    };
    let root =
        project_root.ok_or_else(|| "No active project root available for cwd".to_string())?;
    let candidate = PathBuf::from(cwd);
    let joined = if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    };
    let normalized_root = normalize_existing_or_lexical(root);
    let normalized_joined = normalize_existing_or_lexical(&joined);
    if !normalized_joined.starts_with(&normalized_root) {
        return Err("cwd must stay within the active project".to_string());
    }
    Ok(Some(normalized_joined))
}

fn normalize_existing_or_lexical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .map(|path| dunce::simplified(&path).to_path_buf())
        .unwrap_or_else(|_| dunce::simplified(&lexical_normalize(path)).to_path_buf())
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let last = normalized.components().next_back().map(|component| {
                    (
                        matches!(component, std::path::Component::Normal(_)),
                        matches!(component, std::path::Component::ParentDir),
                    )
                });
                match last {
                    Some((true, _)) => {
                        normalized.pop();
                    }
                    Some((_, true)) | None => normalized.push(".."),
                    Some((false, false)) => {}
                }
            }
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::Normal(part) => normalized.push(part),
        }
    }
    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

fn collect_exec_output(read: &ExecReadResult) -> (String, String) {
    let mut stdout = String::new();
    let mut stderr = String::new();
    for chunk in &read.chunks {
        match chunk.stream {
            ExecOutputStream::Stdout | ExecOutputStream::Combined => stdout.push_str(&chunk.text),
            ExecOutputStream::Stderr => stderr.push_str(&chunk.text),
        }
    }
    (stdout, stderr)
}

fn collect_foreground_output(
    read: &ExecReadResult,
    raw_output: Option<&ExecRawOutput>,
) -> (String, String) {
    raw_output
        .map(|raw| (raw.stdout.clone(), raw.stderr.clone()))
        .unwrap_or_else(|| collect_exec_output(read))
}

fn exec_exit_code(status: &ExecStatus) -> Option<i32> {
    match status {
        ExecStatus::Exited { exit_code } => *exit_code,
        ExecStatus::Starting
        | ExecStatus::Running
        | ExecStatus::Failed { .. }
        | ExecStatus::Killed
        | ExecStatus::TimedOut => None,
    }
}

fn command_status_is_success(status: &ExecStatus) -> bool {
    match status {
        ExecStatus::Exited { exit_code } => exit_code.unwrap_or_default() == 0,
        ExecStatus::Starting | ExecStatus::Running => true,
        ExecStatus::Failed { .. } | ExecStatus::Killed | ExecStatus::TimedOut => false,
    }
}

fn append_status_summary(stderr: &mut String, status: &ExecStatus, timeout_secs: u64) {
    let summary = match status {
        ExecStatus::Exited { exit_code } => {
            format!("command exited with code {}", exit_code.unwrap_or_default())
        }
        ExecStatus::Failed { message } => format!("command failed: {message}"),
        ExecStatus::Killed => "command was killed".to_string(),
        ExecStatus::TimedOut => format!("command timed out after {timeout_secs} seconds"),
        ExecStatus::Starting | ExecStatus::Running => {
            format!("command ended with status {status:?}")
        }
    };
    if !stderr.is_empty() && !stderr.ends_with('\n') {
        stderr.push('\n');
    }
    stderr.push_str(&summary);
    stderr.push('\n');
}

fn first_line(value: &str) -> &str {
    value
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(value)
}

#[cfg(target_os = "windows")]
fn powershell_escape(value: &str) -> String {
    let mut needs_escape = value.is_empty();
    for ch in value.chars() {
        match ch {
            ' ' | '"' | '\'' | '$' | '`' | '[' | ']' | '{' | '}' | '(' | ')' | '@' | '&' | '#'
            | ',' | ';' | '.' | '\t' | '\n' | '|' | '<' | '>' | '\\' => {
                needs_escape = true;
                break;
            }
            _ => {}
        }
    }
    if !needs_escape {
        return value.to_string();
    }
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("`\""),
            '$' => escaped.push_str("`$"),
            '`' => escaped.push_str("``"),
            '\t' => escaped.push_str("`t"),
            '\n' => escaped.push_str("`n"),
            '\\' => escaped.push_str("\\"),
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::scheduler::types::{Action, AgentTarget, Delivery, Trigger};

    fn job() -> Job {
        Job {
            id: "command-job".to_string(),
            description: "Run command".to_string(),
            enabled: true,
            durable: false,
            created_at_ms: 1_000,
            recurring: true,
            trigger: Trigger::Interval { every_ms: 60_000 },
            action: Action::Command {
                argv: vec!["printf".to_string(), "hello".to_string()],
                target: AgentTarget::ExistingChat {
                    chat_id: "chat".to_string(),
                },
                cwd: None,
                env: None,
                timeout_secs: None,
            },
            delivery: Delivery::Chat,
            last_fired_at_ms: None,
            fire_count: 0,
            last_status: None,
            last_error: None,
            recent_runs: Vec::new(),
            paused_at_ms: None,
            trigger_at_ms: None,
            auto_expire_after_ms: crate::scheduler::DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS,
        }
    }

    #[test]
    fn command_from_argv_quotes_arguments() {
        let command = command_from_argv(&[
            "printf".to_string(),
            "%s".to_string(),
            "hello world".to_string(),
        ])
        .unwrap();

        assert!(command.contains("printf"));
        assert!(command.contains("hello"));
        assert_eq!(shell_words::split(&command).unwrap()[2], "hello world");
    }

    #[test]
    fn resolve_cwd_rejects_parent_escape() {
        let root = PathBuf::from("/tmp/project");

        let error = resolve_cwd(Some(&root), Some("../outside")).unwrap_err();

        assert_eq!(error, "cwd must stay within the active project");
    }

    #[test]
    fn command_error_summary_prefers_stderr() {
        let result = CommandRunResult {
            status: "error".to_string(),
            stdout: String::new(),
            stderr: "boom\nmore".to_string(),
            exit_code: Some(2),
        };

        assert_eq!(
            command_error_summary(&result),
            "Scheduled command failed: boom"
        );
    }

    #[test]
    fn command_output_message_is_plain_text() {
        let result = CommandRunResult {
            status: "fired".to_string(),
            stdout: "hello".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
        };
        let message = command_output_message(&job(), &result);

        assert_eq!(message.role, "plain_text");
        assert_eq!(message.content.content_text_only(), "hello");
        assert_eq!(
            message.extra["scheduler_command"]["task_id"],
            json!("command-job")
        );
    }

    #[test]
    fn command_env_uses_overrides_only() {
        let mut env = BTreeMap::new();
        env.insert("FROG".to_string(), "green".to_string());
        let cmd = CommandSpec {
            argv: vec!["env".to_string()],
            target: AgentTarget::ExistingChat {
                chat_id: "chat".to_string(),
            },
            cwd: None,
            env: Some(env),
            timeout_secs: None,
        };

        assert_eq!(command_env(&cmd).get("FROG"), Some(&"green".to_string()));
    }
}
