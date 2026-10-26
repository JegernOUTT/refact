use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::buddy::autonomous_workflows::{
    autonomous_workflow_meta, BUDDY_PR_ISSUE_MATCHMAKER_WORKFLOW_ID,
};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};

pub struct BuddyPrIssueMatchmakerJob;

const COOLDOWN_SECONDS: u64 = 4 * 60 * 60;
const PRIORITY: u32 = 33;
const MIN_CHANGED_LINES: u32 = 10;
const TRUSTED_COMMAND_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PrIssueMatchmakerCache {
    checked_at: DateTime<Utc>,
    changed_lines: u32,
}

static PENDING_DIFF_CACHE: OnceLock<Mutex<HashMap<PathBuf, PrIssueMatchmakerCache>>> =
    OnceLock::new();

fn pending_diff_cache() -> &'static Mutex<HashMap<PathBuf, PrIssueMatchmakerCache>> {
    PENDING_DIFF_CACHE.get_or_init(Default::default)
}

fn cache_key(project_root: &Path) -> PathBuf {
    project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf())
}

fn store_pending_diff_cache(project_root: &Path, cache: PrIssueMatchmakerCache) {
    pending_diff_cache()
        .lock()
        .unwrap()
        .insert(cache_key(project_root), cache);
}

fn take_pending_diff_cache(project_root: &Path) -> Option<PrIssueMatchmakerCache> {
    pending_diff_cache()
        .lock()
        .unwrap()
        .remove(&cache_key(project_root))
}

fn parse_cache(raw: Option<&str>) -> Option<PrIssueMatchmakerCache> {
    serde_json::from_str(raw?).ok()
}

fn serialize_cache(cache: &PrIssueMatchmakerCache) -> String {
    serde_json::to_string(cache).unwrap_or_else(|_| "{}".to_string())
}

fn cache_is_fresh(cache: &PrIssueMatchmakerCache) -> bool {
    Utc::now()
        .signed_duration_since(cache.checked_at)
        .num_seconds()
        < COOLDOWN_SECONDS as i64
}

fn fresh_cache(ctx: &BuddyJobContext) -> Option<PrIssueMatchmakerCache> {
    parse_cache(ctx.job_state.last_result.as_deref()).filter(cache_is_fresh)
}

fn parse_shortstat_changed_lines(output: &str) -> u32 {
    output
        .split(',')
        .filter_map(|part| {
            let trimmed = part.trim();
            let number = trimmed.split_whitespace().next()?.parse::<u32>().ok()?;
            if trimmed.contains("insertion") || trimmed.contains("deletion") {
                Some(number)
            } else {
                None
            }
        })
        .sum()
}

async fn trusted_git_output(project_root: &Path, args: &[&str]) -> Option<String> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(project_root)
        .args(args)
        .env("PATH", TRUSTED_COMMAND_PATH)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    // Bound the git invocation so a hung repo/filesystem cannot stall the Tokio worker.
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

async fn git_diff_shortstat_changed_lines(project_root: &Path) -> u32 {
    trusted_git_output(project_root, &["diff", "--shortstat"])
        .await
        .map(|output| parse_shortstat_changed_lines(&output))
        .unwrap_or(0)
}

async fn checked_diff_cache(project_root: PathBuf) -> PrIssueMatchmakerCache {
    let changed_lines = git_diff_shortstat_changed_lines(&project_root).await;
    PrIssueMatchmakerCache {
        checked_at: Utc::now(),
        changed_lines,
    }
}

async fn diff_cache_for_should_run(ctx: &BuddyJobContext) -> PrIssueMatchmakerCache {
    if let Some(cache) = fresh_cache(ctx) {
        return cache;
    }
    let cache = checked_diff_cache(ctx.project_root.clone()).await;
    store_pending_diff_cache(&ctx.project_root, cache.clone());
    cache
}

async fn diff_cache_for_execute(ctx: &BuddyJobContext) -> PrIssueMatchmakerCache {
    if let Some(cache) = fresh_cache(ctx) {
        return cache;
    }
    if let Some(cache) = take_pending_diff_cache(&ctx.project_root) {
        return cache;
    }
    checked_diff_cache(ctx.project_root.clone()).await
}

fn cache_result(cache: &PrIssueMatchmakerCache) -> BuddyJobResult {
    BuddyJobResult {
        last_result: Some(serialize_cache(cache)),
        ..Default::default()
    }
}

async fn diff_evidence(ctx: &BuddyJobContext, changed_lines: u32) -> String {
    let diff_stat = trusted_git_output(&ctx.project_root, &["diff", "--stat"])
        .await
        .unwrap_or_else(|| "none".to_string());
    let changed_files = trusted_git_output(&ctx.project_root, &["diff", "--name-only"])
        .await
        .unwrap_or_else(|| "none".to_string());
    format!(
        "project_root={}\nchanged_lines={}\nthreshold={}\n\nChanged files:\n{}\n\nDiff stat:\n{}",
        ctx.project_root.to_string_lossy(),
        changed_lines,
        MIN_CHANGED_LINES,
        changed_files,
        diff_stat
    )
}

async fn build_pr_matchmaker_spec(
    ctx: &BuddyJobContext,
    changed_lines: u32,
) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(BUDDY_PR_ISSUE_MATCHMAKER_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    let evidence = diff_evidence(ctx, changed_lines).await;
    AutonomousBuddyChatSpec::new(
        meta.id,
        meta.title,
        "Look for likely issue or PR context for the current local diff and suggest one useful connection.",
        evidence,
    )
    .with_display(meta.icon, meta.badge, meta.priority)
    .with_project_root(project_root)
}

#[async_trait::async_trait]
impl BuddyJob for BuddyPrIssueMatchmakerJob {
    fn id(&self) -> &str {
        BUDDY_PR_ISSUE_MATCHMAKER_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        COOLDOWN_SECONDS
    }

    fn priority(&self) -> u32 {
        PRIORITY
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        let had_fresh_cache = fresh_cache(ctx).is_some();
        let cache = diff_cache_for_should_run(ctx).await;
        cache.changed_lines >= MIN_CHANGED_LINES || !had_fresh_cache
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let cache = diff_cache_for_execute(&ctx).await;
        if cache.changed_lines < MIN_CHANGED_LINES {
            return cache_result(&cache);
        }
        let mut result = execute_autonomous_spec(
            gcx,
            &ctx,
            build_pr_matchmaker_spec(&ctx, cache.changed_lines).await,
            self.cooldown_seconds(),
        )
        .await;
        result.last_result = cache_result(&cache).last_result;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
