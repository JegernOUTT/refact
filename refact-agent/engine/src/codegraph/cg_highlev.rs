use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use tracing::{error, info, warn};

use refact_codegraph::{lang_from_path, CodeGraphService, Counts, QueuedPath};

use crate::global_context::GlobalContext;

const CODEGRAPH_DB_FILE: &str = "codegraph.sqlite";
const DRAIN_BATCH: usize = 512;
const DRAIN_CONCURRENCY: usize = 8;
const PROGRESS_REPORT_GRANULARITY: usize = 100;
const CONNECT_EVERY_BATCHES: u32 = 8;
const CONNECT_EVERY_SECS: u64 = 30;

fn progress_bucket(remaining: usize) -> usize {
    (remaining + PROGRESS_REPORT_GRANULARITY - 1) / PROGRESS_REPORT_GRANULARITY
}

fn should_report_unprocessed(remaining: usize, reported_unprocessed: &mut usize) -> bool {
    if remaining == 0 || progress_bucket(remaining) == progress_bucket(*reported_unprocessed) {
        return false;
    }
    *reported_unprocessed = remaining;
    true
}

fn should_connect_usages(batches_since: u32, elapsed: Duration) -> bool {
    batches_since >= CONNECT_EVERY_BATCHES || elapsed >= Duration::from_secs(CONNECT_EVERY_SECS)
}

fn completion_message(counts: &Counts) -> String {
    format!(
        "codegraph: index complete — {} nodes, {} edges, {} files",
        counts.nodes, counts.edges, counts.files
    )
}

fn path_is_genuinely_absent(path: &Path) -> bool {
    matches!(std::fs::metadata(path), Err(err) if err.kind() == std::io::ErrorKind::NotFound)
}

async fn remove_missing_path(
    service: Arc<CodeGraphService>,
    store_path: &str,
    err: String,
) -> Result<(), String> {
    service
        .remove_path(store_path)
        .await
        .map_err(|remove_err| format!("codegraph: remove {store_path} failed: {remove_err}"))
        .map(|_| {
            warn!("codegraph: removed missing path after read failure: {store_path}: {err}");
        })
}

fn stale_stored_paths(stored_paths: Vec<String>) -> Vec<String> {
    stored_paths
        .into_iter()
        .filter(|path| path_is_genuinely_absent(Path::new(path)))
        .collect()
}

async fn reconcile_deleted_paths(service: Arc<CodeGraphService>) -> Result<usize, String> {
    let stale_paths = stale_stored_paths(service.all_paths().await?);
    let removed = stale_paths.len();
    for path in stale_paths {
        service.remove_path(&path).await?;
    }
    Ok(removed)
}

async fn index_or_remove_path(
    gcx: Arc<GlobalContext>,
    service: Arc<CodeGraphService>,
    path: QueuedPath,
) -> Result<(), String> {
    let store_path = path.store_path;
    let read_path = path.read_path;
    let store_path_buf = PathBuf::from(&store_path);
    let read_path_buf = PathBuf::from(&read_path);
    if let Err(err) =
        crate::files_in_workspace::check_file_privacy_for_send(gcx.clone(), &store_path_buf).await
    {
        if path_is_genuinely_absent(&store_path_buf) {
            return remove_missing_path(service, &store_path, err).await;
        }
        warn!("codegraph: read {read_path} failed for {store_path}, keeping existing index: {err}");
        return Ok(());
    }
    match crate::files_in_workspace::get_file_text_from_memory_or_disk(gcx, &read_path_buf).await {
        Ok(text) => {
            let lang = lang_from_path(&store_path);
            service
                .index_file(&store_path, &text, lang)
                .await
                .map_err(|err| format!("codegraph: index {store_path} failed: {err}"))
        }
        Err(err) if path_is_genuinely_absent(&store_path_buf) => {
            remove_missing_path(service, &store_path, err).await
        }
        Err(err) => {
            warn!("codegraph: read {read_path} failed for {store_path}, keeping existing index: {err}");
            Ok(())
        }
    }
}

pub(crate) async fn process_index_batch(
    gcx: Arc<GlobalContext>,
    service: Arc<CodeGraphService>,
    batch: Vec<QueuedPath>,
) {
    let batch_len = batch.len();
    let mut results = stream::iter(batch)
        .map(|path| {
            let gcx = gcx.clone();
            let service = service.clone();
            async move { index_or_remove_path(gcx, service, path).await }
        })
        .buffer_unordered(DRAIN_CONCURRENCY);

    while let Some(result) = results.next().await {
        if let Err(err) = result {
            error!("{err}");
            *gcx.codegraph_error.lock().unwrap() = err;
        }
    }
    service.record_index_completions(batch_len);
}

