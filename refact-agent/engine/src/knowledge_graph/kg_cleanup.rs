use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use tokio::fs;
use tracing::{info, warn};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::global_context::GlobalContext;
use crate::memories::delete_document_from_disk;
use super::kg_builder::build_knowledge_graph;

const CLEANUP_INTERVAL_SECS: u64 = 7 * 24 * 60 * 60;
const TRAJECTORY_MAX_AGE_DAYS: i64 = 90;
const STALE_DOC_AGE_DAYS: i64 = 180;

#[derive(Debug, Serialize, Deserialize, Default)]
struct CleanupState {
    last_run: i64,
}

async fn load_cleanup_state(gcx: Arc<ARwLock<GlobalContext>>) -> CleanupState {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let state_file = cache_dir.join("knowledge_cleanup_state.json");

    match fs::read_to_string(&state_file).await {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => CleanupState::default(),
    }
}

async fn save_cleanup_state(gcx: Arc<ARwLock<GlobalContext>>, state: &CleanupState) {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let state_file = cache_dir.join("knowledge_cleanup_state.json");

    if let Ok(content) = serde_json::to_string(state) {
        let _ = fs::write(&state_file, content).await;
    }
}

pub async fn knowledge_cleanup_background_task(gcx: Arc<ARwLock<GlobalContext>>) {
    loop {
        let state = load_cleanup_state(gcx.clone()).await;
        let now = Utc::now().timestamp();

        if now - state.last_run >= CLEANUP_INTERVAL_SECS as i64 {
            info!("knowledge_cleanup: running weekly cleanup");

            match run_cleanup(gcx.clone()).await {
                Ok(report) => {
                    info!("knowledge_cleanup: completed - deleted {} trajectories, {} inactive docs, {} stale docs, {} orphan warnings",
                        report.deleted_trajectories,
                        report.deleted_inactive,
                        report.deleted_stale,
                        report.orphan_warnings,
                    );
                }
                Err(e) => {
                    warn!("knowledge_cleanup: failed - {}", e);
                }
            }

            let new_state = CleanupState { last_run: now };
            save_cleanup_state(gcx.clone(), &new_state).await;
        }

        let shutdown_flag = gcx.read().await.shutdown_flag.clone();
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(24 * 60 * 60)) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                }
            } => {
                tracing::info!("Knowledge cleanup: shutdown detected, stopping");
                return;
            }
        }
    }
}

#[derive(Debug, Default)]
struct CleanupReport {
    deleted_trajectories: usize,
    deleted_inactive: usize,
    deleted_stale: usize,
    orphan_warnings: usize,
}

async fn run_cleanup(gcx: Arc<ARwLock<GlobalContext>>) -> Result<CleanupReport, String> {
    let kg = build_knowledge_graph(gcx.clone()).await;
    let staleness = kg.check_staleness(STALE_DOC_AGE_DAYS, TRAJECTORY_MAX_AGE_DAYS);
    let mut report = CleanupReport::default();

    for path in staleness.stale_trajectories {
        match delete_document_from_disk(gcx.clone(), &path).await {
            Ok(_) => report.deleted_trajectories += 1,
            Err(e) => warn!(
                "Failed to delete stale trajectory {}: {}",
                path.display(),
                e
            ),
        }
    }

    for path in staleness.inactive_docs {
        match delete_document_from_disk(gcx.clone(), &path).await {
            Ok(_) => report.deleted_inactive += 1,
            Err(e) => warn!("Failed to delete inactive doc {}: {}", path.display(), e),
        }
    }

    let mut stale_docs = Vec::new();
    let mut seen_stale_docs = HashSet::new();
    for (path, age_days) in staleness.stale_by_age {
        if seen_stale_docs.insert(path.clone()) {
            stale_docs.push((path, format!("{} days old", age_days)));
        }
    }
    for path in staleness.past_review {
        if seen_stale_docs.insert(path.clone()) {
            stale_docs.push((path, "past review date".to_string()));
        }
    }

    for (path, reason) in stale_docs {
        match delete_document_from_disk(gcx.clone(), &path).await {
            Ok(_) => report.deleted_stale += 1,
            Err(e) => warn!(
                "Failed to delete stale doc {} ({}): {}",
                path.display(),
                reason,
                e
            ),
        }
    }

    report.orphan_warnings = staleness.orphan_file_refs.len();
    for (path, missing_files) in &staleness.orphan_file_refs {
        info!(
            "knowledge_cleanup: {} references missing files: {:?}",
            path.display(),
            missing_files
        );
    }

    Ok(report)
}
