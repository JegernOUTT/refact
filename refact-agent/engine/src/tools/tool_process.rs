use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::exec::ExecRegistry;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::exec::{
    generate_short_description, sanitize_short_description, ExecMode, ExecOutputChunk,
    ExecOutputStream, ExecOwnerMeta, ExecProcessFilter, ExecProcessId, ExecProcessSnapshot,
    ExecReadResult, ExecReadinessProbe, ExecServiceLookup, ExecSpawnRequest, ExecStatus,
};
use crate::exec::types::normalize_workspace_path;
use crate::files_correction::{
    canonical_path, canonicalize_normalized_path, check_if_its_inside_a_workspace_or_config,
    correct_to_nearest_dir_path, get_active_project_path, get_project_dirs,
    preprocess_path_for_normalization,
};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::postprocessing::pp_command_output::{
    output_mini_postprocessing, parse_output_filter_args, OutputFilter,
};
use crate::privacy::{check_file_privacy, load_privacy_if_needed, FilePrivacyLevel};
use crate::tools::file_edit::auxiliary::{active_execution_scope, scoped_path_warnings};
use crate::tools::tools_description::{
    json_schema_from_params, Tool, ToolDesc, ToolSource, ToolSourceType,
};
use crate::worktrees::scope::ExecutionScope;

const PROCESS_TRANSCRIPT_MAX_BYTES: usize = 2 * 1024 * 1024;
const TTY_DESCRIPTION: &str = "If true, run the command attached to a pseudo-terminal (PTY). Enables interactive stdin via process_write_stdin and merges stdout+stderr into a single combined stream. Defeats some pipe-only output buffering. Defaults to false.";
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

pub struct ToolProcessStart {
    pub config_path: String,
}

pub struct ToolProcessList {
    pub config_path: String,
}

pub struct ToolProcessRead {
    pub config_path: String,
}

pub struct ToolProcessKill {
    pub config_path: String,
}

pub struct ToolProcessWait {
    pub config_path: String,
}

pub struct ToolShellServiceAlias {
    pub config_path: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProcessListStatus {
    Running,
    Completed,
    All,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProcessListScope {
    Chat,
    Workspace,
    All,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProcessStreamSelection {
    Stdout,
    Stderr,
    Combined,
    All,
}

struct ProcessStartArgs {
    command: String,
    workdir: Option<PathBuf>,
    mode: ExecMode,
    service_name: Option<String>,
    startup_wait: Option<Duration>,
    readiness: Option<ExecReadinessProbe>,
    description: String,
    tty: Option<bool>,
    scope_warnings: Vec<String>,
}

struct ShellServiceAliasArgs {
    service_name: String,
    action: String,
    command: Option<String>,
    workdir: Option<String>,
    startup_wait_ms: Option<u64>,
    startup_wait_port: Option<u16>,
    startup_wait_keyword: Option<String>,
    output_filter: Option<String>,
    output_limit: Option<String>,
}

#[async_trait]
impl Tool for ToolProcessStart {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, exec_registry, execution_scope, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.runtime.exec_registry.clone(),
                ccx_lock.execution_scope.clone(),
                ccx_lock.chat_id.clone(),
            )
        };
        let parsed = parse_start_args(gcx.clone(), args, execution_scope.as_ref()).await?;
        let mut error_log = Vec::new();
        let env_variables =
            crate::integrations::setting_up_integrations::get_vars_for_replacements(
                gcx.clone(),
                &mut error_log,
            )
            .await;
        let workspace = match parsed.workdir.clone() {
            Some(workdir) => workspace_containing_path(gcx.clone(), &workdir)
                .await
                .or_else(|| Some(normalize_workspace_path(&workdir))),
            None => process_workspace(gcx.clone(), execution_scope.as_ref()).await,
        };
        if parsed.mode == ExecMode::Service && parsed.service_name.is_none() {
            return Err("service mode requires service_name".to_string());
        }
        if let Some(service_name) = parsed.service_name.as_ref() {
            let mut lookup = ExecServiceLookup::new(service_name.clone());
            if !chat_id.is_empty() {
                lookup = lookup.with_chat_id(chat_id.clone());
            }
            if let Some(workspace) = workspace.clone() {
                lookup = lookup.with_workspace(workspace);
            }
            match exec_registry.find_service(lookup).await {
                Some(existing) if !existing.status.is_terminal() => {
                    return Err(format!(
                        "Service '{}' is already running as {}. Use process_kill first.",
                        service_name, existing.meta.process_id
                    ));
                }
                Some(_) | None => {}
            }
        }
        let short_description = sanitize_short_description(&parsed.description);
        let tty = parsed.tty.unwrap_or(false);
        let owner = ExecOwnerMeta {
            chat_id: Some(chat_id),
            tool_call_id: Some(tool_call_id.clone()),
            service_name: parsed.service_name.clone(),
            workspace,
        };
        let mut request = ExecSpawnRequest::new(parsed.mode.clone(), parsed.command.clone())
            .with_env_map(env_variables)
            .with_owner(owner)
            .with_transcript_limit(PROCESS_TRANSCRIPT_MAX_BYTES)
            .with_short_description(short_description)
            .with_tty(tty);
        if let Some(cwd) = parsed.workdir.clone() {
            request = request.with_cwd(cwd);
        }
        if let Some(startup_wait) = parsed.startup_wait {
            request = request.with_startup_wait(startup_wait);
        }
        if let Some(readiness) = parsed.readiness.clone() {
            request = request.with_readiness(readiness);
        }
        let result = exec_registry.spawn(request).await?;
        let read = exec_registry
            .read(&result.snapshot.meta.process_id, 0, None)
            .await;
        let mut content = format_process_snapshot("Process started", &result.snapshot);
        if !parsed.scope_warnings.is_empty() {
            content.push_str(&format!("\n{}\n", parsed.scope_warnings.join("\n")));
        }
        content.push_str(&format_read_sections(
            &read,
            ProcessStreamSelection::All,
            &OutputFilter::no_limits(),
        ));
        Ok(tool_result(
            tool_call_id,
            content,
            Some(exec_extra(&result.snapshot, Some(&read), None, Some(tty))),
            tool_failed_for_status(&result.snapshot.status),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "process_start".to_string(),
            display_name: "Process Start".to_string(),
            source: source(&self.config_path),
            experimental: false,
            allow_parallel: false,
            description: "Start a runtime-owned background or service process and return its process ID, initial status, output cursor, and metadata. Set tty=true only when a command needs PTY behavior: it enables interactive stdin and reduces pipe buffering, but merges stdout and stderr into one combined stream.".to_string(),
            input_schema: process_start_input_schema(),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let (gcx, execution_scope) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.app.gcx.clone(), ccx_lock.execution_scope.clone())
        };
        Ok(parse_start_args(gcx, args, execution_scope.as_ref())
            .await?
            .command)
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

#[async_trait]
impl Tool for ToolProcessList {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, exec_registry, execution_scope, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.runtime.exec_registry.clone(),
                ccx_lock.execution_scope.clone(),
                ccx_lock.chat_id.clone(),
            )
        };
        let status = parse_list_status(args)?;
        let scope = parse_list_scope(args)?;
        let workspace = process_workspace(gcx, execution_scope.as_ref()).await;
        let snapshots = exec_registry
            .list(ExecProcessFilter::default())
            .await
            .into_iter()
            .filter(|snapshot| matches_list_status(snapshot, status))
            .filter(|snapshot| matches_list_scope(snapshot, scope, &chat_id, workspace.as_ref()))
            .collect::<Vec<_>>();
        let mut content = format!(
            "Processes (status: {}, scope: {})\ncount: {}\n",
            list_status_label(status),
            list_scope_label(scope),
            snapshots.len()
        );
        for snapshot in &snapshots {
            content.push_str("\n");
            content.push_str(&format_process_snapshot("Process", snapshot));
        }
        let mut extra = serde_json::Map::new();
        extra.insert(
            "exec".to_string(),
            json!({
                "count": snapshots.len(),
                "status_filter": list_status_label(status),
                "scope_filter": list_scope_label(scope),
                "processes": snapshots.iter().map(process_value).collect::<Vec<_>>(),
            }),
        );
        Ok(tool_result(tool_call_id, content, Some(extra), None))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "process_list".to_string(),
            display_name: "Process List".to_string(),
            source: source(&self.config_path),
            experimental: false,
            allow_parallel: true,
            description: "List runtime-owned processes by status and owner scope.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    (
                        "status",
                        "string",
                        "Optional status filter: running, completed, or all. Default: running.",
                    ),
                    (
                        "scope",
                        "string",
                        "Optional scope filter: chat, workspace, or all. Default: chat.",
                    ),
                ],
                &[],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

