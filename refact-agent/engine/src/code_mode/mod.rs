//! Governed code mode: model-authored orchestration, executed only where the OS
//! can actually confine it.
//!
//! Worker threads are not a security boundary, so code mode does not pretend to
//! be one. The orchestration script runs as an OS process confined by
//! `refact-sandbox` through `refact-exec`, and when no provider can enforce that
//! confinement the feature is unavailable — not degraded, not best-effort. The
//! script has no ambient authority: an empty environment, no writable path, no
//! network, no stdin, and no reachable tool except the declared host operations
//! its authority ceiling already grants.
//!
//! What is enforced is stated exactly. `SandboxMode::ReadOnly` removes all write
//! authority and `allow_network: false` removes the network; `ro_paths` is not a
//! read restriction under the bubblewrap provider, which read-only binds `/`, so
//! code mode does not claim read confinement it does not have.

pub mod hosts;
pub mod policy;
pub mod session;

pub use hosts::{
    declared_names, operation, HostArgument, HostCall, HostOperation, HostValueType,
    HOST_OPERATIONS,
};
pub use policy::{capabilities, Capabilities};
pub use session::{
    authorize_program, execute, interpreter, worker_request, worker_sandbox, AuthorizedCall,
    CodeModeLimits, CodeModeRequest, CodeModeRun, Intermediate,
};

use refact_sandbox::{Enforcement, SandboxStatus};

use crate::agents::sandbox_contract::confinement_unavailable_reason;
use crate::global_context::TerminalSecurityMode;

/// Whether code mode can run here, and if not, precisely why. The capability
/// report the Settings surface shows is this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeModeStatus {
    pub enabled: bool,
    pub provider: &'static str,
    pub enforcement: Enforcement,
    pub refusal: Option<String>,
}

impl CodeModeStatus {
    pub fn available(&self) -> bool {
        self.refusal.is_none()
    }

    pub fn require_available(&self) -> Result<(), String> {
        match self.refusal.as_ref() {
            Some(refusal) => Err(refusal.clone()),
            None => Ok(()),
        }
    }
}

/// Default off, and closed whenever confinement cannot be enforced. The two are
/// separate refusals so an operator can tell "not switched on" from "cannot be
/// switched on here".
pub fn status(
    enabled: bool,
    terminal_mode: TerminalSecurityMode,
    sandbox: SandboxStatus,
) -> CodeModeStatus {
    let refusal = if !enabled {
        Some("code mode is unavailable: the `code_mode_enabled` setting is off".to_string())
    } else {
        confinement_unavailable_reason(terminal_mode, sandbox).map(|reason| {
            format!(
                "code mode is unavailable: {reason}. Orchestration code is not executed unconfined."
            )
        })
    };
    CodeModeStatus {
        enabled,
        provider: sandbox.provider,
        enforcement: sandbox.enforcement,
        refusal,
    }
}

/// The live capability report: the configured flag joined to the sandbox the
/// process actually probed.
pub fn current_status(
    settings: &refact_work_core::CoordinationSettings,
    terminal_mode: TerminalSecurityMode,
) -> CodeModeStatus {
    status(
        settings.code_mode_enabled,
        terminal_mode,
        refact_sandbox::sandbox_status(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(enforcement: Enforcement) -> SandboxStatus {
        SandboxStatus {
            provider: "test",
            enforcement,
        }
    }

    #[test]
    fn code_mode_is_off_by_default_and_says_so() {
        let settings = refact_work_core::CoordinationSettings::default();
        assert!(!settings.code_mode_enabled);

        let status = current_status(&settings, TerminalSecurityMode::SandboxRequired);

        assert!(!status.available());
        assert!(status
            .require_available()
            .unwrap_err()
            .contains("`code_mode_enabled` setting is off"));
    }

    #[test]
    fn an_unusable_provider_closes_code_mode_rather_than_degrading_it() {
        let status = status(
            true,
            TerminalSecurityMode::SandboxRequired,
            sandbox(Enforcement::Unusable),
        );

        let refusal = status.require_available().unwrap_err();

        assert!(!status.available());
        assert!(refusal.contains("no usable sandbox provider"));
        assert!(refusal.contains("not executed unconfined"));
        assert_eq!(status.enforcement, Enforcement::Unusable);
    }

    #[test]
    fn a_terminal_mode_that_never_confines_closes_code_mode_too() {
        for mode in [
            TerminalSecurityMode::Off,
            TerminalSecurityMode::Audit,
            TerminalSecurityMode::ApprovalOnly,
        ] {
            let status = status(true, mode, sandbox(Enforcement::Full));
            assert!(status
                .require_available()
                .unwrap_err()
                .contains("never confines execution"));
        }
        assert!(status(
            true,
            TerminalSecurityMode::SandboxPreferred,
            sandbox(Enforcement::Partial)
        )
        .require_available()
        .unwrap_err()
        .contains("runs unconfined"));
    }

    #[test]
    fn enforced_confinement_with_the_flag_on_is_the_only_available_state() {
        for enforcement in [Enforcement::Full, Enforcement::Partial] {
            let status = status(
                true,
                TerminalSecurityMode::SandboxRequired,
                sandbox(enforcement),
            );
            assert!(status.available());
            status.require_available().unwrap();
        }
    }
}
