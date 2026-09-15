//! Running orchestration code as a confined OS process.
//!
//! The worker is spawned through `refact-exec` with a `refact-sandbox` spec that
//! grants no write authority and no network. It receives an empty environment
//! and a null stdin, so its only channel back is the host-call program it writes
//! to stdout, and every line of that program is authorized against the ceiling
//! before anything happens.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use refact_work_core::{AdmissionRequest, AdmissionScheduler, CoordinationSettings, RunKey, WorkScope};

use crate::exec::types::{ExecSandboxMode, ExecSandboxSpec};
use crate::exec::{
    ExecEnvPolicy, ExecMode, ExecOutputStream, ExecRegistry, ExecSpawnRequest, ExecStatus,
};

use super::hosts::HostCall;
use super::policy::Capabilities;
use super::CodeModeStatus;

/// Absolute interpreters code mode is willing to run. Resolution never consults
/// `PATH`: the worker has no environment, and a relative program name would be
/// ambient authority over which binary runs.
const WORKER_INTERPRETERS: &[&str] =
    &["/usr/bin/python3", "/usr/local/bin/python3", "/bin/python3"];

const WORKER_INTERPRETER_FLAGS: &[&str] = &["-I", "-S", "-B", "-c"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeModeLimits {
    pub timeout: Duration,
    pub max_host_calls: usize,
    pub max_intermediate_bytes: usize,
}

impl CodeModeLimits {
    pub fn from_settings(settings: &CoordinationSettings) -> Self {
        Self {
            timeout: Duration::from_millis(settings.code_mode_timeout_ms),
            max_host_calls: settings.code_mode_max_host_calls,
            max_intermediate_bytes: settings.code_mode_max_intermediate_bytes,
        }
    }
}

/// Everything that travels through host calls, counted. An orchestration run
/// cannot accumulate state the host never sized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intermediate {
    cap: usize,
    used: usize,
}

