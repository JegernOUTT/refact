use crate::app_state::AppState;

use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::{BuddyControl, BuddySpeechItem};
use crate::buddy::voice_service::{SpeechIntent, VoiceCtx, voice_service};

fn greeting_fallback_text(ctx: &BuddyJobContext) -> String {
    if !ctx.onboarding.greeted {
        format!("Hi! I'm {}! 🎉 I'll help you with errors, setup, and staying productive. Want a quick tour?", ctx.identity_name)
    } else {
        format!(
            "Welcome back! 👋 I'm {} — ready to help.",
            ctx.identity_name
        )
    }
}

fn greeting_controls(ctx: &BuddyJobContext) -> Vec<BuddyControl> {
    if !ctx.onboarding.greeted {
        vec![
            BuddyControl {
                id: "tour".to_string(),
                label: "Take a Tour".to_string(),
                action: "open_buddy".to_string(),
                action_param: None,
                style: "primary".to_string(),
            },
            BuddyControl {
                id: "skip".to_string(),
                label: "Skip".to_string(),
                action: "dismiss".to_string(),
                action_param: None,
                style: "secondary".to_string(),
            },
        ]
    } else {
        vec![]
    }
}

fn greeting_workflow_summary(
    ctx: &BuddyJobContext,
    pulse_one_liner: &str,
    fallback: &str,
) -> String {
    ctx.workflow_summaries
        .last()
        .map(|summary| {
            let status = summary
                .last_outcome
                .as_deref()
                .or(summary.failure_summary.as_deref())
                .unwrap_or("unknown");
            format!(
                "recent workflow: {} {}; {}",
                summary.workflow_id, status, pulse_one_liner
            )
        })
        .unwrap_or_else(|| fallback.to_string())
}

pub struct GreetingJob;

#[async_trait::async_trait]
impl BuddyJob for GreetingJob {
    fn id(&self) -> &str {
        "greeting"
    }
    fn cooldown_seconds(&self) -> u64 {
        86400
    }
    fn priority(&self) -> u32 {
        0
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        !ctx.onboarding.greeted || ctx.job_state.last_run.is_none()
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let fallback_text = greeting_fallback_text(&ctx);
        let controls = greeting_controls(&ctx);
        let mut speech = match crate::buddy::actor::buddy_snapshot(gcx.clone()).await {
            Some(snapshot) => {
                let pulse_one_liner = format!(
                    "{} pending ops, {} recent stuck task alerts",
                    snapshot.pulse.memory.pending_ops,
                    snapshot.pulse.tasks.recent_stuck_alert_count_1h()
                );
                let workflow_summary =
                    greeting_workflow_summary(&ctx, &pulse_one_liner, &fallback_text);
                let voice_ctx = VoiceCtx {
                    persona: &snapshot.state.personality,
                    identity_name: snapshot.state.identity.name.as_str(),
                    pulse_one_liner,
                    workflow_id: Some("greeting"),
                    workflow_summary: Some(&workflow_summary),
                };
                voice_service()
                    .await
                    .render_speech(gcx, voice_ctx, SpeechIntent::Greeting)
                    .await
            }
            None => BuddySpeechItem {
                id: format!("greeting-{}", chrono::Utc::now().timestamp()),
                text: fallback_text,
                mood: "happy".to_string(),
                scope: "global".to_string(),
                persistent: false,
                ttl_seconds: if ctx.onboarding.greeted { 8 } else { 15 },
                dedupe_key: Some("greeting".to_string()),
                created_at: chrono::Utc::now().to_rfc3339(),
                controls: vec![],
                chat_id: None,
                speech_intent: None,
            },
        };
        speech.ttl_seconds = if ctx.onboarding.greeted { 8 } else { 15 };
        speech.dedupe_key = Some("greeting".to_string());
        speech.controls = controls;
        BuddyJobResult {
            speech: Some(speech),
            speech_intent: Some(SpeechIntent::Greeting),
            mark_greeted: true,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::scheduler::BuddyJobContext;
    use crate::buddy::settings::BuddySettings;
    use crate::buddy::types::{
        BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse, BuddySuggestion,
    };

    fn test_context() -> BuddyJobContext {
        BuddyJobContext {
            identity_name: "Pixel".to_string(),
            personality: Default::default(),
            onboarding: BuddyOnboarding::default(),
            recent_diagnostics: vec![],
            project_root: std::path::PathBuf::from("/tmp/project"),
            job_state: BuddyJobState::default(),
            workflow_summaries: vec![],
            total_workflow_runs: 0,
            suggestion_state: Vec::<BuddySuggestion>::new(),
            pet: BuddyPetState::default(),
            active_quest: None,
            settings: BuddySettings::default(),
            pulse: BuddyPulse::default(),
            facts: vec![],
            recent_activities: vec![],
        }
    }

    async fn make_gcx_with_buddy() -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let mut state = crate::buddy::state::default_buddy_state();
        state.identity.name = "Pixel".to_string();
        let service = crate::buddy::actor::BuddyService::new(
            std::env::temp_dir().join(format!(
                "buddy-greeting-voice-test-{}",
                uuid::Uuid::new_v4()
            )),
            state,
            BuddySettings::default(),
            Vec::new(),
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        let app = AppState::from_gcx(gcx).await;
        let buddy_arc = app.buddy.buddy.clone();
        *buddy_arc.lock().await = Some(service);
        app
    }

    #[tokio::test]
    async fn greeting_uses_voice_service_when_available() {
        let (service, renderer) = crate::buddy::voice_service::test_voice_service_with_responses(
            vec![Some("voice hello".to_string())],
        );
        let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
        let gcx = make_gcx_with_buddy().await;
        let job = GreetingJob;

        let result = job.execute(gcx, test_context()).await;

        let speech = result.speech.unwrap();
        assert_eq!(speech.text, "voice hello");
        assert_eq!(speech.dedupe_key.as_deref(), Some("greeting"));
        assert!(result.mark_greeted);
        assert_eq!(renderer.intent_kinds(), vec!["speech:greeting".to_string()]);
    }
}
