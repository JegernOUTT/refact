use std::io::{Read, Write};
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty};
use process_wrap::tokio::{TokioChildWrapper, TokioCommandWrap};
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
#[cfg(windows)]
use process_wrap::tokio::JobObject;
use refact_core::net_utils::is_someone_listening_on_that_tcp_port;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::task::JoinHandle;

use crate::env::{apply_pty_child_env, apply_tokio_child_env};
use crate::observe::ObservationStatus;
#[cfg(target_os = "linux")]
use crate::observe::{Handle as ObservationHandle, Setup};
use crate::registry::{ExecProcessCommand, ExecProcessRuntime};
use crate::types::{
    ExecMode, ExecOutputStream, ExecProcessId, ExecProcessMeta, ExecProcessSnapshot,
    ExecReadinessProbe, ExecSpawnRequest, ExecStatus,
};
use crate::ExecRegistry;

const PIPE_READ_BYTES: usize = 8192;
const KILL_REAP_TIMEOUT: Duration = Duration::from_secs(2);
const KILL_PUMP_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(not(test))]
const EXIT_PUMP_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const EXIT_PUMP_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);
const ABORT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(50);
const READINESS_PORT_CONNECT_TIMEOUT: Duration = Duration::from_millis(100);

pub struct ExecSpawnResult {
    pub snapshot: ExecProcessSnapshot,
    pub observation: ObservationStatus,
}

impl ExecSpawnResult {
    fn new(snapshot: ExecProcessSnapshot, observation: ObservationStatus) -> Self {
        Self {
            snapshot,
            observation,
        }
    }
}

struct PtyRuntimeProcess {
    child: Box<dyn portable_pty::Child + Send>,
    process_id: Option<u32>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
}

enum RuntimeChild {
    Tokio(Box<dyn TokioChildWrapper>),
    Pty(PtyRuntimeProcess),
}

impl RuntimeChild {
    fn is_pty(&self) -> bool {
        matches!(self, RuntimeChild::Pty(_))
    }

    fn start_kill(&mut self) -> Result<(), String> {
        match self {
            RuntimeChild::Tokio(child) => child
                .start_kill()
                .map_err(|error| format!("failed to kill process: {error}")),
            RuntimeChild::Pty(process) => start_kill_pty(process),
        }
    }

    fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        match self {
            RuntimeChild::Pty(process) => process
                .master
                .resize(crate::pty::pty_size(rows, cols))
                .map_err(|error| format!("failed to resize pty: {error}")),
            RuntimeChild::Tokio(_) => Err("process is not PTY-backed".to_string()),
        }
    }

    fn try_wait_exit_code(&mut self) -> Result<Option<Option<i32>>, String> {
        match self {
            RuntimeChild::Tokio(child) => child
                .try_wait()
                .map(|status| status.map(|status| status.code()))
                .map_err(|error| format!("failed to check process status: {error}")),
            RuntimeChild::Pty(process) => process
                .child
                .try_wait()
                .map(|status| status.map(|status| Some(status.exit_code() as i32)))
                .map_err(|error| format!("failed to check process status: {error}")),
        }
    }
}

#[cfg(unix)]
fn start_kill_pty(process: &mut PtyRuntimeProcess) -> Result<(), String> {
    if let Ok(mut writer) = process.writer.try_lock() {
        let _ = writer.write_all(&[3]);
        let _ = writer.flush();
    }
    let mut errors = Vec::new();
    match process.process_id {
        Some(process_id) => {
            for signal in [libc::SIGTERM, libc::SIGKILL] {
                if let Err(error) = signal_pty_process_group(process_id, signal) {
                    errors.push(error);
                }
            }
        }
        None => errors.push("PTY child has no process id".to_string()),
    }
    if let Err(error) = process.child.kill() {
        if error.raw_os_error() != Some(libc::ESRCH) {
            errors.push(format!("failed to kill PTY child: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(not(unix))]
fn start_kill_pty(process: &mut PtyRuntimeProcess) -> Result<(), String> {
    process
        .child
        .kill()
        .map_err(|error| format!("failed to kill process: {error}"))
}

#[cfg(unix)]
fn signal_pty_process_group(process_id: u32, signal: libc::c_int) -> Result<(), String> {
    if unsafe { libc::kill(-(process_id as i32), signal) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!(
            "failed to signal PTY process group {process_id} with signal {signal}: {error}"
        ))
    }
}

fn shell_parts() -> (&'static str, &'static str) {
    if cfg!(target_os = "windows") {
        ("powershell.exe", "-Command")
    } else {
        ("sh", "-c")
    }
}

fn ensure_command_is_not_empty(request: &ExecSpawnRequest) -> Result<(), String> {
    match request.argv.as_ref() {
        Some(argv)
            if argv
                .first()
                .is_some_and(|program| !program.trim().is_empty()) => {}
        Some(_) => return Err("Command argv is empty".to_string()),
        None if request.command.trim().is_empty() => return Err("Command is empty".to_string()),
        None => {}
    }
    Ok(())
}

fn launch_parts(request: &ExecSpawnRequest) -> Result<(String, Vec<String>), String> {
    ensure_command_is_not_empty(request)?;
    let (program, args) = if let Some(argv) = request.argv.as_ref() {
        (argv[0].clone(), argv[1..].to_vec())
    } else {
        let (shell, shell_arg) = shell_parts();
        (
            shell.to_string(),
            vec![shell_arg.to_string(), request.command.clone()],
        )
    };
    let Some(spec) = request.sandbox.as_ref() else {
        return Ok((program, args));
    };
    let cwd = match request.cwd.as_deref() {
        Some(cwd) => cwd.to_path_buf(),
        None => std::env::current_dir()
            .map_err(|error| format!("sandbox: cannot resolve working directory: {error}"))?,
    };
    let spec = refact_sandbox::ExecSandboxSpec {
        mode: match spec.mode {
            crate::types::ExecSandboxMode::ReadOnly => refact_sandbox::SandboxMode::ReadOnly,
            crate::types::ExecSandboxMode::WorkspaceWrite => {
                refact_sandbox::SandboxMode::WorkspaceWrite
            }
            crate::types::ExecSandboxMode::FullAccess => refact_sandbox::SandboxMode::FullAccess,
        },
        ro_paths: spec.ro_paths.clone(),
        rw_paths: spec.rw_paths.clone(),
        allow_network: spec.allow_network,
    }
    .normalized(&cwd);
    let (provider, _) = refact_sandbox::select_provider();
    provider
        .confine(&spec, &program, &args)
        .map_err(|error| error.to_string())
}

fn shell_command(request: &ExecSpawnRequest) -> Result<tokio::process::Command, String> {
    let (program, args) = launch_parts(request)?;
    let mut command = tokio::process::Command::new(program);
    command.args(args);
    command.kill_on_drop(true);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    if let Some(cwd) = request.cwd.as_ref() {
        command.current_dir(cwd);
    }
    apply_tokio_child_env(&mut command, &request.env_policy, &request.env);
    Ok(command)
}

fn pty_command(request: &ExecSpawnRequest) -> Result<CommandBuilder, String> {
    let (program, args) = launch_parts(request)?;
    let mut command = CommandBuilder::new(program);
    command.args(args);
    if let Some(cwd) = request.cwd.as_ref() {
        command.cwd(cwd.as_os_str());
    }
    apply_pty_child_env(&mut command, &request.env_policy, &request.env);
    Ok(command)
}

fn build_process_meta(
    request: &ExecSpawnRequest,
) -> Result<(ExecProcessMeta, ExecProcessId), String> {
    let owner = request.owner.clone().with_normalized_workspace();
    let mut meta = ExecProcessMeta::new(request.mode.clone(), request.command.clone())
        .with_owner(owner.clone())
        .with_tty(request.tty);
    if matches!(request.mode, ExecMode::Service) {
        let service_name = request
            .owner
            .service_name
            .as_deref()
            .ok_or_else(|| "service mode requires service_name".to_string())?;
        meta = meta.with_process_id(ExecProcessId::for_service(service_name, &owner));
    }
    if let Some(cwd) = request.cwd.clone() {
        meta = meta.with_cwd(cwd);
    }
    if let Some(short_description) = request.short_description.clone() {
        meta = meta.with_short_description(short_description);
    }
    let process_id = meta.process_id.clone();
    Ok((meta, process_id))
}

fn wrap_command(command: tokio::process::Command) -> TokioCommandWrap {
    let mut command_wrap = TokioCommandWrap::from(command);
    #[cfg(unix)]
    command_wrap.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command_wrap.wrap(JobObject);
    command_wrap
}

fn output_to_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&strip_ansi_escapes::strip(bytes)).to_string()
}

fn incomplete_utf8_suffix_len(bytes: &[u8]) -> usize {
    let len = bytes.len();
    let start = len.saturating_sub(3);
    for index in (start..len).rev() {
        let byte = bytes[index];
        let seq_len = if byte >= 0xF0 {
            4
        } else if byte >= 0xE0 {
            3
        } else if byte >= 0xC0 {
            2
        } else if byte < 0x80 {
            1
        } else {
            continue;
        };
        return if index + seq_len > len {
            len - index
        } else {
            0
        };
    }
    0
}

#[derive(Default)]
struct Utf8ChunkDecoder {
    pending: Vec<u8>,
}

impl Utf8ChunkDecoder {
    fn decode(&mut self, bytes: &[u8]) -> String {
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(bytes);
        let split = data.len() - incomplete_utf8_suffix_len(&data);
        self.pending = data.split_off(split);
        String::from_utf8_lossy(&data).into_owned()
    }

    fn finish(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        let pending = std::mem::take(&mut self.pending);
        Some(String::from_utf8_lossy(&pending).into_owned())
    }
}

fn pump_output(
    registry: ExecRegistry,
    process_id: crate::types::ExecProcessId,
    stream: ExecOutputStream,
    mut pipe: impl AsyncRead + Unpin + Send + 'static,
    progress_tx: Option<mpsc::UnboundedSender<crate::types::ExecOutputChunk>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = [0; PIPE_READ_BYTES];
        loop {
            match pipe.read(&mut buffer).await {
                Ok(0) => break,
                Ok(bytes_read) => {
                    let text = output_to_text(&buffer[..bytes_read]);
                    if !text.is_empty() {
                        if let Ok(chunk) = registry
                            .append_output(&process_id, stream.clone(), text)
                            .await
                        {
                            if let Some(progress_tx) = progress_tx.as_ref() {
                                let _ = progress_tx.send(chunk);
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!("exec output pump failed for {process_id}: {error}");
                    break;
                }
            }
        }
    })
}

fn pump_blocking_output(
    registry: ExecRegistry,
    process_id: crate::types::ExecProcessId,
    stream: ExecOutputStream,
    mut reader: Box<dyn Read + Send>,
    progress_tx: Option<mpsc::UnboundedSender<crate::types::ExecOutputChunk>>,
) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let mut buffer = [0; PIPE_READ_BYTES];
        let mut decoder = Utf8ChunkDecoder::default();
        let append_raw = |raw_text: String| {
            if raw_text.is_empty() {
                return;
            }
            let text = output_to_text(raw_text.as_bytes());
            if let Ok(chunk) = futures::executor::block_on(registry.append_output_with_raw(
                &process_id,
                stream.clone(),
                text,
                &raw_text,
            )) {
                if !chunk.text.is_empty() {
                    if let Some(progress_tx) = progress_tx.as_ref() {
                        let _ = progress_tx.send(chunk);
                    }
                }
            }
        };
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    if let Some(raw_text) = decoder.finish() {
                        append_raw(raw_text);
                    }
                    break;
                }
                Ok(bytes_read) => {
                    append_raw(decoder.decode(&buffer[..bytes_read]));
                }
                Err(error) => {
                    tracing::warn!("exec output pump failed for {process_id}: {error}");
                    break;
                }
            }
        }
    })
}

