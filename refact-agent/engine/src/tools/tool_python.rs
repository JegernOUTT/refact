use std::{
    collections::HashMap,
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use refact_chat_api::{PythonCallRecord, PythonRunUpdate, PythonRunSnapshot, PythonRunLiveStatus};
use refact_python_kernel::{client::ClientEvent, protocol::*};
use crate::{
    app_state::AppState,
    at_commands::at_commands::AtCommandsContext,
    call_validation::{ChatMessage, ChatContent, ContextEnum},
    privacy::classification_cache::Snapshot,
    python_tool::{CallSink, NestedDispatcher, NestedDispatchConfig, workspace::PythonWorkspace},
    tools::tools_description::{Tool, ToolDesc},
};

#[derive(Default)]
pub struct ToolPython {
    pub description: String,
}
#[derive(Clone)]
pub(crate) struct Invocation {
    pub app: AppState,
    pub catalog: Arc<refact_runtime_api::ToolCatalogSnapshot>,
    pub pool: Option<refact_runtime_api::TurnToolPool>,
    pub mode: String,
    pub foreground: bool,
}
tokio::task_local! { pub(crate) static INVOCATION: Invocation; }

struct Progress {
    app: AppState,
    chat: String,
    call: String,
    run: String,
    calls: Mutex<Vec<PythonCallRecord>>,
    updates: tokio::sync::mpsc::UnboundedSender<PythonRunUpdate>,
}
impl Progress {
    async fn update(&self, update: PythonRunUpdate) {
        if let Some(session) = self.app.chat.sessions.read().await.get(&self.chat).cloned() {
            let mut session = session.lock().await;
            if let Some(snapshot) = session
                .runtime
                .python_runs
                .iter_mut()
                .find(|r| r.run_id == self.run)
            {
                match &update {
                    PythonRunUpdate::CallStarted {
                        record,
                    }
                    | PythonRunUpdate::CallFinished {
                        record,
                    } => {
                        if let Some(old) = snapshot.calls.iter_mut().find(|c| c.seq == record.seq) {
                            *old = record.clone();
                        } else {
                            snapshot.calls.push(record.clone());
                            if snapshot.calls.len() > 200 {
                                snapshot.calls.remove(0);
                            }
                        }
                    }
                    PythonRunUpdate::Stdout {
                        text,
                    } => {
                        snapshot.stdout_tail.push_str(text);
                        let offset = snapshot
                            .stdout_tail
                            .char_indices()
                            .map(|(i, _)| i)
                            .find(|i| snapshot.stdout_tail.len() - i <= 4096)
                            .unwrap_or(0);
                        snapshot.stdout_tail.drain(..offset);
                    }
                    PythonRunUpdate::Detached {} => {
                        snapshot.status = PythonRunLiveStatus::Background
                    }
                    PythonRunUpdate::Display {
                        display,
                    } => snapshot.displays.push(display.clone()),
                    _ => {}
                }
            }
            session.emit(crate::chat::types::ChatEvent::PythonRunUpdate {
                tool_call_id: self.call.clone(),
                run_id: self.run.clone(),
                update,
            });
        }
    }
}
impl CallSink for Progress {
    fn call_started(&self, record: &PythonCallRecord) {
        let mut calls = self.calls.lock().unwrap();
        calls.push(record.clone());
        if calls.len() > 200 {
            calls.remove(0);
        }
        drop(calls);
        self.send(PythonRunUpdate::CallStarted {
            record: record.clone(),
        });
    }
    fn call_finished(&self, record: &PythonCallRecord) {
        let mut calls = self.calls.lock().unwrap();
        if let Some(old) = calls.iter_mut().find(|call| call.seq == record.seq) {
            *old = record.clone();
        }
        drop(calls);
        self.send(PythonRunUpdate::CallFinished {
            record: record.clone(),
        });
    }
}
impl Progress {
    fn send(&self, update: PythonRunUpdate) {
        let _ = self.updates.send(update);
    }
}
struct RunCleanup {
    app: AppState,
    chat: String,
    call: String,
    run: String,
}
impl Drop for RunCleanup {
    fn drop(&mut self) {
        self.app
            .python_tool
            .detached
            .lock()
            .unwrap()
            .remove(&(self.chat.clone(), self.call.clone()));
        if let Ok(sessions) = self.app.chat.sessions.try_read() {
            if let Some(session) = sessions.get(&self.chat) {
                if let Ok(mut session) = session.try_lock() {
                    session.runtime.python_runs.retain(|r| r.run_id != self.run);
                    return;
                }
            }
        }
        let app = self.app.clone();
        let chat = self.chat.clone();
        let run = self.run.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Some(session) = app.chat.sessions.read().await.get(&chat).cloned() {
                    session
                        .lock()
                        .await
                        .runtime
                        .python_runs
                        .retain(|r| r.run_id != run);
                }
            });
        }
    }
}
#[async_trait]
impl Tool for ToolPython {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        if let Ok(invocation) = INVOCATION.try_with(Clone::clone) {
            ccx.lock().await.app = invocation.app;
        }
        let reserved_ticket = {
            let context = ccx.lock().await;
            let ticket = context
                .app
                .python_tool
                .tickets
                .lock()
                .unwrap()
                .remove(&(context.chat_id.clone(), id.clone()));
            ticket
        };
        refact_tool_api::schema_validate::validate(
            &self.tool_description().input_schema,
            &json!(args),
        )
        .map_err(|e| format!("Invalid Python arguments: {e:?}"))?;
        let (mut app, chat, scope, model, abort) = {
            let c = ccx.lock().await;
            (
                c.app.clone(),
                c.chat_id.clone(),
                c.execution_scope.clone(),
                c.current_model.clone(),
                c.abort_flag.clone(),
            )
        };
        if let Ok(invocation) = INVOCATION.try_with(Clone::clone) {
            app = invocation.app;
            ccx.lock().await.app = app.clone();
        }
        let (root, workspace_folders) = match scope {
            Some(scope) => {
                scope.ensure_active_root()?;
                (
                    scope.effective_root().to_path_buf(),
                    scope.effective_project_dirs(),
                )
            }
            None => (
                crate::files_correction::get_active_project_path(app.gcx.clone())
                    .await
                    .ok_or("Python requires an active project")?,
                crate::files_correction::get_project_dirs(app.gcx.clone()).await,
            ),
        };
        let settings = crate::runtime_settings::coordination_for_project(&root).await?;
        if !settings.python_tool_enabled {
            return Err(
                "Python tool is disabled for this project (python_tool_enabled=false)".into(),
            );
        }
        let ticket =
            reserved_ticket.unwrap_or_else(|| app.python_tool.registry.reserve_turn(&chat, id));
        let mut lease = tokio::select! {
            result = ticket.acquire() => result.map_err(|e| e.to_string())?,
            _ = async {
                loop {
                    if abort.load(Ordering::Acquire) || app.gcx.shutdown_flag.load(Ordering::Acquire) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            } => return Err("Python turn cancelled while queued".into()),
        };
        let workspace = PythonWorkspace::for_chat(&app, &chat).await?;
        workspace.ensure_dirs().await?;
        let derived = ccx.lock().await.derived_privacy_zones.clone();
        crate::python_tool::run_output::restore_ledger(workspace.dir(), &app.gcx, &derived)
            .await
            .map_err(|reason| format!("Cannot restore Python privacy ledger: {reason}"))?;
        let parent_artifacts =
            crate::python_tool::workspace::parent_artifacts_dir(&app, &chat).await;
        if let Some(parent) = &parent_artifacts {
            crate::python_tool::run_output::import_parent_ledger(parent, &app.gcx, &derived)
                .await
                .map_err(|reason| format!("Cannot verify parent Python artifacts: {reason}"))?;
        }
        let run = format!("{:08x}", rand::random::<u32>());
        let guard = app.python_tool.register_run(&chat, id, &run);
        let cancel = guard.cancellation_token();
        let mut start = lease
            .ensure_ready_at(
                &app,
                args.get("restart") == Some(&Value::Bool(true)),
                root.clone(),
            )
            .await?;
        if args.get("restart") == Some(&Value::Bool(true)) {
            start.reset_reason = Some("requested restart".into());
        }
        let run_dir = workspace.run_dir(&run);
        let _active_path = crate::python_tool::workspace::ActiveRunPath::new(run_dir.clone());
        tokio::fs::create_dir_all(&run_dir)
            .await
            .map_err(|e| e.to_string())?;
        let started = crate::agents::approval::now_ms();
        let requested_background = args.get("run_in_background") == Some(&Value::Bool(true));
        let foreground_only = INVOCATION.try_with(|i| i.foreground).unwrap_or(false);
        let background = requested_background && !foreground_only;
        let code = args["code"].as_str().unwrap().to_owned();
        let labels = crate::python_tool::render::run_labels(
            args.get("description").and_then(Value::as_str),
            &code,
        );
        let mut failed_run = crate::python_tool::workspace::FailedRunGuard::new(
            run_dir.clone(),
            run.clone(),
            started,
            background,
        )
        .with_labels(labels.clone());
        crate::python_tool::workspace::write_running_run_json(
            &run_dir, &run, started, background, &labels,
        )
        .await?;
        tokio::fs::write(run_dir.join("code.py"), &code)
            .await
            .map_err(|e| e.to_string())?;
        let invocation = INVOCATION.try_with(Clone::clone).ok();
        let (thread, mode) =
            if let Some(session) = app.chat.sessions.read().await.get(&chat).cloned() {
                let s = session.lock().await;
                (
                    s.thread.clone(),
                    invocation
                        .as_ref()
                        .map(|i| i.mode.clone())
                        .unwrap_or_else(|| "agent".into()),
                )
            } else {
                (refact_chat_api::ThreadParams::default(), "agent".into())
            };
        let tool_catalog = match &invocation {
            Some(i) => i.catalog.clone(),
            None => {
                app.tool_registry
                    .acquire_tool_catalog(&mode, Some(&model), None)
                    .await
            }
        };
        let llm_enabled = settings.python_max_llm_calls > 0;
        let allowed_tools =
            if let Some(session) = app.chat.sessions.read().await.get(&chat).cloned() {
                session.lock().await.active_command.allowed_tools.clone()
            } else {
                vec![]
            };
        let restricted_child = !allowed_tools.is_empty()
            || ccx
                .lock()
                .await
                .child_tool_ceiling
                .as_ref()
                .is_some_and(|ceiling| !matches!(ceiling, crate::subchat::ToolsPolicy::All))
            || crate::tools::tools_list::python_engine_hidden(&app, &chat).await;
        let mut options =
            crate::python_tool::catalog_binding::BindOptions::for_run(app.gcx.clone(), &settings)
                .await;
        options.engine &= !restricted_child;
        let catalog = Arc::new(crate::python_tool::catalog_binding::catalog_with_options(
            &tool_catalog.index.tools,
            tool_catalog.index.mcp_lazy_mode,
            options,
        ));
        let engine_state = options
            .engine
            .then(|| Arc::new(crate::python_tool::engine_fns::EngineRunState::new()));
        let llm_budget = llm_enabled.then(|| {
            Arc::new(crate::python_tool::llm::LlmBudget::new(
                settings.python_max_llm_calls,
                settings.python_max_llm_tokens,
            ))
        });
        let run_privacy = Arc::new(Mutex::new(crate::python_tool::llm::RunPrivacy::default()));
        let writable =
            script_writes(&app, &ccx, &tool_catalog, &allowed_tools, &mode, &model).await;
        let (mut deny_write, mut deny_read) = protected_paths(&app, &root, &workspace).await;
        deny_write.read_only = (!writable).then(|| read_only_reason(&workspace.artifacts_dir()));
        deny_read.allow_prefixes = vec![workspace.dir().to_path_buf()];
        deny_read
            .allow_prefixes
            .extend(parent_artifacts.iter().cloned());

        if let Some(session) = app.chat.sessions.read().await.get(&chat).cloned() {
            session
                .lock()
                .await
                .runtime
                .python_runs
                .push(PythonRunSnapshot {
                    tool_call_id: id.clone(),
                    run_id: run.clone(),
                    status: PythonRunLiveStatus::Running,
                    started_at_ms: started,
                    calls: vec![],
                    stdout_tail: String::new(),
                    displays: vec![],
                });
        }
        let _cleanup = RunCleanup {
            app: app.clone(),
            chat: chat.clone(),
            call: id.clone(),
            run: run.clone(),
        };
        let (updates, mut updates_rx) = tokio::sync::mpsc::unbounded_channel();
        let progress = Arc::new(Progress {
            app: app.clone(),
            chat: chat.clone(),
            call: id.clone(),
            run: run.clone(),
            calls: Mutex::new(vec![]),
            updates,
        });
        progress
            .update(PythonRunUpdate::Started {
                background,
            })
            .await;
        let dispatcher = Arc::new(NestedDispatcher::new(NestedDispatchConfig {
            app: app.clone(),
            ccx: ccx.clone(),
            chat_id: chat.clone(),
            outer_tool_call_id: id.clone(),
            run_id: run.clone(),
            catalog: catalog.clone(),
            thread,
            mode_id: mode,
            model_id: Some(model),
            allowed_tools,
            tool_catalog: Some(tool_catalog),
            turn_tool_pool: invocation.and_then(|i| i.pool),
            max_nested_calls: settings.python_max_nested_calls,
            max_parallel_calls: settings.python_max_parallel_calls,
            max_nested_result_bytes: settings.python_max_nested_result_bytes,
            run_dir: run_dir.clone(),
            cancel: cancel.clone(),
            sink: progress.clone(),
            llm: llm_budget.clone(),
            run_privacy: run_privacy.clone(),
            engine: engine_state.clone(),
            project_root: root.clone(),
            workspace_folders,
            deny_read: deny_read.clone(),
            workspace_dir: workspace.dir().to_path_buf(),
        }));
        let mut globals: Vec<GlobalBinding> = catalog
            .functions
            .iter()
            .chain(&catalog.helpers)
            .map(|f| GlobalBinding {
                name: f.py_name.clone(),
                kind: if f.py_name == refact_python_catalog::LLM_HELPER && f.is_async {
                    GlobalKind::AsyncFnPositional(
                        refact_python_catalog::LLM_POSITIONAL
                            .iter()
                            .map(|name| name.to_string())
                            .collect(),
                    )
                } else if f.py_name == refact_python_catalog::GIT_HELPER
                    && f.tool_name == refact_python_catalog::GIT_HELPER
                {
                    GlobalKind::AsyncFnPositional(
                        refact_python_catalog::GIT_POSITIONAL
                            .iter()
                            .map(|name| name.to_string())
                            .collect(),
                    )
                } else if f.py_name == refact_python_catalog::RUN_HELPER
                    && f.tool_name == refact_python_catalog::RUN_HELPER
                {
                    GlobalKind::AsyncFnPositional(
                        refact_python_catalog::RUN_POSITIONAL
                            .iter()
                            .map(|name| name.to_string())
                            .collect(),
                    )
                } else if f.py_name == refact_python_catalog::ENGINE_HELPER
                    && f.tool_name == refact_python_catalog::ENGINE_HELPER
                {
                    GlobalKind::AsyncFnPositional(vec![
                        crate::python_tool::engine_fns::FUNCTION_KEY.to_string(),
                    ])
                } else if f.is_async {
                    GlobalKind::AsyncFn
                } else if f.py_name == f.tool_name
                    && refact_python_catalog::FILE_HELPERS.contains(&f.tool_name.as_str())
                {
                    GlobalKind::HostHelper
                } else {
                    GlobalKind::SyncFn
                },
            })
            .collect();
        for (name, value) in [
            ("PROJECT_ROOT", json!(python_path(&root, native_platform()))),
            ("CHAT_ID", json!(chat)),
            (
                "WORKSPACE_DIR",
                json!(python_path(workspace.dir(), native_platform())),
            ),
            (
                "ARTIFACTS_DIR",
                json!(python_path(&workspace.artifacts_dir(), native_platform())),
            ),
            (
                "RUNS_DIR",
                json!(python_path(&workspace.runs_dir(), native_platform())),
            ),
            ("RUN_ID", json!(run)),
            ("RUN_DIR", json!(python_path(&run_dir, native_platform()))),
            (
                "PARENT_ARTIFACTS_DIR",
                json!(parent_artifacts
                    .as_ref()
                    .map(|path| python_path(path, native_platform()))),
            ),
        ] {
            globals.push(GlobalBinding {
                name: name.into(),
                kind: GlobalKind::Json(value),
            });
        }
        let mut errors = vec![];
        let replacements =
            crate::integrations::setting_up_integrations::get_public_vars_for_replacements(
                app.gcx.clone(),
                &mut errors,
            )
            .await;
        let env = refact_exec::build_child_env(
            &refact_exec::ExecEnvPolicy::Scrubbed {
                passthrough: app.gcx.terminal_security_config.env_passthrough.clone(),
            },
            &replacements,
        )
        .into_iter()
        .collect();
        let mut privacy = ChatMessage::default();
        lease.remember_reset_reason(start.reset_reason.clone());
        lease.remember_restored(start.restored);
        let session_taint: Vec<Value> = lease.taint();
        run_privacy.lock().unwrap().unobserved_output =
            crate::python_tool::llm::has_unobserved_output_mark(&session_taint);
        let session_taint: Vec<Value> = session_taint
            .into_iter()
            .filter(|value| value.get("files").is_some())
            .collect();
        crate::python_tool::run_output::merge_taint(&mut privacy, &session_taint)?;
        run_privacy
            .lock()
            .unwrap()
            .taint
            .extend(session_taint.iter().cloned());
        let configure = lease
            .client()
            .send(KernelCommand::Configure {
                project_root: root.clone(),
                env,
                capture_dir: Some(run_dir.join("captures")),
                deny_write,
                deny_read,
            })
            .await
            .map_err(|e| e.to_string());
        let limits = RunLimits {
            max_memory_bytes: settings.python_max_memory_bytes,
            max_compute_ms: settings.python_max_compute_ms,
            max_arg_bytes: RunLimits::default()
                .max_arg_bytes
                .min(settings.python_max_nested_result_bytes as u64),
            max_result_nodes: refact_python_kernel::convert::result_nodes_for(
                settings.python_max_nested_result_bytes as u64,
            ),
            max_result_bytes: settings.python_max_nested_result_bytes as u64,
            ..Default::default()
        };
        let value_limits = (limits.max_value_nodes, limits.max_value_bytes);
        lease.set_feed_active(true);
        let feed = if configure.is_ok() {
            lease
                .client()
                .send(KernelCommand::Feed {
                    run_id: run.clone(),
                    code,
                    globals,
                    limits,
                })
                .await
                .map_err(|e| e.to_string())
        } else {
            configure
        };
        #[cfg(test)]
        if FAIL_AFTER_FEED.try_with(|fail| *fail).unwrap_or(false) {
            return Err("injected failure after Feed".into());
        }
        let mut pending: HashMap<u32, tokio::task::JoinHandle<crate::python_tool::NestedOutcome>> =
            HashMap::new();
        let mut seq = 0u32;
        let mut stdout = String::new();
        let mut stdout_bytes = 0usize;
        let mut stdout_update = String::new();
        let mut last_stdout = tokio::time::Instant::now();
        let mut result = Value::Null;
        let mut result_text = String::new();
        let mut result_truncated = false;
        let mut error = Value::Null;
        let mut images = std::collections::BTreeMap::new();
        let mut captures = std::collections::BTreeMap::new();
        let mut renames = std::collections::BTreeMap::new();
        let mut reads = std::collections::BTreeSet::new();
        let mut writes = std::collections::BTreeMap::new();
        let mut usable = true;
        let mut suspended: Option<Vec<u32>> = None;
        let mut detached = false;
        let mut limit_reason: Option<String> = None;
        let mut displays = crate::python_tool::display::DisplayLog::default();
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(settings.python_max_run_ms);
        app.python_tool.set_run_deadline(&chat, id, deadline);
        let stop = || {
            cancel.is_cancelled()
                || abort.load(Ordering::Acquire)
                || tokio::time::Instant::now() >= deadline
        };
        let run_outcome: Result<&str, String> = async {
            let mut grep_admission: Option<(u32, Arc<Snapshot>)> = None;
            Ok(loop {
            if let Err(message) = &feed {
                usable = false;
                lease
                    .discard_worker("worker connection lost; session lost")
                    .await;
                error = json!({"type":"WorkerExited","message":message,"traceback":""});
                break "failed";
            }
            while let Ok(update) = updates_rx.try_recv() {
                progress.update(update).await;
            }
            if !detached {
                let job = {
                    app.python_tool
                        .detached
                        .lock()
                        .unwrap()
                        .get(&(chat.clone(), id.clone()))
                        .cloned()
                };
                if let Some(job) = job {
                    lease.mark_background(&job);
                    detached = true;
                    progress.update(PythonRunUpdate::Detached {}).await;
                }
            }
            let stopped = cancel.is_cancelled() || abort.load(Ordering::Acquire);
            let timed_out = tokio::time::Instant::now() >= deadline;
            if stopped || timed_out {
                cancel.cancel();
                app.python_tool.approvals.cancel_run_requests(&chat, &run);
                if suspended.is_some() {
                    let _ = lease
                        .client()
                        .send(KernelCommand::Abort {
                            run_id: run.clone(),
                        })
                        .await;
                    usable = tokio::time::timeout(Duration::from_secs(2), async {
                        loop {
                            match lease.client().next_event().await {
                                ClientEvent::Kernel(KernelEvent::Cancelled {
                                    run_id,
                                }) if run_id == run => break true,
                                ClientEvent::Kernel(event)
                                    if event_run_id(&event).is_some_and(|id| id != run) =>
                                {
                                    continue
                                }
                                _ => break false,
                            }
                        }
                    })
                    .await
                    .unwrap_or(false);
                } else {
                    usable = false;
                }
                if !usable {
                    lease
                        .discard_worker(if timed_out {
                            "run timed out; session lost"
                        } else {
                            "run cancelled; session lost"
                        })
                        .await;
                }
                error = json!({"type":if timed_out {"TimeoutError"}else{"CancelledError"},"message":if timed_out {"Python run timed out"}else{"Python run cancelled"},"traceback":""});
                break if timed_out { "timed_out" } else { "cancelled" };
            }
            if let Some(ids) = suspended.as_ref() {
                if ids
                    .iter()
                    .all(|id| pending.get(id).is_none_or(|h| h.is_finished()))
                {
                    let mut answers = vec![];
                    let mut stop = false;
                    for id in suspended.take().unwrap() {
                        let Some(handle) = pending.remove(&id) else {
                            answers.push((id, unknown_call_result(id)));
                            continue;
                        };
                        match handle.await {
                            Ok(o) => {
                                stop |= o.stop_run;
                                crate::python_tool::run_output::merge_taint(
                                    &mut privacy,
                                    &o.privacy,
                                )?;
                                if let HostResult::Ok(envelope) = &o.result {
                                    if let Some(refs) = envelope["images"].as_array() {
                                        for (reference, image) in refs.iter().zip(o.images) {
                                            if let Some(index) = reference["index"].as_u64() {
                                                images.insert(index, image);
                                            }
                                        }
                                    }
                                }
                                answers.push((id, o.result));
                            }
                            Err(e) => answers.push((
                                id,
                                HostResult::Err {
                                    exc_type: "RuntimeError".into(),
                                    message: e.to_string(),
                                },
                            )),
                        }
                    }
                    if stop {
                        cancel.cancel();
                        suspended = Some(vec![]);
                        continue;
                    }
                    lease
                        .client()
                        .send(KernelCommand::Results {
                            run_id: run.clone(),
                            results: answers,
                        })
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
            let event = tokio::select! {e=lease.client().next_event()=>e,_=tokio::time::sleep(Duration::from_millis(25))=>continue};
            if let ClientEvent::Kernel(ref event) = event {
                if event_run_id(event).is_some_and(|id| id != run) {
                    continue;
                }
            }
            match event {
                ClientEvent::Kernel(KernelEvent::CallStarted {
                    call_id,
                    function,
                    kwargs,
                    truncated,
                    line,
                    col,
                    exceeded,
                    ..
                }) => {
                    let too_large = too_large(truncated, exceeded);
                    seq += 1;
                    let d = dispatcher.clone();
                    let n = seq;
                    let llm = function == refact_python_catalog::LLM_HELPER
                        && catalog.resolve(&function).is_some_and(|f| f.tool_name == refact_python_catalog::LLM_HELPER);
                    let engine = function == refact_python_catalog::ENGINE_HELPER
                        && catalog.resolve(&function).is_some_and(|f| f.tool_name == refact_python_catalog::ENGINE_HELPER);
                    let git = function == refact_python_catalog::GIT_HELPER
                        && catalog.resolve(&function).is_some_and(|f| f.tool_name == refact_python_catalog::GIT_HELPER);
                    let run_helper = function == refact_python_catalog::RUN_HELPER
                        && catalog.resolve(&function).is_some_and(|f| f.tool_name == refact_python_catalog::RUN_HELPER);
                    pending.insert(
                        call_id,
                        tokio::spawn(async move {
                            if engine {
                                return d.dispatch_engine(n, kwargs, too_large, (line, col)).await;
                            }
                            let _call = d.enter_call().await;
                            if llm {
                                d.dispatch_llm(n, kwargs, too_large, (line, col)).await
                            } else if git {
                                d.dispatch_git(n, kwargs, too_large, (line, col)).await
                            } else if run_helper {
                                d.dispatch_run(n, kwargs, too_large, (line, col)).await
                            } else {
                                d.dispatch_at(n, &function, kwargs, too_large, (line, col)).await
                            }
                        }),
                    );
                }
                ClientEvent::Kernel(KernelEvent::NeedResults {
                    pending, ..
                }) => suspended = Some(pending),
                ClientEvent::Kernel(KernelEvent::SyncCall {
                    call_id,
                    function,
                    args,
                    kwargs,
                    truncated,
                    exceeded,
                    ..
                }) => {
                    let too_large = too_large(truncated, exceeded);
                    let answer = if function == GREP_FILES {
                        grep_admission = None;
                        let stopped = async {
                            while !stop() {
                                tokio::time::sleep(Duration::from_millis(25)).await;
                            }
                        };
                        let listing = tokio::task::spawn_blocking({
                            let (gcx, args, abort) = (app.gcx.clone(), args.clone(), abort.clone());
                            move || {
                                tokio::runtime::Handle::current()
                                    .block_on(grep_files(gcx, &args, &abort))
                            }
                        });
                        let answer = tokio::select! {
                            answer = listing => Some(answer),
                            _ = stopped => None,
                        };
                        match answer {
                            Some(answer) if !stop() => match answer {
                                Ok(Ok((list, snapshot))) => {
                                    grep_admission = Some((call_id, snapshot));
                                    HostResult::Ok(list)
                                }
                                Ok(Err(message)) => grep_listing_error(message),
                                Err(e) => grep_listing_error(e.to_string()),
                            },
                            _ => {
                                // The kernel waits for this answer: abort it at the top of the loop and keep the session.
                                suspended = Some(vec![]);
                                continue;
                            }
                        }
                    } else if function == refact_python_catalog::SHOW_HELPER {
                        if let Some(size) = too_large {
                            HostResult::Err {
                                exc_type: "TypeError".into(),
                                message: format!("show(): the value is too large to display ({size}); aggregate it first or write it to a file"),
                            }
                        } else {
                            match displays
                                .show(&run_dir, &args, &kwargs, settings.python_max_nested_result_bytes)
                                .await
                            {
                                Ok(inline) => {
                                    if let Some(display) = inline {
                                        progress.update(PythonRunUpdate::Display { display }).await;
                                    }
                                    HostResult::Ok(Value::Null)
                                }
                                Err(crate::python_tool::display::ShowError::Type(message)) => HostResult::Err {
                                    exc_type: "TypeError".into(),
                                    message,
                                },
                                Err(crate::python_tool::display::ShowError::Limit(message)) => HostResult::Err {
                                    exc_type: "RuntimeError".into(),
                                    message,
                                },
                            }
                        }
                    } else {
                        sync_helper(&catalog, &function, &args, too_large.as_deref(), settings.python_max_result_chars)
                    };
                    lease
                        .client()
                        .send(KernelCommand::SyncResult {
                            call_id,
                            result: answer,
                        })
                        .await
                        .map_err(|e| e.to_string())?;
                }
                ClientEvent::Kernel(KernelEvent::GrepRevalidate {
                    run_id,
                    call_id,
                }) => {
                    #[cfg(test)]
                    crate::python_tool::workspace::pause_point(&root, "grep_scanned").await;
                    let allowed = grep_admission.take().is_some_and(|(admitted_id, snapshot)| {
                        run_id == run && call_id == admitted_id && snapshot.finish(&app.gcx).is_ok()
                    });
                    if stop() {
                        suspended = Some(vec![]);
                        continue;
                    }
                    lease
                        .client()
                        .send(KernelCommand::GrepRevalidated { run_id, call_id, allowed })
                        .await
                        .map_err(|e| e.to_string())?;
                }
                ClientEvent::Kernel(KernelEvent::Stdout {
                    text, ..
                }) => {
                    if stdout.len() < MAX_STDOUT_BYTES {
                        stdout.push_str(&crate::python_tool::render::cut(
                            &text,
                            MAX_STDOUT_BYTES - stdout.len(),
                        ));
                    }
                    stdout_bytes = stdout.len();
                    stdout_update.push_str(&text);
                    if last_stdout.elapsed() >= Duration::from_millis(250) {
                        progress
                            .update(PythonRunUpdate::Stdout {
                                text: std::mem::take(&mut stdout_update),
                            })
                            .await;
                        last_stdout = tokio::time::Instant::now();
                    }
                }
                ClientEvent::Kernel(KernelEvent::Renamed {
                    src,
                    dst,
                    ..
                }) => {
                    renames.insert(src, dst);
                }
                ClientEvent::Kernel(KernelEvent::WriteCapture {
                    path,
                    original,
                    ..
                }) => {
                    captures.entry(path).or_insert(original);
                }
                ClientEvent::Kernel(KernelEvent::Fs {
                    op,
                    path,
                    bytes,
                    ..
                }) => {
                    if is_write(&op) {
                        writes.insert(path, bytes);
                    } else {
                        run_privacy.lock().unwrap().reads.insert(path.clone());
                        reads.insert(path);
                    }
                }
                ClientEvent::Kernel(KernelEvent::Completed {
                    value,
                    repr,
                    truncated,
                    ..
                }) => {
                    result_text = if value.get("__repr__").is_some() {
                        repr
                    } else {
                        serde_json::to_string_pretty(&value).unwrap_or(repr)
                    };
                    if truncated {
                        result_truncated = true;
                        result_text = format!(
                            "{result_text}\n[the value is over the result limit ({} values, {} bytes); parts of it are shown as their repr. Return a smaller summary or write the data to a file]",
                            value_limits.0, value_limits.1
                        );
                    }
                    result = value;
                    break "completed";
                }
                ClientEvent::Kernel(KernelEvent::Raised {
                    exc_type,
                    message,
                    traceback,
                    line,
                    ..
                }) => {
                    let hint = crate::python_tool::hints::hint(&exc_type, &message);
                    error = json!({"type":exc_type,"message":message,"traceback":traceback});
                    if let Some(line) = line {
                        error["line"] = json!(line);
                    }
                    if let Some(hint) = hint {
                        error["hint"] = json!(hint);
                    }
                    break "failed";
                }
                ClientEvent::Kernel(KernelEvent::LimitExceeded {
                    kind,
                    message,
                    ..
                }) => {
                    error =
                        json!({"type":format!("{kind:?}Limit"),"message":message,"traceback":""});
                    usable = false;
                    limit_reason = Some(limit_rollback_reason(&workspace).await);
                    lease.discard_worker_for_rollback(limit_reason.as_deref().unwrap()).await;
                    break "limit_exceeded";
                }
                ClientEvent::Kernel(KernelEvent::Cancelled {
                    ..
                }) => break "cancelled",
                ClientEvent::MemoryExceeded {
                    rss_bytes,
                } => {
                    usable = false;
                    limit_reason = Some(limit_rollback_reason(&workspace).await);
                    lease.discard_worker_for_rollback(limit_reason.as_deref().unwrap()).await;
                    error = json!({"type":"MemoryError","message":format!("Worker memory limit exceeded ({rss_bytes} bytes)"),"traceback":""});
                    break "limit_exceeded";
                }
                ClientEvent::ProtocolError {
                    message,
                }
                | ClientEvent::Kernel(KernelEvent::ProtocolError {
                    message,
                }) => {
                    usable = false;
                    lease.discard_worker(&message).await;
                    error = json!({"type":"ProtocolError","message":message,"traceback":""});
                    break "failed";
                }
                ClientEvent::Exited {
                    code,
                    stderr_tail,
                } => {
                    usable = false;
                    lease.discard_worker("worker exited; session lost").await;
                    error = json!({"type":"WorkerExited","message":format!("Worker exited {code:?}: {stderr_tail}"),"traceback":""});
                    break "failed";
                }
                _ => {}
            }
            })
        }.await;
        cancel.cancel();
        app.python_tool.approvals.cancel_run_requests(&chat, &run);
        let drain_started = tokio::time::Instant::now();
        let mut warned = false;
        for (_, mut handle) in pending {
            loop {
                tokio::select! {
                    _ = &mut handle => break,
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {
                        if engine_shutting_down(&app) {
                            break;
                        }
                        if !warned && drain_started.elapsed() >= Duration::from_secs(5) {
                            warned = true;
                            tracing::warn!(run_id = %run, "Waiting for cancelled Python nested tasks to finish");
                            announce_stopping(&progress).await;
                        }
                    }
                }
            }
        }
        {
            let mut calls = progress.calls.lock().unwrap();
            for call in calls.iter_mut().filter(|call| call.duration_ms.is_none()) {
                call.status = refact_chat_api::PythonCallStatus::Cancelled;
                call.summary = "Python run ended before the nested call completed".into();
                call.duration_ms =
                    Some(crate::agents::approval::now_ms().saturating_sub(call.started_at_ms));
            }
        }
        let status = run_outcome?;
        if let Some(reason) = limit_reason {
            start.reset_reason = Some(reason);
        }
        let direct = crate::privacy::records::observed_file_records_with_derived(
            &app.gcx,
            reads.iter().map(std::path::PathBuf::from),
            &derived,
        )?;
        crate::privacy::records::merge_records(&mut privacy, direct);
        let carried: refact_privacy::PrivacyRecord = privacy
            .extra
            .get("privacy")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        for (name, data) in [
            ("stdout.txt", stdout.as_bytes().to_vec()),
            ("result.json", serde_json::to_vec_pretty(&result).unwrap()),
        ] {
            tokio::fs::write(run_dir.join(name), data)
                .await
                .map_err(|e| e.to_string())?;
        }
        // Observation markers belong to this run's message only, never to the session.
        let taint: Vec<Value> = privacy
            .extra
            .get("privacy")
            .cloned()
            .into_iter()
            .chain(
                run_privacy
                    .lock()
                    .unwrap()
                    .unobserved_output
                    .then(crate::python_tool::llm::unobserved_output_mark),
            )
            .collect();
        let mut not_saved = None;
        let persisted = if usable {
            match lease.persist(&app, &run, taint).await {
                Ok(outcome) => {
                    not_saved = outcome.not_saved.clone();
                    outcome
                }
                Err(reason) => {
                    let fallback = if matches!(workspace.pin_state(false).await, Ok(Some(_))) {
                        "variables come from the last successful save"
                    } else {
                        "the session was reset and has no variables"
                    };
                    not_saved = Some(format!("not saved ({reason}); {fallback}"));
                    lease.discard_worker_for_rollback(&reason).await;
                    Default::default()
                }
            }
        } else {
            Default::default()
        };
        while let Ok(update) = updates_rx.try_recv() {
            progress.update(update).await;
        }
        let mut calls = progress.calls.lock().unwrap().clone();
        calls.sort_by_key(|call| call.seq);
        for path in captures.keys() {
            writes.entry(path.clone()).or_insert(0);
        }
        let files = crate::python_tool::run_output::written_files_budget(
            &writes,
            &captures,
            &renames,
            &run_dir,
            (256usize * 1024).saturating_sub(
                calls
                    .iter()
                    .map(|call| serde_json::to_vec(&call.diffs).map_or(0, |bytes| bytes.len()))
                    .sum(),
            ),
        )
        .await?;
        let mut artifacts = vec![];
        for (path, _) in &writes {
            if std::path::Path::new(path).starts_with(workspace.artifacts_dir()) {
                if let Ok(m) = tokio::fs::metadata(path).await {
                    if m.is_file() {
                        artifacts.push(json!({"path":path,"bytes":m.len()}));
                    }
                }
            }
        }
        let extra = json!({"schema":1,"run_id":run,"status":status,"background":background||detached,"started_at_ms":started,"finished_at_ms":crate::agents::approval::now_ms(),"workspace_dir":workspace.dir(),"run_dir":run_dir,"calls":calls,"calls_total":seq,"calls_truncated":seq>200,"stdout_preview":crate::python_tool::render::cut(&stdout,4096),"stdout_bytes":stdout_bytes,"result_preview":crate::python_tool::render::cut(&result_text,4096),"result_bytes":result_text.len(),"error":error,"files_read":reads,"files_written":files,"artifacts":artifacts,"state":{"restored":start.restored,"reset_reason":start.reset_reason,"persisted":persisted.persisted,"bytes":persisted.bytes}});
        let mut extra = extra;
        if result_truncated {
            extra["result_truncated"] = json!(true);
        }
        if let Some(not_saved) = not_saved {
            extra["state"]["not_saved"] = json!(not_saved);
        }
        if let Some(note) = &persisted.note {
            extra["state"]["note"] = json!(note);
        }
        if !start.notes.is_empty() {
            extra["state"]["notes"] = json!(start.notes);
        }
        if !writable {
            extra["read_only"] = json!(true);
        }
        if let (Some(extra), Some(labels)) = (extra.as_object_mut(), labels.as_object()) {
            extra.extend(labels.clone());
        }
        if let Some(totals) = llm_budget.as_ref().and_then(|budget| budget.totals()) {
            extra["llm"] = totals;
        }
        let shutdown_dropped = status != "completed"
            && engine_state
                .as_ref()
                .is_some_and(|engine| engine.drop_requested_shutdown());
        if shutdown_dropped {
            for call in extra["calls"].as_array_mut().into_iter().flatten() {
                if call["tool"] == "engine.system.shutdown" && call["status"] == "ok" {
                    call["status"] = json!("cancelled");
                    call["summary"] = json!(format!("shutdown dropped: the run ended {status}"));
                    let file = run_dir
                        .join("calls")
                        .join(format!("{}_engine.json", call["seq"]));
                    let rewritten = async {
                        let mut log: Value = serde_json::from_slice(
                            &tokio::fs::read(&file).await.map_err(|e| e.to_string())?,
                        )
                        .map_err(|e| e.to_string())?;
                        log["record"] = call.clone();
                        tokio::fs::write(
                            &file,
                            serde_json::to_vec(&log).map_err(|e| e.to_string())?,
                        )
                        .await
                        .map_err(|e| e.to_string())
                    }
                    .await;
                    if let Err(error) = rewritten {
                        tracing::warn!(%error, run_id = %run, "cannot mark the dropped shutdown in its call file");
                    }
                    match serde_json::from_value::<PythonCallRecord>(call.clone()) {
                        Ok(record) => {
                            progress
                                .update(PythonRunUpdate::CallFinished {
                                    record,
                                })
                                .await
                        }
                        Err(error) => {
                            tracing::warn!(%error, run_id = %run, "cannot send the dropped shutdown's call record")
                        }
                    }
                }
            }
        }
        if let Some(engine) = &engine_state {
            engine.extend_extra(&mut extra);
        }
        displays.extend_extra(&mut extra);
        let mut run_meta = extra.clone();
        if let Some(meta) = run_meta.as_object_mut() {
            meta.remove("displays");
            meta.remove("displays_truncated");
            meta.remove("engine_changes");
        }
        crate::python_tool::run_output::persist_run_privacy(
            &app.gcx,
            workspace.dir(),
            &run_dir,
            &carried.files,
            writes.keys().map(std::path::PathBuf::from).collect(),
            &derived,
        )
        .await?;
        crate::python_tool::workspace::atomic_write(
            &run_dir.join("run.json"),
            &serde_json::to_vec_pretty(&run_meta).unwrap(),
        )
        .await?;
        failed_run.disarm();
        if let Err(error) = crate::python_tool::workspace::atomic_write(
            &run_dir.join(crate::python_tool::workspace::RUN_SUMMARY_FILE),
            &serde_json::to_vec(&crate::python_tool::workspace::run_summary(&run_meta)).unwrap(),
        )
        .await
        {
            tracing::warn!(%error, run_id = %run, "cannot write Python run summary cache");
        }
        if !stdout_update.is_empty() {
            progress
                .update(PythonRunUpdate::Stdout {
                    text: stdout_update,
                })
                .await;
        }
        progress
            .update(PythonRunUpdate::Finished {
                status: status.into(),
            })
            .await;
        if let Some(session) = app.chat.sessions.read().await.get(&chat).cloned() {
            session
                .lock()
                .await
                .runtime
                .python_runs
                .retain(|r| r.run_id != run);
        }
        app.python_tool
            .detached
            .lock()
            .unwrap()
            .remove(&(chat.clone(), id.clone()));
        let _ = workspace
            .enforce_quota(settings.python_disk_quota_bytes)
            .await;
        let mut notices = String::new();
        if shutdown_dropped {
            notices.push_str(&format!(
                "The approved engine shutdown was not performed because the run ended {status}.\n"
            ));
        }
        if requested_background && foreground_only {
            notices.push_str("Background runs are not available in this chat.\n");
        }
        let text = crate::python_tool::render::render(
            &result_text,
            &stdout,
            &extra,
            &notices,
            &displays.model_text(),
            settings.python_max_result_chars,
        );
        let mut message = ChatMessage {
            role: "tool".into(),
            tool_call_id: id.clone(),
            content: ChatContent::SimpleText(text),
            tool_failed: Some(status != "completed"),
            ..Default::default()
        };
        message.extra.extend(privacy.extra);
        if !images.is_empty() {
            let mut parts = vec![crate::call_validation::MultimodalElement {
                m_type: "text".into(),
                m_content: message.content.content_text_only(),
            }];
            parts.extend(images.into_values());
            message.content = ChatContent::Multimodal(parts);
        }
        message.extra.insert("python".into(), extra);
        lease.set_feed_active(false);
        if let Some(engine) = &engine_state {
            engine.send_requested_shutdown_when_stored(&app, &chat, &run);
        }
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "python".into(),
            display_name: "Python".into(),
            description: if self.description.is_empty() {
                crate::python_tool::catalog_binding::INTRO.into()
            } else {
                self.description.clone()
            },
            allow_parallel: true,
            experimental: false,
            source: refact_tool_api::ToolSource {
                source_type: refact_tool_api::ToolSourceType::Builtin,
                config_path: String::new(),
            },
            input_schema: json!({"type":"object","additionalProperties":false,"required":["code"],"properties":{"code":{"type":"string","minLength":1,"description":"Python 3 (Monty subset). Top-level await allowed. The last expression's value is returned."},"description":{"type":"string","description":"What this script does, in one short sentence. The user sees it as the title of the run."},"restart":{"type":"boolean","default":false,"description":"Discard this chat's Python variables first. Files are kept."},"run_in_background":{"type":"boolean","default":false,"description":"Return a job receipt now; the result arrives later in this chat."}}}),
            output_schema: None,
            annotations: None,
        }
    }
}
pub(crate) fn engine_shutting_down(app: &AppState) -> bool {
    app.gcx.shutdown_flag.load(Ordering::Acquire) || app.runtime.tool_jobs.scopes.is_shutdown()
}

async fn announce_stopping(progress: &Progress) {
    let running: Vec<PythonCallRecord> = {
        let mut calls = progress.calls.lock().unwrap();
        calls
            .iter_mut()
            .filter(|call| call.duration_ms.is_none())
            .map(|call| {
                call.summary = "stopping…".into();
                call.clone()
            })
            .collect()
    };
    let Some(first) = running.first() else {
        return;
    };
    let tool = first.tool.clone();
    for record in running {
        progress
            .update(PythonRunUpdate::CallStarted {
                record,
            })
            .await;
    }
    progress
        .update(PythonRunUpdate::Stdout {
            text: format!("Waiting for {tool} to stop\n"),
        })
        .await;
}

async fn limit_rollback_reason(workspace: &PythonWorkspace) -> String {
    if workspace.verify_state().await.is_ok() {
        "run limit exceeded: the session was restored to the state saved before this run; variables set in this run are lost".into()
    } else {
        "run limit exceeded: the session was reset and has no variables".into()
    }
}

fn event_run_id(event: &KernelEvent) -> Option<&str> {
    match event {
        KernelEvent::Completed {
            run_id, ..
        }
        | KernelEvent::Raised {
            run_id, ..
        }
        | KernelEvent::Cancelled {
            run_id,
        }
        | KernelEvent::LimitExceeded {
            run_id, ..
        }
        | KernelEvent::NeedResults {
            run_id, ..
        }
        | KernelEvent::CallStarted {
            run_id, ..
        }
        | KernelEvent::SyncCall {
            run_id, ..
        }
        | KernelEvent::Stdout {
            run_id, ..
        }
        | KernelEvent::Fs {
            run_id, ..
        }
        | KernelEvent::Renamed {
            run_id, ..
        }
        | KernelEvent::WriteCapture {
            run_id, ..
        } => Some(run_id),
        _ => None,
    }
}
fn unknown_call_result(id: u32) -> HostResult {
    HostResult::Err {
        exc_type: "RuntimeError".into(),
        message: format!("Unknown host call id {id}"),
    }
}
#[cfg(test)]
tokio::task_local! { static FAIL_AFTER_FEED: bool; }
fn is_write(op: &str) -> bool {
    let op = op.to_ascii_lowercase();
    [
        "write", "append", "mkdir", "unlink", "rmdir", "rename", "create",
    ]
    .iter()
    .any(|s| op.contains(s))
}
/// The measured size of arguments a limit cut, for the refusal text.
fn too_large(
    truncated: bool,
    exceeded: Option<refact_python_kernel::protocol::Exceeded>,
) -> Option<String> {
    truncated.then(|| exceeded.map_or_else(|| "over the size limit".into(), |e| e.describe()))
}
fn sync_helper(
    catalog: &refact_python_catalog::PythonCatalog,
    function: &str,
    args: &Value,
    too_large: Option<&str>,
    max_result_chars: usize,
) -> HostResult {
    if let Some(size) = too_large {
        return HostResult::Err {
            exc_type: "TypeError".into(),
            message: format!(
                "{function}() arguments are too large to send: {size}; pass a file path or a smaller value"
            ),
        };
    }
    let a = args.as_array();
    let arg = a.and_then(|a| a.first()).and_then(Value::as_str);
    let value = match function {
        "tools" if a.is_some_and(|a| a.is_empty()) => Some(json!(catalog
            .functions
            .iter()
            .chain(&catalog.helpers)
            .map(|f| f.py_name.clone())
            .collect::<Vec<_>>())),
        "help" if a.is_some_and(|a| a.is_empty() || (a.len() == 1 && a[0].is_null())) => {
            Some(Value::String(catalog.help_index(max_result_chars / 2)))
        }
        "help" => arg
            .and_then(|name| catalog.help_text(name))
            .map(Value::String),
        "host_path" => arg.map(|p| {
            json!(refact_python_kernel::hostfs::host_path(
                p,
                if cfg!(windows) {
                    refact_python_kernel::hostfs::Platform::Windows
                } else {
                    refact_python_kernel::hostfs::Platform::Unix
                }
            ))
        }),
        "virtual_path" => arg.map(|p| {
            json!(refact_python_kernel::hostfs::virtual_path(
                std::path::Path::new(p),
                if cfg!(windows) {
                    refact_python_kernel::hostfs::Platform::Windows
                } else {
                    refact_python_kernel::hostfs::Platform::Unix
                }
            ))
        }),
        _ => None,
    };
    if value.is_none() && catalog.resolve(function).is_none() {
        return HostResult::Err {
            exc_type: "NameError".into(),
            message: format!("name '{function}' is not defined"),
        };
    }
    value
        .map(HostResult::Ok)
        .unwrap_or_else(|| HostResult::Err {
            exc_type: "TypeError".into(),
            message: format!("Invalid arguments or unknown function: {function}"),
        })
}
/// Answers `grep.files`: the files under a directory with `search_pattern`'s ignore rules, or a
/// named file, without the files the privacy policy blocks for model context. Only the requested
/// directory is walked, and the call's glob is applied before the privacy partition, so the
/// counts cover only files the search would read; names that are not UTF-8 are left out.
async fn grep_files(
    gcx: crate::global_context::SharedGlobalContext,
    args: &Value,
    abort: &std::sync::atomic::AtomicBool,
) -> Result<(Value, Arc<Snapshot>), String> {
    let request: FileListRequest = match args
        .get(0)
        .cloned()
        .map(serde_json::from_value::<FileListRequest>)
    {
        Some(Ok(request)) => request,
        _ => return Err("malformed file list request".into()),
    };
    let glob = request
        .glob
        .as_deref()
        .map(glob::Pattern::new)
        .transpose()
        .map_err(|e| format!("invalid glob: {e}"))?;
    let nested = request
        .glob
        .as_deref()
        .is_some_and(|glob| glob.contains('/'));
    let aborted = || format!("the listing of '{}' was aborted", request.path.display());
    crate::python_tool::workspace::blocking_pause_point(&request.path, "grep_listing");
    let (candidates, truncated) = if request.path.is_dir() {
        let indexing =
            crate::files_blocklist::reload_indexing_everywhere_if_needed(gcx.clone()).await;
        match crate::files_in_workspace::ls_dir_files_limited(
            &indexing,
            &request.path,
            request.max_entries.min(GREP_MAX_FILES) as usize,
            Some(abort),
        ) {
            Ok(listing) if listing.aborted => return Err(aborted()),
            Ok(listing) => (listing.files, listing.truncated),
            Err(message) => return Err(message),
        }
    } else {
        (vec![request.path.clone()], false)
    };
    let (named, unnamed): (Vec<_>, Vec<_>) = candidates
        .into_iter()
        .filter(|path| {
            path.starts_with(&request.path)
                && glob.as_ref().is_none_or(|pattern| {
                    refact_python_kernel::hostfs::glob_matches(pattern, nested, &request.path, path)
                })
        })
        .partition(|path| path.to_str().is_some());
    let read_context = crate::files_in_workspace::prepare_file_read_context(gcx.clone()).await;
    let snapshot = crate::privacy::classification_cache::snapshot(&gcx)?;
    let mut files = Vec::new();
    let mut blocked = 0;
    for path in named {
        if abort.load(Ordering::Acquire) {
            return Err(aborted());
        }
        if crate::files_in_workspace::check_file_privacy_for_read_preparation_with_context(
            &read_context,
            &path,
        )
        .is_err()
        {
            blocked += 1;
            continue;
        }
        let zone = snapshot.classify_for_read_admission(&gcx, &path);
        if zone.name == "blocked" || zone.send_to.is_empty() {
            blocked += 1;
        } else {
            files.push(path);
        }
    }
    snapshot.finish(&gcx)?;
    let list = serde_json::to_value(FileList {
        files,
        blocked,
        truncated,
        unlisted: unnamed.len(),
    })
    .map_err(|e| e.to_string())?;
    Ok((list, snapshot))
}
fn grep_listing_error(message: String) -> HostResult {
    HostResult::Err {
        exc_type: "RuntimeError".into(),
        message: format!("grep(): {message}"),
    }
}
/// The most files one `grep()` listing returns, whatever the request asks for.
const GREP_MAX_FILES: u64 = 20_000;
#[cfg(test)]
#[path = "tool_python_run_tests.rs"]
mod run_tests;
#[cfg(test)]
#[path = "tool_python_tests.rs"]
pub(crate) mod tests;

async fn protected_paths(
    app: &AppState,
    root: &std::path::Path,
    workspace: &PythonWorkspace,
) -> (PathDenyRules, PathDenyRules) {
    let gcx = &app.gcx;
    let ext = crate::ext::config_dirs::get_ext_dirs(app.clone()).await;
    let mut paths = crate::tools::file_edit::auxiliary::protected_config_locations(gcx, root);
    let mut roots = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    roots.push(root.to_path_buf());
    for root in &roots {
        paths.push(root.join(".refact/hooks.yaml"));
    }
    let mut deny_write = PathDenyRules {
        prefixes: ext
            .global_dirs
            .into_iter()
            .chain(ext.installed_dirs)
            .chain(ext.project_dirs)
            .chain([
                root.join(".claude"),
                root.join(".refact"),
                workspace.dir().to_owned(),
            ])
            .collect(),
        allow_prefixes: vec![workspace.artifacts_dir()],
        files: vec![],
        ..Default::default()
    };
    for path in paths {
        if path.file_name().is_some_and(|n| n == "integrations.d") {
            deny_write.prefixes.push(path);
        } else {
            deny_write.files.push(path);
        }
    }
    let mut secrets = vec![
        gcx.config_dir.join("python_state.key"),
        gcx.config_dir.join("secrets.yaml"),
    ];
    if let Some(home) = home::home_dir() {
        secrets.push(home.join(".claude/.credentials.json"));
    }
    let mut read_allow = vec![workspace.dir().to_path_buf()];
    if let Some(parent) =
        crate::python_tool::workspace::parent_artifacts_dir(app, &workspace.chat_id()).await
    {
        read_allow.push(parent);
    }
    let mut read_prefixes = vec![gcx.config_dir.clone(), gcx.cache_dir.join("bug-reports")];
    read_prefixes.extend(
        deny_write
            .prefixes
            .iter()
            .filter(|p| p.file_name().is_some_and(|n| n == "integrations.d"))
            .cloned(),
    );
    protect_custom_configuration(
        &mut deny_write,
        &mut secrets,
        &gcx.cmdline.secrets_yaml,
        &gcx.cmdline.integrations_yaml,
    );
    (
        deny_write,
        PathDenyRules {
            prefixes: read_prefixes,
            allow_prefixes: read_allow,
            files: secrets,
            ..Default::default()
        },
    )
}

/// Scripts may write outside ARTIFACTS_DIR when a built-in editing tool, `rm`, `shell` or
/// `process_start` passes the authorization nested calls use and no tool-wide deny rule of the
/// mode removes it. MCP and integration tools never count.
async fn script_writes(
    app: &AppState,
    ccx: &Arc<AMutex<AtCommandsContext>>,
    tool_catalog: &refact_runtime_api::ToolCatalogSnapshot,
    allowed_tools: &[String],
    mode: &str,
    model: &str,
) -> bool {
    let mut authorized = std::collections::HashSet::new();
    for tool in tool_catalog.index.tools.iter().filter(|tool| {
        matches!(
            tool.source.source_type,
            crate::tools::tools_description::ToolSourceType::Builtin
        ) && crate::chat::tools::grants_script_writes(&tool.name)
    }) {
        if crate::python_tool::dispatcher::authorize_tool(
            ccx,
            Some(tool_catalog),
            allowed_tools,
            &tool.name,
        )
        .await
        .is_ok()
        {
            authorized.insert(tool.name.clone());
        }
    }
    !code_mode_prompt(app, &authorized, mode, Some(model))
        .await
        .1
}

pub(crate) async fn mode_confirm_rules(
    app: &AppState,
    mode: &str,
    model: Option<&str>,
) -> Vec<crate::yaml_configs::customization_types::ToolConfirmRule> {
    use crate::yaml_configs::customization_registry::{get_mode_config, map_legacy_mode_to_id};
    get_mode_config(app.gcx.clone(), map_legacy_mode_to_id(mode), model)
        .await
        .map(|config| config.tool_confirm.rules)
        .unwrap_or_default()
}

/// What Code Mode offers a chat whose runs may call the tools `names`, after the tool-wide deny
/// rules of its mode: the tools its prompt section describes (`run()` only with `shell`) and
/// whether its scripts may write only under ARTIFACTS_DIR. Runs and prompts both decide here.
pub(crate) async fn code_mode_prompt(
    app: &AppState,
    names: &std::collections::HashSet<String>,
    mode: &str,
    model: Option<&str>,
) -> (std::collections::HashSet<String>, bool) {
    let rules = mode_confirm_rules(app, mode, model).await;
    let names: std::collections::HashSet<String> = names
        .iter()
        .filter(|name| {
            crate::yaml_configs::customization_registry::match_tool_confirm_action(&rules, name)
                .as_deref()
                != Some("deny")
        })
        .cloned()
        .collect();
    let read_only = !names
        .iter()
        .any(|name| crate::chat::tools::grants_script_writes(name));
    (names, read_only)
}

fn read_only_reason(artifacts: &std::path::Path) -> String {
    format!(
        "this chat is read-only: scripts may write files only under ARTIFACTS_DIR ({}) because the chat has no file-editing, mv, rm, shell or process_start tool; tools called from the script follow their own rules",
        python_path(artifacts, native_platform())
    )
}

fn native_platform() -> refact_python_kernel::hostfs::Platform {
    if cfg!(windows) {
        refact_python_kernel::hostfs::Platform::Windows
    } else {
        refact_python_kernel::hostfs::Platform::Unix
    }
}
fn python_path(path: &std::path::Path, platform: refact_python_kernel::hostfs::Platform) -> String {
    refact_python_kernel::hostfs::virtual_path(path, platform)
}

fn protect_custom_configuration(
    write: &mut PathDenyRules,
    read: &mut Vec<std::path::PathBuf>,
    secrets: &str,
    integrations: &str,
) {
    for name in [secrets, integrations]
        .into_iter()
        .filter(|name| !name.is_empty())
    {
        let path = crate::files_correction::canonical_path(name);
        write.files.push(path.clone());
        read.push(path);
    }
}
