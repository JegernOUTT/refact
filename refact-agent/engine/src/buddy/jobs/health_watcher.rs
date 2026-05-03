use std::sync::Arc;

use super::super::actor::make_runtime_event;
use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::{BuddyActivity, BuddyRuntimeEvent, BuddySuggestion};
use crate::caps::CodeAssistantCaps;

pub struct HealthWatcherJob;

#[async_trait::async_trait]
impl BuddyJob for HealthWatcherJob {
    fn id(&self) -> &str {
        "health_watcher"
    }
    fn cooldown_seconds(&self) -> u64 {
        900
    }
    fn priority(&self) -> u32 {
        4
    }
    fn produces_suggestion(&self) -> bool {
        true
    }
    fn runs_when_suggestions_blocked(&self) -> bool {
        true
    }

    async fn should_run(
        &self,
        gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        ctx: &BuddyJobContext,
    ) -> bool {
        let caps = {
            let gcx_locked = gcx.read().await;
            gcx_locked.caps.clone()
        };
        health_watcher_has_visible_output(caps.as_deref(), ctx)
    }

    async fn execute(
        &self,
        gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let caps_result = {
            let gcx_locked = gcx.read().await;
            gcx_locked
                .caps
                .as_ref()
                .map(|c| (!c.completion_models.is_empty(), !c.chat_models.is_empty()))
        };

        let Some((has_completion, has_chat)) = caps_result else {
            return BuddyJobResult::default();
        };

        let was_healthy = ctx.job_state.last_result.as_deref() == Some("healthy");

        if !has_completion && !has_chat {
            let suggestion = if !was_healthy {
                None
            } else {
                Some(BuddySuggestion {
                    id: format!("health-no-models-{}", chrono::Utc::now().timestamp()),
                    suggestion_type: "health".to_string(),
                    title: "No AI models configured".to_string(),
                    description: "I couldn't find any completion or chat models. Head to Provider Settings to add one.".to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    dismissed: false,
                    controls: vec![],
                    quest: None,
                })
            };
            let event: BuddyRuntimeEvent = make_runtime_event(
                "health",
                "No AI models configured",
                "health_watcher",
                "health_models",
                "failed",
                Some("high"),
            );
            return BuddyJobResult {
                suggestion,
                runtime_event: Some(event),
                last_result: Some("unhealthy".to_string()),
                ..Default::default()
            };
        }

        let parts: Vec<&str> = [
            if has_completion {
                Some("completion")
            } else {
                None
            },
            if has_chat { Some("chat") } else { None },
        ]
        .into_iter()
        .flatten()
        .collect();
        let title = format!("Models ready: {}", parts.join(", "));

        let activity = if !was_healthy {
            Some(BuddyActivity {
                icon: "💚".to_string(),
                title: title.clone(),
                description: format!("Health check passed — {}", title),
                timestamp: chrono::Utc::now().to_rfc3339(),
                activity_type: "health".to_string(),
                chat_id: None,
            })
        } else {
            None
        };

        let event: BuddyRuntimeEvent = make_runtime_event(
            "health",
            &title,
            "health_watcher",
            "health_models",
            "completed",
            None,
        );
        BuddyJobResult {
            activity,
            runtime_event: Some(event),
            last_result: Some("healthy".to_string()),
            ..Default::default()
        }
    }
}