async fn finish_pumps_with_timeout(
    mut stdout_task: JoinHandle<()>,
    mut stderr_task: JoinHandle<()>,
    timeout: Duration,
) -> bool {
    let wait = async {
        let _ = tokio::join!(&mut stdout_task, &mut stderr_task);
    };
    if tokio::time::timeout(timeout, wait).await.is_ok() {
        return true;
    }
    stdout_task.abort();
    stderr_task.abort();
    false
}

fn pump_drain_timeout_status(timeout: Duration) -> ExecStatus {
    ExecStatus::Failed {
        message: format!(
            "output drain timed out after {:.3}s; descendant process may have inherited stdout/stderr",
            timeout.as_secs_f64()
        ),
    }
}

async fn kill_and_reap(child: &Arc<Mutex<RuntimeChild>>) -> Result<(), String> {
    let kill_result = {
        let mut child = child.lock().await;
        child.start_kill()
    };
    let wait_result = tokio::time::timeout(KILL_REAP_TIMEOUT, reap_child_after_kill(child)).await;
    kill_reap_result(kill_result, wait_result)
}

#[cfg(target_os = "linux")]
async fn kill_and_reap_observed(
    child: &Arc<Mutex<RuntimeChild>>,
    observation: &Option<ObservationHandle>,
) -> Result<(), String> {
    let Some(observation) = observation else {
        return kill_and_reap(child).await;
    };
    let kill_result = {
        let mut child = child.lock().await;
        child.start_kill()
    };
    let wait_result = tokio::time::timeout(KILL_REAP_TIMEOUT, observation.wait_exit()).await;
    kill_reap_result(kill_result, wait_result)
}

#[cfg(not(target_os = "linux"))]
async fn kill_and_reap_observed(
    child: &Arc<Mutex<RuntimeChild>>,
    _observation: &Option<()>,
) -> Result<(), String> {
    kill_and_reap(child).await
}

async fn wait_child_by_polling(child: &Arc<Mutex<RuntimeChild>>) -> Result<Option<i32>, String> {
    loop {
        match child.lock().await.try_wait_exit_code()? {
            Some(exit_code) => return Ok(exit_code),
            None => tokio::time::sleep(ABORT_POLL_INTERVAL).await,
        }
    }
}

async fn reap_child_after_kill(child: &Arc<Mutex<RuntimeChild>>) -> Result<Option<i32>, String> {
    let is_pty = {
        let child = child.lock().await;
        child.is_pty()
    };
    if is_pty {
        return wait_child_by_polling(child).await;
    }

    let mut child = child.lock().await;
    let RuntimeChild::Tokio(child) = &mut *child else {
        unreachable!();
    };
    let status = Box::into_pin(child.wait())
        .await
        .map_err(|error| format!("failed to wait for process: {error}"))?;
    Ok(status.code())
}

fn kill_reap_result(
    kill_result: Result<(), String>,
    wait_result: Result<Result<Option<i32>, String>, tokio::time::error::Elapsed>,
) -> Result<(), String> {
    match (kill_result, wait_result) {
        (Ok(()), Ok(Ok(_))) => Ok(()),
        (Err(kill_error), Ok(Ok(_))) => Err(format!("failed to kill process: {kill_error}")),
        (Ok(()), Ok(Err(wait_error))) => Err(format!("failed to reap process: {wait_error}")),
        (Err(kill_error), Ok(Err(wait_error))) => Err(format!(
            "failed to kill process: {kill_error}; failed to reap process: {wait_error}"
        )),
        (Ok(()), Err(_)) => Err("timed out while reaping process".to_string()),
        (Err(kill_error), Err(_)) => Err(format!(
            "failed to kill process: {kill_error}; timed out while reaping process"
        )),
    }
}

async fn kill_unregistered_child(mut child: Box<dyn TokioChildWrapper>) {
    let _ = child.start_kill();
    let _ = tokio::time::timeout(KILL_REAP_TIMEOUT, Box::into_pin(child.wait())).await;
}

async fn wait_child(
    child: &Arc<Mutex<RuntimeChild>>,
    #[cfg(target_os = "linux")] observation: &Option<ObservationHandle>,
    #[cfg(not(target_os = "linux"))] _observation: &Option<()>,
) -> Result<Option<i32>, String> {
    #[cfg(target_os = "linux")]
    if let Some(observation) = observation {
        return observation.wait_exit().await;
    }
    let is_pty = {
        let child = child.lock().await;
        child.is_pty()
    };
    if is_pty {
        return wait_child_by_polling(child).await;
    }

    let mut child = child.lock().await;
    let RuntimeChild::Tokio(child) = &mut *child else {
        unreachable!();
    };
    let status = child
        .inner_mut()
        .wait()
        .await
        .map_err(|error| format!("failed to wait for process: {error}"))?;
    Ok(status.code())
}

