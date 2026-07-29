use std::sync::Arc;
use std::collections::HashMap;
use serde_json::Value;
use tracing::warn;
use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::return_one_candidate_or_a_good_error;
use crate::at_commands::at_tree::{tree_for_tools_ex, BuildBudget, TreeNode, MAX_TREE_PATHS};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::files_correction::{
    correct_to_nearest_dir_path, correct_to_nearest_filename, get_unscoped_project_dirs,
    paths_from_anywhere,
};
use crate::files_in_workspace::{filter_privacy_allowed_files, ls_files_limited};
use crate::knowledge_index::format_related_memories_section;
use crate::tools::scope_utils::{
    format_scope_notices, is_worktree_root_alias, list_execution_scope_root_limited,
    list_scoped_files_under_dir_limited, resolve_existing_path_with_execution_scope,
};

pub struct ToolTree {
    pub config_path: String,
}

fn preformat_path(path: &String) -> String {
    if path == "/" || path == "\\" {
        return path.clone();
    }
    path.trim_end_matches(&['/', '\\'][..]).to_string()
}

#[async_trait]
impl Tool for ToolTree {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "tree".to_string(),
            display_name: "Tree".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Get a files tree for the project. Shows file sizes and line counts. Folders with many files are truncated (controlled by max_files). Hidden folders, __pycache__, node_modules, and binary files are excluded.".to_string(),
            input_schema: json_schema_from_params(&[("path", "string", "An absolute path to get files tree for. Do not pass it if you need a full project tree."), ("use_ast", "boolean", "If true, for each file an array of AST symbols will appear as well as its filename"), ("max_files", "integer", "Maximum files to show per folder before truncating (default: 10). Root folder is never truncated.")], &[]),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, execution_scope, abort_flag) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.app.gcx.clone(),
                cgcx.execution_scope.clone(),
                cgcx.abort_flag.clone(),
            )
        };
        let mut build_budget = BuildBudget::new(MAX_TREE_PATHS, Some(abort_flag.clone()));
        let scoped_enforced = execution_scope
            .as_ref()
            .map(|scope| scope.is_enforced())
            .unwrap_or(false);
        let paths_from_anywhere = if scoped_enforced {
            vec![]
        } else {
            paths_from_anywhere(gcx.clone()).await
        };

        let path_mb = match args.get("path") {
            Some(Value::String(s)) => Some(preformat_path(s)),
            Some(v) => return Err(format!("argument `path` is not a string: {:?}", v)),
            None => None,
        };
        let path_mb_for_related = path_mb.clone();
        let use_ast = match args.get("use_ast") {
            Some(Value::Null) | None => false,
            Some(value) => refact_tool_api::coerce_bool(value)
                .ok_or_else(|| format!("argument `use_ast` is not a boolean: {:?}", value))?,
        };
        let max_files = match args.get("max_files") {
            Some(Value::Number(n)) => n.as_u64().unwrap_or(10) as usize,
            Some(v) => return Err(format!("argument `max_files` is not an integer: {:?}", v)),
            None => 10,
        };

        let mut scope_notices = vec![];
        let (tree, is_root_query) = if scoped_enforced {
            let scope = execution_scope.as_ref().unwrap();
            match path_mb.clone() {
                Some(path) => {
                    if is_worktree_root_alias(&path) {
                        let listing = list_execution_scope_root_limited(
                            gcx.clone(),
                            scope,
                            true,
                            MAX_TREE_PATHS,
                            Some(&abort_flag),
                        )
                        .await?;
                        build_budget.truncated |= listing.truncated;
                        (
                            TreeNode::build_relative_with_budget(
                                &listing.files,
                                scope.effective_root(),
                                &mut build_budget,
                            ),
                            true,
                        )
                    } else {
                        let resolved = resolve_existing_path_with_execution_scope(
                            gcx.clone(),
                            Some(scope),
                            &path,
                        )
                        .await?
                        .ok_or_else(|| format!("Failed to resolve scoped path '{}'", path))?;
                        scope_notices.extend(resolved.notices);
                        if resolved.path.is_file() {
                            return Err(format!("⚠️ '{}' is a file, not a directory. 💡 Use cat('{}') to read it, or tree() without path for project root", path, path));
                        }
                        if !resolved.path.is_dir() {
                            return Err(format!(
                                "Path '{}' is not a directory",
                                resolved.path.display()
                            ));
                        }
                        let listing = list_scoped_files_under_dir_limited(
                            gcx.clone(),
                            &resolved.path,
                            true,
                            true,
                            MAX_TREE_PATHS,
                            Some(&abort_flag),
                        )
                        .await?;
                        build_budget.truncated |= listing.truncated;
                        (
                            TreeNode::build_relative_with_budget(
                                &listing.files,
                                &resolved.path,
                                &mut build_budget,
                            ),
                            false,
                        )
                    }
                }
                None => {
                    let listing = list_execution_scope_root_limited(
                        gcx.clone(),
                        scope,
                        true,
                        MAX_TREE_PATHS,
                        Some(&abort_flag),
                    )
                    .await?;
                    build_budget.truncated |= listing.truncated;
                    (
                        TreeNode::build_relative_with_budget(
                            &listing.files,
                            scope.effective_root(),
                            &mut build_budget,
                        ),
                        true,
                    )
                }
            }
        } else {
            match path_mb.clone() {
                Some(path) => {
                    let file_candidates =
                        correct_to_nearest_filename(gcx.clone(), &path, false, 10).await;
                    let dir_candidates =
                        correct_to_nearest_dir_path(gcx.clone(), &path, false, 10).await;
                    if dir_candidates.is_empty() && !file_candidates.is_empty() {
                        return Err(format!("⚠️ '{}' is a file, not a directory. 💡 Use cat('{}') to read it, or tree() without path for project root", path, path));
                    }

                    let project_dirs = get_unscoped_project_dirs(gcx.clone()).await;
                    let candidate = return_one_candidate_or_a_good_error(
                        gcx.clone(),
                        &path,
                        &dir_candidates,
                        &project_dirs,
                        true,
                    )
                    .await?;
                    let true_path = crate::files_correction::canonical_path(candidate);

                    let all_project_dirs = get_unscoped_project_dirs(gcx.clone()).await;
                    let is_within_project_dirs =
                        all_project_dirs.iter().any(|p| true_path.starts_with(&p))
                            || project_dirs.iter().any(|p| true_path.starts_with(&p));
                    if !is_within_project_dirs && !gcx.cmdline.inside_container {
                        return Err(format!("⚠️ '{}' is outside project directories. 💡 Use tree() without path to see project root", path));
                    }

                    let indexing_everywhere =
                        crate::files_blocklist::reload_indexing_everywhere_if_needed(gcx.clone())
                            .await;
                    let listing = ls_files_limited(
                        &indexing_everywhere,
                        &true_path,
                        true,
                        MAX_TREE_PATHS,
                        Some(&abort_flag),
                    )
                    .unwrap_or_default();
                    build_budget.truncated |= listing.truncated;
                    let paths_in_dir =
                        filter_privacy_allowed_files(gcx.clone(), listing.files).await;

                    (
                        TreeNode::build_with_budget(&paths_in_dir, &mut build_budget),
                        false,
                    )
                }
                None => (
                    TreeNode::build_with_budget(&paths_from_anywhere, &mut build_budget),
                    true,
                ),
            }
        };

        let content = tree_for_tools_ex(
            ccx.clone(),
            &tree,
            use_ast,
            max_files,
            is_root_query,
            build_budget.truncated,
        )
        .await
        .map_err(|err| {
            warn!("tree_for_tools err: {}", err);
            err
        })?;
        let content = if content.is_empty() {
            "No files found in the specified path.".to_string()
        } else {
            content
        };
        let content = format!("{}{}", format_scope_notices(&scope_notices), content);

        // Append related memories (short form). Since tree() is directory-oriented,
        // we try to surface memories that reference the directory itself via related_files.
        // This keeps the lookup fast (in-memory index) and doesn't require VecDB.
        let related_section = {
            let idx_arc = { gcx.knowledge_index.clone() };
            let idx_guard = idx_arc.lock().await;
            let path_key = path_mb_for_related.clone();
            let mut keys: Vec<String> = Vec::new();
            if let Some(path) = path_key {
                keys.push(path);
            }
            keys.sort();
            keys.dedup();
            let cards = idx_guard.related_for_related_files(&keys, 8);
            format_related_memories_section(&cards, None)
        };

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!("{}{}", content, related_section)),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            })],
        ))
    }
}