impl Intermediate {
    pub fn new(cap: usize) -> Self {
        Self { cap, used: 0 }
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn record(&mut self, value: &Value) -> Result<(), String> {
        let size = serde_json::to_vec(value)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        let used = self.used.saturating_add(size);
        if used > self.cap {
            return Err(format!(
                "code mode refused: intermediate data would exceed {} bytes",
                self.cap
            ));
        }
        self.used = used;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedCall {
    pub op: &'static str,
    pub tool: &'static str,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeModeRun {
    pub calls: Vec<AuthorizedCall>,
    pub exit_code: Option<i32>,
    pub intermediate: Intermediate,
}

pub struct CodeModeRequest {
    pub run: RunKey,
    pub scope: WorkScope,
    pub script: String,
    pub capabilities: Capabilities,
    pub limits: CodeModeLimits,
    pub abort: Arc<AtomicBool>,
}

pub fn interpreter() -> Option<&'static str> {
    WORKER_INTERPRETERS
        .iter()
        .copied()
        .find(|path| Path::new(path).is_file())
}

/// The only confinement code mode ever asks for: nothing writable, no network.
/// `ReadOnly` is what both providers actually enforce, so it is what is claimed.
pub fn worker_sandbox() -> ExecSandboxSpec {
    ExecSandboxSpec {
        mode: ExecSandboxMode::ReadOnly,
        ro_paths: vec![PathBuf::from("/")],
        rw_paths: Vec::new(),
        allow_network: false,
    }
}

pub fn worker_request(
    script: &str,
    limits: &CodeModeLimits,
    abort: Arc<AtomicBool>,
) -> Result<ExecSpawnRequest, String> {
    let interpreter = interpreter().ok_or_else(|| {
        "code mode is unavailable: no confined worker interpreter is installed".to_string()
    })?;
    let mut argv = vec![interpreter.to_string()];
    argv.extend(WORKER_INTERPRETER_FLAGS.iter().map(|flag| flag.to_string()));
    argv.push(script.to_string());
    Ok(ExecSpawnRequest::argv(ExecMode::Foreground, argv)
        .with_cwd(PathBuf::from("/"))
        .with_sandbox(worker_sandbox())
        .with_env_policy(ExecEnvPolicy::Exact)
        .with_env_map(HashMap::new())
        .with_timeout(limits.timeout)
        .with_abort_flag(abort)
        .with_transcript_limit(limits.max_intermediate_bytes)
        .with_short_description("code mode orchestration worker"))
}

/// Read the host-call program the worker emitted. The first line that is not an
/// authorized, schema-valid host call ends the run: partial authorization is not
/// a thing, and unparsed output is never treated as harmless logging.
pub fn authorize_program(
    transcript: &str,
    capabilities: &Capabilities,
    limits: &CodeModeLimits,
) -> Result<(Vec<AuthorizedCall>, Intermediate), String> {
    let mut calls = Vec::new();
    let mut intermediate = Intermediate::new(limits.max_intermediate_bytes);
    for line in transcript.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if calls.len() >= limits.max_host_calls {
            return Err(format!(
                "code mode refused: the worker emitted more than {} host calls",
                limits.max_host_calls
            ));
        }
        let call = HostCall::parse(line)?;
        let op = capabilities.authorize(&call)?;
        intermediate.record(&call.arguments)?;
        calls.push(AuthorizedCall {
            op: op.name,
            tool: op.tool,
            arguments: call.arguments,
        });
    }
    Ok((calls, intermediate))
}

/// Run one confined worker and return the host calls it is authorized to make.
///
/// Capacity is charged against the admission scheduler for the whole run and
/// released on every exit path, including cancellation.
pub async fn execute(
    registry: &ExecRegistry,
    scheduler: &AdmissionScheduler,
    status: &CodeModeStatus,
    request: CodeModeRequest,
) -> Result<CodeModeRun, String> {
    status.require_available()?;
    if request.capabilities.is_empty() {
        return Err(
            "code mode refused: the assignment authority ceiling grants no host operation"
                .to_string(),
        );
    }
    let spawn = worker_request(&request.script, &request.limits, request.abort.clone())?;
    let permit = scheduler
        .reserve(AdmissionRequest::new(
            request.run.clone(),
            request.scope.clone(),
        ))
        .map_err(|error| error.to_string())?
        .acquire()
        .await
        .map_err(|error| error.to_string())?;
    let outcome = run_worker(registry, spawn, &request).await;
    drop(permit);
    outcome
}

async fn run_worker(
    registry: &ExecRegistry,
    spawn: ExecSpawnRequest,
    request: &CodeModeRequest,
) -> Result<CodeModeRun, String> {
    let spawned = registry.spawn(spawn).await?;
    let process_id = spawned.snapshot.meta.process_id.clone();
    let transcript = registry
        .read(&process_id, 0, None)
        .await
        .chunks
        .into_iter()
        .filter(|chunk| chunk.stream == ExecOutputStream::Stdout)
        .map(|chunk| chunk.text)
        .collect::<String>();
    let removed = registry.remove(&process_id).await;
    if request.abort.load(Ordering::SeqCst) {
        removed?;
        return Err("code mode run was cancelled".to_string());
    }
    removed?;
    let (calls, intermediate) =
        authorize_program(&transcript, &request.capabilities, &request.limits)?;
    Ok(CodeModeRun {
        calls,
        exit_code: match spawned.snapshot.status {
            ExecStatus::Exited { exit_code } => exit_code,
            ExecStatus::SandboxLauncherFailed { exit_code } => Some(exit_code),
            _ => None,
        },
        intermediate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_mode::policy::capabilities;
    use crate::global_context::TerminalSecurityMode;
    use crate::subchat::ToolsPolicy;
    use refact_sandbox::{BwrapProvider, Enforcement, SandboxProvider, SandboxStatus};
    use refact_work_core::WorkKey;
    use serde_json::json;

    fn limits() -> CodeModeLimits {
        CodeModeLimits {
            timeout: Duration::from_secs(10),
            max_host_calls: 4,
            max_intermediate_bytes: 4096,
        }
    }

    fn orchestrator() -> Capabilities {
        capabilities(&ToolsPolicy::Only(vec![
            "subagent".into(),
            "agent_wait".into(),
            "progress_report".into(),
        ]))
        .unwrap()
    }

    fn enforcing_status() -> CodeModeStatus {
        crate::code_mode::status(
            true,
            TerminalSecurityMode::SandboxRequired,
            SandboxStatus {
                provider: "bwrap",
                enforcement: Enforcement::Full,
            },
        )
    }

    fn request(script: &str) -> CodeModeRequest {
        CodeModeRequest {
            run: WorkKey::trajectory("code-mode-test").run(1),
            scope: WorkScope::single("code-mode"),
            script: script.to_string(),
            capabilities: orchestrator(),
            limits: limits(),
            abort: Arc::new(AtomicBool::new(false)),
        }
    }

    fn confinement_enforced() -> bool {
        interpreter().is_some() && BwrapProvider.probe() == Enforcement::Full
    }

    #[test]
    fn the_worker_spec_grants_no_write_authority_and_no_network() {
        let sandbox = worker_sandbox();

        assert_eq!(sandbox.mode, ExecSandboxMode::ReadOnly);
        assert!(sandbox.rw_paths.is_empty());
        assert!(!sandbox.allow_network);

        if interpreter().is_none() {
            return;
        }
        let spawn = worker_request("pass", &limits(), Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(spawn.sandbox, Some(sandbox));
        assert_eq!(spawn.env_policy, ExecEnvPolicy::Exact);
        assert!(spawn.env.is_empty());
        assert_eq!(spawn.timeout, Some(Duration::from_secs(10)));
        assert!(!spawn.tty, "a PTY would give the worker an input channel");
        let argv = spawn.argv.clone().unwrap();
        assert!(
            argv[0].starts_with('/'),
            "interpreter must not come from PATH"
        );
        assert_eq!(&argv[1..5], &["-I", "-S", "-B", "-c"]);
    }

    #[test]
    fn the_host_call_count_and_intermediate_size_are_both_capped() {
        let capabilities = orchestrator();
        let program = (0..5)
            .map(|index| format!(r#"{{"op":"report","arguments":{{"status_line":"{index}"}}}}"#))
            .collect::<Vec<_>>()
            .join("\n");

        let too_many = authorize_program(&program, &capabilities, &limits()).unwrap_err();
        assert!(too_many.contains("more than 4 host calls"));

        let tight = CodeModeLimits {
            max_intermediate_bytes: 40,
            ..limits()
        };
        let too_big = authorize_program(
            &format!(
                r#"{{"op":"report","arguments":{{"status_line":"{}"}}}}"#,
                "x".repeat(200)
            ),
            &capabilities,
            &tight,
        )
        .unwrap_err();
        assert!(too_big.contains("intermediate data would exceed 40 bytes"));

        let (calls, intermediate) = authorize_program(
            r#"{"op":"report","arguments":{"status_line":"one"}}"#,
            &capabilities,
            &limits(),
        )
        .unwrap();
        assert_eq!(calls.len(), 1);
        assert!(intermediate.used() > 0 && intermediate.used() <= intermediate.cap());
    }

    #[test]
    fn unparsed_worker_output_is_refused_rather_than_ignored() {
        let denied =
            authorize_program("hello from the worker", &orchestrator(), &limits()).unwrap_err();

        assert!(denied.contains("not a host call"));
    }

    #[tokio::test]
    async fn code_mode_refuses_to_run_when_no_sandbox_provider_can_enforce_it() {
        let registry = ExecRegistry::new();
        let scheduler = AdmissionScheduler::new(CoordinationSettings::default());
        let unusable = crate::code_mode::status(
            true,
            TerminalSecurityMode::SandboxRequired,
            SandboxStatus {
                provider: "noop",
                enforcement: Enforcement::Unusable,
            },
        );

        let refusal = execute(
            &registry,
            &scheduler,
            &unusable,
            request(r#"print('{"op":"report","arguments":{"status_line":"x"}}')"#),
        )
        .await
        .unwrap_err();

        assert!(refusal.contains("code mode is unavailable"));
        assert!(refusal.contains("no usable sandbox provider"));
        assert!(refusal.contains("not executed unconfined"));
        assert_eq!(scheduler.snapshot().outstanding, 0);
        assert!(
            registry.list(Default::default()).await.is_empty(),
            "nothing may be spawned by a refused run"
        );
    }

    #[tokio::test]
    async fn code_mode_is_inert_until_the_setting_enables_it() {
        let registry = ExecRegistry::new();
        let scheduler = AdmissionScheduler::new(CoordinationSettings::default());
        let disabled = crate::code_mode::status(
            CoordinationSettings::default().code_mode_enabled,
            TerminalSecurityMode::SandboxRequired,
            SandboxStatus {
                provider: "bwrap",
                enforcement: Enforcement::Full,
            },
        );

        let refusal = execute(
            &registry,
            &scheduler,
            &disabled,
            request(r#"print('{"op":"report","arguments":{"status_line":"x"}}')"#),
        )
        .await
        .unwrap_err();

        assert!(refusal.contains("code_mode_enabled"));
        assert_eq!(scheduler.snapshot().outstanding, 0);
        assert!(registry.list(Default::default()).await.is_empty());
    }

    #[tokio::test]
    async fn a_ceiling_that_grants_nothing_never_reaches_the_worker() {
        let registry = ExecRegistry::new();
        let scheduler = AdmissionScheduler::new(CoordinationSettings::default());
        let mut request = request("pass");
        request.capabilities = capabilities(&ToolsPolicy::None).unwrap();

        let refusal = execute(&registry, &scheduler, &enforcing_status(), request)
            .await
            .unwrap_err();

        assert!(refusal.contains("grants no host operation"));
        assert_eq!(scheduler.snapshot().outstanding, 0);
        assert!(registry.list(Default::default()).await.is_empty());
    }

    #[tokio::test]
    async fn a_confined_worker_cannot_write_outside_the_sandbox_or_open_a_socket() {
        if !confinement_enforced() {
            return;
        }
        let registry = ExecRegistry::new();
        let scheduler = AdmissionScheduler::new(CoordinationSettings::default());
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let denied = PathBuf::from(home).join("code-mode-should-not-exist");
        let write = format!(
            "import json\ntry:\n    open({}, 'w').write('x')\n    ok = 'wrote'\nexcept Exception as error:\n    ok = 'denied'\nprint(json.dumps({{'op': 'report', 'arguments': {{'status_line': ok}}}}))\n",
            json!(denied.to_string_lossy())
        );

        let run = execute(&registry, &scheduler, &enforcing_status(), request(&write))
            .await
            .unwrap();

        assert_eq!(run.calls.len(), 1);
        assert_eq!(run.calls[0].arguments["status_line"], json!("denied"));
        assert!(!denied.exists());

        let socket = "import json, socket\ntry:\n    socket.create_connection(('127.0.0.1', 1), 1)\n    ok = 'connected'\nexcept Exception:\n    ok = 'denied'\nprint(json.dumps({'op': 'report', 'arguments': {'status_line': ok}}))\n";

        let run = execute(&registry, &scheduler, &enforcing_status(), request(socket))
            .await
            .unwrap();

        assert_eq!(run.calls[0].arguments["status_line"], json!("denied"));
        assert_eq!(scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn a_confined_worker_call_outside_the_ceiling_is_denied_after_it_runs() {
        if !confinement_enforced() {
            return;
        }
        let registry = ExecRegistry::new();
        let scheduler = AdmissionScheduler::new(CoordinationSettings::default());
        let mut request = request(r#"print('{"op":"claim","arguments":{"card_id":"R-7"}}')"#);
        request.capabilities = orchestrator();

        let denied = execute(&registry, &scheduler, &enforcing_status(), request)
            .await
            .unwrap_err();

        assert!(denied.contains("outside the assignment authority ceiling"));
        assert!(denied.contains("board_claim"));
        assert_eq!(scheduler.snapshot().outstanding, 0);
        assert!(registry.list(Default::default()).await.is_empty());
    }

    #[tokio::test]
    async fn cancelling_mid_execution_kills_the_worker_and_releases_capacity() {
        if !confinement_enforced() {
            return;
        }
        let registry = ExecRegistry::new();
        let scheduler = AdmissionScheduler::new(CoordinationSettings::default());
        let abort = Arc::new(AtomicBool::new(false));
        let mut request = request("import time\ntime.sleep(60)\n");
        request.abort = abort.clone();
        let flag = abort.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            flag.store(true, Ordering::SeqCst);
        });

        let cancelled = execute(&registry, &scheduler, &enforcing_status(), request)
            .await
            .unwrap_err();

        assert!(cancelled.contains("cancelled"));
        assert_eq!(scheduler.snapshot().outstanding, 0);
        assert_eq!(scheduler.snapshot().active, 0);
        assert!(registry.list(Default::default()).await.is_empty());
    }
}
