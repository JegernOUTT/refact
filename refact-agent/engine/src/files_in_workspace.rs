use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Weak, Mutex as StdMutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use indexmap::IndexSet;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind, RemoveKind, RenameMode};
use ropey::Rope;
use tokio::io::AsyncReadExt;
use tokio::sync::{RwLock as ARwLock, Mutex as AMutex, Notify, mpsc};
use walkdir::WalkDir;
use which::which;
use tracing::info;
use chrono::Utc;
use refact_codegraph::QueuedPath;

use refact_buddy_core::user_action::UserAction;
use crate::files_correction::{canonical_path, resolve_codegraph_queue_path, CommandSimplifiedDirExt};
use crate::git::operations::git_ls_files;
use crate::global_context::{get_app_searchable_id, GlobalContext};
use crate::integrations::running_integrations::load_integrations;
use crate::file_filter::{is_valid_file, SOURCE_FILE_EXTENSIONS};
use crate::privacy::{
    check_file_privacy, load_privacy_if_needed, LegacyPrivacyMatcher, FilePrivacyLevel,
    PrivacySettings,
};
use crate::files_blocklist::{IndexingEverywhere, is_blocklisted, reload_indexing_everywhere_if_needed};
use crate::files_in_jsonl::enqueue_all_docs_from_jsonl_but_read_first;

pub use refact_files::correction_cache::CacheCorrection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatchRegistrationMode {
    Recursive,
    NonRecursive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WatchRegistration {
    path: PathBuf,
    mode: WatchRegistrationMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatchSubtreeClass {
    Excluded,
    CleanSubtree,
    Mixed,
}

// How this works
// --------------
//
// IDE Window communicates workspace folders via LSP:
//    workspace_folder1:
//       some_dir/
//          vcs_root1/
//       vcs_root2/
//    workspace_folder2:
//       dir_without_version/
//          maybe_because_its_new/
//
// We use version control (git, hg, svn) to list files, whenever we can find it.
// If we can't, just use built-in blocklist and recursive directory walk.
// When a file event arrives (such as file created, file modified) we just add the file into index, because it
// might be new (not yet in version control), but apply blocklists to avoid indexing all kinds of junk
// files.
// So blocklist is mainly useful to deal with file events.
// You can customize blocklist using:
//   ~/.config/refact/indexing.yaml
//   ~/path/to/your/project/.refact/indexing.yaml

pub use refact_core::ast_types::Document;

fn normalize_path_for_workspace_state(gcx: &Arc<GlobalContext>, path: &Path) -> PathBuf {
    let worktree_mappings =
        crate::files_correction::registered_worktree_path_mappings(gcx.cache_dir.as_path());
    crate::files_correction::normalize_path_for_unscoped_root_selection(path, &worktree_mappings)
        .unwrap_or_else(|| crate::files_correction::canonical_path(path.to_string_lossy()))
}

fn normalize_path_for_index_store(gcx: &Arc<GlobalContext>, path: &Path) -> PathBuf {
    let worktree_mappings =
        crate::files_correction::registered_worktree_path_mappings(gcx.cache_dir.as_path());
    crate::files_correction::normalize_path_for_unscoped_root_selection(path, &worktree_mappings)
        .unwrap_or_else(|| normalize_path_for_workspace_state(gcx, path))
}

fn path_for_blocklist(path: &Path, roots: &[PathBuf]) -> PathBuf {
    roots
        .iter()
        .filter_map(|root| path.strip_prefix(root).ok().map(|rel| (root, rel)))
        .max_by_key(|(root, _)| root.components().count())
        .map(|(_, rel)| rel.to_path_buf())
        .unwrap_or_else(|| path.to_path_buf())
}

const WATCH_BLOCKLIST_PROBE_ROOT: &str = "__refact_watch_root__";
const WATCH_BLOCKLIST_PROBE_LEAF: &str = "__refact_watch_probe__";

fn path_is_excluded_from_watch(
    scan_root: &Path,
    path: &Path,
    indexing_everywhere: &IndexingEverywhere,
) -> bool {
    if path_is_refact_internal(path) || path_is_trajectory_watcher_owned(path) {
        return true;
    }
    let rel_path = path.strip_prefix(scan_root).unwrap_or(path);
    if path_has_allowed_hidden_component(rel_path) {
        return false;
    }
    if path_has_hidden_component(rel_path) {
        return true;
    }
    if rel_path.as_os_str().is_empty() {
        return false;
    }
    let indexing_settings = indexing_everywhere.indexing_for_path(path);
    let probe = Path::new(WATCH_BLOCKLIST_PROBE_ROOT)
        .join(rel_path)
        .join(WATCH_BLOCKLIST_PROBE_LEAF);
    is_blocklisted(&indexing_settings, &probe)
}

fn classify_watch_subtree(
    scan_root: &Path,
    path: &Path,
    indexing_everywhere: &IndexingEverywhere,
) -> WatchSubtreeClass {
    if path_is_excluded_from_watch(scan_root, path, indexing_everywhere) {
        return WatchSubtreeClass::Excluded;
    }

    let read_dir = match fs::read_dir(path) {
        Ok(read_dir) => read_dir,
        Err(err) => {
            tracing::debug!(
                "Skipping unreadable directory during watch planning {}: {}",
                path.display(),
                err
            );
            return WatchSubtreeClass::CleanSubtree;
        }
    };

    let mut saw_excluded_descendant = false;
    for child in read_dir {
        let child = match child {
            Ok(child) => child,
            Err(err) => {
                tracing::debug!(
                    "Skipping unreadable directory entry during watch planning {}: {}",
                    path.display(),
                    err
                );
                continue;
            }
        };
        let file_type = match child.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                tracing::debug!(
                    "Skipping directory entry with unknown type during watch planning {}: {}",
                    child.path().display(),
                    err
                );
                continue;
            }
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }

        match classify_watch_subtree(scan_root, &child.path(), indexing_everywhere) {
            WatchSubtreeClass::Excluded => saw_excluded_descendant = true,
            WatchSubtreeClass::Mixed => return WatchSubtreeClass::Mixed,
            WatchSubtreeClass::CleanSubtree => {}
        }
    }

    if saw_excluded_descendant {
        WatchSubtreeClass::Mixed
    } else {
        WatchSubtreeClass::CleanSubtree
    }
}

fn build_watch_plan_for_root(
    scan_root: &Path,
    indexing_everywhere: &IndexingEverywhere,
) -> Vec<WatchRegistration> {
    fn visit(
        registrations: &mut Vec<WatchRegistration>,
        scan_root: &Path,
        path: &Path,
        indexing_everywhere: &IndexingEverywhere,
    ) {
        match classify_watch_subtree(scan_root, path, indexing_everywhere) {
            WatchSubtreeClass::Excluded => {}
            WatchSubtreeClass::CleanSubtree => registrations.push(WatchRegistration {
                path: path.to_path_buf(),
                mode: WatchRegistrationMode::Recursive,
            }),
            WatchSubtreeClass::Mixed => {
                registrations.push(WatchRegistration {
                    path: path.to_path_buf(),
                    mode: WatchRegistrationMode::NonRecursive,
                });
                let read_dir = match fs::read_dir(path) {
                    Ok(read_dir) => read_dir,
                    Err(err) => {
                        tracing::debug!(
                            "Skipping unreadable directory during watch plan descent {}: {}",
                            path.display(),
                            err
                        );
                        return;
                    }
                };
                for child in read_dir {
                    let child = match child {
                        Ok(child) => child,
                        Err(err) => {
                            tracing::debug!(
                                "Skipping unreadable directory entry during watch plan descent {}: {}",
                                path.display(),
                                err
                            );
                            continue;
                        }
                    };
                    let file_type = match child.file_type() {
                        Ok(file_type) => file_type,
                        Err(err) => {
                            tracing::debug!(
                                "Skipping directory entry with unknown type during watch plan descent {}: {}",
                                child.path().display(),
                                err
                            );
                            continue;
                        }
                    };
                    if !file_type.is_dir() || file_type.is_symlink() {
                        continue;
                    }
                    visit(registrations, scan_root, &child.path(), indexing_everywhere);
                }
            }
        }
    }

    let mut registrations = Vec::new();
    visit(
        &mut registrations,
        scan_root,
        scan_root,
        indexing_everywhere,
    );
    registrations
}

fn build_watch_plan(
    watch_folders: &[PathBuf],
    indexing_everywhere: &IndexingEverywhere,
) -> Vec<WatchRegistration> {
    let mut plan = Vec::new();
    for root in watch_folders {
        let root = crate::files_correction::canonicalize_normalized_path(root.clone());
        plan.extend(build_watch_plan_for_root(&root, indexing_everywhere));
    }
    plan.sort_by(|a, b| {
        a.path.cmp(&b.path).then_with(|| match (a.mode, b.mode) {
            (WatchRegistrationMode::Recursive, WatchRegistrationMode::NonRecursive) => {
                std::cmp::Ordering::Less
            }
            (WatchRegistrationMode::NonRecursive, WatchRegistrationMode::Recursive) => {
                std::cmp::Ordering::Greater
            }
            _ => std::cmp::Ordering::Equal,
        })
    });
    plan.dedup_by(|a, b| a.path == b.path && a.mode == b.mode);
    plan
}

async fn register_watch_path(
    watcher_lock: &Arc<ARwLock<RecommendedWatcher>>,
    registration: &WatchRegistration,
) {
    let mut watcher = watcher_lock.write().await;
    let recursive_mode = match registration.mode {
        WatchRegistrationMode::Recursive => RecursiveMode::Recursive,
        WatchRegistrationMode::NonRecursive => RecursiveMode::NonRecursive,
    };
    if let Err(err) = watcher.watch(&registration.path, recursive_mode) {
        tracing::warn!(
            "Failed to add watcher for {} ({:?}): {}",
            registration.path.display(),
            registration.mode,
            err
        );
    }
}

async fn unregister_watch_path(watcher_lock: &Arc<ARwLock<RecommendedWatcher>>, path: &Path) {
    let mut watcher = watcher_lock.write().await;
    if let Err(err) = watcher.unwatch(path) {
        tracing::debug!("Failed to remove watcher for {}: {}", path.display(), err);
    }
}

fn event_path_is_valid_file(
    read_path: &PathBuf,
    store_path: &PathBuf,
    roots: &[PathBuf],
    global_config_roots: &[PathBuf],
) -> bool {
    if path_is_refact_internal(store_path) {
        return false;
    }
    if crate::file_filter::is_generated_index_path_with_global_config_roots(
        store_path,
        global_config_roots,
    ) {
        return false;
    }
    let scan_root = roots
        .iter()
        .filter(|root| store_path.starts_with(root))
        .max_by_key(|root| root.components().count());
    match scan_root {
        Some(root) => {
            if is_valid_file(read_path, true, false).is_err() {
                return false;
            }
            let rel_path = store_path
                .strip_prefix(root)
                .unwrap_or(store_path.as_path());
            !(path_has_hidden_component(rel_path) && !path_has_allowed_hidden_component(rel_path))
        }
        None => is_valid_file(read_path, false, false).is_ok(),
    }
}

pub async fn remove_memory_document_for_path(gcx: Arc<GlobalContext>, path: &PathBuf) -> bool {
    let canonical_path = crate::files_correction::canonical_path(path.to_string_lossy());
    let normalized_path = normalize_path_for_workspace_state(&gcx, path);
    let mut doc_map = gcx.documents_state.memory_document_map.lock().await;
    let mut removed = doc_map.remove(&canonical_path).is_some();
    if normalized_path != canonical_path {
        removed |= doc_map.remove(&normalized_path).is_some();
    }
    removed
}

pub async fn remove_memory_documents_under_path(gcx: Arc<GlobalContext>, path: &PathBuf) -> usize {
    let canonical_path = crate::files_correction::canonical_path(path.to_string_lossy());
    let normalized_path = normalize_path_for_workspace_state(&gcx, path);
    let mut doc_map = gcx.documents_state.memory_document_map.lock().await;
    let paths_to_remove = doc_map
        .keys()
        .filter(|p| p.starts_with(&canonical_path) || p.starts_with(&normalized_path))
        .cloned()
        .collect::<Vec<_>>();
    let removed = paths_to_remove.len();
    for path in paths_to_remove {
        doc_map.remove(&path);
    }
    removed
}

pub async fn get_file_text_from_memory_or_disk(
    global_context: Arc<GlobalContext>,
    file_path: &PathBuf,
) -> Result<String, String> {
    let read_context = prepare_file_read_context(global_context.clone()).await;
    get_file_text_from_memory_or_disk_with_context(global_context, file_path, &read_context, None)
        .await
}

pub async fn get_file_text_from_memory_or_disk_for_model_context(
    global_context: Arc<GlobalContext>,
    file_path: &PathBuf,
) -> Result<String, String> {
    let read_context = prepare_file_read_context(global_context.clone()).await;
    get_file_text_from_memory_or_disk_for_model_context_with_context(
        global_context,
        file_path,
        &read_context,
        None,
    )
    .await
}

pub(crate) struct FileReadContext {
    privacy_matcher: LegacyPrivacyMatcher,
    privacy_settings: Arc<PrivacySettings>,
    worktree_mappings: crate::files_correction::WorktreePathMappings,
}

impl FileReadContext {
    pub(crate) fn refresh_review_mappings(&mut self, cache_dir: &Path) {
        self.worktree_mappings = Arc::new(
            crate::files_correction::registered_worktree_path_mappings_uncached(cache_dir),
        );
    }

    /// Configuration identity only: never memoizes a path decision or filesystem fact.
    pub(crate) fn same_inputs(&self, other: &Self) -> bool {
        self.privacy_settings.privacy_rules.blocked == other.privacy_settings.privacy_rules.blocked
            && self
                .privacy_settings
                .privacy_rules
                .only_send_to_servers_I_control
                == other
                    .privacy_settings
                    .privacy_rules
                    .only_send_to_servers_I_control
            && self.worktree_mappings == other.worktree_mappings
    }

    pub(crate) fn worktree_mappings(&self) -> &crate::files_correction::WorktreePathMappings {
        &self.worktree_mappings
    }
}

