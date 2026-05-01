use std::sync::Arc;
use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::{BuddyActivity, BuddySpeechItem, BuddySuggestion};

const WORKFLOW_MILESTONES: &[u64] = &[10, 50, 100, 500];
const RECENT_ERROR_WINDOW_SECS: i64 = 3600;

pub struct StatsWatcherJob;

#[async_trait::async_trait]
impl BuddyJob for StatsWatcherJob {
    fn id(&self) -> &str {
        "stats_watcher"
    }
    fn cooldown_seconds(&self) -> u64 {
        1800
    }
    fn priority(&self) -> u32 {
        5
    }
    fn produces_suggestion(&self) -> bool {
        false
    }

    async fn should_run(
        &self,
        _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        _ctx: &BuddyJobContext,
    ) -> bool {
        true
    }

    async fn execute(
        &self,
        _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let runs = ctx.total_workflow_runs;

        if runs == 0 {
            return BuddyJobResult::default();
        }

        let cutoff = chrono::Utc::now().timestamp() - RECENT_ERROR_WINDOW_SECS;
        let recent_error_count = ctx
            .recent_diagnostics
            .iter()
            .filter(|d| {
                chrono::DateTime::parse_from_rfc3339(&d.collected_at)
                    .map(|t| t.timestamp() >= cutoff)
                    .unwrap_or(false)
            })
            .count();

        if recent_error_count >= 5 {
            return BuddyJobResult {
                suggestion: Some(BuddySuggestion {
                    id: format!("stats-errors-{}", chrono::Utc::now().timestamp()),
                    suggestion_type: "error_pattern".to_string(),
                    title: format!("{} errors in the last hour", recent_error_count),
                    description: "Several errors have been logged recently. Want me to create a GitHub/GitLab issue to track them?".to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    dismissed: false,
                    controls: vec![],
                    quest: None,
                }),
                last_result: Some(runs.to_string()),
                ..Default::default()
            };
        }

        let prev_runs: u64 = ctx
            .job_state
            .last_result
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        for &m in WORKFLOW_MILESTONES {
            if prev_runs < m && runs >= m {
                let speech = BuddySpeechItem {
                    id: format!("stats-milestone-{}", m),
                    text: format!("We've completed {} tasks together!", m),
                    mood: "happy".to_string(),
                    scope: "global".to_string(),
                    persistent: false,
                    ttl_seconds: 12,
                    dedupe_key: Some(format!("milestone_{}", m)),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    controls: vec![],
                    chat_id: None,
                };
                let activity = BuddyActivity {
                    icon: "🎉".to_string(),
                    title: format!("Milestone: {} tasks completed!", m),
                    description: format!("{} workflows have finished successfully.", m),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    activity_type: "milestone".to_string(),
                };
                return BuddyJobResult {
                    speech: Some(speech),
                    activity: Some(activity),
                    last_result: Some(runs.to_string()),
                    ..Default::default()
                };
            }
        }

        BuddyJobResult {
            last_result: Some(runs.to_string()),
            ..Default::default()
        }
    }
}
