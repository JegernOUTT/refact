use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::Instant;

use refact_chat_api::{PythonCallApproval, PythonCallRecord, PythonCallStatus, ThreadParams};
use refact_python_catalog::PythonCatalog;
use refact_python_kernel::protocol::HostResult;
use refact_runtime_api::{ToolCatalogSnapshot, TurnToolPool};
use serde_json::{json, Value};
use tokio::sync::{Mutex as AMutex, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatToolCall, MultimodalElement};
use crate::chat::tools::{
    check_tools_confirmation_reporting_auto, execute_tools, is_editing_tool, ExecuteToolsOptions,
};
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{
    get_project_dir_string, revalidate_rewritten_tool_call, run_control_hooks, ControlVerdict,
    HookPayload,
};
use super::approvals::{ApprovalAsk, ApprovalOutcome};
use super::envelope::{build_envelope, preview};

pub trait CallSink: Send + Sync {
    fn call_started(&self, record: &PythonCallRecord);
    fn call_finished(&self, record: &PythonCallRecord);
}

/// Supply the outer tool's trusted context and the session's thread/catalog snapshot.
/// Approvals use `ccx`; execution freezes its call-specific approval into an abort-isolated context.
pub struct NestedDispatchConfig {
    pub app: AppState,
    pub ccx: Arc<AMutex<AtCommandsContext>>,
    pub chat_id: String,
    pub outer_tool_call_id: String,
    pub run_id: String,
    pub catalog: Arc<PythonCatalog>,
    pub thread: ThreadParams,
    pub mode_id: String,
    pub model_id: Option<String>,
    pub allowed_tools: Vec<String>,
    pub tool_catalog: Option<Arc<ToolCatalogSnapshot>>,
    pub turn_tool_pool: Option<TurnToolPool>,
    pub max_nested_calls: usize,
    pub max_parallel_calls: usize,
    pub max_nested_result_bytes: usize,
    pub run_dir: PathBuf,
    pub cancel: CancellationToken,
    pub sink: Arc<dyn CallSink>,
    /// `llm()` budget of this run; `None` when the helper is disabled.
    pub llm: Option<Arc<super::llm::LlmBudget>>,
    /// Session taint, nested results' and direct reads' privacy collected so far in this run.
    pub run_privacy: Arc<std::sync::Mutex<super::llm::RunPrivacy>>,
    /// `engine()` state of this run; `None` when engine reads are switched off.
    pub engine: Option<Arc<super::engine_fns::EngineRunState>>,
    /// PROJECT_ROOT of the run; `git()` reads the repository that holds it.
    pub project_root: PathBuf,
    /// The chat's workspace folders; `git()` lists and reads nothing outside them.
    pub workspace_folders: Vec<PathBuf>,
    /// The read rules of the run's file access, which `git()` applies to every file it shows.
    pub deny_read: refact_python_kernel::protocol::PathDenyRules,
    /// The chat's Python workspace, whose privacy ledger labels `run()` logs.
    pub workspace_dir: PathBuf,
}

pub struct NestedDispatcher {
    pub(crate) config: NestedDispatchConfig,
    charged: AtomicUsize,
    images: AtomicUsize,
    diff_bytes: AtomicUsize,
    pub(crate) parallel: Arc<Semaphore>,
    /// Every nested call of the run holds a read guard while it runs; an engine change holds
    /// the write guard from before its preview until its handler ended (C15, "nothing in flight").
    pub(crate) gate: Arc<tokio::sync::RwLock<()>>,
}

pub struct NestedOutcome {
    pub result: HostResult,
    pub record: PythonCallRecord,
    pub privacy: Vec<Value>,
    pub images: Vec<MultimodalElement>,
    pub stop_run: bool,
}

impl NestedDispatcher {
    pub fn new(config: NestedDispatchConfig) -> Self {
        Self {
            parallel: Arc::new(Semaphore::new(config.max_parallel_calls.max(1))),
            config,
            charged: AtomicUsize::new(0),
            images: AtomicUsize::new(0),
            diff_bytes: AtomicUsize::new(0),
            gate: Arc::new(tokio::sync::RwLock::new(())),
        }
    }

    /// Waits while an engine change of this run is pending; `None` when the run was cancelled first.
    pub(crate) async fn enter_call(&self) -> Option<tokio::sync::OwnedRwLockReadGuard<()>> {
        tokio::select! {
            biased;
            _ = self.config.cancel.cancelled() => None,
            guard = self.gate.clone().read_owned() => Some(guard),
        }
    }

    pub(crate) fn charged_calls(&self) -> usize {
        self.charged.load(Ordering::Relaxed)
    }