async fn try_wait_child(
    child: &Arc<Mutex<RuntimeChild>>,
    #[cfg(target_os = "linux")] observation: &Option<ObservationHandle>,
    #[cfg(not(target_os = "linux"))] _observation: &Option<()>,
) -> Result<Option<Option<i32>>, String> {
    #[cfg(target_os = "linux")]
    if let Some(observation) = observation {
        return observation.try_exit().transpose();
    }
    let mut child = child.lock().await;
    child.try_wait_exit_code()
}

async fn status_or_killed(
    child: &Arc<Mutex<RuntimeChild>>,
    #[cfg(target_os = "linux")] observation: &Option<ObservationHandle>,
    #[cfg(not(target_os = "linux"))] observation: &Option<()>,
) -> ExecStatus {
    match try_wait_child(child, observation).await {
        Ok(Some(exit_code)) => ExecStatus::Exited { exit_code },
        Ok(None) => ExecStatus::Killed,
        Err(message) => ExecStatus::Failed { message },
    }
}

async fn status_or_timed_out(
    child: &Arc<Mutex<RuntimeChild>>,
    #[cfg(target_os = "linux")] observation: &Option<ObservationHandle>,
    #[cfg(not(target_os = "linux"))] observation: &Option<()>,
) -> ExecStatus {
    match try_wait_child(child, observation).await {
        Ok(Some(exit_code)) => ExecStatus::Exited { exit_code },
        Ok(None) => ExecStatus::TimedOut,
        Err(message) => ExecStatus::Failed { message },
    }
}

async fn monitor_process(
    registry: ExecRegistry,
    process_id: ExecProcessId,
    child: Arc<Mutex<RuntimeChild>>,
    mut control_rx: mpsc::Receiver<ExecProcessCommand>,
    timeout: Option<Duration>,
    output_drain_timeout: Option<Duration>,
    abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    #[cfg(target_os = "linux")] observation: Option<ObservationHandle>,
    #[cfg(not(target_os = "linux"))] observation: Option<()>,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
) {
    let (terminal_status, finish_response) = loop {
        let abort_wait = async {
            loop {
                tokio::time::sleep(ABORT_POLL_INTERVAL).await;
                if abort_flag
                    .as_ref()
                    .map(|flag| flag.load(Ordering::Relaxed))
                    .unwrap_or(false)
                {
                    break;
                }
            }
        };

        match timeout {
            Some(timeout) => {
                tokio::select! {
                    result = wait_child(&child, &observation) => {
                        break (
                            match result {
                                Ok(exit_code) => ExecStatus::Exited { exit_code },
                                Err(message) => ExecStatus::Failed { message },
                            },
                            None,
                        );
                    }
                    _ = tokio::time::sleep(timeout) => {
                        break (status_or_timed_out(&child, &observation).await, None);
                    }
                    _ = abort_wait => {
                        break (status_or_killed(&child, &observation).await, None);
                    }
                    command = control_rx.recv() => {
                        match command {
                            Some(ExecProcessCommand::Kill { response }) => {
                                let status = status_or_killed(&child, &observation).await;
                                break (status, Some(response));
                            }
                            Some(ExecProcessCommand::Finish { status, response }) => {
                                break (status, Some(response));
                            }
                            Some(ExecProcessCommand::Resize { rows, cols, response }) => {
                                let result = child.lock().await.resize(rows, cols);
                                let _ = response.send(result);
                                continue;
                            }
                            None => {
                                let status = status_or_killed(&child, &observation).await;
                                break (status, None);
                            }
                        }
                    }
                }
            }
            None => {
                tokio::select! {
                    result = wait_child(&child, &observation) => {
                        break (
                            match result {
                                Ok(exit_code) => ExecStatus::Exited { exit_code },
                                Err(message) => ExecStatus::Failed { message },
                            },
                            None,
                        );
                    }
                    _ = abort_wait => {
                        break (status_or_killed(&child, &observation).await, None);
                    }
                    command = control_rx.recv() => {
                        match command {
                            Some(ExecProcessCommand::Kill { response }) => {
                                let status = status_or_killed(&child, &observation).await;
                                break (status, Some(response));
                            }
                            Some(ExecProcessCommand::Finish { status, response }) => {
                                break (status, Some(response));
                            }
                            Some(ExecProcessCommand::Resize { rows, cols, response }) => {
                                let result = child.lock().await.resize(rows, cols);
                                let _ = response.send(result);
                                continue;
                            }
                            None => {
                                let status = status_or_killed(&child, &observation).await;
                                break (status, None);
                            }
                        }
                    }
                }
            }
        }
    };

    let terminal_status = match terminal_status {
        ExecStatus::Failed { .. } | ExecStatus::TimedOut | ExecStatus::Killed => {
            if let Err(error) = kill_and_reap_observed(&child, &observation).await {
                tracing::warn!("exec kill/reap failed for {process_id}: {error}");
            }
            finish_pumps_with_timeout(stdout_task, stderr_task, KILL_PUMP_DRAIN_TIMEOUT).await;
            terminal_status
        }
        ExecStatus::Exited { .. } => {
            #[cfg(target_os = "linux")]
            let observation_ended_early = observation.as_ref().is_some_and(|observation| {
                matches!(observation.status(), ObservationStatus::Incomplete(_))
            });
            #[cfg(not(target_os = "linux"))]
            let observation_ended_early = false;
            if observation_ended_early {
                stdout_task.abort();
                stderr_task.abort();
                terminal_status
            } else {
                let drain_timeout = output_drain_timeout.unwrap_or(EXIT_PUMP_DRAIN_TIMEOUT);
                if finish_pumps_with_timeout(stdout_task, stderr_task, drain_timeout).await {
                    terminal_status
                } else {
                    if let Err(error) = kill_and_reap_observed(&child, &observation).await {
                        tracing::warn!("exec kill/reap after output drain timeout failed for {process_id}: {error}");
                    }
                    pump_drain_timeout_status(drain_timeout)
                }
            }
        }
        ExecStatus::Starting | ExecStatus::Running => terminal_status,
    };
    let final_snapshot = registry.complete_status(&process_id, terminal_status).await;
    if let Some(response) = finish_response {
        let _ = response.send(final_snapshot);
    }
}

async fn wait_for_readiness(
    registry: &ExecRegistry,
    process_id: &ExecProcessId,
    readiness: &ExecReadinessProbe,
    startup_wait: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        if let Some(snapshot) = registry.get(process_id).await {
            if snapshot.status.is_terminal() {
                return Err(format!(
                    "process exited before startup readiness: {:?}",
                    snapshot.status
                ));
            }
        } else {
            return Err(format!(
                "process disappeared before startup readiness: {process_id}"
            ));
        }
        let read = registry.read(process_id, 0, None).await;
        if let Some(keyword) = readiness.wait_keyword.as_ref() {
            if read.chunks.iter().any(|chunk| chunk.text.contains(keyword)) {
                return Ok(());
            }
        }
        if let Some(port) = readiness.wait_port {
            if is_someone_listening_on_that_tcp_port(port, READINESS_PORT_CONNECT_TIMEOUT).await {
                return Ok(());
            }
        }
        if started.elapsed() >= startup_wait {
            return Err(format!(
                "startup readiness timed out after {:.3}s",
                startup_wait.as_secs_f64()
            ));
        }
        tokio::time::sleep(READINESS_POLL_INTERVAL).await;
    }
}

