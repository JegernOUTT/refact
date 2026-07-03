use crate::app_state::AppState;

use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::{BuddyControl, BuddyQuest, BuddySpeechItem, BuddySuggestion};
use crate::buddy::voice_service::{SpeechIntent, VoiceCtx, voice_service};

pub struct QuestPromptJob;

fn has_active_suggestion(ctx: &BuddyJobContext, kind: &str) -> bool {
    ctx.suggestion_state
        .iter()
        .any(|suggestion| !suggestion.dismissed && suggestion.suggestion_type == kind)
}

fn make_accept_controls(kind: &str) -> Vec<BuddyControl> {
    let primary = match kind {
        "start_setup" => BuddyControl {
            id: "quest-setup".to_string(),
            label: "Start Setup".to_string(),
            action: "accept_quest".to_string(),
            action_param: Some(kind.to_string()),
            style: "primary".to_string(),
        },
        "care_buddy" => BuddyControl {
            id: "quest-care".to_string(),
            label: "Take Quest".to_string(),
            action: "accept_quest".to_string(),
            action_param: Some(kind.to_string()),
            style: "primary".to_string(),
        },
        "run_workflow" => BuddyControl {
            id: "quest-workflow".to_string(),
            label: "Take Quest".to_string(),
            action: "accept_quest".to_string(),
            action_param: Some(kind.to_string()),
            style: "primary".to_string(),
        },
        _ => BuddyControl {
            id: "quest-default".to_string(),
            label: "Take Quest".to_string(),
            action: "accept_quest".to_string(),
            action_param: Some(kind.to_string()),
            style: "primary".to_string(),
        },
    };
    vec![
        primary,
        BuddyControl {
            id: format!("quest-later-{kind}"),
            label: "Later".to_string(),
            action: "dismiss".to_string(),
            action_param: None,
            style: "secondary".to_string(),
        },
    ]
}

fn make_quest(ctx: &BuddyJobContext, kind: &str) -> Option<BuddyQuest> {
    let now = chrono::Utc::now().to_rfc3339();
    let traits = &ctx.pet;
    let title = match kind {
        "start_setup" => "Warm up this workspace".to_string(),
        "care_buddy" => format!("Give {} a quick play break", ctx.identity_name),
        "run_workflow" => "Make one productive move".to_string(),
        _ => return None,
    };
    let description = match kind {
        "start_setup" => {
            "Kick off setup so I can learn this repo and unlock smarter help.".to_string()
        }
        "care_buddy" => {
            if traits.condition.bored || traits.condition.lonely {
                "I’m getting restless. One playful check-in should lift the vibe.".to_string()
            } else {
                "A tiny play session keeps my chaos in the fun zone.".to_string()
            }
        }
        "run_workflow" => "Finish one workflow or task and I’ll turn it into growth.".to_string(),
        _ => return None,
    };
    let goal = match kind {
        "start_setup" => 1,
        "care_buddy" => 5,
        "run_workflow" => 1,
        _ => 1,
    };
    let baseline = match kind {
        "care_buddy" => ctx.pet.evolution.care_score.min(u64::from(u32::MAX)) as u32,
        "run_workflow" => ctx.total_workflow_runs.min(u64::from(u32::MAX)) as u32,
        _ => 0,
    };
    let reward_xp = match kind {
        "start_setup" => 10,
        "care_buddy" => 8,
        "run_workflow" => 12,
        _ => 8,
    };

    Some(BuddyQuest {
        id: format!("quest-{}-{}", kind, chrono::Utc::now().timestamp()),
        quest_type: kind.to_string(),
        title,
        description,
        icon: match kind {
            "start_setup" => "🧰".to_string(),
            "care_buddy" => "🎾".to_string(),
            "run_workflow" => "⚙️".to_string(),
            _ => "✨".to_string(),
        },
        created_at: now.clone(),
        accepted_at: now,
        status: "active".to_string(),
        completed_at: None,
        progress: 0,
        goal,
        baseline,
        reward_xp,
        controls: vec![],
    })
}

fn pick_quest(ctx: &BuddyJobContext) -> Option<&'static str> {
    if ctx.active_quest.is_some() {
        return None;
    }
    if !ctx.onboarding.tour_completed && !has_active_suggestion(ctx, "quest_start_setup") {
        return Some("start_setup");
    }
    if (ctx.pet.condition.bored || ctx.pet.condition.lonely)
        && !has_active_suggestion(ctx, "quest_care_buddy")
    {
        return Some("care_buddy");
    }
    if ctx.total_workflow_runs == 0 && !has_active_suggestion(ctx, "quest_run_workflow") {
        return Some("run_workflow");
    }
    None
}

