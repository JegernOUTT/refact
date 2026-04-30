use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::fs;

use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;
use tracing::warn;

use crate::ast::ast_structs::{AstDB, SymbolType};
use crate::at_commands::at_commands::{AtCommand, AtCommandsContext, AtParam};
use crate::at_commands::at_file::return_one_candidate_or_a_good_error;
use crate::at_commands::execute_at::AtCommandMember;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::files_correction::{correct_to_nearest_dir_path, get_project_dirs, paths_from_anywhere};
use crate::tools::scope_utils::{
    format_scope_notices, list_scoped_files_under_dir, resolve_existing_path_with_execution_scope,
};

const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "svg", "mp3", "mp4", "wav", "avi", "mov",
    "mkv", "flv", "webm", "zip", "tar", "gz", "rar", "7z", "bz2", "xz", "exe", "dll", "so",
    "dylib", "bin", "obj", "o", "a", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "woff",
    "woff2", "ttf", "otf", "eot", "pyc", "pyo", "class", "jar", "war", "db", "sqlite", "sqlite3",
    "lock", "sum",
];

const SKIP_DIRS: &[&str] = &[
    "__pycache__",
    "node_modules",
    ".git",
    ".svn",
    ".hg",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
];

pub struct AtTree {
    pub params: Vec<Box<dyn AtParam>>,
}

impl AtTree {
    pub fn new() -> Self {
        AtTree { params: vec![] }
    }
}

pub struct TreeNode {
    pub children: HashMap<String, TreeNode>,
    pub file_size: Option<u64>,
    pub line_count: Option<usize>,
}

impl TreeNode {
    pub fn new() -> Self {
        TreeNode {
            children: HashMap::new(),
            file_size: None,
            line_count: None,
        }
    }

    pub fn build(paths: &Vec<PathBuf>) -> Self {
        let mut root = TreeNode::new();
        for path in paths {
            if should_skip_path(path) {
                continue;
            }
            let mut node = &mut root;
            let components: Vec<_> = path.components().collect();
            let last_idx = components.len().saturating_sub(1);

            for (i, component) in components.iter().enumerate() {
                let key = component.as_os_str().to_string_lossy().to_string();
                node = node.children.entry(key).or_insert_with(TreeNode::new);

                if i == last_idx {
                    if let Ok(meta) = fs::metadata(path) {
                        node.file_size = Some(meta.len());
                        if !is_binary_file(path) {
                            node.line_count = count_lines(path);
                        }
                    }
                }
            }
        }
        root
    }

    pub fn is_dir(&self) -> bool {
        !self.children.is_empty()
    }
}

fn should_skip_path(path: &PathBuf) -> bool {
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy();
        if name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) {
            return true;
        }
    }
    is_binary_file(path)
}

