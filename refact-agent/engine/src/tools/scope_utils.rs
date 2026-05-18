use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

pub use refact_scope_utils::{
    dedup_notices, format_scope_notices, path_with_sep, scoped_path_notices, ScopedFiles,
    ScopedResolvedPath, ScopedScopeFilter,
};

use crate::at_commands::at_file::{file_repair_candidates, return_one_candidate_or_a_good_error};
use crate::call_validation::ContextFile;
use crate::files_correction::{canonical_path, correct_to_nearest_dir_path, get_project_dirs};
use crate::files_in_workspace::{check_file_privacy_for_send, filter_privacy_allowed_files, ls_files};
use crate::global_context::GlobalContext;
use crate::worktrees::scope::ExecutionScope;

async fn get_workspace_files(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    gcx.documents_state
        .workspace_files
        .lock()
        .unwrap()
        .clone()
}

pub async fn resolve_existing_path_with_execution_scope(
    gcx: Arc<GlobalContext>,
    execution_scope: Option<&ExecutionScope>,
    raw: &str,
) -> Result<Option<ScopedResolvedPath>, String> {
    let Some(scope) = execution_scope else {
        return Ok(None);
    };
    if !scope.is_enforced() {
        return Ok(None);
    }
    let scoped = scope.resolve_existing_path(&PathBuf::from(raw))?;
    if scoped.outside_absolute_path && scoped.path.is_file() {
        check_file_privacy_for_send(gcx, &scoped.path).await?;
    }
    Ok(Some(ScopedResolvedPath {
        path: scoped.path.clone(),
        notices: scoped_path_notices(&scoped),
        outside_absolute_path: scoped.outside_absolute_path,
    }))
}

async fn list_files_under_dir(
    gcx: Arc<GlobalContext>,
    dir: &PathBuf,
    recursive: bool,
    privacy_filter: bool,
) -> Result<Vec<PathBuf>, String> {
    let indexing_everywhere =
        crate::files_blocklist::reload_indexing_everywhere_if_needed(gcx.clone()).await;
    let files = ls_files(&indexing_everywhere, dir, recursive)?;
    if privacy_filter {
        Ok(filter_privacy_allowed_files(gcx, files).await)
    } else {
        Ok(files)
    }
}

pub async fn list_scoped_files_under_dir(
    gcx: Arc<GlobalContext>,
    dir: &PathBuf,
    recursive: bool,
    privacy_filter: bool,
) -> Result<Vec<PathBuf>, String> {
    list_files_under_dir(gcx, dir, recursive, privacy_filter).await
}

async fn resolve_scope_legacy(
    gcx: Arc<GlobalContext>,
    scope: &str,
) -> Result<Vec<String>, String> {
    if scope == "workspace" {
        return Ok(get_workspace_files(gcx)
            .await
            .into_iter()
            .map(|f| f.to_string_lossy().to_string())
            .collect());
    }

    let project_dirs = get_project_dirs(gcx.clone()).await;
    let scope_string = scope.to_string();
    let scope_is_dir = scope.ends_with('/') || scope.ends_with('\\');

    if scope_is_dir {
        let dir_path = return_one_candidate_or_a_good_error(
            gcx.clone(),
            &scope_string,
            &correct_to_nearest_dir_path(gcx.clone(), &scope_string, false, 10).await,
            &project_dirs,
            true,
        )
        .await?;

        let dir_path_with_sep = if dir_path.ends_with(std::path::MAIN_SEPARATOR) {
            dir_path.clone()
        } else {
            format!("{}{}", dir_path, std::path::MAIN_SEPARATOR)
        };
        return Ok(get_workspace_files(gcx)
            .await
            .into_iter()
            .filter(|f| {
                f.to_string_lossy().starts_with(&dir_path_with_sep)
                    || f.to_string_lossy() == dir_path
            })
            .map(|f| f.to_string_lossy().to_string())
            .collect());
    }

    match return_one_candidate_or_a_good_error(
        gcx.clone(),
        &scope_string,
        &file_repair_candidates(gcx.clone(), &scope_string, 10, false).await,
        &project_dirs,
        false,
    )
    .await
    {
        Ok(file_path) => Ok(vec![file_path]),
        Err(file_err) => {
            match return_one_candidate_or_a_good_error(
                gcx.clone(),
                &scope_string,
                &correct_to_nearest_dir_path(gcx.clone(), &scope_string, false, 10).await,
                &project_dirs,
                true,
            )
            .await
            {
                Ok(dir_path) => {
                    let dir_path_with_sep = if dir_path.ends_with(std::path::MAIN_SEPARATOR) {
                        dir_path.clone()
                    } else {
                        format!("{}{}", dir_path, std::path::MAIN_SEPARATOR)
                    };
                    Ok(get_workspace_files(gcx)
                        .await
                        .into_iter()
                        .filter(|f| {
                            f.to_string_lossy().starts_with(&dir_path_with_sep)
                                || f.to_string_lossy() == dir_path
                        })
                        .map(|f| f.to_string_lossy().to_string())
                        .collect())
                }
                Err(_) => Err(file_err),
            }
        }
    }
}

