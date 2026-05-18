use std::path::PathBuf;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::process::Stdio;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::Mutex as AMutex;
use tokio::io::{AsyncBufReadExt, BufReader};
use async_trait::async_trait;
use tokio::process::Command;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::return_one_candidate_or_a_good_error;
use crate::files_correction::canonical_path;
use crate::files_correction::canonicalize_normalized_path;
use crate::files_correction::check_if_its_inside_a_workspace_or_config;
use crate::files_correction::correct_to_nearest_dir_path;
use crate::files_correction::get_active_project_path;
use crate::files_correction::get_project_dirs;
use crate::files_correction::preprocess_path_for_normalization;
use crate::files_correction::CommandSimplifiedDirExt;
use crate::global_context::GlobalContext;
use crate::privacy::{check_file_privacy, load_privacy_if_needed, FilePrivacyLevel};
use crate::tools::file_edit::auxiliary::{active_execution_scope, scoped_path_warnings};
use crate::worktrees::scope::ExecutionScope;
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::postprocessing::pp_command_output::{
    OutputFilter, parse_output_filter_args, output_mini_postprocessing,
};
use crate::postprocessing::pp_capture_buffer::{CaptureBuffer, KeepStrategy};
use crate::integrations::integr_abstract::IntegrationConfirmation;

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct SettingsShell {
    #[serde(default)]
    pub timeout: String,
    #[serde(default)]
    pub output_filter: OutputFilter,
}

#[derive(Default)]
pub struct ToolShell {
    pub cfg: SettingsShell,
    pub config_path: String,
}

const ASK_USER_DEFAULT: &[&str] = &[
    "*rm*",
    "*rmdir*",
    "*del /s*",
    "*deltree*",
    "*mkfs*",
    "*dd *",
    "*format*",
    "*> /dev/*",
    ":(){ :|:& };:",
    "*chmod -R*",
    "*chown -R*",
    "*chmod 777*",
    "*chmod a+rwx*",
    "*git push*",
    "*git reset --hard*",
    "curl * | sh",
    "curl * | bash",
    "wget * -O - | sh",
    "wget * -O - | bash",
    "*apt-get remove*",
    "*apt-get purge*",
    "*apt remove*",
    "*apt purge*",
    "*yum remove*",
    "*yum erase*",
    "*dnf remove*",
    "*pacman -R*",
    "*brew uninstall*",
    "*docker rm*",
    "*docker rmi*",
    "*docker system prune*",
    "*kubectl delete*",
    "*kill -9*",
    "*killall*",
    "*pkill*",
    "*shutdown*",
    "*reboot*",
    "*halt*",
    "*poweroff*",
    "*init 0*",
    "*init 6*",
    "*systemctl stop*",
    "*systemctl disable*",
    "*service * stop",
    "*truncate -s 0*",
    "*fdisk*",
    "*parted*",
    "*mkswap*",
    "*swapon*",
    "*swapoff*",
    "*mount*",
    "*umount*",
    "*crontab -r*",
    "*history -c*",
    "*shred*",
    "*wipe*",
    "*srm*",
];

const DENY_DEFAULT: &[&str] = &["sudo*"];

const MAX_CAPTURE_BYTES: usize = 2 * 1024 * 1024;

struct OutputCollector {
    stdout: CaptureBuffer,
    stderr: CaptureBuffer,
}

impl OutputCollector {
    fn new() -> Self {
        Self {
            stdout: CaptureBuffer::new(MAX_CAPTURE_BYTES, KeepStrategy::HeadAndTail),
            stderr: CaptureBuffer::new(MAX_CAPTURE_BYTES / 4, KeepStrategy::HeadAndTail),
        }
    }

    fn push_stdout(&mut self, line: String) {
        self.stdout.push_line(line);
    }

    fn push_stderr(&mut self, line: String) {
        self.stderr.push_line(line);
    }
}

pub struct ShellStreamResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_secs: f64,
    pub interrupted: bool,
}