pub(crate) fn registered_alias_paths(
    path: &Path,
    mappings: &[crate::files_correction::RegisteredWorktreePathMapping],
) -> Vec<PathBuf> {
    let path = crate::files_correction::canonicalize_normalized_path(path.to_path_buf());
    let mut aliases = vec![path.clone()];
    for mapping in mappings {
        if let Ok(suffix) = path.strip_prefix(&mapping.root) {
            aliases.push(crate::files_correction::canonicalize_normalized_path(
                mapping.source_root.join(suffix),
            ));
        }
        if let Ok(suffix) = path.strip_prefix(&mapping.source_root) {
            aliases.push(crate::files_correction::canonicalize_normalized_path(
                mapping.root.join(suffix),
            ));
        }
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

pub fn strictest_zone_for_path(
    policy: &refact_privacy::CompiledPolicy,
    path: &Path,
    workspace_roots: &[PathBuf],
    worktree_mappings: &[crate::files_correction::RegisteredWorktreePathMapping],
) -> refact_privacy::Zone {
    let aliases = registered_alias_paths(path, worktree_mappings);
    let roots = workspace_roots.iter().map(PathBuf::as_path).chain(
        worktree_mappings
            .iter()
            .flat_map(|mapping| [mapping.root.as_path(), mapping.source_root.as_path()]),
    );
    policy.strictest_zone_for_paths_with_roots(aliases, roots)
}

pub(crate) fn check_file_privacy_with_context(
    read_context: &FileReadContext,
    path: &Path,
) -> Result<(), String> {
    for alias in registered_alias_paths(path, &read_context.worktree_mappings) {
        read_context
            .privacy_matcher
            .check(&alias, &FilePrivacyLevel::AllowToSendAnywhere)?;
    }
    Ok(())
}

pub(crate) fn check_file_privacy_for_read_preparation_with_context(
    read_context: &FileReadContext,
    path: &Path,
) -> Result<(), String> {
    for alias in registered_alias_paths(path, &read_context.worktree_mappings) {
        read_context
            .privacy_matcher
            .check_for_read_preparation(&alias)?;
    }
    Ok(())
}

pub(crate) async fn prepare_file_read_context(
    global_context: Arc<GlobalContext>,
) -> FileReadContext {
    let privacy_settings = load_privacy_if_needed(global_context.clone()).await;
    FileReadContext {
        privacy_matcher: LegacyPrivacyMatcher::new(&privacy_settings),
        privacy_settings,
        worktree_mappings: crate::files_correction::registered_worktree_path_mappings(
            global_context.cache_dir.as_path(),
        ),
    }
}

pub(crate) async fn get_file_text_from_memory_or_disk_with_context(
    global_context: Arc<GlobalContext>,
    file_path: &PathBuf,
    read_context: &FileReadContext,
    max_bytes: Option<usize>,
) -> Result<String, String> {
    get_file_text_from_memory_or_disk_with_context_impl(
        global_context,
        file_path,
        read_context,
        max_bytes,
        false,
    )
    .await
}

pub(crate) async fn get_file_text_from_memory_or_disk_for_model_context_with_context(
    global_context: Arc<GlobalContext>,
    file_path: &PathBuf,
    read_context: &FileReadContext,
    max_bytes: Option<usize>,
) -> Result<String, String> {
    get_file_text_from_memory_or_disk_with_context_impl(
        global_context,
        file_path,
        read_context,
        max_bytes,
        true,
    )
    .await
}

async fn get_file_text_from_memory_or_disk_with_context_impl(
    global_context: Arc<GlobalContext>,
    file_path: &PathBuf,
    read_context: &FileReadContext,
    max_bytes: Option<usize>,
    for_model_context: bool,
) -> Result<String, String> {
    let requested_path = crate::files_correction::canonical_path(file_path.to_string_lossy());
    let mapped_path = crate::files_correction::normalize_path_for_unscoped_root_selection(
        &requested_path,
        &read_context.worktree_mappings,
    )
    .unwrap_or_else(|| requested_path.clone());
    if for_model_context {
        check_file_privacy_for_read_preparation_with_context(read_context, &requested_path)?;
    } else {
        check_file_privacy_with_context(read_context, &requested_path)?;
    }

    let doc = {
        let doc_map = global_context
            .documents_state
            .memory_document_map
            .lock()
            .await;
        doc_map.get(&requested_path).cloned().or_else(|| {
            if mapped_path != requested_path {
                doc_map.get(&mapped_path).cloned()
            } else {
                None
            }
        })
    };
    if let Some(doc) = doc {
        let doc = doc.read().await;
        if let Some(text) = doc.doc_text.as_ref() {
            if let Some(limit) = max_bytes {
                if text.len_bytes() > limit {
                    return Err(format!("File exceeds {} byte search limit", limit));
                }
            }
            return Ok(text.to_string());
        }
    }
    let text = match max_bytes {
        Some(limit) => {
            read_file_from_disk_without_privacy_check_limited(&requested_path, limit).await
        }
        None => read_file_from_disk_without_privacy_check(&requested_path).await,
    }
    .map_err(|e| format!("Not found in memory, not found on disk: {}", e))?;
    Ok(text.to_string())
}

pub async fn check_file_privacy_for_send(
    global_context: Arc<GlobalContext>,
    file_path: &PathBuf,
) -> Result<(), String> {
    let read_context = prepare_file_read_context(global_context).await;
    check_file_privacy_with_context(&read_context, file_path)
}

pub async fn check_file_privacy_for_model_context(
    global_context: Arc<GlobalContext>,
    file_path: &PathBuf,
) -> Result<(), String> {
    let read_context = prepare_file_read_context(global_context).await;
    check_file_privacy_for_read_preparation_with_context(&read_context, file_path)
}

pub async fn filter_privacy_allowed_files(
    global_context: Arc<GlobalContext>,
    files: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let read_context = prepare_file_read_context(global_context).await;
    files
        .into_iter()
        .filter(|path| check_file_privacy_with_context(&read_context, path).is_ok())
        .collect()
}

pub async fn filter_privacy_allowed_files_for_model_context(
    global_context: Arc<GlobalContext>,
    files: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let read_context = prepare_file_read_context(global_context).await;
    files
        .into_iter()
        .filter(|path| {
            check_file_privacy_for_read_preparation_with_context(&read_context, path).is_ok()
        })
        .collect()
}

#[derive(Debug, Clone)]
struct PendingBranchHeadChange {
    old_head: Option<String>,
    new_head: Option<String>,
    last_event: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebouncedFileEventKind {
    Upsert,
    Remove,
}

fn debounced_file_event_kind_for_path(
    is_remove_event: bool,
    read_path: &PathBuf,
    store_path: &PathBuf,
    roots: &[PathBuf],
    global_config_roots: &[PathBuf],
) -> Option<DebouncedFileEventKind> {
    if is_remove_event {
        return store_path
            .extension()
            .is_some()
            .then_some(DebouncedFileEventKind::Remove);
    }
    if read_path.exists()
        && event_path_is_valid_file(read_path, store_path, roots, global_config_roots)
    {
        Some(DebouncedFileEventKind::Upsert)
    } else if !read_path.exists() && store_path.extension().is_some() {
        Some(DebouncedFileEventKind::Remove)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub struct DebouncedFileEvent {
    at: Instant,
    is_remove_event: bool,
}

#[derive(Debug, Clone)]
struct PendingFileEvent {
    store_path: PathBuf,
    read_path: PathBuf,
    kind: DebouncedFileEventKind,
}

impl PendingFileEvent {
    fn queued_path(&self) -> QueuedPath {
        QueuedPath::new(
            self.store_path.to_string_lossy().to_string(),
            self.read_path.to_string_lossy().to_string(),
        )
    }

    fn kind_order(&self) -> u8 {
        match self.kind {
            DebouncedFileEventKind::Remove => 0,
            DebouncedFileEventKind::Upsert => 1,
        }
    }
}

pub async fn update_document_text_from_disk(
    doc: &mut Document,
    gcx: Arc<GlobalContext>,
) -> Result<(), String> {
    match read_file_from_disk(load_privacy_if_needed(gcx.clone()).await, &doc.doc_path).await {
        Ok(res) => {
            doc.doc_text = Some(res);
            return Ok(());
        }
        Err(e) => return Err(e),
    }
}

pub async fn get_document_text_or_read_from_disk(
    doc: &mut Document,
    gcx: Arc<GlobalContext>,
) -> Result<String, String> {
    if doc.doc_text.is_some() {
        return Ok(doc.doc_text.as_ref().unwrap().to_string());
    }
    read_file_from_disk(load_privacy_if_needed(gcx.clone()).await, &doc.doc_path)
        .await
        .map(|x| x.to_string())
}

#[derive(Clone)]
pub struct DocumentsState {
    pub workspace_folders: Arc<StdMutex<Vec<PathBuf>>>,
    pub workspace_project_root: Arc<StdMutex<Option<PathBuf>>>,
    pub workspace_files: Arc<StdMutex<Vec<PathBuf>>>,
    pub workspace_vcs_roots: Arc<StdMutex<Vec<PathBuf>>>,

    pub active_file_path: Arc<AMutex<Option<PathBuf>>>,
    pub jsonl_files: Arc<StdMutex<Vec<PathBuf>>>,
    // document_map on windows: c%3A/Users/user\Documents/file.ext
    // query on windows: C:/Users/user/Documents/file.ext
    pub memory_document_map: Arc<AMutex<HashMap<PathBuf, Arc<ARwLock<Document>>>>>, // if a file is open in IDE, and it's outside workspace dirs, it will be in this map and not in workspace_files
    pub cache_dirty: Arc<AMutex<f64>>,
    pub cache_correction: Arc<StdMutex<Arc<CacheCorrection>>>,
    pub fs_watcher: Arc<StdMutex<Option<Arc<ARwLock<RecommendedWatcher>>>>>,
    pub git_branch_heads: Arc<StdMutex<HashMap<PathBuf, String>>>,
    pub branch_reindex_last_ts: Arc<AtomicU64>,
    pub file_event_debounce: Arc<StdMutex<HashMap<PathBuf, DebouncedFileEvent>>>,
    pub file_event_resolutions: Arc<AtomicU64>,
    pub file_watcher_dropped_events: Arc<AtomicU64>,
    file_event_debounce_task: Arc<StdMutex<Option<tokio::task::JoinHandle<()>>>>,
    file_event_debounce_notify: Arc<Notify>,
    file_watcher_event_task: Arc<StdMutex<Option<tokio::task::JoinHandle<()>>>>,
    branch_head_debounce: Arc<StdMutex<HashMap<PathBuf, PendingBranchHeadChange>>>,
    branch_head_debounce_tasks: Arc<StdMutex<HashMap<PathBuf, tokio::task::JoinHandle<()>>>>,
    watch_registration_tasks: Arc<StdMutex<Vec<tokio::task::JoinHandle<()>>>>,
    watch_registration_drops: Arc<AtomicU64>,
}

async fn mem_overwrite_or_create_document(
    global_context: Arc<GlobalContext>,
    document: Document,
) -> (Arc<ARwLock<Document>>, Arc<AMutex<f64>>, bool) {
    let cx = global_context.clone();
    let mut doc_map = cx.documents_state.memory_document_map.lock().await;
    if let Some(existing_doc) = doc_map.get_mut(&document.doc_path) {
        *existing_doc.write().await = document;
        (
            existing_doc.clone(),
            cx.documents_state.cache_dirty.clone(),
            false,
        )
    } else {
        let path = document.doc_path.clone();
        let darc = Arc::new(ARwLock::new(document));
        doc_map.insert(path, darc.clone());
        (darc, cx.documents_state.cache_dirty.clone(), true)
    }
}

impl DocumentsState {
    pub async fn new(workspace_dirs: Vec<PathBuf>) -> Self {
        Self {
            workspace_folders: Arc::new(StdMutex::new(workspace_dirs)),
            workspace_project_root: Arc::new(StdMutex::new(None)),
            workspace_files: Arc::new(StdMutex::new(Vec::new())),
            workspace_vcs_roots: Arc::new(StdMutex::new(Vec::new())),

            active_file_path: Arc::new(AMutex::new(None)),
            jsonl_files: Arc::new(StdMutex::new(Vec::new())),
            memory_document_map: Arc::new(AMutex::new(HashMap::new())),
            cache_dirty: Arc::new(AMutex::<f64>::new(0.0)),
            cache_correction: Arc::new(StdMutex::new(Arc::new(CacheCorrection::new()))),
            fs_watcher: Arc::new(StdMutex::new(None)),
            git_branch_heads: Arc::new(StdMutex::new(HashMap::new())),
            branch_reindex_last_ts: Arc::new(AtomicU64::new(0)),
            file_event_debounce: Arc::new(StdMutex::new(HashMap::new())),
            file_event_resolutions: Arc::new(AtomicU64::new(0)),
            file_watcher_dropped_events: Arc::new(AtomicU64::new(0)),
            file_event_debounce_task: Arc::new(StdMutex::new(None)),
            file_event_debounce_notify: Arc::new(Notify::new()),
            file_watcher_event_task: Arc::new(StdMutex::new(None)),
            branch_head_debounce: Arc::new(StdMutex::new(HashMap::new())),
            branch_head_debounce_tasks: Arc::new(StdMutex::new(HashMap::new())),
            watch_registration_tasks: Arc::new(StdMutex::new(Vec::new())),
            watch_registration_drops: Arc::new(AtomicU64::new(0)),
        }
    }
}

const WATCHER_EVENT_CHANNEL_CAPACITY: usize = 4096;

fn watcher_event_is_actionable(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Modify(_)
            | EventKind::Remove(_)
            | EventKind::Access(AccessKind::Close(AccessMode::Write))
    )
}

fn enqueue_watcher_event(tx: &mpsc::Sender<Event>, dropped_events: &AtomicU64, event: Event) {
    if !watcher_event_is_actionable(&event.kind) {
        return;
    }
    if let Err(mpsc::error::TrySendError::Full(_)) = tx.try_send(event) {
        let dropped = dropped_events.fetch_add(1, Ordering::Relaxed) + 1;
        if dropped == 1 || dropped % 100 == 0 {
            tracing::warn!(
                "File watcher dropped {} events because the queue is full",
                dropped
            );
        }
    }
}

fn make_watcher_event_callback(
    tx: mpsc::Sender<Event>,
    dropped_events: Arc<AtomicU64>,
) -> impl Fn(notify::Result<Event>) + Send + 'static {
    move |res| {
        if let Ok(event) = res {
            enqueue_watcher_event(&tx, &dropped_events, event);
        }
    }
}

pub(crate) async fn recover_file_index_watcher_drops(gcx: Arc<GlobalContext>) {
    let dropped = gcx
        .documents_state
        .file_watcher_dropped_events
        .load(Ordering::Relaxed);
    if gcx.shutdown_flag.load(Ordering::Relaxed) {
        return;
    }
    let task = if gcx.file_index.observe_watcher_drops(dropped) {
        let roots = crate::files_correction::get_unscoped_project_dirs(gcx.clone()).await;
        crate::file_index::spawn_build(gcx.file_index.clone(), roots, gcx.shutdown_flag.clone())
    } else {
        crate::file_index::spawn_due_build(gcx.file_index.clone(), gcx.shutdown_flag.clone())
    };
    if let Some(task) = task {
        let mut tasks = gcx.documents_state.watch_registration_tasks.lock().unwrap();
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
    }
}

fn start_file_watcher_event_queue(
    gcx: &Arc<GlobalContext>,
    gcx_weak: Weak<GlobalContext>,
    receiver: mpsc::Receiver<Event>,
) {
    let old_event_task = std::mem::replace(
        &mut *gcx.documents_state.file_watcher_event_task.lock().unwrap(),
        Some(tokio::spawn(run_file_watcher_event_queue(
            gcx_weak, receiver,
        ))),
    );
    if let Some(handle) = old_event_task {
        handle.abort();
    }
}

async fn run_file_watcher_event_queue(
    gcx_weak: Weak<GlobalContext>,
    receiver: mpsc::Receiver<Event>,
) {
    let shutdown_flag = match gcx_weak.upgrade() {
        Some(gcx) => gcx.shutdown_flag.clone(),
        None => return,
    };
    let mut receiver = Some(receiver);
    let mut recovery_tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        let event = tokio::select! {
            _ = recovery_tick.tick() => {
                let Some(gcx) = gcx_weak.upgrade() else { return; };
                recover_file_index_watcher_drops(gcx).await;
                continue;
            }
            event = async {
                match receiver.as_mut() {
                    Some(receiver) => receiver.recv().await,
                    None => std::future::pending().await,
                }
            } => event,
            _ = wait_for_shutdown_flag(shutdown_flag.clone()) => return,
        };
        let Some(event) = event else {
            receiver = None;
            continue;
        };
        file_watcher_event(event, gcx_weak.clone()).await;
    }
}

pub async fn watcher_init(gcx: Arc<GlobalContext>) {
    let gcx_weak = Arc::downgrade(&gcx);
    let (event_tx, event_rx) = mpsc::channel::<Event>(WATCHER_EVENT_CHANNEL_CAPACITY);
    let event_callback = make_watcher_event_callback(
        event_tx,
        gcx.documents_state.file_watcher_dropped_events.clone(),
    );
    let watcher = RecommendedWatcher::new(event_callback, Config::default());
    // The queue also drives the deferred inventory walk tick, so it runs without a watcher.
    start_file_watcher_event_queue(&gcx, gcx_weak, event_rx);
    let watcher = match watcher {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("Failed to create file watcher (file watching disabled): {e}");
            return;
        }
    };

    let mut watch_folders = crate::files_correction::get_raw_project_dirs(gcx.clone()).await;
    watch_folders.extend(crate::files_correction::get_unscoped_project_dirs(gcx.clone()).await);
    watch_folders.sort();
    watch_folders.dedup();
    let watch_folders = watch_folders
        .into_iter()
        .map(crate::files_correction::canonicalize_normalized_path)
        .collect::<Vec<_>>();
    let indexing_everywhere = reload_indexing_everywhere_if_needed(gcx.clone()).await;
    let watch_plan = build_watch_plan(&watch_folders, indexing_everywhere.as_ref());

    let watcher_lock = Arc::new(ARwLock::new(watcher));
    for registration in &watch_plan {
        info!(
            "ADD WATCHER ({:?}): {}",
            registration.mode,
            registration.path.display()
        );
        register_watch_path(&watcher_lock, registration).await;
    }

    let new_watcher = Some(watcher_lock);
    let old_watcher = {
        std::mem::replace(
            &mut *gcx.documents_state.fs_watcher.lock().unwrap(),
            new_watcher,
        )
    };
    drop(old_watcher);
}

async fn read_file_from_disk_without_privacy_check(path: &PathBuf) -> Result<Rope, String> {
    tokio::fs::read_to_string(path)
        .await
        .map(|x| Rope::from_str(&x))
        .map_err(|e| {
            format!(
                "failed to read file {}: {}",
                crate::nicer_logs::last_n_chars(&path.display().to_string(), 30),
                e
            )
        })
}

async fn read_file_from_disk_without_privacy_check_limited(
    path: &PathBuf,
    max_bytes: usize,
) -> Result<Rope, String> {
    if std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > max_bytes as u64) {
        return Err(format!("File exceeds {} byte search limit", max_bytes));
    }
    let file = tokio::fs::File::open(path).await.map_err(|error| {
        format!(
            "failed to read file {}: {}",
            crate::nicer_logs::last_n_chars(&path.display().to_string(), 30),
            error
        )
    })?;
    let read_limit = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX - 1)
        .saturating_add(1);
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024).saturating_add(1));
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| {
            format!(
                "failed to read file {}: {}",
                crate::nicer_logs::last_n_chars(&path.display().to_string(), 30),
                error
            )
        })?;
    if bytes.len() > max_bytes {
        return Err(format!("File exceeds {} byte search limit", max_bytes));
    }
    let text = String::from_utf8(bytes).map_err(|error| {
        format!(
            "failed to read file {} as UTF-8: {}",
            crate::nicer_logs::last_n_chars(&path.display().to_string(), 30),
            error
        )
    })?;
    Ok(Rope::from_str(&text))
}

pub async fn read_file_from_disk(
    privacy_settings: Arc<PrivacySettings>,
    path: &PathBuf,
) -> Result<Rope, String> {
    check_file_privacy(
        privacy_settings,
        path,
        &FilePrivacyLevel::AllowToSendAnywhere,
    )?;
    read_file_from_disk_without_privacy_check(path).await
}

async fn _run_command(
    cmd: &str,
    args: &[&str],
    path: &PathBuf,
    filter_out_status: bool,
) -> Option<Vec<PathBuf>> {
    info!("{} EXEC {} {}", path.display(), cmd, args.join(" "));
    let output = tokio::process::Command::new(cmd)
        .args(args)
        .current_dir_simplified(path)
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout.clone()).ok().map(|s| {
        s.lines()
            .map(|line| {
                let trimmed = line.trim();
                if filter_out_status && trimmed.len() > 1 {
                    path.join(&trimmed[1..].trim())
                } else {
                    path.join(line)
                }
            })
            .collect()
    })
}

async fn ls_files_under_version_control(path: &PathBuf) -> Option<Vec<PathBuf>> {
    if path.join(".git").exists() {
        git_ls_files(path)
    } else if path.join(".hg").exists() && which("hg").is_ok() {
        // Mercurial repository
        _run_command(
            "hg",
            &[
                "status",
                "--added",
                "--modified",
                "--clean",
                "--unknown",
                "--no-status",
            ],
            path,
            false,
        )
        .await
    } else if path.join(".svn").exists() && which("svn").is_ok() {
        // SVN repository
        let files_under_vc = _run_command("svn", &["list", "-R"], path, false).await;
        let files_changed = _run_command("svn", &["status"], path, true).await;
        Some(
            files_under_vc
                .unwrap_or_default()
                .into_iter()
                .chain(files_changed.unwrap_or_default().into_iter())
                .collect(),
        )
    } else {
        None
    }
}

pub fn _ls_files(
    indexing_everywhere: &IndexingEverywhere,
    scan_root: &Path,
    path: &PathBuf,
    recursive: bool,
    blocklist_check: bool,
) -> Result<Vec<PathBuf>, String> {
    let mut paths = vec![];
    let mut dirs_to_visit = vec![path.clone()];

    while let Some(dir) = dirs_to_visit.pop() {
        let ls_maybe = fs::read_dir(&dir);
        if ls_maybe.is_err() {
            info!(
                "failed to read directory {}: {}",
                dir.display(),
                ls_maybe.unwrap_err()
            );
            continue;
        }
        let ls: fs::ReadDir = ls_maybe.unwrap();
        let entries_maybe = ls.collect::<Result<Vec<_>, _>>();
        if entries_maybe.is_err() {
            info!(
                "failed to read directory {}: {}",
                dir.display(),
                entries_maybe.unwrap_err()
            );
            continue;
        }
        let mut entries = entries_maybe.unwrap();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let abs_path = entry.path();
            let indexing_settings = indexing_everywhere.indexing_for_path(&abs_path);
            let rel_path = abs_path.strip_prefix(scan_root).unwrap_or(&abs_path);
            if recursive && abs_path.is_dir() {
                if !blocklist_check || !is_blocklisted(&indexing_settings, rel_path) {
                    dirs_to_visit.push(abs_path);
                }
            } else if abs_path.is_file() {
                paths.push(abs_path);
            }
        }
    }
    Ok(paths)
}

// NOTE: don't optimized for large workspaces
pub fn ls_files(
    indexing_everywhere: &IndexingEverywhere,
    path: &PathBuf,
    recursive: bool,
) -> Result<Vec<PathBuf>, String> {
    if !path.is_dir() {
        return Err(format!("path '{}' is not a directory", path.display()));
    }

    let indexing_settings = indexing_everywhere.indexing_for_path(path);
    let mut paths = _ls_files(indexing_everywhere, path.as_path(), path, recursive, true).unwrap();
    if recursive {
        for additional_indexing_dir in indexing_settings.additional_indexing_dirs.iter() {
            let additional_path = PathBuf::from(additional_indexing_dir);
            paths.extend(
                _ls_files(
                    indexing_everywhere,
                    additional_path.as_path(),
                    &additional_path,
                    recursive,
                    false,
                )
                .unwrap(),
            );
        }
    }

    Ok(paths)
}

#[derive(Debug, Clone, Default)]
pub struct LsFilesLimited {
    pub files: Vec<PathBuf>,
    pub truncated: bool,
    pub aborted: bool,
}

impl LsFilesLimited {
    fn stopped(&self) -> bool {
        self.truncated || self.aborted
    }
}

