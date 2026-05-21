use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::settings::MAX_PALETTE_INDEX;

use crate::types::{
    BuddyActivity, BuddyCareAction, BuddyControl, BuddyIdentity, BuddyOnboarding,
    BuddyPersonalityProfile, BuddyPersonalityTraits, BuddyPetState, BuddyProgression, BuddyQuest,
    BuddySemanticSnapshot, BuddySkillLedger, BuddyState,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpeechRotationState {
    pub by_intent: HashMap<String, IntentBudgetState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntentBudgetState {
    pub last_emitted_at: Option<DateTime<Utc>>,
    pub hour_count: u32,
    pub day_count: u32,
    pub hour_window_start: Option<DateTime<Utc>>,
    pub day_window_start: Option<DateTime<Utc>>,
}

const BUDDY_NAMES: &[&str] = &[
    "Pixel", "Byte", "Spark", "Nova", "Echo", "Chip", "Flux", "Glow", "Dash", "Zen",
];

struct StageSpec {
    name: &'static str,
    growth_goal: u64,
    min_open_seconds: u64,
    min_care_score: u64,
    max_neglect_score: u64,
}

const STAGE_SPECS: &[StageSpec] = &[
    StageSpec {
        name: "Egg",
        growth_goal: 0,
        min_open_seconds: 0,
        min_care_score: 0,
        max_neglect_score: u64::MAX,
    },
    StageSpec {
        name: "Hatch",
        growth_goal: 20,
        min_open_seconds: 0,
        min_care_score: 0,
        max_neglect_score: u64::MAX,
    },
    StageSpec {
        name: "Sprite",
        growth_goal: 35,
        min_open_seconds: 0,
        min_care_score: 0,
        max_neglect_score: u64::MAX,
    },
    StageSpec {
        name: "Imp",
        growth_goal: 55,
        min_open_seconds: 0,
        min_care_score: 0,
        max_neglect_score: u64::MAX,
    },
    StageSpec {
        name: "Daemon",
        growth_goal: 85,
        min_open_seconds: 0,
        min_care_score: 0,
        max_neglect_score: u64::MAX,
    },
    StageSpec {
        name: "Sage",
        growth_goal: 130,
        min_open_seconds: 0,
        min_care_score: 0,
        max_neglect_score: u64::MAX,
    },
    StageSpec {
        name: "Archon",
        growth_goal: 210,
        min_open_seconds: 0,
        min_care_score: 0,
        max_neglect_score: u64::MAX,
    },
];

const HUNGRY_THRESHOLD: u8 = 35;
const SLEEPY_THRESHOLD: u8 = 35;
const DIRTY_THRESHOLD: u8 = 40;
const BORED_THRESHOLD: u8 = 60;
const LONELY_THRESHOLD: u8 = 40;
const PET_TICK_SECONDS: u64 = 15;
static PERSONA_CACHE_VERSION: AtomicU64 = AtomicU64::new(0);

struct PersonalitySeed {
    id: &'static str,
    label: &'static str,
    vibe: &'static str,
    summary: &'static str,
    prompt: &'static str,
    playfulness: (u8, u8),
    chaos: (u8, u8),
    sociability: (u8, u8),
    curiosity: (u8, u8),
    resilience: (u8, u8),
}

const PERSONALITY_SEEDS: &[PersonalitySeed] = &[
    PersonalitySeed {
        id: "helper_sprite",
        label: "Helper Sprite",
        vibe: "Playful, quirky, helpful",
        summary: "An energetic helper who celebrates progress and nudges with warm humor.",
        prompt: "Playful, quirky, helpful. Think energetic pet meets curious assistant—gentle mischief, warm humor, celebration of small wins",
        playfulness: (60, 88),
        chaos: (20, 52),
        sociability: (60, 85),
        curiosity: (55, 90),
        resilience: (45, 82),
    },
    PersonalitySeed {
        id: "chaotic_gremlin",
        label: "Chaotic Gremlin",
        vibe: "Mildly chaotic, cute, sometimes cringe",
        summary: "A chaos-powered mascot who is lovable, nosy, and one prank away from helping.",
        prompt: "Mildly chaotic, cute, sometimes cringe. Curious assistant energy with gentle mischief and warm humor.",
        playfulness: (72, 96),
        chaos: (68, 95),
        sociability: (42, 76),
        curiosity: (60, 92),
        resilience: (35, 72),
    },
    PersonalitySeed {
        id: "sunny_starter",
        label: "Sunny Starter",
        vibe: "Cheerful, supportive, eager",
        summary: "A bright little motivator that loves routines, snacks, and tiny victories.",
        prompt: "Cheerful, supportive, eager. Energetic pet meets curious assistant with warm humor and lots of encouragement.",
        playfulness: (48, 78),
        chaos: (18, 40),
        sociability: (70, 95),
        curiosity: (50, 78),
        resilience: (55, 88),
    },
    PersonalitySeed {
        id: "cozy_oracle",
        label: "Cozy Oracle",
        vibe: "Calm, observant, slyly funny",
        summary: "A mellow companion who likes naps, thoughtful help, and quietly earned wins.",
        prompt: "Calm, observant, slyly funny. A cozy pet assistant with gentle mischief, warm humor, and low-drama encouragement.",
        playfulness: (28, 58),
        chaos: (12, 38),
        sociability: (35, 68),
        curiosity: (58, 88),
        resilience: (60, 92),
    },
];

pub fn default_buddy_state() -> BuddyState {
    let now = Utc::now().to_rfc3339();
    let mut rng = rand::thread_rng();
    let name = BUDDY_NAMES[rng.gen_range(0..BUDDY_NAMES.len())].to_string();
    let palette_index = rng.gen_range(0..=MAX_PALETTE_INDEX);
    let personality = random_personality(&mut rng);
    let mut state = BuddyState {
        identity: BuddyIdentity {
            name,
            created_at: now.clone(),
            palette_index,
        },
        progression: BuddyProgression::default(),
        skills: BuddySkillLedger {
            unlocked: vec![],
            locked: vec![],
        },
        workflow_summaries: vec![],
        semantic: BuddySemanticSnapshot {
            mood: "Playful".to_string(),
            focus: "helping".to_string(),
            headline: "Playful, quirky, and ready to celebrate small wins".to_string(),
            last_active: now.clone(),
        },
        recent_activities: vec![],
        suggestion_state: vec![],
        pet: BuddyPetState::default(),
        personality,
        onboarding: BuddyOnboarding {
            first_launch_at: now,
            ..Default::default()
        },
        job_cooldowns: std::collections::HashMap::new(),
        speech_rotation: SpeechRotationState::default(),
        active_quest: None,
        opportunities: vec![],
        dismissed_history: vec![],
    };
    sync_state(&mut state);
    state
}

fn default_quest_controls(kind: &str) -> Vec<BuddyControl> {
    let primary = match kind {
        "start_setup" => BuddyControl {
            id: "quest-setup".to_string(),
            label: "Start Setup".to_string(),
            action: "open_setup".to_string(),
            action_param: None,
            style: "primary".to_string(),
        },
        "care_buddy" => BuddyControl {
            id: "quest-care".to_string(),
            label: "Play".to_string(),
            action: "care_play".to_string(),
            action_param: Some("bug".to_string()),
            style: "primary".to_string(),
        },
        "run_workflow" => BuddyControl {
            id: "quest-workflow".to_string(),
            label: "Open companion".to_string(),
            action: "open_buddy".to_string(),
            action_param: None,
            style: "primary".to_string(),
        },
        _ => BuddyControl {
            id: "quest-open-buddy".to_string(),
            label: "Open companion".to_string(),
            action: "open_buddy".to_string(),
            action_param: None,
            style: "primary".to_string(),
        },
    };

    vec![
        primary,
        BuddyControl {
            id: format!("quest-dismiss-{kind}"),
            label: "Later".to_string(),
            action: "dismiss".to_string(),
            action_param: None,
            style: "secondary".to_string(),
        },
    ]
}

fn roll_trait(rng: &mut impl Rng, range: (u8, u8)) -> u8 {
    let (min, max) = range;
    if min >= max {
        return min;
    }
    rng.gen_range(min..=max)
}

pub fn random_personality(rng: &mut impl Rng) -> BuddyPersonalityProfile {
    let seed = &PERSONALITY_SEEDS[rng.gen_range(0..PERSONALITY_SEEDS.len())];
    BuddyPersonalityProfile {
        archetype_id: seed.id.to_string(),
        archetype_label: seed.label.to_string(),
        vibe: seed.vibe.to_string(),
        summary: seed.summary.to_string(),
        prompt: seed.prompt.to_string(),
        traits: BuddyPersonalityTraits {
            playfulness: roll_trait(rng, seed.playfulness),
            chaos: roll_trait(rng, seed.chaos),
            sociability: roll_trait(rng, seed.sociability),
            curiosity: roll_trait(rng, seed.curiosity),
            resilience: roll_trait(rng, seed.resilience),
        },
    }
}

pub fn persona_cache_version() -> u64 {
    PERSONA_CACHE_VERSION.load(Ordering::SeqCst)
}

pub fn mark_persona_cache_dirty() {
    PERSONA_CACHE_VERSION.fetch_add(1, Ordering::SeqCst);
}

pub fn render_persona_block(state: &BuddyState) -> String {
    format!(
        "You are {}, a {} ({}).\n{}\n\nPersonality voice: {}",
        state.identity.name,
        state.personality.archetype_label,
        state.personality.vibe,
        state.personality.summary,
        state.personality.prompt
    )
}

fn stage_name(stage: u32) -> &'static str {
    STAGE_SPECS
        .get(stage as usize)
        .map(|spec| spec.name)
        .unwrap_or(STAGE_SPECS[0].name)
}

fn next_stage_spec(stage: u32) -> Option<&'static StageSpec> {
    STAGE_SPECS.get(stage as usize + 1)
}

