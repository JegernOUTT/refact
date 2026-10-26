use super::super::actor::make_runtime_event;
use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::{BuddyActivity, BuddyRuntimeEvent, BuddySuggestion};
use crate::app_state::AppState;
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

    async fn should_run(&self, gcx: AppState, ctx: &BuddyJobContext) -> bool {
        let caps = gcx.model.caps.read().await.caps.clone();
        health_watcher_has_visible_output(caps.as_deref(), ctx)
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let caps_result = gcx
            .model
            .caps
            .read()
            .await
            .caps
            .as_ref()
            .map(|c| (!c.completion_models.is_empty(), !c.chat_models.is_empty()));

        let Some((has_completion, has_chat)) = caps_result else {
            return BuddyJobResult::default();
        };

        let was_healthy = ctx.job_state.last_result.as_deref() == Some("healthy");

        if !has_completion && !has_chat {
            let suggestion = if !was_healthy {
                None
            } else {
                Some(BuddySuggestion {
                    id: "health-no-models".to_string(),
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
                failure_category: None,
                failure_summary: None,
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

    use crate::buddy::types::{
        BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse, BuddySuggestion,
    };
}