fn _ls_files_limited(
    indexing_everywhere: &IndexingEverywhere,
    scan_root: &Path,
    path: &PathBuf,
    recursive: bool,
    blocklist_check: bool,
    budget: &mut usize,
    abort: Option<&AtomicBool>,
    out: &mut LsFilesLimited,
) {
    let mut dirs_to_visit = vec![path.clone()];

    while let Some(dir) = dirs_to_visit.pop() {
        if abort.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            out.aborted = true;
            return;
        }
        let ls_maybe = fs::read_dir(&dir);
        if ls_maybe.is_err() {
            info!(
                "failed to read directory {}: {}",
                dir.display(),
                ls_maybe.unwrap_err()
            );
            continue;
        }
        let ls: fs::ReadDir = ls_maybe.unwrap();
        let entries_maybe = ls.collect::<Result<Vec<_>, _>>();
        if entries_maybe.is_err() {
            info!(
                "failed to read directory {}: {}",
                dir.display(),
                entries_maybe.unwrap_err()
            );
            continue;
        }
        let mut entries = entries_maybe.unwrap();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if abort.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                out.aborted = true;
                return;
            }
            if *budget == 0 {
                out.truncated = true;
                return;
            }
            *budget -= 1;

            let abs_path = entry.path();
            let indexing_settings = indexing_everywhere.indexing_for_path(&abs_path);
            let rel_path = abs_path.strip_prefix(scan_root).unwrap_or(&abs_path);
            if recursive && abs_path.is_dir() {
                if !blocklist_check || !is_blocklisted(&indexing_settings, rel_path) {
                    dirs_to_visit.push(abs_path);
                }
            } else if abs_path.is_file() {
                out.files.push(abs_path);
            }
        }
    }
}

pub fn ls_files_limited(
    indexing_everywhere: &IndexingEverywhere,
    path: &PathBuf,
    recursive: bool,
    max_entries: usize,
    abort: Option<&AtomicBool>,
) -> Result<LsFilesLimited, String> {
    if !path.is_dir() {
        return Err(format!("path '{}' is not a directory", path.display()));
    }

    let indexing_settings = indexing_everywhere.indexing_for_path(path);
    let mut out = LsFilesLimited::default();
    let mut budget = max_entries;

    _ls_files_limited(
        indexing_everywhere,
        path.as_path(),
        path,
        recursive,
        true,
        &mut budget,
        abort,
        &mut out,
    );

    if recursive && !out.stopped() {
        for additional_indexing_dir in indexing_settings.additional_indexing_dirs.iter() {
            if out.stopped() {
                break;
            }
            let additional_path = PathBuf::from(additional_indexing_dir);
            _ls_files_limited(
                indexing_everywhere,
                additional_path.as_path(),
                &additional_path,
                recursive,
                false,
                &mut budget,
                abort,
                &mut out,
            );
        }
    }

    Ok(out)
}

pub async fn detect_vcs_for_a_file_path(file_path: &Path) -> Option<(PathBuf, &'static str)> {
    let mut dir = file_path.to_path_buf();
    if dir.is_file() {
        dir.pop();
    }
    loop {
        if let Some(vcs_type) = get_vcs_type(&dir) {
            return Some((dir, vcs_type));
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

pub fn get_vcs_type(path: &Path) -> Option<&'static str> {
    if path.join(".git").exists() {
        Some("git")
    } else if path.join(".svn").is_dir() {
        Some("svn")
    } else if path.join(".hg").is_dir() {
        Some("hg")
    } else {
        None
    }
}

// Slow version of version control detection:
// async fn is_git_repo(directory: &PathBuf) -> bool {
//     Command::new("git")
//         .arg("rev-parse")
//         .arg("--is-inside-work-tree")
//         .current_dir(directory)
//         .output()
//         .await
//         .map(|output| output.status.success())
//         .unwrap_or(false)
// }
// async fn is_svn_repo(directory: &PathBuf) -> bool {
//     Command::new("svn")
//         .arg("info")
//         .current_dir(directory)
//         .output()
//         .await
//         .map(|output| output.status.success())
//         .unwrap_or(false)
// }
// async fn is_hg_repo(directory: &PathBuf) -> bool {
//     Command::new("hg")
//         .arg("root")
//         .current_dir(directory)
//         .output()
//         .await
//         .map(|output| output.status.success())
//         .unwrap_or(false)
// }

fn path_has_hidden_component(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy().starts_with('.'))
    })
}

fn path_has_allowed_hidden_component(path: &Path) -> bool {
    let parts = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if parts.first().is_some_and(|part| part == ".config")
        && parts.get(1).is_some_and(|part| part == "refact")
    {
        return true;
    }
    path.components().any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy() == ".refact")
    })
}

fn path_is_refact_import_internal(path: &Path) -> bool {
    let mut last_was_refact = false;
    for component in path.components() {
        if last_was_refact && component == Component::Normal("imports".as_ref()) {
            return true;
        }
        last_was_refact = component == Component::Normal(".refact".as_ref());
    }
    false
}

const REFACT_RUNTIME_STATE_ENTRIES: &[&str] = &[
    "state.json",
    "runtime_queue.jsonl",
    "trajectories",
    "chats",
    "buddy",
];

const REFACT_RUNTIME_STATE_TOP_LEVEL_ENTRIES: &[&str] = &[
    "stats",
    "shell_audit.jsonl",
    "scheduled_tasks.json",
    "background_agents",
    "vecdb",
    "review_scratch",
    "visual_baselines",
];

fn refact_runtime_entry_matches(entries: &[&str], name: &str) -> bool {
    entries.iter().any(|entry| {
        name == *entry
            || name
                .strip_prefix(entry)
                .is_some_and(|rest| rest.starts_with('.'))
    })
}

fn path_is_refact_runtime_state(path: &Path) -> bool {
    let mut seen_refact = false;
    let mut directly_under_refact = false;
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let name = name.to_string_lossy();
        if seen_refact && refact_runtime_entry_matches(REFACT_RUNTIME_STATE_ENTRIES, &name) {
            return true;
        }
        if directly_under_refact
            && refact_runtime_entry_matches(REFACT_RUNTIME_STATE_TOP_LEVEL_ENTRIES, &name)
        {
            return true;
        }
        directly_under_refact = name == ".refact";
        if directly_under_refact {
            seen_refact = true;
        }
    }
    false
}

fn path_is_knowledge_document(path: &Path) -> bool {
    let knowledge: Vec<Component> = Path::new(crate::file_filter::KNOWLEDGE_FOLDER_NAME)
        .components()
        .collect();
    path.components()
        .collect::<Vec<_>>()
        .windows(knowledge.len())
        .any(|window| window == knowledge.as_slice())
}

fn path_is_refact_internal(path: &Path) -> bool {
    path_is_refact_import_internal(path)
        || path_is_refact_runtime_state(path)
        || crate::file_filter::is_refact_codegraph_path(path)
}

fn path_is_trajectory_watcher_owned(path: &Path) -> bool {
    let names = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    names.iter().enumerate().any(|(idx, name)| {
        if name.as_str() != ".refact" {
            return false;
        }
        match names.get(idx + 1).map(String::as_str) {
            Some("trajectories") => true,
            Some("tasks") => names
                .get(idx + 3)
                .is_some_and(|name| name.as_str() == "trajectories"),
            _ => false,
        }
    })
}

fn path_triggers_registry_reload(path: &Path) -> bool {
    if path_is_refact_internal(path) {
        return false;
    }
    if !path
        .components()
        .any(|c| c == Component::Normal(".refact".as_ref()))
    {
        return false;
    }
    path.components().any(|c| {
        c == Component::Normal("modes".as_ref())
            || c == Component::Normal("subagents".as_ref())
            || c == Component::Normal("toolbox_commands".as_ref())
            || c == Component::Normal("code_lens".as_ref())
    })
}

fn path_triggers_tool_catalog_reload(path: &Path) -> bool {
    let is_integration = path
        .components()
        .any(|component| component == Component::Normal("integrations.d".as_ref()));
    let is_privacy = path.file_name().is_some_and(|name| name == "privacy.yaml");
    is_integration || is_privacy
}

fn is_valid_file_for_scan(
    path: &PathBuf,
    scan_root: &Path,
    allow_hidden_folders: bool,
    ignore_size_thresholds: bool,
    global_config_roots: &[PathBuf],
) -> Result<(), Box<dyn std::error::Error>> {
    if path_is_refact_internal(path) {
        return Err(".refact internal path".into());
    }
    if crate::file_filter::is_generated_index_path_with_global_config_roots(
        path,
        global_config_roots,
    ) {
        return Err("generated index.json".into());
    }
    is_valid_file(path, true, ignore_size_thresholds)?;
    if !allow_hidden_folders {
        let rel_path = path.strip_prefix(scan_root).unwrap_or(path.as_path());
        if path_has_hidden_component(rel_path) && !path_has_allowed_hidden_component(rel_path) {
            return Err("Parent dir starts with a dot".into());
        }
    }
    Ok(())
}

async fn _ls_files_under_version_control_recursive(
    all_files: &mut Vec<PathBuf>,
    vcs_folders: &mut Vec<PathBuf>,
    avoid_dups: &mut HashSet<PathBuf>,
    indexing_everywhere: &mut IndexingEverywhere,
    path: PathBuf,
    allow_files_in_hidden_folders: bool,
    ignore_size_thresholds: bool,
    check_blocklist: bool,
    global_config_roots: &[PathBuf],
) {
    let scan_root = crate::files_correction::canonical_path(&path.to_string_lossy().to_string());
    let mut candidates: Vec<PathBuf> = vec![scan_root.clone()];
    let mut rejected_reasons: HashMap<String, usize> = HashMap::new();
    let mut blocklisted_dirs_cnt: usize = 0;
    while !candidates.is_empty() {
        let checkme = candidates.pop().unwrap();
        if checkme.is_file() {
            let maybe_valid = is_valid_file_for_scan(
                &checkme,
                &scan_root,
                allow_files_in_hidden_folders,
                ignore_size_thresholds,
                global_config_roots,
            );
            match maybe_valid {
                Ok(_) => {
                    all_files.push(checkme.clone());
                }
                Err(e) => {
                    rejected_reasons
                        .entry(e.to_string())
                        .and_modify(|x| *x += 1)
                        .or_insert(1);
                    continue;
                }
            }
        }
        if checkme.is_dir() {
            if avoid_dups.contains(&checkme) {
                continue;
            }
            avoid_dups.insert(checkme.clone());
            if get_vcs_type(&checkme).is_some() {
                vcs_folders.push(checkme.clone());
            }
            if let Some(v) = ls_files_under_version_control(&checkme).await {
                // Has version control
                let indexing_yaml_path = checkme.join(".refact").join("indexing.yaml");
                if indexing_yaml_path.exists() {
                    match crate::files_blocklist::load_indexing_yaml(
                        &indexing_yaml_path,
                        Some(&checkme),
                    )
                    .await
                    {
                        Ok(indexing_settings) => {
                            for d in indexing_settings.additional_indexing_dirs.iter() {
                                let cp = crate::files_correction::canonical_path(d.as_str());
                                candidates.push(cp);
                            }
                            indexing_everywhere
                                .vcs_indexing_settings_map
                                .insert(checkme.to_string_lossy().to_string(), indexing_settings);
                        }
                        Err(e) => {
                            tracing::error!(
                                "failed to load indexing.yaml in {}: {}",
                                checkme.display(),
                                e
                            );
                        }
                    };
                }
                for x in v.iter() {
                    let indexing_settings = indexing_everywhere.indexing_for_path(x);
                    let rel_for_blocklist = x.strip_prefix(&scan_root).unwrap_or(x);
                    if check_blocklist && is_blocklisted(&indexing_settings, rel_for_blocklist) {
                        blocklisted_dirs_cnt += 1;
                        continue;
                    }
                    let maybe_valid = is_valid_file_for_scan(
                        x,
                        &scan_root,
                        allow_files_in_hidden_folders,
                        ignore_size_thresholds,
                        global_config_roots,
                    );
                    match maybe_valid {
                        Ok(_) => {
                            all_files.push(x.clone());
                        }
                        Err(e) => {
                            rejected_reasons
                                .entry(e.to_string())
                                .and_modify(|x| *x += 1)
                                .or_insert(1);
                        }
                    }
                }
            } else {
                // Don't have version control
                let indexing_settings = indexing_everywhere.indexing_for_path(&checkme);
                let rel_for_blocklist = checkme.strip_prefix(&scan_root).unwrap_or(&checkme);
                if check_blocklist && is_blocklisted(&indexing_settings, rel_for_blocklist) {
                    blocklisted_dirs_cnt += 1;
                    continue;
                }
                let new_paths: Vec<PathBuf> = WalkDir::new(checkme.clone())
                    .max_depth(1)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .map(|e| {
                        crate::files_correction::canonical_path(
                            &e.path().to_string_lossy().to_string(),
                        )
                    })
                    .filter(|e| e != &checkme)
                    .collect();
                candidates.extend(new_paths);
            }
        }
    }
    info!("when inspecting {:?} rejected files reasons:", path);
    for (reason, count) in &rejected_reasons {
        info!("    {:>6} {}", count, reason);
    }
    if rejected_reasons.is_empty() {
        info!("    no bad files at all");
    }
    info!(
        "also the loop bumped into {} blocklisted dirs",
        blocklisted_dirs_cnt
    );
}

pub async fn retrieve_files_in_workspace_folders(
    proj_folders: Vec<PathBuf>,
    indexing_everywhere: &mut IndexingEverywhere,
    allow_files_in_hidden_folders: bool,
    ignore_size_thresholds: bool,
    global_config_roots: &[PathBuf],
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut all_files: Vec<PathBuf> = Vec::new();
    let mut vcs_folders: Vec<PathBuf> = Vec::new();
    let mut avoid_dups: HashSet<PathBuf> = HashSet::new();
    for proj_folder in proj_folders {
        _ls_files_under_version_control_recursive(
            &mut all_files,
            &mut vcs_folders,
            &mut avoid_dups,
            indexing_everywhere,
            proj_folder.clone(),
            allow_files_in_hidden_folders,
            ignore_size_thresholds,
            true,
            global_config_roots,
        )
        .await;
    }
    info!("in all workspace folders, VCS roots found:");
    for vcs_folder in vcs_folders.iter() {
        info!("    {}", vcs_folder.display());
    }
    (all_files, vcs_folders)
}

fn git_root_at_or_above_dir_cached(
    mut dir: PathBuf,
    cache: &mut HashMap<PathBuf, Option<PathBuf>>,
) -> Option<PathBuf> {
    let mut visited = Vec::new();
    loop {
        if let Some(root) = cache.get(&dir).cloned() {
            for visited_dir in visited {
                cache.insert(visited_dir, root.clone());
            }
            return root;
        }
        visited.push(dir.clone());
        if dir.join(".git").exists() {
            let root = Some(dir.clone());
            for visited_dir in visited {
                cache.insert(visited_dir, root.clone());
            }
            return root;
        }
        if !dir.pop() {
            for visited_dir in visited {
                cache.insert(visited_dir, None);
            }
            return None;
        }
    }
}

fn matching_workspace_folder(path: &Path, workspace_folders: &[PathBuf]) -> Option<usize> {
    workspace_folders
        .iter()
        .enumerate()
        .filter(|(_, folder)| path.starts_with(folder))
        .max_by_key(|(_, folder)| folder.components().count())
        .map(|(idx, _)| idx)
}

fn order_workspace_files_for_initial_enqueue(
    files: Vec<PathBuf>,
    workspace_folders: &[PathBuf],
) -> Vec<PathBuf> {
    let workspace_folders = workspace_folders
        .iter()
        .map(|folder| canonical_path(folder.to_string_lossy()))
        .collect::<Vec<_>>();
    let mut git_root_cache = HashMap::new();
    let workspace_git_roots = workspace_folders
        .iter()
        .map(|folder| git_root_at_or_above_dir_cached(folder.clone(), &mut git_root_cache))
        .collect::<Vec<_>>();
    let mut primary = Vec::new();
    let mut nested = Vec::new();

    for file in files {
        let workspace_idx = matching_workspace_folder(&file, &workspace_folders);
        let is_nested = workspace_idx
            .and_then(|idx| workspace_git_roots.get(idx).and_then(|root| root.as_ref()))
            .and_then(|workspace_root| {
                let file_dir = file.parent().unwrap_or(file.as_path()).to_path_buf();
                git_root_at_or_above_dir_cached(file_dir, &mut git_root_cache)
                    .map(|file_root| (workspace_root.clone(), file_root))
            })
            .is_some_and(|(workspace_root, file_root)| {
                file_root != workspace_root && file_root.starts_with(&workspace_root)
            });
        if is_nested {
            nested.push(file);
        } else {
            primary.push(file);
        }
    }
    primary.extend(nested);
    primary
}

pub fn is_path_to_enqueue_valid(path: &PathBuf) -> Result<(), String> {
    let extension = path.extension().unwrap_or_default();
    if !SOURCE_FILE_EXTENSIONS.contains(&extension.to_str().unwrap_or_default()) {
        return Err(format!("Unsupported file extension {:?}", extension).into());
    }
    Ok(())
}

pub async fn enqueue_some_docs_with_read_paths(
    gcx: Arc<GlobalContext>,
    paths: &[QueuedPath],
    force: bool,
) {
    if paths.is_empty() {
        return;
    }
    let worktree_mappings =
        crate::files_correction::registered_worktree_path_mappings(gcx.cache_dir.as_path());
    let workspace_files_changed = normalize_workspace_files_for_unscoped(&gcx, &worktree_mappings);
    let queue_paths = paths
        .iter()
        .map(|path| resolve_codegraph_queue_path(Path::new(&path.read_path), &worktree_mappings))
        .collect::<Vec<_>>();
    enqueue_resolved_docs(gcx, queue_paths, workspace_files_changed, force).await;
}

async fn enqueue_some_docs(gcx: Arc<GlobalContext>, paths: &Vec<String>, force: bool) {
    let worktree_mappings =
        crate::files_correction::registered_worktree_path_mappings(gcx.cache_dir.as_path());
    let workspace_files_changed = normalize_workspace_files_for_unscoped(&gcx, &worktree_mappings);
    let queue_paths = paths
        .iter()
        .map(|path| resolve_codegraph_queue_path(Path::new(path), &worktree_mappings))
        .collect::<Vec<_>>();
    enqueue_resolved_docs(gcx, queue_paths, workspace_files_changed, force).await;
}

