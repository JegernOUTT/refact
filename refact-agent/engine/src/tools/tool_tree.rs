use std::sync::Arc;
use std::collections::HashMap;
use serde_json::Value;
use tracing::warn;
use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::return_one_candidate_or_a_good_error;
use crate::at_commands::at_tree::{tree_for_tools, TreeNode};
use crate::tools::tools_description::{Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::files_correction::{
    correct_to_nearest_dir_path, correct_to_nearest_filename, get_project_dirs,
    paths_from_anywhere,
};
use crate::files_in_workspace::ls_files;
use crate::knowledge_index::format_related_memories_section;

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
            parameters: vec![
                ToolParam {
                    name: "path".to_string(),
                    description: "An absolute path to get files tree for. Do not pass it if you need a full project tree.".to_string(),
                    param_type: "string".to_string(),
                },
                ToolParam {
                    name: "use_ast".to_string(),
                    description: "If true, for each file an array of AST symbols will appear as well as its filename".to_string(),
                    param_type: "boolean".to_string(),
                },
                ToolParam {
                    name: "max_files".to_string(),
                    description: "Maximum files to show per folder before truncating (default: 10). Root folder is never truncated.".to_string(),
                    param_type: "integer".to_string(),
                },
            ],
            parameters_required: vec![],
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.global_context.clone();
        let paths_from_anywhere = paths_from_anywhere(gcx.clone()).await;

        let path_mb = match args.get("path") {
            Some(Value::String(s)) => Some(preformat_path(s)),
            Some(v) => return Err(format!("argument `path` is not a string: {:?}", v)),
            None => None,
        };
        let path_mb_for_related = path_mb.clone();
        let use_ast = match args.get("use_ast") {
            Some(Value::Bool(b)) => *b,
            Some(v) => return Err(format!("argument `use_ast` is not a boolean: {:?}", v)),
            None => false,
        };
        let max_files = match args.get("max_files") {
            Some(Value::Number(n)) => n.as_u64().unwrap_or(10) as usize,
            Some(v) => return Err(format!("argument `max_files` is not an integer: {:?}", v)),
            None => 10,
        };

        let (tree, is_root_query) = match path_mb.clone() {
            Some(path) => {
                let file_candidates =
                    correct_to_nearest_filename(gcx.clone(), &path, false, 10).await;
                let dir_candidates =
                    correct_to_nearest_dir_path(gcx.clone(), &path, false, 10).await;
                if dir_candidates.is_empty() && !file_candidates.is_empty() {
                    return Err(format!("⚠️ '{}' is a file, not a directory. 💡 Use cat('{}') to read it, or tree() without path for project root", path, path));
                }

                let project_dirs = get_project_dirs(gcx.clone()).await;
                let candidate = return_one_candidate_or_a_good_error(
                    gcx.clone(),
                    &path,
                    &dir_candidates,
                    &project_dirs,
                    true,
                )
                .await?;
                let true_path = crate::files_correction::canonical_path(candidate);

                let all_project_dirs = get_project_dirs(gcx.clone()).await;
                let is_within_project_dirs =
                    all_project_dirs.iter().any(|p| true_path.starts_with(&p))
                        || project_dirs.iter().any(|p| true_path.starts_with(&p));
                if !is_within_project_dirs && !gcx.read().await.cmdline.inside_container {
                    return Err(format!("⚠️ '{}' is outside project directories. 💡 Use tree() without path to see project root", path));
                }

                let indexing_everywhere =
                    crate::files_blocklist::reload_indexing_everywhere_if_needed(gcx.clone()).await;
                let paths_in_dir =
                    ls_files(&indexing_everywhere, &true_path, true).unwrap_or(vec![]);

                (TreeNode::build(&paths_in_dir), false)
            }
            None => (TreeNode::build(&paths_from_anywhere), true),
        };

        let content = tree_for_tools(ccx.clone(), &tree, use_ast, max_files, is_root_query)
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

        // Append related memories (short form). Since tree() is directory-oriented,
        // we try to surface memories that reference the directory itself via related_files.
        // This keeps the lookup fast (in-memory index) and doesn't require VecDB.
        let related_section = {
            let idx_arc = { gcx.read().await.knowledge_index.clone() };
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
