use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::{Notify, RwLock};
use uuid::Uuid;

use crate::storage;
use crate::types::{
    AgentCompletion, AgentListFilter, BackgroundAgent, BgAgentKind, BgAgentStatus,
    CreateAgentRequest,
};

#[derive(Clone)]
pub struct AgentRuntime {
    pub abort_flag: Arc<AtomicBool>,
    pub notify: Arc<Notify>,
}

pub struct BackgroundAgentRegistry {
    records: RwLock<HashMap<String, BackgroundAgent>>,
    runtime: RwLock<HashMap<String, AgentRuntime>>,
    storage_root: PathBuf,
}

impl BackgroundAgentRegistry {
    pub async fn new(storage_root: PathBuf) -> Result<Arc<Self>, String> {
        tokio::fs::create_dir_all(&storage_root)
            .await
            .map_err(|e| format!("Failed to create background agent registry directory: {e}"))?;
        let mut records = storage::load_all(&storage_root).await?;
        reconcile_interrupted(&storage_root, &mut records).await?;
        Ok(Arc::new(Self {
            records: RwLock::new(records),
            runtime: RwLock::new(HashMap::new()),
            storage_root,
        }))
    }

    pub async fn create(
        &self,
        req: CreateAgentRequest,
    ) -> Result<(BackgroundAgent, Arc<AtomicBool>, Arc<Notify>), String> {
        let now = Utc::now();
        let agent_id = format!("bgagent-{}", Uuid::new_v4());
        let target_files = match req.kind {
            BgAgentKind::Subagent => Vec::new(),
            BgAgentKind::Delegate => req.target_files,
        };
        let record = BackgroundAgent {
            schema_version: 1,
            agent_id: agent_id.clone(),
            parent_chat_id: req.parent_chat_id,
            parent_root_chat_id: req.parent_root_chat_id,
            parent_tool_call_id: req.parent_tool_call_id,
            child_chat_id: None,
            kind: req.kind,
            config_name: req.config_name,
            title: req.title,
            prompt: req.prompt,
            target_files,
            status: BgAgentStatus::Queued,
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
            model: req.model,
            created_at: now,
            started_at: None,
            finished_at: None,
            last_update_at: now,
            change_seq: 1,
        };
        let abort_flag = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        {
            let mut records = self.records.write().await;
            storage::save_record(&self.storage_root, &record).await?;
            records.insert(agent_id.clone(), record.clone());
        }
        {
            let mut runtime = self.runtime.write().await;
            runtime.insert(
                agent_id,
                AgentRuntime {
                    abort_flag: abort_flag.clone(),
                    notify: notify.clone(),
                },
            );
        }
        Ok((record, abort_flag, notify))
    }

    pub async fn mark_running(
        &self,
        agent_id: &str,
        child_chat_id: String,
    ) -> Result<BackgroundAgent, String> {
        self.update_record(agent_id, |record, now| {
            record.status = BgAgentStatus::Running;
            record.child_chat_id = Some(child_chat_id);
            record.started_at = Some(now);
            record.finished_at = None;
            record.error = None;
        })
        .await
    }

    pub async fn update_progress(
        &self,
        agent_id: &str,
        progress: String,
        step_count: u32,
        last_activity: Option<String>,
    ) -> Result<BackgroundAgent, String> {
        self.update_record(agent_id, |record, _| {
            record.progress = Some(progress);
            record.step_count = step_count;
            record.last_activity = last_activity;
        })
        .await
    }

    pub async fn mark_completed(
        &self,
        agent_id: &str,
        completion: AgentCompletion,
    ) -> Result<BackgroundAgent, String> {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            if current.status.is_terminal() {
                return Ok(current);
            }
            let AgentCompletion {
                result_summary,
                edited_files,
                diff_summary,
                conflict_summary,
                child_chat_id,
            } = completion;
            let payload = serde_json::json!({
                "agent_id": agent_id,
                "result_summary": result_summary.clone(),
                "edited_files": edited_files.clone(),
                "diff_summary": diff_summary.clone(),
                "conflict_summary": conflict_summary.clone(),
                "child_chat_id": child_chat_id.clone(),
            });
            let result_payload_path =
                storage::save_result_payload(&self.storage_root, agent_id, &payload).await?;
            let mut updated = current;
            let now = Utc::now();
            updated.status = BgAgentStatus::Completed;
            updated.result_summary = Some(result_summary);
            updated.result_payload_path = Some(result_payload_path);
            updated.edited_files = edited_files;
            updated.diff_summary = diff_summary;
            updated.conflict_summary = conflict_summary;
            if child_chat_id.is_some() {
                updated.child_chat_id = child_chat_id;
            }
            updated.error = None;
            updated.finished_at = Some(now);
            touch_record(&mut updated, now);
            storage::save_record(&self.storage_root, &updated).await?;
            records.insert(agent_id.to_string(), updated.clone());
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        Ok(updated)
    }

