use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex as StdMutex};

use notify::RecommendedWatcher;
use refact_ast::Document;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};

use crate::correction_cache::CacheCorrection;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[tokio::test]
    async fn new_starts_with_empty_workspace_files() {
        let workspace_dir = PathBuf::from("/tmp/refact-test-workspace");
        let state = DocumentsState::new(vec![workspace_dir.clone()]).await;

        assert_eq!(
            state.workspace_folders.lock().unwrap().as_slice(),
            &[workspace_dir]
        );
        assert!(state.workspace_files.lock().unwrap().is_empty());
        assert!(state.workspace_vcs_roots.lock().unwrap().is_empty());
        assert!(state.active_file_path.lock().await.is_none());
        assert!(state.jsonl_files.lock().unwrap().is_empty());
        assert!(state.memory_document_map.lock().await.is_empty());
        assert_eq!(*state.cache_dirty.lock().await, 0.0);
        assert!(state.fs_watcher.lock().unwrap().is_none());
        assert!(state.git_branch_heads.lock().unwrap().is_empty());
        assert_eq!(state.branch_reindex_last_ts.load(Ordering::Relaxed), 0);
    }
}
