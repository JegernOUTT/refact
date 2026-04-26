use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::process::Stdio;
use async_trait::async_trait;
use serde_json::Value;
use tokio::io::BufReader;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use process_wrap::tokio::*;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::integrations::integr_cmdline::{create_command_from_string, format_output};
use crate::integrations::process_io_utils::{
    blocking_read_until_token_or_timeout, is_someone_listening_on_that_tcp_port,
};
use crate::integrations::sessions::IntegrationSession;
use crate::postprocessing::pp_command_output::{OutputFilter, output_mini_postprocessing};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, MatchConfirmDeny, MatchConfirmDenyResult,
    command_should_be_denied, command_should_be_confirmed_by_user, json_schema_from_params,
};

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
use crate::custom_error::YamlError;

const REALLY_HORRIBLE_ROUNDTRIP: u64 = 3000;

pub struct ToolShellService {
    pub config_path: String,
}

pub struct ShellServiceSession {
    service_name: String,
    command_string: String,
    workdir: String,
    process: Box<dyn TokioChildWrapper>,
    stdout_reader: BufReader<tokio::process::ChildStdout>,
    stderr_reader: BufReader<tokio::process::ChildStderr>,
}

impl IntegrationSession for ShellServiceSession {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn is_expired(&self) -> bool {
        false
    }

    fn try_stop(
        &mut self,
        self_arc: Arc<AMutex<Box<dyn IntegrationSession>>>,
    ) -> Box<dyn Future<Output = String> + Send> {
        Box::new(async move {
            let mut session_locked = self_arc.lock().await;
            let session = session_locked
                .as_any_mut()
                .downcast_mut::<ShellServiceSession>()
                .unwrap();
            stop_service_locked(session).await
        })
    }
}

async fn stop_service_locked(sess: &mut ShellServiceSession) -> String {
    tracing::info!(
        "SERVICE STOP workdir {}:\n{:?}",
        sess.workdir,
        sess.command_string
    );
    let t0 = tokio::time::Instant::now();
    match Box::into_pin(sess.process.kill()).await {
        Ok(_) => {
            format!(
                "Success, it took {:.3}s to stop it.\n\n",
                t0.elapsed().as_secs_f64()
            )
        }
        Err(e) => {
            tracing::warn!(
                "Failed to kill service '{}'. Error: {}. Assuming process died on its own.",
                sess.service_name,
                e
            );
            format!(
                "Failed to kill service. Error: {}.\nAssuming process died on its own, let's continue.\n\n",
                e
            )
        }
    }
}

async fn get_stdout_and_stderr(
    timeout_ms: u64,
    stdout: &mut BufReader<tokio::process::ChildStdout>,
    stderr: &mut BufReader<tokio::process::ChildStderr>,
) -> Result<(String, String), String> {
    let (stdout_out, stderr_out, _) =
        blocking_read_until_token_or_timeout(stdout, stderr, timeout_ms, "").await?;
    Ok((stdout_out, stderr_out))
}

fn parse_service_args(
    args: &HashMap<String, Value>,
) -> Result<(String, String, Option<String>, Option<String>), String> {
    let service_name = match args.get("service_name") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        _ => return Err("Missing required argument `service_name`".to_string()),
    };

    let action = match args.get("action") {
        Some(Value::String(s)) => s.trim().to_lowercase(),
        _ => return Err("Missing required argument `action`".to_string()),
    };

    if !["start", "stop", "status", "logs", "restart"].contains(&action.as_str()) {
        return Err(format!(
            "Invalid action '{}'. Must be one of: start, stop, status, logs, restart",
            action
        ));
    }

    let command = if action == "start" || action == "restart" {
        match args.get("command") {
            Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Some(Value::String(_)) => {
                return Err("Argument `command` cannot be empty for start/restart".to_string())
            }
            None => {
                return Err(
                    "Missing required argument `command` for start/restart action".to_string(),
                )
            }
            _ => return Err("Argument `command` must be a string".to_string()),
        }
    } else {
        None
    };

    let workdir = args
        .get("workdir")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Ok((service_name, action, command, workdir))
}

