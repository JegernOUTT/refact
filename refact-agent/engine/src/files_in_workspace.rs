use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Weak, Mutex as StdMutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use indexmap::IndexSet;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use notify::event::{CreateKind, ModifyKind, RemoveKind};
use ropey::Rope;
use tokio::sync::{RwLock as ARwLock, Mutex as AMutex};
use walkdir::WalkDir;
use which::which;
use tracing::info;
use chrono::Utc;

use refact_buddy_core::user_action::UserAction;
use crate::files_correction::{canonical_path, CommandSimplifiedDirExt};
use crate::git::operations::git_ls_files;
use crate::global_context::{get_app_searchable_id, GlobalContext};
use crate::integrations::running_integrations::load_integrations;
use crate::file_filter::{is_valid_file, SOURCE_FILE_EXTENSIONS};
use crate::ast::ast_indexer_thread::ast_indexer_enqueue_files;
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

pub use refact_ast::Document;

pub async fn get_file_text_from_memory_or_disk(
    global_context: Arc<GlobalContext>,
    file_path: &PathBuf,
) -> Result<String, String> {
    check_file_privacy(
        load_privacy_if_needed(global_context.clone()).await,
        &file_path,
        &FilePrivacyLevel::AllowToSendAnywhere,
    )?;

    if let Some(doc) = global_context
        .documents_state
        .memory_document_map
        .lock()
        .await
        .get(file_path)
    {
        let doc = doc.read().await;
        if doc.doc_text.is_some() {
            return Ok(doc.doc_text.as_ref().unwrap().to_string());
        }
    }
    read_file_from_disk_without_privacy_check(&file_path)
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

    let workspace_folders: Arc<StdMutex<Vec<PathBuf>>> =
        gcx.documents_state.workspace_folders.clone();

    for folder in workspace_folders.lock().unwrap().iter() {
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
    if path.join(".git").is_dir() {
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

fn path_triggers_registry_reload(path: &Path) -> bool {
    if path_is_refact_import_internal(path) {
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
) -> Result<(), Box<dyn std::error::Error>> {
    if path_is_refact_import_internal(path) {
        return Err(".refact/imports is internal".into());
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
        )
        .await;
    }
    info!("in all workspace folders, VCS roots found:");
    for vcs_folder in vcs_folders.iter() {
        info!("    {}", vcs_folder.display());
    }
    (all_files, vcs_folders)
}

pub fn is_path_to_enqueue_valid(path: &PathBuf) -> Result<(), String> {
    let extension = path.extension().unwrap_or_default();
    if !SOURCE_FILE_EXTENSIONS.contains(&extension.to_str().unwrap_or_default()) {
        return Err(format!("Unsupported file extension {:?}", extension).into());
    }
    Ok(())
}

async fn enqueue_some_docs(gcx: Arc<GlobalContext>, paths: &Vec<String>, force: bool) {
    info!("detected {} modified/added/removed files", paths.len());
    for d in paths.iter().take(5) {
        info!("    {}", crate::nicer_logs::last_n_chars(&d, 30));
    }
    if paths.len() > 5 {
        info!("    ...");
    }
    let (vec_db_module, ast_service) = {
        let cx = gcx.clone();
        let ast_service = cx.ast_service.lock().unwrap().clone();
        (cx.vec_db.clone(), ast_service)
    };
    if let Some(ref mut db) = *vec_db_module.lock().await {
        db.vectorizer_enqueue_files(&paths, force).await;
    }
    if let Some(ast) = &ast_service {
        ast_indexer_enqueue_files(ast.clone(), paths, force).await;
    }
    let cache_correction_arc =
        crate::files_correction::files_cache_rebuild_as_needed(gcx.clone()).await;
    let mut moar_files: Vec<PathBuf> = Vec::new();
    for p in paths {
        if cache_correction_arc
            .filenames
            .find_matches(&PathBuf::from(p))
            .len()
            == 0
        {
            moar_files.push(PathBuf::from(p.clone()));
        }
    }
    if moar_files.len() > 0 {
        info!("this made file cache dirty");
        let dirty_arc = {
            gcx.documents_state
                .workspace_files
                .lock()
                .unwrap()
                .extend(moar_files);
            gcx.documents_state.cache_dirty.clone()
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        *dirty_arc.lock().await = now + 1.0; // next rebuild will be one second later, to prevent rapid-fire rebuilds from file events
    }
}

pub async fn enqueue_all_files_from_workspace_folders(
    gcx: Arc<GlobalContext>,
    wake_up_indexers: bool,
    vecdb_only: bool,
) -> i32 {
    let folders = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();

    info!(
        "enqueue_all_files_from_workspace_folders started files search with {} folders",
        folders.len()
    );
    let mut indexing_everywhere =
        crate::files_blocklist::reload_global_indexing_only(gcx.clone()).await;
    let (all_files, vcs_folders) =
        retrieve_files_in_workspace_folders(folders, &mut indexing_everywhere, false, false).await;
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
        // indexing_everywhere is immutable in shared GlobalContext; callers will reload as needed.
        gcx.documents_state.cache_dirty.clone()
    };

    *cache_dirty.lock().await = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

    let (vec_db_module, ast_service) = {
        let ast_service = gcx.ast_service.lock().unwrap().clone();
        (gcx.vec_db.clone(), ast_service)
    };

    // Both vecdb and ast support paths to non-existant files (possibly previously existing files) as a way to remove them from index

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

    if let Some(ref mut db) = *vec_db_module.lock().await {
        db.vectorizer_enqueue_files(&paths_nodups, wake_up_indexers)
            .await;
    }

    if let Some(ast) = ast_service {
        if !vecdb_only {
            ast_indexer_enqueue_files(ast.clone(), &paths_nodups, wake_up_indexers).await;
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
    if path_is_refact_import_internal(cpath) {
        return;
    }
    let mut doc = Document::new(cpath);
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
    {
        let cx = gcx.clone();
        if cx
            .documents_state
            .memory_document_map
            .lock()
            .await
            .remove(cpath)
            .is_none()
        {
            tracing::error!(
                "on_did_close: failed to remove from memory_document_map {:?}",
                cpath.display()
            );
        }
    }
}

pub async fn on_did_change(gcx: Arc<GlobalContext>, path: &PathBuf, text: &String) {
    if path_is_refact_import_internal(path) {
        return;
    }
    let t0 = Instant::now();
    let (doc_arc, dirty_arc, mark_dirty) = {
        let mut doc = Document::new(path);
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
        let is_it_good = is_valid_file(path, false, false);
        if is_it_good.is_err() {
            info!("{:?} ignoring changes: {}", path, is_it_good.err().unwrap());
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
    if path_is_refact_import_internal(path) {
        return;
    }
    info!(
        "on_did_delete {}",
        crate::nicer_logs::last_n_chars(&path.to_string_lossy().to_string(), 30)
    );

    let (vec_db_module, ast_service, dirty_arc) = {
        let cx = gcx.clone();
        cx.documents_state
            .memory_document_map
            .lock()
            .await
            .remove(path);
        let ast_service = cx.ast_service.lock().unwrap().clone();
        (
            cx.vec_db.clone(),
            ast_service,
            cx.documents_state.cache_dirty.clone(),
        )
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    (*dirty_arc.lock().await) = now;

    match *vec_db_module.lock().await {
        Some(ref mut db) => match db.remove_file(path).await {
            Ok(_) => {}
            Err(err) => info!("VECDB Error removing: {}", err),
        },
        None => {}
    }
    if let Some(ast) = &ast_service {
        let cpath = path.to_string_lossy().to_string();
        ast_indexer_enqueue_files(ast.clone(), &vec![cpath], false).await;
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
    std::fs::read_to_string(&head_path)
        .ok()
        .map(|s| s.trim().to_string())
}

fn is_git_head_path(p: &Path) -> bool {
    p.file_name().map(|n| n == "HEAD").unwrap_or(false)
        && p.parent()
            .and_then(|pp| pp.file_name())
            .map(|n| n == ".git")
            .unwrap_or(false)
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

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let last_ms = gcx
        .documents_state
        .branch_reindex_last_ts
        .load(Ordering::Relaxed);
    if now_ms.saturating_sub(last_ms) < 2000 {
        return;
    }

    let mut any_changed = false;
    {
        let mut heads = gcx.documents_state.git_branch_heads.lock().unwrap();
        for repo_path in &repo_paths {
            let new_head = read_git_head(repo_path);
            let old_head = heads.get(repo_path).cloned();
            if new_head != old_head {
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
                        heads.remove(repo_path);
                    }
                }
                any_changed = true;
            }
        }
    }

    if any_changed {
        gcx.documents_state
            .branch_reindex_last_ts
            .store(now_ms, Ordering::Relaxed);
        tracing::info!("Branch switch detected, triggering full workspace reindex");
        enqueue_all_files_from_workspace_folders(gcx, true, false).await;
    }
}

pub async fn on_explicit_branch_change(gcx: Arc<GlobalContext>, repo_path: &PathBuf) {
    let new_head = read_git_head(repo_path);
    {
        let mut heads = gcx.documents_state.git_branch_heads.lock().unwrap();
        match &new_head {
            Some(h) => {
                heads.insert(repo_path.clone(), h.clone());
            }
            None => {
                heads.remove(repo_path);
            }
        }
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    gcx.documents_state
        .branch_reindex_last_ts
        .store(now_ms, Ordering::Relaxed);
    tracing::info!(
        "Explicit branch change notification for {}, triggering full workspace reindex",
        repo_path.display()
    );
    enqueue_all_files_from_workspace_folders(gcx, true, false).await;
}

pub async fn file_watcher_event(event: Event, gcx_weak: Weak<GlobalContext>) {
    async fn on_file_change(gcx_weak: Weak<GlobalContext>, event: Event) {
        let mut docs = vec![];
        let indexing_everywhere_arc;
        if let Some(gcx) = gcx_weak.clone().upgrade() {
            indexing_everywhere_arc = reload_indexing_everywhere_if_needed(gcx.clone()).await;
        } else {
            return; // the program is shutting down
        }
        if let Some(gcx) = gcx_weak.clone().upgrade() {
            if event.paths.iter().any(|p| path_triggers_registry_reload(p)) {
                crate::yaml_configs::customization_registry::invalidate_all_registry_caches(
                    gcx.clone(),
                )
                .await;
            }
        }
        for p in &event.paths {
            if path_is_refact_import_internal(p) {
                continue;
            }
            let indexing_settings = indexing_everywhere_arc.indexing_for_path(p);
            if is_blocklisted(&indexing_settings, &p) {
                // important to filter BEFORE canonical_path
                continue;
            }

            // If it's a removed file or a valid existing file, then we can enqueue it
            if (!p.exists() && p.extension().is_some()) || is_valid_file(p, false, false).is_ok() {
                let cpath = crate::files_correction::canonical_path(p.to_string_lossy());
                docs.push(cpath.to_string_lossy().to_string());
            }
        }
        if docs.is_empty() {
            return;
        }
        // info!("EventKind::Create/Modify/Remove {} paths", event.paths.len());
        if let Some(gcx) = gcx_weak.clone().upgrade() {
            enqueue_some_docs(gcx, &docs, false).await;
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

        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            if event.paths.iter().any(|p| is_git_head_path(p)) =>
        {
            on_git_head_change(gcx_weak.clone(), event).await
        }

        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
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

    async fn scan_workspace(root: &Path) -> Vec<PathBuf> {
        let mut indexing_everywhere = IndexingEverywhere::default();
        let (files, _) = retrieve_files_in_workspace_folders(
            vec![root.to_path_buf()],
            &mut indexing_everywhere,
            false,
            false,
        )
        .await;
        files
    }

    async fn cache_dirty_value(gcx: &Arc<GlobalContext>) -> f64 {
        let dirty = { gcx.documents_state.cache_dirty.clone() };
        let value = *dirty.lock().await;
        value
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
    async fn branch_head_change_triggers_reindex() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let canonical_repo = normalized(temp.path());

        write_head(temp.path(), "ref: refs/heads/main\n");
        {
            let mut heads = gcx.documents_state.git_branch_heads.lock().unwrap();
            heads.insert(canonical_repo.clone(), "ref: refs/heads/main".to_string());
        }

        write_head(temp.path(), "ref: refs/heads/dev\n");

        let head_path = temp.path().join(".git").join("HEAD");
        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(head_path);

        on_git_head_change(Arc::downgrade(&gcx), event).await;

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
    async fn debounce_rapid_head_changes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let canonical_repo = normalized(temp.path());

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        gcx.documents_state
            .branch_reindex_last_ts
            .store(now_ms, Ordering::Relaxed);

        write_head(temp.path(), "ref: refs/heads/dev\n");
        {
            let mut heads = gcx.documents_state.git_branch_heads.lock().unwrap();
            heads.insert(canonical_repo.clone(), "ref: refs/heads/main".to_string());
        }

        let head_path = temp.path().join(".git").join("HEAD");
        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(head_path);

        on_git_head_change(Arc::downgrade(&gcx), event).await;

        let heads = gcx.documents_state.git_branch_heads.lock().unwrap();
        assert_eq!(
            heads.get(&canonical_repo),
            Some(&"ref: refs/heads/main".to_string()),
            "debounce should prevent head update during rapid changes"
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