impl ExecRegistry {
    pub async fn spawn(&self, request: ExecSpawnRequest) -> Result<ExecSpawnResult, String> {
        if request.tty {
            return self.spawn_pty(request).await;
        }

        let mut command = shell_command(&request)?;
        #[cfg(target_os = "linux")]
        let observation_setup = if request.observe {
            if request.sandbox.is_some() && refact_sandbox::sandbox_status().provider == "bwrap" {
                Setup::unavailable("bwrap observation is unavailable")
            } else {
                Setup::prepare(&mut command)
            }
        } else {
            Setup::disabled()
        };
        let mut command = wrap_command(command);
        let (meta, process_id) = build_process_meta(&request)?;
        let startup_wait = request.startup_wait;
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => return Err(format!("failed to spawn command: {error}")),
        };
        let stdout = match child.stdout().take() {
            Some(stdout) => stdout,
            None => {
                kill_unregistered_child(child).await;
                return Err("failed to capture stdout".to_string());
            }
        };
        let stderr = match child.stderr().take() {
            Some(stderr) => stderr,
            None => {
                kill_unregistered_child(child).await;
                return Err("failed to capture stderr".to_string());
            }
        };
        #[cfg(target_os = "linux")]
        let observation = observation_setup.start(child.id());
        #[cfg(target_os = "linux")]
        let observation_handle = observation.handle();
        #[cfg(target_os = "linux")]
        let observation_reader = request.observe.then(|| observation.reader());
        #[cfg(not(target_os = "linux"))]
        let observation_handle = None;
        #[cfg(not(target_os = "linux"))]
        let observation_reader = request.observe.then(|| {
            let ObservationStatus::Unavailable(reason) = crate::observe::status(true) else {
                unreachable!("unsupported observer returned observed access")
            };
            crate::observe::ObservationReader::unavailable(reason)
        });
        let child = Arc::new(Mutex::new(RuntimeChild::Tokio(child)));
        let (control_tx, control_rx) = mpsc::channel(8);
        let terminal = Arc::new(Notify::new());
        if let Err(message) = self
            .register_new_with_runtime(
                meta,
                request.output_limits.transcript_max_bytes,
                ExecProcessRuntime {
                    control_tx,
                    terminal,
                    stdin_writer: None,
                },
                matches!(request.mode, ExecMode::Foreground),
            )
            .await
        {
            if let Err(cleanup_error) = kill_and_reap_observed(&child, &observation_handle).await {
                return Err(format!(
                    "{message}; additionally failed to cleanup spawned child: {cleanup_error}"
                ));
            }
            return Err(message);
        }
        if let Some(observation_reader) = observation_reader {
            self.set_observation_reader(&process_id, observation_reader)
                .await?;
        }
        let stdout_task = pump_output(
            self.clone(),
            process_id.clone(),
            ExecOutputStream::Stdout,
            stdout,
            request.output_progress_tx.clone(),
        );
        let stderr_task = pump_output(
            self.clone(),
            process_id.clone(),
            ExecOutputStream::Stderr,
            stderr,
            request.output_progress_tx.clone(),
        );
        tokio::spawn(monitor_process(
            self.clone(),
            process_id.clone(),
            child,
            control_rx,
            request.timeout,
            request.output_drain_timeout,
            request.abort_flag.clone(),
            observation_handle,
            stdout_task,
            stderr_task,
        ));
        let snapshot = self.mark_started(&process_id).await?;
        if matches!(request.mode, ExecMode::Foreground) {
            #[cfg(target_os = "linux")]
            let observation = observation.finish(true).await;
            #[cfg(not(target_os = "linux"))]
            let observation = crate::observe::status(request.observe);
            return Ok(ExecSpawnResult::new(
                self.wait(&process_id).await?,
                observation,
            ));
        }
        #[cfg(target_os = "linux")]
        let observation = observation.finish(false).await;
        #[cfg(not(target_os = "linux"))]
        let observation = crate::observe::status(request.observe);
        if let Some(readiness) = request.readiness.as_ref() {
            let startup_wait = startup_wait.unwrap_or(Duration::from_secs(10));
            if let Err(message) =
                wait_for_readiness(self, &process_id, readiness, startup_wait).await
            {
                if let Ok(snapshot) = self
                    .finish_with_status(
                        &process_id,
                        ExecStatus::Failed {
                            message: message.clone(),
                        },
                    )
                    .await
                {
                    return Ok(ExecSpawnResult::new(snapshot, observation));
                }
                let snapshot = self
                    .mark_failed(&process_id, message)
                    .await
                    .unwrap_or_else(|_| snapshot.clone());
                return Ok(ExecSpawnResult::new(snapshot, observation));
            }
        } else if let Some(startup_wait) = startup_wait {
            tokio::time::sleep(startup_wait).await;
        }
        Ok(ExecSpawnResult::new(
            self.get(&process_id).await.unwrap_or(snapshot),
            observation,
        ))
    }

    async fn spawn_pty(&self, request: ExecSpawnRequest) -> Result<ExecSpawnResult, String> {
        let command = pty_command(&request)?;
        let (meta, process_id) = build_process_meta(&request)?;
        let startup_wait = request.startup_wait;
        let (pty_handle, child) =
            crate::pty::spawn_pty(command, crate::pty::pty_size(request.rows, request.cols))?;
        let child_process_id = child.process_id();
        let stdin_writer = Arc::new(Mutex::new(pty_handle.writer));
        let child = Arc::new(Mutex::new(RuntimeChild::Pty(PtyRuntimeProcess {
            child,
            process_id: child_process_id,
            writer: stdin_writer.clone(),
            master: pty_handle.master,
        })));
        let (control_tx, control_rx) = mpsc::channel(8);
        let terminal = Arc::new(Notify::new());
        if let Err(message) = self
            .register_new_with_runtime(
                meta,
                request.output_limits.transcript_max_bytes,
                ExecProcessRuntime {
                    control_tx,
                    terminal,
                    stdin_writer: Some(stdin_writer),
                },
                matches!(request.mode, ExecMode::Foreground),
            )
            .await
        {
            if let Err(cleanup_error) = kill_and_reap(&child).await {
                return Err(format!(
                    "{message}; additionally failed to cleanup spawned child: {cleanup_error}"
                ));
            }
            return Err(message);
        }
        if request.observe {
            let ObservationStatus::Unavailable(reason) = crate::observe::unsupported_status(true)
            else {
                unreachable!("unsupported observer returned observed access")
            };
            self.set_observation_reader(
                &process_id,
                crate::observe::ObservationReader::unavailable(reason),
            )
            .await?;
        }
        let stdout_task = pump_blocking_output(
            self.clone(),
            process_id.clone(),
            ExecOutputStream::Combined,
            pty_handle.reader,
            request.output_progress_tx.clone(),
        );
        let stderr_task = tokio::spawn(async {});
        tokio::spawn(monitor_process(
            self.clone(),
            process_id.clone(),
            child,
            control_rx,
            request.timeout,
            request.output_drain_timeout,
            request.abort_flag.clone(),
            None,
            stdout_task,
            stderr_task,
        ));
        let snapshot = self.mark_started(&process_id).await?;
        if matches!(request.mode, ExecMode::Foreground) {
            return Ok(ExecSpawnResult::new(
                self.wait(&process_id).await?,
                crate::observe::unsupported_status(request.observe),
            ));
        }
        if let Some(readiness) = request.readiness.as_ref() {
            let startup_wait = startup_wait.unwrap_or(Duration::from_secs(10));
            if let Err(message) =
                wait_for_readiness(self, &process_id, readiness, startup_wait).await
            {
                if let Ok(snapshot) = self
                    .finish_with_status(
                        &process_id,
                        ExecStatus::Failed {
                            message: message.clone(),
                        },
                    )
                    .await
                {
                    return Ok(ExecSpawnResult::new(
                        snapshot,
                        crate::observe::unsupported_status(request.observe),
                    ));
                }
                let snapshot = self
                    .mark_failed(&process_id, message)
                    .await
                    .unwrap_or_else(|_| snapshot.clone());
                return Ok(ExecSpawnResult::new(
                    snapshot,
                    crate::observe::unsupported_status(request.observe),
                ));
            }
        } else if let Some(startup_wait) = startup_wait {
            tokio::time::sleep(startup_wait).await;
        }
        Ok(ExecSpawnResult::new(
            self.get(&process_id).await.unwrap_or(snapshot),
            crate::observe::unsupported_status(request.observe),
        ))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use super::*;
    use refact_sandbox::Enforcement;
    #[cfg(unix)]
    use crate::types::ExecEnvPolicy;
    use crate::types::{ExecProcessFilter, ExecSandboxMode, ExecSandboxSpec, ExecStatusKind};

    #[cfg(unix)]
    struct ParentEnvGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    #[cfg(unix)]
    impl ParentEnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    #[cfg(unix)]
    impl Drop for ParentEnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[cfg(windows)]
    fn shell_script(script: &str) -> String {
        script.to_string()
    }

    #[cfg(not(windows))]
    fn shell_script(script: &str) -> String {
        script.to_string()
    }

    #[cfg(unix)]
    fn inherited_pipe_command() -> String {
        shell_script("sleep 60 & printf 'parent-done'; exit 0")
    }

    async fn assert_process_missing(process_id: u32) {
        for _ in 0..20 {
            if !process_exists(process_id) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!process_exists(process_id));
    }

    async fn wait_for_registry_output(
        registry: &ExecRegistry,
        process_id: &ExecProcessId,
        needle: &str,
    ) {
        let timeout = if cfg!(windows) {
            Duration::from_secs(15)
        } else {
            Duration::from_secs(2)
        };
        let started = Instant::now();
        while started.elapsed() < timeout {
            let read = registry.read(process_id, 0, None).await;
            if read.chunks.iter().any(|chunk| chunk.text.contains(needle)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let read = registry.read(process_id, 0, None).await;
        panic!(
            "process output did not contain {needle:?}: {:?}",
            read.chunks
        );
    }

    #[cfg(target_os = "linux")]
    fn processes_with_env_marker(marker: &str) -> Vec<u32> {
        let expected = format!("{marker}=1").into_bytes();
        let mut process_ids = Vec::new();
        for entry in std::fs::read_dir("/proc").unwrap().flatten() {
            let Some(process_id) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(environ) = std::fs::read(entry.path().join("environ")) else {
                continue;
            };
            if environ
                .split(|byte| *byte == 0)
                .any(|variable| variable == expected)
            {
                process_ids.push(process_id);
            }
        }
        process_ids
    }

    #[cfg(target_os = "linux")]
    async fn wait_for_marked_processes(marker: &str, minimum: usize) -> Vec<u32> {
        for _ in 0..100 {
            let process_ids = processes_with_env_marker(marker);
            if process_ids.len() >= minimum {
                return process_ids;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        processes_with_env_marker(marker)
    }

    async fn spawn_and_read_stdout(request: ExecSpawnRequest) -> String {
        let registry = ExecRegistry::new();
        let result = registry.spawn(request).await.unwrap();
        let snapshot = if result.snapshot.status.is_terminal() {
            result.snapshot
        } else {
            registry
                .wait(&result.snapshot.meta.process_id)
                .await
                .unwrap()
        };
        assert_eq!(snapshot.status, ExecStatus::Exited { exit_code: Some(0) });
        let read = registry.read(&snapshot.meta.process_id, 0, None).await;
        read.chunks
            .iter()
            .filter(|chunk| chunk.stream == ExecOutputStream::Stdout)
            .map(|chunk| chunk.text.as_str())
            .collect::<String>()
    }

    fn workspace_write_sandbox(cwd: &std::path::Path) -> ExecSandboxSpec {
        ExecSandboxSpec {
            mode: ExecSandboxMode::WorkspaceWrite,
            ro_paths: Vec::new(),
            rw_paths: vec![cwd.to_path_buf()],
            allow_network: true,
        }
    }

    #[cfg(unix)]
    async fn spawn_and_read_all(request: ExecSpawnRequest) -> String {
        let registry = ExecRegistry::new();
        let result = registry.spawn(request).await.unwrap();
        let snapshot = if result.snapshot.status.is_terminal() {
            result.snapshot
        } else {
            registry
                .wait(&result.snapshot.meta.process_id)
                .await
                .unwrap()
        };
        assert_eq!(snapshot.status, ExecStatus::Exited { exit_code: Some(0) });
        let read = registry.read(&snapshot.meta.process_id, 0, None).await;
        read.chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<String>()
    }

    #[test]
    fn utf8_chunk_decoder_buffers_partial_trailing_codepoint() {
        let mut decoder = Utf8ChunkDecoder::default();
        let bytes = "aβc".as_bytes();
        let first = decoder.decode(&bytes[..2]);
        let second = decoder.decode(&bytes[2..]);
        assert_eq!(first, "a");
        assert_eq!(second, "βc");
        assert!(decoder.finish().is_none());
    }

    #[test]
    fn utf8_chunk_decoder_flushes_incomplete_tail_lossy_on_finish() {
        let mut decoder = Utf8ChunkDecoder::default();
        let text = decoder.decode(&[b'a', 0xE2]);
        assert_eq!(text, "a");
        assert_eq!(decoder.finish(), Some("\u{FFFD}".to_string()));
    }

    #[test]
    fn utf8_chunk_decoder_replaces_invalid_bytes_in_the_middle() {
        let mut decoder = Utf8ChunkDecoder::default();
        let text = decoder.decode(&[b'a', 0xFF, b'b']);
        assert_eq!(text, "a\u{FFFD}b");
        assert!(decoder.finish().is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn tty_raw_capture_preserves_crlf_while_transcript_is_normalized() {
        let registry = ExecRegistry::new();
        let result = registry
            .spawn(ExecSpawnRequest::foreground("printf 'a\\r\\nb\\r\\n'").with_tty(true))
            .await
            .unwrap();
        let process_id = result.snapshot.meta.process_id;

        let raw = registry.read_raw_since(&process_id, 0, None).await.unwrap();
        assert!(
            raw.text.contains("\r\n"),
            "raw capture must keep CRLF byte-faithful: {:?}",
            raw.text
        );
        assert!(
            raw.text.contains('a') && raw.text.contains('b'),
            "raw capture must contain the printed lines: {:?}",
            raw.text
        );
        assert_eq!(raw.new_offset, raw.text.len() as u64);

        let read = registry.read(&process_id, 0, None).await;
        let transcript_text = read
            .chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<String>();
        assert!(
            !transcript_text.contains('\r'),
            "transcript stays normalized: {transcript_text:?}"
        );
        assert!(transcript_text.contains('a') && transcript_text.contains('b'));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn tty_process_can_be_resized() {
        let registry = ExecRegistry::new();
        let result = registry
            .spawn(
                ExecSpawnRequest::interactive("cat")
                    .with_tty(true)
                    .with_pty_size(24, 80),
            )
            .await
            .unwrap();
        let process_id = result.snapshot.meta.process_id;

        registry.resize(&process_id, 40, 120).await.unwrap();

        registry.kill(&process_id).await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn tty_kill_terminates_background_descendants() {
        let registry = ExecRegistry::new();
        let marker = format!("REFACT_TEST_MARKER_{}", uuid::Uuid::new_v4().simple());
        let result = registry
            .spawn(
                ExecSpawnRequest::background("bash -c 'sleep 300 & sleep 300 & wait'")
                    .with_tty(true)
                    .with_env(&marker, "1"),
            )
            .await
            .unwrap();
        let process_id = result.snapshot.meta.process_id;
        let spawned_processes = wait_for_marked_processes(&marker, 3).await;
        let kill_result = registry.kill(&process_id).await;
        assert!(
            spawned_processes.len() >= 3,
            "expected PTY leader and background descendants, found {spawned_processes:?}"
        );
        let snapshot = kill_result.unwrap();
        assert_eq!(snapshot.status, ExecStatus::Killed);

        for _ in 0..40 {
            if processes_with_env_marker(&marker).is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let survivors = processes_with_env_marker(&marker);
        assert!(
            survivors.is_empty(),
            "PTY descendants survived registry kill: {survivors:?}"
        );
    }

    fn env_test_request(mode: ExecMode, command: &str) -> ExecSpawnRequest {
        let is_service = matches!(mode, ExecMode::Service);
        let request = ExecSpawnRequest::new(mode, shell_script(command));
        if is_service {
            request.with_owner(crate::types::ExecOwnerMeta {
                service_name: Some("env-default-test".to_string()),
                ..crate::types::ExecOwnerMeta::default()
            })
        } else {
            request
        }
    }

    #[cfg(unix)]
    fn process_exists(process_id: u32) -> bool {
        unsafe { libc::kill(process_id as i32, 0) == 0 }
    }

    #[cfg(windows)]
    fn process_exists(process_id: u32) -> bool {
        std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "if (Get-Process -Id {process_id} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
                ),
            ])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn tty_false_unchanged() {
        let registry = ExecRegistry::new();
        let command = if cfg!(windows) {
            "[Console]::Out.Write('hi')"
        } else {
            "printf hi"
        };
        let result = registry
            .spawn(ExecSpawnRequest::foreground(shell_script(command)).with_tty(false))
            .await
            .unwrap();

        assert_eq!(
            result.snapshot.status,
            ExecStatus::Exited { exit_code: Some(0) }
        );
        let read = registry
            .read(&result.snapshot.meta.process_id, 0, None)
            .await;
        assert_eq!(read.chunks.len(), 1);
        assert_eq!(read.chunks[0].stream, ExecOutputStream::Stdout);
        assert_eq!(read.chunks[0].text, "hi");
    }

    #[tokio::test]
    async fn foreground_success_captures_stdout() {
        let registry = ExecRegistry::new();
        let command = if cfg!(windows) {
            "[Console]::Out.Write('hello')"
        } else {
            "printf hello"
        };
        let result = registry
            .spawn(ExecSpawnRequest::foreground(shell_script(command)))
            .await
            .unwrap();

        assert_eq!(
            result.snapshot.status,
            ExecStatus::Exited { exit_code: Some(0) }
        );
        let read = registry
            .read(&result.snapshot.meta.process_id, 0, None)
            .await;
        assert_eq!(read.chunks.len(), 1);
        assert_eq!(read.chunks[0].stream, ExecOutputStream::Stdout);
        assert_eq!(read.chunks[0].text, "hello");
    }

    #[tokio::test]
    async fn observe_request_uses_platform_backend_for_pipe_and_fallback_for_pty() {
        let command = if cfg!(windows) {
            "[Console]::Out.Write('observed')"
        } else {
            "printf observed"
        };

        for tty in [false, true] {
            let result = ExecRegistry::new()
                .spawn(
                    ExecSpawnRequest::foreground(shell_script(command))
                        .with_observe(true)
                        .with_tty(tty),
                )
                .await
                .unwrap();

            if cfg!(target_os = "linux") && !tty {
                assert!(matches!(result.observation, ObservationStatus::Observed(_)));
            } else {
                let ObservationStatus::Unavailable(reason) = result.observation else {
                    panic!("unsupported observer returned observed access");
                };
                assert_eq!(reason, "backend unavailable");
            }
        }
    }

    #[tokio::test]
    async fn exec_env_defaults_apply() {
        let command = if cfg!(windows) {
            "[Console]::Out.Write(\"$env:NO_COLOR $env:TERM $env:PAGER $env:REFACT_EXEC\")"
        } else {
            "printf '%s %s %s %s' \"$NO_COLOR\" \"$TERM\" \"$PAGER\" \"$REFACT_EXEC\""
        };

        for mode in [
            ExecMode::Foreground,
            ExecMode::Background,
            ExecMode::Service,
        ] {
            let stdout = spawn_and_read_stdout(env_test_request(mode, command)).await;
            assert_eq!(stdout, "1 dumb cat 1");
        }
    }

    #[tokio::test]
    async fn exec_env_request_overrides_defaults() {
        let command = if cfg!(windows) {
            "[Console]::Out.Write($env:TERM)"
        } else {
            "printf '%s' \"$TERM\""
        };

        let stdout = spawn_and_read_stdout(
            ExecSpawnRequest::foreground(shell_script(command)).with_env("TERM", "xterm-256color"),
        )
        .await;

        assert_eq!(stdout, "xterm-256color");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn argv_exec_preserves_spaces_and_quotes_without_shell() {
        let registry = ExecRegistry::new();
        let script = "printf '%s\\n' \"$#\" \"$1\" \"$2\" \"$3\"";
        let result = registry
            .spawn(ExecSpawnRequest::argv(
                ExecMode::Foreground,
                vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    script.to_string(),
                    "argv-test".to_string(),
                    "space value".to_string(),
                    "single'quote".to_string(),
                    "double\"quote".to_string(),
                ],
            ))
            .await
            .unwrap();

        let read = registry
            .read(&result.snapshot.meta.process_id, 0, None)
            .await;
        let output = read
            .chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<String>();

        assert_eq!(output, "3\nspace value\nsingle'quote\ndouble\"quote\n");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn sandbox_wraps_pipe_and_pty_spawn_paths() {
        if refact_sandbox::sandbox_status().enforcement == Enforcement::Unusable {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        let sandbox = workspace_write_sandbox(workspace.path());

        for tty in [false, true] {
            let output = spawn_and_read_all(
                ExecSpawnRequest::foreground("printf sandboxed")
                    .with_cwd(workspace.path())
                    .with_sandbox(sandbox.clone())
                    .with_tty(tty),
            )
            .await;

            assert!(output.contains("sandboxed"), "{output:?}");
        }
    }

    #[tokio::test]
    async fn sandbox_request_fails_closed_when_provider_is_unusable() {
        if refact_sandbox::sandbox_status().enforcement != Enforcement::Unusable {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        let error = match ExecRegistry::new()
            .spawn(
                ExecSpawnRequest::foreground("echo must-not-run")
                    .with_cwd(workspace.path())
                    .with_sandbox(workspace_write_sandbox(workspace.path())),
            )
            .await
        {
            Ok(_) => panic!("unusable sandbox provider must fail closed"),
            Err(error) => error,
        };

        assert!(error.starts_with("sandbox: noop:"), "{error}");
    }

    #[tokio::test]
    async fn exec_env_marker_always_present() {
        let command = if cfg!(windows) {
            "[Console]::Out.Write($env:REFACT_EXEC)"
        } else {
            "printf '%s' \"$REFACT_EXEC\""
        };

        let default_stdout =
            spawn_and_read_stdout(ExecSpawnRequest::foreground(shell_script(command))).await;
        let override_stdout = spawn_and_read_stdout(
            ExecSpawnRequest::foreground(shell_script(command)).with_env("REFACT_EXEC", ""),
        )
        .await;

        assert_eq!(default_stdout, "1");
        assert_eq!(override_stdout, "");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn scrubbed_pipe_and_pty_envs_exclude_parent_values_and_keep_request_defaults() {
        let _secret = ParentEnvGuard::set("SECRET_TEST_API_KEY", "secret-value");
        let _unrelated = ParentEnvGuard::set("TOTALLY_RANDOM_VAR", "random-value");
        let expected_path = std::env::var("PATH").unwrap();
        let expected_home = std::env::var("HOME").unwrap();

        for tty in [false, true] {
            let output = spawn_and_read_all(
                ExecSpawnRequest::foreground("env")
                    .with_tty(tty)
                    .with_env("FOO", "bar"),
            )
            .await;

            assert!(!output.contains("SECRET_TEST_API_KEY="), "{output}");
            assert!(!output.contains("TOTALLY_RANDOM_VAR="), "{output}");
            assert!(output.contains("FOO=bar"), "{output}");
            assert!(output.contains("NO_COLOR=1"), "{output}");
            assert!(output.contains("TERM=dumb"), "{output}");
            assert!(output.contains("LANG=C.UTF-8"), "{output}");
            assert!(output.contains("REFACT_EXEC=1"), "{output}");
            assert!(
                output.contains(&format!("PATH={expected_path}")),
                "{output}"
            );
            assert!(
                output.contains(&format!("HOME={expected_home}")),
                "{output}"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inherit_policy_exposes_parent_env() {
        let _inherited = ParentEnvGuard::set("REFACT_TEST_INHERITED_VAR", "inherited-value");
        let output = spawn_and_read_all(
            ExecSpawnRequest::foreground("env").with_env_policy(ExecEnvPolicy::Inherit),
        )
        .await;

        assert!(
            output.contains("REFACT_TEST_INHERITED_VAR=inherited-value"),
            "{output}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn scrubbed_passthrough_glob_keeps_non_secrets_only() {
        let _visible = ParentEnvGuard::set("REFACT_TEST_PASSTHROUGH_VISIBLE", "visible-value");
        let _secret = ParentEnvGuard::set("REFACT_TEST_PASSTHROUGH_API_KEY", "secret-value");
        let output = spawn_and_read_all(ExecSpawnRequest::foreground("env").with_env_policy(
            ExecEnvPolicy::Scrubbed {
                passthrough: vec!["REFACT_TEST_PASSTHROUGH_*".to_string()],
            },
        ))
        .await;

        assert!(
            output.contains("REFACT_TEST_PASSTHROUGH_VISIBLE=visible-value"),
            "{output}"
        );
        assert!(
            !output.contains("REFACT_TEST_PASSTHROUGH_API_KEY="),
            "{output}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn request_env_can_explicitly_supply_secret_name() {
        let output = spawn_and_read_all(
            ExecSpawnRequest::foreground("env")
                .with_env("REFACT_TEST_REQUEST_API_KEY", "request-value"),
        )
        .await;

        assert!(
            output.contains("REFACT_TEST_REQUEST_API_KEY=request-value"),
            "{output}"
        );
    }

    #[tokio::test]
    async fn foreground_captures_stderr() {
        let registry = ExecRegistry::new();
        let command = if cfg!(windows) {
            "[Console]::Error.Write('warn')"
        } else {
            "printf warn >&2"
        };
        let result = registry
            .spawn(ExecSpawnRequest::foreground(shell_script(command)))
            .await
            .unwrap();

        assert_eq!(
            result.snapshot.status,
            ExecStatus::Exited { exit_code: Some(0) }
        );
        let read = registry
            .read(&result.snapshot.meta.process_id, 0, None)
            .await;
        assert_eq!(read.chunks.len(), 1);
        assert_eq!(read.chunks[0].stream, ExecOutputStream::Stderr);
        assert_eq!(read.chunks[0].text, "warn");
    }

    #[tokio::test]
    async fn foreground_reports_non_zero_exit_code() {
        let registry = ExecRegistry::new();
        let command = if cfg!(windows) { "exit 7" } else { "exit 7" };
        let result = registry
            .spawn(ExecSpawnRequest::foreground(shell_script(command)))
            .await
            .unwrap();

        assert_eq!(
            result.snapshot.status,
            ExecStatus::Exited { exit_code: Some(7) }
        );
    }

    #[tokio::test]
    async fn timeout_kills_and_keeps_partial_output() {
        let registry = ExecRegistry::new();
        let command = if cfg!(windows) {
            "[Console]::Out.WriteLine('start'); Start-Sleep -Seconds 5"
        } else {
            "printf start; sleep 5"
        };
        let timeout = if cfg!(windows) {
            Duration::from_secs(4)
        } else {
            Duration::from_millis(200)
        };
        let result = registry
            .spawn(ExecSpawnRequest::foreground(shell_script(command)).with_timeout(timeout))
            .await
            .unwrap();

        assert_eq!(result.snapshot.status, ExecStatus::TimedOut);
        let read = registry
            .read(&result.snapshot.meta.process_id, 0, None)
            .await;
        if cfg!(windows) {
            assert!(
                read.chunks.is_empty()
                    || read.chunks.iter().any(|chunk| chunk.text.contains("start"))
            );
        } else {
            assert!(read.chunks.iter().any(|chunk| chunk.text.contains("start")));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn foreground_fails_promptly_when_descendant_keeps_pipe_open() {
        let registry = ExecRegistry::new();
        let started = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            registry.spawn(ExecSpawnRequest::foreground(inherited_pipe_command())),
        )
        .await
        .expect("foreground spawn must not hang when descendant holds pipe open")
        .unwrap();

        assert!(started.elapsed() < Duration::from_secs(3));
        match &result.snapshot.status {
            ExecStatus::Failed { message } => {
                assert!(message.contains("output drain timed out"));
                assert!(message.contains("stdout/stderr"));
            }
            status => panic!("expected drain timeout failure, got {status:?}"),
        }
        let read = registry
            .read(&result.snapshot.meta.process_id, 0, None)
            .await;
        assert!(read
            .chunks
            .iter()
            .any(|chunk| chunk.text.contains("parent-done")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn background_fails_when_descendant_keeps_pipe_open() {
        let registry = ExecRegistry::new();
        let result = registry
            .spawn(ExecSpawnRequest::background(inherited_pipe_command()))
            .await
            .unwrap();
        assert_eq!(result.snapshot.status, ExecStatus::Running);

        let snapshot = tokio::time::timeout(
            Duration::from_secs(5),
            registry.wait(&result.snapshot.meta.process_id),
        )
        .await
        .expect("background process must become terminal when descendant holds pipe open")
        .unwrap();

        match &snapshot.status {
            ExecStatus::Failed { message } => {
                assert!(message.contains("output drain timed out"));
                assert!(message.contains("stdout/stderr"));
            }
            status => panic!("expected drain timeout failure, got {status:?}"),
        }
        let listed = registry
            .list(ExecProcessFilter {
                status: Some(ExecStatusKind::Running),
                ..ExecProcessFilter::default()
            })
            .await;
        assert!(listed.is_empty());
    }

    #[tokio::test]
    async fn abort_flag_kills_and_keeps_partial_output() {
        let registry = ExecRegistry::new();
        let abort_flag = Arc::new(AtomicBool::new(false));
        let command = if cfg!(windows) {
            "[Console]::Out.WriteLine('start'); Start-Sleep -Seconds 5"
        } else {
            "printf start; sleep 5"
        };
        let request = ExecSpawnRequest::foreground(shell_script(command))
            .with_abort_flag(abort_flag.clone())
            .with_timeout(Duration::from_secs(10));
        let abort_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            abort_flag.store(true, Ordering::Relaxed);
        });
        let result = registry.spawn(request).await.unwrap();
        abort_task.await.unwrap();

        assert_eq!(result.snapshot.status, ExecStatus::Killed);
        let read = registry
            .read(&result.snapshot.meta.process_id, 0, None)
            .await;
        if cfg!(windows) {
            assert!(
                read.chunks.is_empty()
                    || read.chunks.iter().any(|chunk| chunk.text.contains("start"))
            );
        } else {
            assert!(read.chunks.iter().any(|chunk| chunk.text.contains("start")));
        }
    }

    #[tokio::test]
    async fn large_output_is_bounded() {
        let registry = ExecRegistry::new();
        let command = if cfg!(windows) {
            "[Console]::Out.Write(('x' * 4096))"
        } else {
            "chunk=x; i=0; while [ $i -lt 12 ]; do chunk=\"$chunk$chunk\"; i=$((i + 1)); done; printf '%s' \"$chunk\""
        };
        let result = registry
            .spawn(ExecSpawnRequest::foreground(shell_script(command)).with_transcript_limit(1024))
            .await
            .unwrap();

        assert_eq!(
            result.snapshot.status,
            ExecStatus::Exited { exit_code: Some(0) }
        );
        let read = registry
            .read(&result.snapshot.meta.process_id, 0, None)
            .await;
        assert!(read.current_bytes <= 1024);
        assert!(read.is_truncated);
    }

    #[tokio::test]
    async fn background_can_be_listed_read_and_killed() {
        let registry = ExecRegistry::new();
        let command = if cfg!(windows) {
            "[Console]::Out.WriteLine('ready'); Start-Sleep -Seconds 5"
        } else {
            "printf ready; sleep 5"
        };
        let result = registry
            .spawn(ExecSpawnRequest::background(shell_script(command)))
            .await
            .unwrap();
        assert_eq!(result.snapshot.status, ExecStatus::Running);

        let listed = registry
            .list(ExecProcessFilter {
                status: Some(ExecStatusKind::Running),
                ..ExecProcessFilter::default()
            })
            .await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].meta.process_id, result.snapshot.meta.process_id);

        wait_for_registry_output(&registry, &result.snapshot.meta.process_id, "ready").await;

        let killed = registry
            .kill(&result.snapshot.meta.process_id)
            .await
            .unwrap();
        assert_eq!(killed.status, ExecStatus::Killed);
        let waited = registry
            .wait(&result.snapshot.meta.process_id)
            .await
            .unwrap();
        assert_eq!(waited.status, ExecStatus::Killed);
    }

    #[tokio::test]
    async fn closed_channel_does_not_spin() {
        let registry = ExecRegistry::new();
        let command = if cfg!(windows) {
            "[Console]::Out.WriteLine('ready'); Start-Sleep -Seconds 30"
        } else {
            "printf ready; sleep 30"
        };
        let result = registry
            .spawn(ExecSpawnRequest::background(shell_script(command)))
            .await
            .unwrap();
        let process_id = result.snapshot.meta.process_id.clone();
        let (replacement_tx, _replacement_rx) = mpsc::channel(1);
        registry
            .attach_runtime(
                &process_id,
                ExecProcessRuntime {
                    control_tx: replacement_tx,
                    terminal: Arc::new(Notify::new()),
                    stdin_writer: None,
                },
            )
            .await
            .unwrap();

        let snapshot = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = registry.get(&process_id).await.unwrap();
                if snapshot.status.is_terminal() {
                    return snapshot;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("monitor should finish after control channel closes");

        assert_eq!(snapshot.status, ExecStatus::Killed);
    }

    #[tokio::test]
    async fn remove_kills_active_process() {
        let registry = ExecRegistry::new();
        let command = if cfg!(windows) {
            "[Console]::Out.WriteLine($PID); Start-Sleep -Seconds 30"
        } else {
            "printf \"%s\\n\" $$; sleep 30"
        };
        let result = registry
            .spawn(ExecSpawnRequest::background(shell_script(command)))
            .await
            .unwrap();
        let process_id = result.snapshot.meta.process_id.clone();
        let child_id = loop {
            let read = registry.read(&process_id, 0, None).await;
            if let Some(id) = read.chunks.iter().find_map(|chunk| {
                chunk
                    .text
                    .lines()
                    .find_map(|line| line.trim().parse::<u32>().ok())
            }) {
                break id;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };

        let removed = registry.remove(&process_id).await.unwrap().unwrap();

        assert_eq!(removed.status, ExecStatus::Killed);
        assert!(registry.get(&process_id).await.is_none());
        assert_process_missing(child_id).await;
    }

    #[tokio::test]
    async fn spawn_attach_failure_kills_child() {
        let registry = ExecRegistry::new();
        let owner = crate::types::ExecOwnerMeta {
            service_name: Some("dup".to_string()),
            ..crate::types::ExecOwnerMeta::default()
        };
        let first = registry
            .spawn(
                ExecSpawnRequest::service(shell_script(if cfg!(windows) {
                    "Start-Sleep -Seconds 30"
                } else {
                    "sleep 30"
                }))
                .with_owner(owner.clone()),
            )
            .await
            .unwrap();
        let pid_file = tempfile::NamedTempFile::new().unwrap();
        let pid_path = pid_file.path().to_path_buf();
        let pid_arg = pid_path.to_string_lossy();
        let command = if cfg!(windows) {
            format!(
                "[Console]::Out.WriteLine($PID); [System.IO.File]::WriteAllText('{}', [string]$PID); Start-Sleep -Seconds 30",
                pid_arg.replace("'", "''''")
            )
        } else {
            format!(
                "printf \"%s\\n\" $$; printf \"%s\\n\" $$ > '{}'; sleep 30",
                pid_arg.replace("'", "'\\''")
            )
        };
        let started = Instant::now();
        let err = match registry
            .spawn(
                ExecSpawnRequest::service(shell_script(&command))
                    .with_owner(owner)
                    .with_startup_wait(Duration::from_secs(30)),
            )
            .await
        {
            Ok(_) => panic!("duplicate service spawn should fail"),
            Err(err) => err,
        };

        assert!(
            err.contains("process already exists"),
            "unexpected error: {err}"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        if let Ok(Ok(pid)) =
            std::fs::read_to_string(&pid_path).map(|value| value.trim().parse::<u32>())
        {
            assert_process_missing(pid).await;
        }
        assert_eq!(
            registry
                .get(&first.snapshot.meta.process_id)
                .await
                .unwrap()
                .status,
            ExecStatus::Running
        );
        registry
            .kill(&first.snapshot.meta.process_id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn service_ids_include_workspace_scope() {
        let registry = ExecRegistry::new();
        let first_workspace = tempfile::tempdir().unwrap();
        let second_workspace = tempfile::tempdir().unwrap();
        let command = if cfg!(windows) {
            "[Console]::Out.Write('svc'); Start-Sleep -Seconds 5"
        } else {
            "printf svc; sleep 5"
        };
        let owner_a = crate::types::ExecOwnerMeta {
            chat_id: Some("chat".to_string()),
            tool_call_id: Some("tool-a".to_string()),
            service_name: Some("api".to_string()),
            workspace: Some(first_workspace.path().to_path_buf()),
        };
        let owner_b = crate::types::ExecOwnerMeta {
            chat_id: Some("chat".to_string()),
            tool_call_id: Some("tool-b".to_string()),
            service_name: Some("api".to_string()),
            workspace: Some(second_workspace.path().to_path_buf()),
        };

        let first = registry
            .spawn(
                ExecSpawnRequest::service(shell_script(command))
                    .with_owner(owner_a.clone())
                    .with_startup_wait(Duration::from_millis(50)),
            )
            .await
            .unwrap();
        let second = registry
            .spawn(
                ExecSpawnRequest::service(shell_script(command))
                    .with_owner(owner_b.clone())
                    .with_startup_wait(Duration::from_millis(50)),
            )
            .await
            .unwrap();

        assert_ne!(
            first.snapshot.meta.process_id,
            second.snapshot.meta.process_id
        );
        assert_eq!(first.snapshot.status, ExecStatus::Running);
        assert_eq!(second.snapshot.status, ExecStatus::Running);
        assert_eq!(
            registry
                .find_service(
                    crate::types::ExecServiceLookup::new("api")
                        .with_chat_id("chat")
                        .with_workspace(first_workspace.path().to_path_buf()),
                )
                .await
                .unwrap()
                .meta
                .process_id,
            first.snapshot.meta.process_id
        );
        assert_eq!(
            registry
                .find_service(
                    crate::types::ExecServiceLookup::new("api")
                        .with_chat_id("chat")
                        .with_workspace(second_workspace.path().to_path_buf()),
                )
                .await
                .unwrap()
                .meta
                .process_id,
            second.snapshot.meta.process_id
        );

        registry
            .kill(&first.snapshot.meta.process_id)
            .await
            .unwrap();
        registry
            .kill(&second.snapshot.meta.process_id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn service_ids_include_chat_scope() {
        let registry = ExecRegistry::new();
        let workspace = tempfile::tempdir().unwrap();
        let command = if cfg!(windows) {
            "[Console]::Out.Write('svc'); Start-Sleep -Seconds 5"
        } else {
            "printf svc; sleep 5"
        };
        let owner_a = crate::types::ExecOwnerMeta {
            chat_id: Some("chat-a".to_string()),
            tool_call_id: Some("tool-a".to_string()),
            service_name: Some("api".to_string()),
            workspace: Some(workspace.path().to_path_buf()),
        };
        let owner_b = crate::types::ExecOwnerMeta {
            chat_id: Some("chat-b".to_string()),
            tool_call_id: Some("tool-b".to_string()),
            service_name: Some("api".to_string()),
            workspace: Some(workspace.path().to_path_buf()),
        };

        let first = registry
            .spawn(
                ExecSpawnRequest::service(shell_script(command))
                    .with_owner(owner_a)
                    .with_startup_wait(Duration::from_millis(50)),
            )
            .await
            .unwrap();
        let second = registry
            .spawn(
                ExecSpawnRequest::service(shell_script(command))
                    .with_owner(owner_b)
                    .with_startup_wait(Duration::from_millis(50)),
            )
            .await
            .unwrap();

        assert_ne!(
            first.snapshot.meta.process_id,
            second.snapshot.meta.process_id
        );
        assert_eq!(first.snapshot.status, ExecStatus::Running);
        assert_eq!(second.snapshot.status, ExecStatus::Running);

        registry
            .kill(&first.snapshot.meta.process_id)
            .await
            .unwrap();
        registry
            .kill(&second.snapshot.meta.process_id)
            .await
            .unwrap();
    }
}
