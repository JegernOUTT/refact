use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use tracing::{error, info};

use refact_codegraph::{lang_from_path, CodeGraphService, Counts};

use crate::global_context::GlobalContext;

const CODEGRAPH_DB_FILE: &str = "codegraph.sqlite";
const DRAIN_BATCH: usize = 512;
const DRAIN_CONCURRENCY: usize = 8;
const PROGRESS_REPORT_GRANULARITY: usize = 100;

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

fn completion_message(counts: &Counts) -> String {
    format!(
        "codegraph: index complete — {} nodes, {} edges, {} files",
        counts.nodes, counts.edges, counts.files
    )
}

fn stale_stored_paths(stored_paths: Vec<String>, existing_paths: &HashSet<String>) -> Vec<String> {
    stored_paths
        .into_iter()
        .filter(|path| !existing_paths.contains(path) || !PathBuf::from(path).exists())
        .collect()
}

fn workspace_existing_paths(gcx: &Arc<GlobalContext>) -> HashSet<String> {
    gcx.documents_state
        .workspace_files
        .lock()
        .unwrap()
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect()
}

async fn reconcile_deleted_paths(
    service: Arc<CodeGraphService>,
    existing_paths: &HashSet<String>,
) -> Result<usize, String> {
    let stale_paths = stale_stored_paths(service.all_paths().await?, existing_paths);
    let removed = stale_paths.len();
    for path in stale_paths {
        service.remove_path(&path).await?;
    }
    Ok(removed)
}

async fn index_or_remove_path(
    gcx: Arc<GlobalContext>,
    service: Arc<CodeGraphService>,
    path: String,
) -> Result<(), String> {
    match crate::files_in_workspace::get_file_text_from_memory_or_disk(gcx, &PathBuf::from(&path))
        .await
    {
        Ok(text) => {
            let lang = lang_from_path(&path);
            service
                .index_file(&path, &text, lang)
                .await
                .map_err(|err| format!("codegraph: index {path} failed: {err}"))
        }
        Err(_) => service
            .remove_path(&path)
            .await
            .map_err(|err| format!("codegraph: remove {path} failed: {err}")),
    }
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

    let existing_paths = workspace_existing_paths(&gcx);
    match reconcile_deleted_paths(service.clone(), &existing_paths).await {
        Ok(removed) if removed > 0 => info!("codegraph: removed {removed} stale stored files"),
        Ok(_) => {}
        Err(err) => {
            error!("codegraph: startup deletion reconciliation failed: {err}");
            *gcx.codegraph_error.lock().unwrap() = err;
        }
    }

    let mut reported_unprocessed = 0;

    loop {
        if gcx.shutdown_flag.load(Ordering::Relaxed) {
            break;
        }

        let remaining = service.queue_len();
        if should_report_unprocessed(remaining, &mut reported_unprocessed) {
            info!("codegraph: {remaining} unprocessed files in queue");
        }

        let batch = service.drain_batch(DRAIN_BATCH);
        if batch.is_empty() {
            reported_unprocessed = 0;
            match service.has_dirty_usage_paths().await {
                Ok(true) => {
                    if let Err(err) = service.connect_usages().await {
                        error!("codegraph: connect_usages failed: {err}");
                        *gcx.codegraph_error.lock().unwrap() = err;
                    } else {
                        *gcx.codegraph_error.lock().unwrap() = String::new();
                    }
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
                _ = async move {
                    while !shutdown_flag.load(Ordering::Relaxed) {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                } => break,
            }
            continue;
        }

        let mut results = stream::iter(batch)
            .map(|path| {
                let gcx = gcx.clone();
                let service = service.clone();
                async move {
                    if gcx.shutdown_flag.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    index_or_remove_path(gcx, service, path).await
                }
            })
            .buffer_unordered(DRAIN_CONCURRENCY);

        while let Some(result) = results.next().await {
            if gcx.shutdown_flag.load(Ordering::Relaxed) {
                break;
            }
            if let Err(err) = result {
                error!("{err}");
                *gcx.codegraph_error.lock().unwrap() = err;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn stale_stored_paths_diffs_against_existing_workspace_paths() {
        let temp = tempfile::tempdir().unwrap();
        let kept = temp.path().join("src").join("kept.rs");
        let outside = temp.path().join("outside.rs");
        std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
        std::fs::write(&kept, "fn kept() {}\n").unwrap();
        std::fs::write(&outside, "fn outside() {}\n").unwrap();
        let kept = kept.to_string_lossy().to_string();
        let missing = temp
            .path()
            .join("src")
            .join("gone.rs")
            .to_string_lossy()
            .to_string();
        let outside = outside.to_string_lossy().to_string();
        let existing = HashSet::from([kept.clone()]);

        let stale = stale_stored_paths(vec![kept, missing.clone(), outside.clone()], &existing);

        assert_eq!(stale, vec![missing, outside]);
    }

    #[tokio::test]
    async fn reconcile_deleted_paths_removes_stale_store_entries() {
        let service = Arc::new(CodeGraphService::open_in_memory().unwrap());
        let temp = tempfile::tempdir().unwrap();
        let kept = temp.path().join("src").join("kept.rs");
        let gone = temp.path().join("src").join("gone.rs");
        std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
        std::fs::write(&kept, "fn kept() {}\n").unwrap();
        let kept = kept.to_string_lossy().to_string();
        let gone = gone.to_string_lossy().to_string();

        service
            .index_file(&kept, "fn kept() {}\n", "rust")
            .await
            .unwrap();
        service
            .index_file(&gone, "fn gone() {}\n", "rust")
            .await
            .unwrap();
        let existing = HashSet::from([kept.clone()]);

        let removed = reconcile_deleted_paths(service.clone(), &existing)
            .await
            .unwrap();

        assert_eq!(removed, 1);
        let mut paths = service.all_paths().await.unwrap();
        paths.sort();
        assert_eq!(paths, vec![kept]);
    }
}