async fn voice_quest_speech(
    gcx: AppState,
    _ctx: &BuddyJobContext,
    kind: &str,
    fallback: &str,
) -> BuddySpeechItem {
    let Some(snapshot) = crate::buddy::actor::buddy_snapshot(gcx.clone()).await else {
        return BuddySpeechItem {
            id: format!("quest-prompt-{}-{}", kind, chrono::Utc::now().timestamp()),
            text: fallback.to_string(),
            mood: "excited".to_string(),
            scope: "global".to_string(),
            persistent: false,
            ttl_seconds: 12,
            dedupe_key: Some(format!("quest_prompt_{kind}")),
            created_at: chrono::Utc::now().to_rfc3339(),
            controls: vec![],
            chat_id: None,
            speech_intent: None,
        };
    };
    let pulse_one_liner = format!(
        "{} pending ops, {} recent stuck task alerts",
        snapshot.pulse.memory.pending_ops,
        snapshot.pulse.tasks.recent_stuck_alert_count_1h()
    );
    let voice_ctx = VoiceCtx {
        persona: &snapshot.state.personality,
        identity_name: snapshot.state.identity.name.as_str(),
        pulse_one_liner,
        workflow_id: Some(kind),
        workflow_summary: Some(fallback),
    };
    let mut speech = voice_service()
        .await
        .render_speech(gcx, voice_ctx, SpeechIntent::QuestAccept)
        .await;
    speech.ttl_seconds = 12;
    speech.dedupe_key = Some(format!("quest_prompt_{kind}"));
    speech
}

#[async_trait::async_trait]
impl BuddyJob for QuestPromptJob {
    fn id(&self) -> &str {
        "quest_prompt"
    }

    fn cooldown_seconds(&self) -> u64 {
        900
    }

    fn priority(&self) -> u32 {
        5
    }

    fn produces_suggestion(&self) -> bool {
        true
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        pick_quest(ctx).is_some()
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let Some(kind) = pick_quest(&ctx) else {
            return BuddyJobResult::default();
        };
        let Some(mut quest) = make_quest(&ctx, kind) else {
            return BuddyJobResult::default();
        };
        let speech = voice_quest_speech(gcx, &ctx, kind, quest.description.as_str()).await;
        quest.description = speech.text.clone();

        let suggestion_id = format!(
            "quest-suggestion-{}-{}",
            kind,
            chrono::Utc::now().timestamp()
        );
        let now = chrono::Utc::now();
        let quest_opportunity = super::super::types::BuddyOpportunity {
            id: format!("quest-opp-{}-{}", kind, now.timestamp()),
            kind: super::super::types::BuddyOpportunityKind::Quest,
            summary: format!("{} (+{} XP)", quest.title, quest.reward_xp),
            priority: super::super::types::BuddyPriority::Normal,
            confidence: 1.0,
            fact_keys: vec![],
            cooldown_key: format!("quest:{}", kind),
            cooldown_secs: self.cooldown_seconds(),
            status: super::super::types::OpportunityStatus::New,
            proposed_actions: vec![
                super::super::types::BuddyAction::AcceptQuest {
                    suggestion_id: suggestion_id.clone(),
                },
                super::super::types::BuddyAction::Dismiss,
            ],
            humor: None,
            humor_allowed: false,
            related: super::super::types::BuddyOpportunityLinks::default(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(24),
            resolved_at: None,
        };

        BuddyJobResult {
            opportunities: vec![(quest_opportunity, self.cooldown_seconds())],
            speech_intent: Some(SpeechIntent::QuestAccept),
            runtime_event: Some(super::super::scheduler::speech_runtime_event(
                self.id(),
                SpeechIntent::QuestAccept,
                &speech,
                quest.title.clone(),
                Some(quest.description.clone()),
            )),
            speech: Some(speech),
            suggestion: Some(BuddySuggestion {
                id: suggestion_id.clone(),
                suggestion_type: format!("quest_{kind}"),
                title: quest.title.clone(),
                description: quest.description.clone(),
                created_at: chrono::Utc::now().to_rfc3339(),
                dismissed: false,
                controls: make_accept_controls(kind)
                    .into_iter()
                    .map(|mut ctrl| {
                        if ctrl.action == "accept_quest" {
                            ctrl.action_param = Some(suggestion_id.clone());
                        }
                        ctrl
                    })
                    .collect(),
                quest: Some(quest),
            }),
            last_result: Some(kind.to_string()),
            ..Default::default()
        }
    }
}