#[async_trait]
impl Tool for ToolShell {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, subchat_tx, abort_flag, execution_scope) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.subchat_tx.clone(),
                ccx_lock.abort_flag.clone(),
                ccx_lock.execution_scope.clone(),
            )
        };
        let (command, workdir_maybe, custom_filter, timeout_override, scope_warnings) =
            parse_args_with_filter(
                gcx.clone(),
                args,
                &self.cfg.output_filter,
                execution_scope.as_ref(),
            )
            .await?;
        let timeout =
            timeout_override.unwrap_or_else(|| self.cfg.timeout.parse::<u64>().unwrap_or(10));

        let mut error_log = Vec::new();
        let env_variables =
            crate::integrations::setting_up_integrations::get_vars_for_replacements(
                gcx.clone(),
                &mut error_log,
            )
            .await;

        let output_filter = custom_filter.unwrap_or_else(|| self.cfg.output_filter.clone());

        let result = execute_shell_command_with_streaming(
            &command,
            &workdir_maybe,
            timeout,
            &env_variables,
            gcx.clone(),
            &subchat_tx,
            tool_call_id,
            abort_flag,
        )
        .await?;

        let filtered_stdout = output_mini_postprocessing(&output_filter, &result.stdout);
        let filtered_stderr = output_mini_postprocessing(&output_filter, &result.stderr);

        let mut out =
            crate::integrations::integr_cmdline::format_output(&filtered_stdout, &filtered_stderr);
        if !scope_warnings.is_empty() {
            out = format!("{}\n{}", scope_warnings.join("\n"), out);
        }
        if result.interrupted {
            out.push_str(&format!(
                "⚠️ The command was interrupted by user after {:.3}s (process killed). Output above may be incomplete.\n",
                result.duration_secs
            ));
        } else {
            out.push_str(&format!(
                "The command was running {:.3}s, finished with exit code {}\n",
                result.duration_secs, result.exit_code
            ));
        }

        let msg = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(out),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            tool_failed: if result.interrupted { Some(true) } else { None },
            output_filter: Some(OutputFilter::no_limits()),
            ..Default::default()
        })];

        Ok((false, msg))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "shell".to_string(),
            display_name: "Shell".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Execute a single command, using the \"sh\" on unix-like systems and \"powershell.exe\" on windows. Use it for one-time tasks like dependencies installation. Don't call this unless you have to. Not suitable for regular work because it requires a confirmation at each step. Output is compressed by default - use output_filter and output_limit parameters to see specific parts if needed. In worktree-scoped chats, the default cwd and explicit workdir are enforced to the active worktree or privacy-permitted outside paths with visible warnings; the shell command text itself is not OS-sandboxed. Note: sudo commands cannot be run - if you need elevated privileges, ask the user to run them directly.".to_string(),
            input_schema: json_schema_from_params(&[("command", "string", "shell command to execute"), ("workdir", "string", "workdir for the command"), ("output_filter", "string", "Optional regex pattern to filter output lines. Only lines matching this pattern (and context) will be shown. Use to find specific errors or content in large outputs."), ("output_limit", "string", "Optional. Max lines to show (default: 40). Use higher values like '200' or 'all' to see more output."), ("timeout", "string", "Optional. Timeout in seconds for the command (default: 10). Use higher values for long-running commands.")], &["command", "workdir"]),
            output_schema: None,
            annotations: None,
        }
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let (gcx, execution_scope) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.app.gcx.clone(),
                cgcx.execution_scope.clone(),
            )
        };
        let (command, _) = parse_args(gcx, args, execution_scope.as_ref()).await?;
        Ok(command)
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: ASK_USER_DEFAULT.iter().map(|s| s.to_string()).collect(),
            deny: DENY_DEFAULT.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

