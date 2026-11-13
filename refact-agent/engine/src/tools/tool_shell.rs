use crate::chat::perf_diagnostics::{self, PerfComponent, PerfOutcome};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex as AMutex};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::return_one_candidate_or_a_good_error;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::exec::types::normalize_workspace_path;
use crate::exec::command_policy::{
    build_exec_request, chat_mode_for_exec, escalation_from_args, queue_sandbox_audit, CommandKind,
    prepare_managed_cache, CommandPolicyInput, ExecEscalation, ExecSource, ManagedCacheDecision,
};
use crate::exec::{
    sanitize_short_description, ExecOutputChunk, ExecOutputStream, ExecOwnerMeta,
    ExecProcessSnapshot, ExecRawOutput, ExecReadResult, ExecStatus,
};
use crate::files_correction::canonical_path;
use crate::files_correction::canonicalize_normalized_path;
use crate::files_correction::check_if_its_inside_a_workspace_or_config;
use crate::files_correction::correct_to_nearest_dir_path;
use crate::files_correction::get_active_project_path;
use crate::files_correction::get_unscoped_project_dirs;
use crate::files_correction::preprocess_path_for_normalization;
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::postprocessing::pp_command_output::{
    output_mini_postprocessing, parse_output_filter_args, OutputFilter,
};
use crate::privacy::{check_file_privacy, load_privacy_if_needed, FilePrivacyLevel};
use crate::tools::file_edit::auxiliary::{active_execution_scope, scoped_path_warnings};
use crate::tools::command_checkout::{self, ValidatedCommandCheckout};
use crate::tools::shell_gate;
use crate::tools::tools_description::{
    json_schema_from_params, MatchConfirmDeny, Tool, ToolDesc, ToolSource, ToolSourceType,
};
use crate::worktrees::scope::ExecutionScope;
use refact_core::chat_types::PushMode;

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct SettingsShell {
    #[serde(default)]
    pub output_filter: OutputFilter,
}

#[derive(Default)]
pub struct ToolShell {
    pub cfg: SettingsShell,
    pub config_path: String,
}

const MAX_SHELL_TIMEOUT_SECS: u64 = 3600;
const SHELL_TRANSCRIPT_MAX_BYTES: usize = 2 * 1024 * 1024;
const TOOL_PTY_ROWS: u16 = 32;
const TOOL_PTY_COLS: u16 = 120;
const TTY_DESCRIPTION: &str = "If true, run the command attached to a pseudo-terminal (PTY). Enables interactive stdin via process_write_stdin and merges stdout+stderr into a single combined stream. Defeats some pipe-only output buffering. Defaults to false for foreground commands and true for background commands, except on Windows, where a pty cannot currently deliver output and pipes are used instead.";
const RUN_IN_BACKGROUND_DESCRIPTION: &str = "Set to true to run this command in the background. Returns immediately with a process_id. Use process_read or wait with a process target to retrieve output later. You will receive a process_completed event when the process exits. Do not use '&' at the end of the command.";
const SHELL_PROGRESS_MAX_CHARS_PER_STREAM: usize = 4 * 1024;
const SHELL_PROGRESS_MIN_INTERVAL_MS: u64 = 250;
pub(crate) const SHELL_FOREGROUND_WINDOW: Duration =
    Duration::from_secs(crate::chat::tools::TOOL_JOB_INLINE_BUDGET.as_secs() - 5);
const SHELL_OBSERVATION_GRACE_AFTER_EXIT: Duration = Duration::from_secs(3);

#[cfg(test)]
tokio::task_local! {
    pub(crate) static TEST_FOREGROUND_WINDOW: Duration;
}

fn foreground_window() -> Duration {
    #[cfg(test)]
    if let Ok(window) = TEST_FOREGROUND_WINDOW.try_with(|window| *window) {
        return window;
    }
    SHELL_FOREGROUND_WINDOW
}

struct ParsedShellArgs {
    command: String,
    workdir: Option<PathBuf>,
    custom_filter: Option<OutputFilter>,
    timeout: Option<u64>,
    description: Option<String>,
    tty: Option<bool>,
    run_in_background: bool,
    scope_warnings: Vec<String>,
    escalation: Option<ExecEscalation>,
    push: PushMode,
}

struct ShellScopeGuard(std::sync::Mutex<Option<crate::tool_jobs::ExecutionLease>>);
impl std::fmt::Debug for ShellScopeGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ShellScopeGuard")
    }
}
impl refact_exec::ExecLifecycleGuard for ShellScopeGuard {
    fn settle(&self) {
        if let Some(lease) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            lease.settle();
        }
    }
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
    let policy = shell_gate::load_policy(gcx.clone()).await;
    let mut filter = OutputFilter::default();
    filter.limit_lines = policy.execution.output_limit_lines;
    let checkout = command_checkout::resolve(ccx, args).await?;
    let parsed = parse_args_with_filter(
        gcx.clone(),
        args,
        &filter,
        scope.as_ref(),
        checkout.as_ref(),
    )
    .await?;
    let cwd = parsed
        .workdir
        .or(get_active_project_path(gcx.clone()).await);
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
            source: ExecSource::ShellTool,
            command: CommandKind::Shell(&parsed.command),
            cwd,
            env,
            chat_mode,
            escalation: parsed.escalation,
        },
    )
    .await
    .map_err(|error| error.message)?;
    let mut request = policy.request;
    if checkout.is_some() {
        request.mode = if parsed.run_in_background {
            crate::exec::ExecMode::Background
        } else {
            crate::exec::ExecMode::Foreground
        };
        let tty = parsed
            .tty
            .unwrap_or(parsed.run_in_background && refact_exec::default_tty());
        request = request
            .with_tty(tty)
            .with_transcript_limit(SHELL_TRANSCRIPT_MAX_BYTES);
        if tty {
            request = request.with_pty_size(TOOL_PTY_ROWS, TOOL_PTY_COLS);
        }
    }
    if !parsed.run_in_background {
        if let Some(timeout) = parsed.timeout {
            request = request.with_timeout(Duration::from_secs(timeout));
        }
    }
    Ok((request, checkout))
}

