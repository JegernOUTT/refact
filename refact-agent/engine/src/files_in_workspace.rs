use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Weak, Mutex as StdMutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use indexmap::IndexSet;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind, RemoveKind};
use ropey::Rope;
use tokio::sync::{RwLock as ARwLock, Mutex as AMutex, Notify};
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
use crate::privacy::{check_file_privacy, load_privacy_if_needed, PrivacySettings, FilePrivacyLevel};
use crate::files_blocklist::{IndexingEverywhere, is_blocklisted, reload_indexing_everywhere_if_needed};
use crate::files_in_jsonl::enqueue_all_docs_from_jsonl_but_read_first;

pub use refact_files::correction_cache::CacheCorrection;

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
    let requested_path = crate::files_correction::canonical_path(file_path.to_string_lossy());
    let mapped_path = normalize_path_for_workspace_state(&global_context, &requested_path);
    check_file_privacy(
        load_privacy_if_needed(global_context.clone()).await,
        &requested_path,
        &FilePrivacyLevel::AllowToSendAnywhere,
    )?;

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
        if doc.doc_text.is_some() {
            return Ok(doc.doc_text.as_ref().unwrap().to_string());
        }
    }
    read_file_from_disk_without_privacy_check(&requested_path)
        .await
        .map(|x| x.to_string())
        .map_err(|e| format!("Not found in memory, not found on disk: {}", e))
}

pub async fn check_file_privacy_for_send(
    global_context: Arc<GlobalContext>,
    file_path: &PathBuf,
) -> Result<(), String> {
    check_file_privacy(
        load_privacy_if_needed(global_context).await,
        file_path,
        &FilePrivacyLevel::AllowToSendAnywhere,
    )
}

pub async fn filter_privacy_allowed_files(
    global_context: Arc<GlobalContext>,
    files: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let privacy = load_privacy_if_needed(global_context).await;
    files
        .into_iter()
        .filter(|path| {
            check_file_privacy(
                privacy.clone(),
                path,
                &FilePrivacyLevel::AllowToSendAnywhere,
            )
            .is_ok()
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
    read_path: PathBuf,
    at: Instant,
    kind: DebouncedFileEventKind,
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
    file_event_debounce_task: Arc<StdMutex<Option<tokio::task::JoinHandle<()>>>>,
    file_event_debounce_notify: Arc<Notify>,
    branch_head_debounce: Arc<StdMutex<HashMap<PathBuf, PendingBranchHeadChange>>>,
    branch_head_debounce_tasks: Arc<StdMutex<HashMap<PathBuf, tokio::task::JoinHandle<()>>>>,
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
            file_event_debounce_task: Arc::new(StdMutex::new(None)),
            file_event_debounce_notify: Arc::new(Notify::new()),
            branch_head_debounce: Arc::new(StdMutex::new(HashMap::new())),
            branch_head_debounce_tasks: Arc::new(StdMutex::new(HashMap::new())),
        }
    }
}

