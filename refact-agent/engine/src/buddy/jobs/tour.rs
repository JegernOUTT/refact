use crate::app_state::AppState;

use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult, SpeechCandidate};
use super::super::types::BuddySpeechItem;
use crate::buddy::voice_service::SpeechIntent;

pub struct TourJob;

fn tour_fallback_speech(text: String) -> BuddySpeechItem {
    BuddySpeechItem {
        editorial_delivery_required: false,
        id: format!("tour-{}", chrono::Utc::now().timestamp()),
        text,
        mood: "excited".to_string(),
        scope: "global".to_string(),
        persistent: false,
        ttl_seconds: 15,
        dedupe_key: Some("tour".to_string()),
        created_at: chrono::Utc::now().to_rfc3339(),
        controls: vec![],
        chat_id: None,
        speech_intent: None,
    }
}

pub(crate) fn normalize_tour_speech(
    mut fallback: BuddySpeechItem,
    voice_speech: BuddySpeechItem,
) -> BuddySpeechItem {
    if !voice_speech.text.trim().is_empty() {
        fallback.text = voice_speech.text;
        if !voice_speech.mood.trim().is_empty() {
            fallback.mood = voice_speech.mood;
        }
    }
    fallback
}

async fn tour_speech_allowed(gcx: &AppState) -> bool {
    let buddy_arc = gcx.buddy.buddy.clone();
    let buddy = buddy_arc.lock().await;
    let Some(svc) = buddy.as_ref() else {
        return false;
    };
    crate::buddy::speech_policy::gate_speech(
        &svc.settings,
        &svc.state.speech_rotation,
        Some(SpeechIntent::Tour),
        None,
        chrono::Timelike::hour(&chrono::Local::now()),
        svc.auto_quiet_window,
        chrono::Utc::now(),
    )
    .allowed
}

#[async_trait::async_trait]
impl BuddyJob for TourJob {
    fn id(&self) -> &str {
        "tour"
    }
    fn cooldown_seconds(&self) -> u64 {
        86400
    }
    fn priority(&self) -> u32 {
        1
    }

    fn records_empty_result(&self) -> bool {
        false
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        ctx.onboarding.greeted && !ctx.onboarding.tour_completed && ctx.job_state.last_run.is_none()
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        if !tour_speech_allowed(&gcx).await {
            return BuddyJobResult::default();
        }
        let fallback_text = "Your Desk keeps saved results and pending questions together. Open Play whenever you want a break.".to_string();
        let mut candidate = SpeechCandidate::from_context(
            &ctx,
            SpeechIntent::Tour,
            Some("tour".into()),
            fallback_text.clone(),
            fallback_text.clone(),
            "Buddy Desk tour".into(),
            None,
        );
        candidate.template = Some(tour_fallback_speech(fallback_text));
        BuddyJobResult {
            speech_candidate: Some(candidate),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::scheduler::BuddyJobContext;
    use crate::buddy::settings::{BuddySettings, QuietHoursMode};
    use crate::buddy::types::{BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse};

    fn test_context() -> BuddyJobContext {
        BuddyJobContext {
            identity_name: "Pixel".to_string(),
            personality: Default::default(),
            onboarding: BuddyOnboarding {
                greeted: true,
                tour_completed: false,
                ..Default::default()
            },
            recent_diagnostics: vec![],
            project_root: std::path::PathBuf::from("/tmp/project"),
            job_state: BuddyJobState::default(),
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

    async fn make_gcx_with_quiet_buddy() -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let mut settings = BuddySettings::default();
        settings.quiet_hours_mode = QuietHoursMode::Fixed;
        let hour = chrono::Timelike::hour(&chrono::Local::now());
        settings.quiet_hours_start = hour as u8;
        settings.quiet_hours_end = ((hour + 1) % 24) as u8;
        let mut state = crate::buddy::state::default_buddy_state();
        state.onboarding.greeted = true;
        let service = crate::buddy::actor::BuddyService::new(
            std::env::temp_dir().join(format!("buddy-tour-drop-test-{}", uuid::Uuid::new_v4())),
            state,
            settings,
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

    #[test]
    fn tour_speech_preserves_fallback_fields_after_voice_render() {
        let fallback = tour_fallback_speech("fallback".to_string());
        let voice = BuddySpeechItem {
            editorial_delivery_required: false,
            id: "voice-id".to_string(),
            text: "voice text".to_string(),
            mood: "curious".to_string(),
            scope: "chat".to_string(),
            persistent: true,
            ttl_seconds: 999,
            dedupe_key: Some("voice".to_string()),
            created_at: "voice-created".to_string(),
            controls: vec![crate::buddy::types::BuddyControl {
                id: "voice-control".to_string(),
                label: "Voice".to_string(),
                action: "voice".to_string(),
                action_param: None,
                style: "primary".to_string(),
            }],
            chat_id: Some("chat".to_string()),
            speech_intent: None,
        };

        let normalized = normalize_tour_speech(fallback.clone(), voice);

        assert_eq!(normalized.text, "voice text");
        assert_eq!(normalized.mood, "curious");
        assert_eq!(normalized.id, fallback.id);
        assert_eq!(normalized.scope, fallback.scope);
        assert_eq!(normalized.persistent, fallback.persistent);
        assert_eq!(normalized.ttl_seconds, fallback.ttl_seconds);
        assert_eq!(normalized.dedupe_key, fallback.dedupe_key);
        assert_eq!(normalized.created_at, fallback.created_at);
        assert_eq!(normalized.controls.len(), fallback.controls.len());
        assert_eq!(normalized.chat_id, fallback.chat_id);
    }

    #[test]
    fn tour_speech_preserves_fallback_text_and_mood_when_voice_text_empty() {
        let fallback = tour_fallback_speech("fallback".to_string());
        let voice = BuddySpeechItem {
            editorial_delivery_required: false,
            id: "voice-id".to_string(),
            text: "   ".to_string(),
            mood: "curious".to_string(),
            scope: "chat".to_string(),
            persistent: true,
            ttl_seconds: 999,
            dedupe_key: Some("voice".to_string()),
            created_at: "voice-created".to_string(),
            controls: vec![],
            chat_id: Some("chat".to_string()),
            speech_intent: None,
        };

        let normalized = normalize_tour_speech(fallback.clone(), voice);

        assert_eq!(normalized.text, "fallback");
        assert_eq!(normalized.mood, fallback.mood);
        assert_eq!(normalized.id, fallback.id);
    }

    #[tokio::test]
    async fn dropped_tour_speech_does_not_return_recordable_completion() {
        let (voice, renderer) = crate::buddy::voice_service::test_voice_service_with_responses(
            vec![Some("should not render".into())],
        );
        let _guard = crate::buddy::voice_service::install_test_voice_service(voice).await;
        let gcx = make_gcx_with_quiet_buddy().await;
        let job = TourJob;

        let result = job.execute(gcx.clone(), test_context()).await;

        assert!(result.last_result.is_none());
        assert!(result.speech.is_none());
        assert!(result.speech_intent.is_none());
        assert!(!job.records_empty_result());
        assert_eq!(renderer.calls(), 0);
        let buddy = gcx.buddy.buddy.lock().await;
        let service = buddy.as_ref().unwrap();
        assert!(service.active_speech.is_none());
        assert!(service.state.job_cooldowns.get("tour").is_none());
    }
}