async fn enqueue_resolved_docs(
    gcx: Arc<GlobalContext>,
    queue_paths: Vec<QueuedPath>,
    workspace_files_changed: bool,
    force: bool,
) {
    let queue_paths = queue_paths
        .into_iter()
        .filter(|path| {
            !crate::file_filter::is_transient_tmp_path(Path::new(path.store_path.as_str()))
        })
        .collect::<Vec<_>>();
    let normalized_paths = queue_paths
        .iter()
        .map(|path| path.store_path.clone())
        .collect::<Vec<_>>();
    info!(
        "detected {} modified/added/removed files",
        normalized_paths.len()
    );
    for d in normalized_paths.iter().take(5) {
        info!("    {}", crate::nicer_logs::last_n_chars(&d, 30));
    }
    if normalized_paths.len() > 5 {
        info!("    ...");
    }
    crate::indexing_routing::route_index_enqueue(gcx.clone(), &normalized_paths, force, true).await;
    let roots = crate::indexing_routing::memory_plane_roots(gcx.clone()).await;
    let (_memory_paths, code_paths) =
        crate::indexing_routing::partition_paths(&normalized_paths, &roots);
    if !code_paths.is_empty() {
        let code_path_set = code_paths.into_iter().collect::<HashSet<_>>();
        let code_queue_paths = queue_paths
            .iter()
            .filter(|path| code_path_set.contains(&path.store_path))
            .cloned()
            .collect::<Vec<_>>();
        let codegraph = gcx.codegraph.lock().await.clone();
        match codegraph {
            Some(service) => service.enqueue_paths_with_read_paths(&code_queue_paths),
            None => {
                tracing::warn!(
                    "codegraph unavailable; skipping {} code file(s) (memory-plane vec_db never receives code)",
                    code_queue_paths.len()
                );
            }
        }
    }
    let cache_correction_arc =
        crate::files_correction::files_cache_rebuild_as_needed(gcx.clone()).await;
    let mut moar_files: Vec<PathBuf> = Vec::new();
    let mut removed_files: HashSet<PathBuf> = HashSet::new();
    for path in &queue_paths {
        let store_path = PathBuf::from(&path.store_path);
        let read_path = PathBuf::from(&path.read_path);
        if read_path.exists()
            && cache_correction_arc
                .filenames
                .find_matches(&store_path)
                .len()
                == 0
        {
            moar_files.push(store_path);
        } else if !read_path.exists() {
            removed_files.insert(store_path);
        }
    }
    if workspace_files_changed || !moar_files.is_empty() || !removed_files.is_empty() {
        info!("this made file cache dirty");
        let dirty_arc = {
            let mut workspace_files = gcx.documents_state.workspace_files.lock().unwrap();
            if !removed_files.is_empty() {
                workspace_files.retain(|path| !removed_files.contains(path));
            }
            workspace_files.extend(moar_files);
            let mut seen = HashSet::new();
            workspace_files.retain(|path| seen.insert(path.clone()));
            gcx.documents_state.cache_dirty.clone()
        };
        crate::files_correction::mark_files_cache_dirty(&dirty_arc).await;
    }
}

fn normalize_workspace_files_for_unscoped(
    gcx: &Arc<GlobalContext>,
    worktree_mappings: &[crate::files_correction::RegisteredWorktreePathMapping],
) -> bool {
    let mut seen = HashSet::new();
    let normalized = {
        let workspace_files = gcx.documents_state.workspace_files.lock().unwrap();
        workspace_files
            .iter()
            .filter_map(|path| {
                crate::files_correction::normalize_path_for_unscoped_paths(path, worktree_mappings)
            })
            .filter(|path| seen.insert(path.clone()))
            .collect::<Vec<_>>()
    };
    let mut workspace_files = gcx.documents_state.workspace_files.lock().unwrap();
    if *workspace_files == normalized {
        return false;
    }
    *workspace_files = normalized;
    true
}

pub async fn enqueue_all_files_from_workspace_folders(
    gcx: Arc<GlobalContext>,
    wake_up_indexers: bool,
    vecdb_only: bool,
) -> i32 {
    let folders = crate::files_correction::get_unscoped_project_dirs(gcx.clone()).await;

    if let Some(task) = crate::file_index::spawn_build(
        gcx.file_index.clone(),
        folders.clone(),
        gcx.shutdown_flag.clone(),
    ) {
        let mut tasks = gcx.documents_state.watch_registration_tasks.lock().unwrap();
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
    }

    info!(
        "enqueue_all_files_from_workspace_folders started files search with {} folders",
        folders.len()
    );
    let mut indexing_everywhere =
        crate::files_blocklist::reload_global_indexing_only(gcx.clone()).await;
    let global_config_roots = vec![gcx.config_dir.clone()];
    let (all_files, vcs_folders) = retrieve_files_in_workspace_folders(
        folders.clone(),
        &mut indexing_everywhere,
        false,
        false,
        &global_config_roots,
    )
    .await;
    let all_files = order_workspace_files_for_initial_enqueue(all_files, &folders);
    info!(
        "enqueue_all_files_from_workspace_folders found {} files => workspace_files",
        all_files.len()
    );
    let workspace_vcs_roots = vcs_folders.clone();

    let mut old_workspace_files = Vec::new();
    let cache_dirty = {
        {
            let mut workspace_files = gcx.documents_state.workspace_files.lock().unwrap();
            std::mem::swap(&mut *workspace_files, &mut old_workspace_files);
            workspace_files.extend(all_files.clone());
        }
        {
            let mut roots = gcx.documents_state.workspace_vcs_roots.lock().unwrap();
            *roots = workspace_vcs_roots;
        }
        update_git_branch_heads_for_roots(&gcx, &vcs_folders);
        gcx.documents_state.cache_dirty.clone()
    };

    crate::files_correction::mark_files_cache_dirty(&cache_dirty).await;

    let current_paths = all_files.iter().cloned().collect::<HashSet<_>>();
    let removed_paths = old_workspace_files
        .iter()
        .filter(|path| !current_paths.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    let mut updated_paths: IndexSet<String> = IndexSet::new();
    updated_paths.extend(
        all_files
            .iter()
            .map(|file| file.to_string_lossy().to_string()),
    );
    let paths_nodups: Vec<String> = updated_paths.into_iter().collect();

    crate::indexing_routing::route_index_enqueue(
        gcx.clone(),
        &paths_nodups,
        wake_up_indexers,
        vecdb_only,
    )
    .await;
    if !vecdb_only {
        for path in removed_paths {
            on_did_delete(gcx.clone(), &path).await;
        }
    }

    all_files.len() as i32
}

pub async fn on_workspaces_init(gcx: Arc<GlobalContext>) -> i32 {
    // Called from lsp and lsp_like
    // Not called from main.rs as part of initialization
    let folders = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    let old_app_searchable_id = gcx.app_searchable_id.lock().unwrap().clone();
    let new_app_searchable_id = get_app_searchable_id(&folders);
    if old_app_searchable_id != new_app_searchable_id {
        *gcx.app_searchable_id.lock().unwrap() = get_app_searchable_id(&folders);
    }
    // Project competitor import runs only here for normal startup and workspace add/remove changes.
    let _ = crate::ext::competitor_import::run_project_import(
        crate::app_state::AppState::from_gcx(gcx.clone()).await,
    )
    .await;
    watcher_init(gcx.clone()).await;
    let files_enqueued = enqueue_all_files_from_workspace_folders(gcx.clone(), false, false).await;

    crate::git::checkpoints::enqueue_init_shadow_repos(gcx.clone()).await;

    crate::chat::start_trajectory_watcher(gcx.clone());

    let _ = load_integrations(gcx.clone(), &["**/mcp_*".to_string()]).await;

    files_enqueued
}

pub async fn on_did_open(
    gcx: Arc<GlobalContext>,
    cpath: &PathBuf,
    text: &String,
    _language_id: &String,
) {
    if path_is_refact_internal(cpath) {
        return;
    }
    gcx.file_index.refresh(cpath);
    let normalized_path = normalize_path_for_workspace_state(&gcx, cpath);
    let mut doc = Document::new(&normalized_path);
    doc.update_text(text);
    info!(
        "on_did_open {}",
        crate::nicer_logs::last_n_chars(&cpath.display().to_string(), 30)
    );
    let (_doc_arc, dirty_arc, mark_dirty) =
        mem_overwrite_or_create_document(gcx.clone(), doc).await;
    if mark_dirty {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        *dirty_arc.lock().await = now;
    }
    *gcx.documents_state.active_file_path.lock().await = Some(cpath.clone());
}

pub async fn on_did_close(gcx: Arc<GlobalContext>, cpath: &PathBuf) {
    info!(
        "on_did_close {}",
        crate::nicer_logs::last_n_chars(&cpath.display().to_string(), 30)
    );
    if !remove_memory_document_for_path(gcx.clone(), cpath).await {
        tracing::error!(
            "on_did_close: failed to remove from memory_document_map {:?}",
            normalize_path_for_workspace_state(&gcx, cpath).display()
        );
    }
}

pub async fn on_did_change(gcx: Arc<GlobalContext>, path: &PathBuf, text: &String) {
    if path_is_refact_internal(path) {
        return;
    }
    gcx.file_index.refresh(path);
    let t0 = Instant::now();
    let normalized_path = normalize_path_for_workspace_state(&gcx, path);
    let (doc_arc, dirty_arc, mark_dirty) = {
        let mut doc = Document::new(&normalized_path);
        doc.update_text(text);
        let (doc_arc, dirty_arc, set_mark_dirty) =
            mem_overwrite_or_create_document(gcx.clone(), doc).await;
        (doc_arc, dirty_arc, set_mark_dirty)
    };

    if mark_dirty {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        *dirty_arc.lock().await = now;
    }

    *gcx.documents_state.active_file_path.lock().await = Some(path.clone());

    let mut go_ahead = true;
    {
        let is_it_good = is_valid_file(&normalized_path, false, false);
        if is_it_good.is_err() {
            info!(
                "{:?} ignoring changes: {}",
                normalized_path,
                is_it_good.err().unwrap()
            );
            go_ahead = false;
        }
    }

    let cpath = doc_arc
        .read()
        .await
        .doc_path
        .clone()
        .to_string_lossy()
        .to_string();
    if go_ahead {
        enqueue_some_docs(gcx.clone(), &vec![cpath], false).await;
    }

    info!(
        "on_did_change {}, total time {:.3}s",
        crate::nicer_logs::last_n_chars(&path.to_string_lossy().to_string(), 30),
        t0.elapsed().as_secs_f32()
    );
}

pub async fn on_did_delete(gcx: Arc<GlobalContext>, path: &PathBuf) {
    if path_is_refact_internal(path) {
        return;
    }
    gcx.file_index.refresh(path);
    info!(
        "on_did_delete {}",
        crate::nicer_logs::last_n_chars(&path.to_string_lossy().to_string(), 30)
    );

    let (vec_db_module, codegraph, dirty_arc) = {
        let cx = gcx.clone();
        remove_memory_document_for_path(cx.clone(), path).await;
        let codegraph = cx.codegraph.lock().await.clone();
        (
            cx.vec_db.clone(),
            codegraph,
            cx.documents_state.cache_dirty.clone(),
        )
    };

    crate::files_correction::mark_files_cache_dirty(&dirty_arc).await;

    let delete_path = normalize_path_for_index_store(&gcx, path);

    let vec_db = vec_db_module.lock().await.clone();
    match vec_db {
        Some(db) => match db.remove_file(&delete_path).await {
            Ok(_) => {}
            Err(err) => info!("VECDB Error removing: {}", err),
        },
        None => {}
    }
    if let Some(service) = &codegraph {
        let _ = service
            .remove_path(&delete_path.to_string_lossy().to_string())
            .await;
    }
}

pub async fn add_folder(gcx: Arc<GlobalContext>, fpath: &PathBuf) {
    let canonical_path =
        crate::files_correction::canonical_path(fpath.to_string_lossy().to_string());
    let was_added = {
        let documents_state = &gcx.documents_state;
        let mut folders = documents_state.workspace_folders.lock().unwrap();
        if folders.iter().any(|p| *p == canonical_path) {
            false
        } else {
            folders.push(canonical_path.clone());
            true
        }
    };
    if was_added {
        gcx.enrichment_generation.fetch_add(1, Ordering::SeqCst);
        gcx.knowledge_generation.fetch_add(1, Ordering::SeqCst);
        tracing::info!("Added folder {} to workspace", canonical_path.display());
        on_workspaces_init(gcx.clone()).await;
    } else {
        tracing::debug!(
            "Folder {} already in workspace, skipping",
            canonical_path.display()
        );
    }
}

pub async fn remove_folder(gcx: Arc<GlobalContext>, path: &PathBuf) {
    let canonical_path =
        crate::files_correction::canonical_path(path.to_string_lossy().to_string());
    let was_removed = {
        let documents_state = &gcx.documents_state;
        let mut folders = documents_state.workspace_folders.lock().unwrap();
        let before = folders.len();
        folders.retain(|p| *p != canonical_path && *p != *path);
        folders.len() < before
    };
    if was_removed {
        gcx.enrichment_generation.fetch_add(1, Ordering::SeqCst);
        gcx.knowledge_generation.fetch_add(1, Ordering::SeqCst);
        tracing::info!("Removed folder {} from workspace", path.display());
        on_workspaces_init(gcx.clone()).await;
    } else {
        tracing::debug!("Folder {} not found in workspace, skipping", path.display());
    }
}

fn read_git_head(repo_path: &Path) -> Option<String> {
    let head_path = repo_path.join(".git").join("HEAD");
    if let Some(head) = std::fs::read_to_string(&head_path)
        .ok()
        .map(|s| s.trim().to_string())
    {
        return Some(head);
    }
    let repository = git2::Repository::open(repo_path).ok()?;
    repository
        .head()
        .ok()
        .and_then(|head| head.target().map(|oid| oid.to_string()))
}

fn update_git_branch_heads_for_roots(gcx: &Arc<GlobalContext>, repo_paths: &[PathBuf]) {
    let repo_paths = repo_paths
        .iter()
        .map(|path| canonical_path(path.to_string_lossy()))
        .collect::<HashSet<_>>();
    let mut heads = gcx.documents_state.git_branch_heads.lock().unwrap();
    heads.retain(|repo_path, _| repo_paths.contains(repo_path));
    for repo_path in repo_paths {
        if let Some(head) = read_git_head(&repo_path) {
            heads.insert(repo_path, head);
        } else {
            heads.remove(&repo_path);
        }
    }
}

fn is_git_head_path(p: &Path) -> bool {
    p.file_name().map(|n| n == "HEAD").unwrap_or(false)
        && p.parent()
            .and_then(|pp| pp.file_name())
            .map(|n| n == ".git")
            .unwrap_or(false)
}

#[cfg(not(test))]
const FILE_EVENT_DEBOUNCE_WINDOW: Duration = Duration::from_millis(500);
#[cfg(test)]
const FILE_EVENT_DEBOUNCE_WINDOW: Duration = Duration::from_millis(100);
const FILE_EVENT_DEBOUNCE_RETAIN: Duration = Duration::from_secs(10);
const MAX_IN_FLIGHT_WATCH_REGISTRATIONS: usize = 64;
#[cfg(not(test))]
const BRANCH_HEAD_DEBOUNCE_WINDOW: Duration = Duration::from_secs(2);
#[cfg(test)]
const BRANCH_HEAD_DEBOUNCE_WINDOW: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
struct BranchHeadChange {
    repo_path: PathBuf,
    old_head: Option<String>,
    new_head: Option<String>,
}

fn resolve_head_text_to_oid(
    repository: &git2::Repository,
    head_text: &str,
) -> Result<git2::Oid, String> {
    let head_text = head_text.trim();
    if let Some(ref_name) = head_text.strip_prefix("ref:") {
        let ref_name = ref_name.trim();
        return repository
            .refname_to_id(ref_name)
            .or_else(|_| repository.revparse_single(ref_name).map(|obj| obj.id()))
            .map_err(|e| format!("resolve {ref_name}: {e}"));
    }
    git2::Oid::from_str(head_text)
        .or_else(|_| repository.revparse_single(head_text).map(|obj| obj.id()))
        .map_err(|e| format!("resolve {head_text}: {e}"))
}

fn changed_files_between_heads(
    repo_path: &Path,
    old_head: Option<&str>,
    new_head: Option<&str>,
) -> Result<Vec<PathBuf>, String> {
    let old_head = old_head.ok_or_else(|| "previous HEAD is unknown".to_string())?;
    let new_head = new_head.ok_or_else(|| "new HEAD is unknown".to_string())?;
    if old_head == new_head {
        return Ok(Vec::new());
    }
    let repository = git2::Repository::open(repo_path).map_err(|e| format!("git open: {e}"))?;
    let old_oid = resolve_head_text_to_oid(&repository, old_head)?;
    let new_oid = resolve_head_text_to_oid(&repository, new_head)?;
    if old_oid == new_oid {
        return Ok(Vec::new());
    }
    let old_tree = repository
        .find_object(old_oid, None)
        .and_then(|obj| obj.peel_to_tree())
        .map_err(|e| format!("read old tree: {e}"))?;
    let new_tree = repository
        .find_object(new_oid, None)
        .and_then(|obj| obj.peel_to_tree())
        .map_err(|e| format!("read new tree: {e}"))?;
    let mut diff_options = git2::DiffOptions::new();
    diff_options.include_typechange(true);
    let mut diff = repository
        .diff_tree_to_tree(Some(&old_tree), Some(&new_tree), Some(&mut diff_options))
        .map_err(|e| format!("git diff: {e}"))?;
    let mut find_options = git2::DiffFindOptions::new();
    find_options
        .renames(true)
        .rename_threshold(50)
        .rename_limit(200);
    if let Err(err) = diff.find_similar(Some(&mut find_options)) {
        tracing::debug!("branch switch rename detection skipped: {err}");
    }

    let mut paths = IndexSet::new();
    for delta in diff.deltas() {
        match delta.status() {
            git2::Delta::Added | git2::Delta::Copied => {
                if let Some(path) = delta.new_file().path() {
                    paths.insert(canonical_path(repo_path.join(path).to_string_lossy()));
                }
            }
            git2::Delta::Deleted => {
                if let Some(path) = delta.old_file().path() {
                    paths.insert(canonical_path(repo_path.join(path).to_string_lossy()));
                }
            }
            git2::Delta::Renamed => {
                if let Some(path) = delta.old_file().path() {
                    paths.insert(canonical_path(repo_path.join(path).to_string_lossy()));
                }
                if let Some(path) = delta.new_file().path() {
                    paths.insert(canonical_path(repo_path.join(path).to_string_lossy()));
                }
            }
            _ => {
                if let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) {
                    paths.insert(canonical_path(repo_path.join(path).to_string_lossy()));
                }
            }
        }
    }
    Ok(paths.into_iter().collect())
}

fn record_git_head_change(gcx: &Arc<GlobalContext>, repo_path: &Path) -> Option<BranchHeadChange> {
    let repo_path = canonical_path(repo_path.to_string_lossy());
    let new_head = read_git_head(&repo_path);
    let mut heads = gcx.documents_state.git_branch_heads.lock().unwrap();
    let old_head = heads.get(&repo_path).cloned();
    if new_head == old_head {
        return None;
    }
    tracing::info!(
        "git HEAD changed in {}: {:?} -> {:?}",
        repo_path.display(),
        old_head,
        new_head
    );
    match &new_head {
        Some(h) => {
            heads.insert(repo_path.clone(), h.clone());
        }
        None => {
            heads.remove(&repo_path);
        }
    }
    Some(BranchHeadChange {
        repo_path,
        old_head,
        new_head,
    })
}

fn record_pending_git_head_change(
    gcx: &Arc<GlobalContext>,
    repo_path: &Path,
    now: Instant,
) -> Option<PathBuf> {
    let repo_path = canonical_path(repo_path.to_string_lossy());
    let new_head = read_git_head(&repo_path);
    let old_head = {
        let mut heads = gcx.documents_state.git_branch_heads.lock().unwrap();
        let old_head = heads.get(&repo_path).cloned();
        if new_head == old_head {
            return None;
        }
        tracing::info!(
            "git HEAD changed in {}: {:?} -> {:?}",
            repo_path.display(),
            old_head,
            new_head
        );
        match &new_head {
            Some(h) => {
                heads.insert(repo_path.clone(), h.clone());
            }
            None => {
                heads.remove(&repo_path);
            }
        }
        old_head
    };
    let mut pending = gcx.documents_state.branch_head_debounce.lock().unwrap();
    pending
        .entry(repo_path.clone())
        .and_modify(|change| {
            change.new_head = new_head.clone();
            change.last_event = now;
        })
        .or_insert(PendingBranchHeadChange {
            old_head,
            new_head,
            last_event: now,
        });
    Some(repo_path)
}