pub async fn watcher_init(gcx: Arc<GlobalContext>) {
    let gcx_weak = Arc::downgrade(&gcx);
    let rt = tokio::runtime::Handle::current();
    let event_callback = move |res| {
        rt.block_on(async {
            if let Ok(event) = res {
                file_watcher_event(event, gcx_weak.clone()).await;
            }
        });
    };
    let mut watcher = match RecommendedWatcher::new(event_callback, Config::default()) {
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

    for folder in &watch_folders {
        info!("ADD WATCHER (1): {}", folder.display());
        let _ = watcher.watch(folder, RecursiveMode::Recursive);
    }

    let new_watcher = Some(Arc::new(ARwLock::new(watcher)));
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

fn path_is_refact_internal(path: &Path) -> bool {
    path_is_refact_import_internal(path) || crate::file_filter::is_refact_codegraph_path(path)
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
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        *dirty_arc.lock().await = now + 1.0;
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
        // indexing_everywhere is immutable in shared GlobalContext; callers will reload as needed.
        gcx.documents_state.cache_dirty.clone()
    };

    *cache_dirty.lock().await = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

    let mut updated_or_removed: IndexSet<String> = IndexSet::new();
    updated_or_removed.extend(
        all_files
            .iter()
            .map(|file| file.to_string_lossy().to_string()),
    );
    updated_or_removed.extend(
        old_workspace_files
            .iter()
            .map(|p| p.to_string_lossy().to_string()),
    );
    let paths_nodups: Vec<String> = updated_or_removed.into_iter().collect();

    crate::indexing_routing::route_index_enqueue(
        gcx.clone(),
        &paths_nodups,
        wake_up_indexers,
        vecdb_only,
    )
    .await;

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

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    (*dirty_arc.lock().await) = now;

    let delete_path = normalize_path_for_index_store(&gcx, path);

    match *vec_db_module.lock().await {
        Some(ref mut db) => match db.remove_file(&delete_path).await {
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

const FILE_EVENT_DEBOUNCE_WINDOW: Duration = Duration::from_millis(100);
const FILE_EVENT_DEBOUNCE_RETAIN: Duration = Duration::from_secs(10);
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
    queue_path: QueuedPath,
    kind: DebouncedFileEventKind,
    now: Instant,
) {
    {
        let mut debounce = gcx.documents_state.file_event_debounce.lock().unwrap();
        debounce.retain(|_, event| {
            now.saturating_duration_since(event.at) <= FILE_EVENT_DEBOUNCE_RETAIN
        });
        debounce.insert(
            PathBuf::from(queue_path.store_path),
            DebouncedFileEvent {
                read_path: PathBuf::from(queue_path.read_path),
                at: now,
                kind,
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

fn take_all_debounced_file_events(gcx: &Arc<GlobalContext>) -> Vec<PendingFileEvent> {
    let mut debounce = gcx.documents_state.file_event_debounce.lock().unwrap();
    let mut events = debounce
        .drain()
        .map(|(store_path, event)| PendingFileEvent {
            store_path,
            read_path: event.read_path,
            kind: event.kind,
        })
        .collect::<Vec<_>>();
    events.sort_by(|a, b| {
        a.kind_order()
            .cmp(&b.kind_order())
            .then_with(|| a.store_path.cmp(&b.store_path))
    });
    events
}

fn take_due_debounced_file_events(
    gcx: &Arc<GlobalContext>,
    now: Instant,
) -> (Vec<PendingFileEvent>, Option<Duration>) {
    let mut debounce = gcx.documents_state.file_event_debounce.lock().unwrap();
    debounce
        .retain(|_, event| now.saturating_duration_since(event.at) <= FILE_EVENT_DEBOUNCE_RETAIN);
    let mut due_paths = Vec::new();
    let mut wait_for: Option<Duration> = None;
    for (store_path, event) in debounce.iter() {
        let elapsed = now.saturating_duration_since(event.at);
        if elapsed >= FILE_EVENT_DEBOUNCE_WINDOW {
            due_paths.push(store_path.clone());
        } else {
            let remaining = FILE_EVENT_DEBOUNCE_WINDOW.saturating_sub(elapsed);
            wait_for = Some(wait_for.map_or(remaining, |current| current.min(remaining)));
        }
    }
    let mut due = Vec::new();
    for path in due_paths {
        if let Some(event) = debounce.remove(&path) {
            due.push(PendingFileEvent {
                store_path: path,
                read_path: event.read_path,
                kind: event.kind,
            });
        }
    }
    due.sort_by(|a, b| {
        a.kind_order()
            .cmp(&b.kind_order())
            .then_with(|| a.store_path.cmp(&b.store_path))
    });
    (due, wait_for)
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
            let paths = take_all_debounced_file_events(&gcx);
            flush_debounced_file_paths(gcx.clone(), paths).await;
            if finish_file_event_debounce_worker(&gcx) {
                return;
            }
            continue;
        }

        let (paths, wait_for) = take_due_debounced_file_events(&gcx, Instant::now());
        if !paths.is_empty() {
            flush_debounced_file_paths(gcx.clone(), paths).await;
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

pub async fn file_watcher_event(event: Event, gcx_weak: Weak<GlobalContext>) {
    async fn on_file_change(gcx_weak: Weak<GlobalContext>, event: Event) {
        let gcx = match gcx_weak.clone().upgrade() {
            Some(gcx) => gcx,
            None => return,
        };
        let indexing_everywhere_arc = reload_indexing_everywhere_if_needed(gcx.clone()).await;
        if event.paths.iter().any(|p| path_triggers_registry_reload(p)) {
            crate::yaml_configs::customization_registry::invalidate_all_registry_caches(
                gcx.clone(),
            )
            .await;
        }
        let mut blocklist_roots = gcx
            .documents_state
            .workspace_vcs_roots
            .lock()
            .unwrap()
            .clone();
        blocklist_roots
            .extend(crate::files_correction::get_unscoped_project_dirs(gcx.clone()).await);
        blocklist_roots = blocklist_roots
            .into_iter()
            .map(crate::files_correction::canonicalize_normalized_path)
            .collect();
        blocklist_roots.sort();
        blocklist_roots.dedup();
        let worktree_mappings =
            crate::files_correction::registered_worktree_path_mappings(gcx.cache_dir.as_path());
        let global_config_roots = vec![gcx.config_dir.clone()];
        let debounce_now = Instant::now();
        let is_remove_event = matches!(&event.kind, EventKind::Remove(_));
        for p in &event.paths {
            if path_is_refact_internal(p) {
                continue;
            }
            let queue_path = resolve_codegraph_queue_path(p, &worktree_mappings);
            let store_path = PathBuf::from(&queue_path.store_path);
            let read_path = PathBuf::from(&queue_path.read_path);
            let indexing_settings = indexing_everywhere_arc.indexing_for_path(&store_path);
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
            if let Some(kind) = debounced_file_event_kind_for_path(
                is_remove_event,
                &read_path,
                &store_path,
                &blocklist_roots,
                &global_config_roots,
            ) {
                schedule_debounced_file_event(gcx.clone(), queue_path, kind, debounce_now);
            }
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

    fn path_counts(paths: &[PathBuf]) -> HashMap<PathBuf, usize> {
        let mut counts = HashMap::new();
        for path in paths {
            *counts.entry(path.clone()).or_insert(0) += 1;
        }
        counts
    }

    #[test]
    fn tiering_is_permutation_nothing_dropped() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".git")).unwrap();
        std::fs::create_dir_all(temp.path().join("competitors").join("one").join(".git")).unwrap();
        std::fs::create_dir_all(temp.path().join("vendor").join("two").join(".git")).unwrap();
        let primary_a = temp.path().join("src").join("a.rs");
        let nested_a = temp
            .path()
            .join("competitors")
            .join("one")
            .join("src")
            .join("a.rs");
        let primary_b = temp.path().join("src").join("b.rs");
        let nested_b = temp
            .path()
            .join("vendor")
            .join("two")
            .join("src")
            .join("b.rs");
        write_file(&primary_a, "fn a() {}\n");
        write_file(&nested_a, "fn nested_a() {}\n");
        write_file(&primary_b, "fn b() {}\n");
        write_file(&nested_b, "fn nested_b() {}\n");
        let files = vec![
            normalized(&primary_a),
            normalized(&nested_a),
            normalized(&primary_b),
            normalized(&nested_b),
        ];

        let ordered =
            order_workspace_files_for_initial_enqueue(files.clone(), &[normalized(temp.path())]);

        assert_eq!(path_counts(&ordered), path_counts(&files));
        assert_eq!(
            ordered,
            vec![
                normalized(&primary_a),
                normalized(&primary_b),
                normalized(&nested_a),
                normalized(&nested_b),
            ]
        );
    }

    #[test]
    fn nested_git_detection() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".git")).unwrap();
        std::fs::create_dir_all(temp.path().join("nested_a").join(".git")).unwrap();
        std::fs::create_dir_all(temp.path().join("nested_b").join(".git")).unwrap();
        let primary = temp.path().join("src").join("main.rs");
        let nested_a = temp.path().join("nested_a").join("lib.rs");
        let nested_b = temp.path().join("nested_b").join("lib.rs");
        write_file(&primary, "fn main() {}\n");
        write_file(&nested_a, "fn nested_a() {}\n");
        write_file(&nested_b, "fn nested_b() {}\n");
        let files = vec![
            normalized(&nested_a),
            normalized(&primary),
            normalized(&nested_b),
        ];

        let ordered =
            order_workspace_files_for_initial_enqueue(files.clone(), &[normalized(temp.path())]);

        assert_eq!(
            ordered,
            vec![
                normalized(&primary),
                normalized(&nested_a),
                normalized(&nested_b),
            ]
        );

        let no_git = tempfile::tempdir().unwrap();
        let plain = no_git.path().join("plain.rs");
        let nested = no_git.path().join("nested").join("lib.rs");
        std::fs::create_dir_all(no_git.path().join("nested").join(".git")).unwrap();
        write_file(&plain, "fn plain() {}\n");
        write_file(&nested, "fn nested() {}\n");
        let files = vec![normalized(&nested), normalized(&plain)];

        let ordered =
            order_workspace_files_for_initial_enqueue(files.clone(), &[normalized(no_git.path())]);

        assert_eq!(ordered, files);
    }

    #[tokio::test]
    async fn workspace_scan_excludes_refact_import_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let regular = temp.path().join("src").join("main.rs");
        let manifest = temp
            .path()
            .join(".refact")
            .join("imports")
            .join("competitors.json");
        write_file(&regular, "fn main() {}\n");
        write_file(&manifest, "{\"ok\":true}");

        let files = scan_workspace(temp.path()).await;

        assert!(files.contains(&normalized(&regular)));
        assert!(!files.contains(&normalized(&manifest)));
    }

    #[tokio::test]
    async fn workspace_scan_excludes_refact_import_staging_content() {
        let temp = tempfile::tempdir().unwrap();
        let regular = temp.path().join("src").join("lib.rs");
        let staged = temp
            .path()
            .join(".refact")
            .join("imports")
            .join("staging")
            .join("skill")
            .join("SKILL.md");
        write_file(&regular, "pub fn ok() {}\n");
        write_file(&staged, "staged skill content\n");

        let files = scan_workspace(temp.path()).await;

        assert!(files.contains(&normalized(&regular)));
        assert!(!files.contains(&normalized(&staged)));
    }

    #[tokio::test]
    async fn workspace_scan_keeps_refact_skills() {
        let temp = tempfile::tempdir().unwrap();
        let skill = temp
            .path()
            .join(".refact")
            .join("skills")
            .join("example")
            .join("SKILL.md");
        write_file(&skill, "# Example skill\nUse this skill.\n");

        let files = scan_workspace(temp.path()).await;

        assert!(files.contains(&normalized(&skill)));
    }

    #[tokio::test]
    async fn workspace_scan_excludes_refact_generated_index_json() {
        let temp = tempfile::tempdir().unwrap();
        let regular = temp.path().join("docs").join("index.json");
        let generated = temp
            .path()
            .join(".refact")
            .join("trajectories")
            .join("index.json");
        write_file(&regular, "{\"user\":true}\n");
        write_file(&generated, "{\"schema_version\":1}\n");

        let files = scan_workspace(temp.path()).await;

        assert!(files.contains(&normalized(&regular)));
        assert!(!files.contains(&normalized(&generated)));
    }

    #[tokio::test]
    async fn workspace_scan_keeps_repo_local_config_refact_index_json() {
        let temp = tempfile::tempdir().unwrap();
        let config_index = temp
            .path()
            .join(".config")
            .join("refact")
            .join("trajectories")
            .join("index.json");
        write_file(&config_index, "{\"user\":true}\n");

        let files = scan_workspace(temp.path()).await;

        assert!(files.contains(&normalized(&config_index)));
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
    async fn on_did_change_maps_registered_worktree_file_to_source_cache_path() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("source");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn old() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn old() {}\n");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);

        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![worktree.clone()];
        *gcx.documents_state.workspace_files.lock().unwrap() = vec![normalized(&worktree_file)];
        *gcx.documents_state.cache_dirty.lock().await = 1.0;
        crate::files_correction::files_cache_rebuild_as_needed(gcx.clone()).await;

        on_did_change(
            gcx.clone(),
            &worktree_file,
            &"fn changed() {}\n".to_string(),
        )
        .await;

        let workspace_files = gcx.documents_state.workspace_files.lock().unwrap().clone();
        assert!(workspace_files.contains(&normalized(&source_file)));
        assert!(!workspace_files.contains(&normalized(&worktree_file)));
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
    async fn raw_worktree_watcher_event_uses_source_blocklist_after_mapping() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("source");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let source_file = source.join("src").join("blocked.rs");
        write_file(&source_file, "fn blocked() {}\n");
        write_file(
            &source.join(".refact").join("indexing.yaml"),
            "blocklist:\n  - 'src/blocked.rs'\n",
        );

        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("blocked.rs");
        write_file(&worktree_file, "fn blocked_worktree() {}\n");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![worktree.clone()];
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() = vec![source.clone()];

        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(worktree_file.clone());
        file_watcher_event(event, Arc::downgrade(&gcx)).await;

        let workspace_files = gcx.documents_state.workspace_files.lock().unwrap().clone();
        assert!(!workspace_files.contains(&normalized(&source_file)));
        assert!(!workspace_files.contains(&normalized(&worktree_file)));
    }

    #[tokio::test]
    async fn on_did_delete_registered_worktree_file_removes_source_keyed_codegraph_row() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("source");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn source() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree() {}\n");
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
    async fn on_did_open_ignores_refact_import_internal_paths() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let path = temp
            .path()
            .join(".refact")
            .join("imports")
            .join("staging")
            .join("x.md");
        let text = "staged content".to_string();
        let language_id = "markdown".to_string();

        on_did_open(gcx.clone(), &path, &text, &language_id).await;

        let (memory_doc_map, active_fp) = {
            (
                gcx.documents_state.memory_document_map.clone(),
                gcx.documents_state.active_file_path.clone(),
            )
        };
        let has_doc = memory_doc_map.lock().await.contains_key(&path);
        let active_file_path = active_fp.lock().await.clone();
        assert!(!has_doc);
        assert!(active_file_path.is_none());
        assert_eq!(cache_dirty_value(&gcx).await, 0.0);
    }

    #[tokio::test]
    async fn on_did_change_ignores_refact_import_internal_paths() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let path = temp
            .path()
            .join(".refact")
            .join("imports")
            .join("staging")
            .join("x.md");
        let text = "changed staged content".to_string();

        on_did_change(gcx.clone(), &path, &text).await;

        let (memory_doc_map2, active_fp2, workspace_files_len) = {
            let wf_len = gcx.documents_state.workspace_files.lock().unwrap().len();
            (
                gcx.documents_state.memory_document_map.clone(),
                gcx.documents_state.active_file_path.clone(),
                wf_len,
            )
        };
        let has_doc = memory_doc_map2.lock().await.contains_key(&path);
        let active_file_path = active_fp2.lock().await.clone();
        assert!(!has_doc);
        assert!(active_file_path.is_none());
        assert_eq!(workspace_files_len, 0);
        assert_eq!(cache_dirty_value(&gcx).await, 0.0);
    }

    #[tokio::test]
    async fn watcher_ignores_refact_codegraph_db_changes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let path = temp
            .path()
            .join(".refact")
            .join("codegraph")
            .join("codegraph.sqlite-wal");
        write_file(&path, "sqlite wal content\n");

        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(path.clone());
        file_watcher_event(event, Arc::downgrade(&gcx)).await;

        let workspace_files_len = gcx.documents_state.workspace_files.lock().unwrap().len();
        assert_eq!(workspace_files_len, 0);
        assert_eq!(cache_dirty_value(&gcx).await, 0.0);
    }

    #[tokio::test]
    async fn on_did_delete_ignores_refact_import_internal_paths() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let path = temp
            .path()
            .join(".refact")
            .join("imports")
            .join("competitors.json");
        let mut doc = Document::new(&path);
        doc.update_text(&"{}".to_string());
        {
            gcx.documents_state
                .memory_document_map
                .lock()
                .await
                .insert(path.clone(), Arc::new(ARwLock::new(doc)));
        }

        on_did_delete(gcx.clone(), &path).await;

        let mdm = gcx.documents_state.memory_document_map.clone();
        let has_doc = mdm.lock().await.contains_key(&path);
        assert!(has_doc);
        assert_eq!(cache_dirty_value(&gcx).await, 0.0);
    }

    #[tokio::test]
    async fn on_did_open_keeps_refact_skills_paths() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let path = temp
            .path()
            .join(".refact")
            .join("skills")
            .join("example")
            .join("SKILL.md");
        let text = "# Example skill".to_string();
        let language_id = "markdown".to_string();

        on_did_open(gcx.clone(), &path, &text, &language_id).await;

        let (mdm3, afp3) = {
            (
                gcx.documents_state.memory_document_map.clone(),
                gcx.documents_state.active_file_path.clone(),
            )
        };
        let has_doc = mdm3.lock().await.contains_key(&path);
        let active_file_path = afp3.lock().await.clone();
        assert!(has_doc);
        assert_eq!(active_file_path, Some(path));
        assert!(cache_dirty_value(&gcx).await > 0.0);
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

    #[test]
    fn is_git_head_path_detects_head() {
        assert!(is_git_head_path(Path::new("/project/.git/HEAD")));
        assert!(!is_git_head_path(Path::new("/project/.git/config")));
        assert!(!is_git_head_path(Path::new("/project/src/HEAD")));
        assert!(!is_git_head_path(Path::new("/project/.git")));
    }

    #[test]
    fn read_git_head_returns_trimmed_content() {
        let temp = tempfile::tempdir().unwrap();
        write_head(temp.path(), "ref: refs/heads/main\n");
        let head = read_git_head(temp.path());
        assert_eq!(head, Some("ref: refs/heads/main".to_string()));
    }

    #[test]
    fn read_git_head_returns_none_for_missing_repo() {
        let temp = tempfile::tempdir().unwrap();
        let head = read_git_head(temp.path());
        assert!(head.is_none());
    }

    #[tokio::test]
    async fn debounced_duplicate_events_single_enqueue() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src").join("lib.rs");
        write_file(&file, "pub fn debounce() -> &'static str { \"first\" }\n");
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![normalized(temp.path())];
        *gcx.documents_state.workspace_files.lock().unwrap() = vec![normalized(&file)];
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        *gcx.codegraph.lock().await = Some(service.clone());

        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(file.clone());
        file_watcher_event(event, Arc::downgrade(&gcx)).await;
        write_file(&file, "pub fn debounce() -> &'static str { \"second\" }\n");
        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(file.clone());
        file_watcher_event(event, Arc::downgrade(&gcx)).await;

        assert_eq!(service.queue_len(), 0);
        tokio::time::sleep(FILE_EVENT_DEBOUNCE_WINDOW + Duration::from_millis(80)).await;

        assert_eq!(service.queue_len(), 1);
        let batch = service.drain_batch(10);
        let indexed_path = normalized(&file).to_string_lossy().to_string();
        assert_eq!(batch, vec![indexed_path.clone()]);
        let text = get_file_text_from_memory_or_disk(gcx.clone(), &PathBuf::from(&indexed_path))
            .await
            .unwrap();
        service
            .index_file(&indexed_path, &text, "rust")
            .await
            .unwrap();
        let mut files = service.all_files_with_text().await.unwrap();
        files.sort();
        assert_eq!(
            files,
            vec![(
                normalized(&file).to_string_lossy().to_string(),
                "pub fn debounce() -> &'static str { \"second\" }\n".to_string(),
            ),]
        );
    }

    #[tokio::test]
    async fn debounced_worktree_edit_flushes_source_store_with_worktree_read_path() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("source");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn source() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree() {}\n");
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
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn source() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree() {}\n");
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

    #[test]
    fn debounced_file_event_preserves_worktree_read_path_under_source_key() {
        let temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = temp.path().join("source");
        let cache_dir = temp.path().join("cache");
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn source() {}\n");
        let worktree = worktree_root(&cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree() {}\n");
        write_worktree_registry(&cache_dir, &source, &worktree);
        let mappings = crate::files_correction::registered_worktree_path_mappings(&cache_dir);
        let queue_path = resolve_codegraph_queue_path(&worktree_file, &mappings);
        let mut debounce = HashMap::new();

        debounce.insert(
            PathBuf::from(&queue_path.store_path),
            DebouncedFileEvent {
                read_path: PathBuf::from(&queue_path.read_path),
                at: Instant::now(),
                kind: DebouncedFileEventKind::Upsert,
            },
        );

        let event = debounce
            .remove(&normalized(&source_file))
            .expect("debounce key should be canonical source path");
        assert_eq!(event.read_path, normalized(&worktree_file));
    }

    #[tokio::test]
    async fn worktree_remove_event_preserves_remove_kind_when_source_exists() {
        let source_temp = tempfile::Builder::new()
            .prefix("refact-src-")
            .tempdir()
            .unwrap();
        let source = source_temp.path().join("source");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        allow_all_privacy(&gcx);
        let source_file = source.join("src").join("lib.rs");
        write_file(&source_file, "fn source() {}\n");
        let worktree = worktree_root(&gcx.cache_dir, &source, "wt");
        let worktree_file = worktree.join("src").join("lib.rs");
        write_file(&worktree_file, "fn worktree() {}\n");
        write_worktree_registry(&gcx.cache_dir, &source, &worktree);
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![worktree.clone()];

        let event = notify::Event::new(notify::EventKind::Remove(notify::event::RemoveKind::File))
            .add_path(worktree_file.clone());
        file_watcher_event(event, Arc::downgrade(&gcx)).await;

        let event = gcx
            .documents_state
            .file_event_debounce
            .lock()
            .unwrap()
            .get(&normalized(&source_file))
            .cloned()
            .expect("remove event should be keyed by canonical source path");
        assert_eq!(event.kind, DebouncedFileEventKind::Remove);
        assert_eq!(event.read_path, normalized(&worktree_file));
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
        schedule_debounced_file_event(
            gcx.clone(),
            QueuedPath::new(
                normalized(&first).to_string_lossy().to_string(),
                normalized(&first).to_string_lossy().to_string(),
            ),
            DebouncedFileEventKind::Upsert,
            now,
        );
        schedule_debounced_file_event(
            gcx.clone(),
            QueuedPath::new(
                normalized(&second).to_string_lossy().to_string(),
                normalized(&second).to_string_lossy().to_string(),
            ),
            DebouncedFileEventKind::Upsert,
            now,
        );
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
    async fn close_write_event_routes_to_file_change() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src").join("close_write.rs");
        write_file(&file, "pub fn close_write() {}\n");
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![normalized(temp.path())];
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        *gcx.codegraph.lock().await = Some(service.clone());

        let event = notify::Event::new(notify::EventKind::Access(AccessKind::Close(
            AccessMode::Write,
        )))
        .add_path(file.clone());
        file_watcher_event(event, Arc::downgrade(&gcx)).await;
        tokio::time::sleep(FILE_EVENT_DEBOUNCE_WINDOW + Duration::from_millis(80)).await;

        assert_eq!(
            service.drain_batch(10),
            vec![normalized(&file).to_string_lossy().to_string()]
        );
    }

    #[tokio::test]
    async fn rename_remove_create_removes_old_and_enqueues_new() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let old_file = temp.path().join("src").join("old.rs");
        let new_file = temp.path().join("src").join("new.rs");
        write_file(&old_file, "pub fn old_name() {}\n");
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![normalized(temp.path())];
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

    #[tokio::test]
    async fn branch_head_change_triggers_reindex() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let file = temp.path().join("src").join("lib.rs");
        write_file(&file, "pub fn branch_test() -> &'static str { \"main\" }\n");
        let main_oid = git_commit_all(&repo, "main");
        {
            let main_commit = repo.find_commit(main_oid).unwrap();
            repo.branch("dev", &main_commit, false).unwrap();
        }
        git_checkout_branch(&repo, "dev");
        write_file(&file, "pub fn branch_test() -> &'static str { \"dev\" }\n");
        git_commit_all(&repo, "dev");
        git_checkout_branch(&repo, "main");
        let canonical_repo = normalized(temp.path());
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![canonical_repo.clone()];
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() = vec![canonical_repo.clone()];
        {
            let mut heads = gcx.documents_state.git_branch_heads.lock().unwrap();
            heads.insert(canonical_repo.clone(), "ref: refs/heads/main".to_string());
        }

        git_checkout_branch(&repo, "dev");
        let head_path = temp.path().join(".git").join("HEAD");
        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(head_path);

        on_git_head_change(Arc::downgrade(&gcx), event).await;
        tokio::time::sleep(BRANCH_HEAD_DEBOUNCE_WINDOW + Duration::from_millis(80)).await;

        let heads = gcx.documents_state.git_branch_heads.lock().unwrap();
        assert_eq!(
            heads.get(&canonical_repo),
            Some(&"ref: refs/heads/dev".to_string())
        );
        let ts = gcx
            .documents_state
            .branch_reindex_last_ts
            .load(Ordering::Relaxed);
        assert!(
            ts > 0,
            "reindex timestamp should be set after branch change"
        );
    }

    #[tokio::test]
    async fn no_reindex_when_head_unchanged() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let canonical_repo = normalized(temp.path());

        write_head(temp.path(), "ref: refs/heads/main\n");
        {
            let mut heads = gcx.documents_state.git_branch_heads.lock().unwrap();
            heads.insert(canonical_repo.clone(), "ref: refs/heads/main".to_string());
        }

        let head_path = temp.path().join(".git").join("HEAD");
        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(head_path);

        on_git_head_change(Arc::downgrade(&gcx), event).await;
        tokio::time::sleep(BRANCH_HEAD_DEBOUNCE_WINDOW + Duration::from_millis(80)).await;

        let ts = gcx
            .documents_state
            .branch_reindex_last_ts
            .load(Ordering::Relaxed);
        assert_eq!(
            ts, 0,
            "no reindex should occur when HEAD content is unchanged"
        );
    }

    #[tokio::test]
    async fn debounce_rapid_head_changes_flushes_newest_head() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let file = temp.path().join("src").join("lib.rs");
        write_file(&file, "pub fn branch_test() -> &'static str { \"main\" }\n");
        let main_oid = git_commit_all(&repo, "main");
        {
            let main_commit = repo.find_commit(main_oid).unwrap();
            repo.branch("dev", &main_commit, false).unwrap();
            repo.branch("feature", &main_commit, false).unwrap();
        }
        git_checkout_branch(&repo, "dev");
        write_file(&file, "pub fn branch_test() -> &'static str { \"dev\" }\n");
        git_commit_all(&repo, "dev");
        git_checkout_branch(&repo, "feature");
        write_file(
            &file,
            "pub fn branch_test() -> &'static str { \"feature\" }\n",
        );
        git_commit_all(&repo, "feature");
        git_checkout_branch(&repo, "main");
        let canonical_repo = normalized(temp.path());

        {
            let mut heads = gcx.documents_state.git_branch_heads.lock().unwrap();
            heads.insert(canonical_repo.clone(), "ref: refs/heads/main".to_string());
        }
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![canonical_repo.clone()];
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() = vec![canonical_repo.clone()];
        *gcx.documents_state.workspace_files.lock().unwrap() = vec![normalized(&file)];
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        *gcx.codegraph.lock().await = Some(service.clone());

        git_checkout_branch(&repo, "dev");
        let head_path = temp.path().join(".git").join("HEAD");
        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(head_path);

        on_git_head_change(Arc::downgrade(&gcx), event).await;
        git_checkout_branch(&repo, "feature");
        let head_path = temp.path().join(".git").join("HEAD");
        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(head_path);
        on_git_head_change(Arc::downgrade(&gcx), event).await;
        tokio::time::sleep(BRANCH_HEAD_DEBOUNCE_WINDOW + Duration::from_millis(80)).await;

        let heads = gcx.documents_state.git_branch_heads.lock().unwrap();
        assert_eq!(
            heads.get(&canonical_repo),
            Some(&"ref: refs/heads/feature".to_string()),
            "debounce should keep latest head from rapid changes"
        );
        assert_eq!(
            service.drain_batch(10),
            vec![normalized(&file).to_string_lossy().to_string()],
            "debounce should enqueue the final branch diff once"
        );
    }

    #[test]
    fn registry_reload_ignores_refact_import_paths() {
        assert!(path_is_refact_import_internal(Path::new(
            "/repo/.refact/imports/competitors.json"
        )));
        assert!(!path_is_refact_import_internal(Path::new(
            "/repo/.refact/skills/example/SKILL.md"
        )));
        assert!(!path_triggers_registry_reload(Path::new(
            "/repo/.refact/imports/staging/source/.refact/subagents/agent.yaml"
        )));
        assert!(!path_triggers_registry_reload(Path::new(
            "/repo/.refact/codegraph/codegraph.sqlite-wal"
        )));
        assert!(path_triggers_registry_reload(Path::new(
            "/repo/.refact/modes/agent.yaml"
        )));
        assert!(path_triggers_registry_reload(Path::new(
            "/repo/.refact/subagents/agent.yaml"
        )));
        assert!(path_triggers_registry_reload(Path::new(
            "/repo/.refact/toolbox_commands/command.yaml"
        )));
        assert!(path_triggers_registry_reload(Path::new(
            "/repo/.refact/code_lens/lens.yaml"
        )));
    }
}
