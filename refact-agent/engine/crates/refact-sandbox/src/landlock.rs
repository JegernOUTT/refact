use std::path::PathBuf;

use crate::probe::{run_with_timeout, PROBE_TIMEOUT};
use crate::{Enforcement, ExecSandboxSpec, SandboxError, SandboxProvider};

const SANDBOX_EXEC_FLAG: &str = "--sandbox-exec";

#[derive(Default)]
pub struct LandlockProvider;

impl SandboxProvider for LandlockProvider {
    fn name(&self) -> &'static str {
        "landlock"
    }

    fn probe(&self) -> Enforcement {
        #[cfg(target_os = "linux")]
        {
            let Ok(temp) = tempfile_path() else {
                return Enforcement::Unusable;
            };
            let denied_path = temp.join("denied");
            let spec = ExecSandboxSpec {
                mode: crate::SandboxMode::ReadOnly,
                ro_paths: vec![PathBuf::from("/")],
                rw_paths: Vec::new(),
                allow_network: true,
            };
            let Ok((program, args)) = self.confine(
                &spec,
                "sh",
                &[
                    "-c".to_string(),
                    format!("touch {}", shell_quote(&denied_path)),
                ],
            ) else {
                let _ = std::fs::remove_dir_all(&temp);
                return Enforcement::Unusable;
            };
            let success = self
                .confine(&spec, "true", &[])
                .ok()
                .and_then(|(program, args)| run_with_timeout(&program, &args, PROBE_TIMEOUT).ok())
                .map(|status| status.success())
                .unwrap_or(false);
            let denied = run_with_timeout(&program, &args, PROBE_TIMEOUT)
                .map(|status| !status.success() && !denied_path.exists())
                .unwrap_or(false);
            let _ = std::fs::remove_dir_all(&temp);
            if success && denied {
                Enforcement::Partial
            } else {
                Enforcement::Unusable
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            Enforcement::Unusable
        }
    }

    fn confine(
        &self,
        spec: &ExecSandboxSpec,
        program: &str,
        args: &[String],
    ) -> Result<(String, Vec<String>), SandboxError> {
        #[cfg(target_os = "linux")]
        {
            ensure_supported_spec(spec)?;
            let current_exe = std::env::current_exe().map_err(|error| {
                SandboxError::new(
                    self.name(),
                    format!("cannot resolve current executable: {error}"),
                )
            })?;
            let spec_json = serde_json::to_string(spec).map_err(|error| {
                SandboxError::new(self.name(), format!("cannot encode sandbox spec: {error}"))
            })?;
            let mut launcher_args = vec![
                SANDBOX_EXEC_FLAG.to_string(),
                spec_json,
                "--".to_string(),
                program.to_string(),
            ];
            launcher_args.extend_from_slice(args);
            Ok((current_exe.to_string_lossy().into_owned(), launcher_args))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (spec, program, args);
            Err(SandboxError::new(
                self.name(),
                "Landlock is only available on Linux",
            ))
        }
    }
}

pub fn run_sandbox_exec_from_env() -> Option<Result<(), SandboxError>> {
    let args = std::env::args_os().collect::<Vec<_>>();
    if args.get(1).and_then(|arg| arg.to_str()) != Some(SANDBOX_EXEC_FLAG) {
        return None;
    }
    Some(run_sandbox_exec_args(&args[2..]))
}

fn run_sandbox_exec_args(args: &[std::ffi::OsString]) -> Result<(), SandboxError> {
    let provider = "landlock";
    let spec_json = args
        .first()
        .and_then(|arg| arg.to_str())
        .ok_or_else(|| SandboxError::new(provider, "missing JSON sandbox spec"))?;
    if args.get(1).and_then(|arg| arg.to_str()) != Some("--") {
        return Err(SandboxError::new(provider, "missing argv separator"));
    }
    let program = args
        .get(2)
        .ok_or_else(|| SandboxError::new(provider, "missing target program"))?;
    let spec: ExecSandboxSpec = serde_json::from_str(spec_json).map_err(|error| {
        SandboxError::new(provider, format!("invalid JSON sandbox spec: {error}"))
    })?;
    apply_landlock(&spec)?;
    exec_target(program, &args[3..])
}

