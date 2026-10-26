use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use refact_sandbox::{BwrapProvider, Enforcement, ExecSandboxSpec, SandboxMode, SandboxProvider};

fn run_confined(
    provider: &dyn SandboxProvider,
    spec: ExecSandboxSpec,
    program: &str,
    args: &[String],
) -> Output {
    let (launcher, launcher_args) = provider.confine(&spec, program, args).unwrap();
    Command::new(launcher).args(launcher_args).output().unwrap()
}

fn shell_touch(path: &Path) -> Vec<String> {
    vec![
        "-c".to_string(),
        format!("touch '{}'", path.to_string_lossy().replace('\'', "'\\''")),
    ]
}

fn dir_outside_temp() -> Option<tempfile::TempDir> {
    tempfile::tempdir_in(std::env::var_os("HOME")?).ok()
}

fn bwrap_spec(mode: SandboxMode, workspace: &Path, allow_network: bool) -> ExecSandboxSpec {
    ExecSandboxSpec {
        mode,
        ro_paths: Vec::new(),
        rw_paths: Vec::new(),
        allow_network,
    }
    .normalized(workspace)
}

#[test]
fn workspace_write_allows_workspace_and_denies_outside() {
    let provider = BwrapProvider;
    if provider.probe() != Enforcement::Full {
        assert_eq!(provider.probe(), Enforcement::Unusable);
        return;
    }
    let workspace = tempfile::tempdir().unwrap();
    let Some(outside) = dir_outside_temp() else {
        return;
    };
    let allowed = workspace.path().join("allowed");
    let denied = outside.path().join("denied");
    let spec = bwrap_spec(SandboxMode::WorkspaceWrite, workspace.path(), true);

    let allowed_output = run_confined(&provider, spec.clone(), "sh", &shell_touch(&allowed));
    let denied_output = run_confined(&provider, spec, "sh", &shell_touch(&denied));

    assert!(allowed_output.status.success());
    assert!(allowed.exists());
    assert!(!denied_output.status.success());
    assert!(!denied.exists());
}

#[test]
fn read_only_denies_workspace_writes() {
    let provider = BwrapProvider;
    if provider.probe() != Enforcement::Full {
        assert_eq!(provider.probe(), Enforcement::Unusable);
        return;
    }
    let Some(workspace) = dir_outside_temp() else {
        return;
    };
    let denied = workspace.path().join("denied");
    let spec = bwrap_spec(SandboxMode::ReadOnly, workspace.path(), true);

    let output = run_confined(&provider, spec, "sh", &shell_touch(&denied));

    assert!(!output.status.success());
    assert!(!denied.exists());
}

fn hidden_landlock_output(spec: &ExecSandboxSpec, program: &str, args: &[String]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_refact-sandbox-helper"));
    command
        .arg("--sandbox-exec")
        .arg(serde_json::to_string(spec).unwrap())
        .arg("--")
        .arg(program)
        .args(args)
        .output()
        .unwrap()
}

#[cfg(target_os = "linux")]
const LOADER_FAILURE_EXIT_CODE: i32 = 127;

#[cfg(target_os = "linux")]
fn hidden_launcher_can_exec() -> bool {
    let spec = ExecSandboxSpec {
        mode: SandboxMode::ReadOnly,
        ro_paths: vec![PathBuf::from("/")],
        rw_paths: Vec::new(),
        allow_network: true,
    };
    hidden_landlock_output(&spec, "true", &[]).status.code() != Some(LOADER_FAILURE_EXIT_CODE)
}

#[cfg(target_os = "linux")]
#[test]
fn landlock_hidden_launcher_refuses_network_isolation() {
    if !hidden_launcher_can_exec() {
        return;
    }
    let spec = ExecSandboxSpec {
        mode: SandboxMode::ReadOnly,
        ro_paths: vec![PathBuf::from("/")],
        rw_paths: Vec::new(),
        allow_network: false,
    };

    let output = hidden_landlock_output(&spec, "true", &[]);

    assert_eq!(
        output.status.code(),
        Some(refact_sandbox::SANDBOX_LAUNCHER_FAILURE_EXIT_CODE)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("network isolation is unsupported"));
}

#[cfg(target_os = "linux")]
fn landlock_available() -> bool {
    let spec = ExecSandboxSpec {
        mode: SandboxMode::ReadOnly,
        ro_paths: vec![PathBuf::from("/")],
        rw_paths: Vec::new(),
        allow_network: true,
    };
    hidden_landlock_output(&spec, "true", &[]).status.success()
}

#[cfg(target_os = "linux")]
#[test]
fn hidden_sandbox_exec_runs_target() {
    if !landlock_available() {
        return;
    }
    let spec = ExecSandboxSpec {
        mode: SandboxMode::ReadOnly,
        ro_paths: vec![PathBuf::from("/")],
        rw_paths: Vec::new(),
        allow_network: true,
    };

    let output = hidden_landlock_output(&spec, "sh", &["-c".to_string(), "echo ok".to_string()]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok\n");
}

#[cfg(target_os = "linux")]
#[test]
fn landlock_workspace_write_allows_workspace_and_denies_outside() {
    if !landlock_available() {
        return;
    }
    let workspace = tempfile::tempdir().unwrap();
    let Some(outside) = dir_outside_temp() else {
        return;
    };
    let allowed = workspace.path().join("allowed");
    let denied = outside.path().join("denied");
    let spec = ExecSandboxSpec {
        mode: SandboxMode::WorkspaceWrite,
        ro_paths: vec![PathBuf::from("/")],
        rw_paths: vec![workspace.path().to_path_buf(), std::env::temp_dir()],
        allow_network: true,
    };

    let allowed_output = hidden_landlock_output(&spec, "sh", &shell_touch(&allowed));
    let denied_output = hidden_landlock_output(&spec, "sh", &shell_touch(&denied));

    assert!(allowed_output.status.success());
    assert!(allowed.exists());
    assert!(!denied_output.status.success());
    assert!(!denied.exists());
}
