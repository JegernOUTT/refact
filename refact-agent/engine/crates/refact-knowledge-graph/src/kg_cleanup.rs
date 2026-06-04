use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::fs;
use tracing::{info, warn};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::kg_structs::KnowledgeGraph;

const CLEANUP_INTERVAL_SECS: u64 = 7 * 24 * 60 * 60;
const TRAJECTORY_MAX_AGE_DAYS: i64 = 90;
const STALE_DOC_AGE_DAYS: i64 = 180;

pub type KgFileDeleter =
    Arc<dyn Fn(PathBuf) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;
pub type KgGraphBuilder =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = KnowledgeGraph> + Send>> + Send + Sync>;

#[derive(Debug, Serialize, Deserialize, Default)]
struct CleanupState {
    last_run: i64,
}

async fn load_cleanup_state(cache_dir: &PathBuf) -> CleanupState {
    let state_file = cache_dir.join("knowledge_cleanup_state.json");
    match fs::read_to_string(&state_file).await {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => CleanupState::default(),
    }
}

async fn save_cleanup_state(cache_dir: &PathBuf, state: &CleanupState) {
    let state_file = cache_dir.join("knowledge_cleanup_state.json");
    if let Ok(content) = serde_json::to_string(state) {
        let _ = fs::write(&state_file, content).await;
    }
}

pub async fn knowledge_cleanup_background_task(
    shutdown_flag: Arc<AtomicBool>,
    cache_dir: PathBuf,
    build_graph: KgGraphBuilder,
    delete_file: KgFileDeleter,
) {
    loop {
        let state = load_cleanup_state(&cache_dir).await;
        let now = Utc::now().timestamp();

        if now - state.last_run >= CLEANUP_INTERVAL_SECS as i64 {
            info!("knowledge_cleanup: running weekly cleanup");

            match run_cleanup(&build_graph, &delete_file).await {
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
            save_cleanup_state(&cache_dir, &new_state).await;
        }

        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(24 * 60 * 60)) => {}
            _ = async {
                while !shutdown_flag.load(Ordering::SeqCst) {
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

async fn run_cleanup(
    build_graph: &KgGraphBuilder,
    delete_file: &KgFileDeleter,
) -> Result<CleanupReport, String> {
    let kg = build_graph().await;
    let staleness = kg.check_staleness(STALE_DOC_AGE_DAYS, TRAJECTORY_MAX_AGE_DAYS);
    let mut report = CleanupReport::default();

    for path in staleness.stale_trajectories {
        match delete_file(path.clone()).await {
            Ok(_) => report.deleted_trajectories += 1,
            Err(e) => warn!(
                "Failed to delete stale trajectory {}: {}",
                path.display(),
                e
            ),
        }
    }

    for path in staleness.inactive_docs {
        match delete_file(path.clone()).await {
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
        match delete_file(path.clone()).await {
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
/// One-shot removal of every memory whose status is not active (i.e. archived or
/// deprecated). Unlike [`run_cleanup`], this does not touch stale-by-age docs,
/// trajectories, or past-review docs — it only purges non-active memories.
///
/// Intended to run on engine startup so deprecated/archived memories don't linger
/// until the next weekly cleanup pass. Returns the number of memories removed.
pub async fn remove_inactive_memories(
    build_graph: &KgGraphBuilder,
    delete_file: &KgFileDeleter,
) -> Result<usize, String> {
    let kg = build_graph().await;
    let staleness = kg.check_staleness(STALE_DOC_AGE_DAYS, TRAJECTORY_MAX_AGE_DAYS);
    let mut deleted = 0;
    for path in staleness.inactive_docs {
        match delete_file(path.clone()).await {
            Ok(_) => deleted += 1,
            Err(e) => warn!("Failed to delete inactive memory {}: {}", path.display(), e),
        }
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::super::kg_structs::{KnowledgeDoc, KnowledgeFrontmatter, KnowledgeGraph};
    use super::*;

    fn doc(path: &str, status: &str) -> KnowledgeDoc {
        KnowledgeDoc {
            path: PathBuf::from(path),
            frontmatter: KnowledgeFrontmatter {
                id: Some(path.to_string()),
                status: Some(status.to_string()),
                ..Default::default()
            },
            content: String::new(),
            entities: Vec::new(),
        }
    }

    #[tokio::test]
    async fn remove_inactive_memories_deletes_only_non_active() {
        let docs = vec![
            doc("/tmp/active.md", "active"),
            doc("/tmp/deprecated.md", "deprecated"),
            doc("/tmp/archived.md", "archived"),
        ];
        let build_graph: KgGraphBuilder = Arc::new(move || {
            let docs = docs.clone();
            Box::pin(async move {
                let mut graph = KnowledgeGraph::new();
                for d in docs {
                    graph.add_doc(d);
                }
                graph
            })
        });

        let deleted_paths = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
        let delete_file: KgFileDeleter = {
            let deleted_paths = deleted_paths.clone();
            Arc::new(move |path: PathBuf| {
                let deleted_paths = deleted_paths.clone();
                Box::pin(async move {
                    deleted_paths.lock().unwrap().push(path);
                    Ok(())
                })
            })
        };

        let deleted = remove_inactive_memories(&build_graph, &delete_file)
            .await
            .unwrap();

        assert_eq!(deleted, 2);
        let paths = deleted_paths.lock().unwrap();
        assert!(paths.contains(&PathBuf::from("/tmp/deprecated.md")));
        assert!(paths.contains(&PathBuf::from("/tmp/archived.md")));
        assert!(!paths.contains(&PathBuf::from("/tmp/active.md")));
    }
}