fn health_watcher_has_visible_output(
    caps: Option<&CodeAssistantCaps>,
    ctx: &BuddyJobContext,
) -> bool {
    let Some(caps) = caps else {
        return false;
    };
    let has_completion = !caps.completion_models.is_empty();
    let has_chat = !caps.chat_models.is_empty();
    let was_healthy = ctx.job_state.last_result.as_deref() == Some("healthy");
    !has_completion && !has_chat || !was_healthy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::scheduler::result_after_suggestion_policy;
    use crate::buddy::settings::BuddySettings;
    use crate::buddy::types::{
        BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse, BuddySuggestion,
    };
    use crate::caps::ChatModelRecord;

    fn test_context(last_result: Option<&str>) -> BuddyJobContext {
        BuddyJobContext {
            identity_name: "Pixel".to_string(),
            onboarding: BuddyOnboarding::default(),
            recent_diagnostics: vec![],
            project_root: std::path::PathBuf::from("/tmp/project"),
            job_state: BuddyJobState {
                last_result: last_result.map(ToString::to_string),
                ..Default::default()
            },
            total_workflow_runs: 0,
            suggestion_state: vec![],
            pet: BuddyPetState::default(),
            active_quest: None,
            settings: BuddySettings::default(),
            pulse: BuddyPulse::default(),
            facts: vec![],
        }
    }

    fn active_suggestion(idx: usize) -> BuddySuggestion {
        BuddySuggestion {
            id: format!("suggestion-{idx}"),
            suggestion_type: "test".to_string(),
            title: format!("Suggestion {idx}"),
            description: "Test".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            dismissed: false,
            controls: vec![],
            quest: None,
        }
    }

    #[test]
    fn health_watcher_declares_suggestion_output() {
        let job = HealthWatcherJob;

        assert!(job.produces_suggestion());
    }

    #[tokio::test]
    async fn health_watcher_first_unhealthy_is_runtime_only_without_suggestion() {
        let job = HealthWatcherJob;
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.write().await.caps = Some(Arc::new(CodeAssistantCaps::default()));

        let result = job.execute(gcx, test_context(None)).await;

        assert!(result.suggestion.is_none());
        assert!(result.runtime_event.is_some());
        assert_eq!(result.last_result.as_deref(), Some("unhealthy"));
    }

    #[tokio::test]
    async fn health_watcher_recovered_state_produces_setup_suggestion_when_models_disappear() {
        let job = HealthWatcherJob;
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.write().await.caps = Some(Arc::new(CodeAssistantCaps::default()));

        let result = job.execute(gcx, test_context(Some("healthy"))).await;

        assert!(result.suggestion.is_some());
        assert!(result.runtime_event.is_some());
        assert_eq!(result.last_result.as_deref(), Some("unhealthy"));
    }

    #[tokio::test]
    async fn health_watcher_suppresses_suggestion_when_proactive_is_disabled_but_keeps_event() {
        let job = HealthWatcherJob;
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.write().await.caps = Some(Arc::new(CodeAssistantCaps::default()));
        let mut ctx = test_context(Some("healthy"));
        ctx.settings.proactive_enabled = false;

        let result = job.execute(gcx, ctx.clone()).await;
        assert!(result.suggestion.is_some());
        let result = result_after_suggestion_policy(result, &ctx.settings, &ctx.suggestion_state);

        assert!(result.suggestion.is_none());
        assert!(result.runtime_event.is_some());
        assert_eq!(result.last_result.as_deref(), Some("unhealthy"));
    }

    #[tokio::test]
    async fn health_watcher_suppresses_suggestion_when_unread_cap_is_full_but_keeps_event() {
        let job = HealthWatcherJob;
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.write().await.caps = Some(Arc::new(CodeAssistantCaps::default()));
        let mut ctx = test_context(Some("healthy"));
        ctx.suggestion_state = (0..crate::buddy::scheduler::MAX_UNREAD_SUGGESTIONS)
            .map(active_suggestion)
            .collect();

        let result = job.execute(gcx, ctx.clone()).await;
        assert!(result.suggestion.is_some());
        let result = result_after_suggestion_policy(result, &ctx.settings, &ctx.suggestion_state);

        assert!(result.suggestion.is_none());
        assert!(result.runtime_event.is_some());
        assert_eq!(result.last_result.as_deref(), Some("unhealthy"));
    }

    #[tokio::test]
    async fn health_watcher_should_run_only_for_visible_health_policy() {
        let job = HealthWatcherJob;
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut healthy_caps = CodeAssistantCaps::default();
        healthy_caps.chat_models.insert(
            "provider/model".to_string(),
            Arc::new(ChatModelRecord::default()),
        );

        gcx.write().await.caps = Some(Arc::new(healthy_caps));
        assert!(job.should_run(gcx.clone(), &test_context(None)).await);
        assert!(
            !job.should_run(gcx.clone(), &test_context(Some("healthy")))
                .await
        );

        gcx.write().await.caps = Some(Arc::new(CodeAssistantCaps::default()));
        assert!(job.should_run(gcx, &test_context(Some("healthy"))).await);
    }
}