fn send_streaming_update(
    subchat_tx: &Arc<AMutex<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>>,
    tool_call_id: &str,
    content: &str,
) {
    let streaming_msg = json!({
        "tool_call_id": tool_call_id,
        "subchat_id": content,
        "add_message": {
            "role": "assistant",
            "content": content
        }
    });
    if let Ok(tx) = subchat_tx.try_lock() {
        let _ = tx.send(streaming_msg);
    }
}

fn spawn_output_streaming_task(
    subchat_tx: Arc<AMutex<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>>,
    tool_call_id: String,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    cancel_token: tokio_util::sync::CancellationToken,
    output_collector: Arc<AMutex<OutputCollector>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();
        let mut last_update = tokio::time::Instant::now();
        let update_interval = tokio::time::Duration::from_secs(2);
        let mut stdout_line_count: usize = 0;
        let mut stdout_done = false;
        let mut stderr_done = false;

        loop {
            if stdout_done && stderr_done {
                break;
            }
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    break;
                }
                result = stdout_reader.next_line(), if !stdout_done => {
                    match result {
                        Ok(Some(line)) => {
                            let stripped = strip_ansi_escapes::strip(line.as_bytes());
                            let clean_line = String::from_utf8_lossy(&stripped).to_string();
                            {
                                let mut collector = output_collector.lock().await;
                                collector.push_stdout(clean_line);
                            }
                            stdout_line_count += 1;
                            if last_update.elapsed() >= update_interval {
                                send_streaming_update(
                                    &subchat_tx,
                                    &tool_call_id,
                                    &format!("📤 stdout ({} lines captured)", stdout_line_count)
                                );
                                last_update = tokio::time::Instant::now();
                            }
                        }
                        Ok(None) => { stdout_done = true; }
                        Err(e) => {
                            tracing::warn!("Error reading stdout: {}", e);
                            stdout_done = true;
                        }
                    }
                }
                result = stderr_reader.next_line(), if !stderr_done => {
                    match result {
                        Ok(Some(line)) => {
                            let stripped = strip_ansi_escapes::strip(line.as_bytes());
                            let clean_line = String::from_utf8_lossy(&stripped).to_string();
                            {
                                let mut collector = output_collector.lock().await;
                                collector.push_stderr(clean_line.clone());
                            }
                            if !clean_line.trim().is_empty() {
                                send_streaming_update(
                                    &subchat_tx,
                                    &tool_call_id,
                                    &format!("⚠️ stderr: {}", clean_line)
                                );
                            }
                        }
                        Ok(None) => { stderr_done = true; }
                        Err(e) => {
                            tracing::warn!("Error reading stderr: {}", e);
                            stderr_done = true;
                        }
                    }
                }
            }
        }
    })
}

/// Kill a child process and reap it to prevent zombies.
/// On Unix, kills the entire process group.
async fn kill_and_reap(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            // Kill the entire process group
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
    let _ = child.kill().await;
    // Reap the child to prevent zombie processes
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(2), child.wait()).await;
}

