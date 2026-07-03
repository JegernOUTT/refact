use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, BUDDY_SKILL_AUTHOR_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{
    execute_autonomous_spec, same_signal, AutonomousBuddyChatSpec,
};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::ext::competitor_import::markdown::sanitize_skill_id;
use crate::app_state::AppState;

pub struct BuddySkillAuthorJob;

const COOLDOWN_SECONDS: u64 = 24 * 60 * 60;
const PRIORITY: u32 = 6;
const MAX_TRAJECTORY_BYTES: u64 = 4 * 1024;
const MIN_CLUSTER_SIZE: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SkillCandidate {
    key: String,
    skill_id: String,
    count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SkillScanResult {
    candidate: Option<SkillCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SkillScanCache {
    scanned_at: i64,
    signal_hash: String,
    scan: SkillScanResult,
}

fn serialize_scan(signal_hash: &str, scan: &SkillScanResult) -> String {
    serde_json::to_string(&SkillScanCache {
        scanned_at: Utc::now().timestamp(),
        signal_hash: signal_hash.to_string(),
        scan: scan.clone(),
    })
    .unwrap_or_default()
}

fn cached_scan(ctx: &BuddyJobContext) -> Option<SkillScanCache> {
    serde_json::from_str::<SkillScanCache>(ctx.job_state.last_result.as_deref()?).ok()
}

fn cache_is_fresh(scanned_at: i64) -> bool {
    Utc::now().timestamp().saturating_sub(scanned_at) < COOLDOWN_SECONDS as i64
}

fn scan_cache_result(ctx: &BuddyJobContext) -> Option<(SkillScanResult, String)> {
    cached_scan(ctx)
        .filter(|cache| cache_is_fresh(cache.scanned_at))
        .map(|cache| (cache.scan, cache.signal_hash))
}

fn first_user_message_prefix(content: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let messages = value.get("messages")?.as_array()?;
    for message in messages {
        if message.get("role").and_then(|role| role.as_str()) != Some("user") {
            continue;
        }
        let text = message_text(message.get("content")?)?;
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            return None;
        }
        return Some(crate::llm::safe_truncate(&normalized, 100).to_string());
    }
    None
}

fn message_text(content: &serde_json::Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    if let Some(items) = content.as_array() {
        let text = items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(|value| value.as_str())
                    .or_else(|| item.get("m_content").and_then(|value| value.as_str()))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !text.trim().is_empty() {
            return Some(text);
        }
    }
    None
}

fn read_trajectory_prefix(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut content = String::new();
    let mut reader = file.take(MAX_TRAJECTORY_BYTES);
    reader.read_to_string(&mut content).ok()?;
    first_user_message_prefix(&content)
}

fn collect_trajectory_json_files(dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return paths;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();
    paths
}

fn skill_exists(project_root: &Path, skill_id: &str) -> bool {
    project_root.join(".refact/skills").join(skill_id).exists()
        || project_root
            .join(".refact/skills")
            .join(format!("{skill_id}.md"))
            .exists()
}

fn scan_skill_author(project_root: &Path) -> SkillScanResult {
    let trajectories_dir = project_root.join(".refact/trajectories");
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for path in collect_trajectory_json_files(&trajectories_dir) {
        if let Some(prefix) = read_trajectory_prefix(&path) {
            *counts.entry(prefix).or_default() += 1;
        }
    }
    let candidate = counts
        .into_iter()
        .filter(|(_, count)| *count >= MIN_CLUSTER_SIZE)
        .find_map(|(key, count)| {
            let skill_id = sanitize_skill_id(&key);
            if skill_id.is_empty() || skill_exists(project_root, &skill_id) {
                return None;
            }
            Some(SkillCandidate {
                key,
                skill_id,
                count,
            })
        });
    SkillScanResult { candidate }
}

fn build_skill_author_spec(
    ctx: &BuddyJobContext,
    scan: &SkillScanResult,
) -> Option<AutonomousBuddyChatSpec> {
    let candidate = scan.candidate.as_ref()?;
    let meta = autonomous_workflow_meta(BUDDY_SKILL_AUTHOR_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    let evidence = format!(
        "project_root={}\ncluster_key={}\ncluster_count={}\nproposed_skill_id={}",
        project_root, candidate.key, candidate.count, candidate.skill_id
    );
    Some(
        AutonomousBuddyChatSpec::new(
            meta.id,
            meta.title,
            "Draft one reusable skill for a repeated trajectory pattern if the cluster is specific enough.",
            evidence,
        )
        .with_display(meta.icon, meta.badge, meta.priority)
        .with_project_root(project_root),
    )
}

async fn current_scan(ctx: &BuddyJobContext) -> SkillScanResult {
    if let Some((scan, _)) = scan_cache_result(ctx) {
        return scan;
    }
    let project_root = ctx.project_root.clone();
    tokio::task::spawn_blocking(move || scan_skill_author(&project_root))
        .await
        .unwrap_or(SkillScanResult { candidate: None })
}

#[async_trait::async_trait]
impl BuddyJob for BuddySkillAuthorJob {
    fn id(&self) -> &str {
        BUDDY_SKILL_AUTHOR_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        COOLDOWN_SECONDS
    }

    fn priority(&self) -> u32 {
        PRIORITY
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        let Some(cache) = cached_scan(ctx) else {
            return true;
        };
        if !cache_is_fresh(cache.scanned_at) {
            return true;
        }
        let (scan, cached_hash) = (cache.scan, cache.signal_hash);
        let Some(spec) = build_skill_author_spec(ctx, &scan) else {
            return false;
        };
        cached_hash == spec.signal_hash && !same_signal(ctx, &spec.signal_hash)
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let scan = current_scan(&ctx).await;
        let Some(spec) = build_skill_author_spec(&ctx, &scan) else {
            return BuddyJobResult::default();
        };
        if same_signal(&ctx, &spec.signal_hash) {
            return BuddyJobResult::default();
        }
        let mut result =
            execute_autonomous_spec(gcx, &ctx, spec.clone(), self.cooldown_seconds()).await;
        if result.last_result.is_none() {
            result.last_result = Some(serialize_scan(&spec.signal_hash, &scan));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::settings::BuddySettings;
    use crate::buddy::types::{BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse};
    use std::path::Path;

    fn test_context(project_root: &Path, last_result: Option<String>) -> BuddyJobContext {
        BuddyJobContext {
            identity_name: "Pixel".to_string(),
            personality: Default::default(),
            onboarding: BuddyOnboarding::default(),
            recent_diagnostics: vec![],
            project_root: project_root.to_path_buf(),
            job_state: BuddyJobState {
                last_result,
                ..Default::default()
            },
            workflow_summaries: vec![],
            total_workflow_runs: 0,
            suggestion_state: vec![],
            pet: BuddyPetState::default(),
            active_quest: None,
            settings: BuddySettings::default(),
            pulse: BuddyPulse::default(),
            facts: vec![],
            recent_activities: vec![],
        }
    }

    fn write_trajectory(path: &Path, text: &str) {
        std::fs::write(
            path,
            serde_json::json!({
                "messages": [{"role": "user", "content": text}]
            })
            .to_string(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn buddy_skill_author_finds_5_plus_similar_trajectories() {
        let dir = tempfile::tempdir().unwrap();
        let trajectories = dir.path().join(".refact/trajectories");
        std::fs::create_dir_all(&trajectories).unwrap();
        for idx in 0..5 {
            write_trajectory(
                &trajectories.join(format!("chat_{idx}.json")),
                "Please review the auth flow for missing tests and edge cases in the service layer.",
            );
        }
        write_trajectory(
            &trajectories.join("other.json"),
            "A different one-off request",
        );
        let scan = scan_skill_author(dir.path());
        let spec = build_skill_author_spec(&test_context(dir.path(), None), &scan).unwrap();
        let ctx = test_context(dir.path(), Some(serialize_scan(&spec.signal_hash, &scan)));
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        assert!(BuddySkillAuthorJob.should_run(gcx, &ctx).await);
        let candidate = scan.candidate.unwrap();
        assert_eq!(candidate.count, 5);
        assert_eq!(
            candidate.skill_id,
            "please-review-the-auth-flow-for-missing-tests-and-edge-cases-in-the-service-layer"
        );
    }
}