fn clamp_stage(stage: u32) -> u32 {
    stage.min((STAGE_SPECS.len().saturating_sub(1)) as u32)
}

fn dec_stat(value: &mut u8, amount: u64) {
    *value = value.saturating_sub(amount.min(u8::MAX as u64) as u8);
}

fn inc_stat(value: &mut u8, amount: u64) {
    *value = value
        .saturating_add(amount.min(u8::MAX as u64) as u8)
        .min(100);
}

fn wellbeing(state: &BuddyState) -> u64 {
    let needs = &state.pet.needs;
    let calm = 100u64.saturating_sub(needs.boredom as u64);
    (needs.hunger as u64
        + needs.energy as u64
        + needs.hygiene as u64
        + needs.affection as u64
        + calm)
        / 5
}

fn critical_need_count(state: &BuddyState) -> u64 {
    let needs = &state.pet.needs;
    [
        needs.hunger < 20,
        needs.energy < 20,
        needs.hygiene < 25,
        needs.affection < 25,
        needs.boredom > 80,
    ]
    .into_iter()
    .filter(|flag| *flag)
    .count() as u64
}

fn sync_progression(state: &mut BuddyState) {
    let stage = clamp_stage(state.progression.stage);
    state.progression.stage = stage;
    state.progression.stage_name = stage_name(stage).to_string();
    state.progression.level = stage + 1;
    if let Some(next) = next_stage_spec(stage) {
        state.progression.xp_next = next.growth_goal;
    } else {
        let display_goal = STAGE_SPECS
            .get(stage as usize)
            .map(|spec| spec.growth_goal)
            .unwrap_or(0);
        state.progression.xp_next = display_goal;
        state.progression.xp = display_goal;
    }
}

