use std::path::Path;

use tokio::fs;
use tracing::warn;

pub use refact_buddy_core::settings::*;
pub use refact_buddy_core::snapshot::BuddyStorageMetadata;

pub fn storage_metadata(project_root: &Path) -> BuddyStorageMetadata {
    let buddy_dir = project_root.join(".refact").join("buddy");
    let settings_path = buddy_dir.join("settings.json");
    BuddyStorageMetadata {
        project_root: project_root.to_string_lossy().to_string(),
        buddy_dir: buddy_dir.to_string_lossy().to_string(),
        settings_path: settings_path.to_string_lossy().to_string(),
    }
}

pub async fn load_settings(project_root: &Path) -> BuddySettings {
    let path = project_root
        .join(".refact")
        .join("buddy")
        .join("settings.json");
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

pub static SETTINGS_WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn settings_snapshot(root: &Path) -> Result<Option<Vec<u8>>, String> {
    let path = root.join(".refact/buddy/settings.json");
    for candidate in [
        root.to_path_buf(),
        root.join(".refact"),
        root.join(".refact/buddy"),
        path.clone(),
    ] {
        match fs::symlink_metadata(candidate).await {
            Ok(m) if m.file_type().is_symlink() => {
                return Err("settings symlink not allowed".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("settings snapshot failed".into()),
        }
    }
    match fs::read(path).await {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("settings snapshot failed".into()),
    }
}

pub async fn settings_compare_exchange(
    root: &Path,
    expected: &Option<Vec<u8>>,
    replacement: &Option<Vec<u8>>,
) -> Result<(), String> {
    let _guard = SETTINGS_WRITE_LOCK.lock().await;
    if &settings_snapshot(root).await? != expected {
        return Err("settings version mismatch".into());
    }
    let path = root.join(".refact/buddy/settings.json");
    if let Some(bytes) = replacement {
        serde_json::from_slice::<BuddySettings>(bytes).map_err(|_| "invalid buddy settings")?;
        use tokio::io::AsyncWriteExt;
        fs::create_dir_all(path.parent().unwrap())
            .await
            .map_err(|_| "settings persistence failed")?;
        let tmp = path.with_file_name(format!(".settings-{}.tmp", uuid::Uuid::new_v4()));
        let result = async {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&tmp).await?;
            file.write_all(bytes).await?;
            file.sync_all().await?;
            fs::rename(&tmp, path).await
        }
        .await;
        if result.is_err() {
            let _ = fs::remove_file(tmp).await;
            return Err("settings persistence failed".into());
        }
    } else {
        match fs::remove_file(path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("settings persistence failed".into()),
        }
    }
    Ok(())
}

fn merge_known(target: &mut serde_json::Value, source: serde_json::Value) {
    if let (Some(target), Some(source)) = (target.as_object_mut(), source.as_object()) {
        for (key, value) in source {
            if value.is_object() && target.get(key).is_some_and(|v| v.is_object()) {
                merge_known(target.get_mut(key).unwrap(), value.clone());
            } else {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

pub async fn save_settings(project_root: &Path, settings: &BuddySettings) -> Result<(), String> {
    let _guard = SETTINGS_WRITE_LOCK.lock().await;
    let path = project_root.join(".refact/buddy/settings.json");
    let mut document = match settings_snapshot(project_root).await? {
        Some(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|_| "invalid buddy settings")?,
        None => serde_json::json!({}),
    };
    if !document.is_object() {
        return Err("invalid buddy settings".into());
    }
    merge_known(
        &mut document,
        serde_json::to_value(settings).map_err(|_| "invalid buddy settings")?,
    );
    fs::create_dir_all(path.parent().unwrap())
        .await
        .map_err(|_| "settings persistence failed")?;
    super::storage::atomic_write_json(&path, &document).await
}