async fn enqueue_branch_head_changes(gcx: Arc<GlobalContext>, changes: Vec<BranchHeadChange>) {
    let mut docs = IndexSet::new();
    for change in &changes {
        match changed_files_between_heads(
            &change.repo_path,
            change.old_head.as_deref(),
            change.new_head.as_deref(),
        ) {
            Ok(paths) => {
                docs.extend(paths.into_iter().map(|p| p.to_string_lossy().to_string()));
            }
            Err(err) => {
                tracing::warn!(
                    "Branch switch diff failed for {}: {}; triggering full workspace reindex",
                    change.repo_path.display(),
                    err
                );
                enqueue_all_files_from_workspace_folders(gcx, true, false).await;
                return;
            }
        }
    }
    let docs = docs.into_iter().collect::<Vec<_>>();
    tracing::info!(
        "Branch switch detected, enqueueing {} changed file(s)",
        docs.len()
    );
    for doc in &docs {
        let path = PathBuf::from(doc);
        if !path.exists() {
            on_did_delete(gcx.clone(), &path).await;
        }
    }
    enqueue_some_docs(gcx, &docs, true).await;
}

fn schedule_branch_head_debounce(gcx: Arc<GlobalContext>, repo_path: PathBuf) {
    let mut tasks = gcx
        .documents_state
        .branch_head_debounce_tasks
        .lock()
        .unwrap();
    if tasks.contains_key(&repo_path) {
        return;
    }
    let task_gcx = gcx.clone();
    let task_repo_path = repo_path.clone();
    let handle = tokio::spawn(async move {
        flush_branch_head_debounce(task_gcx, task_repo_path).await;
    });
    tasks.insert(repo_path, handle);
}

async fn flush_branch_head_debounce(gcx: Arc<GlobalContext>, repo_path: PathBuf) {
    loop {
        if gcx.shutdown_flag.load(Ordering::Relaxed) {
            gcx.documents_state
                .branch_head_debounce_tasks
                .lock()
                .unwrap()
                .remove(&repo_path);
            return;
        }
        tokio::time::sleep(BRANCH_HEAD_DEBOUNCE_WINDOW).await;
        if gcx.shutdown_flag.load(Ordering::Relaxed) {
            gcx.documents_state
                .branch_head_debounce_tasks
                .lock()
                .unwrap()
                .remove(&repo_path);
            return;
        }
        let change = {
            let now = Instant::now();
            let mut pending = gcx.documents_state.branch_head_debounce.lock().unwrap();
            let Some(existing) = pending.get(&repo_path) else {
                gcx.documents_state
                    .branch_head_debounce_tasks
                    .lock()
                    .unwrap()
                    .remove(&repo_path);
                return;
            };
            if now.saturating_duration_since(existing.last_event) < BRANCH_HEAD_DEBOUNCE_WINDOW {
                continue;
            }
            let pending_change = pending.remove(&repo_path).unwrap();
            gcx.documents_state
                .branch_head_debounce_tasks
                .lock()
                .unwrap()
                .remove(&repo_path);
            pending_change
        };
        if change.old_head != change.new_head {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            gcx.documents_state
                .branch_reindex_last_ts
                .store(now_ms, Ordering::Relaxed);
            enqueue_branch_head_changes(
                gcx,
                vec![BranchHeadChange {
                    repo_path,
                    old_head: change.old_head,
                    new_head: change.new_head,
                }],
            )
            .await;
        }
        return;
    }
}

async fn on_git_head_change(gcx_weak: Weak<GlobalContext>, event: Event) {
    let gcx = match gcx_weak.upgrade() {
        Some(gcx) => gcx,
        None => return,
    };

    let repo_paths: Vec<PathBuf> = event
        .paths
        .iter()
        .filter(|p| is_git_head_path(p))
        .filter_map(|p| p.parent()?.parent())
        .map(|p| canonical_path(p.to_string_lossy()))
        .collect();

    if repo_paths.is_empty() {
        return;
    }

    let now = Instant::now();
    for repo_path in repo_paths {
        if let Some(repo_path) = record_pending_git_head_change(&gcx, &repo_path, now) {
            schedule_branch_head_debounce(gcx.clone(), repo_path);
        }
    }
}

pub async fn on_explicit_branch_change(gcx: Arc<GlobalContext>, repo_path: &PathBuf) {
    let change = record_git_head_change(&gcx, repo_path);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    gcx.documents_state
        .branch_reindex_last_ts
        .store(now_ms, Ordering::Relaxed);

    if let Some(change) = change {
        tracing::info!(
            "Explicit branch change notification for {}, enqueueing changed files",
            change.repo_path.display()
        );
        enqueue_branch_head_changes(gcx, vec![change]).await;
    }
}

fn schedule_debounced_file_event(
    gcx: Arc<GlobalContext>,
    raw_path: PathBuf,
    is_remove_event: bool,
    now: Instant,
) {
    {
        let mut debounce = gcx.documents_state.file_event_debounce.lock().unwrap();
        debounce.retain(|_, event| {
            now.saturating_duration_since(event.at) <= FILE_EVENT_DEBOUNCE_RETAIN
        });
        debounce.insert(
            raw_path,
            DebouncedFileEvent {
                at: now,
                is_remove_event,
            },
        );
    }

    let mut task = gcx.documents_state.file_event_debounce_task.lock().unwrap();
    let should_spawn = task
        .as_ref()
        .map(|handle| handle.is_finished())
        .unwrap_or(true);
    if should_spawn {
        let task_gcx = gcx.clone();
        *task = Some(tokio::spawn(async move {
            flush_debounced_file_event_worker(task_gcx).await;
        }));
    }
    drop(task);
    gcx.documents_state.file_event_debounce_notify.notify_one();
}

fn take_all_debounced_file_events(gcx: &Arc<GlobalContext>) -> Vec<(PathBuf, bool)> {
    let mut debounce = gcx.documents_state.file_event_debounce.lock().unwrap();
    let mut events = debounce
        .drain()
        .map(|(raw_path, event)| (raw_path, event.is_remove_event))
        .collect::<Vec<_>>();
    events.sort();
    events
}

fn take_due_debounced_file_events(
    gcx: &Arc<GlobalContext>,
    now: Instant,
) -> (Vec<(PathBuf, bool)>, Option<Duration>) {
    let mut debounce = gcx.documents_state.file_event_debounce.lock().unwrap();
    debounce
        .retain(|_, event| now.saturating_duration_since(event.at) <= FILE_EVENT_DEBOUNCE_RETAIN);
    let mut due_paths = Vec::new();
    let mut wait_for: Option<Duration> = None;
    for (raw_path, event) in debounce.iter() {
        let elapsed = now.saturating_duration_since(event.at);
        if elapsed >= FILE_EVENT_DEBOUNCE_WINDOW {
            due_paths.push(raw_path.clone());
        } else {
            let remaining = FILE_EVENT_DEBOUNCE_WINDOW.saturating_sub(elapsed);
            wait_for = Some(wait_for.map_or(remaining, |current| current.min(remaining)));
        }
    }
    let mut due = Vec::new();
    for path in due_paths {
        if let Some(event) = debounce.remove(&path) {
            due.push((path, event.is_remove_event));
        }
    }
    due.sort();
    (due, wait_for)
}

async fn blocklist_roots_for_file_events(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut roots = gcx
        .documents_state
        .workspace_vcs_roots
        .lock()
        .unwrap()
        .clone();
    roots.extend(crate::files_correction::get_unscoped_project_dirs(gcx.clone()).await);
    let mut roots = roots
        .into_iter()
        .map(crate::files_correction::canonicalize_normalized_path)
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots
}

async fn resolve_debounced_file_events(
    gcx: Arc<GlobalContext>,
    raw_events: Vec<(PathBuf, bool)>,
) -> Vec<PendingFileEvent> {
    if raw_events.is_empty() {
        return Vec::new();
    }
    let indexing_everywhere = reload_indexing_everywhere_if_needed(gcx.clone()).await;
    let blocklist_roots = blocklist_roots_for_file_events(gcx.clone()).await;
    let worktree_mappings =
        crate::files_correction::registered_worktree_path_mappings(gcx.cache_dir.as_path());
    let global_config_roots = vec![gcx.config_dir.clone()];

    let mut resolved: HashMap<PathBuf, PendingFileEvent> = HashMap::new();
    for (raw_path, is_remove_event) in raw_events {
        gcx.documents_state
            .file_event_resolutions
            .fetch_add(1, Ordering::Relaxed);
        let queue_path = resolve_codegraph_queue_path(&raw_path, &worktree_mappings);
        let store_path = PathBuf::from(&queue_path.store_path);
        let read_path = PathBuf::from(&queue_path.read_path);
        let indexing_settings = indexing_everywhere.indexing_for_path(&store_path);
        let blocklist_path = path_for_blocklist(&store_path, &blocklist_roots);
        if is_blocklisted(&indexing_settings, &blocklist_path) {
            continue;
        }
        if crate::file_filter::is_generated_index_path_with_global_config_roots(
            &store_path,
            &global_config_roots,
        ) {
            continue;
        }
        let Some(kind) = debounced_file_event_kind_for_path(
            is_remove_event,
            &read_path,
            &store_path,
            &blocklist_roots,
            &global_config_roots,
        ) else {
            continue;
        };
        resolved.insert(
            store_path.clone(),
            PendingFileEvent {
                store_path,
                read_path,
                kind,
            },
        );
    }

    let mut events = resolved.into_values().collect::<Vec<_>>();
    events.sort_by(|a, b| {
        a.kind_order()
            .cmp(&b.kind_order())
            .then_with(|| a.store_path.cmp(&b.store_path))
    });
    events
}

fn finish_file_event_debounce_worker(gcx: &Arc<GlobalContext>) -> bool {
    let debounce = gcx.documents_state.file_event_debounce.lock().unwrap();
    if debounce.is_empty() {
        *gcx.documents_state.file_event_debounce_task.lock().unwrap() = None;
        true
    } else {
        false
    }
}

async fn wait_for_shutdown_flag(shutdown_flag: Arc<AtomicBool>) {
    while !shutdown_flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_file_debounce_signal(gcx: &Arc<GlobalContext>, wait_for: Option<Duration>) {
    let notify = gcx.documents_state.file_event_debounce_notify.clone();
    let shutdown_flag = gcx.shutdown_flag.clone();
    match wait_for {
        Some(wait_for) => {
            tokio::select! {
                _ = tokio::time::sleep(wait_for) => {}
                _ = notify.notified() => {}
                _ = wait_for_shutdown_flag(shutdown_flag) => {}
            }
        }
        None => {
            tokio::select! {
                _ = notify.notified() => {}
                _ = wait_for_shutdown_flag(shutdown_flag) => {}
            }
        }
    }
}

async fn flush_debounced_file_paths(gcx: Arc<GlobalContext>, events: Vec<PendingFileEvent>) {
    if events.is_empty() {
        return;
    }
    for event in &events {
        if matches!(event.kind, DebouncedFileEventKind::Remove) || !event.read_path.exists() {
            on_did_delete(gcx.clone(), &event.store_path).await;
        }
    }
    let docs = events
        .into_iter()
        .map(|event| event.queued_path())
        .collect::<Vec<_>>();
    enqueue_some_docs_with_read_paths(gcx, &docs, false).await;
}

async fn flush_debounced_file_event_worker(gcx: Arc<GlobalContext>) {
    loop {
        if gcx.shutdown_flag.load(Ordering::Relaxed) {
            let raw_events = take_all_debounced_file_events(&gcx);
            let events = resolve_debounced_file_events(gcx.clone(), raw_events).await;
            flush_debounced_file_paths(gcx.clone(), events).await;
            if finish_file_event_debounce_worker(&gcx) {
                return;
            }
            continue;
        }

        let (raw_events, wait_for) = take_due_debounced_file_events(&gcx, Instant::now());
        if !raw_events.is_empty() {
            let events = resolve_debounced_file_events(gcx.clone(), raw_events).await;
            flush_debounced_file_paths(gcx.clone(), events).await;
            continue;
        }

        if gcx
            .documents_state
            .file_event_debounce
            .lock()
            .unwrap()
            .is_empty()
        {
            if finish_file_event_debounce_worker(&gcx) {
                return;
            }
            continue;
        }
        wait_for_file_debounce_signal(&gcx, wait_for).await;
    }
}

fn subtree_scan_target(event: &Event) -> Option<PathBuf> {
    let path = match &event.kind {
        EventKind::Create(_) => event.paths.first(),
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => None,
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => event.paths.first(),
        EventKind::Modify(ModifyKind::Name(_)) if event.paths.len() == 2 => event.paths.get(1),
        EventKind::Modify(ModifyKind::Name(_)) => event.paths.first(),
        _ => None,
    }?;
    std::fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_dir())
        .then(|| path.clone())
}

pub async fn file_watcher_event(event: Event, gcx_weak: Weak<GlobalContext>) {
    async fn update_directory_watch_registration(
        gcx: Arc<GlobalContext>,
        indexing_everywhere: Arc<IndexingEverywhere>,
        path: PathBuf,
        should_watch: bool,
    ) {
        let indexing_everywhere = indexing_everywhere.as_ref();
        let path = path.as_path();
        let watcher_lock = {
            gcx.documents_state
                .fs_watcher
                .lock()
                .unwrap()
                .as_ref()
                .cloned()
        };
        let Some(watcher_lock) = watcher_lock else {
            return;
        };

        let canonical_path =
            crate::files_correction::canonicalize_normalized_path(path.to_path_buf());
        if should_watch {
            let mut watch_folders = gcx
                .documents_state
                .workspace_vcs_roots
                .lock()
                .unwrap()
                .clone();
            watch_folders.extend(
                gcx.documents_state
                    .workspace_folders
                    .lock()
                    .unwrap()
                    .clone(),
            );
            let unscoped_watch_folders =
                crate::files_correction::get_unscoped_project_dirs(gcx.clone()).await;
            watch_folders.extend(unscoped_watch_folders);
            let watch_folders = watch_folders
                .into_iter()
                .map(crate::files_correction::canonicalize_normalized_path)
                .collect::<Vec<_>>();
            let scan_root = watch_folders
                .iter()
                .filter(|root| canonical_path.starts_with(root))
                .max_by_key(|root| root.components().count())
                .cloned();
            let Some(scan_root) = scan_root else {
                return;
            };
            let registration =
                match classify_watch_subtree(&scan_root, &canonical_path, indexing_everywhere) {
                    WatchSubtreeClass::Excluded => None,
                    WatchSubtreeClass::CleanSubtree => Some(WatchRegistration {
                        path: canonical_path,
                        mode: WatchRegistrationMode::Recursive,
                    }),
                    WatchSubtreeClass::Mixed => Some(WatchRegistration {
                        path: canonical_path,
                        mode: WatchRegistrationMode::NonRecursive,
                    }),
                };
            if let Some(registration) = registration {
                register_watch_path(&watcher_lock, &registration).await;
            }
        } else {
            unregister_watch_path(&watcher_lock, &canonical_path).await;
        }
    }

    fn spawn_watch_registration(
        gcx: Arc<GlobalContext>,
        indexing_everywhere: Arc<IndexingEverywhere>,
        path: PathBuf,
        should_watch: bool,
    ) {
        let tasks = gcx.documents_state.watch_registration_tasks.clone();
        let drops = gcx.documents_state.watch_registration_drops.clone();
        let mut tasks = tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        tasks.retain(|task| !task.is_finished());
        if tasks.len() >= MAX_IN_FLIGHT_WATCH_REGISTRATIONS {
            let dropped = drops.fetch_add(1, Ordering::Relaxed) + 1;
            if dropped == 1 || dropped % 100 == 0 {
                tracing::warn!(
                    "File watcher skipped {} directory watch updates because {} subtree rescans are already in flight",
                    dropped,
                    MAX_IN_FLIGHT_WATCH_REGISTRATIONS
                );
            }
            return;
        }
        tasks.push(tokio::spawn(update_directory_watch_registration(
            gcx,
            indexing_everywhere,
            path,
            should_watch,
        )));
    }

    async fn on_file_change(gcx_weak: Weak<GlobalContext>, event: Event) {
        let gcx = match gcx_weak.clone().upgrade() {
            Some(gcx) => gcx,
            None => return,
        };
        // Inventory receives all names, including extensionless files and empty folders,
        // before the source-code indexing filters. A directory move may contain unseen
        // descendants: report uncertainty until the next bounded inventory rebuild.
        let structural = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
        );
        if gcx.file_index.observe_event(&event.paths, structural) {
            let roots = crate::files_correction::get_unscoped_project_dirs(gcx.clone()).await;
            if let Some(dir) = subtree_scan_target(&event) {
                if let Some(task) = crate::file_index::request_subtree_scan(
                    gcx.file_index.clone(),
                    dir,
                    roots.clone(),
                    gcx.shutdown_flag.clone(),
                ) {
                    let mut tasks = gcx.documents_state.watch_registration_tasks.lock().unwrap();
                    tasks.retain(|task| !task.is_finished());
                    tasks.push(task);
                }
            }
            if let Some(task) = crate::file_index::spawn_build(
                gcx.file_index.clone(),
                roots,
                gcx.shutdown_flag.clone(),
            ) {
                let mut tasks = gcx.documents_state.watch_registration_tasks.lock().unwrap();
                tasks.retain(|task| !task.is_finished());
                tasks.push(task);
            }
        }
        let maybe_created_dir = match &event.kind {
            EventKind::Create(CreateKind::Folder | CreateKind::Any) => event.paths.first().cloned(),
            EventKind::Modify(ModifyKind::Name(_)) => event.paths.last().cloned(),
            _ => None,
        }
        .filter(|path| path.is_dir());
        let maybe_removed_dir = match &event.kind {
            EventKind::Remove(RemoveKind::Folder) => event.paths.first().cloned(),
            EventKind::Modify(ModifyKind::Name(_))
                if event.paths.len() == 2 && event.paths[1].is_dir() =>
            {
                event.paths.first().cloned()
            }
            _ => None,
        };
        if maybe_created_dir.is_some() || maybe_removed_dir.is_some() {
            let indexing_everywhere_arc = reload_indexing_everywhere_if_needed(gcx.clone()).await;
            if let Some(path) = maybe_removed_dir {
                spawn_watch_registration(gcx.clone(), indexing_everywhere_arc.clone(), path, false);
            }
            if let Some(path) = maybe_created_dir {
                spawn_watch_registration(gcx.clone(), indexing_everywhere_arc, path, true);
            }
        }
        if event.paths.iter().any(|p| path_triggers_registry_reload(p)) {
            crate::yaml_configs::customization_registry::invalidate_all_registry_caches(
                gcx.clone(),
            )
            .await;
        }
        if event
            .paths
            .iter()
            .any(|p| path_triggers_tool_catalog_reload(p))
        {
            if event.paths.iter().any(|p| {
                p.components()
                    .any(|component| component == Component::Normal("integrations.d".as_ref()))
            }) {
                gcx.tool_catalog_generations.advance_integrations();
            }
            if event
                .paths
                .iter()
                .any(|path| path.file_name().is_some_and(|name| name == "privacy.yaml"))
            {
                gcx.tool_catalog_generations.advance_privacy();
            }
        }
        if event
            .paths
            .iter()
            .any(|path| !path_is_refact_internal(path))
        {
            gcx.enrichment_generation.fetch_add(1, Ordering::SeqCst);
        }
        if event
            .paths
            .iter()
            .any(|path| !path_is_refact_internal(path) && path_is_knowledge_document(path))
        {
            gcx.knowledge_generation.fetch_add(1, Ordering::SeqCst);
        }
        let debounce_now = Instant::now();
        let is_remove_event = matches!(&event.kind, EventKind::Remove(_));
        for p in &event.paths {
            if path_is_refact_internal(p) {
                continue;
            }
            schedule_debounced_file_event(gcx.clone(), p.clone(), is_remove_event, debounce_now);
        }
    }
    async fn on_dot_git_dir_change(gcx_weak: Weak<GlobalContext>, event: Event) {
        if let Some(gcx) = gcx_weak.clone().upgrade() {
            // Get the path before .git component, and check if repo associated exists
            let repo_paths = event
                .paths
                .iter()
                .filter_map(|p| {
                    p.components()
                        .position(|c| c == Component::Normal(".git".as_ref()))
                        .map(|i| {
                            let repo_p = p.components().take(i).collect::<PathBuf>();
                            canonical_path(repo_p.to_string_lossy())
                        })
                })
                .map(|p| {
                    let exists = p.join(".git").exists();
                    (p.clone(), exists)
                })
                .collect::<Vec<_>>();

            if repo_paths.is_empty() {
                return;
            }

            let workspace_vcs_roots = gcx.documents_state.workspace_vcs_roots.clone();

            let mut should_reindex = false;
            {
                let mut workspace_vcs_roots_locked = workspace_vcs_roots.lock().unwrap();
                for (repo_path, exists_in_disk) in repo_paths {
                    if exists_in_disk && !workspace_vcs_roots_locked.contains(&repo_path) {
                        tracing::info!(
                            "Found .git folder in workspace: {}",
                            repo_path.to_string_lossy()
                        );
                        should_reindex = true;
                        workspace_vcs_roots_locked.push(repo_path);
                    } else if !exists_in_disk && workspace_vcs_roots_locked.contains(&repo_path) {
                        tracing::info!(
                            "Removed .git folder from workspace: {}",
                            repo_path.to_string_lossy()
                        );
                        should_reindex = true;
                        workspace_vcs_roots_locked.retain(|p| p != &repo_path);
                    }
                }
            }

            if should_reindex {
                tracing::info!("Reindexing all files");
                enqueue_all_files_from_workspace_folders(gcx, false, false).await;
            }
        }
    }

    match event.kind {
        // We may receive specific event that a folder is being added/removed, but not the .git itself, this happens on Unix systems
        EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder)
            if event.paths.iter().any(|p| {
                p.components()
                    .any(|c| c == Component::Normal(".git".as_ref()))
            }) =>
        {
            on_dot_git_dir_change(gcx_weak.clone(), event).await
        }

        // In Windows, we receive generic events (Any subtype), but we receive them about each exact folder
        EventKind::Create(CreateKind::Any)
        | EventKind::Modify(ModifyKind::Any)
        | EventKind::Remove(RemoveKind::Any)
            if event.paths.iter().any(|p| p.ends_with(".git")) =>
        {
            on_dot_git_dir_change(gcx_weak, event).await
        }

        EventKind::Create(_)
        | EventKind::Modify(_)
        | EventKind::Remove(_)
        | EventKind::Access(AccessKind::Close(AccessMode::Write))
            if event.paths.iter().any(|p| is_git_head_path(p)) =>
        {
            on_git_head_change(gcx_weak.clone(), event).await
        }

        EventKind::Create(_)
        | EventKind::Modify(_)
        | EventKind::Remove(_)
        | EventKind::Access(AccessKind::Close(AccessMode::Write)) => {
            on_file_change(gcx_weak.clone(), event).await
        }

        EventKind::Other | EventKind::Any | EventKind::Access(_) => {}
    }
}