    /// Charges one call against `python_max_nested_calls`; false when exhausted.
    pub(crate) fn charge_nested_call(&self) -> bool {
        self.charged
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < self.config.max_nested_calls).then(|| n + 1)
            })
            .is_ok()
    }

    pub fn help_text(&self, name: &str) -> Option<String> {
        self.config.catalog.help_text(name)
    }
    pub fn tool_names(&self) -> Vec<String> {
        self.config
            .catalog
            .functions
            .iter()
            .chain(&self.config.catalog.helpers)
            .map(|f| f.py_name.clone())
            .collect()
    }

    pub async fn dispatch(
        &self,
        seq: u32,
        function: &str,
        kwargs: Value,
        too_large: Option<String>,
    ) -> NestedOutcome {
        self.dispatch_at(seq, function, kwargs, too_large, (None, None))
            .await
    }

    pub async fn dispatch_at(
        &self,
        seq: u32,
        function: &str,
        kwargs: Value,
        too_large: Option<String>,
        site: (Option<u32>, Option<u32>),
    ) -> NestedOutcome {
        self.dispatch_with(seq, function, kwargs, too_large, site, None)
            .await
    }

    /// `run()`: one nested `shell` call under the shell's whole policy, projected into a
    /// `RunResult` from the output the shell kept, with that output as a log under RUN_DIR/logs.
    pub async fn dispatch_run(
        &self,
        seq: u32,
        kwargs: Value,
        too_large: Option<String>,
        site: (Option<u32>, Option<u32>),
    ) -> NestedOutcome {
        let shell = self
            .config
            .catalog
            .functions
            .iter()
            .find(|f| f.tool_name == refact_python_catalog::RUN_TOOL)
            .map(|f| f.py_name.clone());
        let parsed = match (too_large, shell) {
            (Some(size), _) => Err(format!(
                "run() arguments are too large to send: {size}; pass a shorter command"
            )),
            (None, None) => Err("run() is not available: this chat has no shell tool".to_string()),
            (None, Some(shell)) => {
                refact_postprocessing::run_helper::parse_args(&kwargs).map(|args| (shell, args))
            }
        };
        match parsed {
            Ok((shell, args)) => {
                let shell_kwargs = refact_postprocessing::run_helper::shell_kwargs(&args);
                let projection = RunProjection {
                    tail: args.tail,
                    kwargs,
                };
                self.dispatch_with(seq, &shell, shell_kwargs, None, site, Some(&projection))
                    .await
            }
            Err(message) => {
                let mut record =
                    self.new_record(seq, refact_python_catalog::RUN_HELPER, &kwargs, site);
                self.config.sink.call_started(&record);
                record.status = PythonCallStatus::Failed;
                record.summary = preview(&message);
                self.finish_call(
                    seq,
                    record,
                    Instant::now(),
                    &kwargs,
                    None,
                    HostResult::Err {
                        exc_type: "TypeError".into(),
                        message,
                    },
                    Vec::new(),
                    Vec::new(),
                    false,
                )
                .await
            }
        }
    }

    pub(super) fn new_record(
        &self,
        seq: u32,
        tool: &str,
        kwargs: &Value,
        site: (Option<u32>, Option<u32>),
    ) -> PythonCallRecord {
        PythonCallRecord {
            seq,
            call_id: format!("py_{}_{}", self.config.run_id, seq),
            tool: tool.into(),
            args_preview: super::approvals::args_preview(&kwargs.to_string(), 1000, 200),
            status: PythonCallStatus::Running,
            summary: String::new(),
            started_at_ms: crate::agents::approval::now_ms(),
            duration_ms: None,
            approval: PythonCallApproval::NotNeeded,
            diffs: Vec::new(),
            line: site.0,
            col: site.1,
            model: None,
            tokens_in: None,
            tokens_out: None,
        }
    }

    /// `PreToolUse` for a call of this run's script, with the ids that tie it to the run.
    pub(crate) async fn pre_tool_use(
        &self,
        tool: &str,
        args: Value,
        call_id: &str,
    ) -> ControlVerdict {
        let c = &self.config;
        let mut payload = HookPayload::new(
            &HookEvent::PreToolUse,
            c.chat_id.clone(),
            get_project_dir_string(c.app.clone()).await,
        )
        .with_tool(tool, Some(args));
        payload
            .extra
            .insert("python_run_id".into(), json!(c.run_id));
        payload.extra.insert(
            "python_parent_tool_call_id".into(),
            json!(c.outer_tool_call_id),
        );
        payload.extra.insert("tool_call_id".into(), json!(call_id));
        run_control_hooks(c.app.clone(), HookEvent::PreToolUse, payload).await
    }

    async fn dispatch_with(
        &self,
        seq: u32,
        function: &str,
        kwargs: Value,
        too_large: Option<String>,
        site: (Option<u32>, Option<u32>),
        projection: Option<&RunProjection>,
    ) -> NestedOutcome {
        let start = Instant::now();
        let mut record = self.new_record(seq, function, &kwargs, site);
        self.config.sink.call_started(&record);
        let awaiting_approval = AtomicBool::new(false);
        let preflight = tokio::select! {
            biased;
            _ = self.config.cancel.cancelled() => Err(("RuntimeError", "python run cancelled".into(), true)),
            result = self.dispatch_inner(function, &kwargs, too_large.as_deref(), &mut record, &awaiting_approval, projection.is_some()) => result,
        };
        if awaiting_approval.load(Ordering::Acquire) && preflight.is_err() {
            record.approval = PythonCallApproval::Cancelled;
        }
        let outcome = match preflight {
            Ok((call, messages, thread)) => self.execute(call, messages, thread).await,
            Err(error) => Err(error),
        };
        let outcome = match (outcome, projection) {
            (Ok((messages, false)), Some(projection)) => {
                let built =
                    build_envelope(&messages, self.config.max_nested_result_bytes, &self.images);
                match self
                    .project_run(seq, &record.call_id, &messages, projection.tail)
                    .await
                {
                    Ok((value, summary)) => {
                        record.status = if value["ok"] == true {
                            PythonCallStatus::Ok
                        } else {
                            PythonCallStatus::Failed
                        };
                        record.summary = summary;
                        Ok((HostResult::Ok(value), built.privacy, Vec::new(), false))
                    }
                    Err(error) => Err(error),
                }
            }
            (Ok((messages, stopped)), _) => {
                if record.tool == refact_python_catalog::RUN_TOOL {
                    self.note_unobserved_output(&record.call_id, &messages)
                        .await;
                }
                let built =
                    build_envelope(&messages, self.config.max_nested_result_bytes, &self.images);
                record.status = if built.envelope["ok"] == true {
                    PythonCallStatus::Ok
                } else {
                    PythonCallStatus::Failed
                };
                record.summary = built.summary;
                let bytes = serde_json::to_vec(&built.diffs).map_or(usize::MAX, |b| b.len());
                if self
                    .diff_bytes
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                        used.checked_add(bytes).filter(|n| *n <= 256 * 1024)
                    })
                    .is_ok()
                {
                    record.diffs = built.diffs;
                } else {
                    record.summary.push_str(" [diffs dropped: message budget]");
                }
                Ok((
                    HostResult::Ok(built.envelope),
                    built.privacy,
                    built.images,
                    stopped,
                ))
            }
            (Err(error), _) => Err(error),
        };
        let (result, privacy, images, stop_run) = match outcome {
            Ok(done) => done,
            Err((exc_type, message, stopped)) => {
                record.status = if stopped {
                    PythonCallStatus::Cancelled
                } else if exc_type == "PermissionError" {
                    PythonCallStatus::Denied
                } else {
                    PythonCallStatus::Failed
                };
                record.summary = preview(&message);
                (
                    HostResult::Err {
                        exc_type: exc_type.into(),
                        message,
                    },
                    Vec::new(),
                    Vec::new(),
                    stopped,
                )
            }
        };
        self.finish_call(
            seq,
            record,
            start,
            &kwargs,
            projection.map(|projection| &projection.kwargs),
            result,
            privacy,
            images,
            stop_run,
        )
        .await
    }

    /// Marks the run when the script receives output of this call's shell process that the exec
    /// layer did not observe.
    async fn note_unobserved_output(&self, call_id: &str, messages: &[ChatMessage]) {
        let Some(process_id) = messages
            .iter()
            .find(|m| m.role == "tool" && m.tool_call_id == call_id)
            .filter(|message| {
                message
                    .extra
                    .get(refact_tool_api::tool_output::TOOL_OUTPUT_EXTRA_KEY)
                    .is_some_and(|output| output["output_withheld"] == false)
            })
            .and_then(|message| message.extra.get("exec")?["process_id"].as_str())
        else {
            return;
        };
        let process_id = crate::exec::ExecProcessId(process_id.to_string());
        if self
            .config
            .app
            .runtime
            .exec_registry
            .observation_reader(&process_id)
            .await
            .is_none()
        {
            self.config.run_privacy.lock().unwrap().unobserved_output = true;
        }
    }

    /// Records the call's privacy, writes `calls/<seq>_<tool>.json` and reports it finished.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn finish_call(
        &self,
        seq: u32,
        mut record: PythonCallRecord,
        start: Instant,
        kwargs: &Value,
        run_kwargs: Option<&Value>,
        result: HostResult,
        privacy: Vec<Value>,
        images: Vec<MultimodalElement>,
        stop_run: bool,
    ) -> NestedOutcome {
        if !privacy.is_empty() {
            self.config
                .run_privacy
                .lock()
                .unwrap()
                .taint
                .extend(privacy.iter().cloned());
        }
        record.duration_ms = Some(start.elapsed().as_millis().min(u64::MAX as u128) as u64);
        let envelope = match &result {
            HostResult::Ok(value) => value.clone(),
            HostResult::Err {
                exc_type,
                message,
            } => json!({"exception":exc_type,"message":message}),
        };
        let mut artifact = json!({"seq":seq,"call_id":record.call_id,"tool":record.tool,"kwargs":kwargs,"envelope":envelope,"record":record});
        if let Some(run_kwargs) = run_kwargs {
            artifact["run_kwargs"] = run_kwargs.clone();
        }
        let name: String = record
            .tool
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let dir = self.config.run_dir.join("calls");
        if let Err(error) = async {
            tokio::fs::create_dir_all(&dir).await?;
            tokio::fs::write(
                dir.join(format!("{seq}_{name}.json")),
                serde_json::to_vec(&artifact)?,
            )
            .await
        }
        .await
        {
            tracing::warn!(%error, "cannot persist nested tool call");
        }
        self.config.sink.call_finished(&record);
        NestedOutcome {
            result,
            record,
            privacy,
            images,
            stop_run,
        }
    }

    /// The `RunResult` of a finished shell call. Output comes only from a shell result that
    /// cleared it for the script, of this call's own ended process; what the exec layer kept of
    /// it becomes a log that is labeled in the privacy ledger before it holds anything. Anything
    /// less gives the withheld result and no log, and Stop leaves no log.
    async fn project_run(
        &self,
        seq: u32,
        call_id: &str,
        messages: &[ChatMessage],
        tail: usize,
    ) -> Result<(Value, String), (&'static str, String, bool)> {
        use refact_postprocessing::run_helper::{result, withheld, RunEnd};
        let c = &self.config;
        let cancelled = || ("RuntimeError", "python run cancelled".to_string(), true);
        let Some(message) = messages
            .iter()
            .find(|m| m.role == "tool" && m.tool_call_id == call_id)
        else {
            return Ok(withheld(
                None,
                0,
                "the shell returned no result, so there is no output and no log",
            ));
        };
        let exec = message.extra.get("exec").cloned().unwrap_or(Value::Null);
        let exit_code = exec["exit_code"].as_i64();
        let duration_ms = exec["duration_ms"].as_u64().unwrap_or(0);
        if message
            .extra
            .get("privacy_shell")
            .is_some_and(|shell| shell["withheld"] != false || shell["ask_pending"] != false)
        {
            return Ok(withheld(
                exit_code,
                duration_ms,
                "output withheld by the privacy policy",
            ));
        }
        let Some(process_id) = exec["process_id"].as_str() else {
            let started = !c
                .app
                .runtime
                .exec_registry
                .list(crate::exec::ExecProcessFilter {
                    chat_id: Some(c.chat_id.clone()),
                    tool_call_id: Some(call_id.to_string()),
                    ..Default::default()
                })
                .await
                .is_empty();
            if started {
                return Ok(withheld(
                    None,
                    duration_ms,
                    "output withheld: the shell gave no result for the command it started",
                ));
            }
            let text = message.content.content_text_only();
            return Ok(result(
                &RunEnd {
                    status: "failed",
                    exit_code: None,
                    duration_ms: 0,
                    output: "",
                    log: None,
                    truncated: false,
                    failure: Some(text.trim()),
                },
                tail,
            ));
        };
        if exec["detached"] == true {
            return Ok(withheld(
                None,
                duration_ms,
                "the shell detached the command before it ended, so there is no output and no log",
            ));
        }
        if message
            .extra
            .get(refact_tool_api::tool_output::TOOL_OUTPUT_EXTRA_KEY)
            .is_none_or(|output| output["output_withheld"] != false)
        {
            return Ok(withheld(
                exit_code,
                duration_ms,
                "output withheld: the shell did not clear it for the script",
            ));
        }
        if message
            .extra
            .get("privacy_observation")
            .is_some_and(|observation| observation["incomplete"] != false)
        {
            return Ok(withheld(
                exit_code,
                duration_ms,
                "output withheld: the privacy observation of the command did not complete",
            ));
        }
        let registry = &c.app.runtime.exec_registry;
        let process_id = crate::exec::ExecProcessId(process_id.to_string());
        let Some(snapshot) = registry.get(&process_id).await.filter(|snapshot| {
            snapshot.status.is_terminal()
                && snapshot.meta.owner.chat_id.as_deref() == Some(c.chat_id.as_str())
                && snapshot.meta.owner.tool_call_id.as_deref() == Some(call_id)
        }) else {
            return Ok(withheld(
                exit_code,
                duration_ms,
                "output withheld: the exec layer holds no ended process of this call",
            ));
        };
        let (status, exit_code, failure) = match &snapshot.status {
            crate::exec::ExecStatus::Exited {
                exit_code,
            } => ("exited", exit_code.map(i64::from), None),
            crate::exec::ExecStatus::TimedOut => ("timed_out", None, None),
            crate::exec::ExecStatus::Killed => ("killed", None, None),
            crate::exec::ExecStatus::Failed {
                message,
            } => ("failed", None, Some(message.clone())),
            crate::exec::ExecStatus::SandboxLauncherFailed {
                exit_code,
            } => (
                "failed",
                None,
                Some(format!("the sandbox launcher exited with code {exit_code}")),
            ),
            crate::exec::ExecStatus::Starting | crate::exec::ExecStatus::Running => {
                ("failed", None, None)
            }
        };
        let kept = tokio::select! {
            biased;
            _ = c.cancel.cancelled() => return Err(cancelled()),
            kept = retained_output(registry, &process_id) => kept,
        };
        let Some((output, appended)) = kept else {
            return Ok(withheld(
                exit_code,
                duration_ms,
                "output withheld: the exec layer holds no output of this call",
            ));
        };
        let path = run_log_path(&c.run_dir, seq);
        let log = refact_python_kernel::hostfs::virtual_path(
            &path,
            if cfg!(windows) {
                refact_python_kernel::hostfs::Platform::Windows
            } else {
                refact_python_kernel::hostfs::Platform::Unix
            },
        );
        let mut projected = result(
            &RunEnd {
                status,
                exit_code,
                duration_ms,
                output: &output,
                log: Some(log),
                truncated: output.len() < appended,
                failure: failure.as_deref(),
            },
            tail,
        );
        let max = c.max_nested_result_bytes as u64;
        refact_postprocessing::run_helper::fit_result(&mut projected.0, |value| {
            use refact_python_kernel::convert::{fits_result, json_size, result_nodes_for};
            fits_result(&json_size(value), result_nodes_for(max), max)
        });
        let observed = registry.observation_reader(&process_id).await.is_some();
        let staged = self.stage_run_log(&path, message, &output, observed).await;
        let published = match staged {
            Ok(staging) if !c.cancel.is_cancelled() => {
                let renamed = tokio::fs::rename(&staging, &path)
                    .await
                    .map_err(|error| error.to_string());
                if renamed.is_err() {
                    let _ = tokio::fs::remove_file(&staging).await;
                    let _ = tokio::fs::remove_file(&path).await;
                }
                renamed
            }
            Ok(staging) => {
                let _ = tokio::fs::remove_file(&staging).await;
                let _ = tokio::fs::remove_file(&path).await;
                return Err(cancelled());
            }
            Err(error) => Err(error),
        };
        if c.cancel.is_cancelled() {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(cancelled());
        }
        published.map_err(|error| {
            (
                "RuntimeError",
                format!(
                    "run(): the command ended ({status}) but its log could not be written: {error}"
                ),
                false,
            )
        })?;
        if !observed {
            c.run_privacy.lock().unwrap().unobserved_output = true;
        }
        Ok(projected)
    }

    /// Creates the log empty and a file under a temporary name next to it, gives both the
    /// strictest zone of the shell result's privacy records, saves the log's label in the
    /// workspace ledger and syncs the ledger's folder, and only then writes the output into the
    /// temporary file, which the caller renames over the log. A process the exec layer did not
    /// observe (no zone kept the output from this provider) has no records, so its log gets every
    /// zone of the policy. On any failure both files are removed.
    async fn stage_run_log(
        &self,
        path: &std::path::Path,
        message: &ChatMessage,
        output: &str,
        observed: bool,
    ) -> Result<PathBuf, String> {
        let c = &self.config;
        let mut reads = match message.extra.get("privacy") {
            Some(value) => {
                serde_json::from_value::<refact_privacy::PrivacyRecord>(value.clone())
                    .map_err(|error| {
                        format!("the shell's privacy records are unreadable: {error}")
                    })?
                    .files
            }
            None => Vec::new(),
        };
        let folder = path.parent().ok_or("log path has no folder")?;
        tokio::fs::create_dir_all(folder)
            .await
            .map_err(|error| error.to_string())?;
        let staging = folder.join(format!(
            ".{}.{}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            uuid::Uuid::new_v4().simple()
        ));
        let staged = async {
            for file in [path, staging.as_path()] {
                tokio::fs::write(file, b"")
                    .await
                    .map_err(|error| error.to_string())?;
            }
            let derived = c.ccx.lock().await.derived_privacy_zones.clone();
            let policy = c.app.gcx.privacy_policy_load.read().unwrap().policy.clone();
            if !observed {
                reads.extend(super::llm::unobserved_output_records(&policy));
            }
            crate::privacy::records::inherit_observed_write_zones(
                &c.app.gcx,
                &policy,
                &reads,
                [path.to_path_buf(), staging.clone()],
                &derived,
            )?;
            super::run_output::save_ledger(
                &c.workspace_dir,
                [path.to_path_buf()],
                &c.app.gcx,
                &derived,
            )
            .await?;
            super::workspace::sync_dir(&c.workspace_dir)
                .await
                .map_err(|error| format!("the privacy ledger is not durable: {error}"))?;
            tokio::fs::write(&staging, output.as_bytes())
                .await
                .map_err(|error| error.to_string())
        }
        .await;
        match staged {
            Ok(()) => Ok(staging),
            Err(error) => {
                let _ = tokio::fs::remove_file(&staging).await;
                let _ = tokio::fs::remove_file(path).await;
                Err(error)
            }
        }
    }

    async fn dispatch_inner(
        &self,
        function: &str,
        kwargs: &Value,
        too_large: Option<&str>,
        record: &mut PythonCallRecord,
        awaiting_approval: &AtomicBool,
        run: bool,
    ) -> Result<(ChatToolCall, Vec<ChatMessage>, ThreadParams), (&'static str, String, bool)> {
        let c = &self.config;
        let entry = c
            .catalog
            .resolve(function)
            .filter(|f| f.is_async)
            .ok_or_else(|| {
                (
                    "RuntimeError",
                    format!("unknown function: {function}"),
                    false,
                )
            })?;
        let tool = if function == "mcp_search" {
            "mcp_tool_search"
        } else {
            &entry.tool_name
        };
        record.tool = tool.into();
        if c.tool_catalog
            .as_ref()
            .is_some_and(|catalog| !catalog.index.tools.iter().any(|entry| entry.name == tool))
        {
            return Err((
                "PermissionError",
                "tool is outside chat tool set".into(),
                false,
            ));
        }
        if let Some(size) = too_large {
            return Err((
                "ValueError",
                format!("{function}() arguments are too large to send: {size}; pass a file path or a smaller value"),
                false,
            ));
        }
        let normalized = normalize_optional_nulls(&entry.input_schema, kwargs);
        let kwargs = &normalized;
        validate(&entry.input_schema, kwargs)?;
        if !self.charge_nested_call() {
            return Err(("RuntimeError", "nested call budget exhausted".into(), false));
        }
        authorize_tool(&c.ccx, c.tool_catalog.as_deref(), &c.allowed_tools, tool)
            .await
            .map_err(|error| ("PermissionError", error, false))?;
        let mut args = kwargs.clone();
        match self.pre_tool_use(tool, args.clone(), &record.call_id).await {
            ControlVerdict::Allow => {}
            ControlVerdict::Deny(reason) => return Err(("PermissionError", reason, false)),
            ControlVerdict::Rewrite(value) => {
                let authority = (!c.allowed_tools.is_empty()).then(|| {
                    crate::subchat::ToolsPolicy::Only(
                        c.allowed_tools
                            .iter()
                            .map(|s| crate::agents::tool_authority::canonical(s))
                            .collect(),
                    )
                });
                args = Value::Object(
                    revalidate_rewritten_tool_call(tool, &value, authority.as_ref())
                        .map_err(|e| ("PermissionError", e.to_string(), false))?,
                );
                validate(&entry.input_schema, &args)?;
                record.args_preview = super::approvals::args_preview(&args.to_string(), 1000, 200);
            }
        }
        if run && args["run_in_background"] == true {
            return Err((
                "RuntimeError",
                "run(): a PreToolUse hook turned the command into a background one; run() waits for a command in the foreground, so nothing was started".to_string(),
                false,
            ));
        }
        let call: ChatToolCall = serde_json::from_value(json!({"id":record.call_id,"type":"function","function":{"name":tool,"arguments":args.to_string()}})).map_err(|e| ("RuntimeError", e.to_string(), false))?;
        let messages = c.ccx.lock().await.messages.clone();
        let thread =
            if let Some(session) = c.app.chat.sessions.read().await.get(&c.chat_id).cloned() {
                session.lock().await.thread.clone()
            } else {
                c.thread.clone()
            };
        let (asks, denials, auto_suppressed) = check_tools_confirmation_reporting_auto(
            c.app.clone(),
            std::slice::from_ref(&call),
            &messages,
            &thread,
            c.tool_catalog.as_deref(),
            &c.mode_id,
            c.model_id.as_deref(),
            &c.allowed_tools,
            "python",
            c.turn_tool_pool.as_ref(),
        )
        .await;
        if let Some(denial) = denials.first() {
            return Err(("PermissionError", denial.rule.clone(), false));
        }
        if asks.is_empty() && auto_suppressed.contains(&call.id) {
            record.approval = PythonCallApproval::Auto;
        }
        for ask in asks {
            awaiting_approval.store(true, Ordering::Release);
            let outcome = c
                .app
                .python_tool
                .approvals
                .request_with_context(
                    &c.app,
                    ApprovalAsk {
                        chat_id: c.chat_id.clone(),
                        tool_call_id: c.outer_tool_call_id.clone(),
                        run_id: c.run_id.clone(),
                        call_id: call.id.clone(),
                        tool_name: tool.into(),
                        args_preview: call.function.arguments.clone(),
                        command: (!ask.command.is_empty() && ask.command != tool)
                            .then_some(ask.command),
                        rule: ask.rule,
                        editing_tool: is_editing_tool(tool),
                        engine: None,
                    },
                    &c.ccx,
                    &call,
                )
                .await;
            awaiting_approval.store(false, Ordering::Release);
            match outcome {
                ApprovalOutcome::Allowed {
                    for_chat,
                } => {
                    record.approval = if for_chat {
                        PythonCallApproval::AllowedForChat
                    } else {
                        PythonCallApproval::Allowed
                    }
                }
                ApprovalOutcome::AllowedForRun => {
                    record.approval = PythonCallApproval::AllowedForRun
                }
                ApprovalOutcome::Denied => {
                    record.approval = PythonCallApproval::Denied;
                    return Err(("PermissionError", "rejected by the user".into(), false));
                }
                ApprovalOutcome::Stopped | ApprovalOutcome::Cancelled => {
                    record.approval = PythonCallApproval::Cancelled;
                    return Err(("RuntimeError", "python run stopped".into(), true));
                }
            }
        }
        Ok((call, messages, thread))
    }

    async fn execute(
        &self,
        call: ChatToolCall,
        messages: Vec<ChatMessage>,
        thread: ThreadParams,
    ) -> Result<(Vec<ChatMessage>, bool), (&'static str, String, bool)> {
        let c = &self.config;
        let _permit = tokio::select! {
            biased;
            _ = c.cancel.cancelled() => return Err(("RuntimeError", "python run cancelled".into(), true)),
            permit = self.parallel.acquire() => permit.map_err(|_| ("RuntimeError", "dispatcher closed".into(), true))?,
        };
        let (subchat_tool_parameters, postprocess_settings) = {
            let context = c.ccx.lock().await;
            (
                context.subchat_tool_parameters.clone(),
                context.postprocess_parameters.clone(),
            )
        };
        let native_abort = Arc::new(AtomicBool::new(false));
        let (nested_context, outer_abort) = {
            let mut context = c.ccx.lock().await;
            let outer_abort = context.abort_flag.clone();
            let mut nested = context
                .freeze_tool_job(&call.id, native_abort.clone())
                .map_err(|e| ("RuntimeError", e, false))?;
            nested.execution_abort = native_abort.clone();
            (Arc::new(AMutex::new(nested)), outer_abort)
        };
        let calls = [call];
        let app = c.app.clone();
        let mode_id = c.mode_id.clone();
        let model_id = c.model_id.clone();
        let options = ExecuteToolsOptions {
            structured_output: true,
            wait_until_terminal: true,
            native_abort: Some(native_abort.clone()),
            execution_context: Some(nested_context),
            allowed_tools: c.allowed_tools.clone(),
            catalog: c.tool_catalog.clone(),
            turn_tool_pool: c.turn_tool_pool.clone(),
            subchat_tool_parameters: Some(subchat_tool_parameters),
            postprocess_settings: Some(postprocess_settings),
            results_for_script: true,
            ..Default::default()
        };
        // The drainer owns the foreground claim until nested execution is terminal.
        struct ParentRegistration {
            app: AppState,
            key: (String, String),
        }
        impl Drop for ParentRegistration {
            fn drop(&mut self) {
                self.app
                    .python_tool
                    .nested_parents
                    .lock()
                    .unwrap()
                    .remove(&self.key);
            }
        }
        let key = (c.chat_id.clone(), calls[0].id.clone());
        c.app
            .python_tool
            .nested_parents
            .lock()
            .unwrap()
            .insert(key.clone(), c.outer_tool_call_id.clone());
        let registration = ParentRegistration {
            app: c.app.clone(),
            key,
        };
        let mut execution = tokio::spawn(async move {
            let _registration = registration;
            execute_tools(
                app,
                &calls,
                &messages,
                &thread,
                &mode_id,
                model_id.as_deref(),
                options,
            )
            .await
        });
        tokio::select! {
            result = &mut execution => result.map(|r| (r.0, false)).map_err(|e| ("RuntimeError", e.to_string(), true)),
            _ = async {
                loop {
                    if outer_abort.load(Ordering::Acquire) { break; }
                    tokio::select! {
                        _ = c.cancel.cancelled() => break,
                        _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
                    }
                }
            } => {
                native_abort.store(true, Ordering::Release);
                // Cancellation is a request, not proof of terminal execution. Keep the
                // outer owner (and semaphore permit) alive until the drainer settles;
                // otherwise a scope mutation could pass a still-running nested tool.
                let warning = tokio::time::sleep(std::time::Duration::from_secs(5));
                tokio::pin!(warning);
                let mut warned = false;
                loop {
                    tokio::select! {
                        _ = &mut execution => break,
                        _ = &mut warning, if !warned => {
                            warned = true;
                            tracing::warn!(chat_id = %c.chat_id, run_id = %c.run_id,
                                "cancelled Python run waiting for nested execution to settle");
                        }
                        _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                            // Only engine shutdown may escape this drain. Admission is
                            // permanently closed then; the spawned drainer still owns
                            // ParentRegistration until its execution actually ends.
                            if crate::tools::tool_python::engine_shutting_down(&c.app) {
                                break;
                            }
                        }
                    }
                }
                Err(("RuntimeError", "python run cancelled".into(), true))
            }
        }
    }
}

