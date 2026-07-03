use serde::{Deserialize, Serialize};

pub const MAX_PALETTE_INDEX: usize = 7;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HumorLevel {
    Off,
    Light,
    Normal,
}

impl Default for HumorLevel {
    fn default() -> Self {
        Self::Light
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuietHoursMode {
    Off,
    Auto,
    Fixed,
}

impl Default for QuietHoursMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyLevel {
    ReadOnly,
    Suggest,
    Propose,
    SafeAuto,
}

impl Default for AutonomyLevel {
    fn default() -> Self {
        Self::Propose
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserverToggles {
    #[serde(default = "default_true")]
    pub task_health: bool,
    #[serde(default = "default_true")]
    pub trajectory_clutter: bool,
    #[serde(default = "default_true")]
    pub chat_pattern: bool,
    #[serde(default = "default_true")]
    pub customization_drift: bool,
    #[serde(default = "default_true")]
    pub memory_garden: bool,
    #[serde(default = "default_true")]
    pub mcp_auth: bool,
    #[serde(default = "default_true")]
    pub git_pressure: bool,
    #[serde(default = "default_true")]
    pub diagnostic_cluster: bool,
    #[serde(default = "default_true")]
    pub provider_health: bool,
}

impl Default for ObserverToggles {
    fn default() -> Self {
        Self {
            task_health: true,
            trajectory_clutter: true,
            chat_pattern: true,
            customization_drift: true,
            memory_garden: true,
            mcp_auth: true,
            git_pressure: true,
            diagnostic_cluster: true,
            provider_health: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddySettings {
    pub enabled: bool,
    pub auto_diagnostics: bool,
    pub auto_issue_creation: bool,
    pub personality_prompt: Option<String>,
    #[serde(default = "default_true")]
    pub autonomous_chats_enabled: bool,
    #[serde(default = "default_true")]
    pub proactive_enabled: bool,
    #[serde(default = "default_true")]
    pub message_observation_enabled: bool,
    #[serde(default = "default_true")]
    pub chat_reactions_enabled: bool,
    #[serde(default = "default_true")]
    pub housekeeping_enabled: bool,
    #[serde(default = "default_true")]
    pub humor_enabled: bool,
    #[serde(default)]
    pub humor_level: HumorLevel,
    #[serde(default)]
    pub autonomy_level: AutonomyLevel,
    #[serde(default)]
    pub quiet_mode: bool,
    #[serde(default = "default_daily_digest_hour")]
    pub daily_digest_hour: Option<u8>,
    #[serde(default)]
    pub quiet_hours_mode: QuietHoursMode,
    #[serde(default = "default_quiet_start")]
    pub quiet_hours_start: u8,
    #[serde(default = "default_quiet_end")]
    pub quiet_hours_end: u8,
    #[serde(default)]
    pub muted_intents: Vec<String>,
    #[serde(default)]
    pub muted_chat_ids: Vec<String>,
    #[serde(default)]
    pub daily_llm_token_budget: Option<u64>,
    #[serde(default)]
    pub observers: ObserverToggles,
}

fn default_true() -> bool {
    true
}

fn default_daily_digest_hour() -> Option<u8> {
    Some(18)
}

fn default_quiet_start() -> u8 {
    22
}

fn default_quiet_end() -> u8 {
    8
}

impl Default for BuddySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_diagnostics: true,
            auto_issue_creation: false,
            personality_prompt: None,
            autonomous_chats_enabled: true,
            proactive_enabled: true,
            message_observation_enabled: true,
            chat_reactions_enabled: true,
            housekeeping_enabled: true,
            humor_enabled: true,
            humor_level: HumorLevel::default(),
            autonomy_level: AutonomyLevel::default(),
            quiet_mode: false,
            daily_digest_hour: Some(18),
            quiet_hours_mode: QuietHoursMode::default(),
            quiet_hours_start: default_quiet_start(),
            quiet_hours_end: default_quiet_end(),
            muted_intents: Vec::new(),
            muted_chat_ids: Vec::new(),
            daily_llm_token_budget: None,
            observers: ObserverToggles::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_get_proactive_default() {
        let json = r#"{"enabled": true, "auto_diagnostics": true, "auto_issue_creation": false}"#;
        let settings: BuddySettings = serde_json::from_str(json).unwrap();

        assert!(settings.proactive_enabled);
        assert!(settings.message_observation_enabled);
        assert!(settings.chat_reactions_enabled);
        assert!(settings.observers.chat_pattern);
        assert_eq!(settings.quiet_hours_mode, QuietHoursMode::Auto);
        assert_eq!(settings.quiet_hours_start, 22);
        assert_eq!(settings.quiet_hours_end, 8);
        assert!(settings.muted_intents.is_empty());
        assert!(settings.muted_chat_ids.is_empty());
        assert_eq!(settings.daily_llm_token_budget, None);
    }

    #[test]
    fn settings_default_observer_toggles() {
        let settings = BuddySettings::default();

        assert!(settings.observers.chat_pattern);
        assert!(settings.observers.task_health);
        assert!(settings.observers.trajectory_clutter);
        assert!(settings.observers.customization_drift);
        assert!(settings.observers.memory_garden);
        assert!(settings.observers.mcp_auth);
        assert!(settings.observers.git_pressure);
        assert!(settings.observers.diagnostic_cluster);
        assert!(settings.observers.provider_health);
    }

    #[test]
    fn humor_level_and_autonomy_serde() {
        let humor = HumorLevel::Light;
        let json = serde_json::to_string(&humor).unwrap();
        let back: HumorLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(humor, back);

        let autonomy = AutonomyLevel::Suggest;
        let json = serde_json::to_string(&autonomy).unwrap();
        let back: AutonomyLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(autonomy, back);

        let settings = BuddySettings::default();
        assert_eq!(settings.humor_level, HumorLevel::Light);
        assert_eq!(settings.autonomy_level, AutonomyLevel::Propose);
        let propose_json = serde_json::to_string(&AutonomyLevel::Propose).unwrap();
        assert_eq!(propose_json, "\"propose\"");
    }
}
