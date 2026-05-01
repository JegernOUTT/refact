use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::tools::tools_description::{
    json_schema_from_params, Tool, ToolDesc, ToolSource, ToolSourceType,
};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{MergeWorktreeRequest, MergeWorktreeResponse, WorktreeMergeStrategy};

fn strategy_from_arg(strategy: Option<&str>) -> Result<WorktreeMergeStrategy, String> {
    match strategy.unwrap_or("squash") {
        "merge" => Ok(WorktreeMergeStrategy::Merge),
        "squash" => Ok(WorktreeMergeStrategy::Squash),
        other => Err(format!(
            "Invalid strategy '{}', must be 'merge' or 'squash'",
            other
        )),
    }
}

fn bool_arg(args: &HashMap<String, Value>, name: &str, default: bool) -> bool {
    args.get(name)
        .and_then(|value| value.as_bool())
        .unwrap_or(default)
}

fn string_arg(args: &HashMap<String, Value>, name: &str) -> Option<String> {
    args.get(name)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn cleanup_summary(response: &MergeWorktreeResponse) -> String {
    match response.cleanup.as_ref() {
        Some(cleanup) => format!(
            "worktree_deleted={}, branch_deleted={}, registry_deleted={}, affected_references={}",
            cleanup.worktree_deleted,
            cleanup.branch_deleted,
            cleanup.registry_deleted,
            response.affected_reference_count
        ),
        None => format!(
            "no cleanup requested, affected_references={}",
            response.affected_reference_count
        ),
    }
}

fn merge_response_message(response: &MergeWorktreeResponse) -> String {
    if let Some(conflict) = response.conflict.as_ref() {
        let files = if conflict.files.is_empty() {
            "None detected".to_string()
        } else {
            conflict
                .files
                .iter()
                .map(|file| format!("- {}", file))
                .collect::<Vec<_>>()
                .join("\n")
        };
        return format!(
            "# Worktree Merge Conflicts\n\n**Worktree:** {}\n**Branch:** {} → {}\n**Strategy:** {}\n**Aborted:** {}\n\n## Conflicting Files\n{}\n\n{}",
            response.id,
            response.source_branch,
            response.target_branch,
            response.strategy,
            conflict.aborted,
            files,
            conflict.instructions
        );
    }

    if response.status == "nothing_to_merge" {
        return format!(
            "# Nothing to Merge\n\n**Worktree:** {}\n**Branch:** {} → {}\n\nCleanup: {}.",
            response.id,
            response.source_branch,
            response.target_branch,
            cleanup_summary(response)
        );
    }

    format!(
        "# Worktree Merged\n\n**Worktree:** {}\n**Strategy:** {}\n**Branch:** {} → {}\n**Merge commit:** {}\n**Cleanup:** {}\n\nThe worktree changes have been merged into the target branch.",
        response.id,
        response.strategy,
        response.source_branch,
        response.target_branch,
        response.merge_commit.as_deref().unwrap_or("unknown"),
        cleanup_summary(response)
    )
}

async fn service_from_gcx(
    gcx: Arc<tokio::sync::RwLock<GlobalContext>>,
    requested_source_root: Option<String>,
) -> Result<WorktreeService, String> {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let project_dirs = crate::files_correction::get_project_dirs(gcx).await;
    if project_dirs.is_empty() {
        return Err("No project root available".to_string());
    }
    let source_root = if let Some(requested) = requested_source_root {
        let requested = std::path::PathBuf::from(requested);
        let requested = std::fs::canonicalize(&requested).map_err(|e| {
            format!(
                "Failed to resolve source workspace root '{}': {}",
                requested.display(),
                e
            )
        })?;
        let requested = dunce::simplified(&requested).to_path_buf();
        let matches = project_dirs.iter().any(|dir| {
            std::fs::canonicalize(dir)
                .map(|canonical| dunce::simplified(&canonical).to_path_buf() == requested)
                .unwrap_or(false)
        });
        if !matches {
            return Err("Worktree source root is not a current workspace directory".to_string());
        }
        requested
    } else {
        project_dirs[0].clone()
    };
    WorktreeService::new(cache_dir, source_root)
}

pub struct ToolWorktreeMerge;

impl ToolWorktreeMerge {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolWorktreeMerge {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "worktree_merge".to_string(),
            display_name: "Merge Worktree".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Merge a registered worktree back into its target branch. Generates a commit message from the worktree diff unless commit_message is provided. By default uses squash merge and deletes the worktree/branch after successful merge or no-op cleanup.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("worktree_id", "string", "Registered worktree id to merge. Defaults to the active chat worktree when omitted."),
                    ("source_workspace_root", "string", "Source workspace root for repositories with multiple workspace folders."),
                    ("target_branch", "string", "Target branch to merge into. Defaults to the worktree base branch."),
                    ("strategy", "string", "Merge strategy: 'squash' (default) or 'merge'."),
                    ("commit_message", "string", "Optional commit message override. If omitted, Refact generates one from the diff."),
                    ("delete_after_merge", "boolean", "Delete the worktree and source branch after a successful merge or nothing-to-merge cleanup. Defaults to true."),
                    ("include_uncommitted", "boolean", "Auto-commit dirty worktree changes before merging. Defaults to false."),
                ],
                &[],
            ),
            output_schema: None,
            annotations: Some(serde_json::json!({
                "destructiveHint": true,
                "idempotentHint": false,
                "openWorldHint": false,
            })),
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, active_worktree) = {
            let ccx = ccx.lock().await;
            (ccx.global_context.clone(), ccx.execution_scope_worktree())
        };

        let worktree_id = string_arg(args, "worktree_id")
            .or_else(|| active_worktree.as_ref().map(|worktree| worktree.id.clone()))
            .ok_or_else(|| {
                "Missing 'worktree_id' and this chat has no active worktree".to_string()
            })?;
        let source_workspace_root = string_arg(args, "source_workspace_root").or_else(|| {
            active_worktree
                .as_ref()
                .map(|worktree| worktree.source_workspace_root.to_string_lossy().to_string())
        });
        let strategy = strategy_from_arg(args.get("strategy").and_then(|value| value.as_str()))?;
        let delete_after_merge = bool_arg(args, "delete_after_merge", true);
        let include_uncommitted = bool_arg(args, "include_uncommitted", false);
        let target_branch = string_arg(args, "target_branch");

        let service = service_from_gcx(gcx.clone(), source_workspace_root).await?;
        let commit_message = match string_arg(args, "commit_message") {
            Some(message) => Some(message),
            None => {
                let diff = service.diff_worktree(&worktree_id).await?;
                match crate::agentic::generate_commit_message::generate_commit_message_by_diff(
                    gcx.clone(),
                    &diff.patch,
                    &target_branch,
                )
                .await
                {
                    Ok(message) if !message.trim().is_empty() => Some(message),
                    _ => None,
                }
            }
        };

        let response = service
            .merge_worktree(
                &worktree_id,
                MergeWorktreeRequest {
                    strategy,
                    delete_after_merge,
                    include_uncommitted,
                    target_branch,
                    commit_message,
                    generate_commit_message: false,
                },
            )
            .await?;

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(merge_response_message(&response)),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }
}
