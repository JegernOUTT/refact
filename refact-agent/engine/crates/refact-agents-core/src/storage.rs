use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tokio::fs;

use crate::types::BackgroundAgent;

const RECORDS_FILE: &str = "records.json";
const RESULTS_DIR: &str = "results";

async fn atomic_write_file(tmp_path: &Path, dest_path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if dest_path.exists() {
        fs::remove_file(dest_path)
            .await
            .map_err(|e| format!("Failed to remove existing file: {e}"))?;
    }
    fs::rename(tmp_path, dest_path)
        .await
        .map_err(|e| format!("Failed to rename: {e}"))
}

async fn save_all(storage_root: &Path, records: Vec<BackgroundAgent>) -> Result<(), String> {
    fs::create_dir_all(storage_root)
        .await
        .map_err(|e| format!("Failed to create background agents directory: {e}"))?;
    let records_path = storage_root.join(RECORDS_FILE);
    let tmp_path = storage_root.join(format!("{RECORDS_FILE}.tmp"));
    let content = serde_json::to_string_pretty(&records)
        .map_err(|e| format!("Failed to serialize background agents: {e}"))?;
    fs::write(&tmp_path, content)
        .await
        .map_err(|e| format!("Failed to write background agents file: {e}"))?;
    atomic_write_file(&tmp_path, &records_path).await
}

pub async fn load_all(storage_root: &Path) -> Result<HashMap<String, BackgroundAgent>, String> {
    let records_path = storage_root.join(RECORDS_FILE);
    if !records_path.exists() {
        return Ok(HashMap::new());
    }
    let content = fs::read_to_string(&records_path)
        .await
        .map_err(|e| format!("Failed to read background agents file: {e}"))?;
    if content.trim().is_empty() {
        return Ok(HashMap::new());
    }
    let records: Vec<BackgroundAgent> = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse background agents file: {e}"))?;
    Ok(records
        .into_iter()
        .map(|record| (record.agent_id.clone(), record))
        .collect())
}

pub async fn save_record(storage_root: &Path, record: &BackgroundAgent) -> Result<(), String> {
    let mut records = load_all(storage_root).await?;
    records.insert(record.agent_id.clone(), record.clone());
    let mut ordered: Vec<BackgroundAgent> = records.into_values().collect();
    ordered.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then(a.agent_id.cmp(&b.agent_id))
    });
    save_all(storage_root, ordered).await
}

pub async fn save_result_payload(
    storage_root: &Path,
    agent_id: &str,
    payload: &serde_json::Value,
) -> Result<PathBuf, String> {
    let results_dir = storage_root.join(RESULTS_DIR);
    fs::create_dir_all(&results_dir)
        .await
        .map_err(|e| format!("Failed to create background agent results directory: {e}"))?;
    let result_path = results_dir.join(format!("{agent_id}.json"));
    let tmp_path = results_dir.join(format!("{agent_id}.json.tmp"));
    let content = serde_json::to_string_pretty(payload)
        .map_err(|e| format!("Failed to serialize background agent result: {e}"))?;
    fs::write(&tmp_path, content)
        .await
        .map_err(|e| format!("Failed to write background agent result: {e}"))?;
    atomic_write_file(&tmp_path, &result_path).await?;
    Ok(result_path)
}