#[async_trait]
impl Tool for ToolShell {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let prepare_span = perf_diagnostics::span(PerfComponent::ShellPrepare, None, None);
        let (gcx, exec_registry, abort_flag, execution_scope, chat_id, derived_privacy_zones) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.runtime.exec_registry.clone(),
                ccx_lock.execution_abort.clone(),
                ccx_lock.execution_scope.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.derived_privacy_zones.clone(),
            )
        };
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let current_model = ccx.lock().await.current_model.clone();
        let destination = crate::privacy::records::provider_destination(&current_model);
        let observe_reason = crate::privacy::records::shell_observation_reason_for_session(
            &gcx,
            &destination,
            &derived_privacy_zones,
        );
        let observe = observe_reason.needed();
        tracing::debug!(
            "shell observation {} for destination {}: {} blocked patterns, {} zones excluding it, \
             {} derived labels",
            if observe { "on" } else { "off" },
            destination.id.0,
            observe_reason.blocked_patterns,
            observe_reason.guarded_zones,
            observe_reason.derived_labels
        );
        let shell_policy = shell_gate::load_policy(gcx.clone()).await;
        let mut default_filter = self.cfg.output_filter.clone();
        default_filter.limit_lines = shell_policy.execution.output_limit_lines;
        default_filter.limit_chars = shell_policy
            .execution
            .output_limit_lines
            .saturating_mul(200);
        if let Some(meta) = execution_scope
            .as_ref()
            .filter(|scope| scope.is_enforced() && scope.worktree().kind == "task_agent")
            .map(|scope| scope.worktree().clone())
        {
            if let Err(error) =
                crate::exec::command_policy::ensure_managed_task_execution(&ccx, &meta).await
            {
                prepare_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
                return Err(error);
            }
        }
        let checkout = match command_checkout::resolve(&ccx, args).await {
            Ok(checkout) => checkout,
            Err(error) => {
                prepare_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
                return Err(error);
            }
        };
        let parsed = match parse_args_with_filter(
            gcx.clone(),
            args,
            &default_filter,
            execution_scope.as_ref(),
            checkout.as_ref(),
        )
        .await
        {
            Ok(parsed) => parsed,
            Err(error) => {
                prepare_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
                return Err(error);
            }
        };
        let timeout = parsed.timeout;

        let mut error_log = Vec::new();
        let env_variables =
            crate::integrations::setting_up_integrations::get_vars_for_replacements(
                gcx.clone(),
                &mut error_log,
            )
            .await;
        let output_filter = parsed.custom_filter.clone().unwrap_or(default_filter);
        let cwd = match parsed.workdir.clone() {
            Some(workdir) => Some(workdir),
            None => get_active_project_path(gcx.clone()).await,
        };
        let workspace = if let Some(checkout) = &checkout {
            Some(checkout.root().to_path_buf())
        } else {
            shell_owner_workspace(gcx.clone(), execution_scope.as_ref(), cwd.as_ref()).await
        };
        let short_description =
            sanitize_short_description(parsed.description.as_deref().unwrap_or_default());
        let owner = ExecOwnerMeta {
            chat_id: Some(chat_id.clone()),
            tool_call_id: Some(tool_call_id.clone()),
            service_name: None,
            workspace,
        };
        let tty = parsed
            .tty
            .unwrap_or(parsed.run_in_background && refact_exec::default_tty());
        let (progress_tx, progress_task) = if parsed.run_in_background || observe {
            (None, None)
        } else {
            let (tx, rx) = mpsc::unbounded_channel();
            let subchat_tx = {
                let ccx_lock = ccx.lock().await;
                ccx_lock.subchat_tx.clone()
            };
            (
                Some(tx),
                Some(tokio::spawn(emit_shell_progress(
                    subchat_tx,
                    tool_call_id.clone(),
                    rx,
                ))),
            )
        };
        let chat_mode = chat_mode_for_exec(gcx.clone(), &chat_id).await;
        let policy = match build_exec_request(
            gcx.clone(),
            CommandPolicyInput {
                source: ExecSource::ShellTool,
                command: CommandKind::Shell(&parsed.command),
                cwd,
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
                prepare_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
                return Err(error.message);
            }
        };
        if let Some(audit) = policy.audit.clone() {
            queue_sandbox_audit(gcx.clone(), &chat_id, audit).await;
        }
        let sandbox_active = policy.request.sandbox.is_some();
        let sandbox_warning = policy.warning;
        let mut request = policy.request;
        request.mode = if parsed.run_in_background {
            crate::exec::ExecMode::Background
        } else {
            crate::exec::ExecMode::Foreground
        };
        request = request
            .with_owner(owner)
            .with_transcript_limit(SHELL_TRANSCRIPT_MAX_BYTES)
            .with_short_description(short_description)
            .with_tty(tty)
            .with_observe(observe)
            .with_push(parsed.push)
            .with_chat_spawn_notification();
        if tty {
            request = request.with_pty_size(TOOL_PTY_ROWS, TOOL_PTY_COLS);
        }
        if !parsed.run_in_background {
            request = request.with_abort_flag(abort_flag.clone());
            if let Some(timeout) = timeout {
                request = request.with_timeout(Duration::from_secs(timeout));
            }
        }
        if let Some(progress_tx) = progress_tx {
            request = request.with_output_progress_tx(progress_tx);
        }

        let cache = match prepare_managed_cache(&ccx, checkout.as_ref(), &mut request).await {
            Ok(cache) => cache,
            Err(error) => {
                drop(request);
                if let Some(progress_task) = progress_task {
                    let _ = progress_task.await;
                }
                prepare_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
                return Err(error);
            }
        };
        prepare_span.finish_tool(PerfOutcome::Success, 1, 1, None);
        let approval_span = perf_diagnostics::span(PerfComponent::ShellApproval, None, None);
        let started = tokio::time::Instant::now();
        if let Err(error) = crate::agents::command_approval::enforce(
            &ccx,
            tool_call_id,
            "shell",
            args,
            &mut request,
            checkout.as_ref(),
            cache.as_ref(),
        )
        .await
        {
            drop(request);
            if let Some(progress_task) = progress_task {
                let _ = progress_task.await;
            }
            approval_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
            return Err(error);
        }
        approval_span.finish_tool(PerfOutcome::Success, 1, 1, None);
        let revalidate_span = perf_diagnostics::span(PerfComponent::ShellRevalidate, None, None);
        if let Some(checkout) = &checkout {
            if let Err(error) = checkout.revalidate(&ccx, args, &request).await {
                drop(request);
                if let Some(progress_task) = progress_task {
                    let _ = progress_task.await;
                }
                revalidate_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
                return Err(error);
            }
        }
        if let Some(cache) = &cache {
            if let Err(error) = cache.revalidate(&ccx, &request).await {
                drop(request);
                if let Some(progress_task) = progress_task {
                    let _ = progress_task.await;
                }
                revalidate_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
                return Err(error);
            }
        }
        revalidate_span.finish_tool(PerfOutcome::Success, 1, 1, None);
        let spawn_span = perf_diagnostics::span(PerfComponent::ShellSpawnWait, None, None);
        let (jobs, mut scope_keys) = {
            let context = ccx.lock().await;
            let jobs = context.app.runtime.tool_jobs.clone();
            let mut keys = vec![jobs.chat_scope_key(&chat_id)];
            if let Some(checkout) = &checkout {
                keys.push(checkout.scope_key());
            } else if let Some(scope) = &execution_scope {
                let meta = scope.worktree();
                keys.push(crate::tool_jobs::ScopeKey::Worktree {
                    project: meta.source_workspace_root.clone(),
                    root: meta.root.clone(),
                });
            } else {
                let roots = context
                    .global_context
                    .documents_state
                    .workspace_folders
                    .lock()
                    .unwrap()
                    .clone();
                keys.extend(
                    crate::chat::tools::workspace_scope_keys(&roots)
                        .map_err(|error| format!("shell {error}"))?,
                );
            }
            (jobs, keys)
        };
        scope_keys.sort();
        scope_keys.dedup();
        let lease = jobs
            .scopes
            .acquire_wait(&scope_keys, abort_flag.clone())
            .await
            .map_err(|error| {
                if error == crate::tool_jobs::ScopeChangeError::Closed {
                    jobs.scopes.shutdown_description()
                } else {
                    shell_scope_refusal(error)
                }
            })?;
        lease.label(format!(
            "tool=shell chat={chat_id} job={tool_call_id} monitor"
        ));
        let scope_guard = Arc::new(ShellScopeGuard(std::sync::Mutex::new(Some(lease))));
        request.execution_guard = Some(scope_guard.clone());
        request = request.with_abort_flag(abort_flag);
        request.return_after_monitor_attach = !parsed.run_in_background;
        let mut result = match exec_registry.spawn(request).await {
            Ok(result) => result,
            Err(error) => {
                if let Some(progress_task) = progress_task {
                    let _ = progress_task.await;
                }
                spawn_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
                return Err(error);
            }
        };
        if !parsed.run_in_background {
            let process_id = &result.snapshot.meta.process_id;
            let completed = tokio::time::timeout(foreground_window(), async {
                let snapshot = exec_registry.wait(process_id).await?;
                let observation = match exec_registry.observation_reader(process_id).await {
                    Some(reader) => tokio::time::timeout(
                        SHELL_OBSERVATION_GRACE_AFTER_EXIT,
                        reader.wait_status(),
                    )
                    .await
                    .ok(),
                    None => Some(result.observation.clone()),
                };
                Ok::<_, String>((snapshot, observation))
            })
            .await;
            match completed {
                Ok(Ok((snapshot, Some(observation)))) => {
                    result.snapshot = snapshot;
                    result.observation = observation;
                }
                Ok(Err(error)) => return Err(error),
                Ok(Ok((_, None))) | Err(_) => {
                    // Only the caller stops waiting. The registry monitor still owns the same
                    // child, original timeout, output drain, observation and cancellation flag.
                    let snapshot = exec_registry.detach_foreground(process_id).await?;
                    if let Some(task) = progress_task {
                        task.abort();
                        let _ = task.await;
                    }
                    let mut extra = serde_json::Map::new();
                    extra.insert(
                        "exec".into(),
                        json!({
                            "process_id": snapshot.meta.process_id.as_str(),
                            "status": exec_status_label(&snapshot.status),
                            "mode": snapshot.meta.mode.to_string(),
                            "tty": snapshot.meta.tty,
                            "timeout_secs": timeout,
                            "detached": true,
                        }),
                    );
                    if let Some(cache) = &cache {
                        cache.attach_report(&mut extra);
                    }
                    spawn_span.finish_tool(PerfOutcome::Success, 1, 1, None);
                    let message = ChatMessage {
                        role: "tool".into(),
                        tool_call_id: tool_call_id.clone(),
                        content: ChatContent::SimpleText(format!(
                            "Foreground command continues under runtime ownership.\nprocess_id: {}\nUse process_read or wait with this process ID; do not rerun the command. Output is withheld until observation is ready.\n",
                            snapshot.meta.process_id)),
                        tool_failed: None,
                        output_filter: Some(OutputFilter::no_limits()),
                        extra,
                        ..Default::default()
                    };
                    return Ok((false, vec![ContextEnum::ChatMessage(message)]));
                }
            }
        }
        spawn_span.finish_tool(PerfOutcome::Success, 1, 1, None);
        let progress_span = perf_diagnostics::span(PerfComponent::ShellProgressDrain, None, None);
        if let Some(progress_task) = progress_task {
            let _ = progress_task.await;
        }
        progress_span.finish_tool(PerfOutcome::Success, 1, 1, None);
        let duration = started.elapsed();
        let postprocess_span = perf_diagnostics::span(PerfComponent::ShellPostprocess, None, None);
        let read = exec_registry
            .read(&result.snapshot.meta.process_id, 0, None)
            .await;
        if parsed.run_in_background {
            let mut out = background_started_output(&result.snapshot);
            if let Some(warning) = sandbox_warning.as_ref() {
                out = format!("{warning}\n{out}");
            }
            if !parsed.scope_warnings.is_empty() {
                out = format!("{}\n{}", parsed.scope_warnings.join("\n"), out);
            }
            let mut message = ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(out),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                tool_failed: tool_failed_for_status(&result.snapshot.status),
                output_filter: Some(OutputFilter::no_limits()),
                extra: exec_extra(&result.snapshot, &read, duration, None, tty),
                ..Default::default()
            };
            if let Some(cache) = &cache {
                cache.attach_report(&mut message.extra);
            }
            attach_exec_path_references(
                &mut message.extra,
                gcx.clone(),
                &destination,
                &derived_privacy_zones,
                &result.snapshot,
                &parsed.command,
                &collect_combined(&read.chunks),
            )
            .await;
            if observe {
                let observation = exec_registry
                    .observation_reader(&result.snapshot.meta.process_id)
                    .await
                    .map(|reader| reader.status())
                    .unwrap_or(result.observation);
                if let Err(error) = apply_shell_privacy(
                    &gcx,
                    &parsed.command,
                    request_cwd(&result.snapshot),
                    &destination,
                    observation,
                    &derived_privacy_zones,
                    &mut message,
                )
                .await
                {
                    postprocess_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
                    return Err(error);
                }
            }
            postprocess_span.finish_tool(PerfOutcome::Success, 1, 1, None);
            return Ok((false, vec![ContextEnum::ChatMessage(message)]));
        }
        let raw_output = exec_registry
            .read_raw_capture(&result.snapshot.meta.process_id)
            .await;
        let (stdout, stderr) = collect_foreground_output(&read, raw_output.as_ref());
        let filtered_stdout = output_mini_postprocessing(&output_filter, &stdout);
        let filtered_stderr = output_mini_postprocessing(&output_filter, &stderr);

        let mut out =
            crate::integrations::integr_cmdline::format_output(&filtered_stdout, &filtered_stderr);
        if let Some(warning) = sandbox_warning.as_ref() {
            out = format!("{warning}\n{out}");
        }
        if !parsed.scope_warnings.is_empty() {
            out = format!("{}\n{}", parsed.scope_warnings.join("\n"), out);
        }
        if read.is_truncated {
            out.push_str(&format!(
                "⚠️ Output was truncated by exec transcript limits ({} bytes kept, {} bytes dropped, {} chunks truncated).\n",
                read.current_bytes, read.dropped_bytes, read.truncated_chunks
            ));
        }
        if let Some(raw_output) = raw_output.as_ref() {
            if raw_output.is_truncated() {
                out.push_str(&format!(
                    "⚠️ Raw foreground capture reached limits (stdout: {}/{} bytes kept, {} bytes elided; stderr: {}/{} bytes kept, {} bytes elided).\n",
                    raw_output.stdout_captured_bytes,
                    raw_output.stdout_max_bytes,
                    raw_output.stdout_elided_bytes,
                    raw_output.stderr_captured_bytes,
                    raw_output.stderr_max_bytes,
                    raw_output.stderr_elided_bytes
                ));
            }
        }
        append_status_line(
            &mut out,
            &result.snapshot.status,
            duration,
            timeout.unwrap_or(0),
        );
        append_sandbox_denial_hint(&mut out, &result.snapshot.status, sandbox_active);

        let mut message = ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(out),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            tool_failed: tool_failed_for_status(&result.snapshot.status),
            output_filter: Some(OutputFilter::no_limits()),
            extra: exec_extra(&result.snapshot, &read, duration, timeout, tty),
            ..Default::default()
        };
        if let Some(cache) = &cache {
            cache.attach_report(&mut message.extra);
        }
        attach_exec_path_references(
            &mut message.extra,
            gcx.clone(),
            &destination,
            &derived_privacy_zones,
            &result.snapshot,
            &parsed.command,
            &format!("{stdout}\n{stderr}"),
        )
        .await;
        if observe {
            if let Err(error) = apply_shell_privacy(
                &gcx,
                &parsed.command,
                request_cwd(&result.snapshot),
                &destination,
                result.observation,
                &derived_privacy_zones,
                &mut message,
            )
            .await
            {
                postprocess_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
                return Err(error);
            }
        }

        postprocess_span.finish_tool(PerfOutcome::Success, 1, 1, None);
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
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
            description: "Execute a single command, using the \"sh\" on unix-like systems and \"powershell.exe\" on windows. Use it for one-time tasks like dependencies installation. Don't call this unless you have to. Not suitable for regular work because it requires a confirmation at each step. Output is compressed by default - use output_filter and output_limit parameters to see specific parts if needed. Set run_in_background=true for long-running commands you will inspect later with process_read or wait with a process target. Foreground commands default to pipe streams, while background commands default to a pseudo-terminal (PTY) with interactive stdin and a combined stdout/stderr stream, except on Windows, which uses pipes for both; explicit tty overrides either default. The timeout parameter only applies to foreground commands (run_in_background=false); supplying timeout with run_in_background=true is an error — use wait with a process target or process_kill instead. In worktree-scoped chats, the default cwd and explicit workdir are enforced to the active worktree or privacy-permitted outside paths; OS confinement follows the terminal security mode. Note: sudo commands cannot be run - if you need elevated privileges, ask the user to run them directly.".to_string(),
            input_schema: shell_input_schema(),
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
            (cgcx.app.gcx.clone(), cgcx.execution_scope.clone())
        };
        if let Some(meta) = execution_scope
            .as_ref()
            .filter(|scope| scope.is_enforced() && scope.worktree().kind == "task_agent")
            .map(|scope| scope.worktree().clone())
        {
            crate::exec::command_policy::ensure_managed_task_execution(&ccx, &meta).await?;
        }
        let checkout = command_checkout::resolve(&ccx, args).await?;
        let parsed = parse_args_with_filter(
            gcx,
            args,
            &OutputFilter::default(),
            execution_scope.as_ref(),
            checkout.as_ref(),
        )
        .await?;
        let command = parsed.command;
        Ok(command)
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

