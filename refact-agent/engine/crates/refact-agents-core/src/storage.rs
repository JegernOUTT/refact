use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use tokio::fs;

use crate::types::{BackgroundAgent, BgAgentStatus};

const RECORDS_FILE: &str = "records.json";
const RESULTS_DIR: &str = "results";
pub const TERMINAL_RETENTION_DAYS: i64 = 7;
pub const MAX_RECORDS: usize = 2000;

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

fn record_terminal_timestamp(record: &BackgroundAgent) -> Option<DateTime<Utc>> {
    let finished = match record.status {
        BgAgentStatus::Completed
        | BgAgentStatus::Failed
        | BgAgentStatus::Cancelled
        | BgAgentStatus::Interrupted => record.finished_at.or(Some(record.last_update_at)),
        _ => None,
    };
    finished
}

fn prune_records(records: &mut Vec<BackgroundAgent>, now: DateTime<Utc>) {
    let cutoff = now - Duration::days(TERMINAL_RETENTION_DAYS);
    records.retain(|record| {
        if !record.status.is_terminal() {
            return true;
        }
        record_terminal_timestamp(record)
            .map(|timestamp| timestamp >= cutoff)
            .unwrap_or(false)
    });
    if records.len() <= MAX_RECORDS {
        return;
    }
    records.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then(a.agent_id.cmp(&b.agent_id))
    });
    let mut excess = records.len().saturating_sub(MAX_RECORDS);
    let mut retained = Vec::with_capacity(records.len().saturating_sub(excess));
    for record in records.drain(..) {
        if excess > 0 && record.status.is_terminal() {
            excess -= 1;
            continue;
        }
        retained.push(record);
    }
    *records = retained;
}

pub async fn save_all(
    storage_root: &Path,
    mut records: Vec<BackgroundAgent>,
) -> Result<(), String> {
    prune_records(&mut records, Utc::now());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(
        agent_id: &str,
        status: BgAgentStatus,
        created_at: DateTime<Utc>,
        finished_at: Option<DateTime<Utc>>,
    ) -> BackgroundAgent {
        BackgroundAgent {
            schema_version: 1,
            agent_id: agent_id.to_string(),
            parent_chat_id: "parent".to_string(),
            parent_root_chat_id: None,
            parent_tool_call_id: None,
            child_chat_id: None,
            kind: crate::types::BgAgentKind::Subagent,
            config_name: "cfg".to_string(),
            title: "title".to_string(),
            prompt: "prompt".to_string(),
            target_files: Vec::new(),
            status,
            progress: None,
            step_count: 0,
            last_activity: None,
            result_summary: None,
            result_payload_path: None,
            error: None,
            edited_files: Vec::new(),
            diff_summary: None,
            conflict_summary: None,
            completion_message_id: None,
            completion_pushed_at: None,
            deferred_at: None,
            model: "model".to_string(),
            model_type: None,
            current_tool: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
            merge_status: None,
            questions: Vec::new(),
            tokens_used: 0,
            cost_usd: None,
            created_at,
            started_at: None,
            finished_at,
            last_update_at: finished_at.unwrap_or(created_at),
            change_seq: 1,
        }
    }

    #[test]
    fn prune_records_keeps_live_and_recent_terminal_records() {
        let now = Utc::now();
        let mut records = vec![
            make_record(
                "running",
                BgAgentStatus::Running,
                now - Duration::days(30),
                None,
            ),
            make_record(
                "recent-terminal",
                BgAgentStatus::Completed,
                now - Duration::days(2),
                Some(now - Duration::days(1)),
            ),
            make_record(
                "old-terminal",
                BgAgentStatus::Failed,
                now - Duration::days(20),
                Some(now - Duration::days(10)),
            ),
        ];

        prune_records(&mut records, now);

        assert_eq!(records.len(), 2);
        assert!(records.iter().any(|record| record.agent_id == "running"));
        assert!(records
            .iter()
            .any(|record| record.agent_id == "recent-terminal"));
        assert!(!records
            .iter()
            .any(|record| record.agent_id == "old-terminal"));
    }

    #[test]
    fn prune_records_caps_oldest_terminal_records_first() {
        let now = Utc::now();
        let mut records = Vec::new();
        records.push(make_record("live", BgAgentStatus::Running, now, None));
        for index in 0..(MAX_RECORDS + 20) {
            let created_at = now - Duration::minutes((MAX_RECORDS + 20 - index) as i64);
            records.push(make_record(
                &format!("terminal-{index}"),
                BgAgentStatus::Completed,
                created_at,
                Some(created_at),
            ));
        }

        prune_records(&mut records, now);

        assert_eq!(records.len(), MAX_RECORDS);
        assert!(records.iter().any(|record| record.agent_id == "live"));
        assert!(!records.iter().any(|record| record.agent_id == "terminal-0"));
        assert!(records
            .iter()
            .any(|record| record.agent_id == "terminal-21"));
    }
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