#[allow(dead_code)]
pub async fn resolve_scope(
    gcx: Arc<GlobalContext>,
    scope: &str,
) -> Result<Vec<String>, String> {
    resolve_scope_legacy(gcx, scope).await
}

pub async fn resolve_scope_with_execution_scope(
    gcx: Arc<GlobalContext>,
    execution_scope: Option<&ExecutionScope>,
    scope: &str,
) -> Result<ScopedFiles, String> {
    let Some(execution_scope) = execution_scope else {
        return Ok(ScopedFiles {
            files: resolve_scope_legacy(gcx, scope).await?,
            notices: vec![],
        });
    };
    if !execution_scope.is_enforced() {
        return Ok(ScopedFiles {
            files: resolve_scope_legacy(gcx, scope).await?,
            notices: vec![],
        });
    }

    if scope == "workspace" {
        let files = list_files_under_dir(
            gcx,
            &execution_scope.effective_root().to_path_buf(),
            true,
            false,
        )
        .await?
        .into_iter()
        .map(|file| file.to_string_lossy().to_string())
        .collect();
        return Ok(ScopedFiles {
            files,
            notices: vec![],
        });
    }

    let scope_is_dir = scope.ends_with('/') || scope.ends_with('\\');
    let scoped = execution_scope.resolve_existing_path(&PathBuf::from(scope))?;
    if scoped.path.is_file() {
        if scope_is_dir {
            return Err(format!(
                "⚠️ '{}' is a file, not a directory. 💡 Remove the trailing slash or use a directory scope",
                scope
            ));
        }
        if scoped.outside_absolute_path {
            check_file_privacy_for_send(gcx, &scoped.path).await?;
        }
        return Ok(ScopedFiles {
            files: vec![scoped.path.to_string_lossy().to_string()],
            notices: scoped_path_notices(&scoped),
        });
    }
    if !scoped.path.is_dir() {
        return Err(format!(
            "Path '{}' is not a file or directory",
            scoped.path.display()
        ));
    }

    let files = list_files_under_dir(gcx, &scoped.path, true, scoped.outside_absolute_path)
        .await?
        .into_iter()
        .map(|file| file.to_string_lossy().to_string())
        .collect();

    Ok(ScopedFiles {
        files,
        notices: scoped_path_notices(&scoped),
    })
}