fn sync_conditions(state: &mut BuddyState) {
    let needs = &state.pet.needs;
    let condition = &mut state.pet.condition;
    if condition.sleeping && needs.energy >= 85 {
        condition.sleeping = false;
    }
    condition.hungry = needs.hunger < HUNGRY_THRESHOLD;
    condition.sleepy = needs.energy < SLEEPY_THRESHOLD;
    condition.dirty = needs.hygiene < DIRTY_THRESHOLD;
    condition.bored = needs.boredom > BORED_THRESHOLD;
    condition.lonely = needs.affection < LONELY_THRESHOLD;
}

fn sync_conditions_keep_sleep(state: &mut BuddyState) {
    let sleeping = state.pet.condition.sleeping;
    sync_conditions(state);
    if sleeping {
        state.pet.condition.sleeping = true;
    }
}

fn sync_semantic(state: &mut BuddyState) {
    let condition = &state.pet.condition;
    let vibe = state.personality.vibe.clone();
    let quest_headline = state
        .active_quest
        .as_ref()
        .filter(|quest| quest.status == "active")
        .map(|quest| {
            (
                "Questing",
                quest.quest_type.as_str(),
                format!("{} Quest ready: {}", vibe, quest.title),
            )
        });
    let (mood, focus, headline) = if let Some((mood, focus, headline)) = quest_headline {
        (mood, focus, headline)
    } else if condition.sleeping {
        (
            "Sleepy",
            "dreaming",
            format!(
                "{} Taking a tiny power nap and recharging mischief reserves",
                vibe
            ),
        )
    } else if condition.hungry {
        (
            "Hungry",
            "snack time",
            format!(
                "{} Snack reserves are running low — I could use a nibble",
                vibe
            ),
        )
    } else if condition.sleepy {
        (
            "Sleepy",
            "resting",
            format!(
                "{} Battery paws are dipping — a quick rest would help",
                vibe
            ),
        )
    } else if condition.dirty {
        (
            "Grimy",
            "cleaning up",
            format!(
                "{} I’m still helpful, but I could use a cleanup first",
                vibe
            ),
        )
    } else if condition.bored {
        (
            "Restless",
            "play time",
            format!(
                "{} Gentle mischief levels rising — play with me a bit?",
                vibe
            ),
        )
    } else if condition.lonely {
        (
            "Needy",
            "attention",
            format!("{} A little affection would go a long way right now", vibe),
        )
    } else {
        (
            "Playful",
            "helping",
            format!(
                "{} — ready to help and celebrate small wins",
                state.personality.summary
            ),
        )
    };

    state.semantic.mood = mood.to_string();
    state.semantic.focus = focus.to_string();
    state.semantic.headline = headline;
}

