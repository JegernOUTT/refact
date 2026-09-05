use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::{Mutex, Notify, RwLock};
use uuid::Uuid;
use refact_chat_api::{DeliveryOutcome, PendingDelivery, PushMode};

use crate::storage;
use crate::types::{
    AgentCompletion, AgentListFilter, AgentQuestion, BackgroundAgent, BgAgentStatus,
    CreateAgentRequest,
};
#[cfg(test)]
use crate::types::BgAgentKind;

const MAX_INBOX_MESSAGES: usize = 100;

#[derive(Debug, Clone)]
pub struct InboxMessage {
    pub from: String,
    pub text: String,
    pub queued_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct AgentRuntime {
    pub abort_flag: Arc<AtomicBool>,
    pub interrupt_notify: Arc<Notify>,
    pub notify: Arc<Notify>,
    pub inbox: Arc<Mutex<VecDeque<InboxMessage>>>,
    delivery_state: Arc<Mutex<RunnerDeliveryState>>,
}

#[derive(Default)]
struct RunnerDeliveryState {
    claimed: HashSet<String>,
    sealed: bool,
}

pub struct BackgroundAgentRegistry {
    records: RwLock<HashMap<String, BackgroundAgent>>,
    runtime: RwLock<HashMap<String, AgentRuntime>>,
    storage_root: PathBuf,
    write_state: Mutex<RegistryWriteState>,
    storage_write: Mutex<()>,
}

struct RegistryWriteState {
    pending: bool,
    last_write: Option<Instant>,
    flush_task: Option<tokio::task::JoinHandle<()>>,
}

impl Default for RegistryWriteState {
    fn default() -> Self {
        Self {
            pending: false,
            last_write: None,
            flush_task: None,
        }
    }
}

const WRITE_DEBOUNCE: Duration = Duration::from_secs(10);

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
            write_state: Mutex::new(RegistryWriteState::default()),
            storage_write: Mutex::new(()),
        }))
    }

    pub async fn create(
        &self,
        req: CreateAgentRequest,
    ) -> Result<(BackgroundAgent, Arc<AtomicBool>, Arc<Notify>), String> {
        let now = Utc::now();
        let agent_id = format!("bgagent-{}", Uuid::new_v4());
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
            target_files: req.target_files,
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
            completion_push: PushMode::Append,
            pending_deliveries: Vec::new(),
            delivery_ids: Vec::new(),
            deferred_at: None,
            model: req.model,
            model_type: req.model_type,
            current_tool: None,
            goal_summary: req.goal_summary,
            plan_present: req.plan_present,
            worktree_id: req.worktree_id,
            worktree_branch: req.worktree_branch,
            merge_status: None,
            questions: Vec::new(),
            tokens_used: 0,
            cost_usd: None,
            created_at: now,
            started_at: None,
            finished_at: None,
            last_update_at: now,
            change_seq: 1,
        };
        let abort_flag = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        let inbox = Arc::new(Mutex::new(VecDeque::new()));
        {
            let mut records = self.records.write().await;
            records.insert(agent_id.clone(), record.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
        }
        {
            let mut runtime = self.runtime.write().await;
            runtime.insert(
                agent_id,
                AgentRuntime {
                    abort_flag: abort_flag.clone(),
                    interrupt_notify: Arc::new(Notify::new()),
                    notify: notify.clone(),
                    inbox,
                    delivery_state: Arc::new(Mutex::new(RunnerDeliveryState::default())),
                },
            );
        }
        Ok((record, abort_flag, notify))
    }

    pub async fn mark_running(
        self: &Arc<Self>,
        agent_id: &str,
        child_chat_id: String,
    ) -> Result<BackgroundAgent, String> {
        self.update_record_if_not_terminal(agent_id, false, |record, now| {
            record.status = BgAgentStatus::Running;
            record.child_chat_id = Some(child_chat_id);
            record.started_at = Some(now);
            record.finished_at = None;
            record.error = None;
            Ok(())
        })
        .await
    }

    pub async fn update_progress(
        self: &Arc<Self>,
        agent_id: &str,
        progress: String,
        step_count: u32,
    ) -> Result<BackgroundAgent, String> {
        self.update_record_if_not_terminal(agent_id, true, |record, now| {
            record.progress = Some(progress);
            record.step_count = step_count;
            record.last_activity = Some(now.to_rfc3339());
            Ok(())
        })
        .await
    }

    pub async fn update_activity(
        self: &Arc<Self>,
        agent_id: &str,
        progress: Option<String>,
        step_count: Option<u32>,
        current_tool: Option<Option<String>>,
    ) -> Result<BackgroundAgent, String> {
        self.update_record_if_not_terminal(agent_id, true, |record, now| {
            if let Some(progress) = progress {
                record.progress = Some(progress);
            }
            if let Some(step_count) = step_count {
                record.step_count = step_count;
            }
            if let Some(current_tool) = current_tool {
                record.current_tool = current_tool;
            }
            record.last_activity = Some(now.to_rfc3339());
            Ok(())
        })
        .await
    }

    pub async fn add_usage(
        &self,
        agent_id: &str,
        tokens_delta: u64,
        cost_delta_usd: Option<f64>,
    ) -> Result<BackgroundAgent, String> {
        self.update_record(agent_id, |record, _| {
            record.tokens_used = record.tokens_used.saturating_add(tokens_delta);
            if let Some(cost_delta_usd) = cost_delta_usd {
                record.cost_usd = Some(record.cost_usd.unwrap_or_default() + cost_delta_usd);
            }
            Ok(())
        })
        .await
    }

    pub async fn set_merge_status(
        &self,
        agent_id: &str,
        status: &str,
    ) -> Result<BackgroundAgent, String> {
        self.set_merge_outcome(agent_id, status, None, None).await
    }

    pub async fn set_merge_outcome(
        &self,
        agent_id: &str,
        status: &str,
        conflict_summary: Option<String>,
        error: Option<String>,
    ) -> Result<BackgroundAgent, String> {
        if !matches!(
            status,
            "pending" | "merged" | "conflict" | "skipped" | "failed"
        ) {
            return Err("invalid merge status".to_string());
        }
        let status = status.to_string();
        self.update_record(agent_id, |record, _| {
            record.merge_status = Some(status);
            if let Some(conflict_summary) = conflict_summary {
                record.conflict_summary = Some(conflict_summary);
            }
            if let Some(error) = error {
                record.error = Some(match record.error.take() {
                    Some(existing) => format!("{}\n{}", existing, error),
                    None => error,
                });
            }
            Ok(())
        })
        .await
    }

    pub async fn add_question(
        &self,
        agent_id: &str,
        text: String,
    ) -> Result<(BackgroundAgent, String), String> {
        let question_id = Uuid::new_v4().simple().to_string()[..8].to_string();
        let updated = self
            .update_record(agent_id, |record, now| {
                if record.status.is_terminal() {
                    return Err("agent already finished".to_string());
                }
                record.questions.push(AgentQuestion {
                    id: question_id.clone(),
                    text,
                    asked_at: now,
                    answer: None,
                    answered_at: None,
                });
                Ok(())
            })
            .await?;
        Ok((updated, question_id))
    }

    pub async fn answer_question(
        &self,
        agent_id: &str,
        question_id: &str,
        answer: String,
    ) -> Result<BackgroundAgent, String> {
        let question_id = question_id.to_string();
        self.update_record(agent_id, |record, now| {
            let question = record
                .questions
                .iter_mut()
                .find(|question| question.id == question_id)
                .ok_or_else(|| "question not found".to_string())?;
            if question.answer.is_some() {
                return Err("question already answered".to_string());
            }
            question.answer = Some(answer);
            question.answered_at = Some(now);
            Ok(())
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
            updated.current_tool = None;
            if child_chat_id.is_some() {
                updated.child_chat_id = child_chat_id;
            }
            updated.error = None;
            updated.finished_at = Some(now);
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        self.retire_runtime(agent_id).await;
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
            updated.current_tool = None;
            updated.result_payload_path = Some(result_payload_path);
            updated.finished_at = Some(now);
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        self.retire_runtime(agent_id).await;
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
            updated.current_tool = None;
            updated.result_payload_path = Some(result_payload_path);
            updated.finished_at = Some(now);
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        self.retire_runtime(agent_id).await;
        Ok(updated)
    }

    pub async fn mark_interrupted(
        &self,
        agent_id: &str,
        reason: String,
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
            let mut updated = current;
            let now = Utc::now();
            updated.status = BgAgentStatus::Interrupted;
            updated.error = Some(reason);
            updated.current_tool = None;
            updated.finished_at = Some(now);
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        self.retire_runtime(agent_id).await;
        Ok(updated)
    }

    pub async fn mark_waiting_for_approval(
        self: &Arc<Self>,
        agent_id: &str,
    ) -> Result<BackgroundAgent, String> {
        self.update_record_if_not_terminal(agent_id, false, |record, _| {
            record.status = BgAgentStatus::WaitingForApproval;
            Ok(())
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
            records.insert(agent_id.to_string(), updated);
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
        }
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        Ok(())
    }

    pub async fn count_active_for_parent_root(&self, parent_root_chat_id: &str) -> usize {
        self.records
            .read()
            .await
            .values()
            .filter(|record| {
                record.parent_root_chat_id.as_deref() == Some(parent_root_chat_id)
                    || record.parent_chat_id == parent_root_chat_id
            })
            .filter(|record| !record.status.is_terminal())
            .count()
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

    pub async fn find_agent_id_by_child_chat_id(&self, chat_id: &str) -> Option<String> {
        self.records
            .read()
            .await
            .values()
            .find(|record| record.child_chat_id.as_deref() == Some(chat_id))
            .map(|record| record.agent_id.clone())
    }

    pub async fn list_descendants(&self, agent_id: &str) -> Vec<BackgroundAgent> {
        let records = self.records.read().await;
        let Some(root) = records.get(agent_id) else {
            return Vec::new();
        };
        let Some(child_chat_id) = root.child_chat_id.clone() else {
            return Vec::new();
        };
        let mut pending_chat_ids = VecDeque::from([child_chat_id]);
        let mut visited_chat_ids = HashSet::new();
        let mut visited_agent_ids = HashSet::from([agent_id.to_string()]);
        let mut descendants = Vec::new();
        while let Some(parent_chat_id) = pending_chat_ids.pop_front() {
            if !visited_chat_ids.insert(parent_chat_id.clone()) {
                continue;
            }
            let mut children: Vec<BackgroundAgent> = records
                .values()
                .filter(|record| record.parent_chat_id == parent_chat_id)
                .filter(|record| visited_agent_ids.insert(record.agent_id.clone()))
                .cloned()
                .collect();
            children.sort_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then(a.agent_id.cmp(&b.agent_id))
            });
            for child in children {
                if let Some(child_chat_id) = child.child_chat_id.clone() {
                    pending_chat_ids.push_back(child_chat_id);
                }
                descendants.push(child);
            }
        }
        descendants
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
        if record.finished_at.is_some() {
            record.finished_at = Some(last_update_at);
        }
        let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
        drop(records);
        self.flush_records(snapshot).await
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
        let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
        drop(records);
        self.flush_records(snapshot).await
    }

    #[doc(hidden)]
    pub async fn clear_result_summary_for_test(&self, agent_id: &str) -> Result<(), String> {
        let mut records = self.records.write().await;
        let record = records
            .get_mut(agent_id)
            .ok_or_else(|| "agent not found".to_string())?;
        record.result_summary = None;
        let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
        drop(records);
        self.flush_records(snapshot).await
    }

    pub async fn get(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
    ) -> Result<BackgroundAgent, String> {
        let records = self.records.read().await;
        scoped_record(&records, parent_chat_id, agent_id)
    }

    pub async fn get_any(&self, agent_id: &str) -> Result<BackgroundAgent, String> {
        self.records
            .read()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| "agent not found".to_string())
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

    pub async fn cancel_subtree(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        subtree: bool,
        reason: Option<String>,
    ) -> Result<Vec<BackgroundAgent>, String> {
        let root = self.get(parent_chat_id, agent_id).await?;
        let mut records = vec![root];
        if subtree {
            records.extend(self.list_descendants(agent_id).await);
        }
        let mut updated = Vec::with_capacity(records.len());
        for record in records {
            if record.status.is_terminal() {
                updated.push(record);
                continue;
            }
            updated.push(
                self.cancel(&record.parent_chat_id, &record.agent_id, reason.clone())
                    .await?,
            );
        }
        Ok(updated)
    }

    pub async fn abort_flag(&self, agent_id: &str) -> Option<Arc<AtomicBool>> {
        self.runtime
            .read()
            .await
            .get(agent_id)
            .map(|runtime| runtime.abort_flag.clone())
    }

    pub async fn set_completion_push(&self, agent_id: &str, push: PushMode) -> Result<(), String> {
        self.update_record(agent_id, |record, _| {
            record.completion_push = push;
            Ok(())
        })
        .await
        .map(|_| ())
    }

    pub async fn interrupt_notify(&self, agent_id: &str) -> Option<Arc<Notify>> {
        self.runtime
            .read()
            .await
            .get(agent_id)
            .map(|runtime| runtime.interrupt_notify.clone())
    }

    pub async fn pending_deliveries(&self, agent_id: &str) -> Vec<PendingDelivery> {
        self.records
            .read()
            .await
            .get(agent_id)
            .map(|record| record.pending_deliveries.clone())
            .unwrap_or_default()
    }

    pub async fn has_preempt(&self, agent_id: &str) -> bool {
        let runtime = self.runtime.read().await.get(agent_id).cloned();
        let Some(runtime) = runtime else { return false };
        let state = runtime.delivery_state.lock().await;
        self.records
            .read()
            .await
            .get(agent_id)
            .map(|record| {
                record.pending_deliveries.iter().any(|delivery| {
                    delivery.push == PushMode::Preempt && !state.claimed.contains(&delivery.id)
                })
            })
            .unwrap_or(false)
    }

    pub async fn enqueue_delivery(
        &self,
        agent_id: &str,
        delivery: PendingDelivery,
    ) -> Result<DeliveryOutcome, String> {
        let runtime = self
            .runtime
            .read()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| "agent is not running in this process".to_string())?;
        let state = runtime.delivery_state.lock().await;
        if state.sealed {
            return Err("agent delivery turn already finished".to_string());
        }
        let mut outcome = DeliveryOutcome::Queued;
        let preempt = delivery.push == PushMode::Preempt;
        self.update_record(agent_id, |record, _| {
            if record.delivery_ids.contains(&delivery.id)
                || record
                    .pending_deliveries
                    .iter()
                    .any(|pending| pending.id == delivery.id)
            {
                outcome = DeliveryOutcome::Duplicate;
                return Ok(());
            }
            if record.status.is_terminal() {
                return Err("agent already finished".to_string());
            }
            if record.pending_deliveries.len() >= MAX_INBOX_MESSAGES {
                return Err("agent delivery queue is full".to_string());
            }
            record.pending_deliveries.push(delivery);
            Ok(())
        })
        .await?;
        if preempt && outcome == DeliveryOutcome::Queued {
            runtime.interrupt_notify.notify_one();
        }
        Ok(outcome)
    }

    pub async fn update_pending_delivery(
        &self,
        agent_id: &str,
        id: &str,
        push: Option<PushMode>,
        cancel: bool,
    ) -> Result<(), String> {
        let runtime = self.runtime.read().await.get(agent_id).cloned();
        let state = match runtime.as_ref() {
            Some(runtime) => Some(runtime.delivery_state.lock().await),
            None => None,
        };
        if state
            .as_ref()
            .is_some_and(|state| state.claimed.contains(id))
        {
            return Err("delivery is already being applied".to_string());
        }
        self.update_record(agent_id, |record, _| {
            let index = record
                .pending_deliveries
                .iter()
                .position(|delivery| delivery.id == id)
                .ok_or_else(|| "pending delivery not found".to_string())?;
            if cancel {
                let cancelled = record.pending_deliveries.remove(index);
                record.delivery_ids.push(cancelled.id);
            } else if let Some(push) = push {
                record.pending_deliveries[index].push = push;
            }
            Ok(())
        })
        .await?;
        if !cancel && push == Some(PushMode::Preempt) {
            if let Some(notify) = self.interrupt_notify(agent_id).await {
                notify.notify_one();
            }
        }
        Ok(())
    }

    pub async fn acknowledge_delivery(&self, agent_id: &str, id: &str) -> Result<(), String> {
        self.update_record(agent_id, |record, _| {
            record
                .pending_deliveries
                .retain(|delivery| delivery.id != id);
            if !record.delivery_ids.iter().any(|delivered| delivered == id) {
                record.delivery_ids.push(id.to_string());
            }
            Ok(())
        })
        .await
        .map(|_| ())
    }

    async fn claim_pending_deliveries(
        &self,
        agent_id: &str,
        seal: bool,
        filter: impl Fn(&PendingDelivery) -> bool,
    ) -> Result<Vec<PendingDelivery>, String> {
        let runtime = self
            .runtime
            .read()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| "agent is not running in this process".to_string())?;
        let mut state = runtime.delivery_state.lock().await;
        let records = self.records.read().await;
        let record = records
            .get(agent_id)
            .ok_or_else(|| "agent not found".to_string())?;
        let deliveries = record
            .pending_deliveries
            .iter()
            .filter(|delivery| filter(delivery) && !state.claimed.contains(&delivery.id))
            .cloned()
            .collect::<Vec<_>>();
        state
            .claimed
            .extend(deliveries.iter().map(|delivery| delivery.id.clone()));
        if seal {
            state.sealed = !deliveries.iter().any(|delivery| delivery.wake);
        }
        Ok(deliveries)
    }

    pub async fn drain_deliveries(
        &self,
        agent_id: &str,
        idle: bool,
    ) -> Result<Vec<PendingDelivery>, String> {
        self.claim_pending_deliveries(agent_id, false, |delivery| {
            idle || delivery.push != PushMode::WhenIdle
        })
        .await
    }

    /// Claim the final batch and atomically stop accepting if it cannot rearm the runner.
    /// Claims are transient: pending payloads remain durable until trajectory acknowledgement.
    pub async fn finish_delivery_turn(
        &self,
        agent_id: &str,
    ) -> Result<Vec<PendingDelivery>, String> {
        self.claim_pending_deliveries(agent_id, true, |_| true)
            .await
    }

    pub async fn inbox_for(&self, agent_id: &str) -> Option<Arc<Mutex<VecDeque<InboxMessage>>>> {
        self.runtime
            .read()
            .await
            .get(agent_id)
            .map(|runtime| runtime.inbox.clone())
    }

    pub async fn push_inbox(&self, agent_id: &str, msg: InboxMessage) -> Result<(), String> {
        let record = self
            .records
            .read()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| "agent not found".to_string())?;
        if record.status.is_terminal() {
            return Err("agent already finished".to_string());
        }
        let inbox = self
            .inbox_for(agent_id)
            .await
            .ok_or_else(|| "agent is not running in this process".to_string())?;
        let mut inbox = inbox.lock().await;
        if inbox.len() >= MAX_INBOX_MESSAGES {
            return Err("agent inbox is full".to_string());
        }
        inbox.push_back(msg);
        Ok(())
    }

    pub async fn drain_inbox(&self, agent_id: &str) -> Vec<InboxMessage> {
        let Some(inbox) = self.inbox_for(agent_id).await else {
            return Vec::new();
        };
        let mut inbox = inbox.lock().await;
        inbox.drain(..).collect()
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
            if record.parent_chat_id != parent_chat_id || record.status.is_terminal() {
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
                "Running subagent target file overlap detected: {}",
                overlaps.join("; ")
            ))
        }
    }

    async fn update_record<F>(&self, agent_id: &str, update: F) -> Result<BackgroundAgent, String>
    where
        F: FnOnce(&mut BackgroundAgent, DateTime<Utc>) -> Result<(), String>,
    {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            let mut updated = current;
            let now = Utc::now();
            update(&mut updated, now)?;
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        Ok(updated)
    }

    async fn update_record_if_not_terminal<F>(
        self: &Arc<Self>,
        agent_id: &str,
        debounce_write: bool,
        update: F,
    ) -> Result<BackgroundAgent, String>
    where
        F: FnOnce(&mut BackgroundAgent, DateTime<Utc>) -> Result<(), String>,
    {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            if current.status.is_terminal() {
                return Ok(current);
            }
            let mut updated = current;
            let now = Utc::now();
            update(&mut updated, now)?;
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.schedule_or_flush_records(snapshot, !debounce_write)
                .await?;
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

    async fn retire_runtime(&self, agent_id: &str) {
        self.runtime.write().await.remove(agent_id);
    }

    #[doc(hidden)]
    pub async fn flush_pending_writes_for_test(&self) -> Result<(), String> {
        let snapshot: Vec<BackgroundAgent> = self.records.read().await.values().cloned().collect();
        self.flush_records(snapshot).await
    }

    async fn flush_records(&self, _records: Vec<BackgroundAgent>) -> Result<(), String> {
        // Re-snapshot after serializing disk writes: older scheduled flushes must
        // never overwrite a newly accepted durable delivery.
        let _write = self.storage_write.lock().await;
        let records = self.records.read().await.values().cloned().collect();
        storage::save_all(&self.storage_root, records).await?;
        let mut write_state = self.write_state.lock().await;
        write_state.pending = false;
        write_state.last_write = Some(Instant::now());
        if let Some(handle) = write_state.flush_task.take() {
            handle.abort();
        }
        Ok(())
    }

    async fn schedule_or_flush_records(
        self: &Arc<Self>,
        snapshot: Vec<BackgroundAgent>,
        force_flush: bool,
    ) -> Result<(), String> {
        if force_flush {
            return self.flush_records(snapshot).await;
        }
        let mut write_state = self.write_state.lock().await;
        let now = Instant::now();
        let should_flush_now = write_state
            .last_write
            .map(|last| now.duration_since(last) >= WRITE_DEBOUNCE)
            .unwrap_or(true);
        if should_flush_now {
            drop(write_state);
            return self.flush_records(snapshot).await;
        }
        write_state.pending = true;
        if write_state.flush_task.is_none() {
            let registry = Arc::clone(self);
            let wait_for = WRITE_DEBOUNCE
                .checked_sub(now.duration_since(write_state.last_write.unwrap_or(now)))
                .unwrap_or(Duration::ZERO);
            write_state.flush_task = Some(tokio::spawn(async move {
                tokio::time::sleep(wait_for).await;
                let records = registry.records.read().await;
                let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
                drop(records);
                let _ = registry.flush_records(snapshot).await;
            }));
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn registry() -> (tempfile::TempDir, Arc<BackgroundAgentRegistry>) {
        let temp = tempfile::tempdir().unwrap();
        let registry = BackgroundAgentRegistry::new(temp.path().join("agents"))
            .await
            .unwrap();
        (temp, registry)
    }

    fn request(parent_chat_id: &str) -> CreateAgentRequest {
        CreateAgentRequest {
            parent_chat_id: parent_chat_id.to_string(),
            parent_root_chat_id: None,
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "test".to_string(),
            title: "Test agent".to_string(),
            prompt: "Test prompt".to_string(),
            target_files: Vec::new(),
            model: "test-model".to_string(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        }
    }

    fn inbox_message(text: impl Into<String>) -> InboxMessage {
        InboxMessage {
            from: "sibling".to_string(),
            text: text.into(),
            queued_at: Utc::now(),
        }
    }

    fn completion() -> AgentCompletion {
        AgentCompletion {
            result_summary: "done".to_string(),
            edited_files: Vec::new(),
            diff_summary: None,
            conflict_summary: None,
            child_chat_id: None,
        }
    }

    #[tokio::test]
    async fn list_descendants_stops_at_agent_and_chat_cycles() {
        let (_temp, registry) = registry().await;
        let (root, _, _) = registry.create(request("chat-a")).await.unwrap();
        registry
            .mark_running(&root.agent_id, "chat-b".to_string())
            .await
            .unwrap();
        let (child, _, _) = registry.create(request("chat-b")).await.unwrap();
        registry
            .mark_running(&child.agent_id, "chat-a".to_string())
            .await
            .unwrap();

        let descendants = registry.list_descendants(&root.agent_id).await;

        assert_eq!(descendants.len(), 1);
        assert_eq!(descendants[0].agent_id, child.agent_id);
    }

    #[tokio::test]
    async fn update_activity_refreshes_last_activity() {
        let (_temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();

        let updated = registry
            .update_activity(&record.agent_id, None, None, None)
            .await
            .unwrap();

        assert!(updated.last_activity.is_some());
        assert!(
            chrono::DateTime::parse_from_rfc3339(updated.last_activity.as_deref().unwrap()).is_ok()
        );
    }

    #[tokio::test]
    async fn rapid_progress_updates_are_debounced_to_one_persisted_write() {
        let (temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        let records_path = temp.path().join("agents").join("records.json");
        let baseline = tokio::fs::read_to_string(&records_path).await.unwrap();

        let mut last = None;
        for step in 1..=5 {
            last = Some(
                registry
                    .update_progress(&record.agent_id, format!("step {step}"), step)
                    .await
                    .unwrap(),
            );
        }

        let persisted_before_flush = tokio::fs::read_to_string(&records_path).await.unwrap();
        assert_eq!(persisted_before_flush, baseline);

        registry.flush_pending_writes_for_test().await.unwrap();

        let persisted = storage::load_all(&temp.path().join("agents"))
            .await
            .unwrap();
        assert_eq!(persisted.get(&record.agent_id), last.as_ref());
    }

    #[tokio::test]
    async fn old_terminal_records_are_pruned_on_write() {
        let (temp, registry) = registry().await;
        let (old, _, _) = registry.create(request("parent")).await.unwrap();
        registry
            .mark_failed(&old.agent_id, "boom".to_string())
            .await
            .unwrap();
        registry
            .set_last_update_at_for_test(&old.agent_id, Utc::now() - TimeDelta::days(8))
            .await
            .unwrap();
        let (recent, _, _) = registry.create(request("parent")).await.unwrap();
        registry
            .mark_completed(&recent.agent_id, completion())
            .await
            .unwrap();
        let (live, _, _) = registry.create(request("parent")).await.unwrap();

        registry.flush_pending_writes_for_test().await.unwrap();

        let persisted = storage::load_all(&temp.path().join("agents"))
            .await
            .unwrap();
        assert!(!persisted.contains_key(&old.agent_id));
        assert!(persisted.contains_key(&recent.agent_id));
        assert!(persisted.contains_key(&live.agent_id));
    }

    #[tokio::test]
    async fn queued_inbox_accepts_messages_and_terminal_agents_reject_them() {
        let (_temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();

        registry
            .push_inbox(&record.agent_id, inbox_message("queued"))
            .await
            .unwrap();
        registry
            .mark_completed(&record.agent_id, completion())
            .await
            .unwrap();

        assert!(registry.inbox_for(&record.agent_id).await.is_none());
        assert_eq!(
            registry
                .push_inbox(&record.agent_id, inbox_message("late"))
                .await
                .unwrap_err(),
            "agent already finished"
        );
        assert_eq!(
            registry
                .push_inbox("missing", inbox_message("missing"))
                .await
                .unwrap_err(),
            "agent not found"
        );
    }

    #[tokio::test]
    async fn inbox_drops_oldest_message_after_capacity() {
        let (_temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        for index in 0..MAX_INBOX_MESSAGES {
            registry
                .push_inbox(&record.agent_id, inbox_message(index.to_string()))
                .await
                .unwrap();
        }
        assert!(registry
            .push_inbox(&record.agent_id, inbox_message("overflow"))
            .await
            .is_err());
        let inbox = registry.drain_inbox(&record.agent_id).await;
        assert_eq!(inbox.len(), MAX_INBOX_MESSAGES);
        assert_eq!(inbox[0].text, "0");
    }

    fn delivery(push: PushMode) -> PendingDelivery {
        PendingDelivery::new(Vec::new(), push, "registry-test".to_string(), true)
    }

    #[tokio::test]
    async fn delivery_boundaries_dedupe_and_reprioritize() {
        let (_temp, registry) = registry().await;
        let (record, cancel, _) = registry.create(request("parent")).await.unwrap();
        let idle = delivery(PushMode::WhenIdle);
        let append = delivery(PushMode::Append);
        registry
            .enqueue_delivery(&record.agent_id, idle.clone())
            .await
            .unwrap();
        registry
            .enqueue_delivery(&record.agent_id, append.clone())
            .await
            .unwrap();
        assert_eq!(
            registry
                .enqueue_delivery(&record.agent_id, append.clone())
                .await
                .unwrap(),
            DeliveryOutcome::Duplicate
        );
        let safe = registry
            .drain_deliveries(&record.agent_id, false)
            .await
            .unwrap();
        assert_eq!(safe.len(), 1);
        assert_eq!(safe[0].id, append.id);
        assert_eq!(registry.pending_deliveries(&record.agent_id).await.len(), 2);
        registry
            .acknowledge_delivery(&record.agent_id, &append.id)
            .await
            .unwrap();
        registry
            .update_pending_delivery(&record.agent_id, &idle.id, Some(PushMode::Preempt), false)
            .await
            .unwrap();
        assert!(registry.has_preempt(&record.agent_id).await);
        assert!(
            !cancel.load(Ordering::SeqCst),
            "preempt must not cancel the runner"
        );
        registry
            .update_pending_delivery(&record.agent_id, &idle.id, None, true)
            .await
            .unwrap();
        assert!(registry
            .pending_deliveries(&record.agent_id)
            .await
            .is_empty());
        assert_eq!(
            registry
                .enqueue_delivery(&record.agent_id, idle)
                .await
                .unwrap(),
            DeliveryOutcome::Duplicate
        );
    }

    #[tokio::test]
    async fn delivery_idle_and_completion_mode_persist() {
        let (temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        assert_eq!(record.completion_push, PushMode::Append);
        registry
            .set_completion_push(&record.agent_id, PushMode::Preempt)
            .await
            .unwrap();
        let idle = delivery(PushMode::WhenIdle);
        registry
            .enqueue_delivery(&record.agent_id, idle.clone())
            .await
            .unwrap();
        let restored = storage::load_all(&temp.path().join("agents"))
            .await
            .unwrap();
        let restored = &restored[&record.agent_id];
        assert_eq!(restored.completion_push, PushMode::Preempt);
        assert_eq!(restored.pending_deliveries[0].id, idle.id);
        assert!(!restored.delivery_ids.contains(&idle.id));
        assert!(registry
            .drain_deliveries(&record.agent_id, false)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            registry
                .drain_deliveries(&record.agent_id, true)
                .await
                .unwrap()[0]
                .id,
            idle.id
        );
    }

    #[tokio::test]
    async fn claimed_delivery_survives_restart_until_acknowledged() {
        let (temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        let message = delivery(PushMode::Append);
        registry
            .enqueue_delivery(&record.agent_id, message.clone())
            .await
            .unwrap();
        assert_eq!(
            registry
                .drain_deliveries(&record.agent_id, false)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(registry
            .drain_deliveries(&record.agent_id, false)
            .await
            .unwrap()
            .is_empty());
        let restored = BackgroundAgentRegistry::new(temp.path().join("agents"))
            .await
            .unwrap();
        assert_eq!(
            restored.pending_deliveries(&record.agent_id).await[0].id,
            message.id
        );
        registry
            .acknowledge_delivery(&record.agent_id, &message.id)
            .await
            .unwrap();
        let saved = storage::load_all(&temp.path().join("agents"))
            .await
            .unwrap();
        assert!(saved[&record.agent_id].pending_deliveries.is_empty());
        assert!(saved[&record.agent_id].delivery_ids.contains(&message.id));
    }

    #[tokio::test]
    async fn final_boundary_rearms_wake_and_seals_non_wake() {
        let (_temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        let wake = delivery(PushMode::WhenIdle);
        registry
            .enqueue_delivery(&record.agent_id, wake)
            .await
            .unwrap();
        assert_eq!(
            registry
                .finish_delivery_turn(&record.agent_id)
                .await
                .unwrap()
                .len(),
            1
        );
        let mut quiet = delivery(PushMode::WhenIdle);
        quiet.wake = false;
        registry
            .enqueue_delivery(&record.agent_id, quiet)
            .await
            .unwrap();
        assert_eq!(
            registry
                .finish_delivery_turn(&record.agent_id)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(registry
            .enqueue_delivery(&record.agent_id, delivery(PushMode::Append))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn racing_final_boundary_never_strands_accepted_delivery() {
        let (_temp, registry) = registry().await;
        for _ in 0..20 {
            let (record, _, _) = registry.create(request("parent")).await.unwrap();
            let message = delivery(PushMode::WhenIdle);
            let (accepted, drained) = tokio::join!(
                registry.enqueue_delivery(&record.agent_id, message.clone()),
                registry.finish_delivery_turn(&record.agent_id),
            );
            let drained = drained.unwrap();
            if accepted.is_ok() {
                assert!(drained.iter().any(|delivery| delivery.id == message.id));
            } else {
                assert!(drained.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn set_merge_status_rejects_unknown_statuses() {
        let (_temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();

        assert_eq!(
            registry
                .set_merge_status(&record.agent_id, "weird")
                .await
                .unwrap_err(),
            "invalid merge status"
        );
        assert_eq!(
            registry
                .set_merge_outcome(&record.agent_id, "weird", None, None)
                .await
                .unwrap_err(),
            "invalid merge status"
        );
    }
}
