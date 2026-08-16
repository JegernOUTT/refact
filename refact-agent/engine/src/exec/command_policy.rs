use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::chat::internal_roles::{event, EventSubkind};
use crate::exec::{ExecAuditMeta, ExecEnvPolicy, ExecMode, ExecSpawnRequest};
use crate::exec::types::{ExecSandboxMode, ExecSandboxSpec};
use crate::global_context::{GlobalContext, TerminalSecurityMode};

pub type ChatMode = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecSource {
    ShellTool,
    ProcessTool,
    CmdlineIntegration,
    SchedulerJob,
    Verifier,
    ReviewEvidence,
}

impl ExecSource {
    pub fn audit_source(self) -> &'static str {
        match self {
            Self::ShellTool => "shell_tool",
            Self::ProcessTool => "process_tool",
            Self::CmdlineIntegration => "cmdline_integration",
            Self::SchedulerJob => "scheduler_job",
            Self::Verifier => "verifier",
            Self::ReviewEvidence => "review_evidence",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecEscalationMode {
    WorkspaceWrite,
    FullAccess,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExecEscalation {
    pub mode: ExecEscalationMode,
    pub justification: String,
}

impl ExecEscalation {
    pub fn validate(self) -> Result<Self, String> {
        if self.justification.trim().is_empty() {
            return Err("escalate.justification must be a non-empty string".to_string());
        }
        Ok(Self {
            justification: self.justification.trim().to_string(),
            ..self
        })
    }

    pub fn mode_name(&self) -> &'static str {
        match self.mode {
            ExecEscalationMode::WorkspaceWrite => "workspace_write",
            ExecEscalationMode::FullAccess => "full_access",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxAuditRecord {
    pub mode: String,
    pub provider: String,
    pub enforcement: String,
    pub source: String,
    pub content: String,
}

impl SandboxAuditRecord {
    pub fn message(&self) -> crate::call_validation::ChatMessage {
        event(
            EventSubkind::SystemNotice,
            "exec.sandbox",
            json!({
                "mode": self.mode,
                "provider": self.provider,
                "enforcement": self.enforcement,
                "source": self.source,
            }),
            self.content.clone(),
        )
    }
}

pub struct ExecRequestPolicy {
    pub request: ExecSpawnRequest,
    pub warning: Option<String>,
    pub audit: Option<SandboxAuditRecord>,
}

#[derive(Debug)]
pub struct ExecPolicyError {
    pub message: String,
    pub audit: Option<SandboxAuditRecord>,
}

impl std::fmt::Display for ExecPolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub enum CommandKind<'a> {
    Shell(&'a str),
    Argv(&'a [String]),
}

pub struct CommandPolicyInput<'a> {
    pub source: ExecSource,
    pub command: CommandKind<'a>,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub chat_mode: Option<ChatMode>,
    pub escalation: Option<ExecEscalation>,
}

pub async fn build_exec_request(
    gcx: Arc<GlobalContext>,
    input: CommandPolicyInput<'_>,
) -> Result<ExecRequestPolicy, ExecPolicyError> {
    let command = match input.command {
        CommandKind::Shell(_) if input.source == ExecSource::ReviewEvidence => {
            return Err(policy_input_error(
                "Review evidence commands require argv",
                input.source,
            ));
        }
        CommandKind::Shell(command) => {
            if command.trim().is_empty() {
                return Err(policy_input_error("Command is empty", input.source));
            }
            ExecSpawnRequest::new(ExecMode::Foreground, command)
        }
        CommandKind::Argv(argv) => {
            if argv.first().is_none_or(|program| program.trim().is_empty()) {
                return Err(policy_input_error("Command argv is empty", input.source));
            }
            ExecSpawnRequest::argv(ExecMode::Foreground, argv.to_vec())
        }
    };
    let sandbox_mode =
        requested_sandbox_mode(input.chat_mode.as_deref(), input.escalation.as_ref());
    let status = refact_sandbox::sandbox_status();
    let roots = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    let cwd = input.cwd.clone().unwrap_or_else(|| {
        roots
            .first()
            .cloned()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir()))
    });
    let sandbox_policy = resolve_sandbox_policy(
        gcx.terminal_security_config.mode,
        sandbox_mode,
        status,
        &cwd,
        roots,
        input.source,
    )?;
    let justification = input
        .escalation
        .as_ref()
        .map(|escalation| escalation.justification.clone());
    let request = command
        .with_env_map(input.env)
        .with_env_policy(ExecEnvPolicy::Scrubbed {
            passthrough: gcx.terminal_security_config.env_passthrough.clone(),
        })
        .with_audit(ExecAuditMeta {
            source: input.source.audit_source().to_string(),
            justification,
        });
    let request = if let Some(cwd) = input.cwd {
        request.with_cwd(cwd)
    } else {
        request
    };
    let request = if let Some(sandbox) = sandbox_policy.sandbox {
        request.with_sandbox(sandbox)
    } else {
        request
    };
    Ok(ExecRequestPolicy {
        request,
        warning: sandbox_policy.warning,
        audit: sandbox_policy.audit,
    })
}

fn policy_input_error(message: &str, _source: ExecSource) -> ExecPolicyError {
    ExecPolicyError {
        message: message.to_string(),
        audit: None,
    }
}

#[derive(Debug)]
struct SandboxPolicyResolution {
    sandbox: Option<ExecSandboxSpec>,
    warning: Option<String>,
    audit: Option<SandboxAuditRecord>,
}

fn requested_sandbox_mode(
    chat_mode: Option<&str>,
    escalation: Option<&ExecEscalation>,
) -> ExecSandboxMode {
    if let Some(escalation) = escalation {
        return match escalation.mode {
            ExecEscalationMode::WorkspaceWrite => ExecSandboxMode::WorkspaceWrite,
            ExecEscalationMode::FullAccess => ExecSandboxMode::FullAccess,
        };
    }
    match chat_mode.unwrap_or_default().to_ascii_lowercase().as_str() {
        "explore" | "no_tools" | "no-tools" | "chat" => ExecSandboxMode::ReadOnly,
        _ => ExecSandboxMode::WorkspaceWrite,
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

fn sandbox_spec(
    mode: ExecSandboxMode,
    cwd: &std::path::Path,
    mut workspace_roots: Vec<PathBuf>,
) -> ExecSandboxSpec {
    let rw_paths = match mode {
        ExecSandboxMode::ReadOnly => Vec::new(),
        ExecSandboxMode::WorkspaceWrite => {
            if !workspace_roots.contains(&cwd.to_path_buf()) {
                workspace_roots.push(cwd.to_path_buf());
            }
            if !workspace_roots.contains(&std::env::temp_dir()) {
                workspace_roots.push(std::env::temp_dir());
            }
            workspace_roots
        }
        ExecSandboxMode::FullAccess => vec![PathBuf::from("/")],
    };
    ExecSandboxSpec {
        mode,
        ro_paths: vec![PathBuf::from("/")],
        rw_paths,
        allow_network: true,
    }
}

fn resolve_sandbox_policy(
    mode: TerminalSecurityMode,
    sandbox_mode: ExecSandboxMode,
    status: refact_sandbox::SandboxStatus,
    cwd: &std::path::Path,
    workspace_roots: Vec<PathBuf>,
    source: ExecSource,
) -> Result<SandboxPolicyResolution, ExecPolicyError> {
    let mode_name = terminal_mode_name(mode);
    let provider = status.provider.to_string();
    let probed_enforcement = enforcement_name(status.enforcement);
    let audit = |enforcement: &str, content: String| SandboxAuditRecord {
        mode: mode_name.to_string(),
        provider: provider.clone(),
        enforcement: enforcement.to_string(),
        source: source.audit_source().to_string(),
        content,
    };
    let spec = || sandbox_spec(sandbox_mode.clone(), cwd, workspace_roots.clone());

    match mode {
        TerminalSecurityMode::Off | TerminalSecurityMode::ApprovalOnly => {
            Ok(SandboxPolicyResolution {
                sandbox: None,
                warning: None,
                audit: None,
            })
        }
        TerminalSecurityMode::Audit => Ok(SandboxPolicyResolution {
            sandbox: None,
            warning: None,
            audit: Some(audit(
                "unconfined",
                format!(
                    "Sandbox audit: {} would use {provider} ({probed_enforcement}); command ran unconfined",
                    sandbox_mode_name(&sandbox_mode)
                ),
            )),
        }),
        TerminalSecurityMode::SandboxPreferred
            if status.enforcement != refact_sandbox::Enforcement::Full =>
        {
            let warning = format!(
                "⚠️ Sandbox preferred but full enforcement is unavailable ({provider}: {probed_enforcement}); command ran unconfined."
            );
            Ok(SandboxPolicyResolution {
                sandbox: None,
                warning: Some(warning.clone()),
                audit: Some(audit("unconfined", warning)),
            })
        }
        TerminalSecurityMode::SandboxRequired
            if status.enforcement == refact_sandbox::Enforcement::Unusable =>
        {
            if sandbox_mode == ExecSandboxMode::FullAccess {
                let warning = format!(
                    "⚠️ Approved full-access escalation bypassed unavailable sandbox enforcement ({provider}: {probed_enforcement}); command ran unconfined."
                );
                return Ok(SandboxPolicyResolution {
                    sandbox: None,
                    warning: Some(warning.clone()),
                    audit: Some(audit("unconfined", warning)),
                });
            }
            let message = format!(
                "sandbox required but unavailable ({provider}: no usable sandbox provider is available) — retry with escalate:{{...}} or ask the user to install bubblewrap"
            );
            Err(ExecPolicyError {
                message: message.clone(),
                audit: Some(audit("refused", message)),
            })
        }
        TerminalSecurityMode::SandboxPreferred | TerminalSecurityMode::SandboxRequired => {
            Ok(SandboxPolicyResolution {
                sandbox: Some(spec()),
                warning: None,
                audit: None,
            })
        }
    }
}

fn sandbox_mode_name(mode: &ExecSandboxMode) -> &'static str {
    match mode {
        ExecSandboxMode::ReadOnly => "read_only",
        ExecSandboxMode::WorkspaceWrite => "workspace_write",
        ExecSandboxMode::FullAccess => "full_access",
    }
}

pub fn escalation_from_args(
    args: &HashMap<String, serde_json::Value>,
) -> Result<Option<ExecEscalation>, String> {
    let Some(value) = args.get("escalate") else {
        return Ok(None);
    };
    serde_json::from_value::<ExecEscalation>(value.clone())
        .map_err(|error| format!("invalid escalate argument: {error}"))?
        .validate()
        .map(Some)
}

pub fn escalation_audit(escalation: &ExecEscalation, source: ExecSource) -> SandboxAuditRecord {
    let status = refact_sandbox::sandbox_status();
    SandboxAuditRecord {
        mode: escalation.mode_name().to_string(),
        provider: status.provider.to_string(),
        enforcement: enforcement_name(status.enforcement).to_string(),
        source: source.audit_source().to_string(),
        content: format!(
            "Sandbox escalation requested: {} — {}",
            escalation.mode_name(),
            escalation.justification
        ),
    }
}

pub async fn queue_sandbox_audit(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    audit: SandboxAuditRecord,
) {
    let session = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    if let Some(session) = session {
        session
            .lock()
            .await
            .queue_post_tool_side_effect(audit.message());
    }
}

pub async fn chat_mode_for_exec(gcx: Arc<GlobalContext>, chat_id: &str) -> Option<ChatMode> {
    let session = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    }?;
    let mode = session.lock().await.thread.mode.clone();
    if mode.is_empty() {
        None
    } else {
        Some(mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(
        provider: &'static str,
        enforcement: refact_sandbox::Enforcement,
    ) -> refact_sandbox::SandboxStatus {
        refact_sandbox::SandboxStatus {
            provider,
            enforcement,
        }
    }

    #[tokio::test]
    async fn policy_sets_scrubbed_environment_and_each_audit_source() {
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::get_mut(&mut gcx)
            .unwrap()
            .terminal_security_config
            .env_passthrough = vec!["HTTP_PROXY".to_string()];
        let sources = [
            (ExecSource::ShellTool, "shell_tool"),
            (ExecSource::ProcessTool, "process_tool"),
            (ExecSource::CmdlineIntegration, "cmdline_integration"),
            (ExecSource::SchedulerJob, "scheduler_job"),
            (ExecSource::Verifier, "verifier"),
            (ExecSource::ReviewEvidence, "review_evidence"),
        ];

        for (source, expected) in sources {
            let argv = vec!["printf".to_string(), "ok".to_string()];
            let request = build_exec_request(
                gcx.clone(),
                CommandPolicyInput {
                    source,
                    command: CommandKind::Argv(&argv),
                    cwd: None,
                    env: HashMap::new(),
                    chat_mode: None,
                    escalation: None,
                },
            )
            .await
            .unwrap();

            assert_eq!(
                request.request.env_policy,
                ExecEnvPolicy::Scrubbed {
                    passthrough: vec!["HTTP_PROXY".to_string()]
                }
            );
            assert_eq!(request.request.audit.unwrap().source, expected);
            assert!(request.request.sandbox.is_none());
        }
    }

    #[tokio::test]
    async fn policy_preserves_argv_and_cwd() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let argv = vec!["printf".to_string(), "space value".to_string()];
        let cwd = PathBuf::from("workspace");

        let request = build_exec_request(
            gcx,
            CommandPolicyInput {
                source: ExecSource::Verifier,
                command: CommandKind::Argv(&argv),
                cwd: Some(cwd.clone()),
                env: HashMap::new(),
                chat_mode: None,
                escalation: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(request.request.argv, Some(argv));
        assert_eq!(request.request.cwd, Some(cwd));
    }

    #[tokio::test]
    async fn review_evidence_policy_rejects_shell_strings() {
        let gcx = crate::global_context::tests::make_test_gcx().await;

        let error = build_exec_request(
            gcx,
            CommandPolicyInput {
                source: ExecSource::ReviewEvidence,
                command: CommandKind::Shell("cargo test; rm -rf workspace"),
                cwd: None,
                env: HashMap::new(),
                chat_mode: None,
                escalation: None,
            },
        )
        .await
        .err()
        .unwrap();

        assert_eq!(error.message, "Review evidence commands require argv");
    }

    #[test]
    fn sandbox_rollout_mode_matrix_is_honest() {
        let cwd = PathBuf::from("/workspace");
        let roots = vec![cwd.clone()];
        for mode in [
            TerminalSecurityMode::Off,
            TerminalSecurityMode::ApprovalOnly,
        ] {
            for enforcement in [
                refact_sandbox::Enforcement::Full,
                refact_sandbox::Enforcement::Partial,
                refact_sandbox::Enforcement::Unusable,
            ] {
                let resolution = resolve_sandbox_policy(
                    mode,
                    ExecSandboxMode::WorkspaceWrite,
                    status("test", enforcement),
                    &cwd,
                    roots.clone(),
                    ExecSource::ShellTool,
                )
                .unwrap();
                assert!(resolution.sandbox.is_none());
                assert!(resolution.warning.is_none());
            }
        }

        let audit = resolve_sandbox_policy(
            TerminalSecurityMode::Audit,
            ExecSandboxMode::ReadOnly,
            status("landlock", refact_sandbox::Enforcement::Partial),
            &cwd,
            roots.clone(),
            ExecSource::ShellTool,
        )
        .unwrap();
        assert!(audit.sandbox.is_none());
        assert_eq!(audit.audit.unwrap().enforcement, "unconfined");

        let preferred_full = resolve_sandbox_policy(
            TerminalSecurityMode::SandboxPreferred,
            ExecSandboxMode::WorkspaceWrite,
            status("bwrap", refact_sandbox::Enforcement::Full),
            &cwd,
            roots.clone(),
            ExecSource::ShellTool,
        )
        .unwrap();
        assert!(preferred_full.sandbox.is_some());
        assert!(preferred_full.warning.is_none());

        for enforcement in [
            refact_sandbox::Enforcement::Partial,
            refact_sandbox::Enforcement::Unusable,
        ] {
            let preferred = resolve_sandbox_policy(
                TerminalSecurityMode::SandboxPreferred,
                ExecSandboxMode::WorkspaceWrite,
                status("landlock", enforcement),
                &cwd,
                roots.clone(),
                ExecSource::ShellTool,
            )
            .unwrap();
            assert!(preferred.sandbox.is_none());
            assert!(preferred.warning.unwrap().contains("ran unconfined"));
        }

        for enforcement in [
            refact_sandbox::Enforcement::Full,
            refact_sandbox::Enforcement::Partial,
        ] {
            let required = resolve_sandbox_policy(
                TerminalSecurityMode::SandboxRequired,
                ExecSandboxMode::WorkspaceWrite,
                status("provider", enforcement),
                &cwd,
                roots.clone(),
                ExecSource::ShellTool,
            )
            .unwrap();
            assert!(required.sandbox.is_some());
        }

        let required_error = resolve_sandbox_policy(
            TerminalSecurityMode::SandboxRequired,
            ExecSandboxMode::WorkspaceWrite,
            status("noop", refact_sandbox::Enforcement::Unusable),
            &cwd,
            roots,
            ExecSource::ShellTool,
        )
        .unwrap_err();
        assert!(required_error.message.contains("sandbox required"));
        assert_eq!(required_error.audit.unwrap().enforcement, "refused");
    }

    #[test]
    fn chat_mode_defaults_and_escalation_select_expected_sandbox() {
        assert_eq!(
            requested_sandbox_mode(Some("explore"), None),
            ExecSandboxMode::ReadOnly
        );
        assert_eq!(
            requested_sandbox_mode(Some("task_agent"), None),
            ExecSandboxMode::WorkspaceWrite
        );
        let escalation = ExecEscalation {
            mode: ExecEscalationMode::FullAccess,
            justification: "write outside the workspace".to_string(),
        };
        assert_eq!(
            requested_sandbox_mode(Some("explore"), Some(&escalation)),
            ExecSandboxMode::FullAccess
        );
        let spec = sandbox_spec(
            ExecSandboxMode::FullAccess,
            std::path::Path::new("/workspace"),
            vec![PathBuf::from("/workspace")],
        );
        assert_eq!(spec.rw_paths, vec![PathBuf::from("/")]);
    }

    #[test]
    fn sandbox_audit_message_uses_existing_system_notice_role() {
        let audit = SandboxAuditRecord {
            mode: "sandbox_preferred".to_string(),
            provider: "landlock".to_string(),
            enforcement: "unconfined".to_string(),
            source: "shell_tool".to_string(),
            content: "command ran unconfined".to_string(),
        };

        let message = audit.message();
        assert_eq!(message.role, "event");
        assert_eq!(message.extra["event"]["subkind"], "system_notice");
        assert_eq!(message.extra["event"]["source"], "exec.sandbox");
        assert_eq!(message.extra["event"]["payload"]["provider"], "landlock");
    }

    #[test]
    fn approved_full_access_can_bypass_unavailable_required_sandbox_truthfully() {
        let resolution = resolve_sandbox_policy(
            TerminalSecurityMode::SandboxRequired,
            ExecSandboxMode::FullAccess,
            status("noop", refact_sandbox::Enforcement::Unusable),
            std::path::Path::new("/workspace"),
            vec![PathBuf::from("/workspace")],
            ExecSource::ShellTool,
        )
        .unwrap();

        assert!(resolution.sandbox.is_none());
        assert!(resolution.warning.unwrap().contains("ran unconfined"));
        assert_eq!(resolution.audit.unwrap().enforcement, "unconfined");
    }
}