fn shell_scope_refusal(error: crate::tool_jobs::ScopeChangeError) -> String {
    match error {
        crate::tool_jobs::ScopeChangeError::Closed | crate::tool_jobs::ScopeChangeError::Busy => {
            "This chat is closing or busy, so the command was not started. Try again in a moment."
                .to_string()
        }
        crate::tool_jobs::ScopeChangeError::InvalidKey => {
            "The command was not started: its workspace could not be identified.".to_string()
        }
        crate::tool_jobs::ScopeChangeError::Cancelled => {
            "The command was not started: aborted while waiting for tool admission.".to_string()
        }
    }
}

fn request_cwd(snapshot: &ExecProcessSnapshot) -> &std::path::Path {
    snapshot
        .meta
        .cwd
        .as_deref()
        .or(snapshot.meta.owner.workspace.as_deref())
        .unwrap_or_else(|| std::path::Path::new("."))
}

async fn apply_shell_privacy(
    gcx: &Arc<GlobalContext>,
    command: &str,
    cwd: &std::path::Path,
    destination: &refact_privacy::Destination,
    observation: refact_exec::ObservationStatus,
    derived_zones: &crate::privacy::records::DerivedPrivacyZones,
    message: &mut ChatMessage,
) -> Result<(), String> {
    match crate::privacy::records::apply_shell_observation(
        gcx,
        command,
        cwd,
        destination,
        observation,
        derived_zones,
        message,
    )
    .await
    .map_err(crate::privacy::records::unstable_context_notice)?
    {
        crate::privacy::records::ShellReadDecision::Pass => Ok(()),
        crate::privacy::records::ShellReadDecision::Ask => Ok(()),
    }
}

