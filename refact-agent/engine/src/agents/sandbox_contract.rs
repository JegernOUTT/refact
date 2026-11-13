//! Availability of OS execution confinement for code mode.

use crate::global_context::TerminalSecurityMode;

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

/// Why OS confinement cannot be applied here at all, independent of what any
/// particular run asked for. `None` means a child that asks to be confined
/// really would be. This is the single place that answers the question, so a
/// caller that refuses rather than degrades does not invent a second one.
pub fn confinement_unavailable_reason(
    terminal_mode: TerminalSecurityMode,
    status: refact_sandbox::SandboxStatus,
) -> Option<String> {
    let provider = status.provider;
    let enforcement = enforcement_name(status.enforcement);
    if status.enforcement == refact_sandbox::Enforcement::Unusable {
        return Some(format!(
            "no usable sandbox provider is available ({provider}: {enforcement})"
        ));
    }
    match terminal_mode {
        TerminalSecurityMode::Off
        | TerminalSecurityMode::Audit
        | TerminalSecurityMode::ApprovalOnly => Some(format!(
            "terminal security mode `{}` never confines execution",
            terminal_mode_name(terminal_mode)
        )),
        TerminalSecurityMode::SandboxPreferred
            if status.enforcement != refact_sandbox::Enforcement::Full =>
        {
            Some(format!(
                "terminal security mode `sandbox_preferred` runs unconfined without full enforcement ({provider}: {enforcement})"
            ))
        }
        TerminalSecurityMode::SandboxPreferred | TerminalSecurityMode::SandboxRequired => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confinement_availability_preserves_terminal_policy() {
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
                let reason = confinement_unavailable_reason(
                    mode,
                    refact_sandbox::SandboxStatus {
                        provider: "test",
                        enforcement,
                    },
                );
                let supported = enforcement != refact_sandbox::Enforcement::Unusable
                    && (mode == TerminalSecurityMode::SandboxRequired
                        || (mode == TerminalSecurityMode::SandboxPreferred
                            && enforcement == refact_sandbox::Enforcement::Full));
                assert_eq!(reason.is_none(), supported, "{mode:?}: {enforcement:?}");
            }
        }
    }
}
