use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tracing::{error, info};

use refact_codegraph::{lang_from_path, CodeGraphService};

use crate::global_context::GlobalContext;

const CODEGRAPH_DB_FILE: &str = "codegraph.sqlite";
const DRAIN_BATCH: usize = 64;

pub async fn codegraph_db_path(gcx: Arc<GlobalContext>) -> PathBuf {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    if let Some(root) = project_dirs.first() {
        root.join(".refact")
            .join("codegraph")
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
            info!("codegraph: store ready at {db_path:?}");
        }
        Err(err) => {
            error!("codegraph: failed to open store at {db_path:?}: {err}");
        }
    }
}

pub async fn codegraph_background_task(gcx: Arc<GlobalContext>) {
    let service = match gcx.codegraph.lock().await.clone() {
        Some(service) => service,
        None => return,
    };

    crate::files_in_workspace::enqueue_all_files_from_workspace_folders(gcx.clone(), true, false)
        .await;

    loop {
        if gcx.shutdown_flag.load(Ordering::Relaxed) {
            break;
        }

        let batch = service.drain_batch(DRAIN_BATCH);
        if batch.is_empty() {
            if let Err(err) = service.connect_usages().await {
                error!("codegraph: connect_usages failed: {err}");
            }
            if !service.is_initial_index_done() {
                service.mark_initial_index_done();
                info!("codegraph: initial index complete");
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
                    }
                }
                Err(_) => {
                    let _ = service.remove_path(&path).await;
                }
            }
        }
    }
}