fn shell_input_schema() -> Value {
    let mut schema = json_schema_from_params(
        &[
            ("command", "string", "shell command to execute"),
            ("description", "string", "Required. A single concise active-voice sentence describing what the command does. Short for simple commands: 'List files in current directory'. More descriptive for piped/obscure commands: 'Find and delete all .tmp files recursively'. Never use words like 'complex' or 'risk'."),
            ("workdir", "string", "workdir for the command"),
            ("output_filter", "string", "Optional regex pattern to filter output lines. Only lines matching this pattern (and context) will be shown. Use to find specific errors or content in large outputs."),
            ("output_limit", "string", "Optional. Max lines to show (default: 40). Use higher values like '200' or 'all' to see more output."),
        ],
        &["command", "description"],
    );
    schema["properties"]["checkout"] = command_checkout::input_schema();
    schema["properties"]["timeout"] = json!({
        "oneOf": [
            {"type": "integer"},
            {"type": "string"}
        ],
        "description": "Optional caller-requested timeout in seconds; omitted means no execution deadline. Only applies to foreground commands; rejected when run_in_background=true.",
    });
    schema["properties"]["tty"] = json!({
        "type": "boolean",
        "default": false,
        "description": TTY_DESCRIPTION,
    });
    schema["properties"]["needs_confirmation"] = json!({
        "type": "boolean",
        "default": false,
        "description": "Set true when you know this command is destructive, irreversible, or touches something outside the workspace, and you want the user to confirm before it runs. Setting false never skips the normal safety checks."
    });
    schema["properties"]["run_in_background"] = json!({
        "type": "boolean",
        "default": false,
        "description": RUN_IN_BACKGROUND_DESCRIPTION,
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
    // Foreground output comes back as this call's tool result, which is not an
    // injection; `push` only picks when the asynchronous completion notice of a
    // run_in_background=true command lands in the chat.
    let mut push = PushMode::schema();
    push["description"] = json!(format!(
        "Only used with run_in_background=true, for the asynchronous process-completed notice. {} Foreground output is returned as this call's tool result and is never injected.",
        PushMode::schema()["description"].as_str().unwrap_or_default()
    ));
    schema["properties"]["push"] = push;
    schema
}

fn push_tail(buf: &mut String, text: &str) {
    buf.push_str(text);
    if buf.len() <= SHELL_PROGRESS_MAX_CHARS_PER_STREAM {
        return;
    }
    let keep_from = buf
        .char_indices()
        .map(|(idx, _)| idx)
        .find(|idx| buf.len() - idx <= SHELL_PROGRESS_MAX_CHARS_PER_STREAM)
        .unwrap_or(buf.len());
    buf.drain(..keep_from);
}

fn shell_progress_text(stdout: &str, stderr: &str, combined: &str) -> String {
    let mut parts = Vec::new();
    if !combined.is_empty() {
        parts.push(format!("output:\n{combined}"));
    }
    if !stdout.is_empty() {
        parts.push(format!("stdout:\n{stdout}"));
    }
    if !stderr.is_empty() {
        parts.push(format!("stderr:\n{stderr}"));
    }
    parts.join("\n")
}

async fn send_shell_progress(
    subchat_tx: &Arc<AMutex<mpsc::UnboundedSender<Value>>>,
    tool_call_id: &str,
    stdout: &str,
    stderr: &str,
    combined: &str,
) {
    let progress = shell_progress_text(stdout, stderr, combined);
    if progress.is_empty() {
        return;
    }
    let msg = json!({
        "tool_call_id": tool_call_id,
        "subchat_id": progress,
    });
    let _ = subchat_tx.lock().await.send(msg);
}

async fn emit_shell_progress(
    subchat_tx: Arc<AMutex<mpsc::UnboundedSender<Value>>>,
    tool_call_id: String,
    mut rx: mpsc::UnboundedReceiver<ExecOutputChunk>,
) {
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut combined = String::new();
    let mut last_sent_at: Option<tokio::time::Instant> = None;
    let min_interval = Duration::from_millis(SHELL_PROGRESS_MIN_INTERVAL_MS);

    while let Some(chunk) = rx.recv().await {
        match chunk.stream {
            ExecOutputStream::Stdout => push_tail(&mut stdout, &chunk.text),
            ExecOutputStream::Stderr => push_tail(&mut stderr, &chunk.text),
            ExecOutputStream::Combined => push_tail(&mut combined, &chunk.text),
        }

        let now = tokio::time::Instant::now();
        if last_sent_at
            .map(|last| now.duration_since(last) >= min_interval)
            .unwrap_or(true)
        {
            send_shell_progress(&subchat_tx, &tool_call_id, &stdout, &stderr, &combined).await;
            last_sent_at = Some(now);
        }
    }

    send_shell_progress(&subchat_tx, &tool_call_id, &stdout, &stderr, &combined).await;
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

fn collect_combined(chunks: &[ExecOutputChunk]) -> String {
    chunks
        .iter()
        .map(|chunk| chunk.text.as_str())
        .collect::<String>()
}

fn collect_foreground_output(
    read: &ExecReadResult,
    raw_output: Option<&ExecRawOutput>,
) -> (String, String) {
    raw_output
        .map(|raw| (raw.stdout.clone(), raw.stderr.clone()))
        .unwrap_or_else(|| collect_exec_output(read))
}

pub(crate) fn exec_status_label(status: &ExecStatus) -> &'static str {
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

fn exec_exit_code(status: &ExecStatus) -> Option<i32> {
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

fn append_status_line(
    out: &mut String,
    status: &ExecStatus,
    duration: Duration,
    timeout_secs: u64,
) {
    match status {
        ExecStatus::Exited { exit_code } => out.push_str(&format!(
            "The command was running {:.3}s, finished with exit code {}\n",
            duration.as_secs_f64(),
            exit_code.map(|code| code.to_string()).unwrap_or_else(|| "unknown".into())
        )),
        ExecStatus::SandboxLauncherFailed { exit_code } => out.push_str(&format!(
            "⚠️ The sandbox launcher failed before the command ran (exit code {exit_code}).\n"
        )),
        ExecStatus::Killed => out.push_str(&format!(
            "⚠️ The command was interrupted by user after {:.3}s (process killed). Output above may be incomplete.\n",
            duration.as_secs_f64()
        )),
        ExecStatus::TimedOut => out.push_str(&format!(
            "⚠️ The command timed out after {} seconds (process killed). Output above may be incomplete.\n",
            timeout_secs
        )),
        ExecStatus::Failed { message } => out.push_str(&format!(
            "⚠️ The command failed after {:.3}s: {}\n",
            duration.as_secs_f64(),
            message
        )),
        ExecStatus::Starting | ExecStatus::Running => out.push_str(&format!(
            "⚠️ The command did not reach a terminal state after {:.3}s (status: {}).\n",
            duration.as_secs_f64(),
            exec_status_label(status)
        )),
    }
}

fn append_sandbox_denial_hint(out: &mut String, status: &ExecStatus, sandbox_active: bool) {
    if !sandbox_active {
        return;
    }
    if matches!(status, ExecStatus::SandboxLauncherFailed { .. }) {
        out.push_str("Sandbox confinement could not start; the command did not run.\n");
        return;
    }
    let lower = out.to_ascii_lowercase();
    // Command output cannot prove a sandbox denial; this only supplements the typed launcher status.
    if lower.contains("permission denied")
        || lower.contains("operation not permitted")
        || lower.contains("eacces")
        || lower.contains("eperm")
    {
        out.push_str("Command output may indicate a sandbox denial. Retry with escalate:{mode, justification} if wider access is necessary.\n");
    }
}

fn background_started_output(snapshot: &ExecProcessSnapshot) -> String {
    format!(
        "Process started in background\nprocess_id: {}\nshort_description: {}\nstatus: {}\nmode: {}\nUse process_read or wait with a process target to retrieve output later.\n",
        snapshot.meta.process_id,
        snapshot.meta.short_description,
        exec_status_label(&snapshot.status),
        snapshot.meta.mode
    )
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
        ExecStatus::Exited {
            exit_code,
        } => Some(*exit_code != Some(0)),
        ExecStatus::Starting | ExecStatus::Running => None,
    }
}

fn exec_extra(
    snapshot: &ExecProcessSnapshot,
    read: &ExecReadResult,
    duration: Duration,
    timeout_secs: Option<u64>,
    tty: bool,
) -> serde_json::Map<String, Value> {
    let mut extra = serde_json::Map::new();
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
    let status_detail = serde_json::to_value(&snapshot.status).unwrap_or(Value::Null);
    let mut exec = json!({
        "process_id": snapshot.meta.process_id.as_str(),
        "status": exec_status_label(&snapshot.status),
        "status_detail": status_detail,
        "exit_code": exec_exit_code(&snapshot.status),
        "short_description": snapshot.meta.short_description,
        "command": snapshot.meta.command,
        "cwd": cwd,
        "workspace": workspace,
        "mode": snapshot.meta.mode.to_string(),
        "tty": tty,
        "duration_ms": duration.as_millis() as u64,
        "created_at_ms": snapshot.meta.created_at_ms,
        "started_at_ms": snapshot.meta.started_at_ms,
        "ended_at_ms": snapshot.meta.ended_at_ms,
        "transcript": {
            "total_bytes_appended": read.total_bytes_appended,
            "total_lines_appended": read.total_lines_appended,
            "dropped_chunks": read.dropped_chunks,
            "dropped_bytes": read.dropped_bytes,
            "truncated_chunks": read.truncated_chunks,
            "current_bytes": read.current_bytes,
            "max_bytes": read.max_bytes,
            "chunk_count": read.chunk_count,
            "is_truncated": read.is_truncated,
        }
    });
    if let Some(timeout_secs) = timeout_secs {
        exec["timeout_secs"] = json!(timeout_secs);
    }
    extra.insert("exec".to_string(), exec);
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
) {
    let cwd = snapshot
        .meta
        .cwd
        .as_deref()
        .or(snapshot.meta.owner.workspace.as_deref())
        .unwrap_or_else(|| Path::new("."));
    let workspace = snapshot.meta.owner.workspace.as_deref().unwrap_or(cwd);
    let collected = crate::exec::path_enrichment::collect_async(
        command,
        cwd,
        &[workspace.to_path_buf()],
        output,
    )
    .await;
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

async fn parse_args_with_filter(
    gcx: Arc<GlobalContext>,
    args: &HashMap<String, Value>,
    config_filter: &OutputFilter,
    execution_scope: Option<&ExecutionScope>,
    checkout: Option<&ValidatedCommandCheckout>,
) -> Result<ParsedShellArgs, String> {
    let command = match args.get("command") {
        Some(Value::String(s)) => {
            if s.is_empty() {
                return Err("Command is empty".to_string());
            }
            s.clone()
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
    let (workdir, scope_warnings) = if let Some(checkout) = checkout {
        (Some(checkout.cwd().to_path_buf()), Vec::new())
    } else {
        if args.contains_key("checkout") {
            return Err("command checkout requires validated authority".into());
        }
        resolve_shell_workdir(gcx.clone(), raw_workdir, execution_scope).await?
    };

    let has_filter_override =
        args.get("output_filter").is_some() || args.get("output_limit").is_some();
    let custom_filter = if has_filter_override {
        Some(parse_output_filter_args(args, config_filter))
    } else {
        None
    };

    let timeout = match args.get("timeout") {
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::String(s)) => Some(validate_shell_timeout_secs(
            s.parse::<u64>()
                .map_err(|_| "argument `timeout` must be seconds as an integer".to_string())?,
        )?),
        Some(Value::Number(n)) => Some(parse_numeric_shell_timeout(n)?),
        Some(v) => {
            return Err(format!(
                "argument `timeout` is not a string or number: {:?}",
                v
            ))
        }
        None => None,
    };

    let description = match args.get("description") {
        Some(Value::String(s)) if s.trim().is_empty() => {
            return Err(
                "description is required and must be a non-empty single concise sentence"
                    .to_string(),
            );
        }
        Some(Value::String(s)) => Some(s.clone()),
        Some(v) => return Err(format!("argument `description` is not a string: {:?}", v)),
        None => {
            return Err(
                "description is required and must be a non-empty single concise sentence"
                    .to_string(),
            );
        }
    };

    let tty = match args.get("tty") {
        Some(value) => Some(
            refact_tool_api::coerce_bool(value)
                .ok_or_else(|| format!("argument `tty` is not a boolean: {value:?}"))?,
        ),
        None => None,
    };
    if let Some(value) = args.get("needs_confirmation") {
        refact_tool_api::coerce_bool(value)
            .ok_or_else(|| format!("argument `needs_confirmation` is not a boolean: {value:?}"))?;
    }
    let run_in_background = match args.get("run_in_background") {
        Some(value) => refact_tool_api::coerce_bool(value)
            .ok_or_else(|| format!("argument `run_in_background` is not a boolean: {value:?}"))?,
        None => false,
    };
    let escalation = escalation_from_args(args)?;
    let push = PushMode::from_args(args)?;

    if run_in_background && timeout.is_some() {
        return Err(
            "timeout is not supported with run_in_background=true; use process_kill or wait with a process target instead"
                .to_string(),
        );
    }

    Ok(ParsedShellArgs {
        command,
        workdir,
        custom_filter,
        timeout,
        description,
        tty,
        run_in_background,
        scope_warnings,
        escalation,
        push,
    })
}

fn parse_numeric_shell_timeout(n: &serde_json::Number) -> Result<u64, String> {
    let Some(secs) = n.as_u64() else {
        return Err(format!(
            "timeout must be a positive integer (seconds), got {n}"
        ));
    };
    validate_shell_timeout_secs(secs)
}

fn validate_shell_timeout_secs(secs: u64) -> Result<u64, String> {
    if secs == 0 {
        return Err("timeout must be > 0 seconds".to_string());
    }
    if secs > MAX_SHELL_TIMEOUT_SECS {
        return Err(format!(
            "timeout exceeds maximum of {MAX_SHELL_TIMEOUT_SECS} seconds"
        ));
    }
    Ok(secs)
}

async fn shell_owner_workspace(
    gcx: Arc<GlobalContext>,
    execution_scope: Option<&ExecutionScope>,
    cwd: Option<&PathBuf>,
) -> Option<PathBuf> {
    if let Some(scope) = active_execution_scope(execution_scope) {
        return Some(normalize_workspace_path(scope.effective_root()));
    }
    if let Some(cwd) = cwd {
        if let Some(workspace) = workspace_containing_path(gcx.clone(), cwd).await {
            return Some(workspace);
        }
        return Some(normalize_workspace_path(cwd));
    }
    get_active_project_path(gcx)
        .await
        .map(|path| normalize_workspace_path(&path))
}

async fn workspace_containing_path(
    gcx: Arc<GlobalContext>,
    path: &std::path::Path,
) -> Option<PathBuf> {
    let path = normalize_workspace_path(path);
    get_unscoped_project_dirs(gcx)
        .await
        .into_iter()
        .map(|workspace| normalize_workspace_path(&workspace))
        .filter(|workspace| path.starts_with(workspace))
        .max_by_key(|workspace| workspace.components().count())
}

async fn resolve_shell_workdir(
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
            "⚠️ Worktree scope: shell cwd/workdir is enforced as '{}'; OS confinement follows the terminal security mode",
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
        let project_dirs = get_unscoped_project_dirs(gcx.clone()).await;
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::app_state::AppState;
    use crate::exec::ExecProcessId;
    use crate::exec::types::normalize_workspace_path;
    use crate::tools::tools_description::Tool;

    use super::*;

    #[test]
    fn closed_or_busy_scope_is_a_plain_sentence_for_the_model() {
        for error in [
            crate::tool_jobs::ScopeChangeError::Closed,
            crate::tool_jobs::ScopeChangeError::Busy,
        ] {
            let text = shell_scope_refusal(error);
            assert!(text.contains("closing or busy"), "{text}");
            assert!(text.contains("not started") && text.contains("Try again"));
            assert!(!text.contains("Closed") && !text.contains("Busy"));
        }
    }

    fn args(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    async fn ccx_with_gcx_and_abort(
        abort_flag: Option<Arc<AtomicBool>>,
    ) -> (Arc<GlobalContext>, Arc<AMutex<AtCommandsContext>>) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_with_abort(
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
                abort_flag,
            )
            .await,
        ));
        (gcx, ccx)
    }

    async fn detach_quickly<F: std::future::Future>(future: F) -> F::Output {
        TEST_FOREGROUND_WINDOW
            .scope(Duration::from_millis(100), future)
            .await
    }

    #[tokio::test]
    async fn shell_request_has_no_implicit_timeout_but_preserves_an_explicit_one() {
        let (_gcx, ccx) = ccx_with_gcx_and_abort(None).await;
        let base = HashMap::from([
            ("command".to_string(), json!("echo ready")),
            ("description".to_string(), json!("Check readiness")),
        ]);
        let (request, _) = approval_request_without_cache(&ccx, &base).await.unwrap();
        assert!(request.timeout.is_none());
        let mut explicit = base;
        explicit.insert("timeout".into(), json!(12));
        let (request, _) = approval_request_without_cache(&ccx, &explicit)
            .await
            .unwrap();
        assert_eq!(request.timeout, Some(Duration::from_secs(12)));
    }

    #[test]
    fn foreground_window_stays_below_tool_job_inline_budget() {
        assert!(SHELL_FOREGROUND_WINDOW < crate::chat::tools::TOOL_JOB_INLINE_BUDGET);
        assert!(
            SHELL_FOREGROUND_WINDOW + SHELL_OBSERVATION_GRACE_AFTER_EXIT
                <= crate::chat::tools::TOOL_JOB_INLINE_BUDGET
        );
        assert!(SHELL_FOREGROUND_WINDOW >= Duration::from_secs(30));
    }

    #[tokio::test]
    async fn foreground_command_longer_than_old_window_returns_inline() {
        let (gcx, ccx) = ccx_with_gcx_and_abort(None).await;
        let command = if cfg!(target_os = "windows") {
            "Start-Sleep -Milliseconds 700; [Console]::Out.Write('late-inline')"
        } else {
            "sleep 0.7; printf late-inline"
        };
        let started = std::time::Instant::now();
        let (_, messages) = ToolShell::default()
            .tool_execute(
                ccx,
                &"shell".into(),
                &args(vec![
                    ("command", json!(command)),
                    ("description", json!("Run command slower than old window")),
                ]),
            )
            .await
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(500));
        let message = only_chat_message(messages);
        assert!(exec(&message).get("detached").is_none());
        assert_eq!(exec(&message)["status"], "exited");
        assert!(text(&message).contains("late-inline"));
        let id = process_id(&message);
        assert!(gcx
            .exec_registry
            .get(&id)
            .await
            .unwrap()
            .status
            .is_terminal());
    }

    async fn ccx_with_workspace(
        workspace: &std::path::Path,
    ) -> (Arc<GlobalContext>, Arc<AMutex<AtCommandsContext>>) {
        let (gcx, ccx) = ccx_with_gcx_and_abort(None).await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.to_path_buf()];
        (gcx, ccx)
    }

    #[tokio::test]
    async fn shell_runs_when_a_workspace_folder_was_removed() {
        let existing = tempfile::tempdir().unwrap();
        let removed = tempfile::tempdir().unwrap();
        let removed_path = removed.path().to_path_buf();
        drop(removed);
        let (gcx, ccx) = ccx_with_gcx_and_abort(None).await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![existing.path().to_path_buf(), removed_path];
        let (_, messages) = ToolShell::default()
            .tool_execute(
                ccx,
                &"shell".into(),
                &args(vec![
                    ("command", json!(success_command())),
                    ("description", json!("Run with a stale workspace folder")),
                ]),
            )
            .await
            .unwrap();
        let message = only_chat_message(messages);
        assert!(text(&message).contains("hello"));
        assert_eq!(message.tool_failed, Some(false));
    }

    #[tokio::test]
    async fn shell_names_stale_folders_when_no_workspace_root_remains() {
        let removed = tempfile::tempdir().unwrap();
        let removed_path = removed.path().to_path_buf();
        drop(removed);
        let (gcx, ccx) = ccx_with_gcx_and_abort(None).await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![removed_path.clone()];
        let error = ToolShell::default()
            .tool_execute(
                ccx,
                &"shell".into(),
                &args(vec![
                    ("command", json!(success_command())),
                    ("description", json!("Run with only a stale folder")),
                ]),
            )
            .await
            .unwrap_err();
        assert!(error.contains("workspace identity unavailable"), "{error}");
        assert!(
            error.contains(&removed_path.display().to_string()),
            "{error}"
        );
    }

    async fn ccx_with_abort(abort_flag: Option<Arc<AtomicBool>>) -> Arc<AMutex<AtCommandsContext>> {
        ccx_with_gcx_and_abort(abort_flag).await.1
    }

    async fn run_shell(args: HashMap<String, Value>) -> ChatMessage {
        let (gcx, ccx) = ccx_with_gcx_and_abort(None).await;
        let mut shell = ToolShell::default();
        let (_, messages) = shell
            .tool_execute(ccx, &"shell".to_string(), &args)
            .await
            .unwrap();
        let mut message = only_chat_message(messages);
        // Output regressions inspect the completed capture of the same execution,
        // even when the shell receipt detached. Never rerun the command in tests.
        if exec(&message)["detached"] == true {
            let id = process_id(&message);
            let snapshot =
                tokio::time::timeout(Duration::from_secs(60), gcx.exec_registry.wait(&id))
                    .await
                    .unwrap()
                    .unwrap();
            let observation = match gcx.exec_registry.observation_reader(&id).await {
                Some(reader) => Some(reader.wait_status().await),
                None => None,
            };
            // Terminal execution alone does not make observed output safe to disclose.
            assert!(!matches!(
                observation,
                Some(refact_exec::ObservationStatus::Pending(_))
            ));
            let read = gcx.exec_registry.read(&id, 0, None).await;
            let raw = gcx.exec_registry.read_raw_capture(&id).await;
            let (stdout, stderr) = collect_foreground_output(&read, raw.as_ref());
            let filter = parse_output_filter_args(&args, &OutputFilter::default());
            let mut out = crate::integrations::integr_cmdline::format_output(
                &output_mini_postprocessing(&filter, &stdout),
                &output_mini_postprocessing(&filter, &stderr),
            );
            if raw.as_ref().is_some_and(|raw| raw.is_truncated()) {
                out.push_str("Raw foreground capture reached limits\n");
            }
            let timeout = args
                .get("timeout")
                .map(|value| match value {
                    Value::String(value) => value.parse().unwrap(),
                    value => value.as_u64().unwrap(),
                })
                .unwrap_or(10);
            append_status_line(&mut out, &snapshot.status, Duration::ZERO, timeout);
            message.content = ChatContent::SimpleText(out);
            message.tool_failed = tool_failed_for_status(&snapshot.status);
            message.extra = exec_extra(
                &snapshot,
                &read,
                Duration::ZERO,
                Some(timeout),
                snapshot.meta.tty,
            );
            if let Some(observation) = observation {
                apply_shell_privacy(
                    &gcx,
                    &snapshot.meta.command,
                    request_cwd(&snapshot),
                    &crate::privacy::records::provider_destination("model"),
                    observation,
                    &Default::default(),
                    &mut message,
                )
                .await
                .unwrap();
            }
            assert_eq!(process_id(&message), id);
            assert_eq!(gcx.exec_registry.list(Default::default()).await.len(), 1);
        }
        message
    }

    async fn shell_error(args: HashMap<String, Value>) -> String {
        let ccx = ccx_with_abort(None).await;
        let mut shell = ToolShell::default();
        shell
            .tool_execute(ccx, &"shell".to_string(), &args)
            .await
            .unwrap_err()
    }

    fn only_chat_message(messages: Vec<ContextEnum>) -> ChatMessage {
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

    fn success_command() -> String {
        if cfg!(target_os = "windows") {
            "[Console]::Out.Write('hello')".to_string()
        } else {
            "printf hello".to_string()
        }
    }

    fn delayed_success_command() -> String {
        if cfg!(target_os = "windows") {
            "Start-Sleep -Milliseconds 20; [Console]::Out.Write('hello')".to_string()
        } else {
            "sleep 0.02; printf hello".to_string()
        }
    }

    fn stderr_command() -> String {
        if cfg!(target_os = "windows") {
            "[Console]::Error.Write('warn')".to_string()
        } else {
            "printf warn >&2".to_string()
        }
    }

    fn nonzero_command() -> String {
        if cfg!(target_os = "windows") {
            "[Console]::Out.Write('bad'); exit 7".to_string()
        } else {
            "printf bad; exit 7".to_string()
        }
    }

    fn slow_command() -> String {
        if cfg!(target_os = "windows") {
            "[Console]::Out.WriteLine('start'); Start-Sleep -Seconds 5".to_string()
        } else {
            "printf start; sleep 5".to_string()
        }
    }

    fn background_sleep_command() -> String {
        if cfg!(target_os = "windows") {
            "Start-Sleep -Seconds 30".to_string()
        } else {
            "sleep 30".to_string()
        }
    }

    fn background_readable_command() -> String {
        if cfg!(target_os = "windows") {
            "[Console]::Out.Write('background-ready'); Start-Sleep -Seconds 30".to_string()
        } else {
            "printf background-ready; sleep 30".to_string()
        }
    }

    fn multiline_command() -> String {
        if cfg!(target_os = "windows") {
            "[Console]::Out.Write(\"line1`nline2`nline3`nline4`nline5`n\")".to_string()
        } else {
            "printf 'line1\nline2\nline3\nline4\nline5\n'".to_string()
        }
    }

    fn late_marker_command(marker: &str) -> String {
        if cfg!(target_os = "windows") {
            format!(
                "$chunk = ('f' * 1024) + \"`n\"; [Console]::Out.Write($chunk * 1024); [Console]::Out.Write('{marker}`n'); $tail = ('t' * 1024) + \"`n\"; [Console]::Out.Write($tail * 3072)"
            )
        } else {
            format!(
                "chunk=$(printf '%01024d\\n' 0 | tr '0' f); i=0; while [ $i -lt 1024 ]; do printf '%s\\n' \"$chunk\"; i=$((i + 1)); done; printf '%s\\n' '{marker}'; chunk=$(printf '%01024d\\n' 0 | tr '0' t); i=0; while [ $i -lt 3072 ]; do printf '%s\\n' \"$chunk\"; i=$((i + 1)); done"
            )
        }
    }

    fn above_raw_capture_limit_command() -> String {
        if cfg!(target_os = "windows") {
            "$max = 16 * 1024 * 1024; $tail = \"`nsmall1`nsmall2`nsmall3`nsmall4`n\"; $line = ('x' * 1024) + \"`n\"; $count = [math]::Floor(($max - $tail.Length - 64) / $line.Length); for ($i = 0; $i -lt $count; $i++) { [Console]::Out.Write($line) }; [Console]::Out.Write('x' * ($max - $tail.Length - 64 - ($count * $line.Length))); [Console]::Out.Write($tail); [Console]::Out.Write(('y' * 1024) * 1024)".to_string()
        } else {
            "max_bytes=$((16 * 1024 * 1024)); tail='\nsmall1\nsmall2\nsmall3\nsmall4\n'; tail_len=29; line=$(printf '%01024d\\n' 0 | tr '0' x); line_len=1025; count=$(((max_bytes - tail_len - 64) / line_len)); i=0; while [ $i -lt $count ]; do printf '%s\\n' \"$line\"; i=$((i + 1)); done; rem=$((max_bytes - tail_len - 64 - count * line_len)); if [ $rem -gt 0 ]; then printf '%*s' \"$rem\" '' | tr ' ' x; fi; printf '%b' \"$tail\"; chunk=$(printf '%01024d\\n' 0 | tr '0' y); i=0; while [ $i -lt 1024 ]; do printf '%s\\n' \"$chunk\"; i=$((i + 1)); done".to_string()
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

    #[test]
    fn sandbox_denial_result_suggests_escalation() {
        let mut output = "sh: cannot create /root/file: Permission denied\n".to_string();

        append_sandbox_denial_hint(
            &mut output,
            &ExecStatus::Exited {
                exit_code: Some(1),
            },
            true,
        );

        assert!(output.contains("Retry with escalate"));
    }

    #[test]
    fn ordinary_eacces_without_sandbox_is_not_reported_as_sandbox_denial() {
        let mut output = "sh: cannot open file: EACCES\n".to_string();

        append_sandbox_denial_hint(
            &mut output,
            &ExecStatus::Exited {
                exit_code: Some(1),
            },
            false,
        );

        assert!(!output.contains("sandbox denial"));
    }

    #[test]
    fn unknown_exit_is_failed_without_claiming_exit_zero() {
        let status = ExecStatus::Exited {
            exit_code: None,
        };
        let mut output = "retained output\n".to_string();
        append_status_line(&mut output, &status, Duration::from_secs(1), 10);
        assert_eq!(tool_failed_for_status(&status), Some(true));
        assert_eq!(exec_status_label(&status), "exited");
        assert_eq!(
            output,
            "retained output\nThe command was running 1.000s, finished with exit code unknown\n"
        );
    }

    #[test]
    fn typed_launcher_failure_is_primary_sandbox_signal() {
        let mut output = String::new();

        append_sandbox_denial_hint(
            &mut output,
            &ExecStatus::SandboxLauncherFailed {
                exit_code: 125,
            },
            true,
        );

        assert!(output.contains("confinement could not start"));
        assert!(!output.contains("Retry with escalate"));
    }

    #[tokio::test]
    async fn shell_exec_success_contains_output_and_metadata() {
        let message = run_shell(args(vec![
            ("command", json!(success_command())),
            ("description", json!("Run hello")),
        ]))
        .await;

        let body = text(&message);
        let exec = exec(&message);
        assert!(body.contains("hello"));
        assert!(body.contains("exit code 0"));
        assert_eq!(exec["status"], "exited");
        assert_eq!(exec["exit_code"], 0);
        assert_eq!(exec["short_description"], "Run hello");
        assert_eq!(exec["tty"], false);
        assert!(exec["process_id"].as_str().unwrap().starts_with("exec_"));
        assert_eq!(message.tool_failed, Some(false));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_receipt_scope_change_cancels_and_drains_actual_monitor() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, ccx) = ccx_with_workspace(workspace.path()).await;
        let (jobs, execution_abort) = {
            let context = ccx.lock().await;
            (
                context.app.runtime.tool_jobs.clone(),
                context.execution_abort.clone(),
            )
        };
        let (_, messages) = detach_quickly(ToolShell::default().tool_execute(
            ccx,
            &"scope-shell".into(),
            &args(vec![
                ("command", json!("printf ready; sleep 30")),
                (
                    "description",
                    json!("Hold actual process for scope cancellation"),
                ),
            ]),
        ))
        .await
        .unwrap();
        let message = only_chat_message(messages);
        assert_eq!(exec(&message)["detached"], true);
        let id = process_id(&message);
        let root =
            dunce::simplified(&std::fs::canonicalize(workspace.path()).unwrap()).to_path_buf();
        let keys = [crate::tool_jobs::ScopeKey::Worktree {
            project: root.clone(),
            root,
        }];
        let transition = jobs.scopes.begin_scope_change(&keys);
        tokio::pin!(transition);
        assert!(
            futures::poll!(&mut transition).is_pending(),
            "receipt must retain the actual process lease"
        );
        assert!(
            execution_abort.load(Ordering::Acquire),
            "scope change signals the process flag"
        );
        let change = tokio::time::timeout(Duration::from_secs(10), transition)
            .await
            .unwrap()
            .unwrap();
        assert!(gcx
            .exec_registry
            .get(&id)
            .await
            .unwrap()
            .status
            .is_terminal());
        drop(change);
    }

    #[tokio::test]
    async fn foreground_shell_detaches_same_id_and_keeps_timeout() {
        let (gcx, ccx) = ccx_with_gcx_and_abort(None).await;
        let mut shell = ToolShell::default();
        let (_, messages) = detach_quickly(shell.tool_execute(
            ccx,
            &"shell".into(),
            &args(vec![
                ("command", json!(slow_command())),
                ("description", json!("Run foreground timeout test")),
                ("timeout", json!(1)),
            ]),
        ))
        .await
        .unwrap();
        let message = only_chat_message(messages);
        assert_eq!(exec(&message)["detached"], true);
        assert_eq!(exec(&message)["mode"], "foreground");
        assert_eq!(exec(&message)["tty"], false);
        assert_eq!(exec(&message)["timeout_secs"], 1);
        assert_eq!(message.tool_failed, None);
        assert!(!text(&message).contains("start"));
        let id = process_id(&message);
        assert_eq!(gcx.exec_registry.list(Default::default()).await.len(), 1);
        let terminal = tokio::time::timeout(Duration::from_secs(10), gcx.exec_registry.wait(&id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(terminal.meta.process_id, id);
        assert_eq!(terminal.status, ExecStatus::TimedOut);
        assert_eq!(gcx.exec_registry.list(Default::default()).await.len(), 1);
        let mut output = String::new();
        append_status_line(&mut output, &terminal.status, Duration::ZERO, 1);
        assert!(output.contains("timed out after 1 seconds"));
        assert_eq!(tool_failed_for_status(&terminal.status), Some(true));
    }

    #[tokio::test]
    async fn run_in_background_true_returns_immediately() {
        let (gcx, ccx) = ccx_with_gcx_and_abort(None).await;
        let mut shell = ToolShell::default();
        let started = tokio::time::Instant::now();
        let (_, messages) = shell
            .tool_execute(
                ccx,
                &"shell".to_string(),
                &args(vec![
                    ("command", json!(background_sleep_command())),
                    ("description", json!("Run background sleep test")),
                    ("run_in_background", json!(true)),
                ]),
            )
            .await
            .unwrap();
        let elapsed = started.elapsed();
        let message = only_chat_message(messages);
        let process_id = process_id(&message);

        assert!(
            elapsed < Duration::from_secs(5),
            "background shell returned after {elapsed:?}"
        );
        let snapshot = gcx.exec_registry.get(&process_id).await.unwrap();
        assert!(matches!(
            snapshot.status,
            ExecStatus::Starting | ExecStatus::Running
        ));
        assert!(text(&message).contains(process_id.as_str()));
        assert_eq!(exec(&message)["process_id"], process_id.as_str());
        assert_eq!(
            exec(&message)["short_description"],
            "Run background sleep test"
        );
        assert_eq!(exec(&message)["mode"], "background");
        assert_eq!(exec(&message)["status"], "running");
        assert_eq!(exec(&message)["tty"], refact_exec::default_tty());
        assert!(exec(&message)["timeout_secs"].is_null());

        gcx.exec_registry.kill(&process_id).await.unwrap();
    }

    #[tokio::test]
    async fn run_in_background_sets_workspace_from_resolved_cwd() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, ccx) = ccx_with_workspace(workspace.path()).await;
        let mut shell = ToolShell::default();
        let (_, messages) = shell
            .tool_execute(
                ccx,
                &"shell".to_string(),
                &args(vec![
                    ("command", json!(background_sleep_command())),
                    ("description", json!("Run background sleep with workspace")),
                    ("run_in_background", json!(true)),
                ]),
            )
            .await
            .unwrap();
        let message = only_chat_message(messages);
        let process_id = process_id(&message);
        let snapshot = gcx.exec_registry.get(&process_id).await.unwrap();

        assert_eq!(
            snapshot.meta.owner.workspace,
            Some(normalize_workspace_path(workspace.path()))
        );
        assert_eq!(
            exec(&message)["workspace"],
            normalize_workspace_path(workspace.path())
                .to_string_lossy()
                .as_ref()
        );

        gcx.exec_registry.kill(&process_id).await.unwrap();
    }

    #[tokio::test]
    async fn run_in_background_process_is_readable_via_process_read() {
        let (gcx, ccx) = ccx_with_gcx_and_abort(None).await;
        let mut shell = ToolShell::default();
        let (_, messages) = shell
            .tool_execute(
                ccx.clone(),
                &"shell".to_string(),
                &args(vec![
                    ("command", json!(background_readable_command())),
                    ("description", json!("Run readable background test")),
                    ("run_in_background", json!(true)),
                ]),
            )
            .await
            .unwrap();
        let message = only_chat_message(messages);
        let process_id = process_id(&message);
        wait_for_output(gcx.clone(), &process_id, "background-ready").await;

        let mut read_tool = crate::tools::tool_process::ToolProcessRead {
            config_path: String::new(),
        };
        let (_, read_messages) = read_tool
            .tool_execute(
                ccx,
                &"process_read".to_string(),
                &args(vec![("process_id", json!(process_id.as_str()))]),
            )
            .await
            .unwrap();
        let read_message = only_chat_message(read_messages);

        assert!(text(&read_message).contains("background-ready"));
        assert_eq!(exec(&read_message)["process_id"], process_id.as_str());

        gcx.exec_registry.kill(&process_id).await.unwrap();
    }

    #[tokio::test]
    async fn background_shell_explicit_tty_false_uses_pipes() {
        let (gcx, ccx) = ccx_with_gcx_and_abort(None).await;
        let mut shell = ToolShell::default();
        let (_, messages) = shell
            .tool_execute(
                ccx,
                &"shell".to_string(),
                &args(vec![
                    ("command", json!(background_sleep_command())),
                    ("description", json!("Run background shell through pipes")),
                    ("run_in_background", json!(true)),
                    ("tty", json!(false)),
                ]),
            )
            .await
            .unwrap();
        let message = only_chat_message(messages);
        let process_id = process_id(&message);

        assert_eq!(exec(&message)["tty"], false);
        assert!(!gcx.exec_registry.get(&process_id).await.unwrap().meta.tty);
        gcx.exec_registry.kill(&process_id).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn tty_true_uses_pty() {
        let message = run_shell(args(vec![
            ("command", json!(stderr_command())),
            ("description", json!("Run stderr through pty")),
            ("tty", json!(true)),
        ]))
        .await;

        assert!(text(&message).contains("warn"));
        assert!(!text(&message).contains("STDERR"));
        assert_eq!(exec(&message)["tty"], true);
        assert_eq!(exec(&message)["status"], "exited");
        assert_eq!(message.tool_failed, Some(false));
    }

    #[tokio::test]
    async fn shell_exec_captures_stderr() {
        let message = run_shell(args(vec![
            ("command", json!(stderr_command())),
            ("description", json!("Capture stderr output")),
        ]))
        .await;
        let body = text(&message);

        assert!(body.contains("STDERR"));
        assert!(body.contains("warn"));
        assert_eq!(exec(&message)["status"], "exited");
    }

    #[tokio::test]
    async fn shell_exec_reports_nonzero_exit() {
        let message = run_shell(args(vec![
            ("command", json!(nonzero_command())),
            ("description", json!("Report nonzero exit code")),
        ]))
        .await;
        let body = text(&message);

        assert!(body.contains("bad"));
        assert!(body.contains("exit code 7"));
        assert_eq!(exec(&message)["status"], "exited");
        assert_eq!(exec(&message)["exit_code"], 7);
        assert_eq!(message.tool_failed, Some(true));
    }

    #[tokio::test]
    async fn shell_exec_timeout_returns_partial_output_and_failed_metadata() {
        let message = run_shell(args(vec![
            ("command", json!(slow_command())),
            ("description", json!("Run slow command with short timeout")),
            ("timeout", json!(1)),
        ]))
        .await;
        assert_eq!(exec(&message)["status"], "timed_out");
        assert_eq!(exec(&message)["timeout_secs"], 1);
        assert_eq!(message.tool_failed, Some(true));
        if !cfg!(windows) {
            assert!(text(&message).contains("start"));
        }
    }

    #[tokio::test]
    async fn shell_timeout_rejects_negative() {
        let err = shell_error(args(vec![
            ("command", json!(success_command())),
            ("description", json!("Reject negative timeout")),
            ("timeout", json!(-1)),
        ]))
        .await;

        assert_eq!(err, "timeout must be a positive integer (seconds), got -1");
    }

    #[tokio::test]
    async fn shell_timeout_rejects_fractional() {
        let err = shell_error(args(vec![
            ("command", json!(success_command())),
            ("description", json!("Reject fractional timeout")),
            ("timeout", json!(1.5)),
        ]))
        .await;

        assert_eq!(err, "timeout must be a positive integer (seconds), got 1.5");
    }

    #[tokio::test]
    async fn shell_timeout_rejects_zero() {
        let err = shell_error(args(vec![
            ("command", json!(success_command())),
            ("description", json!("Reject zero timeout")),
            ("timeout", json!(0)),
        ]))
        .await;

        assert_eq!(err, "timeout must be > 0 seconds");
    }

    #[tokio::test]
    async fn shell_timeout_rejects_above_max() {
        let err = shell_error(args(vec![
            ("command", json!(success_command())),
            ("description", json!("Reject above max timeout")),
            ("timeout", json!(99_999_999)),
        ]))
        .await;

        assert_eq!(err, "timeout exceeds maximum of 3600 seconds");
    }

    #[tokio::test]
    async fn shell_timeout_accepts_positive() {
        let message = run_shell(args(vec![
            ("command", json!(success_command())),
            ("description", json!("Accept positive timeout")),
            ("timeout", json!(30)),
        ]))
        .await;

        assert_eq!(exec(&message)["timeout_secs"], 30);
    }

    #[tokio::test]
    async fn shell_timeout_accepts_string() {
        let message = run_shell(args(vec![
            ("command", json!(success_command())),
            ("description", json!("Accept string timeout")),
            ("timeout", json!("60")),
        ]))
        .await;

        assert_eq!(exec(&message)["timeout_secs"], 60);
    }

    #[tokio::test]
    async fn background_shell_rejects_timeout() {
        let err = shell_error(args(vec![
            ("command", json!(background_sleep_command())),
            ("description", json!("Background with timeout should fail")),
            ("run_in_background", json!(true)),
            ("timeout", json!(10)),
        ]))
        .await;

        assert_eq!(
            err,
            "timeout is not supported with run_in_background=true; use process_kill or wait with a process target instead"
        );
    }

    #[tokio::test]
    async fn shell_exec_abort_returns_partial_output_and_failed_metadata() {
        let abort_flag = Arc::new(AtomicBool::new(false));
        let (gcx, ccx) = ccx_with_gcx_and_abort(Some(abort_flag.clone())).await;
        let mut shell = ToolShell::default();
        let tool_call_id = "shell".to_string();
        let (_, messages) = detach_quickly(shell.tool_execute(
            ccx.clone(),
            &tool_call_id,
            &args(vec![
                ("command", json!(slow_command())),
                ("description", json!("Run slow command for abort test")),
                ("timeout", json!(10)),
            ]),
        ))
        .await
        .unwrap();
        let message = only_chat_message(messages);
        assert_eq!(exec(&message)["detached"], true);
        assert_eq!(message.tool_failed, None);
        let id = process_id(&message);
        assert_eq!(gcx.exec_registry.list(Default::default()).await.len(), 1);
        wait_for_output(gcx.clone(), &id, "start").await;
        abort_flag.store(true, Ordering::Relaxed);
        let terminal = tokio::time::timeout(Duration::from_secs(10), gcx.exec_registry.wait(&id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(terminal.meta.process_id, id);
        assert_eq!(terminal.status, ExecStatus::Killed);
        assert_eq!(tool_failed_for_status(&terminal.status), Some(true));
        if let Some(reader) = gcx.exec_registry.observation_reader(&id).await {
            assert!(!matches!(
                reader.wait_status().await,
                refact_exec::ObservationStatus::Pending(_)
            ));
        }
        let mut read = crate::tools::tool_process::ToolProcessRead {
            config_path: String::new(),
        };
        let (_, messages) = read
            .tool_execute(
                ccx,
                &"read-aborted".into(),
                &args(vec![("process_id", json!(id.as_str()))]),
            )
            .await
            .unwrap();
        let output = only_chat_message(messages);
        assert_eq!(process_id(&output), id);
        assert_eq!(exec(&output)["status"], "killed");
        assert!(text(&output).contains("start"));
        assert_eq!(gcx.exec_registry.list(Default::default()).await.len(), 1);
    }

    #[tokio::test]
    async fn foreground_output_filter_finds_late_match() {
        let marker = "MARKER_FOREGROUND_LATE_MATCH";
        let message = run_shell(args(vec![
            ("command", json!(late_marker_command(marker))),
            ("description", json!("Find late marker in large output")),
            ("timeout", json!(20)),
            ("output_filter", json!(marker)),
            ("output_limit", json!("8")),
        ]))
        .await;
        let body = text(&message);

        assert!(body.contains(marker));
        assert!(body.contains("filtered"));
        assert_eq!(exec(&message)["status"], "exited");
    }

    #[tokio::test]
    async fn foreground_output_filter_bounded() {
        let message = run_shell(args(vec![
            ("command", json!(above_raw_capture_limit_command())),
            (
                "description",
                json!("Generate output above raw capture limit"),
            ),
            ("timeout", json!(20)),
            ("output_filter", json!("small|bytes elided")),
            ("output_limit", json!("20")),
        ]))
        .await;
        let body = text(&message);

        assert!(body.contains("bytes elided]"));
        assert!(body.contains("Raw foreground capture reached limits"));
        assert!(body.len() < 100_000);
        assert_eq!(exec(&message)["status"], "exited");
    }

    #[tokio::test]
    async fn description_required_rejects_missing() {
        let ccx = ccx_with_abort(None).await;
        let mut shell = ToolShell::default();
        let err = shell
            .tool_execute(
                ccx,
                &"shell".to_string(),
                &args(vec![("command", json!(success_command()))]),
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
        let ccx = ccx_with_abort(None).await;
        let mut shell = ToolShell::default();
        let err = shell
            .tool_execute(
                ccx,
                &"shell".to_string(),
                &args(vec![
                    ("command", json!(success_command())),
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
}
