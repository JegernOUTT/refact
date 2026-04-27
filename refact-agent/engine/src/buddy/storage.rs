use std::path::Path;
use serde::Serialize;
use tokio::fs;

use super::state::default_buddy_state;

const DEFAULT_MAIN_PROMPT: &str = "You are Buddy, a persistent project companion inside Refact.\nYou help with code tasks, project setup, diagnostics, and keeping things running smoothly.\nYou are friendly, concise, and focused on being genuinely useful.\n";

pub async fn atomic_write_json<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string(data).map_err(|e| e.to_string())?;
    fs::write(&tmp_path, &json)
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)
            .await
            .map_err(|e| format!("Failed to remove existing file: {}", e))?;
    }
    fs::rename(&tmp_path, path)
        .await
        .map_err(|e| format!("Failed to rename: {}", e))
}

pub async fn bootstrap_buddy_storage(project_root: &Path) -> Result<(), String> {
    let buddy_dir = project_root.join(".refact/buddy");
    let dirs = [
        buddy_dir.clone(),
        buddy_dir.join("skills"),
        buddy_dir.join("chats/conversations"),
        buddy_dir.join("chats/workflows"),
    ];
    for dir in &dirs {
        fs::create_dir_all(dir)
            .await
            .map_err(|e| format!("Failed to create dir {:?}: {}", dir, e))?;
    }
    let state_path = buddy_dir.join("state.json");
    if !state_path.exists() {
        let state = default_buddy_state();
        atomic_write_json(&state_path, &state).await?;
    }
    let settings_path = buddy_dir.join("settings.json");
    if !settings_path.exists() {
        let settings = super::settings::BuddySettings::default();
        atomic_write_json(&settings_path, &settings).await?;
    }
    let prompt_path = buddy_dir.join("main_prompt.md");
    if !prompt_path.exists() {
        fs::write(&prompt_path, DEFAULT_MAIN_PROMPT)
            .await
            .map_err(|e| format!("Failed to write main_prompt.md: {}", e))?;
    }
    Ok(())
}

