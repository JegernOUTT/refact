use serde::{Deserialize, Serialize};
use super::settings::BuddySettings;
use super::types::{BuddyRuntimeEvent, BuddyState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddySnapshot {
    pub state: BuddyState,
    pub settings: BuddySettings,
    pub enabled: bool,
    pub runtime_queue: Vec<BuddyRuntimeEvent>,
    pub now_playing: Option<BuddyRuntimeEvent>,
}
