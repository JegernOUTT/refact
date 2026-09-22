pub use refact_agentic::generate_commit_message::remove_fencing;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::files_correction::CommandSimplifiedDirExt;
use crate::global_context::GlobalContext;
use crate::subchat::{run_subchat_once_with_abort, TraceParent};
use crate::yaml_configs::customization_registry::get_subagent_config;
use hashbrown::HashMap;
use tracing::warn;
use crate::files_in_workspace::detect_vcs_for_a_file_path;

const SUBAGENT_ID: &str = "commit_message";

fn commit_message_input_fingerprint(diff: &str, prompt: &Option<String>) -> String {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    digest.update(b"commit-message-proposal-v1");
    digest.update((diff.len() as u64).to_be_bytes());
    digest.update(diff.as_bytes());
    match prompt {
        Some(prompt) => {
            digest.update([1]);
            digest.update((prompt.len() as u64).to_be_bytes());
            digest.update(prompt.as_bytes());
        }
        None => digest.update([0]),
    }
    format!("sha256:{:x}", digest.finalize())
}

fn commit_message_report(
    result: &Result<String, String>,
) -> crate::buddy::workflows::WorkflowReport {
    use crate::buddy::outcome_runtime::{terminal_from_workflow_result, WorkflowTerminal};

    let (body, terminal) = match result {
        Ok(message) if !message.trim().is_empty() => (message.clone(), WorkflowTerminal::Completed),
        Ok(_) => (
            "No readable commit-message proposal was generated.".to_string(),
            WorkflowTerminal::Inconclusive,
        ),
        Err(error) => (
            crate::buddy::workflows::workflow_failure_report(SUBAGENT_ID, error, None).summary,
            terminal_from_workflow_result(&Err(error.clone())),
        ),
    };
    crate::buddy::workflows::WorkflowReport {
        title: "Commit-message proposal".to_string(),
        body,
        terminal,
        evidence: Vec::new(),
    }
}

pub async fn generate_commit_message_by_diff(
    gcx: Arc<GlobalContext>,
    diff: &String,
    commit_message_prompt: &Option<String>,
    parent_chat_id: Option<&str>,
) -> Result<String, String> {
    generate_commit_message_by_diff_with_abort(
        gcx,
        diff,
        commit_message_prompt,
        None,
        parent_chat_id,
    )
    .await
}

pub async fn generate_commit_message_by_diff_with_abort(
    gcx: Arc<GlobalContext>,
    diff: &String,
    commit_message_prompt: &Option<String>,
    abort_flag: Option<Arc<AtomicBool>>,
    parent_chat_id: Option<&str>,
) -> Result<String, String> {
    if diff.is_empty() {
        return Err("The provided diff is empty".to_string());
    }
    let diff = diff.clone();
    let commit_message_prompt = commit_message_prompt.clone();
    let gcx2 = gcx.clone();
    let parent_chat_id = parent_chat_id.map(|id| id.to_string());
    let input_fingerprint = commit_message_input_fingerprint(&diff, &commit_message_prompt);
    crate::buddy::workflows::buddy_wrap_workflow_report(
        crate::app_state::AppState::from_gcx(gcx).await,
        "commit_message",
        "📦",
        5,
        |msg: &String| {
            let short: String = msg.lines().next().unwrap_or("").chars().take(50).collect();
            format!("Commit message: {}", short)
        },
        crate::buddy::workflows::WorkflowReportScope {
            observed_scope: "Commit-message proposal generation only from the supplied diff and custom prompt; no commit created, no changes applied, and no verification performed.".into(),
            input_fingerprint,
        },
        commit_message_report,
        move || async move {
            let subagent_config = get_subagent_config(gcx2.clone(), SUBAGENT_ID, None)
                .await
                .ok_or_else(|| format!("subagent config '{}' not found", SUBAGENT_ID))?;

            let messages = if let Some(text) = commit_message_prompt {
                let system_prompt = subagent_config
                    .prompts
                    .diff_with_user_text
                    .as_ref()
                    .ok_or_else(|| {
                        format!(
                            "prompts.diff_with_user_text not defined for subagent '{}'",
                            SUBAGENT_ID
                        )
                    })?;
                vec![
                    ChatMessage {
                        role: "system".to_string(),
                        content: ChatContent::SimpleText(system_prompt.clone()),
                        ..Default::default()
                    },
                    ChatMessage {
                        role: "user".to_string(),
                        content: ChatContent::SimpleText(format!(
                            "Commit message:\n```\n{}\n```\nDiff:\n```\n{}\n```\n",
                            text, diff
                        )),
                        ..Default::default()
                    },
                ]
            } else {
                let system_prompt =
                    subagent_config.prompts.diff_only.as_ref().ok_or_else(|| {
                        format!(
                            "prompts.diff_only not defined for subagent '{}'",
                            SUBAGENT_ID
                        )
                    })?;
                vec![
                    ChatMessage {
                        role: "system".to_string(),
                        content: ChatContent::SimpleText(system_prompt.clone()),
                        ..Default::default()
                    },
                    ChatMessage {
                        role: "user".to_string(),
                        content: ChatContent::SimpleText(format!("Diff:\n```\n{}\n```\n", diff)),
                        ..Default::default()
                    },
                ]
            };
            let result = run_subchat_once_with_abort(
                gcx2,
                SUBAGENT_ID,
                messages,
                abort_flag,
                TraceParent::from_parts(parent_chat_id.as_deref(), None),
            )
            .await
            .map_err(|e| format!("Error: {}", e))?;

            if result.aborted {
                return Err("Commit-message generation cancelled".into());
            }
            let commit_message = result
                .messages
                .last()
                .filter(|message| message.role == "assistant")
                .and_then(|last_m| match &last_m.content {
                    ChatContent::SimpleText(text) => Some(text.clone()),
                    _ => None,
                })
                .ok_or("No commit message was generated".to_string())?;

            let code_blocks = remove_fencing(&commit_message);
            let proposal = if !code_blocks.is_empty() {
                code_blocks[0].clone()
            } else {
                commit_message
            };
            if proposal.trim().is_empty() {
                Err("No readable commit-message proposal was generated".into())
            } else {
                Ok(proposal)
            }
        },
    )
    .await
}