    pub async fn mark_failed(
        &self,
        agent_id: &str,
        error: String,
    ) -> Result<BackgroundAgent, String> {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            if current.status.is_terminal() {
                return Ok(current);
            }
            let payload = terminal_result_payload("failed", &error, &current);
            let result_payload_path =
                storage::save_result_payload(&self.storage_root, agent_id, &payload).await?;
            let mut updated = current;
            let now = Utc::now();
            updated.status = BgAgentStatus::Failed;
            updated.error = Some(error);
            updated.result_payload_path = Some(result_payload_path);
            updated.finished_at = Some(now);
            touch_record(&mut updated, now);
            storage::save_record(&self.storage_root, &updated).await?;
            records.insert(agent_id.to_string(), updated.clone());
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        Ok(updated)
    }

    pub async fn mark_cancelled(
        &self,
        agent_id: &str,
        reason: Option<String>,
    ) -> Result<BackgroundAgent, String> {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            if current.status.is_terminal() {
                return Ok(current);
            }
            let payload_error = reason
                .clone()
                .unwrap_or_else(|| "Agent was cancelled.".to_string());
            let payload = terminal_result_payload("cancelled", &payload_error, &current);
            let result_payload_path =
                storage::save_result_payload(&self.storage_root, agent_id, &payload).await?;
            let mut updated = current;
            let now = Utc::now();
            updated.status = BgAgentStatus::Cancelled;
            updated.error = reason;
            updated.result_payload_path = Some(result_payload_path);
            updated.finished_at = Some(now);
            touch_record(&mut updated, now);
            storage::save_record(&self.storage_root, &updated).await?;
            records.insert(agent_id.to_string(), updated.clone());
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        Ok(updated)
    }

    pub async fn mark_interrupted(
        &self,
        agent_id: &str,
        reason: String,
    ) -> Result<BackgroundAgent, String> {
        self.update_record(agent_id, |record, now| {
            record.status = BgAgentStatus::Interrupted;
            record.error = Some(reason);
            record.finished_at = Some(now);
        })
        .await
    }

    pub async fn mark_waiting_for_approval(
        &self,
        agent_id: &str,
    ) -> Result<BackgroundAgent, String> {
        self.update_record(agent_id, |record, _| {
            record.status = BgAgentStatus::WaitingForApproval;
        })
        .await
    }

    pub async fn set_completion_message_id(
        &self,
        agent_id: &str,
        message_id: String,
    ) -> Result<(), String> {
        {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            if let Some(current_id) = current.completion_message_id.as_deref() {
                if current_id != "pending" && current_id != "deferred" {
                    return Ok(());
                }
                if current_id == message_id && current_id != "deferred" {
                    return Ok(());
                }
            }
            if matches!(
                (
                    current.completion_message_id.as_deref(),
                    message_id.as_str()
                ),
                (Some("deferred"), "pending")
            ) {
                return Ok(());
            }
            let mut updated = current;
            let now = Utc::now();
            let pushed = message_id != "pending" && message_id != "deferred";
            let deferred = message_id == "deferred";
            updated.completion_message_id = Some(message_id);
            updated.completion_pushed_at = pushed.then_some(now);
            updated.deferred_at = deferred.then_some(now);
            touch_record(&mut updated, now);
            storage::save_record(&self.storage_root, &updated).await?;
            records.insert(agent_id.to_string(), updated);
        }
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        Ok(())
    }

    pub async fn list_for_parent(
        &self,
        parent_chat_id: &str,
        filter: AgentListFilter,
    ) -> Vec<BackgroundAgent> {
        let cutoff = terminal_cutoff(filter.include_terminal_within_hours.unwrap_or(24));
        let mut records: Vec<BackgroundAgent> = self
            .records
            .read()
            .await
            .values()
            .filter(|record| record.parent_chat_id == parent_chat_id)
            .filter(|record| filter.kind.map_or(true, |kind| record.kind == kind))
            .filter(|record| {
                filter
                    .status
                    .as_ref()
                    .map_or(true, |statuses| statuses.contains(&record.status))
            })
            .filter(|record| should_include_record(record, cutoff))
            .cloned()
            .collect();
        records.sort_by(|a, b| {
            b.last_update_at
                .cmp(&a.last_update_at)
                .then(b.created_at.cmp(&a.created_at))
                .then(a.agent_id.cmp(&b.agent_id))
        });
        if let Some(limit) = filter.limit {
            records.truncate(limit);
        }
        records
    }