pub async fn execute_shell_command_with_streaming(
    command: &str,
    workdir_maybe: &Option<PathBuf>,
    timeout: u64,
    env_variables: &HashMap<String, String>,
    gcx: Arc<GlobalContext>,
    subchat_tx: &Arc<AMutex<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>>,
    tool_call_id: &str,
    abort_flag: Arc<AtomicBool>,
) -> Result<ShellStreamResult, String> {
    let shell = if cfg!(target_os = "windows") {
        "powershell.exe"
    } else {
        "sh"
    };
    let shell_arg = if cfg!(target_os = "windows") {
        "-Command"
    } else {
        "-c"
    };
    let mut cmd = Command::new(shell);

    if let Some(workdir) = workdir_maybe {
        cmd.current_dir_simplified(workdir);
    } else if let Some(project_path) = get_active_project_path(gcx.clone()).await {
        cmd.current_dir_simplified(&project_path);
    } else {
        tracing::warn!("no working directory, using whatever directory this binary is run :/");
    }

    for (key, value) in env_variables {
        cmd.env(key, value);
    }

    cmd.arg(shell_arg).arg(command);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // On Unix, spawn in its own process group so we can kill the entire tree
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }

    tracing::info!(
        "SHELL: running command directory {:?}\n{:?}",
        workdir_maybe,
        command
    );

    send_streaming_update(
        subchat_tx,
        tool_call_id,
        &format!("🔧 Running: {}", command),
    );

    let t0 = tokio::time::Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn command: {}", e))?;

    let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
    let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;

    let output_collector: Arc<AMutex<OutputCollector>> =
        Arc::new(AMutex::new(OutputCollector::new()));
    let cancel_token = tokio_util::sync::CancellationToken::new();

    let streaming_handle = spawn_output_streaming_task(
        subchat_tx.clone(),
        tool_call_id.to_string(),
        stdout,
        stderr,
        cancel_token.clone(),
        output_collector.clone(),
    );

    let timeout_duration = tokio::time::Duration::from_secs(timeout);
    let abort_flag_clone = abort_flag.clone();

    // Wait for child, with timeout and abort polling
    let wait_result = tokio::select! {
        result = tokio::time::timeout(timeout_duration, child.wait()) => {
            match result {
                Ok(Ok(status)) => Ok(Some(status)),
                Ok(Err(e)) => Err(format!("Failed to wait for command: {}", e)),
                Err(_) => Ok(None), // timeout
            }
        }
        _ = async {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                if abort_flag_clone.load(Ordering::Relaxed) {
                    break;
                }
            }
        } => {
            Ok(None) // abort
        }
    };

    let was_aborted = abort_flag.load(Ordering::Relaxed);
    let _was_timeout = matches!(&wait_result, Ok(None)) && !was_aborted;

    match wait_result {
        Ok(Some(status)) => {
            // Normal completion
            let _ = streaming_handle.await;
            let duration = t0.elapsed();
            tracing::info!("SHELL: /finished in {:.3}s", duration.as_secs_f64());

            let (stdout_str, stderr_str) = {
                let mut collector = output_collector.lock().await;
                (
                    collector.stdout.take_result(),
                    collector.stderr.take_result(),
                )
            };
            let exit_code = status.code().unwrap_or_default();

            send_streaming_update(
                subchat_tx,
                tool_call_id,
                &format!(
                    "✅ Finished (exit code: {}, {:.1}s)",
                    exit_code,
                    duration.as_secs_f64()
                ),
            );

            Ok(ShellStreamResult {
                stdout: stdout_str,
                stderr: stderr_str,
                exit_code,
                duration_secs: duration.as_secs_f64(),
                interrupted: false,
            })
        }
        Ok(None) if was_aborted => {
            // User abort — kill, drain remaining output, return partial results
            kill_and_reap(&mut child).await;
            // Give streaming task time to drain remaining buffered pipe data
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            cancel_token.cancel();
            let _ = streaming_handle.await;

            let duration = t0.elapsed();
            tracing::info!(
                "SHELL: /interrupted by user after {:.3}s",
                duration.as_secs_f64()
            );

            let (stdout_str, stderr_str) = {
                let mut collector = output_collector.lock().await;
                (
                    collector.stdout.take_result(),
                    collector.stderr.take_result(),
                )
            };

            send_streaming_update(
                subchat_tx,
                tool_call_id,
                &format!("⚠️ Interrupted by user ({:.1}s)", duration.as_secs_f64()),
            );

            Ok(ShellStreamResult {
                stdout: stdout_str,
                stderr: stderr_str,
                exit_code: -1,
                duration_secs: duration.as_secs_f64(),
                interrupted: true,
            })
        }
        Ok(None) => {
            // Timeout
            kill_and_reap(&mut child).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            cancel_token.cancel();
            let _ = streaming_handle.await;

            return Err(format!(
                "Command '{}' timed out after {} seconds",
                command, timeout
            ));
        }
        Err(e) => {
            cancel_token.cancel();
            kill_and_reap(&mut child).await;
            return Err(e);
        }
    }
}