pub async fn files_in_workspace_init_task(gcx: Arc<GlobalContext>) {
    let previous_folders = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    let ev = crate::buddy::actor::make_runtime_event(
        "indexing",
        "Indexing project files...",
        "indexer",
        "indexing",
        "started",
        None,
    );
    crate::buddy::actor::buddy_enqueue_event(
        crate::app_state::AppState::from_gcx(gcx.clone()).await,
        ev,
    )
    .await;
    let file_count = enqueue_all_files_from_workspace_folders(gcx.clone(), true, false).await;
    let current_folders = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    let added = current_folders
        .iter()
        .filter(|folder| !previous_folders.contains(folder))
        .map(|folder| folder.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let removed = previous_folders
        .iter()
        .filter(|folder| !current_folders.contains(folder))
        .map(|folder| folder.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if !added.is_empty() || !removed.is_empty() {
        let user_activity = gcx.user_activity.clone();
        if let Ok(mut ring) = user_activity.try_lock() {
            ring.push(UserAction::WorkspaceChanged {
                folders_added: added,
                folders_removed: removed,
                ts: Utc::now(),
            });
        };
    }
    enqueue_all_docs_from_jsonl_but_read_first(gcx.clone(), true, false).await;
    crate::git::checkpoints::enqueue_init_shadow_repos(gcx.clone()).await;
    let ev = crate::buddy::actor::make_runtime_event(
        "indexing",
        &format!("Workspace indexed: {} files", file_count),
        "indexer",
        "indexing",
        "completed",
        None,
    );
    crate::buddy::actor::buddy_enqueue_event(crate::app_state::AppState::from_gcx(gcx).await, ev)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration_modes(
        plan: &[WatchRegistration],
        root: &Path,
    ) -> Vec<(PathBuf, WatchRegistrationMode)> {
        let mut entries = plan
            .iter()
            .map(|registration| {
                (
                    registration
                        .path
                        .strip_prefix(root)
                        .unwrap_or(registration.path.as_path())
                        .to_path_buf(),
                    registration.mode,
                )
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    fn plan_covers(plan: &[WatchRegistration], path: &Path) -> bool {
        plan.iter().any(|registration| match registration.mode {
            WatchRegistrationMode::Recursive => path.starts_with(&registration.path),
            WatchRegistrationMode::NonRecursive => {
                path == registration.path || path.parent() == Some(registration.path.as_path())
            }
        })
    }

    fn write_file(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn normalized(path: &Path) -> PathBuf {
        crate::files_correction::canonical_path(path.to_string_lossy().to_string())
    }

    fn worktree_root(cache_dir: &Path, source: &Path, id: &str) -> PathBuf {
        let source = normalized(source);
        cache_dir
            .join("worktrees")
            .join(refact_worktrees::service::project_hash_for_path(&source))
            .join(id)
    }

    fn write_worktree_registry(cache_dir: &Path, source: &Path, worktree: &Path) {
        let source = normalized(source);
        let worktree = normalized(worktree);
        let hash = refact_worktrees::service::project_hash_for_path(&source);
        let registry_dir = cache_dir.join("worktrees").join(&hash);
        std::fs::create_dir_all(&registry_dir).unwrap();
        let registry = refact_worktrees::types::WorktreeRegistry {
            schema_version: 1,
            source_workspace_root: source.clone(),
            project_hash: hash,
            records: vec![refact_worktrees::types::WorktreeRegistryRecord {
                creator_chat_id: None,
                meta: refact_worktrees::types::WorktreeMeta {
                    id: "wt".to_string(),
                    kind: "chat".to_string(),
                    root: worktree,
                    source_workspace_root: source.clone(),
                    repo_root: source,
                    branch: Some("refact/chat/test".to_string()),
                    base_branch: Some("main".to_string()),
                    base_commit: None,
                    task_id: None,
                    card_id: None,
                    agent_id: None,
                    enforce: true,
                },
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                last_seen_at: None,
                references: Vec::new(),
                last_known_status: None,
            }],
        };
        std::fs::write(
            registry_dir.join("index.json"),
            serde_json::to_string_pretty(&registry).unwrap(),
        )
        .unwrap();
    }

    async fn scan_workspace(root: &Path) -> Vec<PathBuf> {
        let mut indexing_everywhere = IndexingEverywhere::default();
        let (files, _) = retrieve_files_in_workspace_folders(
            vec![root.to_path_buf()],
            &mut indexing_everywhere,
            false,
            false,
            &[],
        )
        .await;
        files
    }

    async fn scan_workspace_with_global_config_root(
        root: &Path,
        global_config_root: &Path,
    ) -> Vec<PathBuf> {
        let mut indexing_everywhere = IndexingEverywhere::default();
        let roots = vec![global_config_root.to_path_buf()];
        let (files, _) = retrieve_files_in_workspace_folders(
            vec![root.to_path_buf()],
            &mut indexing_everywhere,
            false,
            false,
            &roots,
        )
        .await;
        files
    }

    async fn cache_dirty_value(gcx: &Arc<GlobalContext>) -> f64 {
        let dirty = { gcx.documents_state.cache_dirty.clone() };
        let value = *dirty.lock().await;
        value
    }

    fn allow_all_privacy(gcx: &Arc<GlobalContext>) {
        let loaded_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 60;
        *gcx.privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                only_send_to_servers_I_control: Vec::new(),
                blocked: Vec::new(),
            },
            loaded_ts,
        });
    }

    fn set_blocked_privacy(gcx: &Arc<GlobalContext>, blocked: Vec<String>) {
        *gcx.privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                only_send_to_servers_I_control: Vec::new(),
                blocked,
            },
            loaded_ts: u64::MAX / 2,
        });
    }

    #[tokio::test]
    async fn review_inputs_refresh_mapping_values_without_waiting_for_cache_ttl() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let file = source.join("src/lib.rs");
        write_file(&file, "fn source() {}\n");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::privacy::save_privacy_policy(gcx.clone(), crate::privacy::PrivacyPolicy::default())
            .await
            .unwrap();
        let inputs = crate::privacy::records::prepare_review_privacy_inputs(gcx.clone())
            .await
            .unwrap();
        assert!(inputs.still_current(gcx.clone()).await);
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        write_file(&worktree.join("src/lib.rs"), "fn alias() {}\n");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);
        assert!(!inputs.still_current(gcx.clone()).await);
        let fresh = crate::privacy::records::prepare_review_privacy_inputs(gcx.clone())
            .await
            .unwrap();
        assert!(fresh.still_current(gcx).await);
    }

    #[tokio::test]
    async fn read_context_compiles_once_and_preserves_repeated_alias_checks() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn source() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree() {}\n");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);
        let mut mappings =
            crate::files_correction::registered_worktree_path_mappings(&gcx.cache_dir)
                .as_ref()
                .clone();
        for index in 0..16 {
            mappings.push(crate::files_correction::RegisteredWorktreePathMapping {
                root: temp.path().join(format!("alias-{index}")),
                source_root: normalized(&source),
            });
        }

        for (blocked, controlled, send, read) in [
            (Vec::new(), Vec::new(), Ok(()), Ok(())),
            (
                vec![worktree_file.to_string_lossy().to_string()],
                Vec::new(),
                Err("privacy level Blocked".to_string()),
                Err("privacy level Blocked".to_string()),
            ),
            (
                Vec::new(),
                vec![worktree_file.to_string_lossy().to_string()],
                Err("privacy level OnlySendToServersIControl".to_string()),
                Ok(()),
            ),
            (
                vec!["[".to_string()],
                Vec::new(),
                Err("privacy level Blocked".to_string()),
                Err("privacy level Blocked".to_string()),
            ),
        ] {
            let settings = Arc::new(PrivacySettings {
                privacy_rules: crate::privacy::FilePrivacySettings {
                    blocked,
                    only_send_to_servers_I_control: controlled,
                },
                loaded_ts: u64::MAX / 2,
            });
            *gcx.privacy_settings.write().unwrap() = settings.clone();
            let before = crate::privacy::LEGACY_PRIVACY_COMPILE_COUNT.with(|count| count.get());
            let mut context = prepare_file_read_context(gcx.clone()).await;
            context.worktree_mappings = Arc::new(mappings.clone());
            let aliases = registered_alias_paths(&source_file, &context.worktree_mappings);
            assert_eq!(aliases.len(), 18);
            for _ in 0..4 {
                for path in [&source_file, &worktree_file] {
                    assert_eq!(check_file_privacy_with_context(&context, path), send);
                    assert_eq!(
                        check_file_privacy_for_read_preparation_with_context(&context, path),
                        read
                    );
                }
            }
            assert_eq!(
                crate::privacy::LEGACY_PRIVACY_COMPILE_COUNT.with(|count| count.get()),
                before + 1
            );
            assert_eq!(
                aliases.iter().try_for_each(|alias| check_file_privacy(
                    settings.clone(),
                    alias,
                    &FilePrivacyLevel::AllowToSendAnywhere,
                )),
                send
            );
            assert_eq!(
                aliases.iter().try_for_each(|alias| {
                    crate::privacy::check_file_privacy_for_read_preparation(settings.clone(), alias)
                }),
                read
            );
        }
    }

    #[tokio::test]
    async fn read_context_new_request_observes_changed_policy() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("lib.rs");
        write_file(&file, "fn main() {}\n");
        allow_all_privacy(&gcx);
        let allowed = prepare_file_read_context(gcx.clone()).await;
        assert_eq!(check_file_privacy_with_context(&allowed, &file), Ok(()));

        set_blocked_privacy(&gcx, vec![file.to_string_lossy().to_string()]);
        let blocked = prepare_file_read_context(gcx.clone()).await;
        assert_eq!(
            check_file_privacy_with_context(&blocked, &file),
            Err("privacy level Blocked".to_string())
        );
        assert_eq!(check_file_privacy_with_context(&allowed, &file), Ok(()));

        allow_all_privacy(&gcx);
        let allowed_again = prepare_file_read_context(gcx).await;
        assert_eq!(
            check_file_privacy_with_context(&allowed_again, &file),
            Ok(())
        );
        assert_eq!(
            check_file_privacy_for_read_preparation_with_context(&blocked, &file),
            Err("privacy level Blocked".to_string())
        );
    }

    #[tokio::test]
    async fn read_context_new_request_reloads_changed_policy_file() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("lib.rs");
        write_file(&file, "fn main() {}\n");
        crate::privacy::save_privacy_policy(gcx.clone(), crate::privacy::PrivacyPolicy::default())
            .await
            .unwrap();
        let allowed = prepare_file_read_context(gcx.clone()).await;
        assert_eq!(check_file_privacy_with_context(&allowed, &file), Ok(()));
        write_file(
            &gcx.config_dir.join("privacy.yaml"),
            "privacy_rules:\n  blocked: ['*.rs']\n",
        );
        *gcx.privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                blocked: Vec::new(),
                only_send_to_servers_I_control: Vec::new(),
            },
            loaded_ts: 0,
        });
        let blocked = prepare_file_read_context(gcx).await;
        assert_eq!(
            check_file_privacy_with_context(&blocked, &file),
            Err("privacy level Blocked".to_string())
        );
        assert_eq!(check_file_privacy_with_context(&allowed, &file), Ok(()));
    }

    fn path_counts(paths: &[PathBuf]) -> HashMap<PathBuf, usize> {
        let mut counts = HashMap::new();
        for path in paths {
            *counts.entry(path.clone()).or_insert(0) += 1;
        }
        counts
    }

    #[test]
    fn ls_files_limited_stops_after_small_entry_budget_and_reports_truncation() {
        let temp = tempfile::tempdir().unwrap();
        for i in 0..50 {
            write_file(&temp.path().join(format!("file_{:03}.txt", i)), "x");
        }
        let indexing_everywhere = IndexingEverywhere::default();
        let result = ls_files_limited(
            &indexing_everywhere,
            &temp.path().to_path_buf(),
            true,
            5,
            None,
        )
        .unwrap();
        assert!(result.truncated, "expected truncated result");
        assert!(!result.aborted, "did not expect abort");
        assert!(
            result.files.len() <= 5,
            "collected {} files, expected <= budget",
            result.files.len()
        );
        assert!(!result.files.is_empty(), "expected some files collected");
    }

    #[test]
    fn ls_files_limited_observes_preset_abort_flag() {
        let temp = tempfile::tempdir().unwrap();
        for i in 0..10 {
            write_file(&temp.path().join(format!("file_{:03}.txt", i)), "x");
        }
        let indexing_everywhere = IndexingEverywhere::default();
        let abort = AtomicBool::new(true);
        let result = ls_files_limited(
            &indexing_everywhere,
            &temp.path().to_path_buf(),
            true,
            usize::MAX,
            Some(&abort),
        )
        .unwrap();
        assert!(result.aborted, "expected aborted result");
        assert!(
            result.files.is_empty(),
            "expected no files when aborted up-front"
        );
    }

    #[test]
    fn watch_plan_excludes_blocklisted_dirs_and_keeps_refact_visible() {
        let temp = tempfile::tempdir().unwrap();
        let root = normalized(temp.path());
        write_file(
            &root.join("src").join("a").join("b").join("lib.rs"),
            "pub fn ok() {}
",
        );
        write_file(
            &root.join("node_modules").join("pkg").join("index.js"),
            "console.log('x');
",
        );
        write_file(
            &root.join("target").join("debug").join("app"),
            "binary
",
        );
        write_file(
            &root.join(".git").join("HEAD"),
            "ref: refs/heads/main
",
        );
        write_file(
            &root.join(".venv").join("bin").join("python"),
            "#!/usr/bin/env python
",
        );
        write_file(
            &root.join("dist").join("bundle.js"),
            "console.log('dist');
",
        );
        write_file(
            &root
                .join(".refact")
                .join("skills")
                .join("example")
                .join("SKILL.md"),
            "# skill
",
        );

        let indexing_everywhere = IndexingEverywhere::default();
        let plan = build_watch_plan(&[root.clone()], &indexing_everywhere);
        let entries = registration_modes(&plan, &root);

        assert!(entries.contains(&(PathBuf::from(""), WatchRegistrationMode::NonRecursive)));
        assert!(entries.contains(&(PathBuf::from("src"), WatchRegistrationMode::Recursive)));
        assert!(entries.contains(&(PathBuf::from(".refact"), WatchRegistrationMode::Recursive)));
        assert!(!entries
            .iter()
            .any(|(path, _)| path == Path::new("node_modules")));
        assert!(!entries.iter().any(|(path, _)| path == Path::new("target")));
        assert!(!entries.iter().any(|(path, _)| path == Path::new(".git")));
        assert!(!entries.iter().any(|(path, _)| path == Path::new(".venv")));
        assert!(!entries.iter().any(|(path, _)| path == Path::new("dist")));
    }

    #[test]
    fn watch_plan_excludes_only_trajectory_dirs_under_tasks() {
        let temp = tempfile::tempdir().unwrap();
        let root = normalized(temp.path());
        let refact = root.join(".refact");
        let task = refact.join("tasks").join("task-1");
        write_file(
            &refact.join("trajectories").join("chat-1").join("chat.json"),
            "{}\n",
        );
        write_file(
            &task.join("trajectories").join("planner").join("chat.json"),
            "{}\n",
        );
        write_file(
            &task
                .join("trajectories")
                .join("agents")
                .join("agent-1")
                .join("chat.json"),
            "{}\n",
        );
        write_file(&task.join("documents").join("master-plan.md"), "# plan\n");
        write_file(&task.join("memories").join("note.md"), "# note\n");
        write_file(&task.join("meta.yaml"), "id: task-1\n");
        write_file(&task.join("board.yaml"), "cards: []\n");

        let indexing_everywhere = IndexingEverywhere::default();
        let plan = build_watch_plan(&[root.clone()], &indexing_everywhere);
        let entries = registration_modes(&plan, &root);

        for owned_root in [refact.join("trajectories"), task.join("trajectories")] {
            assert!(
                !plan.iter().any(|reg| reg.path.starts_with(&owned_root)),
                "{} is owned by the trajectory watcher and must not be double-watched, plan was {:?}",
                owned_root.display(),
                entries
            );
        }

        for owned_file in [
            refact.join("trajectories").join("chat-1").join("chat.json"),
            task.join("trajectories").join("planner").join("chat.json"),
            task.join("trajectories")
                .join("agents")
                .join("agent-1")
                .join("chat.json"),
        ] {
            assert!(
                !plan_covers(&plan, &owned_file),
                "{} is owned by the trajectory watcher and must not be double-watched, plan was {:?}",
                owned_file.display(),
                entries
            );
        }

        for watched in [
            task.join("documents"),
            task.join("documents").join("master-plan.md"),
            task.join("memories").join("note.md"),
            task.join("meta.yaml"),
            task.join("board.yaml"),
        ] {
            assert!(
                plan_covers(&plan, &watched),
                "{} must stay watched by the workspace watcher, plan was {:?}",
                watched.display(),
                entries
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runtime_created_directory_under_non_recursive_root_is_watched_end_to_end() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let root = normalized(temp.path());
        write_file(
            &root.join("src").join("lib.rs"),
            "pub fn before() {}
",
        );
        write_file(
            &root.join("node_modules").join("pkg").join("index.js"),
            "console.log('x');
",
        );
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() = vec![root.clone()];
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        *gcx.codegraph.lock().await = Some(service.clone());

        let indexing_everywhere = IndexingEverywhere::default();
        assert_eq!(
            classify_watch_subtree(&root, &root, &indexing_everywhere),
            WatchSubtreeClass::Mixed,
            "root must be non-recursive so that only the dynamic hook can cover new dirs"
        );

        watcher_init(gcx.clone()).await;

        let created_dir = root.join("runtime");
        std::fs::create_dir_all(&created_dir).unwrap();
        let runtime_file = created_dir.join("new.rs");
        let expected = normalized(&runtime_file).to_string_lossy().to_string();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = false;
        while !seen && Instant::now() < deadline {
            write_file(
                &runtime_file,
                "pub fn runtime() {}
",
            );
            tokio::time::sleep(FILE_EVENT_DEBOUNCE_WINDOW + Duration::from_millis(80)).await;
            seen = service.drain_batch(10).contains(&expected);
        }
        assert!(
            seen,
            "file inside a runtime-created directory never produced an event: the dynamic \
             watch registration did not happen, or the notify thread deadlocked"
        );

        gcx.shutdown_flag.store(true, Ordering::Relaxed);
        gcx.documents_state.file_event_debounce_notify.notify_one();
    }

    #[test]
    fn refact_runtime_state_denies_engine_written_top_level_entries_only() {
        let refact = Path::new("/repo/.refact");
        for denied in [
            "stats/00000268.jsonl",
            "shell_audit.jsonl",
            "shell_audit.jsonl.tmp",
            "scheduled_tasks.json",
            "background_agents/records.json",
            "background_agents/results/agent.json",
            "vecdb/data.lance",
            "review_scratch/abc/src/lib.rs",
            "visual_baselines/home.png",
            "trajectories/index.json",
            "tasks/t1/trajectories/chat.json",
        ] {
            assert!(
                path_is_refact_runtime_state(&refact.join(denied)),
                "{denied} must be treated as runtime state"
            );
        }
        for kept in [
            "knowledge/stats/note.md",
            "skills/stats/SKILL.md",
            "modes/agent.yaml",
            "integrations.d/github.yaml",
            "project_information.yaml",
            "commands/ship.md",
            "statistics/report.md",
        ] {
            assert!(
                !path_is_refact_runtime_state(&refact.join(kept)),
                "{kept} must stay visible"
            );
        }
        assert!(!path_is_refact_runtime_state(Path::new(
            "/repo/src/stats/mod.rs"
        )));
    }

    #[tokio::test]
    async fn workspace_scan_excludes_engine_runtime_logs_under_refact() {
        let temp = tempfile::tempdir().unwrap();
        let refact = temp.path().join(".refact");
        let stats = refact.join("stats").join("00000001.jsonl");
        let audit = refact.join("shell_audit.jsonl");
        let records = refact.join("background_agents").join("records.json");
        let knowledge = refact.join("knowledge").join("stats").join("note.md");
        write_file(&stats, "{\"tokens\":1}\n");
        write_file(&audit, "{\"cmd\":\"ls\"}\n");
        write_file(&records, "{\"records\":[]}\n");
        write_file(&knowledge, "# stats note\n");

        let files = scan_workspace(temp.path()).await;

        assert!(!files.contains(&normalized(&stats)));
        assert!(!files.contains(&normalized(&audit)));
        assert!(!files.contains(&normalized(&records)));
        assert!(files.contains(&normalized(&knowledge)));
    }

    #[tokio::test]
    async fn workspace_scan_excludes_configured_global_config_index_json() {
        let temp = tempfile::tempdir().unwrap();
        let config_root = temp.path().join("global_config").join("refact");
        let generated = config_root.join("trajectories").join("index.json");
        let regular = config_root.join("knowledge").join("index.json");
        write_file(&generated, "{\"schema_version\":1}\n");
        write_file(&regular, "{\"user\":true}\n");

        let files = scan_workspace_with_global_config_root(&config_root, &config_root).await;

        assert!(!files.contains(&normalized(&generated)));
        assert!(files.contains(&normalized(&regular)));
    }

    #[tokio::test]
    async fn workspace_scan_excludes_refact_codegraph_db_files() {
        let temp = tempfile::tempdir().unwrap();
        let regular = temp.path().join("src").join("lib.rs");
        let db_wal = temp
            .path()
            .join(".refact")
            .join("codegraph")
            .join("codegraph.sqlite-wal");
        write_file(&regular, "pub fn ok() {}\n");
        write_file(&db_wal, "sqlite wal content\n");

        let files = scan_workspace(temp.path()).await;

        assert!(files.contains(&normalized(&regular)));
        assert!(!files.contains(&normalized(&db_wal)));
        assert!(is_valid_file(&normalized(&db_wal), false, false).is_err());
        assert!(!path_triggers_registry_reload(&db_wal));
    }

    #[tokio::test]
    async fn open_worktree_buffer_is_read_through_mapped_source_path() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("source");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn disk() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree_disk() {}\n");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);

        let unsaved_text = "fn unsaved_from_ide() {}\n".to_string();
        on_did_open(
            gcx.clone(),
            &worktree_file,
            &unsaved_text,
            &"rust".to_string(),
        )
        .await;

        let read_text = get_file_text_from_memory_or_disk(gcx.clone(), &source_file)
            .await
            .unwrap();
        assert_eq!(read_text, unsaved_text);

        let memory_doc_map = gcx.documents_state.memory_document_map.lock().await;
        assert!(memory_doc_map.contains_key(&normalized(&source_file)));
        assert!(!memory_doc_map.contains_key(&normalized(&worktree_file)));
    }

    #[tokio::test]
    async fn worktree_read_rejects_blocked_mapped_source_alias() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("blocked_source");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn source() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree() {}\n");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);
        set_blocked_privacy(&gcx, vec!["*/blocked_source/*".to_string()]);

        let error = get_file_text_from_memory_or_disk(gcx, &worktree_file)
            .await
            .unwrap_err();

        assert!(error.contains("privacy"), "unexpected error: {error}");
    }

    #[test]
    fn read_context_allows_controlled_path_but_rejects_blocked_alias() {
        let temp = tempfile::tempdir().unwrap();
        let source = normalized(&temp.path().join("source"));
        let worktree = normalized(&temp.path().join("worktree"));
        let source_file = source.join("src").join("lib.rs");
        let worktree_file = worktree.join("src").join("lib.rs");
        let mappings = Arc::new(vec![
            crate::files_correction::RegisteredWorktreePathMapping {
                root: worktree,
                source_root: source,
            },
        ]);
        let controlled_settings = Arc::new(PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                only_send_to_servers_I_control: vec![source_file.to_string_lossy().to_string()],
                blocked: Vec::new(),
            },
            loaded_ts: 0,
        });
        let controlled_context = FileReadContext {
            privacy_matcher: LegacyPrivacyMatcher::new(&controlled_settings),
            privacy_settings: controlled_settings,
            worktree_mappings: mappings.clone(),
        };

        assert!(check_file_privacy_for_read_preparation_with_context(
            &controlled_context,
            &source_file
        )
        .is_ok());
        assert!(check_file_privacy_with_context(&controlled_context, &source_file).is_err());

        let blocked_alias_context = FileReadContext {
            privacy_settings: Arc::new(PrivacySettings {
                privacy_rules: crate::privacy::FilePrivacySettings {
                    only_send_to_servers_I_control: vec![source_file.to_string_lossy().to_string()],
                    blocked: vec![worktree_file.to_string_lossy().to_string()],
                },
                loaded_ts: 0,
            }),
            privacy_matcher: LegacyPrivacyMatcher::new(&PrivacySettings {
                privacy_rules: crate::privacy::FilePrivacySettings {
                    only_send_to_servers_I_control: vec![source_file.to_string_lossy().to_string()],
                    blocked: vec![worktree_file.to_string_lossy().to_string()],
                },
                loaded_ts: 0,
            }),
            worktree_mappings: mappings,
        };
        assert!(check_file_privacy_for_read_preparation_with_context(
            &blocked_alias_context,
            &source_file,
        )
        .is_err());
    }

    #[test]
    fn strictest_zone_uses_registered_worktree_alias() {
        let temp = tempfile::tempdir().unwrap();
        let source = normalized(&temp.path().join("source"));
        let worktree = normalized(&temp.path().join("worktree"));
        let source_file = source.join("src").join("lib.rs");
        let worktree_file = worktree.join("src").join("lib.rs");
        let policy = refact_privacy::PrivacyPolicy {
            blocked: Vec::new(),
            zones: vec![
                refact_privacy::Zone {
                    name: "secrets".to_string(),
                    patterns: vec![worktree_file.to_string_lossy().to_string()],
                    send_to: Vec::new(),
                    on_shell_read: refact_privacy::ShellBehavior::Deny,
                },
                refact_privacy::Zone {
                    name: "normal".to_string(),
                    patterns: vec!["*".to_string()],
                    send_to: vec!["*".to_string()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                },
            ],
            subagents: refact_privacy::SubagentPolicy::default(),
            ..Default::default()
        }
        .compile()
        .unwrap();
        let mappings = vec![crate::files_correction::RegisteredWorktreePathMapping {
            root: worktree,
            source_root: source.clone(),
        }];

        assert_eq!(policy.zone_for_path(&source_file).name, "normal");
        assert_eq!(policy.zone_for_path(&worktree_file).name, "secrets");
        assert_eq!(
            strictest_zone_for_path(&policy, &source_file, &[source], &mappings).name,
            "secrets"
        );
    }

    #[tokio::test]
    async fn source_read_rejects_blocked_worktree_alias_buffer() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("source");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn source() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree() {}\n");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);
        allow_all_privacy(&gcx);
        on_did_open(
            gcx.clone(),
            &worktree_file,
            &"fn unsaved() {}\n".to_string(),
            &"rust".to_string(),
        )
        .await;
        set_blocked_privacy(&gcx, vec!["*/worktrees/*/wt/*".to_string()]);

        let error = get_file_text_from_memory_or_disk(gcx, &source_file)
            .await
            .unwrap_err();

        assert!(error.contains("privacy"), "unexpected error: {error}");
    }

    #[tokio::test]
    async fn bounded_disk_read_accepts_limit_and_rejects_limit_plus_one() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let temp = tempfile::tempdir().unwrap();
        let exact = temp.path().join("exact.txt");
        let oversized = temp.path().join("oversized.txt");
        std::fs::write(&exact, b"1234").unwrap();
        std::fs::write(&oversized, b"12345").unwrap();
        let read_context = prepare_file_read_context(gcx.clone()).await;

        let exact_text = get_file_text_from_memory_or_disk_with_context(
            gcx.clone(),
            &exact,
            &read_context,
            Some(4),
        )
        .await
        .unwrap();
        let oversized_error =
            get_file_text_from_memory_or_disk_with_context(gcx, &oversized, &read_context, Some(4))
                .await
                .unwrap_err();

        assert_eq!(exact_text, "1234");
        assert!(oversized_error.contains("exceeds 4 byte"));
    }

    #[tokio::test]
    async fn raw_worktree_invalidation_removes_mapped_source_buffer() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("source");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn disk() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree_disk() {}\n");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);

        let unsaved_text = "fn unsaved_from_ide() {}\n".to_string();
        on_did_open(
            gcx.clone(),
            &worktree_file,
            &unsaved_text,
            &"rust".to_string(),
        )
        .await;
        assert_eq!(
            get_file_text_from_memory_or_disk(gcx.clone(), &source_file)
                .await
                .unwrap(),
            unsaved_text
        );

        assert!(remove_memory_document_for_path(gcx.clone(), &worktree_file).await);
        assert_eq!(
            get_file_text_from_memory_or_disk(gcx.clone(), &source_file)
                .await
                .unwrap(),
            "fn disk() {}\n"
        );
    }

    #[tokio::test]
    async fn on_did_delete_registered_worktree_file_removes_source_keyed_codegraph_row() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("source");
        std::fs::create_dir_all(source.join("src")).unwrap();
        // Canonicalize while the dir exists so raw fixture paths match canonical
        // index keys on Windows runners with an 8.3 short TMP (RUNNER~1).
        let source = normalized(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn source() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree() {}\n");
        let worktree = normalized(&worktree);
        let worktree_file = worktree.join("src").join("lib.rs");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        let source_key = normalized(&source_file).to_string_lossy().to_string();
        service
            .index_file(&source_key, "fn source() {}\n", "rust")
            .await
            .unwrap();
        *gcx.codegraph.lock().await = Some(service.clone());

        std::fs::remove_file(&worktree_file).unwrap();
        on_did_delete(gcx.clone(), &worktree_file).await;

        assert_eq!(service.counts().await.unwrap().files, 0);
        assert!(source_file.exists());
    }

    #[tokio::test]
    async fn watcher_bumps_knowledge_generation_only_for_knowledge_paths() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("src").join("lib.rs");
        write_file(&source, "fn main() {}\n");
        let knowledge = temp
            .path()
            .join(".refact")
            .join("knowledge")
            .join("note.md");
        write_file(&knowledge, "---\nid: \"note\"\n---\n\nBody\n");

        let source_event =
            notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
                .add_path(source);
        file_watcher_event(source_event, Arc::downgrade(&gcx)).await;
        assert_eq!(gcx.knowledge_generation.load(Ordering::SeqCst), 0);
        assert!(gcx.enrichment_generation.load(Ordering::SeqCst) > 0);

        let knowledge_event =
            notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
                .add_path(knowledge);
        file_watcher_event(knowledge_event, Arc::downgrade(&gcx)).await;
        assert_eq!(gcx.knowledge_generation.load(Ordering::SeqCst), 1);
    }

    fn write_head(repo_path: &Path, content: &str) {
        let git_dir = repo_path.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), content).unwrap();
    }

    fn git_signature() -> git2::Signature<'static> {
        git2::Signature::now("test", "test@test.com").unwrap()
    }

    fn git_commit_all(repo: &git2::Repository, message: &str) -> git2::Oid {
        let sig = git_signature();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.update_all(["*"].iter(), None).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|oid| repo.find_commit(oid).ok());
        match parent {
            Some(parent) => repo
                .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
                .unwrap(),
            None => repo
                .commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
                .unwrap(),
        }
    }

    fn git_checkout_branch(repo: &git2::Repository, branch: &str) {
        repo.set_head(&format!("refs/heads/{branch}")).unwrap();
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force();
        repo.checkout_head(Some(&mut checkout)).unwrap();
    }

    fn modify_event(path: &Path) -> notify::Event {
        notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(path.to_path_buf())
    }

    #[test]
    fn target_e2e_is_blocklisted() {
        let root = normalized(Path::new("/repo"));
        let indexing_everywhere = IndexingEverywhere::default();

        for blocked in [
            Path::new("refact-agent").join("engine").join("target"),
            Path::new("refact-agent").join("engine").join("target-e2e"),
            Path::new("refact-agent")
                .join("engine")
                .join("target-e2e")
                .join("debug"),
        ] {
            assert!(
                path_is_excluded_from_watch(&root, &root.join(&blocked), &indexing_everywhere),
                "{} must be excluded from watching",
                blocked.display()
            );
        }

        for kept in [
            PathBuf::from("targeting"),
            Path::new("src").join("targeting").join("rules"),
            PathBuf::from("targets"),
            Path::new("refact-agent").join("engine").join("src"),
        ] {
            assert!(
                !path_is_excluded_from_watch(&root, &root.join(&kept), &indexing_everywhere),
                "{} must stay watched",
                kept.display()
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn moved_in_directory_hardlink_is_inventoried_without_full_walk() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = normalized(workspace.path());
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let secret = normalized(outside.path()).join(".env");
        std::fs::write(&secret, "private").unwrap();
        let staging = normalized(outside.path()).join("incoming");
        std::fs::create_dir_all(staging.join("nested")).unwrap();
        std::fs::hard_link(&secret, staging.join("nested/plain.txt")).unwrap();
        let roots = vec![root.clone()];
        let index = gcx.file_index.clone();
        let started = crate::file_index::tests_now_ms();
        assert!(index.request_walk_at(roots.clone(), started).is_some());
        crate::file_index::build_blocking(&index, &roots, &AtomicBool::new(false));
        assert!(index
            .complete_walk_at(
                roots.clone(),
                crate::file_index::tests_now_ms(),
                60 * 60_000,
                crate::file_index::FileIndexWalkOutcome::Complete,
                false,
            )
            .is_none());
        let facts = index.lookup_or_stat(&secret).unwrap();
        assert!(index.aliases_of(&facts, &secret).is_empty());

        let moved = root.join("incoming");
        std::fs::rename(&staging, &moved).unwrap();
        let event = notify::Event::new(notify::EventKind::Modify(ModifyKind::Name(
            notify::event::RenameMode::Both,
        )))
        .add_path(staging.clone())
        .add_path(moved.clone());
        file_watcher_event(event, Arc::downgrade(&gcx)).await;
        let scans: Vec<_> = gcx
            .documents_state
            .watch_registration_tasks
            .lock()
            .unwrap()
            .drain(..)
            .collect();
        for scan in scans {
            scan.await.unwrap();
        }

        assert!(!index.status().walk_running, "no full walk may run here");
        assert_eq!(
            index.aliases_of(&facts, &secret),
            vec![moved.join("nested/plain.txt")]
        );
        assert!(index.get(&moved.join("nested")).is_none());
        assert!(index
            .snapshot()
            .iter()
            .any(|(path, facts)| path == &moved.join("nested") && facts.is_none()));
    }

    #[tokio::test(start_paused = true)]
    async fn deferred_walk_is_served_after_the_watcher_channel_closes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace = tempfile::tempdir().unwrap();
        let root = normalized(workspace.path());
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let roots = vec![root.clone()];
        let index = gcx.file_index.clone();
        let long_ago = crate::file_index::tests_now_ms() - 60_000;
        assert!(index.request_walk_at(roots.clone(), long_ago).is_some());
        assert!(index.request_walk_at(roots.clone(), long_ago).is_none());
        assert!(index
            .complete_walk_at(
                roots.clone(),
                long_ago,
                0,
                crate::file_index::FileIndexWalkOutcome::Complete,
                true,
            )
            .is_none());
        assert!(
            index.status().rebuild_pending,
            "wanted, due, and not started"
        );

        let (tx, rx) = mpsc::channel::<Event>(1);
        drop(tx);
        start_file_watcher_event_queue(&gcx, Arc::downgrade(&gcx), rx);
        let served = tokio::time::timeout(Duration::from_secs(20), async {
            while index.status().rebuild_pending || index.status().walk_running {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(served.is_ok(), "the tick must serve the deferred walk");
        assert!(index.covers_roots(&roots));
        let task = gcx
            .documents_state
            .file_watcher_event_task
            .lock()
            .unwrap()
            .take()
            .unwrap();
        assert!(!task.is_finished(), "the tick outlives the watcher channel");
        gcx.shutdown_flag.store(true, Ordering::Relaxed);
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("the tick must stop on shutdown")
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn event_handling_continues_while_a_subtree_scan_is_blocked() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace = tempfile::tempdir().unwrap();
        let root = normalized(workspace.path());
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let created = root.join("created");
        std::fs::create_dir_all(created.join("inner")).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let release_rx = Arc::new(StdMutex::new(release_rx));
        let blocked_until_release = Arc::new(AtomicBool::new(false));
        let hook_released = blocked_until_release.clone();
        gcx.file_index
            .set_subtree_scan_hook(Some(Arc::new(move |_dir: &Path| {
                let _ = started_tx.send(());
                let released = release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .is_ok();
                hook_released.store(released, Ordering::SeqCst);
            })));
        let create = notify::Event::new(notify::EventKind::Create(CreateKind::Folder))
            .add_path(created.clone());
        file_watcher_event(create, Arc::downgrade(&gcx)).await;
        tokio::task::spawn_blocking(move || started_rx.recv_timeout(Duration::from_secs(10)))
            .await
            .unwrap()
            .expect("the scan must have started");

        let later = root.join("later.txt");
        std::fs::write(&later, "x").unwrap();
        file_watcher_event(modify_event(&later), Arc::downgrade(&gcx)).await;
        assert!(gcx.file_index.get(&later).is_some());
        recover_file_index_watcher_drops(gcx.clone()).await;

        release_tx.send(()).unwrap();
        let task = gcx
            .documents_state
            .watch_registration_tasks
            .lock()
            .unwrap()
            .drain(..)
            .collect::<Vec<_>>();
        tokio::time::timeout(Duration::from_secs(30), async {
            for task in task {
                let _ = task.await;
            }
        })
        .await
        .expect("the subtree scan must finish before shutdown");
        gcx.shutdown_flag.store(true, Ordering::Relaxed);
        assert!(
            blocked_until_release.load(Ordering::SeqCst),
            "event handling must not wait for the subtree scan"
        );
        assert!(gcx.file_index.get(&created.join("inner")).is_none());
        assert!(gcx
            .file_index
            .snapshot()
            .iter()
            .any(|(path, facts)| path == &created.join("inner") && facts.is_none()));
    }

    #[test]
    fn subtree_scan_targets_rename_destination_and_real_directories_only() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(&new, "file").unwrap();
        let both = notify::Event::new(notify::EventKind::Modify(ModifyKind::Name(
            RenameMode::Both,
        )))
        .add_path(old.clone())
        .add_path(new.clone());
        assert_eq!(
            subtree_scan_target(&both),
            None,
            "only the destination counts"
        );
        let from = notify::Event::new(notify::EventKind::Modify(ModifyKind::Name(
            RenameMode::From,
        )))
        .add_path(old.clone());
        assert_eq!(subtree_scan_target(&from), None);
        let created =
            notify::Event::new(notify::EventKind::Create(CreateKind::Folder)).add_path(old.clone());
        assert_eq!(subtree_scan_target(&created), Some(old.clone()));
        #[cfg(unix)]
        {
            let external = tempfile::tempdir().unwrap();
            let link = temp.path().join("link");
            std::os::unix::fs::symlink(external.path(), &link).unwrap();
            let linked =
                notify::Event::new(notify::EventKind::Create(CreateKind::Any)).add_path(link);
            assert_eq!(
                subtree_scan_target(&linked),
                None,
                "a link is never scanned"
            );
        }
    }

    #[tokio::test]
    async fn watcher_callback_does_not_block_on_runtime() {
        let (tx, mut rx) = mpsc::channel::<Event>(8);
        let dropped = Arc::new(AtomicU64::new(0));
        let callback = make_watcher_event_callback(tx, dropped.clone());
        let path = PathBuf::from("/tmp/refact-watcher-callback/lib.rs");
        let event = modify_event(&path);

        // notify delivers events on its own thread, which has no tokio runtime:
        // any block_on/Handle::current() in the callback would panic here.
        std::thread::spawn(move || callback(Ok(event)))
            .join()
            .unwrap();

        let received = rx.try_recv().expect("callback must enqueue the event");
        assert_eq!(received.paths, vec![path]);
        assert!(rx.try_recv().is_err(), "callback must enqueue exactly once");
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn dropped_events_are_counted_under_backpressure() {
        let (tx, _rx) = mpsc::channel::<Event>(1);
        let dropped = AtomicU64::new(0);

        enqueue_watcher_event(&tx, &dropped, modify_event(Path::new("/tmp/a.rs")));
        assert_eq!(dropped.load(Ordering::Relaxed), 0);

        enqueue_watcher_event(&tx, &dropped, modify_event(Path::new("/tmp/b.rs")));
        enqueue_watcher_event(&tx, &dropped, modify_event(Path::new("/tmp/c.rs")));
        assert_eq!(
            dropped.load(Ordering::Relaxed),
            2,
            "events refused by a full queue must be counted"
        );

        let ignored = notify::Event::new(notify::EventKind::Access(AccessKind::Open(
            AccessMode::Read,
        )))
        .add_path(PathBuf::from("/tmp/d.rs"));
        enqueue_watcher_event(&tx, &dropped, ignored);
        assert_eq!(
            dropped.load(Ordering::Relaxed),
            2,
            "events the watcher never acts on are filtered, not dropped"
        );

        assert!(watcher_event_is_actionable(&EventKind::Access(
            AccessKind::Close(AccessMode::Write)
        )));
        assert!(!watcher_event_is_actionable(&EventKind::Any));
        assert!(!watcher_event_is_actionable(&EventKind::Other));
    }

    #[tokio::test]
    async fn expensive_path_resolution_happens_after_debounce() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src").join("hot.rs");
        write_file(&file, "pub fn hot() {}\n");
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![normalized(temp.path())];
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        *gcx.codegraph.lock().await = Some(service.clone());

        for _ in 0..10 {
            file_watcher_event(modify_event(&file), Arc::downgrade(&gcx)).await;
        }

        assert_eq!(
            gcx.documents_state
                .file_event_debounce
                .lock()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            gcx.documents_state
                .file_event_resolutions
                .load(Ordering::Relaxed),
            0,
            "no path resolution may happen before the debounce window elapses"
        );

        tokio::time::sleep(FILE_EVENT_DEBOUNCE_WINDOW + Duration::from_millis(80)).await;

        assert_eq!(
            gcx.documents_state
                .file_event_resolutions
                .load(Ordering::Relaxed),
            1,
            "10 rapid events on one path must resolve the path exactly once"
        );
        assert_eq!(
            service.drain_batch(10),
            vec![normalized(&file).to_string_lossy().to_string()]
        );

        gcx.shutdown_flag.store(true, Ordering::Relaxed);
        gcx.documents_state.file_event_debounce_notify.notify_one();
    }

    #[tokio::test]
    async fn debounced_worktree_edit_flushes_source_store_with_worktree_read_path() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("source");
        std::fs::create_dir_all(source.join("src")).unwrap();
        // Canonicalize while the dir exists so raw fixture paths match canonical
        // index keys on Windows runners with an 8.3 short TMP (RUNNER~1).
        let source = normalized(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn source() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree() {}\n");
        let worktree = normalized(&worktree);
        let worktree_file = worktree.join("src").join("lib.rs");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![worktree.clone()];
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        *gcx.codegraph.lock().await = Some(service.clone());

        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(worktree_file.clone());
        file_watcher_event(event, Arc::downgrade(&gcx)).await;
        tokio::time::sleep(FILE_EVENT_DEBOUNCE_WINDOW + Duration::from_millis(80)).await;

        let queued = service.drain_batch_entries(10);
        assert_eq!(
            queued,
            vec![QueuedPath::new(
                normalized(&source_file).to_string_lossy().to_string(),
                normalized(&worktree_file).to_string_lossy().to_string(),
            )]
        );
        crate::codegraph::cg_highlev::process_index_batch(gcx.clone(), service.clone(), queued)
            .await;
        assert_eq!(
            service.all_files_with_text().await.unwrap(),
            vec![(
                normalized(&source_file).to_string_lossy().to_string(),
                "fn worktree() {}\n".to_string(),
            )]
        );
    }

    #[tokio::test]
    async fn debounced_worktree_remove_flushes_source_store_remove() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("source");
        std::fs::create_dir_all(source.join("src")).unwrap();
        // Canonicalize while the dir exists so raw fixture paths match canonical
        // index keys on Windows runners with an 8.3 short TMP (RUNNER~1).
        let source = normalized(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn source() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree() {}\n");
        let worktree = normalized(&worktree);
        let worktree_file = worktree.join("src").join("lib.rs");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![worktree.clone()];
        *gcx.documents_state.workspace_files.lock().unwrap() = vec![normalized(&source_file)];
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        let source_key = normalized(&source_file).to_string_lossy().to_string();
        service
            .index_file(&source_key, "fn indexed() {}\n", "rust")
            .await
            .unwrap();
        *gcx.codegraph.lock().await = Some(service.clone());

        std::fs::remove_file(&worktree_file).unwrap();
        let event = notify::Event::new(notify::EventKind::Remove(notify::event::RemoveKind::File))
            .add_path(worktree_file.clone());
        file_watcher_event(event, Arc::downgrade(&gcx)).await;
        tokio::time::sleep(FILE_EVENT_DEBOUNCE_WINDOW + Duration::from_millis(80)).await;

        assert_eq!(service.counts().await.unwrap().files, 0);
        assert_eq!(
            service.drain_batch_entries(10),
            vec![QueuedPath::new(
                source_key,
                normalized(&worktree_file).to_string_lossy().to_string(),
            )]
        );
        let workspace_files = gcx.documents_state.workspace_files.lock().unwrap().clone();
        assert!(!workspace_files.contains(&normalized(&source_file)));
        assert!(source_file.exists());
    }

    #[tokio::test]
    async fn worktree_remove_event_preserves_remove_kind_when_source_exists() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("source");
        std::fs::create_dir_all(source.join("src")).unwrap();
        // Canonicalize while the dir exists so raw fixture paths match canonical
        // index keys on Windows runners with an 8.3 short TMP (RUNNER~1).
        let source = normalized(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn source() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree() {}\n");
        let worktree = normalized(&worktree);
        let worktree_file = worktree.join("src").join("lib.rs");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![worktree.clone()];

        let event = notify::Event::new(notify::EventKind::Remove(notify::event::RemoveKind::File))
            .add_path(worktree_file.clone());
        file_watcher_event(event, Arc::downgrade(&gcx)).await;

        let raw_events = take_all_debounced_file_events(&gcx);
        assert_eq!(
            raw_events,
            vec![(worktree_file.clone(), true)],
            "the debounce key must stay the raw event path, no canonicalization yet"
        );

        let resolved = resolve_debounced_file_events(gcx.clone(), raw_events).await;
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].kind, DebouncedFileEventKind::Remove);
        assert_eq!(resolved[0].store_path, normalized(&source_file));
        assert_eq!(resolved[0].read_path, normalized(&worktree_file));
        gcx.shutdown_flag.store(true, Ordering::Relaxed);
        gcx.documents_state.file_event_debounce_notify.notify_one();
    }
    #[tokio::test]
    async fn debounced_file_event_worker_is_bounded_and_flushes_pending_on_shutdown() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("src").join("first.rs");
        let second = temp.path().join("src").join("second.rs");
        write_file(&first, "pub fn first() {}\n");
        write_file(&second, "pub fn second() {}\n");
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![normalized(temp.path())];
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        *gcx.codegraph.lock().await = Some(service.clone());

        let now = Instant::now();
        schedule_debounced_file_event(gcx.clone(), normalized(&first), false, now);
        schedule_debounced_file_event(gcx.clone(), normalized(&second), false, now);
        {
            let task = gcx.documents_state.file_event_debounce_task.lock().unwrap();
            assert!(task.is_some());
        }
        assert_eq!(service.queue_len(), 0);

        gcx.shutdown_flag.store(true, Ordering::Relaxed);
        gcx.documents_state.file_event_debounce_notify.notify_one();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if service.queue_len() == 2
                    && gcx
                        .documents_state
                        .file_event_debounce
                        .lock()
                        .unwrap()
                        .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        let mut queued = service.drain_batch(10);
        queued.sort();
        assert_eq!(
            queued,
            vec![
                normalized(&first).to_string_lossy().to_string(),
                normalized(&second).to_string_lossy().to_string(),
            ]
        );
        assert!(gcx
            .documents_state
            .file_event_debounce_task
            .lock()
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn branch_switch_enqueues_only_changed_files() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let changed = temp.path().join("src").join("changed.rs");
        let unchanged = temp.path().join("src").join("unchanged.rs");
        write_file(&changed, "pub fn changed() -> &'static str { \"main\" }\n");
        write_file(&unchanged, "pub fn unchanged() {}\n");
        let main_oid = git_commit_all(&repo, "main");
        {
            let main_commit = repo.find_commit(main_oid).unwrap();
            repo.branch("dev", &main_commit, false).unwrap();
        }
        git_checkout_branch(&repo, "dev");
        write_file(&changed, "pub fn changed() -> &'static str { \"dev\" }\n");
        git_commit_all(&repo, "dev");
        git_checkout_branch(&repo, "main");

        let canonical_repo = normalized(temp.path());
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![canonical_repo.clone()];
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() = vec![canonical_repo.clone()];
        *gcx.documents_state.workspace_files.lock().unwrap() =
            vec![normalized(&changed), normalized(&unchanged)];
        {
            let mut heads = gcx.documents_state.git_branch_heads.lock().unwrap();
            heads.insert(canonical_repo.clone(), "ref: refs/heads/main".to_string());
        }
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        *gcx.codegraph.lock().await = Some(service.clone());

        git_checkout_branch(&repo, "dev");
        let head_path = temp.path().join(".git").join("HEAD");
        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(head_path);
        on_git_head_change(Arc::downgrade(&gcx), event).await;
        tokio::time::sleep(BRANCH_HEAD_DEBOUNCE_WINDOW + Duration::from_millis(80)).await;

        assert_eq!(
            service.drain_batch(10),
            vec![normalized(&changed).to_string_lossy().to_string()]
        );
    }

    #[tokio::test]
    async fn branch_switch_enqueues_deleted_files_and_removes_old_index() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let deleted = temp.path().join("src").join("deleted.rs");
        let kept = temp.path().join("src").join("kept.rs");
        write_file(&deleted, "pub fn deleted() {}\n");
        write_file(&kept, "pub fn kept() {}\n");
        let main_oid = git_commit_all(&repo, "main");
        {
            let main_commit = repo.find_commit(main_oid).unwrap();
            repo.branch("dev", &main_commit, false).unwrap();
        }
        git_checkout_branch(&repo, "dev");
        std::fs::remove_file(&deleted).unwrap();
        git_commit_all(&repo, "dev deletes file");
        git_checkout_branch(&repo, "main");

        let canonical_repo = normalized(temp.path());
        let deleted_path = normalized(&deleted).to_string_lossy().to_string();
        let kept_path = normalized(&kept);
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![canonical_repo.clone()];
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() = vec![canonical_repo.clone()];
        *gcx.documents_state.workspace_files.lock().unwrap() =
            vec![PathBuf::from(&deleted_path), kept_path.clone()];
        {
            let mut heads = gcx.documents_state.git_branch_heads.lock().unwrap();
            heads.insert(canonical_repo, "ref: refs/heads/main".to_string());
        }
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        service
            .index_file(&deleted_path, "pub fn deleted() {}\n", "rust")
            .await
            .unwrap();
        *gcx.codegraph.lock().await = Some(service.clone());

        git_checkout_branch(&repo, "dev");
        let head_path = temp.path().join(".git").join("HEAD");
        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(head_path);
        on_git_head_change(Arc::downgrade(&gcx), event).await;
        tokio::time::sleep(BRANCH_HEAD_DEBOUNCE_WINDOW + Duration::from_millis(80)).await;

        assert_eq!(service.counts().await.unwrap().files, 0);
        assert_eq!(service.drain_batch(10), vec![deleted_path.clone()]);
        let workspace_files = gcx.documents_state.workspace_files.lock().unwrap().clone();
        assert!(!workspace_files.contains(&PathBuf::from(&deleted_path)));
        assert!(workspace_files.contains(&kept_path));
    }

    #[tokio::test]
    async fn branch_switch_diff_failure_falls_back_to_full_workspace_reindex() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src").join("fallback.rs");
        write_file(&file, "pub fn fallback() {}\n");
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![normalized(temp.path())];
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        *gcx.codegraph.lock().await = Some(service.clone());

        enqueue_branch_head_changes(
            gcx.clone(),
            vec![BranchHeadChange {
                repo_path: temp.path().join("not-a-repo"),
                old_head: Some("missing-old".to_string()),
                new_head: Some("missing-new".to_string()),
            }],
        )
        .await;

        let indexed_path = normalized(&file).to_string_lossy().to_string();
        assert_eq!(service.drain_batch(10), vec![indexed_path.clone()]);
        assert!(gcx
            .documents_state
            .workspace_files
            .lock()
            .unwrap()
            .contains(&PathBuf::from(indexed_path)));
    }

    #[tokio::test]
    async fn rename_remove_create_removes_old_and_enqueues_new() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        // Canonical root first: raw event paths must equal canonical index keys
        // on Windows runners with an 8.3 short TMP.
        let root = normalized(temp.path());
        std::fs::create_dir_all(root.join("src")).unwrap();
        let old_file = root.join("src").join("old.rs");
        let new_file = root.join("src").join("new.rs");
        write_file(&old_file, "pub fn old_name() {}\n");
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        *gcx.documents_state.workspace_files.lock().unwrap() = vec![normalized(&old_file)];
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        service
            .index_file(
                &normalized(&old_file).to_string_lossy(),
                "pub fn old_name() {}\n",
                "rust",
            )
            .await
            .unwrap();
        assert_eq!(service.counts().await.unwrap().files, 1);
        *gcx.codegraph.lock().await = Some(service.clone());

        std::fs::remove_file(&old_file).unwrap();
        let remove_event =
            notify::Event::new(notify::EventKind::Remove(notify::event::RemoveKind::File))
                .add_path(old_file.clone());
        file_watcher_event(remove_event, Arc::downgrade(&gcx)).await;
        write_file(&new_file, "pub fn new_name() {}\n");
        let create_event =
            notify::Event::new(notify::EventKind::Create(notify::event::CreateKind::File))
                .add_path(new_file.clone());
        file_watcher_event(create_event, Arc::downgrade(&gcx)).await;
        tokio::time::sleep(FILE_EVENT_DEBOUNCE_WINDOW + Duration::from_millis(80)).await;

        let workspace_files = gcx.documents_state.workspace_files.lock().unwrap().clone();
        assert!(!workspace_files.contains(&normalized(&old_file)));
        assert!(workspace_files.contains(&normalized(&new_file)));
        assert_eq!(service.counts().await.unwrap().files, 0);
        assert_eq!(
            service.drain_batch(10),
            vec![
                normalized(&old_file).to_string_lossy().to_string(),
                normalized(&new_file).to_string_lossy().to_string()
            ]
        );
    }
}