async fn connect_usages(gcx: &Arc<GlobalContext>, service: &Arc<CodeGraphService>) {
    if let Err(err) = service.connect_usages().await {
        error!("codegraph: connect_usages failed: {err}");
        *gcx.codegraph_error.lock().unwrap() = err;
    } else {
        *gcx.codegraph_error.lock().unwrap() = String::new();
    }
}

async fn maybe_periodic_connect_usages(
    gcx: &Arc<GlobalContext>,
    service: &Arc<CodeGraphService>,
    batches_since_connect: u32,
    elapsed: Duration,
) -> bool {
    if !should_connect_usages(batches_since_connect, elapsed) {
        return false;
    }
    let dirty_count = match service.dirty_usage_path_count().await {
        Ok(dirty_count) => dirty_count,
        Err(err) => {
            error!("codegraph: dirty usage check failed: {err}");
            return false;
        }
    };
    if dirty_count == 0 && batches_since_connect < CONNECT_EVERY_BATCHES {
        return false;
    }
    connect_usages(gcx, service).await;
    info!("codegraph: periodic connect_usages ({dirty_count} dirty)");
    true
}

pub async fn codegraph_db_path(gcx: Arc<GlobalContext>) -> PathBuf {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    if let Some(root) = project_dirs.first() {
        let project_hash = refact_worktrees::service::project_hash_for_path(root);
        gcx.cache_dir
            .join("codegraph")
            .join(project_hash)
            .join(CODEGRAPH_DB_FILE)
    } else {
        gcx.cache_dir.join("codegraph").join(CODEGRAPH_DB_FILE)
    }
}

pub async fn codegraph_init(gcx: Arc<GlobalContext>) {
    let db_path = codegraph_db_path(gcx.clone()).await;
    match CodeGraphService::open(db_path.clone()) {
        Ok(service) => {
            *gcx.codegraph.lock().await = Some(Arc::new(service));
            *gcx.codegraph_error.lock().unwrap() = String::new();
            info!("codegraph: store ready at {db_path:?}");
        }
        Err(err) => {
            error!("codegraph: failed to open store at {db_path:?}: {err}");
            *gcx.codegraph_error.lock().unwrap() = err;
        }
    }
}

