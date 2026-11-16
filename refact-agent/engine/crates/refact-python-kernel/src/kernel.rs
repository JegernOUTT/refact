use crate::{convert::*, protocol::*};
use monty::{Dump, MontyRepl, ReplProgress, Session, SessionRef};
use monty_types::{
    CompileOptions, ExcType, ExtFunctionResult, MontyException, MontyObject, NameLookupResult,
    NamedValues, OsFunctionCall, PrintWriter, ResourceLimits, ResourceTracker,
};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io, time::Duration};
pub trait KernelIo {
    fn emit(&mut self, event: KernelEvent) -> io::Result<()>;
    fn recv(&mut self) -> io::Result<KernelCommand>;
}
pub trait OsBackend: Send {
    fn configure(
        &mut self,
        _project_root: std::path::PathBuf,
        _env: std::collections::BTreeMap<String, String>,
        _capture_dir: Option<std::path::PathBuf>,
        _deny_write: PathDenyRules,
        _deny_read: PathDenyRules,
    ) {
    }
    fn cwd(&self) -> Option<String> {
        None
    }
    fn handle_result(&mut self, call: OsFunctionCall) -> ExtFunctionResult {
        match self.handle(call) {
            Ok(v) => v.into(),
            Err((kind, message)) => exception(&kind, message).into(),
        }
    }
    fn handle(&mut self, call: OsFunctionCall) -> Result<MontyObject, (String, String)>;
    fn take_events(&mut self) -> Vec<KernelEvent> {
        vec![]
    }
}
#[derive(Default)]
pub struct FakeOs {
    pub files: std::collections::HashMap<String, String>,
    events: Vec<KernelEvent>,
}
impl OsBackend for FakeOs {
    fn handle(&mut self, call: OsFunctionCall) -> Result<MontyObject, (String, String)> {
        match call {
            OsFunctionCall::ReadText(p) => {
                let path = p.into_string();
                let text = self
                    .files
                    .get(&path)
                    .ok_or_else(|| ("FileNotFoundError".into(), "not found".into()))?;
                self.events.push(KernelEvent::Fs {
                    run_id: String::new(),
                    op: "Path.read_text".into(),
                    path,
                    bytes: text.len() as u64,
                });
                Ok(MontyObject::string(text))
            }
            _ => Err(("OSError".into(), "unsupported OS call".into())),
        }
    }
    fn take_events(&mut self) -> Vec<KernelEvent> {
        std::mem::take(&mut self.events)
    }
}
pub struct DenyOs;
impl OsBackend for DenyOs {
    fn handle(&mut self, _: OsFunctionCall) -> Result<MontyObject, (String, String)> {
        Err(("OSError".into(), "OS access is disabled".into()))
    }
}
pub fn exception(kind: &str, message: String) -> MontyException {
    MontyException::new(
        kind.parse::<ExcType>().unwrap_or(ExcType::RuntimeError),
        Some(message),
    )
}
fn host_result(function: &str, r: HostResult, limits: &RunLimits) -> ExtFunctionResult {
    match r {
        HostResult::Ok(v) => {
            match from_json_limited(&v, limits.max_result_nodes, limits.max_result_bytes) {
                Ok(value) => value.into(),
                Err(size) => exception(
                    "RuntimeError",
                    too_large_message(
                        &format!("{function}() returned a result"),
                        &size,
                        limits.max_result_nodes,
                        limits.max_result_bytes,
                    ),
                )
                .into(),
            }
        }
        HostResult::Err {
            exc_type,
            message,
        } => exception(&exc_type, message).into(),
    }
}
pub struct Kernel {
    repl: Option<MontyRepl>,
    os: Box<dyn OsBackend>,
    globals: HashSet<String>,
    positional: std::collections::HashMap<String, Vec<String>>,
    calls: std::collections::HashMap<u32, String>,
    shutdown: bool,
}
impl Kernel {
    pub fn new(os: Box<dyn OsBackend>) -> Self {
        Self {
            repl: None,
            os,
            globals: HashSet::new(),
            positional: Default::default(),
            calls: Default::default(),
            shutdown: false,
        }
    }
    fn session(&mut self) -> MontyRepl {
        self.repl.take().unwrap_or_else(|| {
            let mut repl = MontyRepl::new(
                "kernel.py",
                ResourceTracker::default(),
                CompileOptions::default(),
            );
            if let Some(cwd) = self.os.cwd() {
                repl.set_cwd(&cwd);
            }
            repl
        })
    }
    pub fn handle(&mut self, cmd: KernelCommand, io: &mut dyn KernelIo) -> io::Result<bool> {
        match cmd {
            KernelCommand::Configure {
                project_root,
                env,
                capture_dir,
                deny_write,
                deny_read,
            } => {
                self.os
                    .configure(project_root, env, capture_dir, deny_write, deny_read);
                if let Some(cwd) = self.os.cwd() {
                    let mut repl = self.session();
                    repl.set_cwd(&cwd);
                    self.repl = Some(repl);
                }
            }
            KernelCommand::Shutdown => return Ok(false),
            KernelCommand::Feed {
                code,
                globals,
                limits,
                run_id,
            } => self.feed(&run_id, &code, globals, limits, io)?,
            KernelCommand::Reset => {
                self.repl = None;
                self.globals.clear();
                io.emit(KernelEvent::ResetDone)?;
            }
            KernelCommand::Inspect => self.inspect(io)?,
            KernelCommand::Snapshot {
                path,
            } => {
                let repl = self.session();
                let result = (|| -> Result<_, Box<dyn std::error::Error>> {
                    let bytes = monty::dump("kernel.py", None, &[], SessionRef::Idle(&repl))?;
                    let prefix = format!(
                        "{}.",
                        std::path::Path::new(&path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    );
                    if let Some(parent) = std::path::Path::new(&path).parent() {
                        for entry in std::fs::read_dir(parent)?.flatten() {
                            let name = entry.file_name();
                            let name = name.to_string_lossy();
                            if name.starts_with(&prefix) && name.ends_with(".tmp") {
                                let _ = std::fs::remove_file(entry.path());
                            }
                        }
                    }
                    let tmp = format!("{path}.{:032x}.tmp", rand::random::<u128>());
                    let result = (|| {
                        use std::io::Write;
                        let mut f = std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&tmp)?;
                        f.write_all(&bytes)?;
                        f.sync_all()?;
                        std::fs::rename(&tmp, &path)
                    })();
                    if result.is_err() {
                        let _ = std::fs::remove_file(&tmp);
                    }
                    result?;
                    Ok(KernelEvent::SnapshotDone {
                        bytes: bytes.len() as u64,
                        sha256: Sha256::digest(&bytes)
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect(),
                    })
                })();
                self.repl = Some(repl);
                io.emit(result.unwrap_or_else(|e| KernelEvent::SnapshotFailed {
                    message: e.to_string(),
                }))?;
            }
            KernelCommand::LoadSnapshot {
                max_state_bytes,
                path,
                expected_sha256,
            } => {
                let result = (|| -> Result<MontyRepl, Box<dyn std::error::Error>> {
                    use std::io::Read;
                    let file = std::fs::File::open(path)?;
                    let mut bytes = Vec::new();
                    file.take(max_state_bytes.saturating_add(1))
                        .read_to_end(&mut bytes)?;
                    if bytes.len() as u64 > max_state_bytes {
                        return Err(format!(
                            "snapshot exceeds configured state limit ({max_state_bytes} bytes)"
                        )
                        .into());
                    }
                    let hash = Sha256::digest(&bytes)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>();
                    if hash != expected_sha256 {
                        return Err("snapshot hash mismatch".into());
                    }
                    match Dump::load(&bytes)?.state {
                        Session::Idle(r) => Ok(*r),
                        _ => Err("snapshot is not idle".into()),
                    }
                })();
                match result {
                    Ok(r) => {
                        self.repl = Some(r);
                        self.globals.clear();
                        io.emit(KernelEvent::Loaded)?
                    }
                    Err(e) => io.emit(KernelEvent::LoadFailed {
                        message: e.to_string(),
                    })?,
                }
            }
            KernelCommand::Abort {
                ..
            }
            | KernelCommand::Results {
                ..
            }
            | KernelCommand::SyncResult {
                ..
            } => {}
        }
        Ok(!self.shutdown)
    }
    fn feed(
        &mut self,
        run_id: &str,
        code: &str,
        globals: Vec<GlobalBinding>,
        limits: RunLimits,
        io: &mut dyn KernelIo,
    ) -> io::Result<()> {
        let mut repl = self.session();
        // Replace tracker per feed: execution budgets do not include host suspension.
        *repl.tracker_mut() = ResourceTracker::new(
            ResourceLimits {
                max_memory: Some(limits.max_memory_bytes.min(usize::MAX as u64) as usize),
                ..ResourceLimits::default()
            }
            .max_feed_duration(Duration::from_millis(limits.max_compute_ms)),
        );
        self.globals = globals.iter().map(|g| g.name.clone()).collect();
        self.positional = globals
            .iter()
            .filter_map(|g| match &g.kind {
                GlobalKind::AsyncFnPositional(names) => Some((g.name.clone(), names.clone())),
                _ => None,
            })
            .collect();
        // Bind asyncio only when the session has no `asyncio` of its own.
        if let Err(error) = repl.feed_run(
            "try:\n    asyncio\nexcept NameError:\n    import asyncio",
            NamedValues::new(),
            PrintWriter::Disabled,
        ) {
            self.repl = Some(repl);
            return io.emit(KernelEvent::Raised {
                run_id: run_id.into(),
                exc_type: error.exc_type().to_string(),
                message: format!(
                    "asyncio injection failed: {}",
                    error.message().unwrap_or_default()
                ),
                traceback: error.to_string(),
                line: None,
            });
        }
        let mut values = NamedValues::new();
        self.calls.clear();
        for g in globals {
            let value = match g.kind {
                GlobalKind::Json(v) => {
                    match from_json_limited(&v, limits.max_result_nodes, limits.max_result_bytes) {
                        Ok(value) => value,
                        Err(size) => {
                            self.repl = Some(repl);
                            let message = too_large_message(
                                &format!("global {} holds a value", g.name),
                                &size,
                                limits.max_result_nodes,
                                limits.max_result_bytes,
                            );
                            return io.emit(KernelEvent::Raised {
                                run_id: run_id.into(),
                                exc_type: "RuntimeError".into(),
                                traceback: message.clone(),
                                message,
                                line: None,
                            });
                        }
                    }
                }
                GlobalKind::AsyncFn => MontyObject::async_function(&g.name, None),
                GlobalKind::AsyncFnPositional(_) => MontyObject::async_function(&g.name, None),
                GlobalKind::SyncFn => MontyObject::function(&g.name, None),
            };
            values.push(g.name, value);
        }
        let script = repl.next_script_name();
        let mut output = Output::default();
        let mut progress = repl.feed_start(code, values, PrintWriter::Callback(&mut output));
        loop {
            if !output.text.is_empty() {
                io.emit(KernelEvent::Stdout {
                    run_id: run_id.into(),
                    text: std::mem::take(&mut output.text),
                })?;
            }
            let p = match progress {
                Ok(p) => p,
                Err(e) => {
                    let message = e.error.message().unwrap_or_default().to_owned();
                    let exc_type = e.error.exc_type().to_string();
                    let traceback = e.error.to_string();
                    let line = e
                        .error
                        .traceback()
                        .iter()
                        .rev()
                        .find(|f| f.filename == script)
                        .map(|f| f.start.line);
                    self.repl = Some(e.repl);
                    let ev = if output.exceeded {
                        KernelEvent::LimitExceeded {
                            run_id: run_id.into(),
                            kind: LimitKind::Output,
                            message,
                        }
                    } else if e.error.exc_type() == ExcType::TimeoutError {
                        KernelEvent::LimitExceeded {
                            run_id: run_id.into(),
                            kind: LimitKind::Compute,
                            message,
                        }
                    } else if e.error.exc_type() == ExcType::MemoryError {
                        KernelEvent::LimitExceeded {
                            run_id: run_id.into(),
                            kind: LimitKind::Memory,
                            message,
                        }
                    } else {
                        KernelEvent::Raised {
                            run_id: run_id.into(),
                            exc_type,
                            message,
                            traceback,
                            line,
                        }
                    };
                    io.emit(ev)?;
                    return Ok(());
                }
            };
            let print = PrintWriter::Callback(&mut output);
            progress = match p {
                ReplProgress::Complete {
                    repl,
                    value,
                } => {
                    self.repl = Some(repl);
                    let mut budget = Budget::new(limits.max_value_nodes, limits.max_value_bytes);
                    let value_json = budget.convert(value.as_ref());
                    let (repr, _) = bounded_repr(value.as_ref(), MAX_REPR_BYTES);
                    io.emit(KernelEvent::Completed {
                        run_id: run_id.into(),
                        value: value_json,
                        repr,
                        truncated: budget.truncated,
                    })?;
                    return Ok(());
                }
                ReplProgress::FunctionCall(call) => {
                    if call.is_async {
                        let names = self
                            .positional
                            .get(&call.function_name)
                            .cloned()
                            .unwrap_or_default();
                        let given = call.args.args().len();
                        let mut budget = Budget::new(limits.max_value_nodes, limits.max_arg_bytes);
                        let mut kwargs = serde_json::Map::new();
                        for (name, value) in names.iter().zip(call.args.args()) {
                            kwargs.insert(name.clone(), budget.convert(value));
                        }
                        let mut duplicate = None;
                        for (k, v) in call.args.kwargs() {
                            let key: String = k.as_str().unwrap_or_default().into();
                            if kwargs.contains_key(&key) {
                                duplicate = Some(key.clone());
                            }
                            kwargs.insert(key, budget.convert(v));
                        }
                        let refusal = if given > names.len() {
                            Some(if names.is_empty() {
                                "async host functions accept keyword arguments only".to_string()
                            } else {
                                format!(
                                    "{}() takes {} positional argument{} but {given} were given",
                                    call.function_name,
                                    names.len(),
                                    if names.len() == 1 { "" } else { "s" }
                                )
                            })
                        } else {
                            duplicate.map(|name| {
                                format!(
                                    "{}() got multiple values for argument '{name}'",
                                    call.function_name
                                )
                            })
                        };
                        if let Some(message) = refusal {
                            call.resume(exception("TypeError", message), print)
                        } else {
                            let kwargs = serde_json::Value::Object(kwargs);
                            let (line, col) = (call.position.filename == script)
                                .then(|| call_site(code, call.position.start as usize))
                                .flatten()
                                .unzip();
                            io.emit(KernelEvent::CallStarted {
                                line,
                                col,
                                run_id: run_id.into(),
                                call_id: call.call_id,
                                function: call.function_name.clone(),
                                kwargs,
                                truncated: budget.truncated,
                            })?;
                            self.calls.insert(call.call_id, call.function_name.clone());
                            call.resume_pending(print)
                        }
                    } else {
                        let id = call.call_id;
                        let mut budget = if call.function_name == DISPLAY_FUNCTION {
                            Budget::new(limits.max_display_nodes, limits.max_display_bytes)
                        } else {
                            Budget::new(limits.max_value_nodes, limits.max_arg_bytes)
                        };
                        let mut args = call
                            .args
                            .args()
                            .map(|v| budget.convert(v))
                            .collect::<Vec<_>>();
                        let promote = args.is_empty();
                        let mut kwargs = serde_json::Map::new();
                        for (key, value) in call.args.kwargs() {
                            let key = key.as_str().unwrap_or_default();
                            if promote && (key == "name" || key == "p") {
                                args.push(budget.convert(value));
                            } else {
                                kwargs.insert(key.into(), budget.convert(value));
                            }
                        }
                        let args = serde_json::json!(args);
                        io.emit(KernelEvent::SyncCall {
                            run_id: run_id.into(),
                            call_id: id,
                            function: call.function_name.clone(),
                            args,
                            kwargs: serde_json::Value::Object(kwargs),
                            truncated: budget.truncated,
                        })?;
                        match recv_active(io, run_id)? {
                            KernelCommand::Shutdown => {
                                self.shutdown = true;
                                self.repl = Some(call.into_repl());
                                return Ok(());
                            }
                            KernelCommand::SyncResult {
                                call_id,
                                result,
                            } if call_id == id => {
                                let result = host_result(&call.function_name, result, &limits);
                                call.resume(result, print)
                            }
                            _ => {
                                self.repl = Some(call.into_repl());
                                io.emit(KernelEvent::Cancelled {
                                    run_id: run_id.into(),
                                })?;
                                return Ok(());
                            }
                        }
                    }
                }
                ReplProgress::ResolveFutures(pending) => {
                    io.emit(KernelEvent::NeedResults {
                        run_id: run_id.into(),
                        pending: pending.pending_call_ids().to_vec(),
                    })?;
                    match recv_active(io, run_id)? {
                        KernelCommand::Shutdown => {
                            self.shutdown = true;
                            self.repl = Some(pending.into_repl());
                            return Ok(());
                        }
                        KernelCommand::Results {
                            results, ..
                        } => pending.resume(
                            results
                                .into_iter()
                                .map(|(id, r)| {
                                    let function =
                                        self.calls.remove(&id).unwrap_or_else(|| "host".into());
                                    (id, host_result(&function, r, &limits))
                                })
                                .collect(),
                            print,
                        ),
                        _ => {
                            self.repl = Some(pending.into_repl());
                            io.emit(KernelEvent::Cancelled {
                                run_id: run_id.into(),
                            })?;
                            return Ok(());
                        }
                    }
                }
                ReplProgress::OsCall(call) => {
                    let result = call.resume_with(print, |op| self.os.handle_result(op));
                    for mut event in self.os.take_events() {
                        match &mut event {
                            KernelEvent::Fs {
                                run_id: id, ..
                            }
                            | KernelEvent::Renamed {
                                run_id: id, ..
                            }
                            | KernelEvent::WriteCapture {
                                run_id: id, ..
                            } => *id = run_id.into(),
                            _ => {}
                        }
                        io.emit(event)?;
                    }
                    result
                }
                ReplProgress::NameLookup(call) => call.resume(NameLookupResult::Undefined, print),
            };
        }
    }
    fn inspect(&mut self, io: &mut dyn KernelIo) -> io::Result<()> {
        let mut repl = self.session();
        *repl.tracker_mut() = ResourceTracker::new(
            ResourceLimits::default().max_feed_duration(Duration::from_millis(200)),
        );
        let value = repl.feed_run("locals()", NamedValues::new(), PrintWriter::Disabled);
        self.repl = Some(repl);
        let mut items = vec![];
        let mut truncated = false;
        let mut previews_truncated = false;
        let mut total = 0u64;
        if let Ok(v) = value {
            if let Some(pairs) = v.as_ref().pairs() {
                for (k, v) in pairs {
                    if let Some(name) = k.as_str() {
                        let injected_asyncio = name == "asyncio"
                            && matches!(
                                monty_types::unstable::node(v),
                                monty_types::unstable::MontyNode::Repr(repr)
                                    if repr.starts_with("<module") && repr.contains("asyncio")
                            );
                        if injected_asyncio || name.starts_with('_') || self.globals.contains(name)
                        {
                            continue;
                        }
                        total += 1;
                        if items.len() == 200 {
                            truncated = true;
                            continue;
                        }
                        let (preview, cut) = bounded_repr(v, 480);
                        let cut = cut || preview.chars().count() > 120;
                        let preview = preview.chars().take(120).collect();
                        previews_truncated |= cut;
                        items.push(VariableSummary {
                            name: name.into(),
                            type_name: v.type_name().into(),
                            preview,
                        });
                    }
                }
            }
        }
        io.emit(KernelEvent::Variables {
            items,
            truncated,
            total: Some(total),
            previews_truncated,
        })
    }
}