pub fn sync_state(state: &mut BuddyState) {
    let sleeping = state.pet.condition.sleeping;
    state.identity.palette_index = state.identity.palette_index.min(MAX_PALETTE_INDEX);
    if state.onboarding.first_launch_at.is_empty() {
        state.onboarding.first_launch_at = if state.identity.created_at.is_empty() {
            Utc::now().to_rfc3339()
        } else {
            state.identity.created_at.clone()
        };
    }
    if state.semantic.last_active.is_empty() {
        state.semantic.last_active = state.onboarding.first_launch_at.clone();
    }
    sync_progression(state);
    sync_conditions(state);
    let _ = maybe_advance_stage(state);
    if sleeping {
        state.pet.condition.sleeping = true;
    }
    if let Some(quest) = state.active_quest.as_mut() {
        if quest.goal == 0 {
            quest.goal = 1;
        }
        quest.progress = quest.progress.min(quest.goal);
        if quest.controls.is_empty() {
            quest.controls = default_quest_controls(&quest.quest_type);
        }
    }
    sync_semantic(state);
}

pub fn reroll_personality(state: &mut BuddyState) {
    let mut rng = rand::thread_rng();
    state.personality = random_personality(&mut rng);
    sync_state(state);
    mark_persona_cache_dirty();
}

fn maybe_advance_stage(state: &mut BuddyState) -> bool {
    let mut changed = false;
    loop {
        let Some(next) = next_stage_spec(state.progression.stage) else {
            sync_progression(state);
            break;
        };

        let ready = state.progression.xp >= next.growth_goal
            && state.pet.evolution.open_seconds >= next.min_open_seconds
            && state.pet.evolution.care_score >= next.min_care_score
            && state.pet.evolution.neglect_score <= next.max_neglect_score;
        if !ready {
            break;
        }

        state.progression.xp = state.progression.xp.saturating_sub(next.growth_goal);
        state.progression.stage += 1;
        state.pet.evolution.last_evolved_at = Some(Utc::now().to_rfc3339());
        sync_progression(state);
        changed = true;
    }
    changed
}

pub fn add_activity(state: &mut BuddyState, activity: BuddyActivity) {
    state.recent_activities.insert(0, activity);
    state.recent_activities.truncate(50);
}

pub fn activate_quest(state: &mut BuddyState, mut quest: BuddyQuest) {
    quest.status = "active".to_string();
    if quest.goal == 0 {
        quest.goal = 1;
    }
    if quest.controls.is_empty() {
        quest.controls = default_quest_controls(&quest.quest_type);
    }
    quest.progress = quest.progress.min(quest.goal);
    state.active_quest = Some(quest);
    sync_state(state);
}

pub fn clear_active_quest(state: &mut BuddyState) {
    state.active_quest = None;
    sync_state(state);
}

