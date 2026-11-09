use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::exec::ExecRegistry;
use crate::exec::command_policy::{
    build_exec_request, chat_mode_for_exec, escalation_from_args, queue_sandbox_audit, CommandKind,
    prepare_managed_cache, CommandPolicyInput, ExecEscalation, ExecSource, ManagedCacheDecision,
};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::exec::{
    generate_short_description, sanitize_short_description, ExecMode, ExecOutputChunk,
    ExecOutputStream, ExecOwnerMeta, ExecProcessFilter, ExecProcessId, ExecProcessSnapshot,
    ExecReadResult, ExecReadinessProbe, ExecServiceLookup, ExecStatus,
};
use crate::exec::types::{current_timestamp_ms, normalize_workspace_path};
use crate::files_correction::{
    canonical_path, canonicalize_normalized_path, check_if_its_inside_a_workspace_or_config,
    correct_to_nearest_dir_path, get_active_project_path, get_unscoped_project_dirs,
    preprocess_path_for_normalization,
};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::postprocessing::pp_command_output::{
    output_mini_postprocessing, parse_output_filter_args, OutputFilter,
};
use crate::privacy::{check_file_privacy, load_privacy_if_needed, FilePrivacyLevel};
use crate::tools::file_edit::auxiliary::{active_execution_scope, scoped_path_warnings};
use crate::tools::command_checkout::{self, ValidatedCommandCheckout};
use crate::tools::shell_gate;
use crate::tools::tool_shell::exec_status_label;
use crate::tools::tools_description::{
    json_schema_from_params, MatchConfirmDeny, Tool, ToolDesc, ToolSource, ToolSourceType,
};
use crate::worktrees::scope::ExecutionScope;
use refact_core::chat_types::PushMode;

const PROCESS_TRANSCRIPT_MAX_BYTES: usize = 2 * 1024 * 1024;
const DISK_READ_MAX_BYTES: usize = 1024 * 1024;
const TOOL_PTY_ROWS: u16 = 32;
const TOOL_PTY_COLS: u16 = 120;
const TTY_DESCRIPTION: &str = "If true, run the command attached to a pseudo-terminal (PTY). Enables interactive stdin via process_write_stdin and merges stdout+stderr into a single combined stream. Defeats some pipe-only output buffering. Defaults to true, except on Windows, where a pty cannot currently deliver output and pipes are used instead.";

static PATH_ENRICHMENT_CACHE: LazyLock<
    Mutex<HashMap<(String, u64, u64), crate::exec::path_enrichment::CollectedPathEnrichment>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(test)]
tokio::task_local! {
    static CHECKOUT_BEFORE_SPAWN: std::cell::RefCell<Option<Box<dyn FnOnce() + Send>>>;
}

#[cfg(test)]
fn checkout_before_spawn() {
    let _ = CHECKOUT_BEFORE_SPAWN.try_with(|hook| {
        if let Some(change) = hook.borrow_mut().take() {
            change();
        }
    });
}

async fn read_disk_log_tail(path: &std::path::Path) -> Result<(String, bool), String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| format!("failed to open process output from disk: {e}"))?;
    let metadata = file
        .metadata()
        .await
        .map_err(|e| format!("failed to read disk log metadata: {e}"))?;
    let file_size = metadata.len() as usize;
    let truncated = file_size > DISK_READ_MAX_BYTES;
    if truncated {
        let seek_pos = (file_size - DISK_READ_MAX_BYTES) as u64;
        file.seek(std::io::SeekFrom::Start(seek_pos))
            .await
            .map_err(|e| format!("failed to seek disk log: {e}"))?;
    }
    let capacity = DISK_READ_MAX_BYTES.min(file_size);
    let mut buf = Vec::with_capacity(capacity);
    file.read_to_end(&mut buf)
        .await
        .map_err(|e| format!("failed to read process output from disk: {e}"))?;
    Ok((String::from_utf8_lossy(&buf).into_owned(), truncated))
}

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
    escalation: Option<ExecEscalation>,
    push: PushMode,
}

pub(crate) async fn approval_request(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<
    (
        crate::exec::ExecSpawnRequest,
        Option<ValidatedCommandCheckout>,
        Option<ManagedCacheDecision>,
    ),
    String,
> {
    let (mut request, checkout) = approval_request_without_cache(ccx, args).await?;
    let cache = prepare_managed_cache(ccx, checkout.as_ref(), &mut request).await?;
    Ok((request, checkout, cache))
}

pub(crate) async fn approval_request_without_cache(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<
    (
        crate::exec::ExecSpawnRequest,
        Option<ValidatedCommandCheckout>,
    ),
    String,
> {
    let (gcx, scope, chat_id) = {
        let context = ccx.lock().await;
        (
            context.app.gcx.clone(),
            context.execution_scope.clone(),
            context.chat_id.clone(),
        )
    };
    let checkout = command_checkout::resolve(ccx, args).await?;
    let parsed = parse_start_args(gcx.clone(), args, scope.as_ref(), checkout.as_ref()).await?;
    let mut errors = Vec::new();
    let env = crate::integrations::setting_up_integrations::get_vars_for_replacements(
        gcx.clone(),
        &mut errors,
    )
    .await;
    let chat_mode = chat_mode_for_exec(gcx.clone(), &chat_id).await;
    let policy = build_exec_request(
        gcx,
        CommandPolicyInput {
            source: ExecSource::ProcessTool,
            command: CommandKind::Shell(&parsed.command),
            cwd: parsed.workdir,
            env,
            chat_mode,
            escalation: parsed.escalation,
        },
    )
    .await
    .map_err(|error| error.message)?;
    let mut request = policy.request;
    if checkout.is_some() {
        let tty = parsed.tty.unwrap_or(refact_exec::default_tty());
        request.mode = parsed.mode;
        request = request
            .with_tty(tty)
            .with_transcript_limit(PROCESS_TRANSCRIPT_MAX_BYTES);
        if tty {
            request = request.with_pty_size(TOOL_PTY_ROWS, TOOL_PTY_COLS);
        }
        if let Some(wait) = parsed.startup_wait {
            request = request.with_startup_wait(wait);
        }
        if let Some(readiness) = parsed.readiness {
            request = request.with_readiness(readiness);
        }
    }
    Ok((request, checkout))
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
        let (gcx, exec_registry, execution_scope, chat_id, current_model, derived_privacy_zones) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.runtime.exec_registry.clone(),
                ccx_lock.execution_scope.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.current_model.clone(),
                ccx_lock.derived_privacy_zones.clone(),
            )
        };
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let destination = crate::privacy::records::provider_destination(&current_model);
        let observe_reason = crate::privacy::records::shell_observation_reason_for_session(
            &gcx,
            &destination,
            &derived_privacy_zones,
        );
        let observe = observe_reason.needed();
        tracing::debug!(
            "process observation {} for destination {}: {} blocked patterns, {} zones excluding \
             it, {} derived labels",
            if observe { "on" } else { "off" },
            destination.id.0,
            observe_reason.blocked_patterns,
            observe_reason.guarded_zones,
            observe_reason.derived_labels
        );
        let checkout = command_checkout::resolve(&ccx, args).await?;
        let parsed = parse_start_args(
            gcx.clone(),
            args,
            execution_scope.as_ref(),
            checkout.as_ref(),
        )
        .await?;
        let mut error_log = Vec::new();
        let env_variables =
            crate::integrations::setting_up_integrations::get_vars_for_replacements(
                gcx.clone(),
                &mut error_log,
            )
            .await;
        let workspace = if let Some(checkout) = &checkout {
            Some(checkout.root().to_path_buf())
        } else {
            match parsed.workdir.clone() {
                Some(workdir) => workspace_containing_path(gcx.clone(), &workdir)
                    .await
                    .or_else(|| Some(normalize_workspace_path(&workdir))),
                None => process_workspace(gcx.clone(), execution_scope.as_ref()).await,
            }
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
        let tty = parsed.tty.unwrap_or(refact_exec::default_tty());
        let owner = ExecOwnerMeta {
            chat_id: Some(chat_id.clone()),
            tool_call_id: Some(tool_call_id.clone()),
            service_name: parsed.service_name.clone(),
            workspace,
        };
        let chat_mode = chat_mode_for_exec(gcx.clone(), &chat_id).await;
        let policy = match build_exec_request(
            gcx.clone(),
            CommandPolicyInput {
                source: ExecSource::ProcessTool,
                command: CommandKind::Shell(&parsed.command),
                cwd: parsed.workdir.clone(),
                env: env_variables,
                chat_mode,
                escalation: parsed.escalation.clone(),
            },
        )
        .await
        {
            Ok(policy) => policy,
            Err(error) => {
                if let Some(audit) = error.audit {
                    queue_sandbox_audit(gcx.clone(), &chat_id, audit).await;
                }
                return Err(error.message);
            }
        };
        if let Some(audit) = policy.audit.clone() {
            queue_sandbox_audit(gcx.clone(), &chat_id, audit).await;
        }
        let sandbox_active = policy.request.sandbox.is_some();
        let sandbox_warning = policy.warning;
        let mut request = policy.request;
        request.mode = parsed.mode.clone();
        request = request
            .with_owner(owner)
            .with_transcript_limit(PROCESS_TRANSCRIPT_MAX_BYTES)
            .with_short_description(short_description)
            .with_tty(tty)
            .with_observe(observe)
            .with_push(parsed.push)
            .with_chat_spawn_notification();
        if tty {
            request = request.with_pty_size(TOOL_PTY_ROWS, TOOL_PTY_COLS);
        }
        if let Some(startup_wait) = parsed.startup_wait {
            request = request.with_startup_wait(startup_wait);
        }
        if let Some(readiness) = parsed.readiness.clone() {
            request = request.with_readiness(readiness);
        }
        let cache = prepare_managed_cache(&ccx, checkout.as_ref(), &mut request).await?;
        crate::agents::command_approval::enforce(
            &ccx,
            tool_call_id,
            "process_start",
            args,
            &mut request,
            checkout.as_ref(),
            cache.as_ref(),
        )
        .await?;
        if let Some(checkout) = &checkout {
            #[cfg(test)]
            checkout_before_spawn();
            checkout.revalidate(&ccx, args, &request).await?;
        }
        if let Some(cache) = &cache {
            cache.revalidate(&ccx, &request).await?;
        }
        let result = exec_registry.spawn(request).await?;
        let read = exec_registry
            .read(&result.snapshot.meta.process_id, 0, None)
            .await;
        let mut content = format_process_snapshot("Process started", &result.snapshot);
        if let Some(warning) = sandbox_warning {
            content = format!("{warning}\n{content}");
        }
        if !parsed.scope_warnings.is_empty() {
            content.push_str(&format!("\n{}\n", parsed.scope_warnings.join("\n")));
        }
        content.push_str(&format_read_sections(
            &read,
            ProcessStreamSelection::All,
            &OutputFilter::no_limits(),
        ));
        append_sandbox_denial_hint(&mut content, &result.snapshot.status, sandbox_active);
        let mut extra = exec_extra(&result.snapshot, Some(&read), None, Some(tty));
        if let Some(cache) = &cache {
            cache.attach_report(&mut extra);
        }
        attach_exec_path_references(
            &mut extra,
            gcx.clone(),
            &destination,
            &derived_privacy_zones,
            &result.snapshot,
            &parsed.command,
            &collect_combined(&read.chunks),
            read.latest_seq,
        )
        .await;
        let mut result_message = tool_message(
            tool_call_id,
            content,
            Some(extra),
            tool_failed_for_status(&result.snapshot.status),
        );
        if observe {
            let observation = process_observation_status(
                exec_registry.as_ref(),
                &result.snapshot.meta.process_id,
                false,
            )
            .await
            .unwrap_or(result.observation);
            apply_process_privacy(
                &gcx,
                &result.snapshot,
                &destination,
                observation,
                &derived_privacy_zones,
                &mut result_message,
            )
            .await?;
        }
        Ok((false, vec![ContextEnum::ChatMessage(result_message)]))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "process_start".to_string(),
            display_name: "Process Start".to_string(),
            source: source(&self.config_path),
            experimental: false,
            allow_parallel: false,
            description: "Start a runtime-owned background or service process and return its process ID, initial status, output cursor, and metadata. Processes use a pseudo-terminal (PTY) by default, enabling interactive stdin and combining stdout and stderr into a single stream. Set tty=false to use separate pipe streams.".to_string(),
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
        let checkout = command_checkout::resolve(&ccx, args).await?;
        Ok(
            parse_start_args(gcx, args, execution_scope.as_ref(), checkout.as_ref())
                .await?
                .command,
        )
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let command = self
            .command_to_match_against_confirm_deny(ccx.clone(), args)
            .await
            .map_err(|error| format!("Error getting tool command to match: {error}"))?;
        shell_gate::gate_tool_call(ccx, command, args).await
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: shell_gate::default_ask_rules(),
            deny: shell_gate::default_deny_rules(),
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
        let (gcx, exec_registry, execution_scope, chat_id, task_role) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.runtime.exec_registry.clone(),
                ccx_lock.execution_scope.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.task_meta.as_ref().map(|meta| meta.role.clone()),
            )
        };
        let status = parse_list_status(args)?;
        let scope = parse_list_scope(args)?;
        if scope == ProcessListScope::All && task_role.as_deref() != Some("planner") {
            return Err("process_list scope=all requires planner/admin context".to_string());
        }
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
        let (gcx, exec_registry, execution_scope, chat_id, current_model, derived_privacy_zones) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.runtime.exec_registry.clone(),
                ccx_lock.execution_scope.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.current_model.clone(),
                ccx_lock.derived_privacy_zones.clone(),
            )
        };
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let destination = crate::privacy::records::provider_destination(&current_model);
        let process_id = parse_process_id(args)?;
        let snapshot = authorize_process_access(
            gcx.clone(),
            exec_registry.as_ref(),
            execution_scope.as_ref(),
            &chat_id,
            &process_id,
        )
        .await?;
        let since_seq = parse_optional_u64(args, "since_seq")?.unwrap_or(0);
        let stream = parse_stream_selection(args)?;
        let from_disk = parse_optional_bool(args, "from_disk")?.unwrap_or(false);
        let output_filter = parse_output_filter_args(args, &OutputFilter::default());
        if from_disk
            && matches!(
                stream,
                ProcessStreamSelection::Stdout | ProcessStreamSelection::Stderr
            )
        {
            return Err(format!(
                "stream={} is not available with from_disk=true: persisted logs are combined-only; use stream=combined or stream=all",
                stream_label(stream)
            ));
        }
        let observation =
            process_observation_status(exec_registry.as_ref(), &process_id, false).await;
        if matches!(
            observation,
            Some(refact_exec::ObservationStatus::Pending(_))
        ) {
            // Pending reads are not proof that later output is safe. Do not read disk,
            // enrich paths, expose transcript counters or advance the caller's cursor.
            return Ok(tool_result(tool_call_id,
                format!("Process observation pending; output withheld.\nprocess_id: {}\nstatus: {}\nRetry process_read with the same process ID; do not rerun the command.\n", process_id, exec_status_label(&snapshot.status)),
                Some(serde_json::Map::from_iter([("exec".into(), json!({
                    "process_id": process_id.as_str(),
                    "status": exec_status_label(&snapshot.status),
                    "observation_pending": true,
                    "since_seq": since_seq,
                    "next_seq": since_seq,
                }))])), None));
        }
        let read = exec_registry.read(&process_id, since_seq, None).await;
        let mut diagnostic_output = collect_combined(&read.chunks);
        let mut content = format_process_snapshot("Process output", &snapshot);
        content.push_str(&format!(
            "\nsince_seq: {}\nnext_seq: {}\nlatest_seq: {}\n",
            read.since_seq, read.next_seq, read.latest_seq
        ));
        if from_disk {
            if let Some(path) = read.disk_log_path.as_ref() {
                let (text, truncated) = read_disk_log_tail(path).await?;
                diagnostic_output = text.clone();
                content.push_str(&format!(
                    "from_disk: true\npersisted_output_path: {}\n",
                    path.display()
                ));
                if truncated {
                    content.push_str(&format!(
                        "disk_read_truncated: true (showing last {} bytes)\n",
                        DISK_READ_MAX_BYTES
                    ));
                }
                content.push_str(&format_disk_read_sections(&text, stream, &output_filter));
            } else {
                content.push_str(&format_read_sections(&read, stream, &output_filter));
            }
        } else {
            content.push_str(&format_read_sections(&read, stream, &output_filter));
        }
        let mut extra = exec_extra(&snapshot, Some(&read), Some(stream_label(stream)), None);
        attach_exec_path_references(
            &mut extra,
            gcx.clone(),
            &destination,
            &derived_privacy_zones,
            &snapshot,
            &snapshot.meta.command,
            &diagnostic_output,
            read.latest_seq,
        )
        .await;
        let mut result_message = tool_message(tool_call_id, content, Some(extra), None);
        if let Some(observation) = observation {
            apply_process_privacy(
                &gcx,
                &snapshot,
                &destination,
                observation,
                &derived_privacy_zones,
                &mut result_message,
            )
            .await?;
        }
        Ok((false, vec![ContextEnum::ChatMessage(result_message)]))
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
                    ("from_disk", "boolean", "Optional. If true and persisted output exists, read the full on-disk transcript instead of the in-memory ring."),
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
        let (gcx, exec_registry, execution_scope, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.runtime.exec_registry.clone(),
                ccx_lock.execution_scope.clone(),
                ccx_lock.chat_id.clone(),
            )
        };
        let process_id = parse_process_id(args)?;
        authorize_process_access(
            gcx,
            exec_registry.as_ref(),
            execution_scope.as_ref(),
            &chat_id,
            &process_id,
        )
        .await?;
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