fn recv_active(io: &mut dyn KernelIo, active: &str) -> io::Result<KernelCommand> {
    loop {
        let cmd = io.recv()?;
        match &cmd {
            KernelCommand::Abort {
                run_id,
            }
            | KernelCommand::Results {
                run_id, ..
            } if run_id != active => continue,
            _ => return Ok(cmd),
        }
    }
}
#[derive(Default)]
struct Output {
    text: String,
    bytes: usize,
    exceeded: bool,
}
impl monty_types::PrintWriterCallback for Output {
    fn stdout_write(&mut self, text: std::borrow::Cow<'_, str>) -> Result<(), MontyException> {
        let remaining = crate::protocol::MAX_STDOUT_BYTES.saturating_sub(self.bytes);
        if text.len() > remaining {
            let mut end = remaining;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.text.push_str(&text[..end]);
            self.bytes += end;
            self.exceeded = true;
            return Err(exception("MemoryError", "output limit exceeded".into()));
        }
        self.bytes += text.len();
        self.text.push_str(&text);
        Ok(())
    }
    fn stdout_push(&mut self, c: char) -> Result<(), MontyException> {
        self.stdout_write(c.to_string().into())
    }
}

fn call_site(code: &str, offset: usize) -> Option<(u32, u32)> {
    let before = code.get(..offset)?;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    Some((
        before.matches('\n').count() as u32 + 1,
        before[line_start..].chars().count() as u32,
    ))
}