fn is_binary_file(path: &PathBuf) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| BINARY_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn count_lines(path: &PathBuf) -> Option<usize> {
    fs::read_to_string(path).ok().map(|c| c.lines().count())
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn print_symbols(db: Arc<AstDB>, path: &PathBuf) -> String {
    let cpath = path.to_string_lossy().to_string();
    let defs = crate::ast::ast_db::doc_defs(db.clone(), &cpath);
    let symbols: Vec<String> = defs
        .iter()
        .filter(|x| {
            matches!(
                x.symbol_type,
                SymbolType::StructDeclaration
                    | SymbolType::TypeAlias
                    | SymbolType::FunctionDeclaration
            )
        })
        .map(|x| x.name())
        .collect();
    if symbols.is_empty() {
        String::new()
    } else {
        format!(" ({})", symbols.join(", "))
    }
}

fn print_files_tree(
    tree: &TreeNode,
    ast_db: Option<Arc<AstDB>>,
    maxdepth: usize,
    max_files: usize,
    is_root_query: bool,
) -> String {
    fn traverse(
        node: &TreeNode,
        path: PathBuf,
        depth: usize,
        maxdepth: usize,
        max_files: usize,
        is_root_level: bool,
        ast_db: Option<Arc<AstDB>>,
    ) -> Option<String> {
        if depth > maxdepth {
            return None;
        }

        let indent = "  ".repeat(depth);
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        if !node.is_dir() {
            let mut info = String::new();
            if let Some(size) = node.file_size {
                info.push_str(&format!(" [{}]", format_size(size)));
            }
            if let Some(lines) = node.line_count {
                info.push_str(&format!(" {}L", lines));
            }
            if let Some(db) = ast_db.clone() {
                info.push_str(&print_symbols(db, &path));
            }
            return Some(format!("{}{}{}\n", indent, name, info));
        }

        let mut output = format!("{}{}/\n", indent, name);
        let mut sorted_children: Vec<_> = node.children.iter().collect();
        sorted_children.sort_by(|a, b| {
            let a_is_dir = a.1.is_dir();
            let b_is_dir = b.1.is_dir();
            b_is_dir.cmp(&a_is_dir).then(a.0.cmp(b.0))
        });

        let total_files = sorted_children.iter().filter(|(_, c)| !c.is_dir()).count();

        let should_truncate = !is_root_level && total_files > max_files;
        let mut files_shown = 0;
        let mut hidden_files = 0;
        let mut hidden_dirs = 0;

        for (child_name, child) in &sorted_children {
            let mut child_path = path.clone();
            child_path.push(child_name);

            if !child.is_dir() && should_truncate && files_shown >= max_files {
                hidden_files += 1;
                continue;
            }

            if let Some(child_str) = traverse(
                child,
                child_path,
                depth + 1,
                maxdepth,
                max_files,
                false,
                ast_db.clone(),
            ) {
                output.push_str(&child_str);
                if !child.is_dir() {
                    files_shown += 1;
                }
            } else {
                if child.is_dir() {
                    hidden_dirs += 1;
                } else {
                    hidden_files += 1;
                }
            }
        }

        if hidden_dirs > 0 || hidden_files > 0 {
            output.push_str(&format!(
                "{}  ...+{} dirs, +{} files\n",
                indent, hidden_dirs, hidden_files
            ));
        }
        Some(output)
    }

    let mut result = String::new();
    let mut sorted_roots: Vec<_> = tree.children.iter().collect();
    sorted_roots.sort_by(|a, b| {
        let a_is_dir = a.1.is_dir();
        let b_is_dir = b.1.is_dir();
        b_is_dir.cmp(&a_is_dir).then(a.0.cmp(b.0))
    });
    for (name, node) in sorted_roots {
        if let Some(output) = traverse(
            node,
            PathBuf::from(name),
            0,
            maxdepth,
            max_files,
            is_root_query,
            ast_db.clone(),
        ) {
            result.push_str(&output);
        }
    }
    result
}

fn print_files_tree_with_budget(
    tree: &TreeNode,
    char_limit: usize,
    ast_db: Option<Arc<AstDB>>,
    max_files: usize,
    is_root_query: bool,
) -> String {
    let depth1_output = print_files_tree(tree, ast_db.clone(), 1, max_files, is_root_query);
    if depth1_output.len() > char_limit {
        let truncated: String = depth1_output
            .chars()
            .take(char_limit.saturating_sub(20))
            .collect();
        return format!("{}...[truncated]", truncated);
    }
    let mut good_enough = depth1_output;
    for maxdepth in 2..20 {
        let bigger = print_files_tree(tree, ast_db.clone(), maxdepth, max_files, is_root_query);
        if bigger.len() > char_limit {
            break;
        }
        good_enough = bigger;
    }
    good_enough
}

pub async fn tree_for_tools(
    ccx: Arc<AMutex<AtCommandsContext>>,
    tree: &TreeNode,
    use_ast: bool,
    max_files: usize,
    is_root_query: bool,
) -> Result<String, String> {
    let (gcx, tokens_for_rag) = {
        let ccx_locked = ccx.lock().await;
        (ccx_locked.global_context.clone(), ccx_locked.tokens_for_rag)
    };
    const CHARS_PER_TOKEN: f32 = 3.5;
    let char_limit = ((tokens_for_rag as f32) * CHARS_PER_TOKEN) as usize;

    let ast_db = if use_ast {
        if let Some(ast_module) = gcx.read().await.ast_service.clone() {
            crate::ast::ast_indexer_thread::ast_indexer_block_until_finished(
                ast_module.clone(),
                20_000,
                true,
            )
            .await;
            Some(ast_module.lock().await.ast_index.clone())
        } else {
            None
        }
    } else {
        None
    };

    Ok(print_files_tree_with_budget(
        tree,
        char_limit,
        ast_db,
        max_files,
        is_root_query,
    ))
}

#[async_trait]
impl AtCommand for AtTree {
    fn params(&self) -> &Vec<Box<dyn AtParam>> {
        &self.params
    }

    async fn at_execute(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        cmd: &mut AtCommandMember,
        args: &mut Vec<AtCommandMember>,
    ) -> Result<(Vec<ContextEnum>, String), String> {
        let (gcx, execution_scope) = {
            let ccx_locked = ccx.lock().await;
            (
                ccx_locked.global_context.clone(),
                ccx_locked.execution_scope.clone(),
            )
        };
        let scoped_enforced = execution_scope
            .as_ref()
            .map(|scope| scope.is_enforced())
            .unwrap_or(false);
        let paths_from_anywhere = if scoped_enforced {
            vec![]
        } else {
            paths_from_anywhere(gcx.clone()).await
        };
        let project_dirs = get_project_dirs(gcx.clone()).await;
        let filtered_paths: Vec<PathBuf> = paths_from_anywhere
            .into_iter()
            .filter(|path| project_dirs.iter().any(|pd| path.starts_with(pd)))
            .collect();

        *args = args
            .iter()
            .take_while(|arg| arg.text != "\n" || arg.text == "--ast")
            .take(2)
            .cloned()
            .collect();

        let mut scope_notices = vec![];
        let (tree, is_root_query) = if scoped_enforced {
            match args.iter().find(|x| x.text != "--ast") {
                None => {
                    let root = execution_scope
                        .as_ref()
                        .unwrap()
                        .effective_root()
                        .to_path_buf();
                    let paths =
                        list_scoped_files_under_dir(gcx.clone(), &root, true, false).await?;
                    (TreeNode::build(&paths), true)
                }
                Some(arg) => {
                    let path = arg.text.clone();
                    let resolved = resolve_existing_path_with_execution_scope(
                        gcx.clone(),
                        execution_scope.as_ref(),
                        &path,
                    )
                    .await?
                    .ok_or_else(|| format!("Failed to resolve scoped path '{}'", path))?;
                    scope_notices.extend(resolved.notices);
                    if !resolved.path.is_dir() {
                        let e = format!("Path '{}' is not a directory", resolved.path.display());
                        cmd.ok = false;
                        cmd.reason = Some(e.clone());
                        args.clear();
                        return Err(e);
                    }
                    let paths = list_scoped_files_under_dir(
                        gcx.clone(),
                        &resolved.path,
                        true,
                        resolved.outside_absolute_path,
                    )
                    .await?;
                    (TreeNode::build(&paths), false)
                }
            }
        } else {
            match args.iter().find(|x| x.text != "--ast") {
                None => (TreeNode::build(&filtered_paths), true),
                Some(arg) => {
                    let path = arg.text.clone();
                    let candidates =
                        correct_to_nearest_dir_path(gcx.clone(), &path, false, 10).await;
                    let candidate = return_one_candidate_or_a_good_error(
                        gcx.clone(),
                        &path,
                        &candidates,
                        &project_dirs,
                        true,
                    )
                    .await
                    .map_err(|e| {
                        cmd.ok = false;
                        cmd.reason = Some(e.clone());
                        args.clear();
                        e
                    })?;
                    let start_dir = PathBuf::from(candidate);
                    let paths = filtered_paths
                        .iter()
                        .filter(|f| f.starts_with(&start_dir))
                        .cloned()
                        .collect();
                    (TreeNode::build(&paths), false)
                }
            }
        };

        let use_ast = args.iter().any(|x| x.text == "--ast");
        let tree = tree_for_tools(ccx.clone(), &tree, use_ast, 10, is_root_query)
            .await
            .map_err(|err| {
                warn!("{}", err);
                err
            })?;

        let tree = if tree.is_empty() {
            "tree(): directory is empty".to_string()
        } else {
            tree
        };
        let tree = format!("{}{}", format_scope_notices(&scope_notices), tree);
        Ok((
            vec![ContextEnum::ChatMessage(ChatMessage::new(
                "plain_text".to_string(),
                tree,
            ))],
            "".to_string(),
        ))
    }
}