#[async_trait]
impl Tool for ToolProcessRead {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let exec_registry = {
            let ccx_lock = ccx.lock().await;
            ccx_lock.app.runtime.exec_registry.clone()
        };
        let process_id = parse_process_id(args)?;
        let snapshot = require_process(&exec_registry, &process_id).await?;
        let since_seq = parse_optional_u64(args, "since_seq")?.unwrap_or(0);
        let stream = parse_stream_selection(args)?;
        let output_filter = parse_output_filter_args(args, &OutputFilter::default());
        let read = exec_registry.read(&process_id, since_seq, None).await;
        let mut content = format_process_snapshot("Process output", &snapshot);
        content.push_str(&format!(
            "\nsince_seq: {}\nnext_seq: {}\nlatest_seq: {}\n",
            read.since_seq, read.next_seq, read.latest_seq
        ));
        content.push_str(&format_read_sections(&read, stream, &output_filter));
        Ok(tool_result(
            tool_call_id,
            content,
            Some(exec_extra(
                &snapshot,
                Some(&read),
                Some(stream_label(stream)),
                None,
            )),
            None,
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "process_read".to_string(),
            display_name: "Process Read".to_string(),
            source: source(&self.config_path),
            experimental: false,
            allow_parallel: true,
            description: "Read buffered output from a runtime-owned process using a sequence cursor and optional stream/output filters.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("process_id", "string", "Runtime-owned process ID returned by process_start."),
                    ("since_seq", "integer", "Optional output sequence cursor. Default: 0."),
                    ("stream", "string", "Optional stream: stdout, stderr, combined, or all. Default: combined."),
                    ("output_filter", "string", "Optional regex filter for output lines."),
                    ("output_limit", "string", "Optional max lines to show, or all/full for unlimited output."),
                ],
                &["process_id"],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

#[async_trait]
impl Tool for ToolProcessKill {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let exec_registry = {
            let ccx_lock = ccx.lock().await;
            ccx_lock.app.runtime.exec_registry.clone()
        };
        let process_id = parse_process_id(args)?;
        require_process(&exec_registry, &process_id).await?;
        let snapshot = exec_registry.kill(&process_id).await?;
        let content = format_process_snapshot("Process killed", &snapshot);
        Ok(tool_result(
            tool_call_id,
            content,
            Some(exec_extra(&snapshot, None, None, None)),
            tool_failed_for_status(&snapshot.status),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "process_kill".to_string(),
            display_name: "Process Kill".to_string(),
            source: source(&self.config_path),
            experimental: false,
            allow_parallel: false,
            description: "Kill a runtime-owned process by process ID without routing through shell command confirmation.".to_string(),
            input_schema: json_schema_from_params(
                &[("process_id", "string", "Runtime-owned process ID returned by process_start.")],
                &["process_id"],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

#[async_trait]
impl Tool for ToolProcessWait {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let exec_registry = {
            let ccx_lock = ccx.lock().await;
            ccx_lock.app.runtime.exec_registry.clone()
        };
        let process_id = parse_process_id(args)?;
        require_process(&exec_registry, &process_id).await?;
        let timeout_ms = parse_optional_u64(args, "timeout_ms")?;
        let (snapshot, timed_out) = if let Some(timeout_ms) = timeout_ms {
            match tokio::time::timeout(
                Duration::from_millis(timeout_ms),
                exec_registry.wait(&process_id),
            )
            .await
            {
                Ok(result) => (result?, false),
                Err(_) => (require_process(&exec_registry, &process_id).await?, true),
            }
        } else {
            (exec_registry.wait(&process_id).await?, false)
        };
        let read = exec_registry.read(&process_id, 0, None).await;
        let title = if timed_out {
            "Process wait timed out"
        } else {
            "Process wait completed"
        };
        let mut content = format_process_snapshot(title, &snapshot);
        content.push_str(&format!(
            "\nnext_seq: {}\nlatest_seq: {}\n",
            read.next_seq, read.latest_seq
        ));
        content.push_str(&format_read_sections(
            &read,
            ProcessStreamSelection::All,
            &OutputFilter::no_limits(),
        ));
        Ok(tool_result(
            tool_call_id,
            content,
            Some(exec_extra(&snapshot, Some(&read), None, None)),
            tool_failed_for_status(&snapshot.status),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "process_wait".to_string(),
            display_name: "Process Wait".to_string(),
            source: source(&self.config_path),
            experimental: false,
            allow_parallel: true,
            description: "Wait for a runtime-owned process to complete, or return the current snapshot when timeout_ms expires.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("process_id", "string", "Runtime-owned process ID returned by process_start."),
                    ("timeout_ms", "integer", "Optional timeout in milliseconds."),
                ],
                &["process_id"],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

#[async_trait]
impl Tool for ToolShellServiceAlias {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let parsed = parse_shell_service_alias_args(args)?;
        let (gcx, execution_scope, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.execution_scope.clone(),
                ccx_lock.chat_id.clone(),
            )
        };
        let workspace = process_workspace(gcx, execution_scope.as_ref()).await;
        match parsed.action.as_str() {
            "start" => {
                let mut start = ToolProcessStart {
                    config_path: self.config_path.clone(),
                };
                start
                    .tool_execute(
                        ccx,
                        tool_call_id,
                        &process_start_args_from_shell_service(&parsed)?,
                    )
                    .await
            }
            "stop" => {
                let exec_registry = exec_registry_from_ccx(ccx.clone()).await;
                let process_id = service_process_id(
                    &exec_registry,
                    &parsed.service_name,
                    &chat_id,
                    workspace.as_ref(),
                )
                .await?;
                let mut kill = ToolProcessKill {
                    config_path: self.config_path.clone(),
                };
                kill.tool_execute(
                    ccx,
                    tool_call_id,
                    &make_args_map(vec![("process_id", json!(process_id.as_str()))]),
                )
                .await
            }
            "status" | "logs" => {
                let exec_registry = exec_registry_from_ccx(ccx.clone()).await;
                let process_id = service_process_id(
                    &exec_registry,
                    &parsed.service_name,
                    &chat_id,
                    workspace.as_ref(),
                )
                .await?;
                let mut read = ToolProcessRead {
                    config_path: self.config_path.clone(),
                };
                let mut read_args = make_args_map(vec![
                    ("process_id", json!(process_id.as_str())),
                    ("stream", json!("all")),
                ]);
                if let Some(output_filter) = parsed.output_filter {
                    read_args.insert("output_filter".to_string(), json!(output_filter));
                }
                if let Some(output_limit) = parsed.output_limit {
                    read_args.insert("output_limit".to_string(), json!(output_limit));
                }
                read.tool_execute(ccx, tool_call_id, &read_args).await
            }
            "restart" => {
                let exec_registry = exec_registry_from_ccx(ccx.clone()).await;
                if let Ok(process_id) = service_process_id(
                    &exec_registry,
                    &parsed.service_name,
                    &chat_id,
                    workspace.as_ref(),
                )
                .await
                {
                    let mut kill = ToolProcessKill {
                        config_path: self.config_path.clone(),
                    };
                    let _ = kill
                        .tool_execute(
                            ccx.clone(),
                            tool_call_id,
                            &make_args_map(vec![("process_id", json!(process_id.as_str()))]),
                        )
                        .await;
                }
                let mut start = ToolProcessStart {
                    config_path: self.config_path.clone(),
                };
                start
                    .tool_execute(
                        ccx,
                        tool_call_id,
                        &process_start_args_from_shell_service(&parsed)?,
                    )
                    .await
            }
            _ => Err(format!("Unknown action: {}", parsed.action)),
        }
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "shell_service".to_string(),
            display_name: "Shell Service".to_string(),
            source: source(&self.config_path),
            experimental: false,
            allow_parallel: false,
            description: "Legacy alias for runtime process service management. Prefer process_start with mode=service plus process_list/read/kill/wait.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("service_name", "string", "Unique service identifier."),
                    ("action", "string", "Action: start, stop, status, logs, or restart."),
                    ("command", "string", "Command required for start/restart."),
                    ("workdir", "string", "Optional working directory."),
                    ("startup_wait", "string", "Max seconds to wait for startup readiness. Default: 10."),
                    ("startup_wait_port", "string", "Optional TCP port readiness check."),
                    ("startup_wait_keyword", "string", "Optional stdout/stderr readiness keyword."),
                    ("output_filter", "string", "Optional regex pattern to filter logs."),
                    ("output_limit", "string", "Max lines to show, or all/full."),
                ],
                &["service_name", "action"],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let parsed = parse_shell_service_alias_args(args)?;
        if matches!(parsed.action.as_str(), "start" | "restart") {
            return parsed
                .command
                .ok_or_else(|| "Command is required for start/restart action".to_string());
        }
        Ok(format!(
            "shell_service {} {}",
            parsed.action, parsed.service_name
        ))
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: ASK_USER_DEFAULT.iter().map(|s| s.to_string()).collect(),
            deny: DENY_DEFAULT.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

async fn exec_registry_from_ccx(ccx: Arc<AMutex<AtCommandsContext>>) -> Arc<ExecRegistry> {
    let ccx_lock = ccx.lock().await;
    ccx_lock.app.runtime.exec_registry.clone()
}

async fn service_process_id(
    exec_registry: &ExecRegistry,
    service_name: &str,
    chat_id: &str,
    workspace: Option<&PathBuf>,
) -> Result<ExecProcessId, String> {
    let mut lookup = ExecServiceLookup::new(service_name.to_string());
    if !chat_id.is_empty() {
        lookup = lookup.with_chat_id(chat_id.to_string());
    }
    if let Some(workspace) = workspace.cloned() {
        lookup = lookup.with_workspace(workspace);
    }
    exec_registry
        .find_service(lookup)
        .await
        .filter(|snapshot| !snapshot.status.is_terminal())
        .map(|snapshot| snapshot.meta.process_id)
        .ok_or_else(|| format!("Service '{service_name}' is not running"))
}

fn process_start_input_schema() -> Value {
    let mut schema = json_schema_from_params(
        &[
            ("command", "string", "Command to start."),
            ("description", "string", "Required. A single concise active-voice sentence describing what the command does. Short for simple commands: 'Start development server'. More descriptive for piped/obscure commands: 'Build and tail log output in background'. Never use words like 'complex' or 'risk'."),
            ("workdir", "string", "Optional working directory."),
            ("mode", "string", "Optional mode: background or service. Default: background."),
            ("service_name", "string", "Optional service name stored in process metadata."),
            ("startup_wait_ms", "integer", "Optional milliseconds to wait before returning the initial snapshot."),
            ("startup_wait_port", "integer", "Optional readiness port stored with startup metadata."),
            ("startup_wait_keyword", "string", "Optional readiness keyword stored with startup metadata."),
        ],
        &["command", "description"],
    );
    schema["properties"]["tty"] = json!({
        "type": "boolean",
        "default": false,
        "description": TTY_DESCRIPTION,
    });
    schema
}

fn source(config_path: &str) -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: config_path.to_string(),
    }
}

fn tool_result(
    tool_call_id: &String,
    content: String,
    extra: Option<serde_json::Map<String, Value>>,
    tool_failed: Option<bool>,
) -> (bool, Vec<ContextEnum>) {
    let mut message = ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_calls: None,
        tool_call_id: tool_call_id.clone(),
        tool_failed,
        output_filter: Some(OutputFilter::no_limits()),
        ..Default::default()
    };
    if let Some(extra) = extra {
        message.extra = extra;
    }
    (false, vec![ContextEnum::ChatMessage(message)])
}

fn parse_required_string(args: &HashMap<String, Value>, name: &str) -> Result<String, String> {
    match args.get(name) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.trim().to_string()),
        Some(Value::String(_)) => Err(format!("Argument `{name}` cannot be empty")),
        Some(v) => Err(format!("argument `{name}` is not a string: {v:?}")),
        None => Err(format!("Missing argument `{name}`")),
    }
}

fn make_args_map(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn parse_optional_string(
    args: &HashMap<String, Value>,
    name: &str,
) -> Result<Option<String>, String> {
    match args.get(name) {
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(v) => Err(format!("argument `{name}` is not a string: {v:?}")),
        None => Ok(None),
    }
}

fn parse_optional_u64(args: &HashMap<String, Value>, name: &str) -> Result<Option<u64>, String> {
    match args.get(name) {
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => s
            .trim()
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("argument `{name}` must be an integer")),
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| format!("argument `{name}` must be a non-negative integer"))
            .map(Some),
        Some(v) => Err(format!(
            "argument `{name}` is not a string or number: {v:?}"
        )),
        None => Ok(None),
    }
}

fn parse_optional_u16(args: &HashMap<String, Value>, name: &str) -> Result<Option<u16>, String> {
    match parse_optional_u64(args, name)? {
        Some(value) => u16::try_from(value)
            .map(Some)
            .map_err(|_| format!("argument `{name}` must fit in u16")),
        None => Ok(None),
    }
}

fn parse_optional_bool(args: &HashMap<String, Value>, name: &str) -> Result<Option<bool>, String> {
    match args.get(name) {
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(v) => Err(format!("argument `{name}` is not a boolean: {v:?}")),
        None => Ok(Some(false)),
    }
}

fn parse_process_id(args: &HashMap<String, Value>) -> Result<ExecProcessId, String> {
    let process_id = parse_required_string(args, "process_id")?;
    if !process_id.starts_with("exec_") {
        return Err("process_id must be a runtime-owned exec_* ID".to_string());
    }
    Ok(ExecProcessId(process_id))
}

fn parse_start_mode(args: &HashMap<String, Value>) -> Result<ExecMode, String> {
    match parse_optional_string(args, "mode")?.as_deref() {
        Some("background") | None => Ok(ExecMode::Background),
        Some("service") => Ok(ExecMode::Service),
        Some(other) => Err(format!(
            "Invalid mode `{other}`. Must be one of: background, service"
        )),
    }
}

async fn parse_start_args(
    gcx: Arc<GlobalContext>,
    args: &HashMap<String, Value>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<ProcessStartArgs, String> {
    let command = parse_required_string(args, "command")?;
    let mode = parse_start_mode(args)?;
    let service_name = parse_optional_string(args, "service_name")?;
    let description = parse_optional_string(args, "description")?.ok_or_else(|| {
        "description is required and must be a non-empty single concise sentence".to_string()
    })?;
    let raw_workdir = parse_optional_string(args, "workdir")?;
    let (workdir, scope_warnings) =
        resolve_process_workdir(gcx, raw_workdir.as_deref(), execution_scope).await?;
    let startup_wait = parse_optional_u64(args, "startup_wait_ms")?.map(Duration::from_millis);
    let wait_port = parse_optional_u16(args, "startup_wait_port")?;
    let wait_keyword = parse_optional_string(args, "startup_wait_keyword")?;
    let tty = parse_optional_bool(args, "tty")?;
    let readiness = if wait_port.is_some() || wait_keyword.is_some() {
        Some(ExecReadinessProbe {
            wait_keyword,
            wait_port,
        })
    } else {
        None
    };
    Ok(ProcessStartArgs {
        command,
        workdir,
        mode,
        service_name,
        startup_wait,
        readiness,
        description,
        tty,
        scope_warnings,
    })
}

async fn resolve_process_workdir(
    gcx: Arc<GlobalContext>,
    raw_path: Option<&str>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<(Option<PathBuf>, Vec<String>), String> {
    if let Some(scope) = active_execution_scope(execution_scope) {
        let scoped = scope.resolve_workdir(raw_path).map_err(|e| {
            format!(
                "⚠️ Cannot resolve process workdir in active worktree '{}': {}",
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
                "⚠️ Cannot use process workdir '{}' (blocked by privacy: {}). Active worktree root: '{}'",
                scoped.path.display(),
                e,
                scope.effective_root().display()
            ));
        }
        let mut warnings = scoped_path_warnings(&scoped, scope);
        warnings.push(format!(
            "⚠️ Worktree scope: process cwd/workdir is enforced as '{}', but command text is not OS-sandboxed",
            scoped.path.display()
        ));
        return Ok((Some(scoped.path), warnings));
    }

    let Some(raw_path) = raw_path else {
        return Ok((process_workspace(gcx, None).await, Vec::new()));
    };
    let path_str = preprocess_path_for_normalization(raw_path.to_string());
    let path = PathBuf::from(&path_str);
    let workdir = if path.is_absolute() {
        let workdir = canonicalize_normalized_path(path);
        if let Some(workspace) = workspace_containing_path(gcx.clone(), &workdir).await {
            workspace
        } else {
            check_if_its_inside_a_workspace_or_config(gcx.clone(), &workdir).await?;
            workdir
        }
    } else {
        let project_dirs = get_project_dirs(gcx.clone()).await;
        let candidates = correct_to_nearest_dir_path(gcx.clone(), &path_str, false, 3).await;
        canonical_path(
            crate::at_commands::at_file::return_one_candidate_or_a_good_error(
                gcx,
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

async fn workspace_containing_path(gcx: Arc<GlobalContext>, path: &Path) -> Option<PathBuf> {
    let path = normalize_workspace_path(path);
    get_project_dirs(gcx)
        .await
        .into_iter()
        .map(|workspace| normalize_workspace_path(&workspace))
        .filter(|workspace| path.starts_with(workspace))
        .max_by_key(|workspace| workspace.components().count())
}

async fn process_workspace(
    gcx: Arc<GlobalContext>,
    execution_scope: Option<&ExecutionScope>,
) -> Option<PathBuf> {
    if let Some(scope) = active_execution_scope(execution_scope) {
        return Some(normalize_workspace_path(scope.effective_root()));
    }
    get_active_project_path(gcx)
        .await
        .map(|path| normalize_workspace_path(&path))
}

fn parse_list_status(args: &HashMap<String, Value>) -> Result<ProcessListStatus, String> {
    match parse_optional_string(args, "status")?.as_deref() {
        Some("running") | None => Ok(ProcessListStatus::Running),
        Some("completed") | Some("exited") => Ok(ProcessListStatus::Completed),
        Some("all") => Ok(ProcessListStatus::All),
        Some(other) => Err(format!(
            "Invalid status `{other}`. Must be one of: running, completed, all"
        )),
    }
}

fn parse_list_scope(args: &HashMap<String, Value>) -> Result<ProcessListScope, String> {
    match parse_optional_string(args, "scope")?.as_deref() {
        Some("chat") | None => Ok(ProcessListScope::Chat),
        Some("workspace") => Ok(ProcessListScope::Workspace),
        Some("all") => Ok(ProcessListScope::All),
        Some(other) => Err(format!(
            "Invalid scope `{other}`. Must be one of: chat, workspace, all"
        )),
    }
}

fn parse_stream_selection(args: &HashMap<String, Value>) -> Result<ProcessStreamSelection, String> {
    match parse_optional_string(args, "stream")?.as_deref() {
        Some("stdout") => Ok(ProcessStreamSelection::Stdout),
        Some("stderr") => Ok(ProcessStreamSelection::Stderr),
        Some("combined") | None => Ok(ProcessStreamSelection::Combined),
        Some("all") => Ok(ProcessStreamSelection::All),
        Some(other) => Err(format!(
            "Invalid stream `{other}`. Must be one of: stdout, stderr, combined, all"
        )),
    }
}

fn parse_shell_service_alias_args(
    args: &HashMap<String, Value>,
) -> Result<ShellServiceAliasArgs, String> {
    let service_name = parse_required_string(args, "service_name")?;
    let action = parse_required_string(args, "action")?.to_lowercase();
    if !["start", "stop", "status", "logs", "restart"].contains(&action.as_str()) {
        return Err(format!(
            "Invalid action '{}'. Must be one of: start, stop, status, logs, restart",
            action
        ));
    }
    let command = if matches!(action.as_str(), "start" | "restart") {
        Some(parse_required_string(args, "command")?)
    } else {
        parse_optional_string(args, "command")?
    };
    let startup_wait_ms = parse_optional_u64(args, "startup_wait_ms")?.or_else(|| {
        parse_optional_u64(args, "startup_wait")
            .ok()
            .flatten()
            .map(|seconds| seconds.saturating_mul(1000))
    });
    Ok(ShellServiceAliasArgs {
        service_name,
        action,
        command,
        workdir: parse_optional_string(args, "workdir")?,
        startup_wait_ms,
        startup_wait_port: parse_optional_u16(args, "startup_wait_port")?,
        startup_wait_keyword: parse_optional_string(args, "startup_wait_keyword")?,
        output_filter: parse_optional_string(args, "output_filter")?,
        output_limit: parse_optional_string(args, "output_limit")?,
    })
}

fn process_start_args_from_shell_service(
    parsed: &ShellServiceAliasArgs,
) -> Result<HashMap<String, Value>, String> {
    let command = parsed
        .command
        .as_ref()
        .ok_or_else(|| "Command is required for start/restart action".to_string())?;
    let description = generate_short_description(command, &ExecMode::Service);
    let mut result = make_args_map(vec![
        ("command", json!(command)),
        ("description", json!(description)),
        ("mode", json!("service")),
        ("service_name", json!(parsed.service_name)),
    ]);
    if let Some(workdir) = parsed.workdir.as_ref() {
        result.insert("workdir".to_string(), json!(workdir));
    }
    if let Some(startup_wait_ms) = parsed.startup_wait_ms {
        result.insert("startup_wait_ms".to_string(), json!(startup_wait_ms));
    }
    if let Some(startup_wait_port) = parsed.startup_wait_port {
        result.insert("startup_wait_port".to_string(), json!(startup_wait_port));
    }
    if let Some(startup_wait_keyword) = parsed.startup_wait_keyword.as_ref() {
        result.insert(
            "startup_wait_keyword".to_string(),
            json!(startup_wait_keyword),
        );
    }
    Ok(result)
}

async fn require_process(
    registry: &crate::exec::ExecRegistry,
    process_id: &ExecProcessId,
) -> Result<ExecProcessSnapshot, String> {
    registry
        .get(process_id)
        .await
        .ok_or_else(|| format!("process not found: {process_id}"))
}

fn matches_list_status(snapshot: &ExecProcessSnapshot, status: ProcessListStatus) -> bool {
    match status {
        ProcessListStatus::Running => {
            matches!(snapshot.status, ExecStatus::Starting | ExecStatus::Running)
        }
        ProcessListStatus::Completed => snapshot.status.is_terminal(),
        ProcessListStatus::All => true,
    }
}

fn matches_list_scope(
    snapshot: &ExecProcessSnapshot,
    scope: ProcessListScope,
    chat_id: &str,
    workspace: Option<&PathBuf>,
) -> bool {
    match scope {
        ProcessListScope::Chat => {
            chat_id.is_empty() || snapshot.meta.owner.chat_id.as_deref() == Some(chat_id)
        }
        ProcessListScope::Workspace => workspace
            .map(|workspace| {
                snapshot
                    .meta
                    .owner
                    .workspace
                    .as_deref()
                    .map(normalize_workspace_path)
                    == Some(normalize_workspace_path(workspace))
            })
            .unwrap_or(true),
        ProcessListScope::All => true,
    }
}

fn list_status_label(status: ProcessListStatus) -> &'static str {
    match status {
        ProcessListStatus::Running => "running",
        ProcessListStatus::Completed => "completed",
        ProcessListStatus::All => "all",
    }
}

fn list_scope_label(scope: ProcessListScope) -> &'static str {
    match scope {
        ProcessListScope::Chat => "chat",
        ProcessListScope::Workspace => "workspace",
        ProcessListScope::All => "all",
    }
}

fn stream_label(stream: ProcessStreamSelection) -> &'static str {
    match stream {
        ProcessStreamSelection::Stdout => "stdout",
        ProcessStreamSelection::Stderr => "stderr",
        ProcessStreamSelection::Combined => "combined",
        ProcessStreamSelection::All => "all",
    }
}

fn status_label(status: &ExecStatus) -> &'static str {
    match status {
        ExecStatus::Starting => "starting",
        ExecStatus::Running => "running",
        ExecStatus::Exited { .. } => "exited",
        ExecStatus::Failed { .. } => "failed",
        ExecStatus::Killed => "killed",
        ExecStatus::TimedOut => "timed_out",
    }
}

fn exit_code(status: &ExecStatus) -> Option<i32> {
    match status {
        ExecStatus::Exited { exit_code } => *exit_code,
        ExecStatus::Starting
        | ExecStatus::Running
        | ExecStatus::Failed { .. }
        | ExecStatus::Killed
        | ExecStatus::TimedOut => None,
    }
}

fn tool_failed_for_status(status: &ExecStatus) -> Option<bool> {
    match status {
        ExecStatus::Failed { .. } | ExecStatus::Killed | ExecStatus::TimedOut => Some(true),
        ExecStatus::Starting | ExecStatus::Running | ExecStatus::Exited { .. } => None,
    }
}

fn process_value(snapshot: &ExecProcessSnapshot) -> Value {
    let cwd = snapshot
        .meta
        .cwd
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    let workspace = snapshot
        .meta
        .owner
        .workspace
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    json!({
        "process_id": snapshot.meta.process_id.as_str(),
        "status": status_label(&snapshot.status),
        "status_detail": serde_json::to_value(&snapshot.status).unwrap_or(Value::Null),
        "mode": snapshot.meta.mode.to_string(),
        "service_name": snapshot.meta.owner.service_name.as_deref(),
        "chat_id": snapshot.meta.owner.chat_id.as_deref(),
        "tool_call_id": snapshot.meta.owner.tool_call_id.as_deref(),
        "workspace": workspace,
        "command": snapshot.meta.command.as_str(),
        "cwd": cwd,
        "short_description": snapshot.meta.short_description.as_str(),
        "created_at": snapshot.meta.created_at_ms,
        "created_at_ms": snapshot.meta.created_at_ms,
        "started_at": snapshot.meta.started_at_ms,
        "started_at_ms": snapshot.meta.started_at_ms,
        "ended_at": snapshot.meta.ended_at_ms,
        "ended_at_ms": snapshot.meta.ended_at_ms,
        "exit_code": exit_code(&snapshot.status),
    })
}

fn exec_extra(
    snapshot: &ExecProcessSnapshot,
    read: Option<&ExecReadResult>,
    stream: Option<&str>,
    tty: Option<bool>,
) -> serde_json::Map<String, Value> {
    let mut value = process_value(snapshot);
    if let Some(tty) = tty {
        value["tty"] = json!(tty);
    }
    if let Some(read) = read {
        value["transcript"] = read_value(read);
    }
    if let Some(stream) = stream {
        value["stream"] = json!(stream);
    }
    let mut extra = serde_json::Map::new();
    extra.insert("exec".to_string(), value);
    extra
}

fn read_value(read: &ExecReadResult) -> Value {
    json!({
        "process_id": read.process_id.as_str(),
        "found": read.found,
        "since_seq": read.since_seq,
        "next_seq": read.next_seq,
        "latest_seq": read.latest_seq,
        "total_bytes_appended": read.total_bytes_appended,
        "total_lines_appended": read.total_lines_appended,
        "dropped_chunks": read.dropped_chunks,
        "dropped_bytes": read.dropped_bytes,
        "truncated_chunks": read.truncated_chunks,
        "current_bytes": read.current_bytes,
        "max_bytes": read.max_bytes,
        "chunk_count": read.chunk_count,
        "is_truncated": read.is_truncated,
    })
}

fn format_process_snapshot(title: &str, snapshot: &ExecProcessSnapshot) -> String {
    let cwd = snapshot
        .meta
        .cwd
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<default>".to_string());
    let service_name = snapshot
        .meta
        .owner
        .service_name
        .as_deref()
        .unwrap_or("<none>");
    let exit_code = exit_code(&snapshot.status)
        .map(|code| code.to_string())
        .unwrap_or_else(|| "<none>".to_string());
    let failure_message = match &snapshot.status {
        ExecStatus::Failed { message } => Some(message.clone()),
        _ => None,
    };
    let mut out = format!(
        "{title}\nprocess_id: {}\nshort_description: {}\nstatus: {}\nmode: {}\nservice_name: {}\ncommand: {}\ncwd: {}\nstarted_at: {:?}\nexit_code: {}\n",
        snapshot.meta.process_id,
        snapshot.meta.short_description,
        status_label(&snapshot.status),
        snapshot.meta.mode,
        service_name,
        snapshot.meta.command,
        cwd,
        snapshot.meta.started_at_ms,
        exit_code
    );
    if let Some(message) = failure_message {
        out.push_str(&format!("failure_reason: {}\n", message));
    }
    out
}

fn format_read_sections(
    read: &ExecReadResult,
    selection: ProcessStreamSelection,
    output_filter: &OutputFilter,
) -> String {
    let mut out = String::new();
    match selection {
        ProcessStreamSelection::Stdout => append_section(
            &mut out,
            "stdout",
            &collect_stream(&read.chunks, ExecOutputStream::Stdout),
            output_filter,
        ),
        ProcessStreamSelection::Stderr => append_section(
            &mut out,
            "stderr",
            &collect_stream(&read.chunks, ExecOutputStream::Stderr),
            output_filter,
        ),
        ProcessStreamSelection::Combined => append_section(
            &mut out,
            "combined",
            &collect_combined(&read.chunks),
            output_filter,
        ),
        ProcessStreamSelection::All => {
            append_section(
                &mut out,
                "stdout",
                &collect_stream(&read.chunks, ExecOutputStream::Stdout),
                output_filter,
            );
            append_section(
                &mut out,
                "stderr",
                &collect_stream(&read.chunks, ExecOutputStream::Stderr),
                output_filter,
            );
        }
    }
    out.push_str(&format!(
        "transcript: next_seq={}, latest_seq={}, current_bytes={}, dropped_bytes={}, truncated_chunks={}, is_truncated={}\n",
        read.next_seq,
        read.latest_seq,
        read.current_bytes,
        read.dropped_bytes,
        read.truncated_chunks,
        read.is_truncated
    ));
    out
}

fn append_section(out: &mut String, title: &str, text: &str, output_filter: &OutputFilter) {
    out.push_str(&format!("\n{title}:\n"));
    if text.is_empty() {
        out.push_str("<empty>\n");
    } else {
        out.push_str(&output_mini_postprocessing(output_filter, text));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
}

fn collect_stream(chunks: &[ExecOutputChunk], stream: ExecOutputStream) -> String {
    chunks
        .iter()
        .filter(|chunk| chunk.stream == stream)
        .map(|chunk| chunk.text.as_str())
        .collect::<String>()
}

fn collect_combined(chunks: &[ExecOutputChunk]) -> String {
    chunks
        .iter()
        .map(|chunk| chunk.text.as_str())
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::exec::{ExecProcessMeta, ExecStatusKind};

    async fn test_ccx() -> (Arc<GlobalContext>, Arc<AMutex<AtCommandsContext>>) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = AtCommandsContext::new_with_abort(
            AppState::from_gcx(gcx.clone()).await,
            4096,
            20,
            false,
            Vec::new(),
            "chat".to_string(),
            None,
            "model".to_string(),
            None,
            None,
            None,
        )
        .await;
        (gcx, Arc::new(AMutex::new(ccx)))
    }

    async fn test_ccx_for_workspace(
        gcx: Arc<GlobalContext>,
        workspace: PathBuf,
    ) -> Arc<AMutex<AtCommandsContext>> {
        let ccx = AtCommandsContext::new_with_abort(
            AppState::from_gcx(gcx).await,
            4096,
            20,
            false,
            Vec::new(),
            "chat".to_string(),
            None,
            "model".to_string(),
            None,
            None,
            None,
        )
        .await;
        ccx.global_context
            .documents_state
            .active_file_path
            .lock()
            .await
            .replace(workspace);
        Arc::new(AMutex::new(ccx))
    }

    async fn run_tool<T: Tool>(
        tool: &mut T,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: HashMap<String, Value>,
    ) -> Result<ChatMessage, String> {
        let (_, messages) = tool
            .tool_execute(ccx, &"tool_call".to_string(), &args)
            .await?;
        Ok(only_message(messages))
    }

    fn only_message(messages: Vec<ContextEnum>) -> ChatMessage {
        match messages.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => message,
            ContextEnum::ContextFile(_) => panic!("expected chat message"),
        }
    }

    fn text(message: &ChatMessage) -> String {
        match &message.content {
            ChatContent::SimpleText(text) => text.clone(),
            _ => String::new(),
        }
    }

    fn exec(message: &ChatMessage) -> &Value {
        message.extra.get("exec").unwrap()
    }

    fn process_id(message: &ChatMessage) -> ExecProcessId {
        ExecProcessId(exec(message)["process_id"].as_str().unwrap().to_string())
    }

    fn long_running_command(output: &str) -> String {
        if cfg!(target_os = "windows") {
            format!("[Console]::Out.Write('{output}'); Start-Sleep -Seconds 30")
        } else {
            format!("printf {output:?}; sleep 30")
        }
    }

    fn quick_command(output: &str) -> String {
        if cfg!(target_os = "windows") {
            format!("[Console]::Out.Write('{output}')")
        } else {
            format!("printf {output:?}")
        }
    }

    async fn wait_for_output(gcx: Arc<GlobalContext>, process_id: &ExecProcessId, needle: &str) {
        for _ in 0..40 {
            let read = gcx.exec_registry.read(process_id, 0, None).await;
            if read.chunks.iter().any(|chunk| chunk.text.contains(needle)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("output did not contain {needle}");
    }

    fn required_names(desc: ToolDesc) -> Vec<String> {
        desc.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn tool_process_descriptions_require_expected_args() {
        assert_eq!(
            required_names(
                ToolProcessStart {
                    config_path: String::new(),
                }
                .tool_description()
            ),
            vec!["command".to_string(), "description".to_string()]
        );
        let start_desc = ToolProcessStart {
            config_path: String::new(),
        }
        .tool_description();
        assert_eq!(
            start_desc.input_schema["properties"]["tty"]["type"],
            "boolean"
        );
        assert_eq!(
            start_desc.input_schema["properties"]["tty"]["default"],
            false
        );
        assert_eq!(
            start_desc.input_schema["properties"]["tty"]["description"],
            TTY_DESCRIPTION
        );
        assert!(start_desc.description.contains("merges stdout and stderr"));
        assert!(required_names(
            ToolProcessList {
                config_path: String::new(),
            }
            .tool_description()
        )
        .is_empty());
        for desc in [
            ToolProcessRead {
                config_path: String::new(),
            }
            .tool_description(),
            ToolProcessKill {
                config_path: String::new(),
            }
            .tool_description(),
            ToolProcessWait {
                config_path: String::new(),
            }
            .tool_description(),
        ] {
            assert_eq!(required_names(desc), vec!["process_id".to_string()]);
        }
    }

    #[tokio::test]
    async fn tool_processes_are_registered_in_system_group() {
        let names = crate::tools::tools_list::builtin_system_tools(String::new())
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect::<Vec<_>>();
        for expected in [
            "process_start",
            "process_list",
            "process_read",
            "process_kill",
            "process_wait",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[tokio::test]
    async fn tool_process_start_background_lifecycle_start_list_read_kill() {
        let (gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let message = run_tool(
            &mut start,
            ccx.clone(),
            make_args_map(vec![
                ("command", json!(long_running_command("ready"))),
                ("description", json!("Run background gremlin")),
                ("startup_wait_ms", json!(100)),
            ]),
        )
        .await
        .unwrap();
        let process_id = process_id(&message);
        assert_eq!(
            exec(&message)["short_description"],
            "Run background gremlin"
        );
        assert_eq!(exec(&message)["status"], "running");
        assert_eq!(exec(&message)["tty"], false);
        wait_for_output(gcx.clone(), &process_id, "ready").await;

        let mut list = ToolProcessList {
            config_path: String::new(),
        };
        let listed = run_tool(&mut list, ccx.clone(), make_args_map(vec![]))
            .await
            .unwrap();
        assert!(text(&listed).contains(process_id.as_str()));
        assert_eq!(exec(&listed)["count"], 1);

        let mut read = ToolProcessRead {
            config_path: String::new(),
        };
        let output = run_tool(
            &mut read,
            ccx.clone(),
            make_args_map(vec![("process_id", json!(process_id.as_str()))]),
        )
        .await
        .unwrap();
        assert!(text(&output).contains("ready"));
        assert!(exec(&output)["transcript"]["next_seq"].as_u64().unwrap() > 0);

        let mut kill = ToolProcessKill {
            config_path: String::new(),
        };
        let killed = run_tool(
            &mut kill,
            ccx,
            make_args_map(vec![("process_id", json!(process_id.as_str()))]),
        )
        .await
        .unwrap();
        assert_eq!(exec(&killed)["status"], "killed");
    }

    #[tokio::test]
    async fn process_start_tty_true_uses_pty() {
        let (gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let message = run_tool(
            &mut start,
            ccx.clone(),
            make_args_map(vec![
                ("command", json!(long_running_command("pty-ready"))),
                ("description", json!("Run background process through pty")),
                ("startup_wait_ms", json!(100)),
                ("tty", json!(true)),
            ]),
        )
        .await
        .unwrap();
        let process_id = process_id(&message);
        assert_eq!(exec(&message)["tty"], true);
        assert_eq!(exec(&message)["status"], "running");
        wait_for_output(gcx, &process_id, "pty-ready").await;

        let mut kill = ToolProcessKill {
            config_path: String::new(),
        };
        run_tool(
            &mut kill,
            ccx,
            make_args_map(vec![("process_id", json!(process_id.as_str()))]),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tool_process_start_service_mode_tracks_service_name() {
        let (_gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let message = run_tool(
            &mut start,
            ccx.clone(),
            make_args_map(vec![
                ("command", json!(long_running_command("svc"))),
                ("description", json!("Start api service")),
                ("mode", json!("service")),
                ("service_name", json!("api")),
                ("startup_wait_ms", json!(100)),
            ]),
        )
        .await
        .unwrap();
        let process_id = process_id(&message);
        let expected_owner = ExecOwnerMeta {
            chat_id: Some("chat".to_string()),
            service_name: Some("api".to_string()),
            workspace: None,
            tool_call_id: Some("tool_call".to_string()),
        };
        assert_eq!(
            process_id,
            ExecProcessId::for_service("api", &expected_owner)
        );
        assert_eq!(exec(&message)["mode"], "service");
        assert_eq!(exec(&message)["service_name"], "api");

        let mut list = ToolProcessList {
            config_path: String::new(),
        };
        let listed = run_tool(
            &mut list,
            ccx.clone(),
            make_args_map(vec![("status", json!("running")), ("scope", json!("chat"))]),
        )
        .await
        .unwrap();
        assert!(text(&listed).contains("service_name: api"));

        let mut kill = ToolProcessKill {
            config_path: String::new(),
        };
        run_tool(
            &mut kill,
            ccx,
            make_args_map(vec![("process_id", json!(process_id.as_str()))]),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tool_process_start_service_requires_service_name() {
        let (_gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let err = run_tool(
            &mut start,
            ccx,
            make_args_map(vec![
                ("command", json!(long_running_command("svc"))),
                ("description", json!("Start service without name")),
                ("mode", json!("service")),
            ]),
        )
        .await
        .unwrap_err();
        assert_eq!(err, "service mode requires service_name");
    }

    #[tokio::test]
    async fn tool_process_start_service_duplicate_name_is_rejected() {
        let (_gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let first = run_tool(
            &mut start,
            ccx.clone(),
            make_args_map(vec![
                ("command", json!(long_running_command("svc"))),
                ("description", json!("Start first api service")),
                ("mode", json!("service")),
                ("service_name", json!("api")),
                ("startup_wait_ms", json!(100)),
            ]),
        )
        .await
        .unwrap();
        let process_id = process_id(&first);

        let err = run_tool(
            &mut start,
            ccx.clone(),
            make_args_map(vec![
                ("command", json!(long_running_command("svc2"))),
                ("description", json!("Start duplicate api service")),
                ("mode", json!("service")),
                ("service_name", json!("api")),
                ("startup_wait_ms", json!(100)),
            ]),
        )
        .await
        .unwrap_err();
        assert!(err.contains("already running"));
        assert!(err.contains(process_id.as_str()));

        let mut kill = ToolProcessKill {
            config_path: String::new(),
        };
        run_tool(
            &mut kill,
            ccx,
            make_args_map(vec![("process_id", json!(process_id.as_str()))]),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tool_process_start_same_service_name_in_distinct_chats_does_not_collide() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        let first_ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_with_abort(
                app.clone(),
                4096,
                20,
                false,
                Vec::new(),
                "chat-a".to_string(),
                None,
                "model".to_string(),
                None,
                None,
                None,
            )
            .await,
        ));
        let second_ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_with_abort(
                app,
                4096,
                20,
                false,
                Vec::new(),
                "chat-b".to_string(),
                None,
                "model".to_string(),
                None,
                None,
                None,
            )
            .await,
        ));
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let first = run_tool(
            &mut start,
            first_ccx,
            make_args_map(vec![
                ("command", json!(long_running_command("svc-a"))),
                ("description", json!("Start api service in chat-a")),
                ("mode", json!("service")),
                ("service_name", json!("api")),
                ("startup_wait_ms", json!(100)),
            ]),
        )
        .await
        .unwrap();
        let second = run_tool(
            &mut start,
            second_ccx,
            make_args_map(vec![
                ("command", json!(long_running_command("svc-b"))),
                ("description", json!("Start api service in chat-b")),
                ("mode", json!("service")),
                ("service_name", json!("api")),
                ("startup_wait_ms", json!(100)),
            ]),
        )
        .await
        .unwrap();
        let first_process_id = process_id(&first);
        let second_process_id = process_id(&second);

        assert_ne!(first_process_id, second_process_id);
        assert_eq!(exec(&first)["status"], "running");
        assert_eq!(exec(&second)["status"], "running");

        gcx.exec_registry.kill(&first_process_id).await.unwrap();
        gcx.exec_registry.kill(&second_process_id).await.unwrap();
    }

    #[tokio::test]
    async fn tool_process_start_same_service_name_in_distinct_workspaces_does_not_collide() {
        let temp = tempfile::tempdir().unwrap();
        let workspace_a = temp.path().join("workspace-a");
        let workspace_b = temp.path().join("workspace-b");
        std::fs::create_dir_all(&workspace_a).unwrap();
        std::fs::create_dir_all(&workspace_b).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() =
                vec![workspace_a.clone(), workspace_b.clone()];
        }
        let first_ccx = test_ccx_for_workspace(gcx.clone(), workspace_a.clone()).await;
        let second_ccx = test_ccx_for_workspace(gcx.clone(), workspace_b.clone()).await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let first = run_tool(
            &mut start,
            first_ccx,
            make_args_map(vec![
                ("command", json!(long_running_command("svc-a"))),
                ("description", json!("Start api service in workspace-a")),
                ("workdir", json!(workspace_a.to_string_lossy().to_string())),
                ("mode", json!("service")),
                ("service_name", json!("api")),
                ("startup_wait_ms", json!(100)),
            ]),
        )
        .await
        .unwrap();
        let second = run_tool(
            &mut start,
            second_ccx,
            make_args_map(vec![
                ("command", json!(long_running_command("svc-b"))),
                ("description", json!("Start api service in workspace-b")),
                ("workdir", json!(workspace_b.to_string_lossy().to_string())),
                ("mode", json!("service")),
                ("service_name", json!("api")),
                ("startup_wait_ms", json!(100)),
            ]),
        )
        .await
        .unwrap();
        let first_process_id = process_id(&first);
        let second_process_id = process_id(&second);

        assert_ne!(first_process_id, second_process_id);
        assert_eq!(exec(&first)["status"], "running");
        assert_eq!(exec(&second)["status"], "running");

        gcx.exec_registry.kill(&first_process_id).await.unwrap();
        gcx.exec_registry.kill(&second_process_id).await.unwrap();
    }

    #[tokio::test]
    async fn tool_process_start_equivalent_workspace_rejects_duplicate_service() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.clone()];
        }
        let ccx = test_ccx_for_workspace(gcx.clone(), workspace.clone()).await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let first = run_tool(
            &mut start,
            ccx.clone(),
            make_args_map(vec![
                ("command", json!(long_running_command("svc-a"))),
                ("description", json!("Start api service in workspace")),
                ("workdir", json!(workspace.to_string_lossy().to_string())),
                ("mode", json!("service")),
                ("service_name", json!("api")),
                ("startup_wait_ms", json!(100)),
            ]),
        )
        .await
        .unwrap();
        let first_process_id = process_id(&first);

        let err = run_tool(
            &mut start,
            ccx.clone(),
            make_args_map(vec![
                ("command", json!(long_running_command("svc-b"))),
                (
                    "description",
                    json!("Start duplicate api service in equivalent workspace"),
                ),
                (
                    "workdir",
                    json!(workspace.join(".").to_string_lossy().to_string()),
                ),
                ("mode", json!("service")),
                ("service_name", json!("api")),
                ("startup_wait_ms", json!(100)),
            ]),
        )
        .await
        .unwrap_err();

        assert!(err.contains("already running"));
        assert!(err.contains(first_process_id.as_str()));
        assert_eq!(
            gcx.exec_registry
                .list(ExecProcessFilter {
                    service_name: Some("api".to_string()),
                    status: Some(ExecStatusKind::Running),
                    ..ExecProcessFilter::default()
                })
                .await
                .len(),
            1
        );

        gcx.exec_registry.kill(&first_process_id).await.unwrap();
    }

    #[tokio::test]
    async fn tool_process_start_service_keyword_readiness_success() {
        let (gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let command = if cfg!(target_os = "windows") {
            "Start-Sleep -Milliseconds 200; [Console]::Out.Write('ready-keyword'); Start-Sleep -Seconds 30".to_string()
        } else {
            "sleep 0.2; printf ready-keyword; sleep 30".to_string()
        };
        let message = run_tool(
            &mut start,
            ccx.clone(),
            make_args_map(vec![
                ("command", json!(command)),
                ("description", json!("Start keyword readiness service")),
                ("mode", json!("service")),
                ("service_name", json!("keyword")),
                ("startup_wait_ms", json!(2000)),
                ("startup_wait_keyword", json!("ready-keyword")),
            ]),
        )
        .await
        .unwrap();
        let process_id = process_id(&message);
        assert_eq!(exec(&message)["status"], "running");
        wait_for_output(gcx, &process_id, "ready-keyword").await;

        let mut kill = ToolProcessKill {
            config_path: String::new(),
        };
        run_tool(
            &mut kill,
            ccx,
            make_args_map(vec![("process_id", json!(process_id.as_str()))]),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tool_process_start_service_keyword_readiness_timeout_fails_and_stops() {
        let (_gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let message = run_tool(
            &mut start,
            ccx,
            make_args_map(vec![
                ("command", json!(long_running_command("not-yet"))),
                ("description", json!("Start service with readiness timeout")),
                ("mode", json!("service")),
                ("service_name", json!("timeout")),
                ("startup_wait_ms", json!(100)),
                ("startup_wait_keyword", json!("never-there")),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(exec(&message)["status"], "failed");
        assert_eq!(message.tool_failed, Some(true));
        assert!(text(&message).contains("startup readiness timed out"));
    }

    #[tokio::test]
    async fn tool_process_read_since_seq_cursor_and_filters() {
        let (gcx, ccx) = test_ccx().await;
        let snapshot = gcx
            .exec_registry
            .register(
                ExecProcessMeta::new(ExecMode::Background, "test".to_string())
                    .with_chat_id("chat")
                    .with_short_description("Read filter test".to_string()),
                PROCESS_TRANSCRIPT_MAX_BYTES,
            )
            .await;
        let process_id = snapshot.meta.process_id;
        gcx.exec_registry.mark_started(&process_id).await.unwrap();
        gcx.exec_registry
            .append_output(&process_id, ExecOutputStream::Stdout, "alpha\n".to_string())
            .await
            .unwrap();
        gcx.exec_registry
            .append_output(
                &process_id,
                ExecOutputStream::Stderr,
                "needle\n".to_string(),
            )
            .await
            .unwrap();
        gcx.exec_registry
            .append_output(&process_id, ExecOutputStream::Stdout, "omega\n".to_string())
            .await
            .unwrap();

        let mut read = ToolProcessRead {
            config_path: String::new(),
        };
        let message = run_tool(
            &mut read,
            ccx,
            make_args_map(vec![
                ("process_id", json!(process_id.as_str())),
                ("since_seq", json!(1)),
                ("stream", json!("stderr")),
                ("output_filter", json!("needle")),
                ("output_limit", json!("1")),
            ]),
        )
        .await
        .unwrap();
        let body = text(&message);
        assert!(body.contains("needle"));
        assert!(!body.contains("alpha"));
        assert_eq!(exec(&message)["transcript"]["since_seq"], 1);
        assert_eq!(exec(&message)["transcript"]["next_seq"], 3);
        assert_eq!(exec(&message)["stream"], "stderr");
    }

    #[tokio::test]
    async fn background_output_filter_uses_transcript() {
        let (gcx, ccx) = test_ccx().await;
        let snapshot = gcx
            .exec_registry
            .register(
                ExecProcessMeta::new(ExecMode::Background, "test".to_string())
                    .with_chat_id("chat")
                    .with_short_description("Background transcript filter".to_string()),
                64,
            )
            .await;
        let process_id = snapshot.meta.process_id;
        gcx.exec_registry.mark_started(&process_id).await.unwrap();
        gcx.exec_registry
            .append_output(
                &process_id,
                ExecOutputStream::Stdout,
                "BACKGROUND_OLD_NEEDLE\n".to_string(),
            )
            .await
            .unwrap();
        for index in 0..8 {
            gcx.exec_registry
                .append_output(
                    &process_id,
                    ExecOutputStream::Stdout,
                    format!("filler-line-{index:02}-xxxxxxxx\n"),
                )
                .await
                .unwrap();
        }

        let mut read = ToolProcessRead {
            config_path: String::new(),
        };
        let message = run_tool(
            &mut read,
            ccx,
            make_args_map(vec![
                ("process_id", json!(process_id.as_str())),
                ("since_seq", json!(0)),
                ("stream", json!("stdout")),
                ("output_filter", json!("BACKGROUND_OLD_NEEDLE")),
                ("output_limit", json!("4")),
            ]),
        )
        .await
        .unwrap();
        let body = text(&message);

        assert!(!body.contains("BACKGROUND_OLD_NEEDLE"));
        assert_eq!(exec(&message)["transcript"]["since_seq"], 0);
        assert!(
            exec(&message)["transcript"]["dropped_bytes"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(exec(&message)["stream"], "stdout");
    }

    #[tokio::test]
    async fn tool_process_kill_unknown_id_errors() {
        let (_gcx, ccx) = test_ccx().await;
        let mut kill = ToolProcessKill {
            config_path: String::new(),
        };
        let err = run_tool(
            &mut kill,
            ccx,
            make_args_map(vec![("process_id", json!("exec_missing"))]),
        )
        .await
        .unwrap_err();
        assert_eq!(err, "process not found: exec_missing");
    }

    #[tokio::test]
    async fn tool_process_wait_timeout_and_completion_paths() {
        let (_gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let slow = run_tool(
            &mut start,
            ccx.clone(),
            make_args_map(vec![
                ("command", json!(long_running_command("slow"))),
                ("description", json!("Start slow background process")),
                ("startup_wait_ms", json!(50)),
            ]),
        )
        .await
        .unwrap();
        let slow_process_id = process_id(&slow);

        let mut wait = ToolProcessWait {
            config_path: String::new(),
        };
        let timed_out = run_tool(
            &mut wait,
            ccx.clone(),
            make_args_map(vec![
                ("process_id", json!(slow_process_id.as_str())),
                ("timeout_ms", json!(50)),
            ]),
        )
        .await
        .unwrap();
        assert!(text(&timed_out).contains("Process wait timed out"));
        assert_eq!(exec(&timed_out)["status"], "running");

        let mut kill = ToolProcessKill {
            config_path: String::new(),
        };
        run_tool(
            &mut kill,
            ccx.clone(),
            make_args_map(vec![("process_id", json!(slow_process_id.as_str()))]),
        )
        .await
        .unwrap();

        let done = run_tool(
            &mut start,
            ccx.clone(),
            make_args_map(vec![
                ("command", json!(quick_command("done"))),
                (
                    "description",
                    json!("Run quick command for wait completion test"),
                ),
            ]),
        )
        .await
        .unwrap();
        let done_process_id = process_id(&done);
        let completed = run_tool(
            &mut wait,
            ccx,
            make_args_map(vec![
                ("process_id", json!(done_process_id.as_str())),
                ("timeout_ms", json!(2000)),
            ]),
        )
        .await
        .unwrap();
        assert!(text(&completed).contains("Process wait completed"));
        assert_eq!(exec(&completed)["status"], "exited");
        assert_eq!(exec(&completed)["exit_code"], 0);
    }

    #[test]
    fn tool_process_kill_has_no_confirmation_rules() {
        let kill = ToolProcessKill {
            config_path: String::new(),
        };
        assert!(kill.confirm_deny_rules().is_none());
    }

    #[tokio::test]
    async fn tool_process_start_confirmation_matches_command_text() {
        let (_gcx, ccx) = test_ccx().await;
        let start = ToolProcessStart {
            config_path: String::new(),
        };
        let command = "rm -rf target";
        let matched = start
            .match_against_confirm_deny(
                ccx,
                &make_args_map(vec![
                    ("command", json!(command)),
                    ("description", json!("Remove build artifacts")),
                    ("mode", json!("service")),
                    ("service_name", json!("danger")),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(matched.command, command);
        assert_eq!(
            matched.result,
            crate::tools::tools_description::MatchConfirmDenyResult::CONFIRMATION
        );
    }

    #[tokio::test]
    async fn shell_service_alias_is_registered_and_uses_exec_registry() {
        let (gcx, ccx) = test_ccx().await;
        let names = crate::tools::tools_list::builtin_system_tools(String::new())
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect::<Vec<_>>();
        assert!(names.contains(&"shell_service".to_string()));

        let mut alias = ToolShellServiceAlias {
            config_path: String::new(),
        };
        let message = run_tool(
            &mut alias,
            ccx.clone(),
            make_args_map(vec![
                ("service_name", json!("alias")),
                ("action", json!("start")),
                ("command", json!(long_running_command("alias-ready"))),
                ("startup_wait", json!("1")),
            ]),
        )
        .await
        .unwrap();
        let process_id = process_id(&message);
        let expected_owner = ExecOwnerMeta {
            chat_id: Some("chat".to_string()),
            service_name: Some("alias".to_string()),
            workspace: None,
            tool_call_id: Some("tool_call".to_string()),
        };
        assert_eq!(
            process_id,
            ExecProcessId::for_service("alias", &expected_owner)
        );
        assert!(gcx.integration_sessions.lock().await.is_empty());

        let logs = run_tool(
            &mut alias,
            ccx.clone(),
            make_args_map(vec![
                ("service_name", json!("alias")),
                ("action", json!("logs")),
            ]),
        )
        .await
        .unwrap();
        assert!(text(&logs).contains("alias-ready"));

        let stopped = run_tool(
            &mut alias,
            ccx,
            make_args_map(vec![
                ("service_name", json!("alias")),
                ("action", json!("stop")),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(exec(&stopped)["status"], "killed");
    }

    #[tokio::test]
    async fn description_required_rejects_missing() {
        let (_gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let err = run_tool(
            &mut start,
            ccx,
            make_args_map(vec![("command", json!("printf hi"))]),
        )
        .await
        .unwrap_err();
        assert_eq!(
            err,
            "description is required and must be a non-empty single concise sentence"
        );
    }

    #[tokio::test]
    async fn description_required_rejects_empty() {
        let (_gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let err = run_tool(
            &mut start,
            ccx,
            make_args_map(vec![
                ("command", json!("printf hi")),
                ("description", json!("")),
            ]),
        )
        .await
        .unwrap_err();
        assert_eq!(
            err,
            "description is required and must be a non-empty single concise sentence"
        );
    }

    #[tokio::test]
    async fn description_required_rejects_whitespace() {
        let (_gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let err = run_tool(
            &mut start,
            ccx,
            make_args_map(vec![
                ("command", json!("printf hi")),
                ("description", json!("  \t\n")),
            ]),
        )
        .await
        .unwrap_err();
        assert_eq!(
            err,
            "description is required and must be a non-empty single concise sentence"
        );
    }

    #[tokio::test]
    async fn description_accepted_when_present() {
        let (_gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let message = run_tool(
            &mut start,
            ccx,
            make_args_map(vec![
                ("command", json!(quick_command("hi"))),
                ("description", json!("Run engine tests")),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(exec(&message)["short_description"], "Run engine tests");
    }

    #[tokio::test]
    async fn tool_process_list_completed_status() {
        let (gcx, ccx) = test_ccx().await;
        let running = gcx
            .exec_registry
            .register(
                ExecProcessMeta::new(ExecMode::Background, "sleep".to_string())
                    .with_chat_id("chat"),
                PROCESS_TRANSCRIPT_MAX_BYTES,
            )
            .await;
        gcx.exec_registry
            .mark_started(&running.meta.process_id)
            .await
            .unwrap();
        let completed = gcx
            .exec_registry
            .register(
                ExecProcessMeta::new(ExecMode::Background, "true".to_string()).with_chat_id("chat"),
                PROCESS_TRANSCRIPT_MAX_BYTES,
            )
            .await;
        gcx.exec_registry
            .mark_exited(&completed.meta.process_id, Some(0))
            .await
            .unwrap();

        let mut list = ToolProcessList {
            config_path: String::new(),
        };
        let message = run_tool(
            &mut list,
            ccx,
            make_args_map(vec![("status", json!("completed"))]),
        )
        .await
        .unwrap();
        assert!(text(&message).contains(completed.meta.process_id.as_str()));
        assert!(!text(&message).contains(running.meta.process_id.as_str()));
        assert_eq!(exec(&message)["processes"][0]["status"], "exited");
        assert_eq!(
            gcx.exec_registry
                .list(ExecProcessFilter {
                    status: Some(ExecStatusKind::Running),
                    ..ExecProcessFilter::default()
                })
                .await
                .len(),
            1
        );
    }
}
