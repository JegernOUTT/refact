use std::path::PathBuf;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::files_correction::CommandSimplifiedDirExt;
use crate::global_context::GlobalContext;
use crate::subchat::run_subchat_once;
use crate::yaml_configs::customization_registry::get_subagent_config;
use std::sync::Arc;
use hashbrown::HashMap;
use tokio::sync::RwLock as ARwLock;
use tracing::warn;
use crate::files_in_workspace::detect_vcs_for_a_file_path;

const SUBAGENT_ID: &str = "commit_message";

pub fn remove_fencing(message: &String) -> Vec<String> {
    let trimmed_message = message.trim();
    if !trimmed_message.contains("```") {
        return Vec::new();
    }
    if trimmed_message.contains("``````") {
        return Vec::new();
    }

    let mut results = Vec::new();
    let mut in_code_block = false;

    for (_i, part) in trimmed_message.split("```").enumerate() {
        if in_code_block {
            let part_lines: Vec<&str> = part.lines().collect();
            if !part_lines.is_empty() {
                let start_idx = if part_lines[0].trim().split_whitespace().count() <= 1
                    && part_lines.len() > 1
                {
                    1
                } else {
                    0
                };
                if start_idx < part_lines.len() {
                    let code_block = part_lines[start_idx..].join("\n");
                    if !code_block.is_empty() {
                        results.push(code_block.trim().to_string());
                    }
                }
            }
        }

        in_code_block = !in_code_block;
    }
    if !results.is_empty() {
        results
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_fencing() {
        let input = "Simple text without fencing".to_string();
        assert_eq!(remove_fencing(&input), Vec::<String>::new());
    }

    #[test]
    fn test_simple_fencing() {
        let input = "```\nCode block\n```".to_string();
        assert_eq!(remove_fencing(&input), vec!["Code block".to_string()]);
    }

    #[test]
    fn test_language_tag() {
        let input = "```rust\nfn main() {\n    println!(\"Hello\");\n}\n```".to_string();
        assert_eq!(
            remove_fencing(&input),
            vec!["fn main() {\n    println!(\"Hello\");\n}".to_string()]
        );
    }

    #[test]
    fn test_text_before_and_after() {
        let input = "Text before\nText before\n```\nCode block\n```\nText after".to_string();
        assert_eq!(remove_fencing(&input), vec!["Code block".to_string()]);
    }

    #[test]
    fn test_multiple_code_blocks() {
        let input = "First paragraph\n```\nFirst code\n```\nMiddle text\n```python\ndef hello():\n    print('world')\n```\nLast paragraph".to_string();
        assert_eq!(
            remove_fencing(&input),
            vec![
                "First code".to_string(),
                "def hello():\n    print('world')".to_string()
            ]
        );
    }

    #[test]
    fn test_empty_code_block() {
        let input = "Text with `````` empty block".to_string();
        assert_eq!(remove_fencing(&input), Vec::<String>::new());
    }
}

pub async fn generate_commit_message_by_diff(
    gcx: Arc<ARwLock<GlobalContext>>,
    diff: &String,
    commit_message_prompt: &Option<String>,
) -> Result<String, String> {
    if diff.is_empty() {
        return Err("The provided diff is empty".to_string());
    }

    let subagent_config = get_subagent_config(gcx.clone(), SUBAGENT_ID, None)
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
        let system_prompt = subagent_config.prompts.diff_only.as_ref().ok_or_else(|| {
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
    let result = run_subchat_once(gcx, SUBAGENT_ID, messages)
        .await
        .map_err(|e| format!("Error: {}", e))?;

    let commit_message = result
        .messages
        .last()
        .and_then(|last_m| match &last_m.content {
            ChatContent::SimpleText(text) => Some(text.clone()),
            _ => None,
        })
        .ok_or("No commit message was generated".to_string())?;

    let code_blocks = remove_fencing(&commit_message);
    if !code_blocks.is_empty() {
        Ok(code_blocks[0].clone())
    } else {
        Ok(commit_message)
    }
}

pub async fn _generate_commit_message_for_projects(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<HashMap<PathBuf, String>, String> {
    let project_folders = gcx
        .read()
        .await
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    let mut commit_messages = HashMap::new();

    for folder in project_folders {
        let command = if let Some((_, vcs_type)) = detect_vcs_for_a_file_path(&folder).await {
            match vcs_type {
                "git" => "git diff",
                "svn" => "svn diff",
                "hg" => "hg diff",
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
            generate_commit_message_by_diff(gcx.clone(), &diff_output, &None).await?;
        commit_messages.insert(folder, commit_message);
    }

    Ok(commit_messages)
}
