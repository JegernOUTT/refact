use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tracing::{error, info};

use refact_codegraph::{lang_from_path, CodeGraphService, Counts};

use crate::global_context::GlobalContext;

const CODEGRAPH_DB_FILE: &str = "codegraph.sqlite";
const DRAIN_BATCH: usize = 64;
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

        for path in batch {
            if gcx.shutdown_flag.load(Ordering::Relaxed) {
                break;
            }
            match crate::files_in_workspace::get_file_text_from_memory_or_disk(
                gcx.clone(),
                &PathBuf::from(&path),
            )
            .await
            {
                Ok(text) => {
                    let lang = lang_from_path(&path);
                    if let Err(err) = service.index_file(&path, &text, lang).await {
                        error!("codegraph: index {path} failed: {err}");
                        *gcx.codegraph_error.lock().unwrap() = err;
                    }
                }
                Err(_) => {
                    if let Err(err) = service.remove_path(&path).await {
                        error!("codegraph: remove {path} failed: {err}");
                        *gcx.codegraph_error.lock().unwrap() = err;
                    }
                }
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
}
