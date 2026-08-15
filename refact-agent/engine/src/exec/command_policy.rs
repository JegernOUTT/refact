use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::exec::{ExecAuditMeta, ExecEnvPolicy, ExecMode, ExecSpawnRequest};
use crate::global_context::GlobalContext;

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
    fn audit_source(self) -> &'static str {
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
}

pub async fn build_exec_request(
    gcx: Arc<GlobalContext>,
    input: CommandPolicyInput<'_>,
) -> Result<ExecSpawnRequest, String> {
    let command = match input.command {
        CommandKind::Shell(command) => {
            if command.trim().is_empty() {
                return Err("Command is empty".to_string());
            }
            ExecSpawnRequest::new(ExecMode::Foreground, command)
        }
        CommandKind::Argv(argv) => {
            if argv.first().is_none_or(|program| program.trim().is_empty()) {
                return Err("Command argv is empty".to_string());
            }
            ExecSpawnRequest::argv(ExecMode::Foreground, argv.to_vec())
        }
    };
    let _ = input.chat_mode;
    let request = command
        .with_env_map(input.env)
        .with_env_policy(ExecEnvPolicy::Scrubbed {
            passthrough: gcx.terminal_security_config.env_passthrough.clone(),
        })
        .with_audit(ExecAuditMeta {
            source: input.source.audit_source().to_string(),
            justification: None,
        });
    let request = if let Some(cwd) = input.cwd {
        request.with_cwd(cwd)
    } else {
        request
    };
    // SEC-05 hook
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let request = build_exec_request(
                gcx.clone(),
                CommandPolicyInput {
                    source,
                    command: CommandKind::Shell("printf ok"),
                    cwd: None,
                    env: HashMap::new(),
                    chat_mode: None,
                },
            )
            .await
            .unwrap();

            assert_eq!(
                request.env_policy,
                ExecEnvPolicy::Scrubbed {
                    passthrough: vec!["HTTP_PROXY".to_string()]
                }
            );
            assert_eq!(request.audit.unwrap().source, expected);
            assert!(request.sandbox.is_none());
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
            },
        )
        .await
        .unwrap();

        assert_eq!(request.argv, Some(argv));
        assert_eq!(request.cwd, Some(cwd));
    }
}