/// What `run()` adds to its one nested shell call: the tail length and its own arguments.
pub(crate) struct RunProjection {
    tail: usize,
    kwargs: Value,
}

fn run_log_path(run_dir: &std::path::Path, seq: u32) -> PathBuf {
    run_dir.join("logs").join(format!("{seq}_run.log"))
}

/// What the exec layer kept of a process's output in arrival order (its spill file when there
/// is one, else its in-memory transcript) and how many bytes the process printed.
async fn retained_output(
    registry: &crate::exec::ExecRegistry,
    process_id: &crate::exec::ExecProcessId,
) -> Option<(String, usize)> {
    let read = registry.read(process_id, 0, None).await;
    if !read.found {
        return None;
    }
    let spilled = match &read.disk_log_path {
        Some(path) => tokio::fs::read(path).await.ok(),
        None => None,
    };
    let output = match spilled {
        Some(bytes) => String::from_utf8(bytes)
            .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned()),
        None => read
            .chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect(),
    };
    Some((output, read.total_bytes_appended))
}

/// Whether the chat may dispatch `tool`: registered in its tool set, inside the command's
/// allowed tools, the live child ceiling and a background agent's saved tools. Nested calls
/// and the read-only rule of script writes share it.
pub(crate) async fn authorize_tool(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    tool_catalog: Option<&ToolCatalogSnapshot>,
    allowed_tools: &[String],
    tool: &str,
) -> Result<(), String> {
    if tool_catalog
        .is_some_and(|catalog| !catalog.index.tools.iter().any(|entry| entry.name == tool))
    {
        return Err("tool is outside chat tool set".into());
    }
    if !allowed_tools.is_empty()
        && !allowed_tools.iter().any(|name| {
            crate::agents::tool_authority::canonical(name)
                == crate::agents::tool_authority::canonical(tool)
        })
    {
        return Err("tool is outside allowed tools".into());
    }
    if ccx
        .lock()
        .await
        .child_tool_ceiling
        .as_ref()
        .is_some_and(|ceiling| !crate::agents::tool_authority::allows_dispatch(ceiling, tool))
    {
        return Err("outside accepted child tool authority".into());
    }
    crate::agents::tool_authority::ensure_context_tool(ccx, tool).await
}