async fn create_scope_filter_legacy(
    gcx: Arc<GlobalContext>,
    scope: &str,
) -> Result<Option<String>, String> {
    if scope == "workspace" {
        return Ok(None);
    }

    let project_dirs = get_project_dirs(gcx.clone()).await;
    let scope_string = scope.to_string();
    let scope_is_dir = scope.ends_with('/') || scope.ends_with('\\');

    if scope_is_dir {
        let dir_path = return_one_candidate_or_a_good_error(
            gcx.clone(),
            &scope_string,
            &correct_to_nearest_dir_path(gcx.clone(), &scope_string, false, 10).await,
            &project_dirs,
            true,
        )
        .await?;

        let dir_path_with_sep = if dir_path.ends_with(std::path::MAIN_SEPARATOR) {
            dir_path.clone()
        } else {
            format!("{}{}", dir_path, std::path::MAIN_SEPARATOR)
        };
        return Ok(Some(format!("(scope LIKE '{}%')", dir_path_with_sep)));
    }

    match return_one_candidate_or_a_good_error(
        gcx.clone(),
        &scope_string,
        &file_repair_candidates(gcx.clone(), &scope_string, 10, false).await,
        &project_dirs,
        false,
    )
    .await
    {
        Ok(file_path) => Ok(Some(format!("(scope = \"{}\")", file_path))),
        Err(file_err) => {
            match return_one_candidate_or_a_good_error(
                gcx.clone(),
                &scope_string,
                &correct_to_nearest_dir_path(gcx.clone(), &scope_string, false, 10).await,
                &project_dirs,
                true,
            )
            .await
            {
                Ok(dir_path) => {
                    let dir_path_with_sep = if dir_path.ends_with(std::path::MAIN_SEPARATOR) {
                        dir_path.clone()
                    } else {
                        format!("{}{}", dir_path, std::path::MAIN_SEPARATOR)
                    };
                    Ok(Some(format!("(scope LIKE '{}%')", dir_path_with_sep)))
                }
                Err(_) => Err(file_err),
            }
        }
    }
}

#[allow(dead_code)]
pub async fn create_scope_filter(
    gcx: Arc<GlobalContext>,
    scope: &str,
) -> Result<Option<String>, String> {
    create_scope_filter_legacy(gcx, scope).await
}

fn indexed_path_for_scoped_path(execution_scope: &ExecutionScope, path: &Path) -> PathBuf {
    if path.starts_with(execution_scope.effective_root()) {
        if let Ok(relative) = path.strip_prefix(execution_scope.effective_root()) {
            return execution_scope.source_workspace_root().join(relative);
        }
    }
    path.to_path_buf()
}

pub async fn create_scope_filter_with_execution_scope(
    gcx: Arc<GlobalContext>,
    execution_scope: Option<&ExecutionScope>,
    scope: &str,
) -> Result<ScopedScopeFilter, String> {
    let Some(execution_scope) = execution_scope else {
        return Ok(ScopedScopeFilter {
            filter: create_scope_filter_legacy(gcx, scope).await?,
            notices: vec![],
        });
    };
    if !execution_scope.is_enforced() {
        return Ok(ScopedScopeFilter {
            filter: create_scope_filter_legacy(gcx, scope).await?,
            notices: vec![],
        });
    }

    if scope == "workspace" {
        return Ok(ScopedScopeFilter {
            filter: Some(format!(
                "(scope LIKE '{}%')",
                path_with_sep(execution_scope.source_workspace_root())
            )),
            notices: vec![],
        });
    }

    let scope_is_dir = scope.ends_with('/') || scope.ends_with('\\');
    let scoped = execution_scope.resolve_existing_path(&PathBuf::from(scope))?;
    if scoped.outside_absolute_path && scoped.path.is_file() {
        check_file_privacy_for_send(gcx, &scoped.path).await?;
    }
    let indexed_path = indexed_path_for_scoped_path(execution_scope, &scoped.path);
    let filter = if scoped.path.is_dir() || scope_is_dir {
        Some(format!("(scope LIKE '{}%')", path_with_sep(&indexed_path)))
    } else {
        Some(format!("(scope = \"{}\")", indexed_path.to_string_lossy()))
    };
    Ok(ScopedScopeFilter {
        filter,
        notices: scoped_path_notices(&scoped),
    })
}

