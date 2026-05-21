use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// Convert a PathBuf to a String with forward slashes on all platforms.
/// Matches the dominant codebase convention for serialized/context file paths.
fn path_to_string(p: &Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(windows) { s.replace('\\', "/") } else { s.into_owned() }
}

async fn get_workspace_files(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    gcx.documents_state.workspace_files.lock().unwrap().clone()
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
    if scoped.path.is_file() {
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

pub fn is_worktree_root_alias(scope: &str) -> bool {
    let scope = scope.trim().trim_end_matches(&['/', '\\'][..]);
    scope.is_empty() || scope == "." || scope == "workspace"
}

pub async fn list_execution_scope_root(
    gcx: Arc<GlobalContext>,
    execution_scope: &ExecutionScope,
    recursive: bool,
) -> Result<Vec<PathBuf>, String> {
    execution_scope.ensure_active_root()?;
    list_files_under_dir(
        gcx,
        &execution_scope.effective_root().to_path_buf(),
        recursive,
        true,
    )
    .await
}

async fn resolve_scope_legacy(gcx: Arc<GlobalContext>, scope: &str) -> Result<Vec<String>, String> {
    if scope == "workspace" {
        return Ok(get_workspace_files(gcx)
            .await
            .into_iter()
            .map(|f| path_to_string(&f))
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
            .map(|f| path_to_string(&f))
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
                        .map(|f| path_to_string(&f))
                        .collect())
                }
                Err(_) => Err(file_err),
            }
        }
    }
}

#[allow(dead_code)]
pub async fn resolve_scope(gcx: Arc<GlobalContext>, scope: &str) -> Result<Vec<String>, String> {
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

    if is_worktree_root_alias(scope) {
        let files = list_execution_scope_root(gcx, execution_scope, true)
            .await?
            .into_iter()
            .map(|file| path_to_string(&file))
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
        check_file_privacy_for_send(gcx, &scoped.path).await?;
        return Ok(ScopedFiles {
            files: vec![path_to_string(&scoped.path)],
            notices: scoped_path_notices(&scoped),
        });
    }
    if !scoped.path.is_dir() {
        return Err(format!(
            "Path '{}' is not a file or directory",
            scoped.path.display()
        ));
    }

    let files = list_files_under_dir(gcx, &scoped.path, true, true)
        .await?
        .into_iter()
        .map(|file| path_to_string(&file))
        .collect();

    Ok(ScopedFiles {
        files,
        notices: scoped_path_notices(&scoped),
    })
}