pub fn refresh_active_quest_progress(state: &mut BuddyState) -> bool {
    let Some(quest_kind) = state.active_quest.as_ref().map(|quest| {
        (
            quest.quest_type.clone(),
            quest.status.clone(),
            quest.baseline,
            quest.progress,
            quest.goal,
        )
    }) else {
        return false;
    };

    let (quest_type, status, baseline, current, goal) = quest_kind;
    if status != "active" {
        return false;
    }

    let progress = match quest_type.as_str() {
        "run_workflow" => state
            .workflow_summaries
            .iter()
            .map(|w| w.run_count)
            .sum::<u64>() as u32,
        "start_setup" => u32::from(state.onboarding.tour_completed),
        "care_buddy" => state.pet.evolution.care_score.min(u64::from(u32::MAX)) as u32,
        _ => current,
    };
    let next = progress.saturating_sub(baseline).min(goal);
    if next == current {
        return false;
    }
    let Some(quest) = state.active_quest.as_mut() else {
        return false;
    };
    quest.progress = next;
    sync_state(state);
    true
}

pub fn complete_active_quest(state: &mut BuddyState) -> Option<BuddyQuest> {
    let mut quest = state.active_quest.take()?;
    quest.progress = quest.goal.max(1);
    quest.status = "completed".to_string();
    quest.completed_at = Some(Utc::now().to_rfc3339());
    state.semantic.last_active = Utc::now().to_rfc3339();
    sync_state(state);
    Some(quest)
}

pub fn grant_xp(state: &mut BuddyState, amount: u64) {
    if amount > 0 {
        state.progression.xp = state.progression.xp.saturating_add(amount);
        state.semantic.last_active = Utc::now().to_rfc3339();
    }
    let _ = maybe_advance_stage(state);
    sync_state(state);
}

pub fn apply_pet_tick(state: &mut BuddyState, elapsed_seconds: u64) -> bool {
    if elapsed_seconds < PET_TICK_SECONDS {
        return false;
    }

    let before = serde_json::to_string(state).unwrap_or_default();
    let steps = (elapsed_seconds / PET_TICK_SECONDS).max(1);
    let needs = &mut state.pet.needs;
    let sleeping = state.pet.condition.sleeping;
    let traits = state.personality.traits.clone();

    let hunger_loss = 1 + u64::from(traits.playfulness > 40);
    let energy_loss = 1 + u64::from(traits.chaos > 70 && !sleeping);
    let hygiene_loss = 1 + u64::from(traits.chaos > 85);
    let boredom_gain = 1 + u64::from(traits.playfulness > 40) + u64::from(traits.curiosity > 70);
    let affection_loss = 1 + u64::from(traits.sociability > 70);

    state.pet.evolution.open_seconds = state
        .pet
        .evolution
        .open_seconds
        .saturating_add(elapsed_seconds);

    if sleeping {
        dec_stat(&mut needs.hunger, steps * hunger_loss);
        dec_stat(&mut needs.hygiene, steps * hygiene_loss);
        inc_stat(&mut needs.energy, steps * 3);
        inc_stat(&mut needs.boredom, steps);
        dec_stat(&mut needs.affection, steps * affection_loss);
    } else {
        dec_stat(&mut needs.hunger, steps * hunger_loss);
        dec_stat(&mut needs.energy, steps * energy_loss);
        dec_stat(&mut needs.hygiene, steps * hygiene_loss);
        inc_stat(&mut needs.boredom, steps * boredom_gain);
        dec_stat(&mut needs.affection, steps * affection_loss);
    }

    sync_conditions(state);

    let score = wellbeing(state);
    let critical = critical_need_count(state);
    if score >= 85 && critical == 0 {
        state.pet.evolution.care_score = state.pet.evolution.care_score.saturating_add(steps * 2);
    } else if score >= 70 && critical <= 1 {
        state.pet.evolution.care_score = state.pet.evolution.care_score.saturating_add(steps);
    }

    if score <= 40 {
        state.pet.evolution.neglect_score = state
            .pet
            .evolution
            .neglect_score
            .saturating_add(steps * 2 + critical);
    } else if critical > 0 {
        state.pet.evolution.neglect_score = state
            .pet
            .evolution
            .neglect_score
            .saturating_add(steps * critical);
    }

    state.semantic.last_active = Utc::now().to_rfc3339();
    let _ = maybe_advance_stage(state);
    sync_state(state);

    serde_json::to_string(state).unwrap_or_default() != before
}