#[cfg(test)]
mod privacy_and_bounds_tests {
    use super::*;
    use crate::at_commands::at_commands::AtCommandsContext;
    use crate::call_validation::{ChatContent, ContextEnum};
    use crate::privacy::{FilePrivacySettings, PrivacySettings};
    use crate::worktrees::types::WorktreeMeta;
    use std::fs;
    use std::path::PathBuf;

    struct Fixture {
        _temp: tempfile::TempDir,
        worktree: WorktreeMeta,
        root: PathBuf,
        source: PathBuf,
    }

    fn make_fixture() -> Fixture {
        let temp = tempfile::Builder::new()
            .prefix("refact-tree-privacy-")
            .tempdir()
            .unwrap();
        let root = temp
            .path()
            .join(".cache")
            .join("refact")
            .join("worktrees")
            .join("wt")
            .join("proj");
        let source = temp.path().join("source");
        fs::create_dir_all(root.join("subdir")).unwrap();
        fs::create_dir_all(source.join("subdir")).unwrap();
        fs::write(root.join("subdir").join("allowed.rs"), "fn allowed() {}\n").unwrap();
        fs::write(
            root.join("subdir").join("secret.blocked"),
            "SECRET_TOKEN=abcdef\n",
        )
        .unwrap();
        let root = dunce::simplified(&fs::canonicalize(root).unwrap()).to_path_buf();
        let source = dunce::simplified(&fs::canonicalize(source).unwrap()).to_path_buf();
        let worktree = WorktreeMeta {
            id: "wt-tree-privacy".to_string(),
            kind: "chat".to_string(),
            root: root.clone(),
            source_workspace_root: source.clone(),
            repo_root: source.clone(),
            branch: Some("feature".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: Some("base".to_string()),
            task_id: None,
            card_id: None,
            agent_id: None,
            enforce: true,
        };
        Fixture {
            _temp: temp,
            worktree,
            root,
            source,
        }
    }

    async fn make_gcx(
        fixture: &Fixture,
        blocked: Vec<String>,
    ) -> Arc<crate::global_context::GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            let privacy_settings = gcx.privacy_settings.clone();
            let workspace_folders = gcx.documents_state.workspace_folders.clone();
            *privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
                privacy_rules: FilePrivacySettings {
                    only_send_to_servers_I_control: vec![],
                    blocked,
                },
                loaded_ts: u64::MAX / 2,
            });
            *workspace_folders.lock().unwrap() = vec![fixture.source.clone()];
        }
        gcx
    }

    async fn make_ccx(
        gcx: Arc<crate::global_context::GlobalContext>,
        worktree: WorktreeMeta,
    ) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                crate::app_state::AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                "chat".to_string(),
                None,
                "model".to_string(),
                None,
                Some(worktree),
            )
            .await,
        ))
    }

    fn tool_text(results: &[ContextEnum]) -> String {
        results
            .iter()
            .filter_map(|item| match item {
                ContextEnum::ChatMessage(message) => match &message.content {
                    ChatContent::SimpleText(text) => Some(text.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn scoped_subdir_tree_hides_privacy_blocked_files() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec!["*.blocked".to_string()]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = ToolTree {
            config_path: String::new(),
        };
        let tool_call_id = "tree-call".to_string();
        let subdir = fixture.root.join("subdir").to_string_lossy().to_string();
        let args = HashMap::from_iter([("path".to_string(), Value::String(subdir))]);

        let (_corrections, results) = tool.tool_execute(ccx, &tool_call_id, &args).await.unwrap();
        let text = tool_text(&results);

        assert!(
            text.contains("allowed.rs"),
            "allowed file must be listed: {text}"
        );
        assert!(
            !text.contains("secret.blocked"),
            "privacy-blocked file must not leak into the tree listing: {text}"
        );
        assert!(!text.contains("SECRET_TOKEN"), "{text}");
    }
}
