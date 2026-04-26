use std::path::Path;
use chrono::Utc;
use rand::Rng;
use tokio::fs;
use tracing::warn;

use super::types::{
    BuddyActivity, BuddyIdentity, BuddyOnboarding, BuddyProgression, BuddySemanticSnapshot,
    BuddySkillLedger, BuddyState,
};

const BUDDY_NAMES: &[&str] = &[
    "Pixel", "Byte", "Spark", "Nova", "Echo",
    "Chip", "Flux", "Glow", "Dash", "Zen",
];

const STAGE_THRESHOLDS: &[(u64, &str)] = &[
    (0, "Egg"), (30, "Hatch"), (100, "Sprite"), (300, "Imp"),
    (700, "Daemon"), (1500, "Sage"), (3000, "Archon"),
];

pub fn default_buddy_state() -> BuddyState {
    let now = Utc::now().to_rfc3339();
    let mut rng = rand::thread_rng();
    let name = BUDDY_NAMES[rng.gen_range(0..BUDDY_NAMES.len())].to_string();
    let palette_index = rng.gen_range(0..7usize);
    BuddyState {
        identity: BuddyIdentity {
            name,
            created_at: now.clone(),
            palette_index,
        },
        progression: BuddyProgression {
            stage: 0,
            stage_name: "Egg".to_string(),
            level: 1,
            xp: 0,
            xp_next: 100,
        },
        skills: BuddySkillLedger {
            unlocked: vec![],
            locked: vec![],
        },
        workflow_summaries: vec![],
        semantic: BuddySemanticSnapshot {
            mood: "Idle".to_string(),
            focus: "".to_string(),
            headline: "".to_string(),
            last_active: now.clone(),
        },
        recent_activities: vec![],
        suggestion_state: vec![],
        onboarding: BuddyOnboarding {
            first_launch_at: now.clone(),
            ..Default::default()
        },
    }
}

fn compute_cumulative_xp(p: &BuddyProgression) -> u64 {
    let mut total = p.xp;
    for lvl in 1..p.level {
        total += 100 * lvl as u64;
    }
    total
}

fn stage_for_cumulative(cumulative: u64) -> (u32, &'static str) {
    let mut stage = 0u32;
    let mut name = "Egg";
    for (i, (threshold, stage_name)) in STAGE_THRESHOLDS.iter().enumerate() {
        if cumulative >= *threshold {
            stage = i as u32;
            name = stage_name;
        }
    }
    (stage, name)
}

pub async fn load_state(project_root: &Path) -> BuddyState {
    let path = project_root.join(".refact/buddy/state.json");
    match fs::read_to_string(&path).await {
        Ok(content) => match serde_json::from_str::<BuddyState>(&content) {
            Ok(mut s) => {
                let cumulative = compute_cumulative_xp(&s.progression);
                let (stage, name) = stage_for_cumulative(cumulative);
                s.progression.stage = stage;
                s.progression.stage_name = name.to_string();
                s
            }
            Err(e) => {
                warn!("Failed to parse buddy state: {}, using defaults", e);
                default_buddy_state()
            }
        },
        Err(_) => default_buddy_state(),
    }
}

pub async fn save_state(project_root: &Path, state: &BuddyState) -> Result<(), String> {
    let path = project_root.join(".refact/buddy/state.json");
    super::storage::atomic_write_json(&path, state).await
}

pub fn add_activity(state: &mut BuddyState, activity: BuddyActivity) {
    state.recent_activities.insert(0, activity);
    state.recent_activities.truncate(50);
}

pub fn grant_xp(state: &mut BuddyState, amount: u64) {
    state.progression.xp += amount;
    while state.progression.xp >= state.progression.xp_next {
        state.progression.xp -= state.progression.xp_next;
        state.progression.level += 1;
        state.progression.xp_next = 100 * state.progression.level as u64;
    }
    let cumulative = compute_cumulative_xp(&state.progression);
    let (stage, name) = stage_for_cumulative(cumulative);
    state.progression.stage = stage;
    state.progression.stage_name = name.to_string();
}