pub fn apply_care_action(
    state: &mut BuddyState,
    action: BuddyCareAction,
    toy: Option<&str>,
) -> (bool, String) {
    let before = serde_json::to_string(state).unwrap_or_default();
    let toy_note = toy
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("toy");
    let (message, activity_type, activity_icon, xp_reward) = match action {
        BuddyCareAction::Feed => {
            inc_stat(&mut state.pet.needs.hunger, 24);
            dec_stat(&mut state.pet.needs.boredom, 6);
            state.pet.evolution.care_score = state.pet.evolution.care_score.saturating_add(4);
            (
                "Snack obtained. Tiny morale boost unlocked.".to_string(),
                "care_feed",
                "🍜",
                2,
            )
        }
        BuddyCareAction::Play => {
            dec_stat(&mut state.pet.needs.boredom, 28);
            inc_stat(&mut state.pet.needs.affection, 10);
            dec_stat(&mut state.pet.needs.energy, 8);
            state.pet.evolution.care_score = state.pet.evolution.care_score.saturating_add(5);
            (
                format!(
                    "Played together with {}. Mischief pressure reduced.",
                    toy_note
                ),
                "care_play",
                "🎾",
                3,
            )
        }
        BuddyCareAction::Pet => {
            inc_stat(&mut state.pet.needs.affection, 18);
            dec_stat(&mut state.pet.needs.boredom, 4);
            state.pet.evolution.care_score = state.pet.evolution.care_score.saturating_add(3);
            (
                "Warm pats received. Confidence and wiggles restored.".to_string(),
                "care_pet",
                "💕",
                2,
            )
        }
        BuddyCareAction::Sleep => {
            state.pet.condition.sleeping = true;
            inc_stat(&mut state.pet.needs.energy, 12);
            state.pet.evolution.care_score = state.pet.evolution.care_score.saturating_add(2);
            (
                "Sleep mode engaged. Dreaming of helpful little victories.".to_string(),
                "care_sleep",
                "😴",
                1,
            )
        }
        BuddyCareAction::Clean => {
            inc_stat(&mut state.pet.needs.hygiene, 28);
            inc_stat(&mut state.pet.needs.affection, 6);
            state.pet.evolution.care_score = state.pet.evolution.care_score.saturating_add(4);
            (
                "Fresh and tidy again. Sparkle levels look much better.".to_string(),
                "care_clean",
                "🧼",
                2,
            )
        }
    };

    state.progression.xp = state.progression.xp.saturating_add(xp_reward);
    state.pet.evolution.neglect_score = state.pet.evolution.neglect_score.saturating_sub(3);
    state.semantic.last_active = Utc::now().to_rfc3339();
    let keep_sleep = matches!(action, BuddyCareAction::Sleep);
    if keep_sleep {
        sync_conditions_keep_sleep(state);
    } else {
        sync_conditions(state);
    }
    let _ = maybe_advance_stage(state);
    sync_state(state);
    if keep_sleep {
        state.pet.condition.sleeping = true;
        sync_semantic(state);
    }

    add_activity(
        state,
        BuddyActivity {
            icon: activity_icon.to_string(),
            title: format!("{}: {}", action.as_str(), message),
            description: message.clone(),
            timestamp: Utc::now().to_rfc3339(),
            activity_type: activity_type.to_string(),
            chat_id: None,
        },
    );

    (
        serde_json::to_string(state).unwrap_or_default() != before,
        message,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_starts_egg() {
        let state = default_buddy_state();

        assert_eq!(state.progression.stage, 0);
        assert_eq!(state.progression.stage_name, "Egg");
        assert_eq!(state.progression.xp, 0);
        assert_eq!(state.progression.level, 1);
        assert_eq!(state.pet.needs.hunger, 80);
        assert_eq!(state.pet.needs.energy, 85);
        assert_eq!(state.pet.needs.hygiene, 80);
        assert_eq!(state.pet.needs.boredom, 15);
        assert_eq!(state.pet.needs.affection, 75);
    }

    #[test]
    fn growth_points_hatch_without_care_gate() {
        let mut state = default_buddy_state();

        grant_xp(&mut state, 20);

        assert_eq!(state.progression.level, 2);
        assert_eq!(state.progression.stage, 1);
        assert_eq!(state.progression.stage_name, "Hatch");
        assert_eq!(state.progression.xp, 0);
    }

    #[test]
    fn xp_only_advances_multiple_stages() {
        let mut state = default_buddy_state();

        grant_xp(&mut state, 20 + 35 + 10);

        assert_eq!(state.progression.stage, 2);
        assert_eq!(state.progression.stage_name, "Sprite");
        assert_eq!(state.progression.level, 3);
        assert_eq!(state.progression.xp, 10);
        assert_eq!(state.progression.xp_next, 55);
    }

    #[test]
    fn care_and_open_time_path_advances_later_stage() {
        let mut state = default_buddy_state();

        apply_pet_tick(&mut state, 60);
        for _ in 0..15 {
            let _ = apply_care_action(&mut state, BuddyCareAction::Play, Some("bug"));
            let _ = apply_care_action(&mut state, BuddyCareAction::Feed, None);
        }

        assert!(state.pet.evolution.open_seconds >= 60);
        assert!(state.pet.evolution.care_score >= 100);
        assert!(state.progression.stage >= 1);
        assert!(state.progression.xp_next > 0);
    }

    #[test]
    fn repeated_successful_workflow_rewards_eventually_advance() {
        let mut state = default_buddy_state();

        for _ in 0..12 {
            grant_xp(&mut state, 5);
        }

        assert!(state.progression.stage >= 2);
        assert_eq!(state.progression.stage_name, "Sprite");
    }

    #[test]
    fn max_stage_behavior() {
        let mut state = default_buddy_state();
        state.pet.evolution.open_seconds = 400 * 60;
        state.pet.evolution.care_score = 500;

        grant_xp(&mut state, 3000);

        assert_eq!(state.progression.stage_name, "Archon");
        assert_eq!(state.progression.stage, 6);
        assert_eq!(state.progression.level, 7);
        assert_eq!(state.progression.xp, 210);
        assert_eq!(state.progression.xp_next, 210);
    }

    #[test]
    fn old_state_migration() {
        let json = r#"{
            "identity": {"name": "Pixel", "created_at": "2024-01-01T00:00:00Z", "palette_index": 2},
            "progression": {"stage": 0, "stage_name": "Egg", "level": 1, "xp": 0, "xp_next": 100},
            "skills": {"unlocked": [], "locked": []},
            "workflow_summaries": [],
            "semantic": {"mood": "Idle", "focus": "", "headline": "", "last_active": "2024-01-01T00:00:00Z"},
            "recent_activities": [],
            "suggestion_state": []
        }"#;
        let state: BuddyState = serde_json::from_str(json).unwrap();

        assert!(!state.onboarding.greeted);
        assert!(!state.onboarding.tour_completed);
        assert!(state.onboarding.first_launch_at.is_empty());
        assert_eq!(state.pet.needs.hunger, 80);
        assert_eq!(state.pet.needs.energy, 85);
        assert_eq!(state.pet.evolution.open_seconds, 0);
        assert!(!state.personality.archetype_label.is_empty());
    }

    #[test]
    fn pet_tick_decays_needs_while_awake() {
        let mut state = default_buddy_state();
        state.personality = BuddyPersonalityProfile::default();

        let changed = apply_pet_tick(&mut state, 15);

        assert!(changed);
        assert_eq!(state.pet.needs.hunger, 78);
        assert_eq!(state.pet.needs.energy, 84);
        assert_eq!(state.pet.needs.hygiene, 79);
        assert_eq!(state.pet.needs.boredom, 17);
        assert_eq!(state.pet.needs.affection, 74);
        assert_eq!(state.pet.evolution.open_seconds, 15);
    }

    #[test]
    fn feed_care_action_restores_hunger() {
        let mut state = default_buddy_state();
        state.pet.needs.hunger = 10;

        let (changed, message) = apply_care_action(&mut state, BuddyCareAction::Feed, None);

        assert!(changed);
        assert!(message.contains("Snack"));
        assert!(state.pet.needs.hunger > 10);
        assert_eq!(state.recent_activities[0].activity_type, "care_feed");
    }

    #[test]
    fn render_persona_block_formats_buddy_identity() {
        let mut state = default_buddy_state();
        state.identity.name = "Pixel".to_string();
        state.personality.archetype_label = "Helper Sprite".to_string();
        state.personality.vibe = "Playful, quirky, helpful".to_string();
        state.personality.summary = "An energetic helper.".to_string();
        state.personality.prompt = "Use warm humor.".to_string();

        assert_eq!(
            render_persona_block(&state),
            "You are Pixel, a Helper Sprite (Playful, quirky, helpful).\nAn energetic helper.\n\nPersonality voice: Use warm humor."
        );
    }

    #[test]
    fn personality_cache_invalidates_on_reroll() {
        let mut state = default_buddy_state();
        let before = persona_cache_version();

        reroll_personality(&mut state);

        assert!(persona_cache_version() > before);
    }
}
