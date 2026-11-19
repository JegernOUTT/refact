use crate::{bindings::How, convert::*, protocol::*};
use monty::{Dump, MontyRepl, ReplProgress, Session, SessionRef};
use monty_types::{
    BuiltinsFunctions, CompileOptions, ExcType, ExtFunctionResult, MontyException, MontyObject,
    MontyType, NameLookupResult, NamedValues, ObjectRef, OsFunctionCall, PrintWriter,
    ResourceLimits, ResourceTracker,
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
    fn helper(&mut self, call: HelperCall<'_>) -> Result<serde_json::Value, HelperError> {
        Err(HelperError::Raise(
            "NameError".into(),
            format!("name '{}' is not available in this session", call.name),
        ))
    }
}
/// A call of a `GlobalKind::HostHelper` with its converted arguments.
pub struct HelperCall<'a> {
    pub name: &'a str,
    pub args: Vec<serde_json::Value>,
    pub kwargs: serde_json::Map<String, serde_json::Value>,
    /// Arguments whose JSON form hides their Python type: positions in `args` as "0", "1", …,
    /// keyword arguments by name.
    pub marks: Vec<(String, Mark)>,
    /// The largest result a script can receive; a helper refuses to build more.
    pub max_result_bytes: u64,
    /// Asks the engine for the files under a path (`GREP_FILES`).
    pub list_files: &'a mut dyn FnMut(&FileListRequest) -> Result<FileList, HelperError>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// A `pathlib.Path`, passed as its text.
    Path,
    /// An int outside the 64-bit range, passed as None.
    BigInt,
}
fn mark(value: ObjectRef<'_>) -> Option<Mark> {
    match value.type_name() {
        "PosixPath" => Some(Mark::Path),
        "int" if value.as_int().is_none() => Some(Mark::BigInt),
        _ => None,
    }
}
pub enum HelperError {
    Raise(String, String),
    Shutdown,
    Cancelled,
    Io(io::Error),
}
impl From<(String, String)> for HelperError {
    fn from((kind, message): (String, String)) -> Self {
        Self::Raise(kind, message)
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
/// Bound before every run, replacing a session value of the same name; hidden from the
/// variable list while they hold these values.
pub(crate) const DEFAULT_IMPORTS: &[(&str, &str)] = &[
    ("asyncio", "<module 'asyncio'>"),
    ("os", "<module 'os'>"),
    ("re", "<module 're'>"),
    ("json", "<module 'json'>"),
    ("Path", "<class 'PosixPath'>"),
];
const PRELUDE: &str = "import asyncio\nimport os\nimport re\nimport json\nfrom pathlib import Path";
pub struct Kernel {
    repl: Option<MontyRepl>,
    os: Box<dyn OsBackend>,
    globals: HashSet<String>,
    positional: std::collections::HashMap<String, Vec<String>>,
    calls: std::collections::HashMap<u32, String>,
    shutdown: bool,
    helpers: HashSet<String>,
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
            helpers: HashSet::new(),
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
                    use std::io::Write;
                    let bytes = monty::dump("kernel.py", None, &[], SessionRef::Idle(&repl))?;
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)?;
                    if let Err(e) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        return Err(e.into());
                    }
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
            }
            | KernelCommand::GrepRevalidated {
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
        self.helpers = host_helpers(&globals);
        if let Err(error) = repl.feed_run(PRELUDE, NamedValues::new(), PrintWriter::Disabled) {
            self.repl = Some(repl);
            return io.emit(KernelEvent::Raised {
                run_id: run_id.into(),
                exc_type: error.exc_type().to_string(),
                message: format!(
                    "default import injection failed: {}",
                    error.message().unwrap_or_default()
                ),
                traceback: error.to_string(),
                line: None,
            });
        }
        if let Some((line, message)) = refused_bindings(&mut repl, code, &globals) {
            let script = repl.next_script_name();
            let source = code
                .lines()
                .nth(line as usize - 1)
                .unwrap_or_default()
                .trim();
            self.repl = Some(repl);
            return io.emit(KernelEvent::Raised {
                run_id: run_id.into(),
                exc_type: "SyntaxError".into(),
                traceback: format!(
                    "  File \"{script}\", line {line}\n    {source}\nSyntaxError: {message}"
                ),
                message,
                line: Some(line),
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
                GlobalKind::HostHelper => MontyObject::function(&g.name, None),
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
                    if !call.is_async && self.helpers.contains(&call.function_name) {
                        let result = match self.host_helper(run_id, &call, &limits, io) {
                            Ok(value) => {
                                host_result(&call.function_name, HostResult::Ok(value), &limits)
                            }
                            Err(HelperError::Raise(kind, message)) => {
                                exception(&kind, message).into()
                            }
                            Err(HelperError::Shutdown) => {
                                self.shutdown = true;
                                self.repl = Some(call.into_repl());
                                return Ok(());
                            }
                            Err(HelperError::Cancelled) => {
                                self.repl = Some(call.into_repl());
                                return io.emit(KernelEvent::Cancelled {
                                    run_id: run_id.into(),
                                });
                            }
                            Err(HelperError::Io(e)) => {
                                self.repl = Some(call.into_repl());
                                return Err(e);
                            }
                        };
                        call.resume(result, print)
                    } else if call.is_async {
                        let names = self
                            .positional
                            .get(&call.function_name)
                            .cloned()
                            .unwrap_or_default();
                        let given = call.args.args().len();
                        let mut budget = Budget::with_mode(
                            limits.max_value_nodes,
                            limits.max_arg_bytes,
                            Mode::Strict,
                        );
                        let long = long_keyword(&call);
                        budget.frame(
                            given.min(names.len()) + call.args.kwargs().count(),
                            names.iter().take(given).map(String::as_str).chain(
                                call.args
                                    .kwargs()
                                    .map(|(k, _)| k.as_str().unwrap_or_default()),
                            ),
                        );
                        let mut kwargs = serde_json::Map::new();
                        let mut unsupported = None;
                        for (name, value) in names.iter().zip(call.args.args()) {
                            match budget.argument(name, value) {
                                Ok(value) => {
                                    kwargs.insert(name.clone(), value);
                                }
                                Err(e) => {
                                    unsupported.get_or_insert(e);
                                }
                            }
                        }
                        let mut duplicate = None;
                        for (k, v) in call.args.kwargs() {
                            let key = k.as_str().unwrap_or_default();
                            if key.len() > MAX_KEYWORD_BYTES {
                                continue;
                            }
                            let key: String = key.into();
                            if kwargs.contains_key(&key) {
                                duplicate = Some(key.clone());
                            }
                            match budget.argument(&key, v) {
                                Ok(value) => {
                                    kwargs.insert(key, value);
                                }
                                Err(e) => {
                                    unsupported.get_or_insert(e);
                                }
                            }
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
                            long.or_else(|| {
                                duplicate.map(|name| {
                                    format!(
                                        "{}() got multiple values for argument '{name}'",
                                        call.function_name
                                    )
                                })
                            })
                            .or_else(|| unsupported.map(|u| u.message(&call.function_name)))
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
                                exceeded: budget.exceeded.clone(),
                            })?;
                            self.calls.insert(call.call_id, call.function_name.clone());
                            call.resume_pending(print)
                        }
                    } else {
                        let id = call.call_id;
                        let mut budget = if call.function_name == DISPLAY_FUNCTION {
                            Budget::new(limits.max_display_nodes, limits.max_display_bytes)
                        } else if self.globals.contains(&call.function_name) {
                            Budget::with_mode(
                                limits.max_value_nodes,
                                limits.max_arg_bytes,
                                Mode::Helper,
                            )
                        } else {
                            Budget::new(limits.max_value_nodes, limits.max_arg_bytes)
                        };
                        let long = long_keyword(&call);
                        budget.frame(call.args.args().len(), std::iter::empty());
                        budget.frame(
                            call.args.kwargs().count(),
                            call.args
                                .kwargs()
                                .map(|(k, _)| k.as_str().unwrap_or_default()),
                        );
                        let mut unsupported = None;
                        let mut args = vec![];
                        for (i, value) in call.args.args().enumerate() {
                            match budget.argument(&(i + 1).to_string(), value) {
                                Ok(value) => args.push(value),
                                Err(e) => {
                                    unsupported.get_or_insert(e);
                                }
                            }
                        }
                        let promote = call.args.args().len() == 0;
                        let mut kwargs = serde_json::Map::new();
                        for (key, value) in call.args.kwargs() {
                            let key = key.as_str().unwrap_or_default();
                            if key.len() > MAX_KEYWORD_BYTES {
                                continue;
                            }
                            match budget.argument(key, value) {
                                Ok(value) if promote && (key == "name" || key == "p") => {
                                    args.push(value)
                                }
                                Ok(value) => {
                                    kwargs.insert(key.into(), value);
                                }
                                Err(e) => {
                                    unsupported.get_or_insert(e);
                                }
                            }
                        }
                        if let Some(message) =
                            long.or_else(|| unsupported.map(|u| u.message(&call.function_name)))
                        {
                            call.resume(exception("TypeError", message), print)
                        } else {
                            io.emit(KernelEvent::SyncCall {
                                run_id: run_id.into(),
                                call_id: id,
                                function: call.function_name.clone(),
                                args: serde_json::json!(args),
                                kwargs: serde_json::Value::Object(kwargs),
                                truncated: budget.truncated,
                                exceeded: budget.exceeded.clone(),
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
                    let result = call.resume_with(print, |op| match op {
                        OsFunctionCall::Import(args) => {
                            match crate::shims::module_file(&args.module) {
                                Some(file) => ExtFunctionResult::ModuleFile(file),
                                None => self.os.handle_result(OsFunctionCall::Import(args)),
                            }
                        }
                        op => self.os.handle_result(op),
                    });
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
            ResourceLimits::default().max_feed_duration(Duration::from_millis(1000)),
        );
        let mut inputs = NamedValues::new();
        inputs.push(
            INSPECT_NAME,
            MontyObject::tuple([
                MontyObject::builtin_function(BuiltinsFunctions::Type),
                MontyObject::builtin_function(BuiltinsFunctions::Len),
                MontyObject::builtin_function(BuiltinsFunctions::Repr),
                MontyObject::builtin_function(BuiltinsFunctions::Zip),
                MontyObject::type_object(MontyType::Range),
                MontyObject::builtin_function(BuiltinsFunctions::Sum),
                MontyObject::builtin_function(BuiltinsFunctions::All),
                MontyObject::type_object(MontyType::Module),
            ]),
        );
        let summary = repl
            .feed_run(INSPECT, inputs, PrintWriter::Disabled)
            .ok()
            .map(|value| Budget::new(u32::MAX, MAX_VALUE_BYTES).convert(value.as_ref()));
        self.repl = Some(repl);
        let mut items = vec![];
        let mut truncated = false;
        let mut previews_truncated = false;
        let mut measured = vec![];
        let mut unmeasured: Vec<VariableSize> = vec![];
        let listed = summary.as_ref().and_then(|summary| {
            let [count, rows] = summary.as_array()?.as_slice() else {
                return None;
            };
            Some((count.as_u64()?, rows.as_array()?))
        });
        let total = listed.map(|(count, rows)| {
            let mut total = 0u64;
            for row in rows {
                let Some([name, type_name, len, approx, preview]) =
                    row.as_array().map(Vec::as_slice)
                else {
                    continue;
                };
                let (Some(name), Some(type_name)) = (name.as_str(), type_name.as_str()) else {
                    continue;
                };
                let injected = DEFAULT_IMPORTS.iter().any(|(default, _)| *default == name)
                    && matches!(type_name, "module" | "type");
                if injected || self.globals.contains(name) {
                    continue;
                }
                total += 1;
                let len = len.as_u64();
                let size = VariableSize {
                    name: name.into(),
                    type_name: type_name.into(),
                    len,
                    approx_bytes: approx.as_u64(),
                };
                if items.len() < 200 {
                    let summarized = preview.is_null();
                    let preview = match (preview.as_str(), len) {
                        (Some(preview), _) => preview.to_string(),
                        (None, Some(len)) => format!("{type_name}[{len}]"),
                        (None, None) => format!("<{type_name}>"),
                    };
                    let cut = summarized
                        || preview.chars().count() > 120
                        || matches!(type_name, "str" | "bytes") && len.is_some_and(|n| n > 120);
                    previews_truncated |= cut;
                    items.push(VariableSummary {
                        name: name.into(),
                        type_name: type_name.into(),
                        preview: preview.chars().take(120).collect(),
                    });
                } else {
                    truncated = true;
                }
                match size.approx_bytes {
                    Some(0) => {}
                    Some(_) => measured.push(size),
                    None => unmeasured.push(size),
                }
            }
            if count > INSPECT_ROWS {
                truncated = true;
                None
            } else {
                Some(total)
            }
        });
        measured.sort_by_key(|size: &VariableSize| std::cmp::Reverse(size.approx_bytes));
        unmeasured.sort_by_key(|size| !instance_of_script_class(&size.type_name));
        let unmeasured_count = unmeasured.len();
        let mut largest: Vec<VariableSize> = measured.into_iter().take(5).collect();
        largest.extend(unmeasured.into_iter().take(5));
        io.emit(KernelEvent::Variables {
            items,
            truncated,
            total: total.flatten(),
            previews_truncated,
            unknown: total.is_none(),
            largest,
            unmeasured: unmeasured_count,
        })
    }
}

/// The name `INSPECT` reads its builtins from; a script may not bind it at module scope.
const INSPECT_NAME: &str = "__refact__";
/// Session variables listed by `INSPECT` at most.
const INSPECT_ROWS: u64 = 10_000;
/// `[count of globals, [(name, type name, len or -1, estimated bytes or -1, preview or None)]]`;
/// names and type names are cut at 200 characters.
/// It calls only builtins the host binds as `__refact__` and types of literals, so a name the
/// script rebound is never called, and no value of an unknown type is shown or measured;
/// `locals` itself is called only while the session has not bound that name. Sizes are estimated
/// from the first 32 items of a container; functions, classes and modules count as 0.
const INSPECT: &str = "(lambda g, T, N, R, Z, G, U, A, MO: (lambda S, B, L, TU, D, SE, I, F, BO, NT, FN, BF, TY: \
(lambda C, ok, es, tn: [N(g), [(k[:200], tn(T(v)), N(v) if T(v) in C else -1, \
N(v) if T(v) in (S, B) else 32 if T(v) in (I, F, BO, NT) else 0 if T(v) in (FN, BF, TY, MO) else \
64 + N(v) * U(es(a) + es(b) for _, (a, b) in Z(G(32), v.items())) // ((N(v) if N(v) < 32 else 32) or 1) if T(v) is D else \
64 + N(v) * U(es(e) for _, e in Z(G(32), v)) // ((N(v) if N(v) < 32 else 32) or 1) if T(v) in (L, TU, SE) else -1, \
R(v[:120]) if T(v) in (S, B) else R(v) if T(v) in (FN, BF, TY) or ok(ok, v, 0) else None) \
for _, (k, v) in Z(G(10000), g.items()) if not k.startswith('_')]])(\
(S, B, L, TU, D, SE), \
lambda f, e, d: T(e) in (F, BO, NT) or T(e) is I and -10**60 < e < 10**60 or T(e) in (S, B) and N(e) <= 100 \
or d < 2 and T(e) in (L, TU, SE) and N(e) <= 20 and A(f(f, x, d + 1) for x in e) \
or d < 2 and T(e) is D and N(e) <= 20 and A(f(f, a, d + 1) and f(f, b, d + 1) for a, b in e.items()), \
lambda e: N(e) + 50 if T(e) in (S, B) else 64 + 8 * N(e) if T(e) in (L, TU, D, SE) else 32 if T(e) in (I, F, BO, NT) else 64, \
lambda t: (lambda n: n[:200] if T(n) is S else '?')(t.__name__)))(\
T(''), T(b''), T([]), T(()), T({}), T({1}), T(0), T(0.0), T(True), T(None), T(lambda: 0), T(N), T(T(''))))\
(locals(), *__refact__) if [locals for locals in [None]] != [None] else None";

/// The line and text of a refusal: the script binds at module scope a name the session binds
/// again before every run, or reads a module-level comprehension variable that this
/// interpreter would read as the builtin of the same name. Nothing is refused when the
/// interpreter itself rejects the script; it reports its own error.
fn refused_bindings(
    repl: &mut MontyRepl,
    code: &str,
    globals: &[GlobalBinding],
) -> Option<(u32, String)> {
    let inputs: Vec<String> = globals.iter().map(|g| g.name.clone()).collect();
    if repl.check_source(code).is_err()
        || monty::MontyRun::new(code.into(), "check.py", inputs, CompileOptions::default()).is_err()
    {
        return None;
    }
    let scan = crate::bindings::scan(code)?;
    let line = |offset: usize| code[..offset].matches('\n').count() as u32 + 1;
    let mut clashes = Vec::new();
    let mut seen = HashSet::new();
    for binding in &scan.bindings {
        let role = match globals.iter().find(|g| g.name == binding.name) {
            Some(global) => Some(match &global.kind {
                GlobalKind::AsyncFn | GlobalKind::AsyncFnPositional(_) => (
                    "an async function bound by this session (a tool or helper)",
                    "async function",
                ),
                GlobalKind::SyncFn => ("a helper bound by this session", "helper"),
                GlobalKind::HostHelper => ("a helper bound by this session", "helper"),
                GlobalKind::Json(_) => ("a constant set before every run", "constant"),
            }),
            None if binding.canonical => None,
            None if binding.name == INSPECT_NAME => Some((
                "a name this session reserves for listing variables",
                "reserved name",
            )),
            None if binding.name == "Path" => {
                Some(("the class pathlib.Path, imported before every run", "class"))
            }
            None if DEFAULT_IMPORTS
                .iter()
                .any(|(name, _)| *name == binding.name) =>
            {
                Some(("a module imported before every run", "module"))
            }
            None => None,
        };
        if let Some(role) = role {
            if seen.insert(binding.name.clone()) {
                clashes.push((binding, role));
            }
        }
    }
    fn glob_module(b: &crate::bindings::Binding) -> bool {
        b.import.as_deref().is_some_and(|import| {
            import == "import glob"
                || import.starts_with("import glob.")
                || import.starts_with("from glob import ")
        })
    }
    let renamed = |name: &str| {
        if name == INSPECT_NAME {
            "refact_".to_string()
        } else {
            format!("{name}_")
        }
    };
    if let [(binding, (role, _))] = clashes.as_slice() {
        let at = line(binding.offset);
        let name = &binding.name;
        if name == INSPECT_NAME {
            return Some((at, format!("line {at} binds '{name}', {role}; a value kept there would be replaced. Rename it (for example '{}'). Nothing in this script ran.", renamed(name))));
        }
        if glob_module(binding) {
            return Some((at, format!("line {at} imports glob, but glob is the file-search tool (await glob(pattern=...)); for local patterns use Path(dir).glob(...) or .rglob(...). Remove the import. Nothing in this script ran.")));
        }
        let (verb, rename) = match (binding.how, &binding.import) {
            (How::Imports, Some(import)) => (
                format!("binds '{name}' with an import"),
                format!("'{import} as {name}_'"),
            ),
            (How::Defines, _) => (format!("defines '{name}'"), format!("'{name}_'")),
            _ => (format!("binds '{name}'"), format!("'{name}_'")),
        };
        return Some((at, format!("line {at} {verb}, {role}. Tool functions, helpers, constants and the pre-imported modules are bound again before every run, so a value you keep in '{name}' would be replaced by the next run. Rename it (for example {rename}). Nothing in this script ran.")));
    }
    if let Some((first, _)) = clashes.first() {
        let listed: Vec<String> = clashes
            .iter()
            .take(5)
            .map(|(b, (_, short))| match (glob_module(b), b.name.as_str()) {
                (true, "glob") => format!(
                    "line {} 'glob' (the glob tool; there is no glob module)",
                    line(b.offset)
                ),
                (true, name) => format!(
                    "line {} '{name}' (imported from glob; there is no glob module)",
                    line(b.offset)
                ),
                (false, name) => format!("line {} '{name}' ({short})", line(b.offset)),
            })
            .collect();
        let renames: Vec<String> = clashes
            .iter()
            .take(5)
            .filter(|(b, _)| !glob_module(b))
            .map(|(b, _)| format!("'{}'", renamed(&b.name)))
            .collect();
        let more = if clashes.len() > 5 {
            format!(", and {} more", clashes.len() - 5)
        } else {
            String::new()
        };
        let imports = match clashes
            .iter()
            .take(5)
            .filter(|(b, _)| glob_module(b))
            .count()
        {
            0 => None,
            1 => Some("Remove the glob import"),
            _ => Some("Remove the glob imports"),
        };
        let fix = match (imports, renames.is_empty()) {
            (Some(remove), true) => remove.to_string(),
            (Some(remove), false) => format!(
                "{remove} and rename the others (for example {})",
                renames.join(", ")
            ),
            (None, _) => format!("Rename them (for example {})", renames.join(", ")),
        };
        return Some((line(first.offset), format!("this script binds names the session binds again before every run: {}{more}. {fix}. Nothing in this script ran.", listed.join(", "))));
    }
    let candidates: Vec<&(String, usize)> = scan
        .builtin_reads
        .iter()
        .filter(|(name, offset)| {
            !globals.iter().any(|g| &g.name == name)
                && !scan
                    .bindings
                    .iter()
                    .any(|b| &b.name == name && b.effective < *offset)
        })
        .collect();
    let mut found: Vec<(&str, u32)> = Vec::new();
    for (name, offset) in candidates {
        let name = name.as_str();
        if found.iter().any(|(n, _)| *n == name) {
            continue;
        }
        let probe = format!("[{name} for {name} in [None]] == [None]");
        let substituted = matches!(
            repl.feed_run(&probe, NamedValues::new(), PrintWriter::Disabled),
            Ok(value) if value.as_ref().as_bool() == Some(false)
        );
        if substituted {
            found.push((name, line(*offset)));
        }
    }
    match found.as_slice() {
        [] => None,
        [(name, at)] => Some((*at, format!("line {at}: the comprehension variable '{name}' has the name of a builtin, and at module level this interpreter reads the builtin '{name}' instead of the variable. Rename the variable (for example '{name}_'). Nothing in this script ran."))),
        [(_, at), ..] => {
            let listed: Vec<String> = found
                .iter()
                .map(|(name, at)| format!("'{name}' (line {at})"))
                .collect();
            let renames: Vec<String> = found.iter().map(|(name, _)| format!("'{name}_'")).collect();
            Some((*at, format!("line {at}: the comprehension variables {} have names of builtins, and at module level this interpreter reads the builtins instead of the variables. Rename them (for example {}). Nothing in this script ran.", listed.join(", "), renames.join(", "))))
        }
    }
}
/// Keyword argument names longer than this are refused before they are copied.
const MAX_KEYWORD_BYTES: usize = 256;
fn long_keyword(call: &monty::ReplFunctionCall) -> Option<String> {
    call.args
        .kwargs()
        .map(|(k, _)| k.as_str().unwrap_or_default().len())
        .find(|n| *n > MAX_KEYWORD_BYTES)
        .map(|n| {
            format!(
                "{}() got a keyword argument name of {n} bytes; names are limited to {MAX_KEYWORD_BYTES} bytes",
                call.function_name
            )
        })
}
/// Whether a type name from the listing names a class a script defined, not a builtin type.
fn instance_of_script_class(type_name: &str) -> bool {
    MontyType::from_type_name(type_name).is_none()
        && [
            "datetime",
            "collections",
            "re",
            "_io",
            "typing",
            "itertools",
        ]
        .iter()
        .all(|module| MontyType::from_type_name(&format!("{module}.{type_name}")).is_none())
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

fn host_helpers(globals: &[GlobalBinding]) -> HashSet<String> {
    globals
        .iter()
        .filter(|g| matches!(g.kind, GlobalKind::HostHelper))
        .map(|g| g.name.clone())
        .collect()
}
impl Kernel {
    fn host_helper(
        &mut self,
        run_id: &str,
        call: &monty::ReplFunctionCall,
        limits: &RunLimits,
        io: &mut dyn KernelIo,
    ) -> Result<serde_json::Value, HelperError> {
        let name = call.function_name.as_str();
        let mut budget =
            Budget::with_mode(limits.max_value_nodes, limits.max_arg_bytes, Mode::Helper);
        if let Some(message) = long_keyword(call) {
            return Err(HelperError::Raise("TypeError".into(), message));
        }
        budget.frame(call.args.args().len(), std::iter::empty());
        budget.frame(
            call.args.kwargs().count(),
            call.args
                .kwargs()
                .map(|(k, _)| k.as_str().unwrap_or_default()),
        );
        let mut unsupported = None;
        let mut marks = vec![];
        let mut args = vec![];
        for (i, value) in call.args.args().enumerate() {
            let marked = mark(value);
            if let Some(marked) = marked {
                marks.push((i.to_string(), marked));
            }
            if marked == Some(Mark::BigInt) {
                args.push(serde_json::Value::Null);
                continue;
            }
            match budget.argument(&(i + 1).to_string(), value) {
                Ok(value) => args.push(value),
                Err(e) => {
                    unsupported.get_or_insert(e);
                }
            }
        }
        let mut kwargs = serde_json::Map::new();
        for (key, value) in call.args.kwargs() {
            let key = key.as_str().unwrap_or_default();
            let marked = mark(value);
            if let Some(marked) = marked {
                marks.push((key.into(), marked));
            }
            if marked == Some(Mark::BigInt) {
                kwargs.insert(key.into(), serde_json::Value::Null);
                continue;
            }
            match budget.argument(key, value) {
                Ok(value) => {
                    kwargs.insert(key.into(), value);
                }
                Err(e) => {
                    unsupported.get_or_insert(e);
                }
            }
        }
        if let Some(unsupported) = unsupported {
            return Err(HelperError::Raise(
                "TypeError".into(),
                unsupported.message(name),
            ));
        }
        if budget.truncated {
            let size = budget
                .exceeded
                .as_ref()
                .map_or_else(|| "over the size limit".into(), Exceeded::describe);
            return Err(HelperError::Raise(
                "TypeError".into(),
                format!("{name}() arguments are too large: {size}; pass a smaller value"),
            ));
        }
        let id = call.call_id;
        let mut admitted = false;
        let mut list_files = |request: &FileListRequest| -> Result<FileList, HelperError> {
            let request = serde_json::to_value(request).map_err(|e| {
                HelperError::Raise(
                    "OSError".into(),
                    format!("{name}(): the engine cannot list files under a path that is not UTF-8 ({e})"),
                )
            })?;
            io.emit(KernelEvent::SyncCall {
                truncated: false,
                run_id: run_id.into(),
                call_id: id,
                function: GREP_FILES.into(),
                args: serde_json::Value::Array(vec![request]),
                kwargs: serde_json::Value::Object(Default::default()),
                exceeded: None,
            })
            .map_err(HelperError::Io)?;
            match recv_active(&mut *io, run_id).map_err(HelperError::Io)? {
                KernelCommand::Shutdown => Err(HelperError::Shutdown),
                KernelCommand::SyncResult {
                    call_id,
                    result,
                } if call_id == id => match result {
                    HostResult::Ok(value) => {
                        let files = serde_json::from_value(value).map_err(|e| {
                            HelperError::Raise(
                                "RuntimeError".into(),
                                format!("{name}(): the engine sent a malformed file list: {e}"),
                            )
                        })?;
                        admitted = true;
                        Ok(files)
                    }
                    HostResult::Err {
                        exc_type,
                        message,
                    } => Err(HelperError::Raise(exc_type, message)),
                },
                _ => Err(HelperError::Cancelled),
            }
        };
        let result = self.os.helper(HelperCall {
            name,
            args,
            kwargs,
            marks,
            max_result_bytes: result_byte_limit(limits.max_result_bytes),
            list_files: &mut list_files,
        });
        let events = self.os.take_events();
        if admitted {
            io.emit(KernelEvent::GrepRevalidate {
                run_id: run_id.into(),
                call_id: id,
            })
            .map_err(HelperError::Io)?;
            match recv_active(io, run_id) {
                Ok(KernelCommand::GrepRevalidated {
                    run_id: echoed_run,
                    call_id,
                    allowed: true,
                }) if echoed_run == run_id && call_id == id => {}
                Ok(KernelCommand::Shutdown) => return Err(HelperError::Shutdown),
                Ok(KernelCommand::Abort {
                    ..
                }) => return Err(HelperError::Cancelled),
                _ => {
                    return Err(HelperError::Raise(
                        "RuntimeError".into(),
                        "grep(): results unavailable".into(),
                    ));
                }
            }
        }
        if matches!(result, Ok(_) | Err(HelperError::Raise(..))) {
            for mut event in events {
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
                io.emit(event).map_err(HelperError::Io)?;
            }
        }
        result
    }
}
