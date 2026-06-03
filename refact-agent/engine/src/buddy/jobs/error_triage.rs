use crate::app_state::AppState;
use std::collections::HashMap;
use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::BuddySuggestion;

pub struct ErrorTriageJob;

#[async_trait::async_trait]
impl BuddyJob for ErrorTriageJob {
    fn id(&self) -> &str {
        "error_triage"
    }
    fn cooldown_seconds(&self) -> u64 {
        300
    }
    fn priority(&self) -> u32 {
        2
    }
    fn produces_suggestion(&self) -> bool {
        true
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        ctx.recent_diagnostics.len() >= 3
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for d in &ctx.recent_diagnostics {
            *counts.entry(d.error_type.clone()).or_default() += 1;
        }
        let Some((error_type, count)) = counts.iter().max_by_key(|(_, c)| *c) else {
            return BuddyJobResult::default();
        };
        if *count < 3 {
            return BuddyJobResult::default();
        }
        let _ = gcx;
        BuddyJobResult {
            suggestion: Some(BuddySuggestion {
                id: format!("triage-{}", chrono::Utc::now().timestamp()),
                suggestion_type: "error_pattern".to_string(),
                title: format!(
                    "Gremlin error pile: repeated {} hiccups ({}x)",
                    error_type, count
                ),
                description: format!(
                    "I found {} {} crumbs doing laps in the logs. Want me to put on the tiny detective hat?",
                    count, error_type
                ),
                created_at: chrono::Utc::now().to_rfc3339(),
                dismissed: false,
                controls: vec![],
                quest: None,
            }),
            ..Default::default()
        }
    }
}
