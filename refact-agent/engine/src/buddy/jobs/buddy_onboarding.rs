use std::path::Path;
use std::time::SystemTime;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, BUDDY_ONBOARDING_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{
    execute_autonomous_spec, same_signal, AutonomousBuddyChatSpec,
};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::app_state::AppState;

pub struct BuddyOnboardingJob;

const COOLDOWN_SECONDS: u64 = 24 * 60 * 60;
const PRIORITY: u32 = 6;
const MAX_CANDIDATES: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct OnboardingCandidate {
    path: String,
    kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct OnboardingScanResult {
    candidates: Vec<OnboardingCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct OnboardingScanCache {
    scanned_at: i64,
    signal_hash: String,
    scan: OnboardingScanResult,
}

fn serialize_scan(signal_hash: &str, scan: &OnboardingScanResult) -> String {
    serde_json::to_string(&OnboardingScanCache {
        scanned_at: Utc::now().timestamp(),
        signal_hash: signal_hash.to_string(),
        scan: scan.clone(),
    })
    .unwrap_or_default()
}

fn cached_scan(ctx: &BuddyJobContext) -> Option<OnboardingScanCache> {
    serde_json::from_str::<OnboardingScanCache>(ctx.job_state.last_result.as_deref()?).ok()
}

fn cache_is_fresh(scanned_at: i64) -> bool {
    Utc::now().timestamp().saturating_sub(scanned_at) < COOLDOWN_SECONDS as i64
}

fn scan_cache_result(ctx: &BuddyJobContext) -> Option<(OnboardingScanResult, String)> {
    cached_scan(ctx)
        .filter(|cache| cache_is_fresh(cache.scanned_at))
        .map(|cache| (cache.scan, cache.signal_hash))
}

fn agents_modified(project_root: &Path) -> Option<SystemTime> {
    std::fs::metadata(project_root.join("AGENTS.md"))
        .ok()?
        .modified()
        .ok()
}

fn should_skip_top_level(name: &str) -> bool {
    matches!(name, ".git" | "node_modules" | "target")
}

fn scan_onboarding(project_root: &Path) -> OnboardingScanResult {
    let Some(agents_mtime) = agents_modified(project_root) else {
        return OnboardingScanResult { candidates: vec![] };
    };
    let Ok(entries) = std::fs::read_dir(project_root) else {
        return OnboardingScanResult { candidates: vec![] };
    };
    let mut candidates = Vec::new();
    let mut entries = entries.flatten().collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if should_skip_top_level(&name) || name == "AGENTS.md" {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified <= agents_mtime {
            continue;
        }
        candidates.push(OnboardingCandidate {
            path: name,
            kind: if metadata.is_dir() { "dir" } else { "file" }.to_string(),
        });
        if candidates.len() >= MAX_CANDIDATES {
            break;
        }
    }
    OnboardingScanResult { candidates }
}

fn render_evidence(scan: &OnboardingScanResult) -> String {
    let mut lines = vec![
        "Onboarding signal:".to_string(),
        format!("- newer_top_level_candidates: {}", scan.candidates.len()),
    ];
    for candidate in &scan.candidates {
        lines.push(format!("- {} ({})", candidate.path, candidate.kind));
    }
    lines.join("\n")
}

fn build_onboarding_spec(
    ctx: &BuddyJobContext,
    scan: &OnboardingScanResult,
) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(BUDDY_ONBOARDING_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    AutonomousBuddyChatSpec::new(
        meta.id,
        meta.title,
        "Review top-level project additions newer than AGENTS.md and update onboarding notes if needed.",
        format!("project_root={}\n{}", project_root, render_evidence(scan)),
    )
    .with_display(meta.icon, meta.badge, meta.priority)
    .with_project_root(project_root)
}

async fn current_scan(ctx: &BuddyJobContext) -> OnboardingScanResult {
    if let Some((scan, _)) = scan_cache_result(ctx) {
        return scan;
    }
    let project_root = ctx.project_root.clone();
    tokio::task::spawn_blocking(move || scan_onboarding(&project_root))
        .await
        .unwrap_or(OnboardingScanResult { candidates: vec![] })
}

#[async_trait::async_trait]
impl BuddyJob for BuddyOnboardingJob {
    fn id(&self) -> &str {
        BUDDY_ONBOARDING_WORKFLOW_ID
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
        if scan.candidates.is_empty() {
            return false;
        }
        let spec = build_onboarding_spec(ctx, &scan);
        cached_hash == spec.signal_hash && !same_signal(ctx, &spec.signal_hash)
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let scan = current_scan(&ctx).await;
        if scan.candidates.is_empty() {
            return BuddyJobResult::default();
        }
        let spec = build_onboarding_spec(&ctx, &scan);
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
    use std::path::Path;
}