    pub async fn list_all(&self) -> Vec<BackgroundAgent> {
        let mut records: Vec<BackgroundAgent> =
            self.records.read().await.values().cloned().collect();
        records.sort_by(|a, b| {
            b.last_update_at
                .cmp(&a.last_update_at)
                .then(b.created_at.cmp(&a.created_at))
                .then(a.agent_id.cmp(&b.agent_id))
        });
        records
    }

    pub async fn list_with_completion_message_id(&self, ids: &[&str]) -> Vec<BackgroundAgent> {
        let wanted: HashSet<&str> = ids.iter().copied().collect();
        let mut records: Vec<BackgroundAgent> = self
            .records
            .read()
            .await
            .values()
            .filter(|record| {
                record
                    .completion_message_id
                    .as_deref()
                    .map_or(false, |id| wanted.contains(id))
            })
            .cloned()
            .collect();
        records.sort_by(|a, b| {
            b.last_update_at
                .cmp(&a.last_update_at)
                .then(b.created_at.cmp(&a.created_at))
                .then(a.agent_id.cmp(&b.agent_id))
        });
        records
    }

    pub async fn has_runtime(&self, agent_id: &str) -> bool {
        self.runtime.read().await.contains_key(agent_id)
    }

    #[doc(hidden)]
    pub async fn set_last_update_at_for_test(
        &self,
        agent_id: &str,
        last_update_at: DateTime<Utc>,
    ) -> Result<(), String> {
        let mut records = self.records.write().await;
        let record = records
            .get_mut(agent_id)
            .ok_or_else(|| "agent not found".to_string())?;
        record.last_update_at = last_update_at;
        storage::save_record(&self.storage_root, record).await
    }

    #[doc(hidden)]
    pub async fn set_deferred_at_for_test(
        &self,
        agent_id: &str,
        deferred_at: DateTime<Utc>,
    ) -> Result<(), String> {
        let mut records = self.records.write().await;
        let record = records
            .get_mut(agent_id)
            .ok_or_else(|| "agent not found".to_string())?;
        record.deferred_at = Some(deferred_at);
        storage::save_record(&self.storage_root, record).await
    }

    #[doc(hidden)]
    pub async fn clear_result_summary_for_test(&self, agent_id: &str) -> Result<(), String> {
        let mut records = self.records.write().await;
        let record = records
            .get_mut(agent_id)
            .ok_or_else(|| "agent not found".to_string())?;
        record.result_summary = None;
        storage::save_record(&self.storage_root, record).await
    }

    pub async fn get(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
    ) -> Result<BackgroundAgent, String> {
        let records = self.records.read().await;
        scoped_record(&records, parent_chat_id, agent_id)
    }

