use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

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
    let outside = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
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
    let workspace = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
    let denied = workspace.path().join("denied");
    let spec = bwrap_spec(SandboxMode::ReadOnly, workspace.path(), true);

    let output = run_confined(&provider, spec, "sh", &shell_touch(&denied));

    assert!(!output.status.success());
    assert!(!denied.exists());
}

#[test]
fn bwrap_network_isolation_fast_fails_loopback_curl() {
    let provider = BwrapProvider;
    if provider.probe() != Enforcement::Full {
        assert_eq!(provider.probe(), Enforcement::Unusable);
        return;
    }
    if Command::new("curl").arg("--version").output().is_err() {
        return;
    }
    let workspace = tempfile::tempdir().unwrap();
    let spec = bwrap_spec(SandboxMode::WorkspaceWrite, workspace.path(), false);
    let started = Instant::now();

    let output = run_confined(
        &provider,
        spec,
        "curl",
        &[
            "--max-time".to_string(),
            "1".to_string(),
            "http://127.0.0.1:1".to_string(),
        ],
    );

    assert!(!output.status.success());
    assert!(started.elapsed() < Duration::from_secs(2));
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
    let outside = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
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