pub async fn codegraph_background_task(gcx: Arc<GlobalContext>) {
    let service = match gcx.codegraph.lock().await.clone() {
        Some(service) => service,
        None => return,
    };

    let queue_len_before_enqueue = service.queue_len();
    crate::files_in_workspace::enqueue_all_files_from_workspace_folders(gcx.clone(), true, false)
        .await;
    let initial_enqueued = service.queue_len().saturating_sub(queue_len_before_enqueue);
    info!("codegraph: start_background_tasks, enqueued {initial_enqueued} initial files");

    match reconcile_deleted_paths(service.clone()).await {
        Ok(removed) if removed > 0 => info!("codegraph: removed {removed} stale stored files"),
        Ok(_) => {}
        Err(err) => {
            error!("codegraph: startup deletion reconciliation failed: {err}");
            *gcx.codegraph_error.lock().unwrap() = err;
        }
    }

    let mut reported_unprocessed = 0;
    let mut batches_since_connect: u32 = 0;
    let mut last_connect = Instant::now();

    loop {
        if gcx.shutdown_flag.load(Ordering::Relaxed) {
            break;
        }

        let remaining = service.queue_len();
        if should_report_unprocessed(remaining, &mut reported_unprocessed) {
            info!("codegraph: {remaining} unprocessed files in queue");
        }

        let batch = service.drain_batch_entries(DRAIN_BATCH);
        if batch.is_empty() {
            reported_unprocessed = 0;
            match service.has_dirty_usage_paths().await {
                Ok(true) => {
                    connect_usages(&gcx, &service).await;
                    batches_since_connect = 0;
                    last_connect = Instant::now();
                }
                Ok(false) => {}
                Err(err) => error!("codegraph: dirty usage check failed: {err}"),
            }
            if !service.is_initial_index_done() {
                service.mark_initial_index_done();
                match service.counts().await {
                    Ok(counts) => {
                        info!("{}", completion_message(&counts));
                    }
                    Err(err) => {
                        error!("codegraph: index complete counts failed: {err}");
                        *gcx.codegraph_error.lock().unwrap() = err;
                    }
                }
            }
            let shutdown_flag = gcx.shutdown_flag.clone();
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                _ = service.wait_for_enqueue() => {}
                _ = async move {
                    while !shutdown_flag.load(Ordering::Relaxed) {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                } => break,
            }
            continue;
        }

        process_index_batch(gcx.clone(), service.clone(), batch).await;
        batches_since_connect = batches_since_connect.saturating_add(1);
        if gcx.shutdown_flag.load(Ordering::Relaxed) {
            break;
        }
        if maybe_periodic_connect_usages(
            &gcx,
            &service,
            batches_since_connect,
            last_connect.elapsed(),
        )
        .await
        {
            batches_since_connect = 0;
            last_connect = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::{FilePrivacySettings, PrivacySettings};

    fn set_privacy(gcx: &Arc<GlobalContext>, blocked: Vec<String>) {
        let loaded_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 60;
        *gcx.privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: Vec::new(),
                blocked,
            },
            loaded_ts,
        });
    }

    #[tokio::test]
    async fn codegraph_db_path_uses_cache_dir_for_project() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let project = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.path().to_path_buf()];

        let path = codegraph_db_path(gcx.clone()).await;

        assert!(path.starts_with(gcx.cache_dir.join("codegraph")));
        assert!(!path.starts_with(project.path().join(".refact")));
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(CODEGRAPH_DB_FILE)
        );
    }

    #[test]
    fn progress_latch_reports_only_bucket_changes() {
        let mut reported = 0;

        assert!(should_report_unprocessed(250, &mut reported));
        assert_eq!(reported, 250);
        assert!(!should_report_unprocessed(225, &mut reported));
        assert!(should_report_unprocessed(199, &mut reported));
        assert_eq!(reported, 199);
        assert!(!should_report_unprocessed(0, &mut reported));

        reported = 0;
        assert!(should_report_unprocessed(40, &mut reported));
    }

    #[test]
    fn connect_cadence_uses_batch_or_elapsed_thresholds() {
        assert!(!should_connect_usages(
            CONNECT_EVERY_BATCHES - 1,
            Duration::from_secs(CONNECT_EVERY_SECS - 1)
        ));
        assert!(should_connect_usages(
            CONNECT_EVERY_BATCHES,
            Duration::from_secs(0)
        ));
        assert!(should_connect_usages(
            CONNECT_EVERY_BATCHES + 1,
            Duration::from_secs(0)
        ));
        assert!(should_connect_usages(
            0,
            Duration::from_secs(CONNECT_EVERY_SECS)
        ));
        assert!(should_connect_usages(
            0,
            Duration::from_secs(CONNECT_EVERY_SECS + 1)
        ));
    }

    #[test]
    fn completion_message_includes_counts() {
        let counts = Counts {
            nodes: 11,
            edges: 22,
            files: 3,
            fts_docs: 3,
        };

        assert_eq!(
            completion_message(&counts),
            "codegraph: index complete — 11 nodes, 22 edges, 3 files"
        );
    }

    #[test]
    fn stale_stored_paths_only_removes_paths_absent_on_disk() {
        let temp = tempfile::tempdir().unwrap();
        let kept = temp.path().join("src").join("kept.rs");
        let jsonl_only = temp.path().join("jsonl_only.rs");
        std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
        std::fs::write(&kept, "fn kept() {}\n").unwrap();
        std::fs::write(&jsonl_only, "fn jsonl_only() {}\n").unwrap();
        let kept = kept.to_string_lossy().to_string();
        let missing = temp
            .path()
            .join("src")
            .join("gone.rs")
            .to_string_lossy()
            .to_string();
        let jsonl_only = jsonl_only.to_string_lossy().to_string();

        let stale = stale_stored_paths(vec![kept, missing.clone(), jsonl_only]);

        assert_eq!(stale, vec![missing]);
    }

    #[tokio::test]
    async fn reconcile_deleted_paths_preserves_live_jsonl_only_entries() {
        let service = Arc::new(CodeGraphService::open_in_memory().unwrap());
        let temp = tempfile::tempdir().unwrap();
        let kept = temp.path().join("src").join("kept.rs");
        let jsonl_only = temp.path().join("jsonl_only.rs");
        let gone = temp.path().join("src").join("gone.rs");
        std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
        std::fs::write(&kept, "fn kept() {}\n").unwrap();
        std::fs::write(&jsonl_only, "fn jsonl_only() {}\n").unwrap();
        let kept = kept.to_string_lossy().to_string();
        let jsonl_only = jsonl_only.to_string_lossy().to_string();
        let gone = gone.to_string_lossy().to_string();

        service
            .index_file(&kept, "fn kept() {}\n", "rust")
            .await
            .unwrap();
        service
            .index_file(&jsonl_only, "fn jsonl_only() {}\n", "rust")
            .await
            .unwrap();
        service
            .index_file(&gone, "fn gone() {}\n", "rust")
            .await
            .unwrap();

        let removed = reconcile_deleted_paths(service.clone()).await.unwrap();

        assert_eq!(removed, 1);
        let mut paths = service.all_paths().await.unwrap();
        paths.sort();
        assert_eq!(paths, vec![jsonl_only, kept]);
    }

    #[tokio::test]
    async fn shutdown_set_during_drained_batch_still_indexes_paths() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_privacy(&gcx, Vec::new());
        gcx.shutdown_flag.store(true, Ordering::Relaxed);
        let service = Arc::new(CodeGraphService::open_in_memory().unwrap());
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.rs");
        let second = temp.path().join("second.rs");
        std::fs::write(&first, "fn first() {}\n").unwrap();
        std::fs::write(&second, "fn second() {}\n").unwrap();
        let first = first.to_string_lossy().to_string();
        let second = second.to_string_lossy().to_string();

        process_index_batch(
            gcx,
            service.clone(),
            vec![
                QueuedPath::new(first.clone(), first.clone()),
                QueuedPath::new(second.clone(), second.clone()),
            ],
        )
        .await;

        let mut paths = service.all_paths().await.unwrap();
        paths.sort();
        assert_eq!(paths, vec![first, second]);
    }

    #[tokio::test]
    async fn read_error_for_existing_path_keeps_existing_index() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let service = Arc::new(CodeGraphService::open_in_memory().unwrap());
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("blocked.rs");
        std::fs::write(&file, "fn newer() {}\n").unwrap();
        let file = file.to_string_lossy().to_string();
        service
            .index_file(&file, "fn indexed() {}\n", "rust")
            .await
            .unwrap();
        set_privacy(&gcx, vec![file.clone()]);

        index_or_remove_path(
            gcx,
            service.clone(),
            QueuedPath::new(file.clone(), file.clone()),
        )
        .await
        .unwrap();

        assert_eq!(service.all_paths().await.unwrap(), vec![file.clone()]);
        assert_eq!(
            service.all_files_with_text().await.unwrap(),
            vec![(file, "fn indexed() {}\n".to_string())]
        );
    }

    #[tokio::test]
    async fn missing_read_path_preserves_live_store_path_index() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_privacy(&gcx, Vec::new());
        let service = Arc::new(CodeGraphService::open_in_memory().unwrap());
        let temp = tempfile::tempdir().unwrap();
        let store_path = temp.path().join("source").join("src").join("lib.rs");
        let read_path = temp.path().join("worktree").join("src").join("lib.rs");
        std::fs::create_dir_all(store_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(read_path.parent().unwrap()).unwrap();
        std::fs::write(&store_path, "fn source_live() {}\n").unwrap();
        let store_path = store_path.to_string_lossy().to_string();
        let read_path = read_path.to_string_lossy().to_string();
        service
            .index_file(&store_path, "fn indexed_old() {}\n", "rust")
            .await
            .unwrap();

        index_or_remove_path(
            gcx,
            service.clone(),
            QueuedPath::new(store_path.clone(), read_path),
        )
        .await
        .unwrap();

        assert_eq!(service.all_paths().await.unwrap(), vec![store_path.clone()]);
        assert_eq!(
            service.all_files_with_text().await.unwrap(),
            vec![(store_path, "fn indexed_old() {}\n".to_string())]
        );
    }

    #[tokio::test]
    async fn worktree_read_path_indexes_under_store_path_key() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_privacy(&gcx, Vec::new());
        let service = Arc::new(CodeGraphService::open_in_memory().unwrap());
        let temp = tempfile::tempdir().unwrap();
        let store_path = temp.path().join("source").join("src").join("lib.rs");
        let read_path = temp.path().join("worktree").join("src").join("lib.rs");
        std::fs::create_dir_all(store_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(read_path.parent().unwrap()).unwrap();
        std::fs::write(&store_path, "fn source() {}\n").unwrap();
        std::fs::write(&read_path, "fn worktree() {}\n").unwrap();
        let store_path = store_path.to_string_lossy().to_string();
        let read_path = read_path.to_string_lossy().to_string();

        index_or_remove_path(
            gcx,
            service.clone(),
            QueuedPath::new(store_path.clone(), read_path),
        )
        .await
        .unwrap();

        assert_eq!(
            service.all_files_with_text().await.unwrap(),
            vec![(store_path, "fn worktree() {}\n".to_string())]
        );
    }

    #[tokio::test]
    async fn missing_store_path_removes_existing_index() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_privacy(&gcx, Vec::new());
        let service = Arc::new(CodeGraphService::open_in_memory().unwrap());
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("src").join("gone.rs");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "fn gone() {}\n").unwrap();
        let path = path.to_string_lossy().to_string();
        service
            .index_file(&path, "fn indexed() {}\n", "rust")
            .await
            .unwrap();
        std::fs::remove_file(&path).unwrap();

        index_or_remove_path(
            gcx,
            service.clone(),
            QueuedPath::new(path.clone(), path.clone()),
        )
        .await
        .unwrap();

        assert!(service.all_paths().await.unwrap().is_empty());
        assert!(service.all_files_with_text().await.unwrap().is_empty());
    }
}