    pub async fn wait(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        timeout: Duration,
    ) -> Result<BackgroundAgent, String> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        loop {
            let record = self.get(parent_chat_id, agent_id).await?;
            if record.status.is_terminal() || timeout.is_zero() {
                return Ok(record);
            }
            let Some(runtime) = self.runtime.read().await.get(agent_id).cloned() else {
                return Ok(record);
            };
            let notified = runtime.notify.notified();
            let record = self.get(parent_chat_id, agent_id).await?;
            if record.status.is_terminal() {
                return Ok(record);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(record);
            }
            if tokio::time::timeout(deadline - now, notified)
                .await
                .is_err()
            {
                return self.get(parent_chat_id, agent_id).await;
            }
        }
    }

    pub async fn cancel(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        reason: Option<String>,
    ) -> Result<BackgroundAgent, String> {
        let record = self.get(parent_chat_id, agent_id).await?;
        if record.status.is_terminal() {
            return Ok(record);
        }
        if let Some(abort_flag) = self.abort_flag(agent_id).await {
            abort_flag.store(true, Ordering::SeqCst);
        }
        self.mark_cancelled(agent_id, reason).await
    }

    pub async fn abort_flag(&self, agent_id: &str) -> Option<Arc<AtomicBool>> {
        self.runtime
            .read()
            .await
            .get(agent_id)
            .map(|runtime| runtime.abort_flag.clone())
    }

    pub async fn overlap_warning(
        &self,
        parent_chat_id: &str,
        target_files: &[String],
    ) -> Option<String> {
        let requested: HashSet<String> = target_files
            .iter()
            .map(|path| normalize_path_for_overlap(path))
            .collect();
        if requested.is_empty() {
            return None;
        }
        let records = self.records.read().await;
        let mut overlaps = Vec::new();
        for record in records.values() {
            if record.parent_chat_id != parent_chat_id
                || record.kind != BgAgentKind::Delegate
                || record.status.is_terminal()
            {
                continue;
            }
            let shared: Vec<String> = record
                .target_files
                .iter()
                .filter(|path| requested.contains(&normalize_path_for_overlap(path)))
                .cloned()
                .collect();
            if !shared.is_empty() {
                overlaps.push(format!(
                    "{} ({}) overlaps on {}",
                    record.agent_id,
                    record.title,
                    shared.join(", ")
                ));
            }
        }
        if overlaps.is_empty() {
            None
        } else {
            Some(format!(
                "Running delegate target file overlap detected: {}",
                overlaps.join("; ")
            ))
        }
    }

    async fn update_record<F>(&self, agent_id: &str, update: F) -> Result<BackgroundAgent, String>
    where
        F: FnOnce(&mut BackgroundAgent, DateTime<Utc>),
    {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            let mut updated = current;
            let now = Utc::now();
            update(&mut updated, now);
            touch_record(&mut updated, now);
            storage::save_record(&self.storage_root, &updated).await?;
            records.insert(agent_id.to_string(), updated.clone());
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        Ok(updated)
    }

    async fn notify_for(&self, agent_id: &str) -> Option<Arc<Notify>> {
        self.runtime
            .read()
            .await
            .get(agent_id)
            .map(|runtime| runtime.notify.clone())
    }
}

pub fn normalize_path_for_overlap(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let mut collapsed = String::with_capacity(normalized.len());
    let mut previous_slash = false;
    for ch in normalized.chars() {
        if ch == '/' {
            if !previous_slash {
                collapsed.push(ch);
            }
            previous_slash = true;
        } else {
            collapsed.push(ch);
            previous_slash = false;
        }
    }
    while let Some(stripped) = collapsed.strip_prefix("./") {
        collapsed = stripped.to_string();
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        collapsed.to_lowercase()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        collapsed
    }
}

fn touch_record(record: &mut BackgroundAgent, now: DateTime<Utc>) {
    record.change_seq = record.change_seq.saturating_add(1);
    record.last_update_at = now;
}

fn terminal_result_payload(
    status: &str,
    error: &str,
    record: &BackgroundAgent,
) -> serde_json::Value {
    serde_json::json!({
        "status": status,
        "error": error,
        "edited_files": record.edited_files.clone(),
        "diff_summary": record.diff_summary.clone(),
        "conflict_summary": record.conflict_summary.clone(),
    })
}

fn scoped_record(
    records: &HashMap<String, BackgroundAgent>,
    parent_chat_id: &str,
    agent_id: &str,
) -> Result<BackgroundAgent, String> {
    records
        .get(agent_id)
        .filter(|record| record.parent_chat_id == parent_chat_id)
        .cloned()
        .ok_or_else(|| "agent not found".to_string())
}

fn terminal_cutoff(hours: i64) -> Option<DateTime<Utc>> {
    if hours < 0 {
        return None;
    }
    Utc::now().checked_sub_signed(TimeDelta::hours(hours))
}

fn should_include_record(record: &BackgroundAgent, cutoff: Option<DateTime<Utc>>) -> bool {
    if !record.status.is_terminal() {
        return true;
    }
    match (cutoff, record.finished_at) {
        (Some(cutoff), Some(finished_at)) => finished_at >= cutoff,
        _ => false,
    }
}

async fn reconcile_interrupted(
    storage_root: &Path,
    records: &mut HashMap<String, BackgroundAgent>,
) -> Result<(), String> {
    let now = Utc::now();
    let mut interrupted = Vec::new();
    for record in records.values_mut() {
        if matches!(
            record.status,
            BgAgentStatus::Queued | BgAgentStatus::Running | BgAgentStatus::WaitingForApproval
        ) {
            record.status = BgAgentStatus::Interrupted;
            record.finished_at = Some(now);
            record.last_update_at = now;
            record.change_seq = record.change_seq.saturating_add(1);
            record.error = Some(
                "Engine restarted before agent finished. True resume is not supported.".to_string(),
            );
            interrupted.push(record.clone());
        }
    }
    for record in interrupted {
        storage::save_record(storage_root, &record).await?;
    }
    Ok(())
}
