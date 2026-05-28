use std::path::Path;

use tokio::fs;
use tracing::warn;

pub use refact_buddy_core::settings::*;

pub async fn load_settings(project_root: &Path) -> BuddySettings {
    let path = project_root.join(".refact/buddy/settings.json");
    match fs::read_to_string(&path).await {
        Ok(content) => match serde_json::from_str::<BuddySettings>(&content) {
            Ok(mut settings) => {
                if settings.daily_digest_hour.is_some_and(|hour| hour > 23) {
                    warn!("Invalid buddy daily_digest_hour in persisted settings, clearing value");
                    settings.daily_digest_hour = None;
                }
                settings
            }
            Err(e) => {
                warn!("Failed to parse buddy settings: {}, using defaults", e);
                BuddySettings::default()
            }
        },
        Err(_) => BuddySettings::default(),
    }
}

pub async fn save_settings(project_root: &Path, settings: &BuddySettings) -> Result<(), String> {
    let path = project_root.join(".refact/buddy/settings.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create dir {:?}: {}", parent, e))?;
    }
    super::storage::atomic_write_json(&path, settings).await
}
