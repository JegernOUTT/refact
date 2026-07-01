use serde::Deserialize;

#[derive(Debug, PartialEq, PartialOrd)]
pub enum FilePrivacyLevel {
    Blocked = 0,
    OnlySendToServersIControl = 1,
    AllowToSendAnywhere = 2,
}

#[derive(Debug, Deserialize)]
pub struct PrivacySettings {
    pub privacy_rules: FilePrivacySettings,
    #[serde(default)]
    pub loaded_ts: u64,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
pub struct FilePrivacySettings {
    pub only_send_to_servers_I_control: Vec<String>,
    pub blocked: Vec<String>,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        PrivacySettings {
            privacy_rules: FilePrivacySettings {
                blocked: vec!["*".to_string()],
                only_send_to_servers_I_control: vec![],
            },
            loaded_ts: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_level_order_allows_minimum_comparison() {
        assert!(FilePrivacyLevel::Blocked < FilePrivacyLevel::OnlySendToServersIControl);
        assert!(
            FilePrivacyLevel::OnlySendToServersIControl < FilePrivacyLevel::AllowToSendAnywhere
        );
    }

    #[test]
    fn default_privacy_settings_block_all_paths() {
        let settings = PrivacySettings::default();
        assert_eq!(settings.privacy_rules.blocked, vec!["*".to_string()]);
        assert!(settings
            .privacy_rules
            .only_send_to_servers_I_control
            .is_empty());
        assert_eq!(settings.loaded_ts, 0);
    }
}