#[cfg(target_os = "linux")]
fn apply_landlock(spec: &ExecSandboxSpec) -> Result<Enforcement, SandboxError> {
    use landlock::{
        path_beneath_rules, Access, AccessFs, Ruleset, RulesetAttr, RulesetCreatedAttr,
        RulesetStatus, ABI,
    };

    let provider = "landlock";
    ensure_supported_spec(spec)?;
    let abi = ABI::V9;
    let access_all = AccessFs::from_all(abi);
    let access_read = AccessFs::from_read(abi);
    let mut ruleset = Ruleset::default()
        .handle_access(access_all)
        .map_err(|error| SandboxError::new(provider, error.to_string()))?
        .create()
        .map_err(|error| SandboxError::new(provider, error.to_string()))?;
    if !spec.ro_paths.is_empty() {
        ruleset = ruleset
            .add_rules(path_beneath_rules(&spec.ro_paths, access_read))
            .map_err(|error| SandboxError::new(provider, error.to_string()))?;
    }
    if !spec.rw_paths.is_empty() {
        ruleset = ruleset
            .add_rules(path_beneath_rules(&spec.rw_paths, access_all))
            .map_err(|error| SandboxError::new(provider, error.to_string()))?;
    }
    // landlock 0.4.7 RulesetCreated::restrict_self calls try_set_no_new_privs before
    // landlock_restrict_self, which uses prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0).
    let status = ruleset
        .restrict_self()
        .map_err(|error| SandboxError::new(provider, error.to_string()))?;
    match status.ruleset {
        RulesetStatus::FullyEnforced => Ok(Enforcement::Full),
        RulesetStatus::PartiallyEnforced => Ok(Enforcement::Partial),
        RulesetStatus::NotEnforced => Err(SandboxError::new(
            provider,
            "Landlock is not enforced by this kernel",
        )),
    }
}

fn ensure_supported_spec(spec: &ExecSandboxSpec) -> Result<(), SandboxError> {
    if spec.allow_network {
        Ok(())
    } else {
        Err(SandboxError::new(
            "landlock",
            "network isolation is unsupported; use bubblewrap or allow network access explicitly",
        ))
    }
}

#[cfg(not(target_os = "linux"))]
fn apply_landlock(_spec: &ExecSandboxSpec) -> Result<Enforcement, SandboxError> {
    Err(SandboxError::new(
        "landlock",
        "Landlock is only available on Linux",
    ))
}

#[cfg(unix)]
fn exec_target(program: &std::ffi::OsStr, args: &[std::ffi::OsString]) -> Result<(), SandboxError> {
    use std::os::unix::process::CommandExt;

    let error = std::process::Command::new(program).args(args).exec();
    Err(SandboxError::new(
        "landlock",
        format!("failed to execute target: {error}"),
    ))
}

#[cfg(not(unix))]
fn exec_target(
    _program: &std::ffi::OsStr,
    _args: &[std::ffi::OsString],
) -> Result<(), SandboxError> {
    Err(SandboxError::new(
        "landlock",
        "Landlock is only available on Linux",
    ))
}

#[cfg(target_os = "linux")]
fn tempfile_path() -> Result<PathBuf, std::io::Error> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_PROBE: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "refact-sandbox-probe-{}-{}",
        std::process::id(),
        NEXT_PROBE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path)?;
    Ok(path)
}

#[cfg(target_os = "linux")]
fn shell_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SandboxMode;

    #[test]
    fn landlock_json_spec_round_trip() {
        let spec = ExecSandboxSpec {
            mode: SandboxMode::WorkspaceWrite,
            ro_paths: vec![PathBuf::from("/")],
            rw_paths: vec![PathBuf::from("/workspace")],
            allow_network: false,
        };

        let json = serde_json::to_string(&spec).unwrap();
        let decoded: ExecSandboxSpec = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, spec);
    }

    #[test]
    fn hidden_args_require_separator_and_target() {
        let spec = serde_json::to_string(&ExecSandboxSpec {
            mode: SandboxMode::ReadOnly,
            ro_paths: vec![PathBuf::from("/")],
            rw_paths: Vec::new(),
            allow_network: true,
        })
        .unwrap();

        let error = run_sandbox_exec_args(&[spec.into()]).unwrap_err();

        assert!(error.to_string().contains("missing argv separator"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn landlock_refuses_network_isolation() {
        let spec = ExecSandboxSpec {
            mode: SandboxMode::ReadOnly,
            ro_paths: vec![PathBuf::from("/")],
            rw_paths: Vec::new(),
            allow_network: false,
        };

        let error = LandlockProvider.confine(&spec, "true", &[]).unwrap_err();

        assert_eq!(error.provider, "landlock");
        assert!(error.reason.contains("network isolation is unsupported"));
    }
}