pub(crate) async fn execute_process_wait(
    ccx: Arc<AMutex<AtCommandsContext>>,
    tool_call_id: &String,
    args: &HashMap<String, Value>,
) -> Result<(bool, Vec<ContextEnum>), String> {
    let (gcx, exec_registry, execution_scope, chat_id, current_model, derived_privacy_zones) = {
        let ccx_lock = ccx.lock().await;
        (
            ccx_lock.app.gcx.clone(),
            ccx_lock.app.runtime.exec_registry.clone(),
            ccx_lock.execution_scope.clone(),
            ccx_lock.chat_id.clone(),
            ccx_lock.current_model.clone(),
            ccx_lock.derived_privacy_zones.clone(),
        )
    };
    crate::privacy::load_privacy_if_needed(gcx.clone()).await;
    let destination = crate::privacy::records::provider_destination(&current_model);
    let process_id = parse_process_id(args)?;
    authorize_process_access(
        gcx.clone(),
        exec_registry.as_ref(),
        execution_scope.as_ref(),
        &chat_id,
        &process_id,
    )
    .await?;
    let timeout_ms = parse_optional_u64(args, "timeout_ms")?;
    let interrupt = crate::tools::tools_description::WaitInterrupt::from_context(&ccx).await;
    let wait = async {
        if let Some(timeout_ms) = timeout_ms {
            match tokio::time::timeout(
                Duration::from_millis(timeout_ms),
                exec_registry.wait(&process_id),
            )
            .await
            {
                Ok(result) => result.map(|snapshot| (snapshot, false)),
                Err(_) => authorize_process_access(
                    gcx.clone(),
                    exec_registry.as_ref(),
                    execution_scope.as_ref(),
                    &chat_id,
                    &process_id,
                )
                .await
                .map(|snapshot| (snapshot, true)),
            }
        } else {
            exec_registry
                .wait(&process_id)
                .await
                .map(|snapshot| (snapshot, false))
        }
    };
    let (snapshot, timed_out, interrupted) = tokio::select! {
        biased;
        _ = interrupt.wait() => {
            let snapshot = authorize_process_access(gcx.clone(), exec_registry.as_ref(), execution_scope.as_ref(), &chat_id, &process_id).await?;
            (snapshot, false, true)
        }
        result = wait => { let (snapshot, timed_out) = result?; (snapshot, timed_out, false) },
    };
    if matches!(
        process_observation_status(exec_registry.as_ref(), &process_id, false).await,
        Some(refact_exec::ObservationStatus::Pending(_))
    ) {
        return Ok(tool_result(tool_call_id,
            json!({"outcome": "pending", "target": "process", "process_id": process_id,
                "evidence": "Process observation pending; output withheld. Retry with the same process ID."}).to_string(),
            None, None));
    }
    let read = exec_registry.read(&process_id, 0, None).await;
    let (title, timed_out) = process_wait_outcome(&snapshot.status, timed_out);
    let outcome = if interrupted {
        "interrupted"
    } else if timed_out {
        "timed_out"
    } else {
        "completed"
    };
    let mut content = format!(
        "outcome: {outcome}\ntarget: process\n{}",
        format_process_snapshot(title, &snapshot)
    );
    content.push_str(&format!(
        "\nnext_seq: {}\nlatest_seq: {}\n",
        read.next_seq, read.latest_seq
    ));
    content.push_str(&format_read_sections(
        &read,
        ProcessStreamSelection::All,
        &OutputFilter::no_limits(),
    ));
    let mut extra = exec_extra(&snapshot, Some(&read), None, None);
    if let Some(exec) = extra.get_mut("exec").and_then(Value::as_object_mut) {
        exec.insert("wait_timed_out".to_string(), Value::Bool(timed_out));
    }
    attach_exec_path_references(
        &mut extra,
        gcx.clone(),
        &destination,
        &derived_privacy_zones,
        &snapshot,
        &snapshot.meta.command,
        &collect_combined(&read.chunks),
        read.latest_seq,
    )
    .await;
    let mut result_message = tool_message(
        tool_call_id,
        content,
        Some(extra),
        tool_failed_for_status(&snapshot.status),
    );
    if let Some(observation) = process_observation_status(
        exec_registry.as_ref(),
        &process_id,
        snapshot.status.is_terminal(),
    )
    .await
    {
        apply_process_privacy(
            &gcx,
            &snapshot,
            &destination,
            observation,
            &derived_privacy_zones,
            &mut result_message,
        )
        .await?;
    }
    let evidence = result_message.content.content_text_only();
    result_message.content = ChatContent::SimpleText(
        json!({
            "outcome": outcome,
            "target": "process",
            "process_id": process_id,
            "evidence": evidence,
        })
        .to_string(),
    );
    Ok((false, vec![ContextEnum::ChatMessage(result_message)]))
}

fn process_wait_outcome(status: &ExecStatus, caller_timed_out: bool) -> (&'static str, bool) {
    if caller_timed_out && !status.is_terminal() {
        ("Process still running", true)
    } else {
        ("Process wait completed", false)
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

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let command = self
            .command_to_match_against_confirm_deny(ccx.clone(), args)
            .await
            .map_err(|error| format!("Error getting tool command to match: {error}"))?;
        shell_gate::gate_tool_call(ccx, command, args).await
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: shell_gate::default_ask_rules(),
            deny: shell_gate::default_deny_rules(),
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
    schema["properties"]["checkout"] = command_checkout::input_schema();
    schema["properties"]["tty"] = json!({
        "type": "boolean",
        "default": refact_exec::default_tty(),
        "description": TTY_DESCRIPTION,
    });
    schema["properties"]["needs_confirmation"] = json!({
        "type": "boolean",
        "default": false,
        "description": "Set true when you know this command is destructive, irreversible, or touches something outside the workspace, and you want the user to confirm before it runs. Setting false never skips the normal safety checks."
    });
    schema["properties"]["escalate"] = json!({
        "type": "object",
        "properties": {
            "mode": {"type": "string", "enum": ["workspace_write", "full_access"]},
            "justification": {"type": "string"}
        },
        "required": ["mode", "justification"],
        "additionalProperties": false,
        "description": "Request a per-call sandbox escalation. Only has an effect when a sandbox is actually enforced; must explain why the wider access is needed."
    });
    // The process runs asynchronously, so its completion notice is a delivery
    // into the chat rather than this call's tool result; `push` picks when that
    // notice lands.
    schema["properties"]["push"] = PushMode::schema();
    schema
}

fn source(config_path: &str) -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: config_path.to_string(),
    }
}

fn tool_message(
    tool_call_id: &String,
    content: String,
    extra: Option<serde_json::Map<String, Value>>,
    tool_failed: Option<bool>,
) -> ChatMessage {
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
    message
}

fn tool_result(
    tool_call_id: &String,
    content: String,
    extra: Option<serde_json::Map<String, Value>>,
    tool_failed: Option<bool>,
) -> (bool, Vec<ContextEnum>) {
    (
        false,
        vec![ContextEnum::ChatMessage(tool_message(
            tool_call_id,
            content,
            extra,
            tool_failed,
        ))],
    )
}

async fn process_observation_status(
    exec_registry: &ExecRegistry,
    process_id: &ExecProcessId,
    terminal: bool,
) -> Option<refact_exec::ObservationStatus> {
    let Some(reader) = exec_registry.observation_reader(process_id).await else {
        return None;
    };
    Some(if terminal {
        reader.wait_status().await
    } else {
        reader.status()
    })
}

async fn apply_process_privacy(
    gcx: &Arc<GlobalContext>,
    snapshot: &ExecProcessSnapshot,
    destination: &refact_privacy::Destination,
    observation: refact_exec::ObservationStatus,
    derived_zones: &crate::privacy::records::DerivedPrivacyZones,
    message: &mut ChatMessage,
) -> Result<(), String> {
    let cwd = snapshot
        .meta
        .cwd
        .as_deref()
        .or(snapshot.meta.owner.workspace.as_deref())
        .unwrap_or_else(|| Path::new("."));
    match crate::privacy::records::apply_shell_observation(
        gcx,
        &snapshot.meta.command,
        cwd,
        destination,
        observation,
        derived_zones,
        message,
    )
    .await?
    {
        crate::privacy::records::ShellReadDecision::Pass => Ok(()),
        crate::privacy::records::ShellReadDecision::Ask => Ok(()),
    }
}

fn append_sandbox_denial_hint(content: &mut String, status: &ExecStatus, sandbox_active: bool) {
    if !sandbox_active {
        return;
    }
    if matches!(status, ExecStatus::SandboxLauncherFailed { .. }) {
        content.push_str("Sandbox confinement could not start; the command did not run.\n");
        return;
    }
    let lower = content.to_ascii_lowercase();
    // Command output cannot prove a sandbox denial; this only supplements the typed launcher status.
    if lower.contains("permission denied")
        || lower.contains("operation not permitted")
        || lower.contains("eacces")
        || lower.contains("eperm")
    {
        content.push_str("Command output may indicate a sandbox denial. Retry with escalate:{mode, justification} if wider access is necessary.\n");
    }
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
        Some(value) => refact_tool_api::coerce_bool(value)
            .map(Some)
            .ok_or_else(|| format!("argument `{name}` is not a boolean: {value:?}")),
        None => Ok(None),
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
    checkout: Option<&ValidatedCommandCheckout>,
) -> Result<ProcessStartArgs, String> {
    let command = parse_required_string(args, "command")?;
    let mode = parse_start_mode(args)?;
    let service_name = parse_optional_string(args, "service_name")?;
    let description = parse_optional_string(args, "description")?.ok_or_else(|| {
        "description is required and must be a non-empty single concise sentence".to_string()
    })?;
    let raw_workdir = parse_optional_string(args, "workdir")?;
    let (workdir, scope_warnings) = if let Some(checkout) = checkout {
        (Some(checkout.cwd().to_path_buf()), Vec::new())
    } else {
        if args.contains_key("checkout") {
            return Err("command checkout requires validated authority".into());
        }
        resolve_process_workdir(gcx, raw_workdir.as_deref(), execution_scope).await?
    };
    let startup_wait = parse_optional_u64(args, "startup_wait_ms")?.map(Duration::from_millis);
    let wait_port = parse_optional_u16(args, "startup_wait_port")?;
    let wait_keyword = parse_optional_string(args, "startup_wait_keyword")?;
    let tty = parse_optional_bool(args, "tty")?;
    parse_optional_bool(args, "needs_confirmation")?;
    let escalation = escalation_from_args(args)?;
    let push = PushMode::from_args(args)?;
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
        escalation,
        push,
    })
}

