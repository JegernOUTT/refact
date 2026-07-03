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
        let signature = triage_signature(error_type, *count);
        if ctx.job_state.last_result.as_deref() == Some(signature.as_str()) {
            return BuddyJobResult::default();
        }
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
            last_result: Some(signature),
            ..Default::default()
        }
    }
}

pub(crate) fn triage_signature(error_type: &str, count: usize) -> String {
    let bucket = if count == 0 {
        0
    } else {
        1usize << (usize::BITS - 1 - count.leading_zeros())
    };
    format!("triage:{}:{}", error_type, bucket)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::diagnostics::{DiagnosticContext, DiagnosticSeverity};
    use crate::buddy::types::BuddyJobState;

    fn diagnostics(error_type: &str, count: usize) -> Vec<DiagnosticContext> {
        (0..count)
            .map(|idx| DiagnosticContext {
                error_type: error_type.to_string(),
                error_message: format!("boom {idx}"),
                source_file: None,
                tool_name: Some("chat".to_string()),
                chat_id: None,
                model_id: None,
                collected_at: chrono::Utc::now().to_rfc3339(),
                severity: DiagnosticSeverity::High,
            })
            .collect()
    }

    fn context_with(
        recent_diagnostics: Vec<DiagnosticContext>,
        last_result: Option<String>,
    ) -> BuddyJobContext {
        BuddyJobContext {
            identity_name: "Pixel".to_string(),
            personality: Default::default(),
            onboarding: Default::default(),
            recent_diagnostics,
            project_root: std::path::PathBuf::new(),
            job_state: BuddyJobState {
                last_result,
                ..Default::default()
            },
            workflow_summaries: vec![],
            total_workflow_runs: 0,
            suggestion_state: vec![],
            pet: Default::default(),
            active_quest: None,
            settings: Default::default(),
            pulse: Default::default(),
            facts: vec![],
            recent_activities: vec![],
        }
    }

    #[test]
    fn triage_signature_buckets_counts() {
        assert_eq!(triage_signature("llm_error", 3), "triage:llm_error:2");
        assert_eq!(triage_signature("llm_error", 5), "triage:llm_error:4");
        assert_eq!(triage_signature("llm_error", 7), "triage:llm_error:4");
        assert_eq!(triage_signature("llm_error", 9), "triage:llm_error:8");
    }

    #[tokio::test]
    async fn repeated_cluster_signature_is_suppressed() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let job = ErrorTriageJob;

        let first = job
            .execute(app.clone(), context_with(diagnostics("llm_error", 5), None))
            .await;
        assert!(first.suggestion.is_some());
        let signature = first.last_result.clone().unwrap();

        let suppressed = job
            .execute(
                app.clone(),
                context_with(diagnostics("llm_error", 6), Some(signature.clone())),
            )
            .await;
        assert!(suppressed.suggestion.is_none());
        assert!(suppressed.last_result.is_none());

        let escalated = job
            .execute(
                app,
                context_with(diagnostics("llm_error", 16), Some(signature)),
            )
            .await;
        assert!(escalated.suggestion.is_some());
        assert_eq!(
            escalated.last_result.as_deref(),
            Some("triage:llm_error:16")
        );
    }
}