fn parse_startup_wait_params(args: &HashMap<String, Value>) -> (u64, Option<u16>, String) {
    let startup_wait = args
        .get("startup_wait")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(10);

    let startup_wait_port = args
        .get("startup_wait_port")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u16>().ok());

    let startup_wait_keyword = args
        .get("startup_wait_keyword")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    (startup_wait, startup_wait_port, startup_wait_keyword)
}

fn parse_output_params(args: &HashMap<String, Value>) -> OutputFilter {
    let output_filter_pattern = args
        .get("output_filter")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let output_limit = args
        .get("output_limit")
        .and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        })
        .unwrap_or_default();

    let is_unlimited =
        output_limit.eq_ignore_ascii_case("all") || output_limit.eq_ignore_ascii_case("full");

    let limit_lines = if is_unlimited {
        usize::MAX
    } else {
        output_limit.parse::<usize>().unwrap_or(40)
    };

    let skip_filtering = is_unlimited && output_filter_pattern.is_none();

    OutputFilter {
        limit_lines,
        limit_chars: if is_unlimited {
            usize::MAX
        } else {
            limit_lines.saturating_mul(200)
        },
        valuable_top_or_bottom: "top".to_string(),
        grep: output_filter_pattern.unwrap_or_default(),
        grep_context_lines: 3,
        remove_from_output: "".to_string(),
        limit_tokens: if is_unlimited {
            None
        } else {
            Some(limit_lines.saturating_mul(50))
        },
        skip: skip_filtering,
    }
}

async fn resolve_workdir(
    gcx: Arc<ARwLock<GlobalContext>>,
    workdir_str: Option<String>,
) -> Result<String, String> {
    if let Some(wd) = workdir_str {
        let path = PathBuf::from(&wd);
        if path.is_absolute() {
            if path.exists() {
                Ok(wd)
            } else {
                Err(format!("Workdir '{}' does not exist", wd))
            }
        } else {
            let project_dirs = crate::files_correction::get_project_dirs(gcx).await;
            if let Some(first_dir) = project_dirs.first() {
                let resolved = first_dir.join(&wd);
                if resolved.exists() {
                    Ok(resolved.to_string_lossy().to_string())
                } else {
                    Err(format!("Workdir '{}' does not exist", resolved.display()))
                }
            } else {
                Err("No project directory found".to_string())
            }
        }
    } else {
        let project_dirs = crate::files_correction::get_project_dirs(gcx).await;
        if let Some(first_dir) = project_dirs.first() {
            Ok(first_dir.to_string_lossy().to_string())
        } else {
            Ok(String::new())
        }
    }
}