pub async fn remap_context_file_for_execution_scope(
    gcx: Arc<GlobalContext>,
    execution_scope: Option<&ExecutionScope>,
    mut context_file: ContextFile,
) -> Result<Option<(ContextFile, Vec<String>)>, String> {
    let Some(execution_scope) = execution_scope else {
        return Ok(Some((context_file, vec![])));
    };
    if !execution_scope.is_enforced() {
        return Ok(Some((context_file, vec![])));
    }

    let raw_path = PathBuf::from(&context_file.file_name);
    if !raw_path.is_absolute() {
        return Ok(Some((context_file, vec![])));
    }
    let normalized_path =
        dunce::simplified(&canonical_path(context_file.file_name.clone())).to_path_buf();

    if normalized_path.starts_with(execution_scope.effective_root()) {
        context_file.file_name = normalized_path.to_string_lossy().to_string();
        return Ok(Some((context_file, vec![])));
    }

    for source_root in [
        execution_scope.source_workspace_root().to_path_buf(),
        execution_scope.repo_root().to_path_buf(),
    ] {
        if normalized_path.starts_with(&source_root) {
            if let Ok(relative) = normalized_path.strip_prefix(&source_root) {
                let worktree_path = execution_scope.effective_root().join(relative);
                if worktree_path.is_file() {
                    let worktree_path = dunce::simplified(&canonical_path(
                        worktree_path.to_string_lossy().to_string(),
                    ))
                    .to_path_buf();
                    let notice = format!(
                        "⚠️ AST/VecDB result was mapped from source checkout to active worktree: {} -> {}",
                        normalized_path.display(),
                        worktree_path.display()
                    );
                    context_file.file_name = worktree_path.to_string_lossy().to_string();
                    return Ok(Some((context_file, vec![notice])));
                }
            }
            return Ok(None);
        }
    }

    check_file_privacy_for_send(gcx, &normalized_path).await?;
    context_file.file_name = normalized_path.to_string_lossy().to_string();
    Ok(Some((
        context_file,
        vec![format!(
            "⚠️ STRONG NOTICE: result path is outside active worktree; content comes from outside active worktree: {}",
            normalized_path.display()
        )],
    )))
}

pub async fn remap_context_files_for_execution_scope(
    gcx: Arc<GlobalContext>,
    execution_scope: Option<&ExecutionScope>,
    context_files: Vec<ContextFile>,
) -> Result<(Vec<ContextFile>, Vec<String>), String> {
    let mut remapped = Vec::new();
    let mut notices = Vec::new();
    let mut seen = HashSet::new();

    for context_file in context_files {
        if let Some((context_file, mut file_notices)) =
            remap_context_file_for_execution_scope(gcx.clone(), execution_scope, context_file)
                .await?
        {
            let key = format!(
                "{}:{}:{}:{:?}",
                context_file.file_name,
                context_file.line1,
                context_file.line2,
                context_file.symbols
            );
            if seen.insert(key) {
                remapped.push(context_file);
            }
            notices.append(&mut file_notices);
        }
    }

    Ok((remapped, dedup_notices(notices)))
}

pub fn validate_scope_files(files: Vec<String>, scope: &str) -> Result<Vec<String>, String> {
    if files.is_empty() {
        Err(format!(
            "⚠️ No files found in scope '{}'. 💡 Use 'workspace' for all files, 'dir/' (trailing slash) for directories, or check path exists",
            scope
        ))
    } else {
        Ok(files)
    }
}

#[cfg(test)]
mod worktree_scope_read_tools {
    use super::*;
    use crate::at_commands::at_commands::{AtCommand, AtCommandsContext};
    use crate::at_commands::at_file::AtFile;
    use crate::at_commands::execute_at::AtCommandMember;
    use crate::call_validation::{ChatContent, ContextEnum};
    use crate::privacy::{FilePrivacySettings, PrivacySettings};
    use crate::tools::tool_cat::ToolCat;
    use crate::tools::tool_tree::ToolTree;
    use crate::tools::tools_description::Tool;
    use crate::worktrees::types::WorktreeMeta;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::fs;
    use tokio::sync::Mutex as AMutex;

    struct Fixture {
        _temp: tempfile::TempDir,
        worktree: WorktreeMeta,
        root: PathBuf,
        source: PathBuf,
        outside: PathBuf,
    }