fn validate(schema: &Value, args: &Value) -> Result<(), (&'static str, String, bool)> {
    refact_tool_api::schema_validate::validate(schema, args).map_err(|errors| {
        (
            "TypeError",
            errors
                .iter()
                .map(|e| e.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
            false,
        )
    })
}

#[cfg(test)]
#[path = "dispatcher_tests.rs"]
mod tests;

#[cfg(test)]
mod optional_null_tests {
    use super::*;
    #[test]
    fn optional_nonnullable_null_is_omitted_but_required_and_nullable_survive() {
        let schema = json!({"type":"object","required":["required"],"properties":{"optional":{"type":"integer"},"required":{"type":"integer"},"nullable":{"type":["integer","null"]}}});
        let normalized = normalize_optional_nulls(
            &schema,
            &json!({"optional":null,"required":null,"nullable":null}),
        );
        assert!(normalized.get("optional").is_none());
        assert!(normalized.get("required").is_some());
        assert!(normalized.get("nullable").is_some());
        assert!(validate(&schema, &normalized).is_err());
    }
}

pub(crate) fn normalize_optional_nulls(schema: &Value, kwargs: &Value) -> Value {
    let mut kwargs = kwargs.clone();
    if let Some(object) = kwargs.as_object_mut() {
        object.retain(|name, value| {
            if !value.is_null() {
                return true;
            }
            let required = schema["required"]
                .as_array()
                .is_some_and(|keys| keys.iter().any(|key| key.as_str() == Some(name.as_str())));
            required || validate(&property_schema(schema, name), &Value::Null).is_ok()
        });
    }
    kwargs
}

fn property_schema(root: &Value, name: &str) -> Value {
    let mut property = root["properties"][name].clone();
    if let Some(object) = property.as_object_mut() {
        for key in ["$defs", "definitions"] {
            if let Some(defs) = root.get(key) {
                object.entry(key).or_insert_with(|| defs.clone());
            }
        }
    }
    property
}
