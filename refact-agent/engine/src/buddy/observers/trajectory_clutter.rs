use chrono::{DateTime, Utc};

use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::app_state::AppState;

pub struct TrajectoryClutterObserver;
pub(crate) const MAX_TRAJECTORY_SCAN_FILES: usize = 500;
const MAX_TRAJECTORY_FILE_BYTES: u64 = 256 * 1024;

fn path_hash(p: &std::path::Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    p.hash(&mut h);
    format!("{:x}", h.finish())
}

async fn collect_trajectory_candidates(
    dir: &std::path::Path,
    candidates: &mut Vec<(std::time::SystemTime, std::path::PathBuf)>,
    total: &mut u32,
    subdirs: &mut Vec<std::path::PathBuf>,
    owner_stem: Option<&str>,
) {
    let mut rd = match tokio::fs::read_dir(dir).await {
        Ok(r) => r,
        Err(_) => return,
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        let path = entry.path();
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if owner_stem.is_none() {
                subdirs.push(path);
            }
            continue;
        }
        if !path.extension().map_or(false, |e| e == "json") {
            continue;
        }
        if path.file_name().map_or(false, |name| name == "index.json") {
            continue;
        }
        if let Some(owner_stem) = owner_stem {
            let is_folder_owner = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem == owner_stem);
            if !is_folder_owner {
                continue;
            }
        }
        let Ok(meta) = tokio::fs::symlink_metadata(&path).await else {
            continue;
        };
        *total += 1;
        if !meta.is_file() || meta.len() > MAX_TRAJECTORY_FILE_BYTES {
            continue;
        }
        let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        candidates.push((modified, path));
    }
}

pub async fn scan_trajectories_dir(dir: &std::path::Path) -> (u32, u32, u32) {
    let mut total: u32 = 0;
    let mut untitled: u32 = 0;
    let mut oldest_age_days: u32 = 0;
    let now = Utc::now();

    let mut candidates: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    let mut chat_folders: Vec<std::path::PathBuf> = Vec::new();
    collect_trajectory_candidates(dir, &mut candidates, &mut total, &mut chat_folders, None).await;
    for folder in chat_folders {
        let Some(owner_stem) = folder.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let owner_stem = owner_stem.to_string();
        collect_trajectory_candidates(
            &folder,
            &mut candidates,
            &mut total,
            &mut Vec::new(),
            Some(&owner_stem),
        )
        .await;
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    for (_, path) in candidates.into_iter().take(MAX_TRAJECTORY_SCAN_FILES) {
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                let title = v
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if title.is_empty() {
                    untitled += 1;
                }
                if let Some(created) = v
                    .get("created_at")
                    .and_then(|t| t.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                {
                    let age = now
                        .signed_duration_since(created.with_timezone(&Utc))
                        .num_days()
                        .max(0) as u32;
                    if age > oldest_age_days {
                        oldest_age_days = age;
                    }
                }
            } else {
                untitled += 1;
            }
        }
    }

    (total, untitled, oldest_age_days)
}

pub fn detect_trajectory_clutter_facts(
    project_root_hash: &str,
    total: u32,
    untitled: u32,
    oldest_age_days: u32,
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    if total <= 50 && untitled <= 15 {
        return vec![];
    }
    tracing::debug!("trajectory_clutter: total={} untitled={}", total, untitled);
    vec![BuddyFact {
        kind: BuddyFactKind::TrajectoryClutter,
        key: format!("trajectory:clutter:{}", project_root_hash),
        source: "trajectory_clutter",
        payload: serde_json::json!({
            "count": total,
            "untitled_count": untitled,
            "oldest_age_days": oldest_age_days,
        }),
        seen_at: now,
        confidence: 0.9,
    }]
}

#[async_trait::async_trait]
impl BuddyObserver for TrajectoryClutterObserver {
    fn id(&self) -> &'static str {
        "trajectory_clutter"
    }

    fn cadence_seconds(&self) -> u64 {
        300
    }

    fn requires_setting(&self, settings: &BuddySettings) -> bool {
        settings.observers.trajectory_clutter
    }

    async fn observe(&self, gcx: AppState, ctx: &ObserverContext) -> Vec<BuddyFact> {
        let traj_dir = ctx.project_root.join(".refact").join("trajectories");
        if !traj_dir.exists() {
            return vec![];
        }
        let hash = path_hash(&ctx.project_root);
        let (total, untitled, oldest) = scan_trajectories_dir(&traj_dir).await;
        let _ = gcx;
        detect_trajectory_clutter_facts(&hash, total, untitled, oldest, ctx.now)
    }
}