    fn make_fixture() -> Fixture {
        let temp = tempfile::Builder::new()
            .prefix("refact-worktree-scope-")
            .tempdir()
            .unwrap();
        let root = temp.path().join("worktree");
        let source = temp.path().join("source");
        let outside = temp.path().join("outside");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(source.join("src")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(
            root.join("src").join("lib.rs"),
            "fn worktree_version() {}\n",
        )
        .unwrap();
        fs::write(
            root.join("src").join("worktree_only.rs"),
            "pub fn only_worktree() {}\n",
        )
        .unwrap();
        fs::write(
            source.join("src").join("lib.rs"),
            "fn source_version() {}\n",
        )
        .unwrap();
        fs::write(
            source.join("src").join("source_only.rs"),
            "pub fn only_source() {}\n",
        )
        .unwrap();
        fs::write(outside.join("allowed.txt"), "outside allowed\n").unwrap();
        fs::write(outside.join("blocked.blocked"), "outside blocked\n").unwrap();
        let root = dunce::simplified(&fs::canonicalize(root).unwrap()).to_path_buf();
        let source = dunce::simplified(&fs::canonicalize(source).unwrap()).to_path_buf();
        let outside = dunce::simplified(&fs::canonicalize(outside).unwrap()).to_path_buf();
        let worktree = WorktreeMeta {
            id: "wt-read-tools".to_string(),
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
            outside,
        }
    }

    async fn make_gcx(fixture: &Fixture, blocked: Vec<String>) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace_files = vec![
            fixture.source.join("src").join("lib.rs"),
            fixture.source.join("src").join("source_only.rs"),
        ];
        {
            let locked = gcx.clone();
            let privacy_settings = locked.privacy_settings.clone();
            let workspace_folders = locked.documents_state.workspace_folders.clone();
            let workspace_files_lock = locked.documents_state.workspace_files.clone();
            drop(locked);
            *privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
                privacy_rules: FilePrivacySettings {
                    only_send_to_servers_I_control: vec![],
                    blocked,
                },
                loaded_ts: u64::MAX / 2,
            });
            *workspace_folders.lock().unwrap() = vec![fixture.source.clone()];
            *workspace_files_lock.lock().unwrap() = workspace_files;
        }
        gcx
    }

    async fn make_ccx(
        gcx: Arc<GlobalContext>,
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
                    ChatContent::Multimodal(parts) => Some(
                        parts
                            .iter()
                            .filter(|part| part.m_type == "text")
                            .map(|part| part.m_content.clone())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                    ChatContent::ContextFiles(_) => None,
                },
                ContextEnum::ContextFile(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn context_file_names(results: &[ContextEnum]) -> Vec<String> {
        results
            .iter()
            .filter_map(|item| match item {
                ContextEnum::ContextFile(file) => Some(file.file_name.clone()),
                _ => None,
            })
            .collect()
    }

    fn cat_args(path: String) -> HashMap<String, Value> {
        HashMap::from_iter([("paths".to_string(), Value::String(path))])
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_tree_defaults_to_worktree_root() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = ToolTree {
            config_path: String::new(),
        };
        let tool_call_id = "tree-call".to_string();

        let (_corrections, results) = tool
            .tool_execute(ccx, &tool_call_id, &HashMap::new())
            .await
            .unwrap();
        let text = tool_text(&results);

        assert!(text.contains("worktree_only.rs"), "{text}");
        assert!(!text.contains("source_only.rs"), "{text}");
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_cat_relative_uses_worktree_file() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = ToolCat {
            config_path: String::new(),
        };
        let tool_call_id = "cat-call".to_string();

        let (_corrections, results) = tool
            .tool_execute(ccx, &tool_call_id, &cat_args("src/lib.rs".to_string()))
            .await
            .unwrap();
        let names = context_file_names(&results);

        assert_eq!(
            names,
            vec![fixture
                .root
                .join("src")
                .join("lib.rs")
                .to_string_lossy()
                .to_string()]
        );
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_cat_absolute_worktree_warns() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = ToolCat {
            config_path: String::new(),
        };
        let tool_call_id = "cat-call".to_string();
        let path = fixture
            .root
            .join("src")
            .join("lib.rs")
            .to_string_lossy()
            .to_string();

        let (_corrections, results) = tool
            .tool_execute(ccx, &tool_call_id, &cat_args(path.clone()))
            .await
            .unwrap();
        let text = tool_text(&results);
        let names = context_file_names(&results);

        assert!(
            text.contains("Absolute path used in active worktree"),
            "{text}"
        );
        assert_eq!(names, vec![path]);
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_cat_absolute_source_remaps() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = ToolCat {
            config_path: String::new(),
        };
        let tool_call_id = "cat-call".to_string();
        let source_path = fixture
            .source
            .join("src")
            .join("lib.rs")
            .to_string_lossy()
            .to_string();
        let worktree_path = fixture
            .root
            .join("src")
            .join("lib.rs")
            .to_string_lossy()
            .to_string();

        let (_corrections, results) = tool
            .tool_execute(ccx, &tool_call_id, &cat_args(source_path))
            .await
            .unwrap();
        let text = tool_text(&results);
        let names = context_file_names(&results);

        assert!(text.contains("mapped to active worktree"), "{text}");
        assert_eq!(names, vec![worktree_path]);
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_cat_outside_allowed_warns_and_blocked_is_rejected() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = ToolCat {
            config_path: String::new(),
        };
        let tool_call_id = "cat-call".to_string();
        let outside_allowed = fixture
            .outside
            .join("allowed.txt")
            .to_string_lossy()
            .to_string();

        let (_corrections, results) = tool
            .tool_execute(ccx, &tool_call_id, &cat_args(outside_allowed.clone()))
            .await
            .unwrap();
        let text = tool_text(&results);
        let names = context_file_names(&results);

        assert!(text.contains("STRONG NOTICE"), "{text}");
        assert_eq!(names, vec![outside_allowed]);

        let blocked_path = fixture
            .outside
            .join("blocked.blocked")
            .to_string_lossy()
            .to_string();
        let gcx = make_gcx(&fixture, vec!["*.blocked".to_string()]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let (_corrections, results) = tool
            .tool_execute(ccx, &tool_call_id, &cat_args(blocked_path))
            .await
            .unwrap();
        let text = tool_text(&results);
        assert!(context_file_names(&results).is_empty());
        assert!(text.contains("privacy level Blocked"), "{text}");
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_resolve_workspace_scope_lists_worktree_only() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let scope = ExecutionScope::from_worktree(&fixture.worktree);

        let resolved = resolve_scope_with_execution_scope(gcx, Some(&scope), "workspace")
            .await
            .unwrap();

        assert!(resolved
            .files
            .iter()
            .any(|path| path.ends_with("worktree_only.rs")));
        assert!(!resolved
            .files
            .iter()
            .any(|path| path.ends_with("source_only.rs")));
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_at_file_source_path_remaps() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let at_file = AtFile::new();
        let mut cmd = AtCommandMember::new("cmd".to_string(), "@file".to_string(), 1, 6);
        let source_path = fixture
            .source
            .join("src")
            .join("lib.rs")
            .to_string_lossy()
            .to_string();
        let worktree_path = fixture
            .root
            .join("src")
            .join("lib.rs")
            .to_string_lossy()
            .to_string();
        let mut args = vec![AtCommandMember::new("arg".to_string(), source_path, 7, 10)];

        let (results, replacement) = at_file.at_execute(ccx, &mut cmd, &mut args).await.unwrap();
        let text = tool_text(&results);
        let names = context_file_names(&results);

        assert!(text.contains("mapped to active worktree"), "{text}");
        assert_eq!(names, vec![worktree_path.clone()]);
        assert_eq!(replacement, worktree_path);
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_ast_source_context_file_remaps_to_worktree() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let scope = ExecutionScope::from_worktree(&fixture.worktree);
        let source_path = fixture
            .source
            .join("src")
            .join("lib.rs")
            .to_string_lossy()
            .to_string();
        let worktree_path = fixture
            .root
            .join("src")
            .join("lib.rs")
            .to_string_lossy()
            .to_string();
        let context_file = ContextFile {
            file_name: source_path,
            file_content: String::new(),
            line1: 1,
            line2: 1,
            file_rev: None,
            symbols: vec!["worktree_version".to_string()],
            gradient_type: 5,
            usefulness: 100.0,
            skip_pp: false,
        };

        let remapped = remap_context_file_for_execution_scope(gcx, Some(&scope), context_file)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(remapped.0.file_name, worktree_path);
        assert!(remapped
            .1
            .join("\n")
            .contains("mapped from source checkout"));
    }
}
