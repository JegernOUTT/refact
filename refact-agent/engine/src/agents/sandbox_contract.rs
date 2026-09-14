//! What the sandbox actually enforces, stated honestly at launch time.
//!
//! This builds no sandbox. It answers one question before a child is dispatched:
//! can the confinement this run asked for actually be applied here? When the
//! answer is no the launch fails with the specific reason, because running the
//! child unconfined after it asked to be confined is a silent downgrade.

use crate::exec::types::ExecSandboxMode;
use crate::global_context::TerminalSecurityMode;

pub fn parse_requested(value: &serde_json::Value) -> Result<Option<ExecSandboxMode>, String> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(name) => match name.trim() {
            "read_only" => Ok(Some(ExecSandboxMode::ReadOnly)),
            "workspace_write" => Ok(Some(ExecSandboxMode::WorkspaceWrite)),
            "full_access" => Ok(Some(ExecSandboxMode::FullAccess)),
            other => Err(format!(
                "argument `sandbox` must be one of read_only, workspace_write, full_access; got `{other}`"
            )),
        },
        other => Err(format!("argument `sandbox` must be a string: {other:?}")),
    }
}

pub fn mode_name(mode: &ExecSandboxMode) -> &'static str {
    match mode {
        ExecSandboxMode::ReadOnly => "read_only",
        ExecSandboxMode::WorkspaceWrite => "workspace_write",
        ExecSandboxMode::FullAccess => "full_access",
    }
}

fn rank(mode: &ExecSandboxMode) -> u8 {
    match mode {
        ExecSandboxMode::ReadOnly => 0,
        ExecSandboxMode::WorkspaceWrite => 1,
        ExecSandboxMode::FullAccess => 2,
    }
}

fn terminal_mode_name(mode: TerminalSecurityMode) -> &'static str {
    match mode {
        TerminalSecurityMode::Off => "off",
        TerminalSecurityMode::Audit => "audit",
        TerminalSecurityMode::ApprovalOnly => "approval_only",
        TerminalSecurityMode::SandboxPreferred => "sandbox_preferred",
        TerminalSecurityMode::SandboxRequired => "sandbox_required",
    }
}

fn enforcement_name(enforcement: refact_sandbox::Enforcement) -> &'static str {
    match enforcement {
        refact_sandbox::Enforcement::Full => "full",
        refact_sandbox::Enforcement::Partial => "partial",
        refact_sandbox::Enforcement::Unusable => "unusable",
    }
}