pub async fn _generate_commit_message_for_projects(
    gcx: Arc<GlobalContext>,
) -> Result<HashMap<PathBuf, String>, String> {
    let project_folders = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    let mut commit_messages = HashMap::new();

    for folder in project_folders {
        let (command, command_args): (&str, &[&str]) = if let Some((_, vcs_type)) =
            detect_vcs_for_a_file_path(&folder).await
        {
            match vcs_type {
                "git" => ("git", &["diff"]),
                "svn" => ("svn", &["diff"]),
                "hg" => ("hg", &["diff"]),
                other => {
                    warn!("Unrecognizable version control detected for the folder {folder:?}: {other}");
                    continue;
                }
            }
        } else {
            warn!("There's no recognizable version control detected for the folder {folder:?}");
            continue;
        };

        let output = tokio::process::Command::new(command)
            .args(command_args)
            .current_dir_simplified(&folder)
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .map_err(|e| format!("Failed to execute command for folder {folder:?}: {e}"))?;

        if !output.status.success() {
            warn!(
                "Command failed for folder {folder:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            continue;
        }

        let diff_output = String::from_utf8_lossy(&output.stdout).to_string();
        let commit_message =
            generate_commit_message_by_diff(gcx.clone(), &diff_output, &None, None).await?;
        commit_messages.insert(folder, commit_message);
    }

    Ok(commit_messages)
}

#[cfg(test)]
mod operator_report_tests {
    use super::*;
    use crate::buddy::outcome_runtime::WorkflowTerminal;

    #[test]
    fn proposal_fingerprint_binds_exact_diff_and_optional_prompt() {
        let original = commit_message_input_fingerprint("diff", &None);
        assert_eq!(original, commit_message_input_fingerprint("diff", &None));
        assert_ne!(original, commit_message_input_fingerprint("diff\n", &None));
        assert_ne!(
            original,
            commit_message_input_fingerprint("diff", &Some(String::new()))
        );
        assert_ne!(
            commit_message_input_fingerprint("ab", &Some("c".into())),
            commit_message_input_fingerprint("a", &Some("bc".into())),
        );
        assert!(original.starts_with("sha256:"));
    }

    #[test]
    fn completed_proposal_retains_body_without_claiming_a_commit() {
        let message = "Subject\n\nFull proposal body".to_string();
        let report = commit_message_report(&Ok(message.clone()));
        assert_eq!(report.terminal, WorkflowTerminal::Completed);
        assert_eq!(report.body, message);
        assert_eq!(report.title, "Commit-message proposal");
        assert!(report.evidence.is_empty());
        assert_eq!(
            commit_message_report(&Ok(" \n".into())).terminal,
            WorkflowTerminal::Inconclusive
        );
        let cancelled = commit_message_report(&Err("cancelled token=private-token".into()));
        assert!(matches!(
            cancelled.terminal,
            WorkflowTerminal::Cancelled { .. }
        ));
        assert!(!cancelled.body.contains("private-token"));
    }
}