async fn parse_args(
    gcx: Arc<GlobalContext>,
    args: &HashMap<String, Value>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<(String, Option<PathBuf>), String> {
    let (command, workdir, _, _, _) =
        parse_args_with_filter(gcx, args, &OutputFilter::default(), execution_scope).await?;
    Ok((command, workdir))
}

async fn parse_args_with_filter(
    gcx: Arc<GlobalContext>,
    args: &HashMap<String, Value>,
    config_filter: &OutputFilter,
    execution_scope: Option<&ExecutionScope>,
) -> Result<
    (
        String,
        Option<PathBuf>,
        Option<OutputFilter>,
        Option<u64>,
        Vec<String>,
    ),
    String,
> {
    let command = match args.get("command") {
        Some(Value::String(s)) => {
            if s.is_empty() {
                return Err("Command is empty".to_string());
            } else {
                s.clone()
            }
        }
        Some(v) => return Err(format!("argument `command` is not a string: {:?}", v)),
        None => return Err("Missing argument `command`".to_string()),
    };

    let raw_workdir = match args.get("workdir") {
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::String(s)) => Some(s.as_str()),
        Some(v) => return Err(format!("argument `workdir` is not a string: {:?}", v)),
        None => None,
    };
    let (workdir, scope_warnings) =
        resolve_shell_workdir(gcx.clone(), raw_workdir, execution_scope).await?;

    let has_filter_override =
        args.get("output_filter").is_some() || args.get("output_limit").is_some();
    let custom_filter = if has_filter_override {
        Some(parse_output_filter_args(args, config_filter))
    } else {
        None
    };

    let timeout_override = args
        .get("timeout")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok());

    Ok((
        command,
        workdir,
        custom_filter,
        timeout_override,
        scope_warnings,
    ))
}

async fn resolve_shell_workdir(
    gcx: Arc<GlobalContext>,
    raw_path: Option<&str>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<(Option<PathBuf>, Vec<String>), String> {
    if let Some(scope) = active_execution_scope(execution_scope) {
        let scoped = scope.resolve_workdir(raw_path).map_err(|e| {
            format!(
                "⚠️ Cannot resolve shell workdir in active worktree '{}': {}",
                scope.effective_root().display(),
                e
            )
        })?;
        let privacy_settings = load_privacy_if_needed(gcx.clone()).await;
        if let Err(e) = check_file_privacy(
            privacy_settings,
            &scoped.path,
            &FilePrivacyLevel::AllowToSendAnywhere,
        ) {
            return Err(format!(
                "⚠️ Cannot use shell workdir '{}' (blocked by privacy: {}). Active worktree root: '{}'",
                scoped.path.display(),
                e,
                scope.effective_root().display()
            ));
        }
        let mut warnings = scoped_path_warnings(&scoped, scope);
        warnings.push(format!(
            "⚠️ Worktree scope: shell cwd/workdir is enforced as '{}', but shell command text is not OS-sandboxed",
            scoped.path.display()
        ));
        return Ok((Some(scoped.path), warnings));
    }

    let Some(raw_path) = raw_path else {
        return Ok((None, Vec::new()));
    };
    let path_str = preprocess_path_for_normalization(raw_path.to_string());
    let path = PathBuf::from(&path_str);

    let workdir = if path.is_absolute() {
        let path = canonicalize_normalized_path(path);
        check_if_its_inside_a_workspace_or_config(gcx.clone(), &path).await?;
        path
    } else {
        let project_dirs = get_project_dirs(gcx.clone()).await;
        let candidates = correct_to_nearest_dir_path(gcx.clone(), &path_str, false, 3).await;
        canonical_path(
            return_one_candidate_or_a_good_error(
                gcx.clone(),
                &path_str,
                &candidates,
                &project_dirs,
                true,
            )
            .await?,
        )
    };
    if !workdir.exists() {
        Err("Workdir doesn't exist".to_string())
    } else {
        Ok((Some(workdir), Vec::new()))
    }
}