/// `effective` is the sandbox level the child's own chat mode would produce; a
/// request for something stricter than that is not enforceable either, because
/// nothing narrows the level per run.
pub fn preflight(
    requested: Option<ExecSandboxMode>,
    effective: ExecSandboxMode,
    terminal_mode: TerminalSecurityMode,
    status: refact_sandbox::SandboxStatus,
) -> Result<(), String> {
    let Some(requested) = requested else {
        return Ok(());
    };
    let asked = mode_name(&requested);
    let provider = status.provider;
    let enforcement = enforcement_name(status.enforcement);
    let refuse = |reason: String| {
        Err(format!(
            "launch refused before dispatch: sandbox `{asked}` cannot be enforced here — {reason}. The child was not started; it would otherwise have run unsandboxed."
        ))
    };
    if status.enforcement == refact_sandbox::Enforcement::Unusable {
        return refuse(format!(
            "no usable sandbox provider is available ({provider}: {enforcement})"
        ));
    }
    match terminal_mode {
        TerminalSecurityMode::Off
        | TerminalSecurityMode::Audit
        | TerminalSecurityMode::ApprovalOnly => {
            return refuse(format!(
                "terminal security mode `{}` never confines execution",
                terminal_mode_name(terminal_mode)
            ));
        }
        TerminalSecurityMode::SandboxPreferred
            if status.enforcement != refact_sandbox::Enforcement::Full =>
        {
            return refuse(format!(
                "terminal security mode `sandbox_preferred` runs unconfined without full enforcement ({provider}: {enforcement})"
            ));
        }
        TerminalSecurityMode::SandboxPreferred | TerminalSecurityMode::SandboxRequired => {}
    }
    if rank(&requested) < rank(&effective) {
        return refuse(format!(
            "this run resolves to `{}` and no per-run narrowing exists",
            mode_name(&effective)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(enforcement: refact_sandbox::Enforcement) -> refact_sandbox::SandboxStatus {
        refact_sandbox::SandboxStatus {
            provider: "test",
            enforcement,
        }
    }

    #[test]
    fn a_run_that_asks_for_nothing_is_unaffected() {
        for mode in [
            TerminalSecurityMode::Off,
            TerminalSecurityMode::Audit,
            TerminalSecurityMode::ApprovalOnly,
            TerminalSecurityMode::SandboxPreferred,
            TerminalSecurityMode::SandboxRequired,
        ] {
            for enforcement in [
                refact_sandbox::Enforcement::Full,
                refact_sandbox::Enforcement::Partial,
                refact_sandbox::Enforcement::Unusable,
            ] {
                preflight(
                    None,
                    ExecSandboxMode::WorkspaceWrite,
                    mode,
                    status(enforcement),
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn an_unenforceable_request_fails_before_dispatch_rather_than_degrading() {
        let error = preflight(
            Some(ExecSandboxMode::WorkspaceWrite),
            ExecSandboxMode::WorkspaceWrite,
            TerminalSecurityMode::SandboxRequired,
            status(refact_sandbox::Enforcement::Unusable),
        )
        .unwrap_err();
        assert!(error.contains("launch refused before dispatch"));
        assert!(error.contains("no usable sandbox provider"));
        assert!(error.contains("was not started"));

        for mode in [
            TerminalSecurityMode::Off,
            TerminalSecurityMode::Audit,
            TerminalSecurityMode::ApprovalOnly,
        ] {
            let error = preflight(
                Some(ExecSandboxMode::WorkspaceWrite),
                ExecSandboxMode::WorkspaceWrite,
                mode,
                status(refact_sandbox::Enforcement::Full),
            )
            .unwrap_err();
            assert!(error.contains("never confines execution"), "{mode:?}");
        }

        let preferred = preflight(
            Some(ExecSandboxMode::WorkspaceWrite),
            ExecSandboxMode::WorkspaceWrite,
            TerminalSecurityMode::SandboxPreferred,
            status(refact_sandbox::Enforcement::Partial),
        )
        .unwrap_err();
        assert!(preferred.contains("runs unconfined"));
    }

    #[test]
    fn an_enforceable_request_is_admitted() {
        preflight(
            Some(ExecSandboxMode::WorkspaceWrite),
            ExecSandboxMode::WorkspaceWrite,
            TerminalSecurityMode::SandboxRequired,
            status(refact_sandbox::Enforcement::Partial),
        )
        .unwrap();
        preflight(
            Some(ExecSandboxMode::FullAccess),
            ExecSandboxMode::WorkspaceWrite,
            TerminalSecurityMode::SandboxPreferred,
            status(refact_sandbox::Enforcement::Full),
        )
        .unwrap();
    }

    #[test]
    fn asking_for_a_tighter_level_than_the_run_resolves_to_is_refused() {
        let error = preflight(
            Some(ExecSandboxMode::ReadOnly),
            ExecSandboxMode::WorkspaceWrite,
            TerminalSecurityMode::SandboxRequired,
            status(refact_sandbox::Enforcement::Full),
        )
        .unwrap_err();
        assert!(error.contains("no per-run narrowing exists"));
        assert!(error.contains("workspace_write"));
    }

    #[test]
    fn requested_levels_parse_exactly_and_reject_anything_else() {
        assert_eq!(
            parse_requested(&serde_json::json!("workspace_write")).unwrap(),
            Some(ExecSandboxMode::WorkspaceWrite)
        );
        assert_eq!(
            parse_requested(&serde_json::json!("read_only")).unwrap(),
            Some(ExecSandboxMode::ReadOnly)
        );
        assert_eq!(
            parse_requested(&serde_json::json!("full_access")).unwrap(),
            Some(ExecSandboxMode::FullAccess)
        );
        assert_eq!(parse_requested(&serde_json::Value::Null).unwrap(), None);
        assert!(parse_requested(&serde_json::json!("seccomp")).is_err());
        assert!(parse_requested(&serde_json::json!(true)).is_err());
    }
}
