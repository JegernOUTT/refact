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
}