async fn resolve_process_workdir(
    gcx: Arc<GlobalContext>,
    raw_path: Option<&str>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<(Option<PathBuf>, Vec<String>), String> {
    if let Some(scope) = execution_scope {
        scope.ensure_active_root()?;
    }
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
            "⚠️ Worktree scope: process cwd/workdir is enforced as '{}'; OS confinement follows the terminal security mode",
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
        let project_dirs = get_unscoped_project_dirs(gcx.clone()).await;
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
    get_unscoped_project_dirs(gcx)
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

async fn authorize_process_access(
    gcx: Arc<GlobalContext>,
    registry: &crate::exec::ExecRegistry,
    execution_scope: Option<&ExecutionScope>,
    chat_id: &str,
    process_id: &ExecProcessId,
) -> Result<ExecProcessSnapshot, String> {
    let workspace = process_workspace(gcx, execution_scope).await;
    registry
        .authorize_process_access(process_id, chat_id, workspace.as_deref())
        .await
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

pub(super) fn status_label(status: &ExecStatus) -> &'static str {
    match status {
        ExecStatus::Starting => "starting",
        ExecStatus::Running => "running",
        ExecStatus::Exited {
            ..
        } => "exited",
        ExecStatus::SandboxLauncherFailed {
            ..
        } => "sandbox_launcher_failed",
        ExecStatus::Failed {
            ..
        } => "failed",
        ExecStatus::Killed => "killed",
        ExecStatus::TimedOut => "timed_out",
    }
}

fn exit_code(status: &ExecStatus) -> Option<i32> {
    match status {
        ExecStatus::Exited {
            exit_code,
        } => *exit_code,
        ExecStatus::SandboxLauncherFailed {
            exit_code,
        } => Some(*exit_code),
        ExecStatus::Starting
        | ExecStatus::Running
        | ExecStatus::Failed {
            ..
        }
        | ExecStatus::Killed
        | ExecStatus::TimedOut => None,
    }
}

fn tool_failed_for_status(status: &ExecStatus) -> Option<bool> {
    match status {
        ExecStatus::SandboxLauncherFailed {
            ..
        }
        | ExecStatus::Failed {
            ..
        }
        | ExecStatus::Killed
        | ExecStatus::TimedOut => Some(true),
        ExecStatus::Starting
        | ExecStatus::Running
        | ExecStatus::Exited {
            ..
        } => None,
    }
}

pub(super) fn process_value(snapshot: &ExecProcessSnapshot) -> Value {
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
    let started_at_ms = snapshot
        .meta
        .started_at_ms
        .unwrap_or(snapshot.meta.created_at_ms);
    let duration_ms = snapshot
        .meta
        .ended_at_ms
        .unwrap_or_else(current_timestamp_ms)
        .saturating_sub(started_at_ms);
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
        "duration_ms": duration_ms,
        "exit_code": exit_code(&snapshot.status),
        "persisted_output_path": snapshot.disk_log_path.as_ref().map(|path| path.to_string_lossy().to_string()),
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

async fn attach_exec_path_references(
    extra: &mut serde_json::Map<String, Value>,
    gcx: Arc<GlobalContext>,
    destination: &refact_privacy::Destination,
    derived_privacy_zones: &crate::privacy::records::DerivedPrivacyZones,
    snapshot: &ExecProcessSnapshot,
    command: &str,
    output: &str,
    latest_seq: u64,
) {
    let cwd = snapshot
        .meta
        .cwd
        .as_deref()
        .or(snapshot.meta.owner.workspace.as_deref())
        .unwrap_or_else(|| Path::new("."));
    let workspace = snapshot.meta.owner.workspace.as_deref().unwrap_or(cwd);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    output.hash(&mut hasher);
    let key = (
        snapshot.meta.process_id.as_str().to_string(),
        latest_seq,
        hasher.finish(),
    );
    let cached = PATH_ENRICHMENT_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&key)
        .cloned();
    let collected = if let Some(collected) = cached {
        collected
    } else {
        let collected = crate::exec::path_enrichment::collect_async(
            command,
            cwd,
            &[workspace.to_path_buf()],
            output,
        )
        .await;
        let mut cache = PATH_ENRICHMENT_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.len() >= 128 {
            cache.clear();
        }
        cache.insert(key, collected.clone());
        collected
    };
    let enrichment = crate::privacy::records::filter_path_enrichment_for_model_context(
        gcx,
        destination,
        derived_privacy_zones,
        collected,
    )
    .await;
    refact_chat_api::attach_tool_enrichment_to_extra(
        extra,
        crate::privacy::records::tool_enrichment_from_path_references(enrichment),
    );
}

pub(super) fn read_value(read: &ExecReadResult) -> Value {
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
        "persisted_output_path": read.disk_log_path.as_ref().map(|path| path.to_string_lossy().to_string()),
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
        ExecStatus::SandboxLauncherFailed {
            ..
        } => Some("sandbox confinement could not start".to_string()),
        ExecStatus::Failed {
            message,
        } => Some(message.clone()),
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
            let combined = collect_stream(&read.chunks, ExecOutputStream::Combined);
            if !combined.is_empty() {
                append_section(&mut out, "combined", &combined, output_filter);
            }
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

fn format_disk_read_sections(
    text: &str,
    _selection: ProcessStreamSelection,
    output_filter: &OutputFilter,
) -> String {
    let mut out = String::new();
    append_section(&mut out, "combined", text, output_filter);
    out
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
    use crate::exec::spill::SpillTarget;
    use crate::exec::{ExecProcessMeta, ExecStatusKind};

    async fn test_ccx() -> (Arc<GlobalContext>, Arc<AMutex<AtCommandsContext>>) {
        test_ccx_with_chat("chat").await
    }

    async fn test_ccx_with_chat(
        chat_id: &str,
    ) -> (Arc<GlobalContext>, Arc<AMutex<AtCommandsContext>>) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        (gcx.clone(), test_ccx_for_gcx(gcx, chat_id, None).await)
    }

    async fn test_ccx_for_gcx(
        gcx: Arc<GlobalContext>,
        chat_id: &str,
        task_role: Option<&str>,
    ) -> Arc<AMutex<AtCommandsContext>> {
        let ccx = AtCommandsContext::new_with_abort(
            AppState::from_gcx(gcx).await,
            4096,
            20,
            false,
            Vec::new(),
            chat_id.to_string(),
            None,
            "model".to_string(),
            task_role.map(task_meta),
            None,
            None,
        )
        .await;
        Arc::new(AMutex::new(ccx))
    }

    fn task_meta(role: &str) -> crate::chat::types::TaskMeta {
        crate::chat::types::TaskMeta {
            task_id: "task".to_string(),
            role: role.to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: None,
        }
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

    async fn run_process_wait(
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: HashMap<String, Value>,
    ) -> Result<ChatMessage, String> {
        let (_, messages) = execute_process_wait(ccx, &"tool_call".to_string(), &args).await?;
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

    fn wait_completion_timeout_ms() -> u64 {
        if cfg!(target_os = "windows") {
            60_000
        } else {
            2_000
        }
    }

    fn quick_command(output: &str) -> String {
        if cfg!(target_os = "windows") {
            format!("[Console]::Out.Write('{output}')")
        } else {
            format!("printf {output:?}")
        }
    }

    fn delayed_quick_command(output: &str) -> String {
        if cfg!(target_os = "windows") {
            format!("Start-Sleep -Milliseconds 20; [Console]::Out.Write('{output}')")
        } else {
            format!("sleep 0.02; printf {output:?}")
        }
    }

    async fn wait_for_output(gcx: Arc<GlobalContext>, process_id: &ExecProcessId, needle: &str) {
        for _ in 0..40 {
            let read = gcx.exec_registry.read(process_id, 0, None).await;
            let transcript = read
                .chunks
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<String>();
            if transcript.contains(needle) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("output did not contain {needle}");
    }

    async fn register_test_process(
        gcx: &GlobalContext,
        process_id: &str,
        chat_id: &str,
        workspace: Option<PathBuf>,
    ) -> ExecProcessId {
        let mut meta = ExecProcessMeta::new(ExecMode::Background, "test".to_string())
            .with_process_id(ExecProcessId(process_id.to_string()))
            .with_chat_id(chat_id)
            .with_short_description("Authorization test".to_string());
        if let Some(workspace) = workspace {
            meta = meta.with_workspace(workspace);
        }
        let snapshot = gcx
            .exec_registry
            .register(meta, PROCESS_TRANSCRIPT_MAX_BYTES)
            .await;
        snapshot.meta.process_id
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
            refact_exec::default_tty()
        );
        assert_eq!(
            start_desc.input_schema["properties"]["tty"]["description"],
            TTY_DESCRIPTION
        );
        assert_eq!(
            start_desc.input_schema["properties"]["escalate"]["properties"]["mode"]["enum"],
            json!(["workspace_write", "full_access"])
        );
        assert!(start_desc
            .description
            .contains("combining stdout and stderr"));
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
            "wait",
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
        assert_eq!(exec(&message)["tty"], refact_exec::default_tty());
        let snapshot = gcx.exec_registry.get(&process_id).await.unwrap();
        assert_eq!(snapshot.meta.tty, refact_exec::default_tty());
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

    #[cfg(unix)]
    #[tokio::test]
    async fn process_start_defaults_to_120_by_32_pty_for_background_and_service() {
        let (gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };

        let background = run_tool(
            &mut start,
            ccx.clone(),
            make_args_map(vec![
                ("command", json!("stty size; sleep 30")),
                ("description", json!("Report background terminal size")),
                ("startup_wait_ms", json!(2000)),
                ("startup_wait_keyword", json!("32 120")),
            ]),
        )
        .await
        .unwrap();
        let background_id = process_id(&background);
        wait_for_output(gcx.clone(), &background_id, "32 120").await;
        assert_eq!(exec(&background)["tty"], refact_exec::default_tty());
        assert!(text(&background).contains("\ncombined:\n32 120\n"));
        let transcript = gcx.exec_registry.read(&background_id, 0, None).await;
        assert!(!transcript.chunks.is_empty());
        assert!(transcript
            .chunks
            .iter()
            .all(|chunk| chunk.stream == ExecOutputStream::Combined));
        let mut read = ToolProcessRead {
            config_path: String::new(),
        };
        let output = run_tool(
            &mut read,
            ccx.clone(),
            make_args_map(vec![
                ("process_id", json!(background_id.as_str())),
                ("stream", json!("all")),
                ("output_limit", json!("all")),
            ]),
        )
        .await
        .unwrap();
        assert!(text(&output).contains("\ncombined:\n32 120\n"));
        let waited = run_process_wait(
            ccx.clone(),
            make_args_map(vec![
                ("process_id", json!(background_id.as_str())),
                ("timeout_ms", json!(0)),
            ]),
        )
        .await
        .unwrap();
        let waited: Value = serde_json::from_str(&text(&waited)).unwrap();
        assert!(waited["evidence"]
            .as_str()
            .unwrap()
            .contains("\ncombined:\n32 120\n"));
        gcx.exec_registry.kill(&background_id).await.unwrap();

        let service = run_tool(
            &mut start,
            ccx,
            make_args_map(vec![
                ("command", json!("stty size; sleep 30")),
                ("description", json!("Report service terminal size")),
                ("mode", json!("service")),
                ("service_name", json!("pty-size")),
            ]),
        )
        .await
        .unwrap();
        let service_id = process_id(&service);
        wait_for_output(gcx.clone(), &service_id, "32 120").await;
        assert_eq!(exec(&service)["tty"], refact_exec::default_tty());
        gcx.exec_registry.kill(&service_id).await.unwrap();
    }

    #[tokio::test]
    async fn process_start_explicit_tty_false_uses_pipes() {
        let (gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let message = run_tool(
            &mut start,
            ccx,
            make_args_map(vec![
                ("command", json!(long_running_command("pipe-ready"))),
                ("description", json!("Run background process through pipes")),
                ("tty", json!(false)),
            ]),
        )
        .await
        .unwrap();
        let process_id = process_id(&message);
        let snapshot = gcx.exec_registry.get(&process_id).await.unwrap();

        assert_eq!(exec(&message)["tty"], false);
        assert!(!snapshot.meta.tty);
        assert_eq!(snapshot.meta.mode, ExecMode::Background);
        gcx.exec_registry.kill(&process_id).await.unwrap();
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
    async fn process_read_denies_cross_chat_process() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let owner_ccx = test_ccx_for_gcx(gcx.clone(), "chat-a", None).await;
        let other_ccx = test_ccx_for_gcx(gcx.clone(), "chat-b", None).await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let started = run_tool(
            &mut start,
            owner_ccx.clone(),
            make_args_map(vec![
                ("command", json!(long_running_command("secret"))),
                ("description", json!("Run owned process")),
                ("startup_wait_ms", json!(100)),
            ]),
        )
        .await
        .unwrap();
        let process_id = process_id(&started);

        let mut read = ToolProcessRead {
            config_path: String::new(),
        };
        let err = run_tool(
            &mut read,
            other_ccx,
            make_args_map(vec![("process_id", json!(process_id.as_str()))]),
        )
        .await
        .unwrap_err();
        assert_eq!(err, format!("process access denied: {process_id}"));

        let output = run_tool(
            &mut read,
            owner_ccx.clone(),
            make_args_map(vec![("process_id", json!(process_id.as_str()))]),
        )
        .await
        .unwrap();
        assert_eq!(exec(&output)["process_id"], process_id.as_str());

        gcx.exec_registry.kill(&process_id).await.unwrap();
    }

    #[tokio::test]
    async fn process_kill_denies_cross_chat_process() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let owner_ccx = test_ccx_for_gcx(gcx.clone(), "chat-a", None).await;
        let other_ccx = test_ccx_for_gcx(gcx.clone(), "chat-b", None).await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let started = run_tool(
            &mut start,
            owner_ccx.clone(),
            make_args_map(vec![
                ("command", json!(long_running_command("kill-secret"))),
                ("description", json!("Run process for kill auth")),
                ("startup_wait_ms", json!(100)),
            ]),
        )
        .await
        .unwrap();
        let process_id = process_id(&started);

        let mut kill = ToolProcessKill {
            config_path: String::new(),
        };
        let err = run_tool(
            &mut kill,
            other_ccx,
            make_args_map(vec![("process_id", json!(process_id.as_str()))]),
        )
        .await
        .unwrap_err();
        assert_eq!(err, format!("process access denied: {process_id}"));
        assert_eq!(
            gcx.exec_registry.get(&process_id).await.unwrap().status,
            ExecStatus::Running
        );

        let killed = run_tool(
            &mut kill,
            owner_ccx,
            make_args_map(vec![("process_id", json!(process_id.as_str()))]),
        )
        .await
        .unwrap();
        assert_eq!(exec(&killed)["status"], "killed");
    }

    #[tokio::test]
    async fn process_wait_denies_cross_chat_process() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let owner_ccx = test_ccx_for_gcx(gcx.clone(), "chat-a", None).await;
        let other_ccx = test_ccx_for_gcx(gcx.clone(), "chat-b", None).await;
        let process_id = register_test_process(&gcx, "exec_wait_owned", "chat-a", None).await;
        gcx.exec_registry
            .mark_exited(&process_id, Some(0))
            .await
            .unwrap();

        let err = run_process_wait(
            other_ccx,
            make_args_map(vec![("process_id", json!(process_id.as_str()))]),
        )
        .await
        .unwrap_err();
        assert_eq!(err, format!("process access denied: {process_id}"));

        let completed = run_process_wait(
            owner_ccx,
            make_args_map(vec![("process_id", json!(process_id.as_str()))]),
        )
        .await
        .unwrap();
        assert_eq!(exec(&completed)["status"], "exited");
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

        let timed_out = run_process_wait(
            ccx.clone(),
            make_args_map(vec![
                ("process_id", json!(slow_process_id.as_str())),
                ("timeout_ms", json!(50)),
            ]),
        )
        .await
        .unwrap();
        assert!(text(&timed_out).contains("Process still running"));
        assert_eq!(exec(&timed_out)["status"], "running");
        assert_eq!(exec(&timed_out)["wait_timed_out"], true);

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
        let completed = run_process_wait(
            ccx,
            make_args_map(vec![
                ("process_id", json!(done_process_id.as_str())),
                ("timeout_ms", json!(wait_completion_timeout_ms())),
            ]),
        )
        .await
        .unwrap();
        assert!(text(&completed).contains("Process wait completed"));
        assert_eq!(exec(&completed)["status"], "exited");
        assert_eq!(exec(&completed)["exit_code"], 0);
        assert_eq!(exec(&completed)["wait_timed_out"], false);
    }

    #[test]
    fn tool_process_kill_has_no_confirmation_rules() {
        let kill = ToolProcessKill {
            config_path: String::new(),
        };
        assert!(kill.confirm_deny_rules().is_none());
    }

    struct CheckoutFixture {
        _temp: tempfile::TempDir,
        app: AppState,
        ccx: Arc<AMutex<AtCommandsContext>>,
        source: PathBuf,
        root: PathBuf,
        selector: Value,
    }

    struct CheckoutAttempt {
        ccx: Arc<AMutex<AtCommandsContext>>,
        admission: crate::tasks::admission::Attempt,
    }

    impl Drop for CheckoutAttempt {
        fn drop(&mut self) {
            self.admission.end();
        }
    }

    impl CheckoutFixture {
        async fn new() -> Self {
            use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
            use crate::chat::types::{ChatSession, TaskMeta};
            use crate::worktrees::types::CreateWorktreeRequest;
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("repo");
            std::fs::create_dir(&source).unwrap();
            let git = |args: &[&str]| crate::worktrees::git::run_git(&source, args).unwrap();
            git(&["init"]);
            git(&["checkout", "-b", "main"]);
            git(&["config", "user.name", "Test User"]);
            git(&["config", "user.email", "test@example.com"]);
            std::fs::write(source.join("file"), "base").unwrap();
            std::fs::create_dir_all(source.join("refact-agent/engine")).unwrap();
            std::fs::write(
                source.join("refact-agent/engine/Cargo.toml"),
                "[workspace]\n",
            )
            .unwrap();
            git(&["add", "file", "refact-agent/engine/Cargo.toml"]);
            git(&["commit", "-m", "base"]);
            let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
                temp.path().join("cache"),
                temp.path().join("config"),
            )
            .await;
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
            crate::privacy::save_privacy_policy(gcx.clone(), Default::default())
                .await
                .unwrap();
            let service = crate::worktrees::service::WorktreeService::new(
                gcx.cache_dir.clone(),
                source.clone(),
            )
            .unwrap();
            let view = service
                .create_worktree(CreateWorktreeRequest {
                    kind: Some("task_agent".into()),
                    task_id: Some("task".into()),
                    card_id: Some("T-1".into()),
                    agent_id: Some("worker".into()),
                    chat_id: Some("worker-chat".into()),
                    branch: Some("refact/task/task/card/T-1/worker".into()),
                    base_branch: Some("main".into()),
                    ..Default::default()
                })
                .await
                .unwrap()
                .worktree;
            let root = view.meta.root.clone();
            std::fs::create_dir(root.join("child")).unwrap();
            let board: crate::tasks::types::TaskBoard = serde_json::from_value(json!({
                "rev":7,"owner_chat_id":"planner","cards":[{
                    "id":"T-1","title":"Checkout","column":"done","created_at":"now",
                    "assignee":"worker","agent_chat_id":"worker-chat",
                    "agent_worktree":root,"agent_worktree_name":view.meta.id,
                    "agent_branch":view.meta.branch
                }]
            }))
            .unwrap();
            let task_dir = source.join(".refact/tasks/task");
            std::fs::create_dir_all(&task_dir).unwrap();
            std::fs::write(
                task_dir.join("board.yaml"),
                serde_yaml::to_string(&board).unwrap(),
            )
            .unwrap();
            let task = TaskMeta {
                task_id: "task".into(),
                role: "planner".into(),
                ..Default::default()
            };
            let app = AppState::from_gcx(gcx.clone()).await;
            let mut parent = ChatSession::new("planner".into());
            parent.thread.task_meta = Some(task.clone());
            parent.thread.mode = "task_planner".into();
            parent.thread.model = "model".into();
            parent.work_project_root = Some(source.clone());
            let mut child = ChatSession::new("worker-chat".into());
            child.thread = crate::tools::tool_task_spawn_agent::build_agent_thread_params(
                "worker-chat",
                "Checkout",
                "model",
                "task",
                "worker",
                "T-1",
                "planner",
                view.meta.clone(),
            );
            child.runtime.state = refact_runtime_api::SessionState::Completed;
            child
                .queue_processor_running
                .store(true, std::sync::atomic::Ordering::Relaxed);
            {
                let mut sessions = app.chat.sessions.write().await;
                sessions.insert("planner".into(), Arc::new(AMutex::new(parent)));
                sessions.insert("worker-chat".into(), Arc::new(AMutex::new(child)));
            }
            let (defaults_root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
            let mut caps = CodeAssistantCaps::default();
            caps.chat_models.insert(
                "model".into(),
                Arc::new(ChatModelRecord {
                    base: BaseModelRecord {
                        id: "model".into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
            {
                let mut state = gcx.caps_state.write().await;
                state.caps = Some(Arc::new(caps));
                state.loaded_project_root = Some(defaults_root);
                state.last_attempted_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
            }
            let ccx = Arc::new(AMutex::new(
                AtCommandsContext::new_from_app(
                    app.clone(),
                    4096,
                    20,
                    false,
                    vec![],
                    "planner".into(),
                    None,
                    "model".into(),
                    Some(task),
                    None,
                )
                .await,
            ));
            let selector = json!({
                "kind":"task_card","card_id":"T-1","expected_board_rev":7,
                "expected_agent_chat_id":"worker-chat","expected_worktree_id":view.meta.id,
                "expected_head":view.status.head_commit,"expected_cwd":root,
            });
            Self {
                _temp: temp,
                app,
                ccx,
                source,
                root,
                selector,
            }
        }

        async fn retained_attempt(&self) -> CheckoutAttempt {
            let creator = self.app.chat.sessions.read().await["worker-chat"].clone();
            let worktree = creator.lock().await.thread.worktree.clone().unwrap();
            let chat = format!("retained-cache-{}", uuid::Uuid::new_v4());
            let agent = format!("retained-agent-{chat}");
            let thread = crate::tools::tool_task_spawn_agent::build_agent_thread_params(
                &chat,
                "Retained checkout",
                "model",
                "task",
                &agent,
                "T-1",
                "planner",
                worktree.clone(),
            );
            assert!(!worktree.agent_id.as_deref().unwrap().is_empty());
            assert!(!thread
                .task_meta
                .as_ref()
                .unwrap()
                .agent_id
                .as_deref()
                .unwrap()
                .is_empty());
            assert_ne!(
                thread.task_meta.as_ref().unwrap().agent_id,
                worktree.agent_id
            );
            crate::chat::worktree_lifecycle::check_managed_attachment(&self.app, &chat, &thread)
                .await
                .unwrap();
            self.change_board(|board| {
                let card = board.get_card_mut("T-1").unwrap();
                card.column = "doing".into();
                card.assignee = Some(agent);
                card.agent_chat_id = Some(chat.clone());
            })
            .await;
            let mut session = crate::chat::types::ChatSession::new(chat.clone());
            session.thread = thread.clone();
            session.work_project_root = Some(self.source.clone());
            session
                .queue_processor_running
                .store(true, std::sync::atomic::Ordering::Relaxed);
            self.app
                .chat
                .sessions
                .write()
                .await
                .insert(chat.clone(), Arc::new(AMutex::new(session)));
            let ccx = Arc::new(AMutex::new(
                AtCommandsContext::new_from_app(
                    self.app.clone(),
                    4096,
                    20,
                    false,
                    vec![],
                    chat.clone(),
                    thread.root_chat_id,
                    thread.model,
                    thread.task_meta,
                    thread.worktree,
                )
                .await,
            ));
            crate::tasks::admission::reserve(
                &self.app,
                &chat,
                &crate::coordination::roots::project_identity(&self.source),
                "planner",
                0,
            )
            .await
            .unwrap();
            let attempt = CheckoutAttempt {
                ccx,
                admission: crate::tasks::admission::current_attempt(&chat).unwrap(),
            };
            tokio::time::timeout(
                Duration::from_secs(5),
                crate::tasks::admission::wait_for_permit(&chat),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(crate::tasks::admission::current_execution(&self.app, &chat)
                .unwrap()
                .holds_permit());
            assert_eq!(
                attempt.ccx.lock().await.execution_scope_worktree(),
                Some(worktree)
            );
            attempt
        }

        fn command_args(&self) -> HashMap<String, Value> {
            make_args_map(vec![
                (
                    "command",
                    json!(if cfg!(windows) {
                        "[Console]::Out.Write((Get-Location).Path)"
                    } else {
                        "pwd -P"
                    }),
                ),
                ("description", json!("Report checkout cwd")),
                ("checkout", self.selector.clone()),
                ("tty", json!(false)),
            ])
        }

        async fn change_board(&self, change: impl FnOnce(&mut crate::tasks::types::TaskBoard)) {
            let path = self.source.join(".refact/tasks/task/board.yaml");
            let mut board = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            change(&mut board);
            std::fs::write(path, serde_yaml::to_string(&board).unwrap()).unwrap();
        }

        async fn assert_refused(&self, args: HashMap<String, Value>, expected: &str) {
            let mut tool = ToolProcessStart {
                config_path: String::new(),
            };
            let error = tool
                .tool_execute(self.ccx.clone(), &"denied".into(), &args)
                .await
                .unwrap_err();
            assert_eq!(error, expected);
            assert!(self
                .app
                .runtime
                .exec_registry
                .list(Default::default())
                .await
                .is_empty());
        }

        async fn assert_refused_before_spawn(
            &self,
            args: HashMap<String, Value>,
            change: impl FnOnce() + Send + 'static,
            expected: &str,
        ) {
            let (request, candidate, _) = approval_request(&self.ccx, &args).await.unwrap();
            let candidate = candidate.unwrap();
            assert_eq!(candidate.root(), self.root);
            assert_eq!(Some(candidate.cwd()), request.cwd.as_deref());
            assert!(self
                .app
                .runtime
                .exec_registry
                .list(Default::default())
                .await
                .is_empty());
            let reached = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let changed = reached.clone();
            CHECKOUT_BEFORE_SPAWN
                .scope(
                    std::cell::RefCell::new(Some(Box::new(move || {
                        change();
                        changed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }))),
                    self.assert_refused(args, expected),
                )
                .await;
            assert_eq!(reached.load(std::sync::atomic::Ordering::Relaxed), 1);
        }
    }

    impl CheckoutAttempt {
        async fn approve_process(&self, args: &HashMap<String, Value>, call_id: &str) {
            let app = self.ccx.lock().await.app.clone();
            let parent = app.chat.sessions.read().await["planner"].clone();
            {
                let mut parent = parent.lock().await;
                parent.set_runtime_state(refact_runtime_api::SessionState::Generating, None);
                parent
                    .queue_processor_running
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            let call: crate::call_validation::ChatToolCall = serde_json::from_value(json!({
                "id":call_id, "type":"function", "function":{
                    "name":"process_start", "arguments":serde_json::to_string(args).unwrap()
                }
            }))
            .unwrap();
            let mut waiting = Box::pin(crate::agents::approval::request(
                &app,
                &self.ccx,
                &call,
                "saved managed-cache execution",
            ));
            let pending = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let polled = futures::poll!(waiting.as_mut());
                    assert!(
                        polled.is_pending(),
                        "approval ended before decision: {polled:?}"
                    );
                    if let Some(pending) = app
                        .agent_approvals
                        .list_for_parent("planner", crate::agents::approval::now_ms())
                        .into_iter()
                        .find(|pending| pending.tool_call_id == call_id)
                    {
                        let execution = crate::tasks::admission::current_execution(
                            &app,
                            &self.ccx.lock().await.chat_id,
                        )
                        .unwrap();
                        let chat = self.ccx.lock().await.chat_id.clone();
                        let child = app.chat.sessions.read().await[&chat].clone();
                        if execution.state() == crate::agents::execution::ExecutionState::Parked
                            && child.lock().await.runtime.state
                                == refact_runtime_api::SessionState::WaitingUserInput
                        {
                            assert!(!execution.holds_permit());
                            break pending;
                        }
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let target =
                crate::agents::approval_target::ApprovalTarget::from_context(&app, &self.ccx)
                    .await
                    .unwrap();
            target.check(&pending).unwrap();
            assert_eq!(pending.child_work_id, target.work_id);
            assert!(pending.execution.is_some());
            assert!(!self
                .ccx
                .lock()
                .await
                .approved_executions
                .contains_key(call_id));
            let chat = self.ccx.lock().await.chat_id.clone();
            let execution = crate::tasks::admission::current_execution(&app, &chat).unwrap();
            let mut start = ToolProcessStart {
                config_path: String::new(),
            };
            let error = start
                .tool_execute(self.ccx.clone(), &call_id.to_string(), args)
                .await
                .unwrap_err();
            assert_eq!(
                error,
                "managed task execution requires a held current permit"
            );
            assert!(!self
                .ccx
                .lock()
                .await
                .approved_executions
                .contains_key(call_id));
            let parent_thread = parent.lock().await.thread.clone();
            let parent_ccx = Arc::new(AMutex::new(
                AtCommandsContext::new_from_app(
                    app.clone(),
                    4096,
                    20,
                    false,
                    vec![],
                    target.parent_id.clone(),
                    parent_thread.root_chat_id,
                    parent_thread.model,
                    parent_thread.task_meta,
                    None,
                )
                .await,
            ));
            let wrapper_id = format!("approve-{call_id}");
            let wrapper_args = json!({"action":"approve", "request_id":pending.id})
                .as_object()
                .unwrap()
                .clone();
            let decide = || {
                app.tool_registry.execute_tool(
                    &parent_ccx,
                    "task_planner",
                    Some("model"),
                    &wrapper_id,
                    "agent_approval",
                    wrapper_args.clone(),
                )
            };
            assert_eq!(
                app.agent_approvals
                    .wait_now(&pending.id, crate::agents::approval::now_ms()),
                Err(refact_work_core::ApprovalError::NotApproved),
            );
            assert!(app
                .runtime
                .exec_registry
                .list(Default::default())
                .await
                .is_empty());
            app.agent_approvals
                .grant_user_once(
                    &target.parent_id,
                    &pending.id,
                    target.generation,
                    &wrapper_id,
                    crate::agents::approval::now_ms(),
                )
                .unwrap();
            let output = decide()
                .await
                .unwrap()
                .expect("public approval tool unavailable");
            let response: Value =
                serde_json::from_str(&output.messages[0].content.content_text_only()).unwrap();
            assert_eq!(response["request_id"], pending.id);
            assert_eq!(response["decision"], "approved");
            assert_eq!(response["approval_source"], "user_once");
            assert_eq!(
                execution.state(),
                crate::agents::execution::ExecutionState::Parked
            );
            assert!(!execution.holds_permit());
            assert!(app
                .runtime
                .exec_registry
                .list(Default::default())
                .await
                .is_empty());
            assert!(tokio::time::timeout(Duration::from_secs(10), waiting)
                .await
                .unwrap()
                .unwrap());
            let context = self.ccx.lock().await;
            let saved = &context.approved_executions[call_id];
            assert_eq!(saved.agent_id, target.work_id);
            assert_eq!(saved.generation, target.generation);
            assert_eq!(Some(&saved.execution), pending.execution.as_ref());
            assert!(!saved.consumed);
            assert!(
                crate::tasks::admission::current_execution(&app, &context.chat_id)
                    .unwrap()
                    .holds_permit()
            );
        }
    }

    #[tokio::test]
    async fn command_checkout_process_confirmation_approval_and_spawn_preserve_child_cwd() {
        let fixture = CheckoutFixture::new().await;
        let mut args = fixture.command_args();
        if cfg!(target_os = "linux") {
            args.insert(
                "command".into(),
                json!("pwd -P; printf '%s' \"$CARGO_TARGET_DIR\""),
            );
        }
        args.insert("workdir".into(), json!("child"));
        args.insert("needs_confirmation".into(), json!(true));
        args.get_mut("checkout").unwrap()["expected_cwd"] = json!(fixture.root.join("child"));
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let matched = start
            .match_against_confirm_deny(fixture.ccx.clone(), &args)
            .await
            .unwrap();
        assert_eq!(matched.command, args["command"].as_str().unwrap());
        assert_eq!(
            matched.result,
            crate::tools::tools_description::MatchConfirmDenyResult::CONFIRMATION
        );
        let call: crate::call_validation::ChatToolCall = serde_json::from_value(json!({
            "id":"tool_call", "type":"function",
            "function":{"name":"process_start", "arguments":serde_json::to_string(&args).unwrap()}
        }))
        .unwrap();
        let approved = crate::agents::command_approval::prepare(&fixture.ccx, &call, "cwd parity")
            .await
            .unwrap()
            .unwrap();
        let (mut request, candidate, cache) = approval_request(&fixture.ccx, &args).await.unwrap();
        crate::agents::command_approval::freeze(&mut request).unwrap();
        assert_eq!(approved.cwd, fixture.root.join("child").to_str().unwrap());
        assert_ne!(
            approved.fingerprint,
            crate::agents::command_approval::descriptor(&request, "cwd parity")
                .unwrap()
                .fingerprint
        );
        let mut changed = request.clone();
        changed.cwd = Some(fixture.root.clone());
        assert!(candidate
            .as_ref()
            .unwrap()
            .bind_approval(&mut approved.clone(), &changed)
            .is_err());
        assert_eq!(
            request.cwd.as_deref(),
            Some(fixture.root.join("child").as_path())
        );
        let candidate = candidate.unwrap();
        assert_eq!(candidate.cwd(), request.cwd.as_deref().unwrap());
        let mut actual =
            crate::agents::command_approval::descriptor(&request, "cwd parity").unwrap();
        candidate.bind_approval(&mut actual, &request).unwrap();
        cache.as_ref().unwrap().bind_approval(&mut actual);
        assert_eq!(approved, actual);
        assert!(!request.env.contains_key("RUSTC_WRAPPER"));
        assert!(!request.env.keys().any(|key| key.starts_with("SCCACHE_")));
        {
            use crate::http::routers::v1::buddy_native_approvals::{
                handle_approval_response, handle_current_approvals, NativeApprovalRequest,
            };
            let session = fixture.app.chat.sessions.read().await["planner"].clone();
            let mut session = session.lock().await;
            session
                .queue_processor_running
                .store(true, std::sync::atomic::Ordering::Relaxed);
            session.add_message(ChatMessage {
                role: "assistant".into(),
                tool_calls: Some(vec![call.clone()]),
                ..Default::default()
            });
            let index = session.messages.len() - 1;
            session.set_paused_with_reasons_and_auto_approved(
                vec![crate::chat::types::PauseReason {
                    reason_type: "confirmation".into(),
                    tool_name: "process_start".into(),
                    command: matched.command,
                    rule: matched.rule,
                    tool_call_id: call.id.clone(),
                    integr_config_path: None,
                }],
                vec![],
                Some(index),
            );
            drop(session);
            let snapshot = serde_json::to_value(
                handle_current_approvals(axum::extract::State(fixture.app.clone()))
                    .await
                    .unwrap()
                    .0,
            )
            .unwrap();
            let pending = snapshot["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|session| session["chat_id"] == "planner")
                .unwrap();
            assert_eq!(pending["tools"][0]["tool_call_id"], call.id);
            let respond = || async {
                let response: NativeApprovalRequest = serde_json::from_value(json!({
                    "project_root":snapshot["project_root"],
                    "engine_identity":snapshot["engine_identity"],
                    "source_version":pending["source_version"],
                    "client_request_id":"checkout-confirmation",
                    "response":{"kind":"tool", "tool_call_id":call.id, "accepted":true}
                }))
                .unwrap();
                handle_approval_response(
                    axum::extract::State(fixture.app.clone()),
                    axum::extract::Path("planner".into()),
                    axum::Json(response),
                )
                .await
            };
            let mut changed_args = args.clone();
            changed_args.insert("tty".into(), json!(true));
            let session = fixture.app.chat.sessions.read().await["planner"].clone();
            let mut session = session.lock().await;
            session.messages[index].tool_calls.as_mut().unwrap()[0]
                .function
                .arguments = serde_json::to_string(&changed_args).unwrap();
            drop(session);
            let error = respond().await.unwrap_err();
            assert_eq!(error.status_code, hyper::StatusCode::CONFLICT);
            assert_eq!(
                error.message,
                "Native approval source changed; refresh before responding"
            );
            let mut removed_checkout = args.clone();
            removed_checkout.remove("checkout");
            let session = fixture.app.chat.sessions.read().await["planner"].clone();
            let mut session = session.lock().await;
            assert!(session.command_queue.is_empty());
            session.messages[index].tool_calls.as_mut().unwrap()[0]
                .function
                .arguments = serde_json::to_string(&removed_checkout).unwrap();
            drop(session);
            let error = respond().await.unwrap_err();
            assert_eq!(error.status_code, hyper::StatusCode::CONFLICT);
            assert_eq!(
                error.message,
                "Native approval source changed; refresh before responding"
            );
            assert!(fixture
                .app
                .runtime
                .exec_registry
                .list(Default::default())
                .await
                .is_empty());
            let session = fixture.app.chat.sessions.read().await["planner"].clone();
            let mut session = session.lock().await;
            assert!(session.command_queue.is_empty());
            session.messages[index].tool_calls.as_mut().unwrap()[0] = call.clone();
            drop(session);
            assert_eq!(
                respond().await.unwrap().status(),
                hyper::StatusCode::ACCEPTED
            );
            let session = fixture.app.chat.sessions.read().await["planner"].clone();
            let mut session = session.lock().await;
            let queued = session.command_queue.pop_front().unwrap();
            assert_eq!(
                queued.native_approval_guard.unwrap().source_version,
                pending["source_version"].as_str().unwrap()
            );
            let crate::chat::types::ChatCommand::ToolDecision {
                tool_call_id,
                accepted,
            } = queued.command
            else {
                panic!("expected native tool decision");
            };
            assert_eq!(tool_call_id, call.id);
            assert!(accepted);
            let decision =
                session.process_tool_decisions(&[crate::chat::types::ToolDecisionItem {
                    tool_call_id,
                    accepted,
                }]);
            assert_eq!(decision.accepted_ids, vec![call.id]);
            assert!(decision.denied_ids.is_empty());
            session.set_runtime_state(refact_runtime_api::SessionState::Idle, None);
            session
                .queue_processor_running
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
        assert!(fixture.ccx.lock().await.approved_executions.is_empty());
        let message = run_tool(&mut start, fixture.ccx.clone(), args)
            .await
            .unwrap();
        let id = process_id(&message);
        let completed = fixture.app.runtime.exec_registry.wait(&id).await.unwrap();
        let output = fixture.app.runtime.exec_registry.read(&id, 0, None).await;
        assert_eq!(completed.meta.cwd, request.cwd);
        let expected = if cfg!(target_os = "linux") {
            assert_eq!(
                request.env["CARGO_TARGET_DIR"],
                fixture
                    .root
                    .join("refact-agent/engine/target")
                    .to_str()
                    .unwrap()
            );
            format!(
                "{}\n{}",
                candidate.cwd().display(),
                request.env["CARGO_TARGET_DIR"]
            )
        } else {
            candidate.cwd().to_str().unwrap().to_string()
        };
        assert_eq!(collect_combined(&output.chunks).trim(), expected);

        let fixture = CheckoutFixture::new().await;
        let attempt = fixture.retained_attempt().await;
        let mut args = fixture.command_args();
        args.remove("checkout");
        args.insert("workdir".into(), json!(fixture.root.join("child")));
        args.insert("needs_confirmation".into(), json!(true));
        if cfg!(target_os = "linux") {
            args.insert(
                "command".into(),
                json!("pwd -P; printf '%s' \"$CARGO_TARGET_DIR\""),
            );
        }
        let (request, checkout, cache) = approval_request(&attempt.ccx, &args).await.unwrap();
        assert!(checkout.is_none());
        assert!(cache.is_some());
        if cfg!(target_os = "linux") {
            assert_eq!(
                request.env["CARGO_TARGET_DIR"],
                fixture
                    .root
                    .join("refact-agent/engine/target")
                    .to_str()
                    .unwrap()
            );
        }
        attempt.approve_process(&args, "stale-context").await;
        attempt.ccx.lock().await.current_model = "changed/model".into();
        let error = start
            .tool_execute(attempt.ccx.clone(), &"stale-context".into(), &args)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            "approved command, working directory, environment or privilege changed; request approval again"
        );
        assert!(attempt.ccx.lock().await.approved_executions["stale-context"].consumed);
        assert!(fixture
            .app
            .runtime
            .exec_registry
            .list(Default::default())
            .await
            .is_empty());
        attempt.ccx.lock().await.current_model = "model".into();
        attempt.approve_process(&args, "saved-child").await;
        let chat = attempt.ccx.lock().await.chat_id.clone();
        let execution = crate::tasks::admission::current_execution(&fixture.app, &chat).unwrap();
        execution.park().await.unwrap();
        let error = start
            .tool_execute(attempt.ccx.clone(), &"saved-child".into(), &args)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            "managed task execution requires a held current permit"
        );
        assert!(!attempt.ccx.lock().await.approved_executions["saved-child"].consumed);
        assert!(fixture
            .app
            .runtime
            .exec_registry
            .list(Default::default())
            .await
            .is_empty());
        execution.resume().await.unwrap();
        let (request, checkout, cache) = approval_request(&attempt.ccx, &args).await.unwrap();
        assert!(checkout.is_none());
        assert!(cache.is_some());
        let (_, messages) = start
            .tool_execute(attempt.ccx.clone(), &"saved-child".into(), &args)
            .await
            .unwrap();
        assert!(attempt.ccx.lock().await.approved_executions["saved-child"].consumed);
        let id = process_id(&only_message(messages));
        let completed = tokio::time::timeout(
            Duration::from_secs(10),
            fixture.app.runtime.exec_registry.wait(&id),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            completed.status,
            ExecStatus::Exited {
                exit_code: Some(0)
            }
        );
        assert_eq!(completed.meta.cwd, request.cwd);
        let output = fixture.app.runtime.exec_registry.read(&id, 0, None).await;
        let expected = if cfg!(target_os = "linux") {
            let target = fixture.root.join("refact-agent/engine/target");
            assert_eq!(request.env["CARGO_TARGET_DIR"], target.to_str().unwrap());
            format!(
                "{}\n{}",
                fixture.root.join("child").display(),
                target.display()
            )
        } else {
            fixture.root.join("child").to_str().unwrap().to_string()
        };
        assert_eq!(collect_combined(&output.chunks).trim(), expected);
        let error = start
            .tool_execute(attempt.ccx.clone(), &"saved-child".into(), &args)
            .await
            .unwrap_err();
        assert_eq!(error, "command approval has already been used");
        assert_eq!(
            fixture
                .app
                .runtime
                .exec_registry
                .list(Default::default())
                .await
                .len(),
            1
        );

        let fixture = CheckoutFixture::new().await;
        let attempt = fixture.retained_attempt().await;
        let mut args = fixture.command_args();
        args.remove("checkout");
        args.insert("workdir".into(), json!(fixture.root.join("child")));
        args.insert("needs_confirmation".into(), json!(true));
        let call: crate::call_validation::ChatToolCall = serde_json::from_value(json!({
            "id":"changed-pending", "type":"function", "function":{
                "name":"process_start", "arguments":serde_json::to_string(&args).unwrap()
            }
        }))
        .unwrap();
        let parent = fixture.app.chat.sessions.read().await["planner"].clone();
        parent
            .lock()
            .await
            .set_runtime_state(refact_runtime_api::SessionState::Generating, None);
        let mut waiting = Box::pin(crate::agents::approval::request(
            &fixture.app,
            &attempt.ccx,
            &call,
            "pending changes",
        ));
        let chat = attempt.ccx.lock().await.chat_id.clone();
        let execution = crate::tasks::admission::current_execution(&fixture.app, &chat).unwrap();
        let pending = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                assert!(futures::poll!(waiting.as_mut()).is_pending());
                if execution.state() == crate::agents::execution::ExecutionState::Parked {
                    if let Some(pending) = fixture
                        .app
                        .agent_approvals
                        .list_for_parent("planner", crate::agents::approval::now_ms())
                        .into_iter()
                        .find(|pending| pending.tool_call_id == call.id)
                    {
                        let child = fixture.app.chat.sessions.read().await[&chat].clone();
                        if child.lock().await.runtime.state
                            == refact_runtime_api::SessionState::WaitingUserInput
                        {
                            break pending;
                        }
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!execution.holds_permit());
        assert!(pending.execution.is_some());
        fixture
            .app
            .agent_approvals
            .grant_user_once(
                "planner",
                &pending.id,
                pending.run_generation,
                "stale-pending-wrapper",
                crate::agents::approval::now_ms(),
            )
            .unwrap();
        let wrapper_args = json!({"action":"approve", "request_id":pending.id})
            .as_object()
            .unwrap()
            .clone();
        let decide = || {
            fixture.app.tool_registry.execute_tool(
                &fixture.ccx,
                "task_planner",
                Some("model"),
                "stale-pending-wrapper",
                "agent_approval",
                wrapper_args.clone(),
            )
        };
        if cfg!(target_os = "linux") {
            let manifest = fixture.root.join("refact-agent/engine/Cargo.toml");
            let original = std::fs::read(&manifest).unwrap();
            std::fs::write(&manifest, "[workspace]\nmembers = []\n").unwrap();
            let error = decide().await.err().expect("changed cache input approved");
            assert_eq!(error, "child approval execution binding changed");
            std::fs::write(&manifest, original).unwrap();
            let child = fixture.app.chat.sessions.read().await[&chat].clone();
            child.lock().await.thread.model = "changed/model".into();
            let error = decide().await.err().expect("changed model approved");
            assert_eq!(error, "child approval execution binding changed");
            child.lock().await.thread.model = "model".into();
        }
        assert_eq!(
            crate::agents::command_approval::validate_pending(&attempt.ccx, &call, &pending)
                .await
                .unwrap(),
            pending.execution,
        );
        assert!(!execution.holds_permit());
        assert!(fixture
            .app
            .runtime
            .exec_registry
            .list(Default::default())
            .await
            .is_empty());
        fixture
            .change_board(|board| {
                board.get_card_mut("T-1").unwrap().agent_chat_id = Some("replacement".into());
            })
            .await;
        let error = decide().await.err().expect("reassigned card approved");
        assert_eq!(error, "board approval belongs to a stale card attempt");
        fixture
            .change_board(|board| {
                board.get_card_mut("T-1").unwrap().agent_chat_id = Some(chat.clone());
            })
            .await;
        attempt.admission.end();
        assert!(decide().await.is_err());
        drop(waiting);
        assert_eq!(
            fixture
                .app
                .agent_approvals
                .wait_now(&pending.id, crate::agents::approval::now_ms()),
            Ok(refact_work_core::ApprovalDecision::Cancelled),
        );
        assert!(attempt.ccx.lock().await.approved_executions.is_empty());
        assert!(fixture
            .app
            .runtime
            .exec_registry
            .list(Default::default())
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn command_checkout_shell_confirmation_and_execution_use_same_root() {
        let mut fixture = CheckoutFixture::new().await;
        fixture.source = dunce::canonicalize(&fixture.source).unwrap();
        let mut args = fixture.command_args();
        if cfg!(target_os = "linux") {
            args.insert(
                "command".into(),
                json!("pwd -P; printf '%s' \"$CARGO_TARGET_DIR\""),
            );
        }
        let mut inspect = crate::tools::tool_work_inspect::ToolWorkInspect {
            config_path: String::new(),
        };
        let receipt = run_tool(
            &mut inspect,
            fixture.ccx.clone(),
            make_args_map(vec![(
                "selector",
                json!({"kind":"worktree", "worktree_id":fixture.selector["expected_worktree_id"]}),
            )]),
        )
        .await
        .unwrap();
        let response: Value = serde_json::from_str(&text(&receipt)).unwrap();
        assert_eq!(response["command_checkout"], fixture.selector);
        let records = receipt.extra["privacy"]["files"].as_array().unwrap();
        assert!(records.iter().any(|record| {
            record["path"]
                == json!(refact_core::chat_types::normalize_file_name(
                    fixture.root.to_string_lossy().into_owned()
                ))
                && record["attribution"] == "declared"
        }));
        assert!(records.iter().any(|record| {
            record["path"]
                == json!(refact_core::chat_types::normalize_file_name(
                    fixture
                        .source
                        .join(".refact/tasks/task/board.yaml")
                        .to_string_lossy()
                        .into_owned()
                ))
                && record["attribution"] == "declared"
        }));
        let candidate =
            crate::tools::command_checkout::resolve_planner_verifier(&fixture.ccx, "task", "T-1")
                .await
                .unwrap();
        let argv = vec!["git".to_string(), "status".to_string()];
        let policy = crate::exec::command_policy::build_verifier_exec_request(
            fixture.app.gcx.clone(),
            crate::exec::command_policy::CommandPolicyInput {
                source: crate::exec::command_policy::ExecSource::Verifier,
                command: crate::exec::command_policy::CommandKind::Argv(&argv),
                cwd: Some(fixture.root.join("child")),
                env: HashMap::new(),
                chat_mode: None,
                escalation: None,
            },
            &candidate,
        )
        .await
        .unwrap();
        assert_eq!(
            policy.request.cwd.as_deref(),
            Some(fixture.root.join("child").as_path())
        );
        assert_eq!(policy.request.audit.as_ref().unwrap().source, "verifier");
        let mut shell = crate::tools::tool_shell::ToolShell::default();
        let matched = shell
            .match_against_confirm_deny(fixture.ccx.clone(), &args)
            .await
            .unwrap();
        assert_eq!(matched.command, args["command"].as_str().unwrap());
        let (request, _, cache) = crate::tools::shell_approval_request(&fixture.ccx, &args)
            .await
            .unwrap();
        let message = run_tool(&mut shell, fixture.ccx.clone(), args)
            .await
            .unwrap();
        let id = process_id(&message);
        let completed = tokio::time::timeout(
            Duration::from_secs(60),
            fixture.app.runtime.exec_registry.wait(&id),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(completed.meta.process_id, id);
        assert_eq!(completed.meta.cwd, request.cwd);
        assert_eq!(completed.meta.cwd, Some(fixture.root.clone()));
        if let Some(reader) = fixture
            .app
            .runtime
            .exec_registry
            .observation_reader(&id)
            .await
        {
            assert!(!matches!(
                reader.wait_status().await,
                refact_exec::ObservationStatus::Pending(_)
            ));
        }
        let mut read = ToolProcessRead {
            config_path: String::new(),
        };
        let output = run_tool(
            &mut read,
            fixture.ccx.clone(),
            make_args_map(vec![
                ("process_id", json!(id.as_str())),
                ("output_limit", json!("all")),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(process_id(&output), id);
        assert!(text(&output).contains(fixture.root.to_str().unwrap()));
        assert_eq!(
            fixture
                .app
                .runtime
                .exec_registry
                .list(Default::default())
                .await
                .len(),
            1
        );
        if cfg!(target_os = "linux") {
            assert_eq!(
                request.env["CARGO_TARGET_DIR"],
                fixture
                    .root
                    .join("refact-agent/engine/target")
                    .to_str()
                    .unwrap()
            );
            assert!(text(&output).contains(&format!(
                "{}\n{}",
                fixture.root.display(),
                request.env["CARGO_TARGET_DIR"]
            )));
        }
        assert!(cache.is_some());
        assert_eq!(
            exec(&message)["cache_report"]["compiler_cache"]["reason"],
            "unsafe_daemon_boundary"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn command_checkout_managed_cache_policy_binds_target_and_degrades_conflicts() {
        let fixture = CheckoutFixture::new().await;
        let args = fixture.command_args();
        let (mut request, checkout, _) = approval_request(&fixture.ccx, &args).await.unwrap();
        request.env.remove("CARGO_TARGET_DIR");
        request.sandbox = Some(crate::exec::types::ExecSandboxSpec {
            mode: crate::exec::types::ExecSandboxMode::WorkspaceWrite,
            ro_paths: vec![PathBuf::from("/")],
            rw_paths: vec![fixture.root.clone()],
            allow_network: false,
        });
        let target = fixture.root.join("refact-agent/engine/target");
        let roots = request.sandbox.clone();
        let cache = prepare_managed_cache(&fixture.ccx, checkout.as_ref(), &mut request)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request.env["CARGO_TARGET_DIR"], target.to_str().unwrap());
        assert_eq!(request.sandbox, roots);
        let mut approved = crate::agents::command_approval::descriptor(&request, "cache").unwrap();
        cache.bind_approval(&mut approved);
        cache.revalidate(&fixture.ccx, &request).await.unwrap();
        let mut conflict = request.clone();
        conflict
            .env
            .insert("CARGO_TARGET_DIR".into(), "caller-choice".into());
        let disabled = prepare_managed_cache(&fixture.ccx, checkout.as_ref(), &mut conflict)
            .await
            .unwrap()
            .unwrap();
        let mut extra = serde_json::Map::from_iter([("exec".into(), json!({}))]);
        disabled.attach_report(&mut extra);
        assert_eq!(conflict.env["CARGO_TARGET_DIR"], "caller-choice");
        assert_eq!(
            extra["exec"]["cache_report"]["cargo_target"]["reason"],
            "input_mismatch"
        );
        let mut readonly = request.clone();
        readonly.env.remove("CARGO_TARGET_DIR");
        readonly.sandbox.as_mut().unwrap().mode = crate::exec::types::ExecSandboxMode::ReadOnly;
        let disabled = prepare_managed_cache(&fixture.ccx, checkout.as_ref(), &mut readonly)
            .await
            .unwrap()
            .unwrap();
        disabled.attach_report(&mut extra);
        assert!(!readonly.env.contains_key("CARGO_TARGET_DIR"));
        assert_eq!(
            extra["exec"]["cache_report"]["cargo_target"]["reason"],
            "unverified_isolation"
        );
        assert_eq!(
            extra["exec"]["cache_report"]["compiler_cache"]["reason"],
            "unsafe_daemon_boundary"
        );
        {
            let previous_board =
                crate::tools::command_checkout::read_board(&fixture.source, "task")
                    .await
                    .unwrap()
                    .0;
            let attempt = fixture.retained_attempt().await;
            let mut args = fixture.command_args();
            args.remove("checkout");
            args.insert("workdir".into(), json!(fixture.root.join("child")));
            let (retained, checkout, cache) = approval_request(&attempt.ccx, &args).await.unwrap();
            assert!(checkout.is_none());
            assert_eq!(retained.env["CARGO_TARGET_DIR"], target.to_str().unwrap());
            let retained_cache = cache.unwrap();
            retained_cache
                .revalidate(&attempt.ccx, &retained)
                .await
                .unwrap();
            let chat = attempt.ccx.lock().await.chat_id.clone();
            let child = fixture.app.chat.sessions.read().await[&chat].clone();
            let task = child.lock().await.thread.task_meta.clone().unwrap();
            let worktree = child.lock().await.thread.worktree.clone().unwrap();
            let board = crate::tools::command_checkout::read_board(&fixture.source, "task")
                .await
                .unwrap()
                .0;
            assert_eq!(board.get_card("T-1").unwrap().assignee, task.agent_id);
            assert_eq!(
                board.get_card("T-1").unwrap().agent_chat_id.as_deref(),
                Some(chat.as_str())
            );
            child
                .lock()
                .await
                .thread
                .task_meta
                .as_mut()
                .unwrap()
                .agent_id = worktree.agent_id;
            let (denied, _, disabled) = approval_request(&attempt.ccx, &args).await.unwrap();
            assert!(disabled.is_none());
            assert!(!denied.env.contains_key("CARGO_TARGET_DIR"));
            child.lock().await.thread.task_meta = Some(task);
            fixture
                .change_board(|board| board.owner_chat_id = Some("foreign-planner".into()))
                .await;
            let (denied, _, disabled) = approval_request(&attempt.ccx, &args).await.unwrap();
            assert!(disabled.is_none());
            assert!(!denied.env.contains_key("CARGO_TARGET_DIR"));
            fixture
                .change_board(|board| board.owner_chat_id = Some("planner".into()))
                .await;
            let execution =
                crate::tasks::admission::current_execution(&fixture.app, &chat).unwrap();
            execution.park().await.unwrap();
            assert!(!execution.holds_permit());
            let (denied, _, disabled) = approval_request(&attempt.ccx, &args).await.unwrap();
            assert!(disabled.is_none());
            assert!(!denied.env.contains_key("CARGO_TARGET_DIR"));
            assert!(retained_cache
                .revalidate(&attempt.ccx, &retained)
                .await
                .is_err());
            tokio::time::timeout(Duration::from_secs(5), execution.resume())
                .await
                .unwrap()
                .unwrap();
            retained_cache
                .revalidate(&attempt.ccx, &retained)
                .await
                .unwrap();
            fixture
                .change_board(|board| {
                    board.get_card_mut("T-1").unwrap().paused_at = Some("now".into())
                })
                .await;
            let (denied, _, disabled) = approval_request(&attempt.ccx, &args).await.unwrap();
            assert!(disabled.is_none());
            assert!(!denied.env.contains_key("CARGO_TARGET_DIR"));
            fixture
                .change_board(|board| board.get_card_mut("T-1").unwrap().paused_at = None)
                .await;
            attempt.admission.end();
            assert!(crate::tasks::admission::current_execution(&fixture.app, &chat).is_err());
            let (denied, _, disabled) = approval_request(&attempt.ccx, &args).await.unwrap();
            assert!(disabled.is_none());
            assert!(!denied.env.contains_key("CARGO_TARGET_DIR"));
            fixture.change_board(|board| *board = previous_board).await;
        }
        let session = fixture.app.chat.sessions.read().await["planner"].clone();
        session.lock().await.thread.mode = "explore".into();
        let mut unconfined_readonly = readonly.clone();
        unconfined_readonly.sandbox = None;
        let disabled =
            prepare_managed_cache(&fixture.ccx, checkout.as_ref(), &mut unconfined_readonly)
                .await
                .unwrap()
                .unwrap();
        disabled.attach_report(&mut extra);
        assert!(!unconfined_readonly.env.contains_key("CARGO_TARGET_DIR"));
        assert_eq!(
            extra["exec"]["cache_report"]["cargo_target"]["reason"],
            "unverified_isolation"
        );
        session.lock().await.thread.mode = "task_planner".into();
        std::fs::write(
            fixture.root.join("refact-agent/engine/Cargo.toml"),
            "[workspace]\nmembers = []\n",
        )
        .unwrap();
        assert!(cache.revalidate(&fixture.ccx, &request).await.is_err());
        let mut current = request.clone();
        current.env.remove("CARGO_TARGET_DIR");
        let changed = prepare_managed_cache(&fixture.ccx, checkout.as_ref(), &mut current)
            .await
            .unwrap()
            .unwrap();
        let mut current_approval =
            crate::agents::command_approval::descriptor(&current, "cache").unwrap();
        changed.bind_approval(&mut current_approval);
        assert_ne!(approved, current_approval);
        let mut unsupported = current;
        unsupported.env.remove("CARGO_TARGET_DIR");
        let config = fixture.root.join("refact-agent/engine/.cargo");
        std::fs::create_dir(&config).unwrap();
        std::fs::write(
            config.join("config.toml"),
            "[build]\n\"rustc-wrapper\" = \"host-sccache\"\n",
        )
        .unwrap();
        let disabled = prepare_managed_cache(&fixture.ccx, checkout.as_ref(), &mut unsupported)
            .await
            .unwrap()
            .unwrap();
        disabled.attach_report(&mut extra);
        assert_eq!(
            unsupported.env["CARGO_TARGET_DIR"],
            target.to_str().unwrap()
        );
        assert_eq!(
            extra["exec"]["cache_report"]["cargo_target"]["reason"],
            "checkout_local"
        );
        assert!(!unsupported.env.contains_key("RUSTC_WRAPPER"));
        let public = serde_json::to_string(&extra).unwrap();
        assert!(!public.contains(fixture.root.to_str().unwrap()));
        assert!(!public.contains("host-sccache"));
        assert_eq!(
            extra["exec"]["cache_report"]["compiler_cache"]["reason"],
            "unsafe_daemon_boundary"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn command_checkout_managed_input_change_at_shell_spawn_refuses() {
        let fixture = CheckoutFixture::new().await;
        let args = fixture.command_args();
        let (request, _, cache) = crate::tools::shell_approval_request(&fixture.ccx, &args)
            .await
            .unwrap();
        cache
            .unwrap()
            .revalidate(&fixture.ccx, &request)
            .await
            .unwrap();
        let path = fixture.root.join("refact-agent/engine/Cargo.toml");
        let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let changed = reached.clone();
        let mut shell = crate::tools::tool_shell::ToolShell::default();
        let result = crate::exec::command_policy::MANAGED_CACHE_BEFORE_SPAWN
            .scope(
                std::cell::RefCell::new(Some(Box::new(move || {
                    std::fs::write(path, "[workspace]\nmembers = []\n").unwrap();
                    changed.store(true, std::sync::atomic::Ordering::Relaxed);
                }))),
                shell.tool_execute(fixture.ccx.clone(), &"changed-input".into(), &args),
            )
            .await;
        assert!(reached.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(
            result.unwrap_err(),
            "managed cache execution inputs changed; request command approval again"
        );
        assert!(fixture
            .app
            .runtime
            .exec_registry
            .list(Default::default())
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn owning_card_command_allowed_but_nested_id_depth_and_registry_refuse() {
        let fixture = CheckoutFixture::new().await;
        let attempt = fixture.retained_attempt().await;
        let mut args = fixture.command_args();
        args.remove("checkout");
        args.insert("workdir".into(), json!(fixture.root.join("child")));
        {
            let context = attempt.ccx.lock().await;
            assert!(context.background_agent_id.is_none());
            assert_eq!(context.subchat_depth, 0);
        }
        let (mut request, checkout, cache) = approval_request(&attempt.ccx, &args).await.unwrap();
        assert!(checkout.is_none());
        assert!(cache.is_some());
        crate::agents::command_approval::enforce(
            &attempt.ccx,
            "owner",
            "process_start",
            &args,
            &mut request,
            None,
            cache.as_ref(),
        )
        .await
        .unwrap();
        for signal in 0..3 {
            {
                let mut context = attempt.ccx.lock().await;
                context.background_agent_id = (signal == 0).then(|| "nested".into());
                context.subchat_depth = if signal == 1 { 1 } else { 0 };
            }
            if signal == 2 {
                let (record, _, _) = fixture
                    .app
                    .agents
                    .create(crate::agents::types::CreateAgentRequest {
                        parent_chat_id: "owner".into(),
                        parent_root_chat_id: Some("planner".into()),
                        parent_tool_call_id: None,
                        kind: crate::agents::types::BgAgentKind::Subagent,
                        config_name: "test".into(),
                        title: "nested".into(),
                        prompt: "test".into(),
                        target_files: vec![],
                        model: "model".into(),
                        model_type: None,
                        goal_summary: None,
                        plan_present: false,
                        worktree_id: None,
                        worktree_branch: None,
                    })
                    .await
                    .unwrap();
                let chat = attempt.ccx.lock().await.chat_id.clone();
                fixture
                    .app
                    .agents
                    .mark_running(&record.agent_id, chat)
                    .await
                    .unwrap();
            }
            let error = approval_request(&attempt.ccx, &args).await.err().unwrap();
            assert_eq!(error, "Nested subagent command execution is not supported for this task checkout. Ask the owning card agent to run this command; no command was started.");
            let error = crate::agents::command_approval::enforce(
                &attempt.ccx,
                "nested",
                "process_start",
                &args,
                &mut request,
                None,
                None,
            )
            .await
            .unwrap_err();
            assert_eq!(error, "Nested subagent command execution is not supported for this task checkout. Ask the owning card agent to run this command; no command was started.");
        }
        assert!(fixture
            .app
            .runtime
            .exec_registry
            .list(Default::default())
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn command_checkout_registry_change_at_process_spawn_refuses() {
        let fixture = CheckoutFixture::new().await;
        let args = fixture.command_args();
        let service = crate::worktrees::service::WorktreeService::new(
            fixture.app.gcx.cache_dir.clone(),
            fixture.source.clone(),
        )
        .unwrap();
        let path = service.registry_path();
        let id = fixture.selector["expected_worktree_id"]
            .as_str()
            .unwrap()
            .to_string();
        fixture
            .assert_refused_before_spawn(args, move || {
                let mut registry: crate::worktrees::types::WorktreeRegistry =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                assert!(registry.records.iter().any(|record| record.meta.id == id));
                registry.records.retain(|record| record.meta.id != id);
                std::fs::write(path, serde_json::to_vec(&registry).unwrap()).unwrap();
            }, "command checkout is unavailable, unauthorized or changed; request a fresh checkout target")
            .await;
    }

    #[tokio::test]
    async fn command_checkout_stale_board_after_confirmation_has_no_spawn() {
        let fixture = CheckoutFixture::new().await;
        let args = fixture.command_args();
        let start = ToolProcessStart {
            config_path: String::new(),
        };
        start
            .match_against_confirm_deny(fixture.ccx.clone(), &args)
            .await
            .unwrap();
        let path = fixture.source.join(".refact/tasks/task/board.yaml");
        fixture
            .assert_refused_before_spawn(args, move || {
                let mut board: crate::tasks::types::TaskBoard =
                    serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                board.rev += 1;
                std::fs::write(path, serde_yaml::to_string(&board).unwrap()).unwrap();
            }, "command checkout is unavailable, unauthorized or changed; request a fresh checkout target")
            .await;
    }

    #[tokio::test]
    async fn command_checkout_owner_change_after_preparation_has_no_spawn() {
        let fixture = CheckoutFixture::new().await;
        let args = fixture.command_args();
        approval_request(&fixture.ccx, &args).await.unwrap();
        fixture
            .change_board(|board| board.owner_chat_id = Some("peer".into()))
            .await;
        fixture.assert_refused(args, "task board mutation requires the owning planner chat; owning planner chat: peer. Open that planner chat to operate on this checkout").await;
    }

    #[tokio::test]
    async fn command_checkout_current_agent_replacement_has_no_spawn() {
        let fixture = CheckoutFixture::new().await;
        fixture
            .change_board(|board| board.cards[0].agent_chat_id = Some("new-attempt".into()))
            .await;
        fixture.assert_refused(fixture.command_args(), "command checkout is unavailable, unauthorized or changed; request a fresh checkout target").await;
    }

    #[tokio::test]
    async fn command_checkout_head_change_after_preparation_has_no_spawn() {
        let fixture = CheckoutFixture::new().await;
        let args = fixture.command_args();
        approval_request(&fixture.ccx, &args).await.unwrap();
        std::fs::write(fixture.root.join("file"), "changed").unwrap();
        crate::worktrees::git::run_git(&fixture.root, &["add", "file"]).unwrap();
        crate::worktrees::git::run_git(&fixture.root, &["commit", "-m", "new-head"]).unwrap();
        let actual = crate::worktrees::git::run_git(&fixture.root, &["rev-parse", "HEAD"]).unwrap();
        let expected = format!(
            "Checkout {} at {} changed: expected HEAD {}, actual {}; request a fresh checkout target",
            fixture.selector["expected_worktree_id"].as_str().unwrap(),
            fixture.root.display(),
            fixture.selector["expected_head"].as_str().unwrap(),
            actual.trim(),
        );
        fixture.assert_refused(args, &expected).await;
    }

    #[tokio::test]
    async fn command_checkout_wrong_registered_identity_has_no_spawn() {
        let fixture = CheckoutFixture::new().await;
        let mut args = fixture.command_args();
        args.get_mut("checkout").unwrap()["expected_worktree_id"] = json!("different-worktree");
        fixture.assert_refused(args, "command checkout is unavailable, unauthorized or changed; request a fresh checkout target").await;
    }

    #[tokio::test]
    async fn command_checkout_peer_planner_cannot_borrow_owner_target() {
        let fixture = CheckoutFixture::new().await;
        let owner = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get("planner")
            .cloned()
            .unwrap();
        let mut peer = crate::chat::types::ChatSession::new("peer".into());
        peer.thread = owner.lock().await.thread.clone();
        peer.work_project_root = Some(fixture.source.clone());
        fixture
            .app
            .chat
            .sessions
            .write()
            .await
            .insert("peer".into(), Arc::new(AMutex::new(peer)));
        fixture.ccx.lock().await.chat_id = "peer".into();
        let mut inspect = crate::tools::tool_work_inspect::ToolWorkInspect {
            config_path: String::new(),
        };
        let inspected = run_tool(
            &mut inspect,
            fixture.ccx.clone(),
            make_args_map(vec![(
                "selector",
                json!({"kind":"worktree", "worktree_id":fixture.selector["expected_worktree_id"]}),
            )]),
        )
        .await
        .unwrap();
        let response: Value = serde_json::from_str(&text(&inspected)).unwrap();
        assert!(response.get("command_checkout").is_none());
        let hidden = run_tool(
            &mut inspect,
            fixture.ccx.clone(),
            make_args_map(vec![(
                "selector",
                json!({"kind":"worktree", "worktree_id":"unknown-worktree"}),
            )]),
        )
        .await
        .unwrap_err();
        let error: Value = serde_json::from_str(&hidden).unwrap();
        assert_eq!(error["code"], "not_found_or_not_visible");
        assert_eq!(error["message"], "work not found or not visible");
        assert_eq!(error["retryable"], false);
        assert!(!error["request_id"].as_str().unwrap().is_empty());
        assert_eq!(error.as_object().unwrap().len(), 4);
        fixture.assert_refused(fixture.command_args(), "task board mutation requires the owning planner chat; owning planner chat: planner. Open that planner chat to operate on this checkout").await;
    }

    #[tokio::test]
    async fn command_checkout_foreign_bound_task_has_no_spawn() {
        let fixture = CheckoutFixture::new().await;
        fixture.ccx.lock().await.task_meta.as_mut().unwrap().task_id = "foreign-task".into();
        fixture.assert_refused(fixture.command_args(), "command checkout is unavailable, unauthorized or changed; request a fresh checkout target").await;
    }

    #[tokio::test]
    async fn command_checkout_live_agent_cannot_be_controlled_by_done_board() {
        let fixture = CheckoutFixture::new().await;
        let session = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get("worker-chat")
            .cloned()
            .unwrap();
        session.lock().await.runtime.state = refact_runtime_api::SessionState::Generating;
        fixture.assert_refused(fixture.command_args(), "command checkout is unavailable, unauthorized or changed; request a fresh checkout target").await;
    }

    #[tokio::test]
    async fn command_checkout_registered_path_without_selector_stays_denied() {
        let fixture = CheckoutFixture::new().await;
        let mut args = fixture.command_args();
        args.remove("checkout");
        args.insert("workdir".into(), json!(fixture.root));
        let mut raw =
            crate::exec::ExecSpawnRequest::foreground("pwd").with_cwd(fixture.root.clone());
        assert!(prepare_managed_cache(&fixture.ccx, None, &mut raw)
            .await
            .unwrap()
            .is_none());
        assert!(!raw.env.contains_key("CARGO_TARGET_DIR"));
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        assert!(start
            .tool_execute(fixture.ccx.clone(), &"raw".into(), &args)
            .await
            .is_err());
        assert!(fixture
            .app
            .runtime
            .exec_registry
            .list(Default::default())
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn command_checkout_source_alias_and_traversal_are_not_permissions() {
        let fixture = CheckoutFixture::new().await;
        let mut args = fixture.command_args();
        args.insert("workdir".into(), json!(fixture.source));
        fixture.assert_refused(args, "command checkout is unavailable, unauthorized or changed; request a fresh checkout target").await;
        let mut traversal = fixture.command_args();
        traversal.insert("workdir".into(), json!("child/.."));
        fixture.assert_refused(traversal, "command checkout is unavailable, unauthorized or changed; request a fresh checkout target").await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn command_checkout_symlink_retarget_after_confirmation_refuses_spawn() {
        let fixture = CheckoutFixture::new().await;
        let alias = fixture.root.join("alias");
        std::os::unix::fs::symlink(fixture.root.join("child"), &alias).unwrap();
        let mut args = fixture.command_args();
        args.insert("workdir".into(), json!("alias"));
        args.get_mut("checkout").unwrap()["expected_cwd"] = json!(fixture.root.join("child"));
        let source = fixture.source.clone();
        fixture
            .assert_refused_before_spawn(args, move || {
                std::fs::remove_file(&alias).unwrap();
                std::os::unix::fs::symlink(&source, &alias).unwrap();
            }, "command checkout is unavailable, unauthorized or changed; request a fresh checkout target")
            .await;
    }

    #[tokio::test]
    async fn command_checkout_cancelled_prepared_command_refuses_spawn() {
        let fixture = CheckoutFixture::new().await;
        let args = fixture.command_args();
        approval_request(&fixture.ccx, &args).await.unwrap();
        fixture
            .ccx
            .lock()
            .await
            .abort_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        fixture.assert_refused(args, "command checkout is unavailable, unauthorized or changed; request a fresh checkout target").await;
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
    async fn process_extra_exec_has_duration_ms() {
        let (_gcx, ccx) = test_ccx().await;
        let mut start = ToolProcessStart {
            config_path: String::new(),
        };
        let started = run_tool(
            &mut start,
            ccx.clone(),
            make_args_map(vec![
                ("command", json!(delayed_quick_command("hi"))),
                ("description", json!("Run delayed process")),
            ]),
        )
        .await
        .unwrap();
        let process_id = process_id(&started);
        let completed = run_process_wait(
            ccx,
            make_args_map(vec![
                ("process_id", json!(process_id.as_str())),
                ("timeout_ms", json!(1000)),
            ]),
        )
        .await
        .unwrap();
        let duration_ms = exec(&completed)["duration_ms"].as_u64().unwrap();

        assert!(duration_ms > 0);
    }

    #[tokio::test]
    async fn process_list_all_rejected_without_planner_context() {
        let (gcx, ccx) = test_ccx_with_chat("chat-a").await;
        let process_id = register_test_process(&gcx, "exec_hidden_from_all", "chat-b", None).await;
        let mut list = ToolProcessList {
            config_path: String::new(),
        };

        let err = run_tool(
            &mut list,
            ccx,
            make_args_map(vec![("scope", json!("all")), ("status", json!("all"))]),
        )
        .await
        .unwrap_err();

        assert_eq!(err, "process_list scope=all requires planner/admin context");
        assert!(gcx.exec_registry.get(&process_id).await.is_some());
    }

    #[tokio::test]
    async fn process_list_current_scope_still_allows_owned_process() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let owned = register_test_process(&gcx, "exec_list_owned", "chat-a", None).await;
        let other = register_test_process(&gcx, "exec_list_other", "chat-b", None).await;
        let ccx = test_ccx_for_gcx(gcx, "chat-a", None).await;
        let mut list = ToolProcessList {
            config_path: String::new(),
        };

        let listed = run_tool(&mut list, ccx, HashMap::new()).await.unwrap();

        assert!(text(&listed).contains(owned.as_str()));
        assert!(!text(&listed).contains(other.as_str()));
        assert_eq!(exec(&listed)["count"], 1);
    }

    #[tokio::test]
    async fn planner_can_list_all_processes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let owned = register_test_process(&gcx, "exec_planner_list_owned", "chat-a", None).await;
        let other = register_test_process(&gcx, "exec_planner_list_other", "chat-b", None).await;
        let ccx = test_ccx_for_gcx(gcx, "planner-chat", Some("planner")).await;
        let mut list = ToolProcessList {
            config_path: String::new(),
        };

        let listed = run_tool(
            &mut list,
            ccx,
            make_args_map(vec![("scope", json!("all")), ("status", json!("all"))]),
        )
        .await
        .unwrap();

        assert!(text(&listed).contains(owned.as_str()));
        assert!(text(&listed).contains(other.as_str()));
        assert_eq!(exec(&listed)["count"], 2);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn process_read_pending_observation_withholds_output_and_disk_metadata() {
        let (gcx, ccx) = test_ccx().await;
        let mut request = crate::exec::ExecSpawnRequest::foreground(
            "printf PRIVATE_EARLY; sleep 1; printf PRIVATE_LATE",
        )
        .with_observe(true)
        .with_owner(ExecOwnerMeta {
            chat_id: Some("chat".into()),
            ..Default::default()
        });
        request.return_after_monitor_attach = true;
        let result = gcx.exec_registry.spawn(request).await.unwrap();
        let id = result.snapshot.meta.process_id;
        let reader = gcx.exec_registry.observation_reader(&id).await.unwrap();
        if !matches!(reader.status(), refact_exec::ObservationStatus::Pending(_)) {
            // ptrace may be unavailable on restricted CI hosts; degraded policy stays unchanged.
            gcx.exec_registry.kill(&id).await.unwrap();
            return;
        }
        let mut tool = ToolProcessRead {
            config_path: String::new(),
        };
        let message = run_tool(
            &mut tool,
            ccx,
            make_args_map(vec![
                ("process_id", json!(id.as_str())),
                ("from_disk", json!(true)),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(exec(&message)["observation_pending"], true);
        assert!(!text(&message).contains("PRIVATE_"));
        assert!(exec(&message).get("transcript").is_none());
        assert!(exec(&message).get("path_references").is_none());
        gcx.exec_registry.kill(&id).await.unwrap();
    }

    #[tokio::test]
    async fn process_read_from_disk_combined_still_returns_tail() {
        let (gcx, ccx) = test_ccx().await;
        let snapshot = gcx
            .exec_registry
            .register(
                ExecProcessMeta::new(ExecMode::Background, "test".to_string())
                    .with_chat_id("chat")
                    .with_short_description("Disk combined tail test".to_string()),
                PROCESS_TRANSCRIPT_MAX_BYTES,
            )
            .await;
        let process_id = snapshot.meta.process_id;
        let temp = tempfile::tempdir().unwrap();
        gcx.exec_registry
            .set_spill_target_for_test(
                &process_id,
                SpillTarget::with_root(temp.path().to_path_buf(), "chat", &process_id),
            )
            .await
            .unwrap();
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
        let pipes = run_tool(
            &mut read,
            ccx.clone(),
            make_args_map(vec![
                ("process_id", json!(process_id.as_str())),
                ("stream", json!("all")),
                ("output_limit", json!("all")),
            ]),
        )
        .await
        .unwrap();
        let body = text(&pipes);
        assert!(body.contains("\nstdout:\nalpha\nomega\n"));
        assert!(body.contains("\nstderr:\nneedle\n"));
        assert!(!body.contains("\ncombined:\n"));
        for line in ["alpha\n", "needle\n", "omega\n"] {
            assert_eq!(body.matches(line).count(), 1);
        }

        let pty_text = "\u{1b}[31mpty-needle\u{1b}[0m\r\nskip-pty\nlast-pty\n";
        gcx.exec_registry
            .append_output(&process_id, ExecOutputStream::Combined, pty_text.into())
            .await
            .unwrap();
        let transcript = gcx.exec_registry.read(&process_id, 3, None).await;
        assert_eq!(
            collect_stream(&transcript.chunks, ExecOutputStream::Combined),
            pty_text
        );
        assert_eq!(collect_combined(&transcript.chunks), pty_text);
        for selection in [
            ProcessStreamSelection::All,
            ProcessStreamSelection::Combined,
        ] {
            assert!(
                format_read_sections(&transcript, selection, &OutputFilter::no_limits())
                    .contains(pty_text)
            );
        }
        let filtered = run_tool(
            &mut read,
            ccx.clone(),
            make_args_map(vec![
                ("process_id", json!(process_id.as_str())),
                ("since_seq", json!(3)),
                ("stream", json!("all")),
                ("output_filter", json!("pty-needle")),
                ("output_limit", json!("1")),
            ]),
        )
        .await
        .unwrap();
        let body = text(&filtered);
        assert!(body.contains("\ncombined:\n"));
        assert!(body.contains("\u{1b}[31mpty-needle\u{1b}[0m"));
        for excluded in ["alpha", "omega", "skip-pty", "last-pty"] {
            assert!(!body.contains(excluded));
        }
        assert_eq!(exec(&filtered)["transcript"]["since_seq"], 3);
        assert_eq!(exec(&filtered)["transcript"]["next_seq"], 4);
        assert_eq!(exec(&filtered)["stream"], "all");

        let marker = "DISK_TAIL_MARKER";
        let mut data = String::from(marker);
        data.push_str(&"x".repeat(256 * 1024 + 1));
        gcx.exec_registry
            .append_output(&process_id, ExecOutputStream::Stdout, data)
            .await
            .unwrap();

        assert!(
            gcx.exec_registry.disk_log_path(&process_id).await.is_some(),
            "disk log should be created after spill threshold"
        );

        let message = run_tool(
            &mut read,
            ccx,
            make_args_map(vec![
                ("process_id", json!(process_id.as_str())),
                ("from_disk", json!(true)),
                ("stream", json!("combined")),
            ]),
        )
        .await
        .unwrap();
        let body = text(&message);
        assert!(
            body.contains("combined"),
            "output should be labeled combined"
        );
        assert!(body.contains(marker), "output should contain appended data");
    }
}