async fn execute_start_action(
    gcx: Arc<ARwLock<GlobalContext>>,
    service_name: &str,
    command: &str,
    workdir: &str,
    startup_wait: u64,
    startup_wait_port: Option<u16>,
    startup_wait_keyword: &str,
    output_filter: &OutputFilter,
    env_variables: &HashMap<String, String>,
) -> Result<String, String> {
    let session_key = format!("builtin_shell_service_{}", service_name);

    let session_exists = gcx
        .read()
        .await
        .integration_sessions
        .contains_key(&session_key);
    if session_exists {
        return Err(format!(
            "Service '{}' is already running. Use 'stop' or 'restart' action first.",
            service_name
        ));
    }

    let mut port_already_open = false;
    if let Some(wait_port) = startup_wait_port {
        port_already_open = is_someone_listening_on_that_tcp_port(
            wait_port,
            tokio::time::Duration::from_millis(REALLY_HORRIBLE_ROUNDTRIP),
        )
        .await;
    }

    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    let mut cmd =
        create_command_from_string(command, &workdir.to_string(), env_variables, project_dirs)?;

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut command_wrap = TokioCommandWrap::from(cmd);
    #[cfg(unix)]
    command_wrap.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command_wrap.wrap(JobObject);

    let mut process = command_wrap
        .spawn()
        .map_err(|e| format!("Failed to spawn process: {}", e))?;

    let mut stdout_reader = BufReader::new(process.stdout().take().ok_or("Failed to open stdout")?);
    let mut stderr_reader = BufReader::new(process.stderr().take().ok_or("Failed to open stderr")?);

    let t0 = tokio::time::Instant::now();
    let mut accumulated_stdout = String::new();
    let mut accumulated_stderr = String::new();
    let mut actions_log = format!(
        "Starting service '{}' with command:\n{}\n\n",
        service_name, command
    );

    loop {
        if t0.elapsed() >= tokio::time::Duration::from_secs(startup_wait) {
            actions_log.push_str(&format!(
                "Timeout {:.2}s reached while waiting for service to start.\n\n",
                t0.elapsed().as_secs_f64()
            ));
            break;
        }

        let (stdout_out, stderr_out) =
            get_stdout_and_stderr(100, &mut stdout_reader, &mut stderr_reader).await?;
        accumulated_stdout.push_str(&stdout_out);
        accumulated_stderr.push_str(&stderr_out);

        if !startup_wait_keyword.is_empty() {
            if accumulated_stdout.contains(startup_wait_keyword)
                || accumulated_stderr.contains(startup_wait_keyword)
            {
                actions_log.push_str(&format!(
                    "Startup keyword '{}' found in output, success!\n\n",
                    startup_wait_keyword
                ));
                break;
            }
        }

        let exit_status = process.try_wait().map_err(|e| e.to_string())?;
        if let Some(status) = exit_status {
            let exit_code = status.code().unwrap_or(-1);
            actions_log.push_str(&format!(
                "Service process exited prematurely with exit code: {}\nService did not start.\n\n",
                exit_code
            ));
            let filtered_stdout = output_mini_postprocessing(output_filter, &accumulated_stdout);
            let filtered_stderr = output_mini_postprocessing(output_filter, &accumulated_stderr);
            actions_log.push_str(&format_output(&filtered_stdout, &filtered_stderr));
            return Err(actions_log);
        }

        if let Some(wait_port) = startup_wait_port {
            let port_busy = is_someone_listening_on_that_tcp_port(
                wait_port,
                tokio::time::Duration::from_millis(REALLY_HORRIBLE_ROUNDTRIP),
            )
            .await;

            if port_busy && !port_already_open {
                actions_log.push_str(&format!("Port {} is now busy, success!\n\n", wait_port));
                break;
            } else if !port_busy && port_already_open {
                port_already_open = false;
                actions_log.push_str(&format!("Port {} is now free\n", wait_port));
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    let filtered_stdout = output_mini_postprocessing(output_filter, &accumulated_stdout);
    let filtered_stderr = output_mini_postprocessing(output_filter, &accumulated_stderr);
    actions_log.push_str(&format_output(&filtered_stdout, &filtered_stderr));

    let session: Box<dyn IntegrationSession> = Box::new(ShellServiceSession {
        service_name: service_name.to_string(),
        command_string: command.to_string(),
        workdir: workdir.to_string(),
        process,
        stdout_reader,
        stderr_reader,
    });

    gcx.write()
        .await
        .integration_sessions
        .insert(session_key, Arc::new(AMutex::new(session)));

    Ok(actions_log)
}

async fn execute_stop_action(
    gcx: Arc<ARwLock<GlobalContext>>,
    service_name: &str,
) -> Result<String, String> {
    let session_key = format!("builtin_shell_service_{}", service_name);

    let session_arc = gcx
        .read()
        .await
        .integration_sessions
        .get(&session_key)
        .cloned();

    if let Some(session) = session_arc {
        let stop_msg = {
            let mut session_locked = session.lock().await;
            let sess = session_locked
                .as_any_mut()
                .downcast_mut::<ShellServiceSession>()
                .unwrap();
            stop_service_locked(sess).await
        };

        gcx.write().await.integration_sessions.remove(&session_key);
        Ok(format!("Service '{}' stopped.\n{}", service_name, stop_msg))
    } else {
        Err(format!("Service '{}' is not running", service_name))
    }
}

async fn execute_status_action(
    gcx: Arc<ARwLock<GlobalContext>>,
    service_name: &str,
    output_filter: &OutputFilter,
) -> Result<String, String> {
    let session_key = format!("builtin_shell_service_{}", service_name);

    let session_arc = gcx
        .read()
        .await
        .integration_sessions
        .get(&session_key)
        .cloned();

    if let Some(session) = session_arc {
        let mut session_locked = session.lock().await;
        let sess = session_locked
            .as_any_mut()
            .downcast_mut::<ShellServiceSession>()
            .unwrap();

        let exit_status = sess.process.try_wait().map_err(|e| e.to_string())?;

        if let Some(status) = exit_status {
            let exit_code = status.code().unwrap_or(-1);
            let (stdout_out, stderr_out) =
                get_stdout_and_stderr(100, &mut sess.stdout_reader, &mut sess.stderr_reader)
                    .await?;
            let filtered_stdout = output_mini_postprocessing(output_filter, &stdout_out);
            let filtered_stderr = output_mini_postprocessing(output_filter, &stderr_out);

            drop(session_locked);
            gcx.write().await.integration_sessions.remove(&session_key);

            let mut result = format!(
                "Service '{}' has exited with code {}.\n\n",
                service_name, exit_code
            );
            result.push_str(&format_output(&filtered_stdout, &filtered_stderr));
            Ok(result)
        } else {
            let (stdout_out, stderr_out) =
                get_stdout_and_stderr(100, &mut sess.stdout_reader, &mut sess.stderr_reader)
                    .await?;
            let filtered_stdout = output_mini_postprocessing(output_filter, &stdout_out);
            let filtered_stderr = output_mini_postprocessing(output_filter, &stderr_out);

            let mut result = format!(
                "Service '{}' is running.\nworkdir: {}\ncommand: {}\n\n",
                service_name, sess.workdir, sess.command_string
            );
            result.push_str("Recent output:\n");
            result.push_str(&format_output(&filtered_stdout, &filtered_stderr));
            Ok(result)
        }
    } else {
        Err(format!("Service '{}' is not running", service_name))
    }
}

async fn execute_logs_action(
    gcx: Arc<ARwLock<GlobalContext>>,
    service_name: &str,
    output_filter: &OutputFilter,
) -> Result<String, String> {
    execute_status_action(gcx, service_name, output_filter).await
}

async fn execute_restart_action(
    gcx: Arc<ARwLock<GlobalContext>>,
    service_name: &str,
    command: &str,
    workdir: &str,
    startup_wait: u64,
    startup_wait_port: Option<u16>,
    startup_wait_keyword: &str,
    output_filter: &OutputFilter,
    env_variables: &HashMap<String, String>,
) -> Result<String, String> {
    let mut result = String::new();

    let session_key = format!("builtin_shell_service_{}", service_name);
    let session_exists = gcx
        .read()
        .await
        .integration_sessions
        .contains_key(&session_key);

    if session_exists {
        match execute_stop_action(gcx.clone(), service_name).await {
            Ok(stop_msg) => result.push_str(&stop_msg),
            Err(e) => result.push_str(&format!("Warning during stop: {}\n", e)),
        }
    }

    let start_result = execute_start_action(
        gcx,
        service_name,
        command,
        workdir,
        startup_wait,
        startup_wait_port,
        startup_wait_keyword,
        output_filter,
        env_variables,
    )
    .await?;

    result.push_str(&start_result);
    Ok(result)
}

#[async_trait]
impl Tool for ToolShellService {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "shell_service".to_string(),
            display_name: "Shell Service".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Manage background services (start/stop/status/logs/restart). Use this for long-running processes like web servers, databases, or any command that runs until Ctrl+C. For one-time commands, use the shell tool instead.".to_string(),
            input_schema: json_schema_from_params(&[("service_name", "string", "Unique service identifier (e.g., 'api', 'postgres', 'worker')"), ("action", "string", "Action to perform: 'start', 'stop', 'status', 'logs', or 'restart'"), ("command", "string", "Shell command to run (required for start/restart, e.g., 'uvicorn app:app --port 8000')"), ("workdir", "string", "Working directory (optional, can be relative or absolute)"), ("startup_wait", "string", "Max seconds to wait for service to start (default: 10)"), ("startup_wait_port", "string", "TCP port number to wait for (e.g., '8000')"), ("startup_wait_keyword", "string", "Text to wait for in stdout/stderr (e.g., 'Ready')"), ("output_filter", "string", "Optional regex pattern to filter logs"), ("output_limit", "string", "Max lines to show (default: 40, use 'all' for unlimited)")], &["service_name", "action"]),
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
        let (service_name, action, command_opt, workdir_opt) = parse_service_args(args)?;
        let (startup_wait, startup_wait_port, startup_wait_keyword) =
            parse_startup_wait_params(args);
        let output_filter = parse_output_params(args);

        let gcx = ccx.lock().await.global_context.clone();
        let workdir = resolve_workdir(gcx.clone(), workdir_opt).await?;

        let mut error_log = Vec::<YamlError>::new();
        let env_variables =
            crate::integrations::setting_up_integrations::get_vars_for_replacements(
                gcx.clone(),
                &mut error_log,
            )
            .await;

        let result = match action.as_str() {
            "start" => {
                let command = command_opt.ok_or("Command is required for start action")?;
                execute_start_action(
                    gcx,
                    &service_name,
                    &command,
                    &workdir,
                    startup_wait,
                    startup_wait_port,
                    &startup_wait_keyword,
                    &output_filter,
                    &env_variables,
                )
                .await?
            }
            "stop" => execute_stop_action(gcx, &service_name).await?,
            "status" => execute_status_action(gcx, &service_name, &output_filter).await?,
            "logs" => execute_logs_action(gcx, &service_name, &output_filter).await?,
            "restart" => {
                let command = command_opt.ok_or("Command is required for restart action")?;
                execute_restart_action(
                    gcx,
                    &service_name,
                    &command,
                    &workdir,
                    startup_wait,
                    startup_wait_port,
                    &startup_wait_keyword,
                    &output_filter,
                    &env_variables,
                )
                .await?
            }
            _ => return Err(format!("Unknown action: {}", action)),
        };

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            })],
        ))
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let (service_name, action, command_opt, _) = parse_service_args(args)?;

        if action == "start" || action == "restart" {
            if let Some(command) = command_opt {
                return Ok(command);
            }
        }

        Ok(format!("shell_service {} {}", action, service_name))
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: ASK_USER_DEFAULT.iter().map(|s| s.to_string()).collect(),
            deny: DENY_DEFAULT.iter().map(|s| s.to_string()).collect(),
        })
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let command_to_match = self
            .command_to_match_against_confirm_deny(ccx.clone(), args)
            .await
            .map_err(|e| format!("Error getting tool command to match: {}", e))?;

        if command_to_match.is_empty() {
            return Ok(MatchConfirmDeny {
                result: MatchConfirmDenyResult::PASS,
                command: command_to_match,
                rule: "".to_string(),
            });
        }

        if let Some(rules) = self.confirm_deny_rules() {
            let (is_denied, deny_rule) = command_should_be_denied(&command_to_match, &rules.deny);
            if is_denied {
                return Ok(MatchConfirmDeny {
                    result: MatchConfirmDenyResult::DENY,
                    command: command_to_match,
                    rule: deny_rule,
                });
            }

            let (needs_confirmation, confirmation_rule) =
                command_should_be_confirmed_by_user(&command_to_match, &rules.ask_user);
            if needs_confirmation {
                return Ok(MatchConfirmDeny {
                    result: MatchConfirmDenyResult::CONFIRMATION,
                    command: command_to_match,
                    rule: confirmation_rule,
                });
            }
        }

        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::PASS,
            command: command_to_match,
            rule: "".to_string(),
        })
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