fn escape_path_for_scope_filter(path: &str) -> String {
    path.replace('"', "")
        .replace('%', r"\%")
        .replace('_', r"\_")
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
        return Ok(Some(format!(r#"(scope LIKE "{}%" ESCAPE '\')"#, escape_path_for_scope_filter(&dir_path_with_sep))));
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
        Ok(file_path) => Ok(Some(format!("(scope = \"{}\")", escape_path_for_scope_filter(&file_path)))),
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
                    Ok(Some(format!(r#"(scope LIKE "{}%" ESCAPE '\')"#, escape_path_for_scope_filter(&dir_path_with_sep))))
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

pub fn is_worktree_only_path(execution_scope: &ExecutionScope, path: &Path) -> bool {
    if !path.starts_with(execution_scope.effective_root()) {
        return false;
    }
    let Ok(relative) = path.strip_prefix(execution_scope.effective_root()) else {
        return false;
    };
    !execution_scope
        .source_workspace_root()
        .join(relative)
        .is_file()
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

    execution_scope.ensure_active_root()?;
    if is_worktree_root_alias(scope) {
        return Ok(ScopedScopeFilter {
            filter: Some(format!(

                r#"(scope LIKE "{}%" ESCAPE '\')"#,
                escape_path_for_scope_filter(&path_with_sep(execution_scope.source_workspace_root()))
            )),
            notices: vec![],
        });
    }

    let scope_is_dir = scope.ends_with('/') || scope.ends_with('\\');
    let scoped = execution_scope.resolve_existing_path(&PathBuf::from(scope))?;
    if scoped.path.is_file() {
        check_file_privacy_for_send(gcx, &scoped.path).await?;
    }
    let indexed_path = indexed_path_for_scoped_path(execution_scope, &scoped.path);
    let filter = if scoped.path.is_dir() || scope_is_dir {

        Some(format!(r#"(scope LIKE "{}%" ESCAPE '\')"#, escape_path_for_scope_filter(&path_with_sep(&indexed_path))))
    } else {
        Some(format!("(scope = \"{}\")", escape_path_for_scope_filter(&indexed_path.to_string_lossy())))
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

    execution_scope.ensure_active_root()?;
    let raw_path = PathBuf::from(&context_file.file_name);
    if !raw_path.is_absolute() {
        return Ok(Some((context_file, vec![])));
    }
    let normalized_path =
        dunce::simplified(&canonical_path(context_file.file_name.clone())).to_path_buf();

    if normalized_path.starts_with(execution_scope.effective_root()) {
        check_file_privacy_for_send(gcx.clone(), &normalized_path).await?;
        context_file.file_name = path_to_string(&normalized_path);
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
                    check_file_privacy_for_send(gcx.clone(), &worktree_path).await?;
                    let notice = format!(
                        "⚠️ AST/VecDB result was mapped from source checkout to active worktree: {} -> {}",
                        normalized_path.display(),
                        worktree_path.display()
                    );
                    context_file.file_name = path_to_string(&worktree_path);
                    return Ok(Some((context_file, vec![notice])));
                }
            }
            return Ok(None);
        }
    }

    check_file_privacy_for_send(gcx, &normalized_path).await?;
    context_file.file_name = path_to_string(&normalized_path);
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
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::fs;
    use tokio::sync::Mutex as AMutex;

    struct MockVecdb {
        records: Vec<crate::vecdb::vdb_structs::VecdbRecord>,
    }

    #[async_trait]
    impl crate::vecdb::vdb_structs::VecdbSearch for MockVecdb {
        async fn vecdb_search(
            &self,
            query: String,
            _top_n: usize,
            _filter_mb: Option<String>,
        ) -> Result<crate::vecdb::vdb_structs::SearchResult, String> {
            Ok(crate::vecdb::vdb_structs::SearchResult {
                query_text: query,
                results: self.records.clone(),
            })
        }

        async fn get_status(&self) -> Result<crate::vecdb::vdb_structs::VecDbStatus, String> {
            Ok(crate::vecdb::vdb_structs::VecDbStatus {
                files_unprocessed: 0,
                files_total: self.records.len(),
                requests_made_since_start: 0,
                vectors_made_since_start: 0,
                db_size: 0,
                db_cache_size: 0,
                state: "done".to_string(),
                queue_additions: false,
                vecdb_max_files_hit: false,
                vecdb_errors: Default::default(),
            })
        }

        async fn remove_file(&self, _file_path: &PathBuf) -> Result<(), String> {
            Ok(())
        }

        async fn vectorizer_enqueue_files(
            &self,
            _documents: &[String],
            _process_immediately: bool,
        ) {
        }

        fn current_constants(&self) -> (crate::vecdb::vdb_structs::EmbeddingModelConfig, usize) {
            (
                crate::vecdb::vdb_structs::EmbeddingModelConfig {
                    endpoint: String::new(),
                    endpoint_style: String::new(),
                    api_key: String::new(),
                    model_name: String::new(),
                    embedding_size: 0,
                    rejection_threshold: 0.0,
                    embedding_batch: 1,
                    n_ctx: 0,
                },
                0,
            )
        }

        async fn embed_query(&self, _query: &str) -> Result<Vec<f32>, String> {
            Ok(vec![])
        }

        async fn vecdb_search_with_embedding(
            &self,
            _embedding: &Vec<f32>,
            _top_n: usize,
            _filter_mb: Option<String>,
        ) -> Result<Vec<crate::vecdb::vdb_structs::VecdbRecord>, String> {
            Ok(self.records.clone())
        }
    }

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
            root.join("src").join("new_name.rs"),
            "pub fn renamed_marker() {}\n",
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
        fs::write(
            source.join("src").join("deleted_stale.rs"),
            "pub fn deleted_marker() {}\n",
        )
        .unwrap();
        fs::write(
            source.join("src").join("old_name.rs"),
            "pub fn renamed_marker() {}\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("sibling").join("src")).unwrap();
        fs::write(
            temp.path().join("sibling").join("src").join("lib.rs"),
            "fn sibling_version() {}\n",
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
            fixture.source.join("src").join("deleted_stale.rs"),
            fixture.source.join("src").join("old_name.rs"),
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

    async fn install_mock_vecdb(gcx: Arc<GlobalContext>, records: Vec<(&PathBuf, &str)>) {
        let records = records
            .into_iter()
            .enumerate()
            .map(
                |(idx, (path, _label))| crate::vecdb::vdb_structs::VecdbRecord {
                    vector: None,
                    file_path: path.clone(),
                    start_line: 0,
                    end_line: 0,
                    distance: idx as f32,
                    usefulness: 100.0 - idx as f32,
                },
            )
            .collect();
        *gcx.vec_db.lock().await = Some(Arc::new(MockVecdb { records }));
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

    /// Normalize a path string to forward slashes for cross-platform test comparison.
    /// On Windows, tool results may use backslashes; PathBuf::join also produces backslashes.
    fn norm_str(s: &str) -> String {
        s.replace('\\', "/")
    }

    fn norm(p: std::path::PathBuf) -> String {
        norm_str(&p.to_string_lossy())
    }

    fn cat_args(path: String) -> HashMap<String, Value> {
        HashMap::from_iter([("paths".to_string(), Value::String(path))])
    }

    fn semantic_args(query: &str) -> HashMap<String, Value> {
        HashMap::from_iter([
            ("queries".to_string(), Value::String(query.to_string())),
            ("scope".to_string(), Value::String("workspace".to_string())),
        ])
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
    async fn worktree_scope_read_tools_tree_aliases_use_worktree_root() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let tool_call_id = "tree-call".to_string();

        for path in [".", "workspace"] {
            let ccx = make_ccx(gcx.clone(), fixture.worktree.clone()).await;
            let mut tool = ToolTree {
                config_path: String::new(),
            };
            let args = HashMap::from_iter([("path".to_string(), Value::String(path.to_string()))]);

            let (_corrections, results) =
                tool.tool_execute(ccx, &tool_call_id, &args).await.unwrap();
            let text = tool_text(&results);

            assert!(text.contains("worktree_only.rs"), "{text}");
            assert!(!text.contains("source_only.rs"), "{text}");
        }
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
            vec![norm(fixture.root.join("src").join("lib.rs"))]
        );
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_duplicate_relative_path_prefers_worktree() {
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
        let text = tool_text(&results);

        assert_eq!(names.len(), 1);
        assert_eq!(
            names[0],
            norm(fixture.root.join("src").join("lib.rs"))
        );
        assert!(
            !norm_str(&text).contains(&norm(fixture.source.clone())),
            "{text}"
        );
        assert!(!text.contains("sibling"), "{text}");
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
        assert_eq!(names, vec![norm_str(&path)]);
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
        let worktree_path = norm(fixture.root.join("src").join("lib.rs"));

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
        assert_eq!(names, vec![norm_str(&outside_allowed)]);

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
        assert!(resolved
            .files
            .iter()
            .any(|path| path.ends_with("new_name.rs")));
        assert!(!resolved
            .files
            .iter()
            .any(|path| path.ends_with("source_only.rs")));
        assert!(!resolved
            .files
            .iter()
            .any(|path| path.ends_with("deleted_stale.rs")));
        assert!(!resolved
            .files
            .iter()
            .any(|path| path.ends_with("old_name.rs")));
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_search_pattern_uses_worktree_only() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = crate::tools::tool_regex_search::ToolRegexSearch {
            config_path: String::new(),
        };
        let tool_call_id = "search-call".to_string();
        let args = HashMap::from_iter([
            ("pattern".to_string(), Value::String("version".to_string())),
            ("scope".to_string(), Value::String("workspace".to_string())),
        ]);

        let (_corrections, results) = tool.tool_execute(ccx, &tool_call_id, &args).await.unwrap();
        let names = context_file_names(&results);
        let text = tool_text(&results);

        let expected = norm(fixture.root.join("src/lib.rs"));
        assert!(names.iter().any(|path| path == &expected), "expected {expected} in {names:?}");
        assert!(!names.iter().any(|path| path.contains("source_only.rs")));
        assert!(text.contains("worktree_version"), "{text}");
        assert!(!text.contains("source_version"), "{text}");
        assert!(
            !text.contains(&norm(fixture.source.clone())),
            "{text}"
        );
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_search_pattern_finds_worktree_only_file_content() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = crate::tools::tool_regex_search::ToolRegexSearch {
            config_path: String::new(),
        };
        let tool_call_id = "search-call".to_string();
        let args = HashMap::from_iter([
            (
                "pattern".to_string(),
                Value::String("only_worktree".to_string()),
            ),
            ("scope".to_string(), Value::String("workspace".to_string())),
        ]);

        let (_corrections, results) = tool.tool_execute(ccx, &tool_call_id, &args).await.unwrap();
        let names = context_file_names(&results);
        let text = tool_text(&results);

        let expected = norm(fixture.root.join("src/worktree_only.rs"));
        assert!(names.iter().any(|path| path == &expected), "expected {expected} in {names:?}");
        assert!(text.contains("only_worktree"), "{text}");
        assert!(!text.contains("only_source"), "{text}");
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_search_pattern_drops_source_only_and_deleted_stale() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let mut tool = crate::tools::tool_regex_search::ToolRegexSearch {
            config_path: String::new(),
        };
        let tool_call_id = "search-call".to_string();

        for pattern in ["only_source", "deleted_marker"] {
            let ccx = make_ccx(gcx.clone(), fixture.worktree.clone()).await;
            let args = HashMap::from_iter([
                ("pattern".to_string(), Value::String(pattern.to_string())),
                ("scope".to_string(), Value::String("workspace".to_string())),
            ]);

            let err = tool
                .tool_execute(ccx, &tool_call_id, &args)
                .await
                .unwrap_err();

            assert!(err.contains("No matches found"), "{err}");
            assert!(
                !norm_str(&err).contains(&norm(fixture.source.clone())),
                "{err}"
            );
        }
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_search_pattern_renamed_file_new_name_only() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = crate::tools::tool_regex_search::ToolRegexSearch {
            config_path: String::new(),
        };
        let tool_call_id = "search-call".to_string();
        let args = HashMap::from_iter([
            (
                "pattern".to_string(),
                Value::String("renamed_marker".to_string()),
            ),
            ("scope".to_string(), Value::String("workspace".to_string())),
        ]);

        let (_corrections, results) = tool.tool_execute(ccx, &tool_call_id, &args).await.unwrap();
        let names = context_file_names(&results);
        let text = tool_text(&results);

        let expected = norm(fixture.root.join("src/new_name.rs"));
        assert!(names.iter().any(|path| path == &expected), "expected {expected} in {names:?}");
        assert!(!names.iter().any(|path| path.contains("old_name.rs")));
        assert!(text.contains("new_name.rs"), "{text}");
        assert!(!text.contains("old_name.rs"), "{text}");
        assert!(
            !text.contains(&norm(fixture.source.clone())),
            "{text}"
        );
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_semantic_search_drops_source_only_stale_results() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        install_mock_vecdb(
            gcx.clone(),
            vec![
                (&fixture.source.join("src/lib.rs"), "indexed"),
                (&fixture.source.join("src/source_only.rs"), "source-only"),
                (&fixture.source.join("src/deleted_stale.rs"), "deleted"),
                (&fixture.source.join("src/old_name.rs"), "renamed-old"),
            ],
        )
        .await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = crate::tools::tool_search::ToolSearch {
            config_path: String::new(),
        };
        let tool_call_id = "semantic-call".to_string();

        let (_corrections, results) = tool
            .tool_execute(ccx, &tool_call_id, &semantic_args("version"))
            .await
            .unwrap();
        let names = context_file_names(&results);
        let text = tool_text(&results);

        let expected = norm(fixture.root.join("src/lib.rs"));
        assert!(names.iter().any(|path| path == &expected), "expected {expected} in {names:?}");
        assert!(!names.iter().any(|path| path.contains("source_only.rs")));
        assert!(!names.iter().any(|path| path.contains("deleted_stale.rs")));
        assert!(!names.iter().any(|path| path.contains("old_name.rs")));
        assert!(!text.contains("source_only.rs"), "{text}");
        assert!(!text.contains("deleted_stale.rs"), "{text}");
        assert!(!text.contains("old_name.rs"), "{text}");
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_semantic_search_adds_worktree_only_fallback() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        install_mock_vecdb(gcx.clone(), vec![]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = crate::tools::tool_search::ToolSearch {
            config_path: String::new(),
        };
        let tool_call_id = "semantic-call".to_string();

        let (_corrections, results) = tool
            .tool_execute(ccx, &tool_call_id, &semantic_args("only_worktree"))
            .await
            .unwrap();
        let names = context_file_names(&results);
        let text = tool_text(&results);

        let expected = norm(fixture.root.join("src/worktree_only.rs"));
        assert!(names.iter().any(|path| path == &expected), "expected {expected} in {names:?}");
        assert!(
            text.contains("Direct worktree filesystem fallback"),
            "{text}"
        );
        assert!(!text.contains("source_only.rs"), "{text}");
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_semantic_search_renamed_file_new_name_found() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        install_mock_vecdb(
            gcx.clone(),
            vec![(&fixture.source.join("src/old_name.rs"), "old")],
        )
        .await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = crate::tools::tool_search::ToolSearch {
            config_path: String::new(),
        };
        let tool_call_id = "semantic-call".to_string();

        let (_corrections, results) = tool
            .tool_execute(ccx, &tool_call_id, &semantic_args("renamed_marker"))
            .await
            .unwrap();
        let names = context_file_names(&results);
        let text = tool_text(&results);

        let expected = norm(fixture.root.join("src/new_name.rs"));
        assert!(names.iter().any(|path| path == &expected), "expected {expected} in {names:?}");
        assert!(!names.iter().any(|path| path.contains("old_name.rs")));
        assert!(text.contains("new_name.rs"), "{text}");
        assert!(!text.contains("old_name.rs"), "{text}");
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_missing_worktree_fails_closed() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        fs::remove_dir_all(&fixture.root).unwrap();
        let mut tool = ToolCat {
            config_path: String::new(),
        };
        let tool_call_id = "cat-call".to_string();

        let (_corrections, results) = tool
            .tool_execute(ccx, &tool_call_id, &cat_args("src/lib.rs".to_string()))
            .await
            .unwrap();
        let text = tool_text(&results);

        assert!(
            text.contains("Active worktree is missing or stale"),
            "{text}"
        );
        assert!(
            text.contains("will not fallback to source workspace"),
            "{text}"
        );
        assert!(!text.contains("source_version"), "{text}");
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
        let worktree_path = norm(fixture.root.join("src").join("lib.rs"));
        let mut args = vec![AtCommandMember::new("arg".to_string(), source_path, 7, 10)];

        let (results, replacement) = at_file.at_execute(ccx, &mut cmd, &mut args).await.unwrap();
        let text = tool_text(&results);
        let names = context_file_names(&results);

        assert!(text.contains("mapped to active worktree"), "{text}");
        assert_eq!(names, vec![worktree_path.clone()]);
        assert_eq!(norm_str(&replacement), worktree_path);
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

    #[tokio::test]
    async fn worktree_scope_read_tools_ast_source_only_context_file_is_dropped() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let scope = ExecutionScope::from_worktree(&fixture.worktree);
        let source_path = fixture
            .source
            .join("src")
            .join("source_only.rs")
            .to_string_lossy()
            .to_string();
        let context_file = ContextFile {
            file_name: source_path,
            file_content: String::new(),
            line1: 1,
            line2: 1,
            file_rev: None,
            symbols: vec!["only_source".to_string()],
            gradient_type: 5,
            usefulness: 100.0,
            skip_pp: false,
        };

        let remapped = remap_context_file_for_execution_scope(gcx, Some(&scope), context_file)
            .await
            .unwrap();

        assert!(remapped.is_none());
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_ast_deleted_source_context_file_is_dropped() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let scope = ExecutionScope::from_worktree(&fixture.worktree);
        let source_path = fixture
            .source
            .join("src")
            .join("deleted_stale.rs")
            .to_string_lossy()
            .to_string();
        let context_file = ContextFile {
            file_name: source_path,
            file_content: String::new(),
            line1: 1,
            line2: 1,
            file_rev: None,
            symbols: vec!["deleted_marker".to_string()],
            gradient_type: 5,
            usefulness: 100.0,
            skip_pp: false,
        };

        let remapped = remap_context_file_for_execution_scope(gcx, Some(&scope), context_file)
            .await
            .unwrap();

        assert!(remapped.is_none());
    }

    #[tokio::test]
    async fn worktree_scope_read_tools_ast_renamed_old_source_context_file_is_dropped() {
        let fixture = make_fixture();
        let gcx = make_gcx(&fixture, vec![]).await;
        let scope = ExecutionScope::from_worktree(&fixture.worktree);
        let source_path = fixture
            .source
            .join("src")
            .join("old_name.rs")
            .to_string_lossy()
            .to_string();
        let context_file = ContextFile {
            file_name: source_path,
            file_content: String::new(),
            line1: 1,
            line2: 1,
            file_rev: None,
            symbols: vec!["renamed_marker".to_string()],
            gradient_type: 5,
            usefulness: 100.0,
            skip_pp: false,
        };

        let remapped = remap_context_file_for_execution_scope(gcx, Some(&scope), context_file)
            .await
            .unwrap();

        assert!(remapped.is_none());
    }

    #[tokio::test]
    async fn worktree_scope_directory_listing_filters_env_file() {
        let fixture = make_fixture();
        fs::write(fixture.root.join(".env"), "SECRET=value\n").unwrap();
        let gcx = make_gcx(&fixture, vec!["*.env".to_string()]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = ToolCat {
            config_path: String::new(),
        };
        let tool_call_id = "cat-call".to_string();

        let (_corrections, results) = tool
            .tool_execute(ccx, &tool_call_id, &cat_args(".".to_string()))
            .await
            .unwrap();
        let names = context_file_names(&results);
        let text = tool_text(&results);

        assert!(
            !names.iter().any(|n| n.ends_with(".env")),
            "cat('.') should not expose .env: {:?}",
            names
        );
        assert!(!text.contains("SECRET=value"), "{text}");
    }

    #[tokio::test]
    async fn worktree_scope_remap_blocks_env_inside_worktree() {
        let fixture = make_fixture();
        fs::write(fixture.root.join(".env"), "SECRET=value\n").unwrap();
        let gcx = make_gcx(&fixture, vec!["*.env".to_string()]).await;
        let scope = ExecutionScope::from_worktree(&fixture.worktree);
        let env_path = fixture.root.join(".env").to_string_lossy().to_string();
        let context_file = ContextFile {
            file_name: env_path,
            file_content: String::new(),
            line1: 1,
            line2: 1,
            file_rev: None,
            symbols: vec![],
            gradient_type: 5,
            usefulness: 100.0,
            skip_pp: false,
        };

        let result =
            remap_context_file_for_execution_scope(gcx, Some(&scope), context_file).await;
        assert!(
            result.is_err(),
            "privacy-blocked .env inside worktree should return error"
        );
        assert!(
            result.unwrap_err().contains("Blocked"),
            "error should mention privacy level Blocked"
        );
    }

    #[tokio::test]
    async fn worktree_scope_env_file_inside_worktree_is_privacy_checked() {
        let fixture = make_fixture();
        fs::write(fixture.root.join(".env"), "SECRET=value\n").unwrap();
        let gcx = make_gcx(&fixture, vec!["*.env".to_string()]).await;
        let ccx = make_ccx(gcx, fixture.worktree.clone()).await;
        let mut tool = ToolCat {
            config_path: String::new(),
        };
        let tool_call_id = "cat-call".to_string();

        let (_corrections, results) = tool
            .tool_execute(ccx, &tool_call_id, &cat_args(".env".to_string()))
            .await
            .unwrap();
        let text = tool_text(&results);

        assert!(context_file_names(&results).is_empty());
        assert!(text.contains("privacy level Blocked"), "{text}");
    }

    #[test]
    fn scope_filter_construction_handles_special_chars() {
        let apostrophe = "/home/user's project/src/";
        let filter = format!(
            r#"(scope LIKE "{}%" ESCAPE '\')"#,
            escape_path_for_scope_filter(apostrophe)
        );
        assert!(filter.starts_with(r#"(scope LIKE ""#), "{filter}");
        assert!(filter.contains("user's project"), "{filter}");

        let percent = "/home/user/50%/src/";
        let filter = format!(
            r#"(scope LIKE "{}%" ESCAPE '\')"#,
            escape_path_for_scope_filter(percent)
        );
        assert!(filter.contains(r"50\%/"), "{filter}");

        let underscore = "/home/user/my_project/src/";
        let filter = format!(
            r#"(scope LIKE "{}%" ESCAPE '\')"#,
            escape_path_for_scope_filter(underscore)
        );
        assert!(filter.contains(r"my\_project"), "{filter}");
    }

    #[test]
    fn scope_filter_percent_and_underscore_match_literally() {
        let escaped = escape_path_for_scope_filter("/path/50%/a_b/");
        assert!(escaped.contains(r"\%"), "percent should be escaped: {escaped}");
        assert!(escaped.contains(r"\_"), "underscore should be escaped: {escaped}");

        let filter = format!(r#"(scope LIKE "{}%" ESCAPE '\')"#, escaped);
        assert!(filter.contains("ESCAPE"), "filter should include ESCAPE clause: {filter}");
        assert!(filter.contains(r"\%/"), "escaped percent should appear in filter: {filter}");
        assert!(filter.contains(r"\_"), "escaped underscore should appear in filter: {filter}");
    }
}
