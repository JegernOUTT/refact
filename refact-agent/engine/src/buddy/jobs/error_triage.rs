use std::sync::Arc;
use std::collections::HashMap;
use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::BuddySuggestion;

pub struct ErrorTriageJob;

#[async_trait::async_trait]
impl BuddyJob for ErrorTriageJob {
    fn id(&self) -> &str { "error_triage" }
    fn cooldown_seconds(&self) -> u64 { 300 }
    fn priority(&self) -> u32 { 2 }

    async fn should_run(&self, _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        ctx.recent_diagnostics.len() >= 3
    }

    async fn execute(&self, _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>, ctx: BuddyJobContext) -> BuddyJobResult {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for d in &ctx.recent_diagnostics {
            *counts.entry(d.error_type.clone()).or_default() += 1;
        }
        if let Some((error_type, count)) = counts.iter().max_by_key(|(_, c)| *c) {
            if *count >= 3 {
                return BuddyJobResult {
                    suggestion: Some(BuddySuggestion {
                        id: format!("triage-{}", chrono::Utc::now().timestamp()),
                        suggestion_type: "error_pattern".to_string(),
                        title: format!("Repeated {} errors detected ({}x)", error_type, count),
                        description: format!("I'm seeing {} repeated {} errors. Want me to analyze the logs?", count, error_type),
                        created_at: chrono::Utc::now().to_rfc3339(),
                        dismissed: false,
                    }),
                    ..Default::default()
                };
            }
        }
        BuddyJobResult::default()
    }
}
