use std::path::Path;

use crate::probe::{run_with_timeout, PROBE_TIMEOUT};
use crate::{Enforcement, ExecSandboxSpec, SandboxError, SandboxMode, SandboxProvider};

#[derive(Default)]
pub struct BwrapProvider;

impl BwrapProvider {
    fn base_args() -> Vec<String> {
        [
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--tmpfs",
            "/tmp",
            "--unshare-pid",
            "--die-with-parent",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    fn launcher_args(spec: &ExecSandboxSpec, program: &str, args: &[String]) -> Vec<String> {
        let mut launcher_args = Self::base_args();
        if spec.mode != SandboxMode::ReadOnly {
            for path in &spec.rw_paths {
                let path = path.to_string_lossy().into_owned();
                launcher_args.extend(["--bind".to_string(), path.clone(), path]);
            }
        }
        if !spec.allow_network {
            launcher_args.push("--unshare-net".to_string());
        }
        launcher_args.push("--".to_string());
        launcher_args.push(program.to_string());
        launcher_args.extend_from_slice(args);
        launcher_args
    }

    fn probe_args(script: &str) -> Vec<String> {
        let mut args = Self::base_args();
        args.extend([
            "--".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            script.to_string(),
        ]);
        args
    }
}

impl SandboxProvider for BwrapProvider {
    fn name(&self) -> &'static str {
        "bwrap"
    }

    fn probe(&self) -> Enforcement {
        let success = run_with_timeout("bwrap", &Self::probe_args("true"), PROBE_TIMEOUT)
            .map(|status| status.success())
            .unwrap_or(false);
        let denied = run_with_timeout(
            "bwrap",
            &Self::probe_args("touch /probe-denied"),
            PROBE_TIMEOUT,
        )
        .map(|status| !status.success() && !Path::new("/probe-denied").exists())
        .unwrap_or(false);
        if Path::new("/probe-denied").exists() {
            let _ = std::fs::remove_file("/probe-denied");
        }
        if success && denied {
            Enforcement::Full
        } else {
            Enforcement::Unusable
        }
    }

    fn confine(
        &self,
        spec: &ExecSandboxSpec,
        program: &str,
        args: &[String],
    ) -> Result<(String, Vec<String>), SandboxError> {
        Ok((
            "bwrap".to_string(),
            Self::launcher_args(spec, program, args),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn bwrap_argv_matches_workspace_write_profile() {
        let spec = ExecSandboxSpec {
            mode: SandboxMode::WorkspaceWrite,
            ro_paths: vec![PathBuf::from("/")],
            rw_paths: vec![PathBuf::from("/workspace"), PathBuf::from("/tmp")],
            allow_network: false,
        };

        let (_, args) = BwrapProvider
            .confine(&spec, "sh", &["-c".to_string(), "echo ok".to_string()])
            .unwrap();

        assert_eq!(
            args,
            vec![
                "--ro-bind",
                "/",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--tmpfs",
                "/tmp",
                "--unshare-pid",
                "--die-with-parent",
                "--bind",
                "/workspace",
                "/workspace",
                "--bind",
                "/tmp",
                "/tmp",
                "--unshare-net",
                "--",
                "sh",
                "-c",
                "echo ok",
            ]
        );
    }

    #[test]
    fn read_only_profile_has_no_writable_bind() {
        let spec = ExecSandboxSpec {
            mode: SandboxMode::ReadOnly,
            ro_paths: vec![PathBuf::from("/")],
            rw_paths: vec![PathBuf::from("/workspace")],
            allow_network: true,
        };

        let (_, args) = BwrapProvider.confine(&spec, "true", &[]).unwrap();

        assert!(!args.iter().any(|arg| arg == "--bind"));
        assert!(!args.iter().any(|arg| arg == "--unshare-net"));
    }
}
