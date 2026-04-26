use std::path::Path;
use chrono::Utc;
use tokio::fs;
use tracing::warn;

use super::types::{
    BuddyActivity, BuddyIdentity, BuddyProgression, BuddySemanticSnapshot,
    BuddySkillLedger, BuddyState,
};

pub fn default_buddy_state() -> BuddyState {
    let now = Utc::now().to_rfc3339();
    BuddyState {
        identity: BuddyIdentity {
            name: "Pixel".to_string(),
            created_at: now.clone(),
            palette_index: 0,
        },
        progression: BuddyProgression {
            stage: 1,
            stage_name: "Sprite".to_string(),
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
            last_active: now,
        },
        recent_activities: vec![],
        suggestion_state: vec![],
    }
}

pub async fn load_state(project_root: &Path) -> BuddyState {
    let path = project_root.join(".refact/buddy/state.json");
    match fs::read_to_string(&path).await {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(s) => s,
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
}
