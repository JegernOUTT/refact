use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::Instant as TokioInstant;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::get_or_create_session_with_trajectory;
use crate::chat::process_command_queue;
use crate::chat::try_restore_session_if_trajectory_exists;
use crate::chat::types::{ChatCommand, CommandRequest, EnqueueCommandOutcome, max_queue_size};
use crate::files_correction::get_active_project_path;
use crate::global_context::SharedGlobalContext;
use crate::scheduler::scheduler_timezone;

use super::delivery::deliver;
use super::exec_action::{command_error_summary, run_command, CommandRunResult};
use super::jitter::{jittered_next_run_ms_for, one_shot_jittered_next_run_ms_for, JitterConfig};
use super::retry::{classify, retry_delay_ms};
use super::schedule::{next_run_ms, recurring_missed_grace_state, MissedRunGraceConfig};
use super::store::{CronStore, InMemoryCronStore, JsonFileCronStore};
use super::types::{
    Action, AgentTarget, CommandSpec, CronRunRecord, Delivery, Job, SchedulerConfig, Trigger,
};

const DEFAULT_SLEEP_MS: u64 = 60_000;
const IDLE_DEFER_MS: u64 = 30_000;
const INVALID_TARGET_DEFER_MS: u64 = 60_000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

pub struct CronRunner {
    pub store: Arc<dyn CronStore>,
    pub gcx: SharedGlobalContext,
    pub shutdown_flag: Arc<AtomicBool>,
    pub change_notify: Arc<Notify>,
    pub jitter_cfg: JitterConfig,
    run_semaphore: Arc<Semaphore>,
    deferred_until_ms: HashMap<String, u64>,
}

impl CronRunner {
    pub fn new(store: Arc<dyn CronStore>, gcx: SharedGlobalContext) -> Self {
        let shutdown_flag = gcx.shutdown_flag.clone();
        let change_notify = store.change_notify();
        let max_concurrent_runs = gcx.scheduler_config.max_concurrent_runs.max(1);
        Self {
            store,
            gcx,
            shutdown_flag,
            change_notify,
            jitter_cfg: JitterConfig::default(),
            run_semaphore: Arc::new(Semaphore::new(max_concurrent_runs)),
            deferred_until_ms: HashMap::new(),
        }
    }

    pub fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            self.run().await;
        })
    }

    pub async fn fire_manual_job(
        gcx: SharedGlobalContext,
        mut task: Job,
        now: u64,
    ) -> Result<bool, String> {
        if !runner_enabled(&gcx.scheduler_config) {
            return Ok(false);
        }
        task.trigger_at_ms = Some(now);
        let store = Arc::new(InMemoryCronStore::new());
        let runner = Self::new(store, gcx);
        runner.fire_with_missed(&task, false, now, false).await
    }

    pub async fn fire_store_job_now(
        store: Arc<dyn CronStore>,
        gcx: SharedGlobalContext,
        job_id: &str,
        now: u64,
    ) -> Result<bool, String> {
        if !runner_enabled(&gcx.scheduler_config) {
            return Ok(false);
        }
        let mut task = store
            .get(job_id)
            .await
            .ok_or_else(|| format!("Scheduled task {job_id} not found"))?;
        task.trigger_at_ms = Some(now);
        let mut runner = Self::new(store.clone(), gcx);
        let fired = runner.handle_due_task(task, now).await;
        if !fired {
            if let Some(mut stored) = store
                .get(job_id)
                .await
                .filter(|task| task.last_status.as_deref() == Some("deferred"))
            {
                stored.trigger_at_ms = Some(now);
                let _ = store.replace(stored).await?;
            }
        }
        Ok(fired)
    }

    async fn run(mut self) {
        if !runner_enabled(&self.gcx.scheduler_config) {
            return;
        }
        self.catch_up().await;

        loop {
            if self.shutdown_flag.load(Ordering::Relaxed) {
                break;
            }

            let now = now_ms();
            let tasks = self.store.list().await;
            let next = tasks
                .iter()
                .filter_map(|task| self.scheduled_fire_at_ms(task, now))
                .min()
                .unwrap_or(now + DEFAULT_SLEEP_MS);
            let sleep_until = TokioInstant::now() + Duration::from_millis(next.saturating_sub(now));

            tokio::select! {
                _ = tokio::time::sleep_until(sleep_until) => {}
                _ = self.change_notify.notified() => continue,
                _ = wait_for_shutdown(self.shutdown_flag.clone()) => break,
            }

            self.fire_due_tasks(now_ms()).await;
        }
    }

    async fn catch_up(&mut self) {
        let now = now_ms();
        let mut missed_counts = HashMap::<String, u64>::new();

        for task in self.store.list().await {
            if self.shutdown_flag.load(Ordering::Relaxed) {
                break;
            }
            if !task.durable {
                continue;
            }
            if task.recurring {
                self.resume_recurring_task(&task, now).await;
                continue;
            }
            if !missed_one_shot_task(&task, now) {
                continue;
            }
            let chat_id = task.chat_id().map(str::to_string);
            if let Some(chat_id) = &chat_id {
                let app = AppState::from_gcx(self.gcx.clone()).await;
                let restored =
                    try_restore_session_if_trajectory_exists(app, &self.gcx.chat_sessions, chat_id)
                        .await;
                if !restored {
                    tracing::warn!(
                        "skipping missed durable one-shot {}: no trajectory found for chat {}",
                        task.id,
                        chat_id
                    );
                    continue;
                }
            } else if !job_is_isolated(&task) && !command_job_has_non_chat_delivery(&task) {
                continue;
            }
            match self.fire_with_missed(&task, true, now, true).await {
                Ok(true) => {
                    if let Some(chat_id) = chat_id {
                        *missed_counts.entry(chat_id).or_default() += 1;
                    }
                    if let Err(error) = self.store.remove(&task.id).await {
                        tracing::warn!(
                            "failed to remove caught-up scheduled task {}: {}",
                            task.id,
                            error
                        );
                    }
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!("failed to catch up scheduled task {}: {}", task.id, error);
                }
            }
        }

        for (chat_id, missed_count) in missed_counts {
            self.emit_catch_up_notice(&chat_id, missed_count).await;
        }
    }

    async fn resume_recurring_task(&self, task: &Job, now: u64) {
        if !matches!(
            task.trigger,
            Trigger::Cron { .. } | Trigger::Interval { .. }
        ) {
            return;
        }
        if task
            .trigger_at_ms
            .is_some_and(|trigger_at_ms| trigger_at_ms <= now)
        {
            return;
        }
        let from_ms = task.last_fired_at_ms.unwrap_or(task.created_at_ms);
        let Some(state) = recurring_missed_grace_state(
            task,
            from_ms,
            now,
            scheduler_timezone(),
            self.missed_grace_config(),
        ) else {
            return;
        };
        if state.due_ms.is_some() && !state.should_fire {
            let Some(advance_to_ms) = state.advance_last_fired_at_ms else {
                return;
            };
            if let Err(error) = self
                .store
                .update_fired(&task.id, advance_to_ms, task.fire_count)
                .await
            {
                tracing::warn!("failed to resume scheduled task {}: {}", task.id, error);
            }
        }
    }

    fn missed_grace_config(&self) -> MissedRunGraceConfig {
        MissedRunGraceConfig::from(&self.gcx.scheduler_config)
    }

    fn recent_runs_cap(&self) -> usize {
        self.gcx.scheduler_config.recent_runs_cap
    }

    async fn fire_due_tasks(&mut self, now: u64) {
        if !runner_enabled(&self.gcx.scheduler_config) {
            return;
        }
        let due_tasks = self
            .store
            .list()
            .await
            .into_iter()
            .filter(|task| {
                self.task_is_due(task, now) || self.task_needs_missed_fast_forward(task, now)
            })
            .collect::<Vec<_>>();

        let mut handles = Vec::new();
        let max_concurrent_runs = self.gcx.scheduler_config.max_concurrent_runs.max(1);
        for task in due_tasks {
            if self.shutdown_flag.load(Ordering::Relaxed) {
                break;
            }
            match self.fast_forward_recurring_if_missed(&task, now).await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(
                        "failed to fast-forward scheduled task {}: {}",
                        task.id,
                        error
                    );
                    continue;
                }
            }
            if task_can_use_cron_lane(&task) && handles.len() >= max_concurrent_runs {
                self.defer_task(&task, now);
                continue;
            }
            if task_can_use_cron_lane(&task) && self.run_semaphore.available_permits() == 0 {
                self.defer_task(&task, now);
                continue;
            }
            if !task_can_use_cron_lane(&task) {
                self.handle_due_task(task, now).await;
                continue;
            }
            let Ok(permit) = self.run_semaphore.clone().try_acquire_owned() else {
                self.defer_task(&task, now);
                continue;
            };
            let store = self.store.clone();
            let gcx = self.gcx.clone();
            let shutdown_flag = self.shutdown_flag.clone();
            let change_notify = self.change_notify.clone();
            let jitter_cfg = self.jitter_cfg.clone();
            let task_id = task.id.clone();
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                let mut runner = CronRunner {
                    store,
                    gcx,
                    shutdown_flag,
                    change_notify,
                    jitter_cfg,
                    run_semaphore: Arc::new(Semaphore::new(1)),
                    deferred_until_ms: HashMap::new(),
                };
                runner.handle_due_task(task, now).await;
                (task_id, runner.deferred_until_ms)
            }));
        }
        for handle in handles {
            if let Ok((task_id, deferred)) = handle.await {
                if deferred.contains_key(&task_id) {
                    self.deferred_until_ms.extend(deferred);
                } else {
                    self.deferred_until_ms.remove(&task_id);
                }
            }
        }
    }

    async fn handle_due_task(&mut self, task: Job, now: u64) -> bool {
        if matches!(&task.action, Action::Command { .. }) {
            return self.handle_due_command_task(task, now).await;
        }
        if job_is_isolated(&task) {
            return self.handle_due_isolated_task(task, now).await;
        }

        let chat_id = match runnable_chat_id(&task) {
            Ok(chat_id) => chat_id,
            Err("missing chat_id") => {
                self.handle_unfireable_task(&task, now, "missing chat_id")
                    .await;
                return false;
            }
            Err(reason) => {
                self.handle_unfireable_task(&task, now, reason).await;
                return false;
            }
        };

        match chat_fire_status(&self.gcx, chat_id).await {
            ChatFireStatus::Fireable => {}
            ChatFireStatus::Busy => {
                if let Err(error) = self.record_run(&task.id, "deferred", None, now).await {
                    tracing::warn!(
                        "failed to record deferred scheduled task {}: {}",
                        task.id,
                        error
                    );
                }
                self.defer_task(&task, now);
                return false;
            }
            ChatFireStatus::Missing => {
                if task.durable {
                    let app = AppState::from_gcx(self.gcx.clone()).await;
                    let restored = try_restore_session_if_trajectory_exists(
                        app,
                        &self.gcx.chat_sessions,
                        chat_id,
                    )
                    .await;
                    if !restored {
                        tracing::warn!(
                            "durable task {} deferred: no trajectory found for chat {}",
                            task.id,
                            chat_id
                        );
                        if let Err(error) = self
                            .record_run(
                                &task.id,
                                "deferred",
                                Some("no trajectory found".to_string()),
                                now,
                            )
                            .await
                        {
                            tracing::warn!(
                                "failed to record deferred scheduled task {}: {}",
                                task.id,
                                error
                            );
                        }
                        self.defer_invalid_target_task(&task, now);
                        return false;
                    }
                    match chat_fire_status(&self.gcx, chat_id).await {
                        ChatFireStatus::Fireable => {}
                        status => {
                            tracing::warn!(
                                "durable task {} deferred after session restore ({:?})",
                                task.id,
                                status
                            );
                            if let Err(error) = self
                                .record_run(
                                    &task.id,
                                    "deferred",
                                    Some(format!("session restore status: {status:?}")),
                                    now,
                                )
                                .await
                            {
                                tracing::warn!(
                                    "failed to record deferred scheduled task {}: {}",
                                    task.id,
                                    error
                                );
                            }
                            self.defer_invalid_target_task(&task, now);
                            return false;
                        }
                    }
                } else {
                    self.handle_unfireable_task(&task, now, "chat session not found")
                        .await;
                    return false;
                }
            }
            ChatFireStatus::Closed => {
                self.handle_unfireable_task(&task, now, "chat session is closed")
                    .await;
                return false;
            }
        }

        let final_fire = task_final_after_fire(&task, now);
        match self.fire(&task, final_fire, now).await {
            Ok(true) => {}
            Ok(false) => {
                if self.record_skipped_after_advance(&task, now).await {
                    return false;
                }
                if let Err(error) = self.record_run(&task.id, "deferred", None, now).await {
                    tracing::warn!(
                        "failed to record deferred scheduled task {}: {}",
                        task.id,
                        error
                    );
                }
                self.defer_task(&task, now);
                return false;
            }
            Err(error) => {
                tracing::warn!("failed to fire scheduled task {}: {}", task.id, error);
                if self
                    .handle_classifiable_fire_error(&task, final_fire, now, &error)
                    .await
                {
                    return false;
                }
                self.handle_unfireable_task(&task, now, &error).await;
                return false;
            }
        }

        if let Err(error) = self.record_run(&task.id, "fired", None, now).await {
            tracing::warn!(
                "failed to record fired scheduled task {}: {}",
                task.id,
                error
            );
        }
        if final_fire {
            match self.store.remove(&task.id).await {
                Ok(true) => self.emit_auto_expired_notice(&task).await,
                Ok(false) => {
                    tracing::warn!("expired scheduled task {} was already removed", task.id);
                }
                Err(error) => {
                    tracing::warn!(
                        "failed to remove expired scheduled task {}: {}",
                        task.id,
                        error
                    );
                }
            }
        } else if !task.recurring {
            if let Err(error) = self.remove_task(&task).await {
                tracing::warn!(
                    "failed to remove fired one-shot scheduled task {}: {}",
                    task.id,
                    error
                );
            }
        }
        self.deferred_until_ms.remove(&task.id);
        true
    }

    async fn handle_due_command_task(&mut self, task: Job, now: u64) -> bool {
        let cmd = match task.command_spec() {
            Some(cmd) => cmd,
            None => {
                self.handle_unfireable_task(&task, now, "command action is missing")
                    .await;
                return false;
            }
        };
        if let Err(reason) = runnable_command_job(&task) {
            self.handle_unfireable_task(&task, now, reason).await;
            return false;
        }
        if matches!(task.delivery, Delivery::Chat) {
            if let AgentTarget::ExistingChat { chat_id } = &cmd.target {
                if chat_id.is_empty() {
                    self.handle_unfireable_task(&task, now, "missing chat_id")
                        .await;
                    return false;
                }
                match chat_fire_status(&self.gcx, chat_id).await {
                    ChatFireStatus::Fireable => {}
                    ChatFireStatus::Busy => {
                        if let Err(error) = self.record_run(&task.id, "deferred", None, now).await {
                            tracing::warn!(
                                "failed to record deferred scheduled task {}: {}",
                                task.id,
                                error
                            );
                        }
                        self.defer_task(&task, now);
                        return false;
                    }
                    ChatFireStatus::Missing => {
                        if task.durable {
                            let app = AppState::from_gcx(self.gcx.clone()).await;
                            let restored = try_restore_session_if_trajectory_exists(
                                app,
                                &self.gcx.chat_sessions,
                                chat_id,
                            )
                            .await;
                            if !restored {
                                if let Err(error) = self
                                    .record_run(
                                        &task.id,
                                        "deferred",
                                        Some("no trajectory found".to_string()),
                                        now,
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        "failed to record deferred scheduled task {}: {}",
                                        task.id,
                                        error
                                    );
                                }
                                self.defer_invalid_target_task(&task, now);
                                return false;
                            }
                        } else {
                            self.handle_unfireable_task(&task, now, "chat session not found")
                                .await;
                            return false;
                        }
                    }
                    ChatFireStatus::Closed => {
                        self.handle_unfireable_task(&task, now, "chat session is closed")
                            .await;
                        return false;
                    }
                }
            }
        }

        let final_fire = task_final_after_fire(&task, now);
        let result = match self
            .fire_command_with_missed(&task, &cmd, final_fire, now, false)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!("failed to fire scheduled task {}: {}", task.id, error);
                if self
                    .handle_classifiable_fire_error(&task, final_fire, now, &error)
                    .await
                {
                    return false;
                }
                self.handle_unfireable_task(&task, now, &error).await;
                return false;
            }
        };

        let error = (result.status == "error").then(|| command_record_error(&result));
        if let Err(store_error) = self.record_run(&task.id, &result.status, error, now).await {
            tracing::warn!(
                "failed to record scheduled task {} status {}: {}",
                task.id,
                result.status,
                store_error
            );
        }
        if result.status == "error" && command_failure_can_retry(&result) {
            match self
                .schedule_retry(&task, now, &command_retry_text(&result))
                .await
            {
                Ok(true) => {
                    self.deferred_until_ms.remove(&task.id);
                    return false;
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!("failed to schedule retry for task {}: {}", task.id, error);
                }
            }
        }
        if final_fire {
            match self.store.remove(&task.id).await {
                Ok(true) => self.emit_auto_expired_notice(&task).await,
                Ok(false) => {
                    tracing::warn!("expired scheduled task {} was already removed", task.id);
                }
                Err(error) => {
                    tracing::warn!(
                        "failed to remove expired scheduled task {}: {}",
                        task.id,
                        error
                    );
                }
            }
        } else if !task.recurring {
            if let Err(error) = self.remove_task(&task).await {
                tracing::warn!(
                    "failed to remove fired one-shot scheduled task {}: {}",
                    task.id,
                    error
                );
            }
        }
        self.deferred_until_ms.remove(&task.id);
        result.status == "fired"
    }

    async fn handle_due_isolated_task(&mut self, task: Job, now: u64) -> bool {
        if let Err(reason) = runnable_isolated_job(&task) {
            self.handle_unfireable_task(&task, now, reason).await;
            return false;
        }

        let final_fire = task_final_after_fire(&task, now);
        match self.fire(&task, final_fire, now).await {
            Ok(true) => {}
            Ok(false) => {
                if self.record_skipped_after_advance(&task, now).await {
                    return false;
                }
                if let Err(error) = self.record_run(&task.id, "deferred", None, now).await {
                    tracing::warn!(
                        "failed to record deferred scheduled task {}: {}",
                        task.id,
                        error
                    );
                }
                self.defer_task(&task, now);
                return false;
            }
            Err(error) => {
                tracing::warn!("failed to fire scheduled task {}: {}", task.id, error);
                if self
                    .handle_classifiable_fire_error(&task, final_fire, now, &error)
                    .await
                {
                    return false;
                }
                self.handle_unfireable_task(&task, now, &error).await;
                return false;
            }
        }

        if let Err(error) = self.record_run(&task.id, "fired", None, now).await {
            tracing::warn!(
                "failed to record fired scheduled task {}: {}",
                task.id,
                error
            );
        }
        if final_fire {
            match self.store.remove(&task.id).await {
                Ok(true) => self.emit_auto_expired_notice(&task).await,
                Ok(false) => {
                    tracing::warn!("expired scheduled task {} was already removed", task.id);
                }
                Err(error) => {
                    tracing::warn!(
                        "failed to remove expired scheduled task {}: {}",
                        task.id,
                        error
                    );
                }
            }
        } else if !task.recurring {
            if let Err(error) = self.remove_task(&task).await {
                tracing::warn!(
                    "failed to remove fired one-shot scheduled task {}: {}",
                    task.id,
                    error
                );
            }
        }
        self.deferred_until_ms.remove(&task.id);
        true
    }

    async fn handle_unfireable_task(&mut self, task: &Job, now: u64, reason: &str) {
        if task.recurring || task.durable {
            tracing::warn!(
                "deferred scheduled task {} because it is not fireable: {}",
                task.id,
                reason
            );
            if let Err(error) = self
                .record_run(&task.id, "deferred", Some(reason.to_string()), now)
                .await
            {
                tracing::warn!(
                    "failed to record deferred scheduled task {}: {}",
                    task.id,
                    error
                );
            }
            self.defer_invalid_target_task(task, now);
            return;
        }

        tracing::warn!(
            "removing one-shot scheduled task {} because it is not fireable: {}",
            task.id,
            reason
        );
        if let Err(error) = self
            .record_run(&task.id, "skipped", Some(reason.to_string()), now)
            .await
        {
            tracing::warn!(
                "failed to record skipped scheduled task {}: {}",
                task.id,
                error
            );
        }
        if let Err(error) = self.remove_task(task).await {
            tracing::warn!(
                "failed to remove unfireable one-shot scheduled task {}: {}",
                task.id,
                error
            );
        }
    }

    async fn handle_classifiable_fire_error(
        &mut self,
        task: &Job,
        final_fire: bool,
        now: u64,
        error: &str,
    ) -> bool {
        if classify(error).is_none() {
            return false;
        }
        if let Err(store_error) = self
            .record_run(&task.id, "error", Some(error.to_string()), now)
            .await
        {
            tracing::warn!(
                "failed to record retryable scheduled task {} error: {}",
                task.id,
                store_error
            );
        }
        match self.schedule_retry(task, now, error).await {
            Ok(true) => {
                self.deferred_until_ms.remove(&task.id);
                return true;
            }
            Ok(false) => {}
            Err(schedule_error) => {
                tracing::warn!(
                    "failed to schedule retry for task {}: {}",
                    task.id,
                    schedule_error
                );
            }
        }
        if final_fire {
            match self.store.remove(&task.id).await {
                Ok(true) => self.emit_auto_expired_notice(task).await,
                Ok(false) => {
                    tracing::warn!("expired scheduled task {} was already removed", task.id)
                }
                Err(remove_error) => tracing::warn!(
                    "failed to remove expired scheduled task {}: {}",
                    task.id,
                    remove_error
                ),
            }
        } else if !task.recurring {
            if let Err(remove_error) = self.remove_task(task).await {
                tracing::warn!(
                    "failed to remove failed one-shot scheduled task {}: {}",
                    task.id,
                    remove_error
                );
            }
        } else {
            self.deferred_until_ms.remove(&task.id);
        }
        true
    }

    async fn record_skipped_after_advance(&mut self, task: &Job, now: u64) -> bool {
        if !job_was_advanced_before_fire(task, now) {
            return false;
        }
        if let Err(error) = self
            .record_run(
                &task.id,
                "skipped",
                Some("slot advanced before enqueue".to_string()),
                now,
            )
            .await
        {
            tracing::warn!(
                "failed to record skipped scheduled task {}: {}",
                task.id,
                error
            );
        }
        self.deferred_until_ms.remove(&task.id);
        true
    }

    async fn schedule_retry(&self, task: &Job, now: u64, error: &str) -> Result<bool, String> {
        let Some(category) = classify(error) else {
            return Ok(false);
        };
        if self.shutdown_flag.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let mut stored = match self.store.get(&task.id).await {
            Some(stored) => stored,
            None => return Ok(false),
        };
        let retry_cfg = self.gcx.scheduler_config.retry.clone();
        let Some(delay_ms) = retry_delay_ms(&retry_cfg, stored.retry_attempts) else {
            return Ok(false);
        };
        stored.retry_attempts = stored.retry_attempts.saturating_add(1);
        stored.trigger_at_ms = Some(now.saturating_add(delay_ms));
        let next_retry_at_ms = stored.trigger_at_ms;
        let retry_attempts = stored.retry_attempts;
        if !self.store.replace(stored).await? {
            return Ok(false);
        }
        tracing::info!(
            task_id = %task.id,
            ?category,
            retry_attempts,
            ?next_retry_at_ms,
            "scheduled retry for transient scheduler failure"
        );
        Ok(true)
    }

    async fn fast_forward_recurring_if_missed(&self, task: &Job, now: u64) -> Result<bool, String> {
        if !task.recurring || task.trigger_at_ms.is_some() || task.retry_attempts > 0 {
            return Ok(false);
        }
        let from_ms = task.last_fired_at_ms.unwrap_or(task.created_at_ms);
        let Some(state) = recurring_missed_grace_state(
            task,
            from_ms,
            now,
            scheduler_timezone(),
            self.missed_grace_config(),
        ) else {
            return Ok(false);
        };
        if state.due_ms.is_some() && !state.should_fire {
            let Some(advance_to_ms) = state.advance_last_fired_at_ms else {
                return Ok(false);
            };
            self.store
                .update_fired(&task.id, advance_to_ms, task.fire_count)
                .await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn record_run(
        &self,
        job_id: &str,
        status: &str,
        error: Option<String>,
        now_ms: u64,
    ) -> Result<(), String> {
        record_run(
            &self.store,
            job_id,
            status,
            error,
            now_ms,
            self.recent_runs_cap(),
        )
        .await
    }

    async fn remove_task(&mut self, task: &Job) -> Result<(), String> {
        let _ = self.store.remove(&task.id).await?;
        self.deferred_until_ms.remove(&task.id);
        Ok(())
    }

    fn defer_task(&mut self, task: &Job, now: u64) {
        self.deferred_until_ms
            .insert(task.id.clone(), now + IDLE_DEFER_MS);
    }

    fn defer_invalid_target_task(&mut self, task: &Job, now: u64) {
        self.deferred_until_ms
            .insert(task.id.clone(), now + INVALID_TARGET_DEFER_MS);
    }

    fn scheduled_fire_at_ms(&self, task: &Job, now: u64) -> Option<u64> {
        if task.trigger_at_ms.is_none() && task.is_paused() {
            return None;
        }
        if task
            .trigger_at_ms
            .is_some_and(|trigger_at_ms| trigger_at_ms <= now)
        {
            return task.trigger_at_ms;
        }
        if task.retry_attempts > 0 && task.trigger_at_ms.is_some() {
            return task.trigger_at_ms;
        }
        if let Some(deferred_at) = self.deferred_until_ms.get(&task.id).copied() {
            return Some(deferred_at);
        }
        if self.task_needs_missed_fast_forward(task, now) {
            return Some(now);
        }
        scheduled_fire_at_ms(task, now, &self.jitter_cfg, self.missed_grace_config())
    }

    fn task_is_due(&self, task: &Job, now: u64) -> bool {
        self.scheduled_fire_at_ms(task, now)
            .is_some_and(|fire_at| fire_at <= now)
    }

    fn task_needs_missed_fast_forward(&self, task: &Job, now: u64) -> bool {
        if !task.recurring || task.trigger_at_ms.is_some() || task.retry_attempts > 0 {
            return false;
        }
        recurring_missed_grace_state(
            task,
            task.last_fired_at_ms.unwrap_or(task.created_at_ms),
            now,
            scheduler_timezone(),
            self.missed_grace_config(),
        )
        .is_some_and(|state| state.due_ms.is_some() && !state.should_fire)
    }

    async fn fire(&self, task: &Job, final_fire: bool, fired_at_ms: u64) -> Result<bool, String> {
        self.fire_with_missed(task, final_fire, fired_at_ms, false)
            .await
    }

    pub async fn fire_with_missed(
        &self,
        task: &Job,
        final_fire: bool,
        fired_at_ms: u64,
        missed: bool,
    ) -> Result<bool, String> {
        if let Some(cmd) = task.command_spec() {
            let _ = self
                .fire_command_with_missed(task, &cmd, final_fire, fired_at_ms, missed)
                .await?;
            return Ok(true);
        }
        if job_is_isolated(&task) {
            return self
                .fire_isolated_with_missed(task, final_fire, fired_at_ms, missed)
                .await;
        }

        let chat_id = runnable_chat_id(task).map_err(|reason| {
            format!(
                "Scheduled task {} is not a runnable chat job: {reason}",
                task.id
            )
        })?;
        let session_arc = {
            let sessions = self.gcx.chat_sessions.read().await;
            sessions.get(chat_id).cloned()
        }
        .ok_or_else(|| format!("Chat session {chat_id} not found"))?;
        let app = AppState::from_gcx(self.gcx.clone()).await;
        let event_message = if missed {
            cron_fire_message_with_missed(task, final_fire, missed)
        } else {
            cron_fire_message(task, final_fire)
        };
        let prompt = task.prompt().unwrap_or_default().to_string();
        let mode = task.mode().map(str::to_string);
        let processor_flag = {
            let mut session = session_arc.lock().await;
            if session.closed {
                return Err(format!("Chat session {chat_id} is closed"));
            }
            if !session.is_idle() || session.command_queue.iter().any(|r| !r.priority) {
                return Ok(false);
            }
            let queued_commands = 1 + usize::from(mode.is_some());
            if session.command_queue.len().saturating_add(queued_commands) > max_queue_size() {
                return Ok(false);
            }
            if let Some(ref mode) = mode {
                if session.enqueue_priority_command(CommandRequest {
                    client_request_id: format!("cron-set-mode-{}", Uuid::new_v4()),
                    priority: true,
                    command: ChatCommand::SetParams {
                        patch: json!({"mode": mode}),
                    },
                }) != EnqueueCommandOutcome::Accepted
                {
                    return Ok(false);
                }
            }
            if session.enqueue_priority_command(CommandRequest {
                client_request_id: format!("cron-fire-{}", Uuid::new_v4()),
                priority: true,
                command: ChatCommand::UserMessage {
                    content: serde_json::Value::String(prompt),
                    attachments: vec![],
                    context_files: vec![],
                    suppress_auto_enrichment: false,
                },
            }) != EnqueueCommandOutcome::Accepted
            {
                return Ok(false);
            }
            session.add_message(event_message);
            session.queue_processor_running.clone()
        };

        if !processor_flag.swap(true, Ordering::SeqCst) {
            tokio::spawn(process_command_queue(app, session_arc, processor_flag));
        }
        Ok(true)
    }

    async fn fire_command_with_missed(
        &self,
        task: &Job,
        cmd: &CommandSpec,
        final_fire: bool,
        fired_at_ms: u64,
        missed: bool,
    ) -> Result<CommandRunResult, String> {
        runnable_command_job(task).map_err(|reason| {
            format!(
                "Scheduled task {} is not a runnable command job: {reason}",
                task.id
            )
        })?;
        let app = AppState::from_gcx(self.gcx.clone()).await;
        let result = run_command(&app, task, cmd).await;
        if let Err(error) = self
            .deliver_command_result(&app, task, cmd, final_fire, fired_at_ms, missed, &result)
            .await
        {
            if let Err(store_error) = self.record_delivery_error(&task.id, error).await {
                tracing::warn!(
                    "failed to record delivery error for scheduled task {}: {}",
                    task.id,
                    store_error
                );
            }
        } else if let Err(store_error) = self.clear_delivery_error(&task.id).await {
            tracing::warn!(
                "failed to clear delivery error for scheduled task {}: {}",
                task.id,
                store_error
            );
        }
        Ok(result)
    }

    async fn clear_delivery_error(&self, job_id: &str) -> Result<(), String> {
        let Some(mut task) = self.store.get(job_id).await else {
            return Ok(());
        };
        task.last_delivery_error = None;
        if self.store.replace(task).await? {
            Ok(())
        } else {
            Ok(())
        }
    }

    async fn record_delivery_error(&self, job_id: &str, error: String) -> Result<(), String> {
        let mut task = self
            .store
            .get(job_id)
            .await
            .ok_or_else(|| format!("Scheduled task {job_id} not found"))?;
        task.last_delivery_error = Some(error);
        if self.store.replace(task).await? {
            Ok(())
        } else {
            Err(format!("Scheduled task {job_id} not found"))
        }
    }

    async fn deliver_command_result(
        &self,
        app: &AppState,
        task: &Job,
        _cmd: &CommandSpec,
        _final_fire: bool,
        fired_at_ms: u64,
        _missed: bool,
        result: &CommandRunResult,
    ) -> Result<(), String> {
        let mut delivery_job = task.clone();
        delivery_job.last_status = Some(result.status.clone());
        delivery_job.last_error = (result.status == "error").then(|| command_record_error(result));
        delivery_job.last_fired_at_ms = Some(fired_at_ms);
        delivery_job.fire_count = if job_was_advanced_before_fire(task, fired_at_ms) {
            task.fire_count.saturating_sub(1)
        } else {
            task.fire_count
        };
        let output = if result.status == "error" {
            command_error_summary(result)
        } else {
            result.stdout.clone()
        };
        deliver(app, &delivery_job, &output).await
    }

    async fn fire_isolated_with_missed(
        &self,
        task: &Job,
        final_fire: bool,
        fired_at_ms: u64,
        missed: bool,
    ) -> Result<bool, String> {
        runnable_isolated_job(task).map_err(|reason| {
            format!(
                "Scheduled task {} is not a runnable isolated chat job: {reason}",
                task.id
            )
        })?;
        let chat_id = format!("cron_{}_{}", task.id, fired_at_ms);
        let app = AppState::from_gcx(self.gcx.clone()).await;
        let session_arc =
            get_or_create_session_with_trajectory(app.clone(), &app.chat.sessions, &chat_id).await;
        let event_message = if missed {
            cron_fire_message_with_missed(task, final_fire, missed)
        } else {
            cron_fire_message(task, final_fire)
        };
        let prompt = task.prompt().unwrap_or_default().to_string();
        let set_params = isolated_set_params_patch(task);
        let processor_flag = {
            let mut session = session_arc.lock().await;
            if session.closed {
                return Err(format!("Chat session {chat_id} is closed"));
            }
            let queued_commands = 1 + usize::from(set_params.is_some());
            if session.command_queue.len().saturating_add(queued_commands) > max_queue_size() {
                return Ok(false);
            }
            if let Some(patch) = set_params {
                if session.enqueue_priority_command(CommandRequest {
                    client_request_id: format!("cron-set-params-{}", Uuid::new_v4()),
                    priority: true,
                    command: ChatCommand::SetParams { patch },
                }) != EnqueueCommandOutcome::Accepted
                {
                    return Ok(false);
                }
            }
            if session.enqueue_priority_command(CommandRequest {
                client_request_id: format!("cron-fire-{}", Uuid::new_v4()),
                priority: true,
                command: ChatCommand::UserMessage {
                    content: serde_json::Value::String(prompt),
                    attachments: vec![],
                    context_files: vec![],
                    suppress_auto_enrichment: false,
                },
            }) != EnqueueCommandOutcome::Accepted
            {
                return Ok(false);
            }
            session.add_message(event_message);
            session.queue_processor_running.clone()
        };

        if !processor_flag.swap(true, Ordering::SeqCst) {
            tokio::spawn(process_command_queue(app, session_arc, processor_flag));
        }
        Ok(true)
    }

    async fn emit_catch_up_notice(&self, chat_id: &str, missed_count: u64) {
        let session_arc = {
            let sessions = self.gcx.chat_sessions.read().await;
            sessions.get(chat_id).cloned()
        };
        let Some(session_arc) = session_arc else {
            return;
        };
        let mut session = session_arc.lock().await;
        session.add_message(catch_up_notice_message(missed_count));
    }

    async fn emit_auto_expired_notice(&self, task: &Job) {
        let Some(chat_id) = task.chat_id() else {
            return;
        };
        let session_arc = {
            let sessions = self.gcx.chat_sessions.read().await;
            sessions.get(chat_id).cloned()
        };
        let Some(session_arc) = session_arc else {
            return;
        };
        let mut session = session_arc.lock().await;
        session.add_message(auto_expired_notice_message(task));
    }
}

pub fn spawn(store: Arc<dyn CronStore>, gcx: SharedGlobalContext) -> JoinHandle<()> {
    CronRunner::new(store, gcx).spawn()
}

pub async fn spawn_from_active_project(gcx: SharedGlobalContext) -> Vec<JoinHandle<()>> {
    if !runner_enabled(&gcx.scheduler_config) {
        return Vec::new();
    }

    let mut handles = vec![spawn(session_cron_store(), gcx.clone())];
    if let Some(project_root) = get_active_project_path(gcx.clone()).await {
        match JsonFileCronStore::new(project_root) {
            Ok(store) => handles.push(spawn(Arc::new(store), gcx)),
            Err(error) => tracing::warn!("durable scheduler runner disabled: {error}"),
        }
    }
    handles
}

static SESSION_CRON_STORE: OnceLock<Arc<InMemoryCronStore>> = OnceLock::new();
static RUNNER_CHANGE_NOTIFY: OnceLock<Arc<tokio::sync::Notify>> = OnceLock::new();

pub fn session_cron_store() -> Arc<dyn CronStore> {
    SESSION_CRON_STORE
        .get_or_init(|| Arc::new(InMemoryCronStore::new()))
        .clone()
}

pub fn runner_change_notify() -> Arc<tokio::sync::Notify> {
    RUNNER_CHANGE_NOTIFY
        .get_or_init(|| Arc::new(tokio::sync::Notify::new()))
        .clone()
}

pub fn notify_runner_change() {
    runner_change_notify().notify_waiters();
}

pub fn spawn_if_enabled(
    store: Arc<dyn CronStore>,
    config: SchedulerConfig,
) -> Option<JoinHandle<()>> {
    if !runner_enabled(&config) {
        return None;
    }
    // best-effort: we don't have a gcx here, so just spawn a no-op task that mirrors the
    // configured kill-switch semantics. Real spawn happens via spawn_from_active_project.
    let _ = store;
    Some(tokio::spawn(async {}))
}

pub fn scheduler_enabled() -> bool {
    std::env::var("REFACT_DISABLE_SCHEDULER").map_or(true, |value| {
        let value = value.trim();
        value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false")
    })
}

pub fn runner_enabled(config: &SchedulerConfig) -> bool {
    config.enabled && scheduler_enabled()
}

#[derive(Debug, Eq, PartialEq)]
enum ChatFireStatus {
    Fireable,
    Busy,
    Missing,
    Closed,
}

pub async fn chat_is_idle(gcx: &SharedGlobalContext, chat_id: &str) -> bool {
    chat_fire_status(gcx, chat_id).await == ChatFireStatus::Fireable
}

async fn chat_fire_status(gcx: &SharedGlobalContext, chat_id: &str) -> ChatFireStatus {
    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return ChatFireStatus::Missing;
    };
    let session = session_arc.lock().await;
    if session.closed {
        return ChatFireStatus::Closed;
    }
    if session.is_idle() && !session.command_queue.iter().any(|r| !r.priority) {
        ChatFireStatus::Fireable
    } else {
        ChatFireStatus::Busy
    }
}

fn runnable_chat_id(job: &Job) -> Result<&str, &'static str> {
    if !runner_supported_trigger(&job.trigger) {
        return Err("trigger is not supported by the chat runner yet");
    }
    match (&job.action, &job.delivery) {
        (
            Action::AgentTurn {
                target: AgentTarget::ExistingChat { chat_id },
                ..
            },
            Delivery::Chat,
        ) if !chat_id.is_empty() => Ok(chat_id),
        (Action::AgentTurn { .. }, Delivery::Chat) => Err("missing chat_id"),
        (Action::AgentTurn { .. }, _) => Err("non-chat delivery is not supported yet"),
        (Action::Command { .. }, _) => Err("command actions use the command runner"),
    }
}

fn runnable_isolated_job(job: &Job) -> Result<(), &'static str> {
    if !runner_supported_trigger(&job.trigger) {
        return Err("trigger is not supported by the chat runner yet");
    }
    match (&job.action, &job.delivery) {
        (
            Action::AgentTurn {
                target: AgentTarget::Isolated,
                ..
            },
            Delivery::Chat,
        ) => Ok(()),
        (Action::AgentTurn { .. }, Delivery::Chat) => Err("target is not isolated"),
        (Action::AgentTurn { .. }, _) => Err("non-chat delivery is not supported yet"),
        (Action::Command { .. }, _) => Err("command actions use the command runner"),
    }
}

fn runnable_command_job(job: &Job) -> Result<(), &'static str> {
    if !runner_supported_trigger(&job.trigger) {
        return Err("trigger is not supported by the command runner yet");
    }
    match (&job.action, &job.delivery) {
        (Action::Command { argv, .. }, _) if argv.is_empty() => Err("command argv is empty"),
        (
            Action::Command {
                target: AgentTarget::ExistingChat { chat_id },
                ..
            },
            Delivery::Chat,
        ) if chat_id.is_empty() => Err("missing chat_id"),
        (Action::Command { .. }, Delivery::Chat) => Ok(()),
        (Action::Command { .. }, _) => Ok(()),
        (Action::AgentTurn { .. }, _) => Err("action is not command"),
    }
}

fn runner_supported_trigger(trigger: &Trigger) -> bool {
    matches!(
        trigger,
        Trigger::Cron { .. }
            | Trigger::Interval { .. }
            | Trigger::Once { .. }
            | Trigger::Manual
            | Trigger::Webhook { .. }
    )
}

fn isolated_set_params_patch(task: &Job) -> Option<Value> {
    let mut patch = serde_json::Map::new();
    if let Some(mode) = task.mode() {
        patch.insert("mode".to_string(), json!(mode));
    }
    if let Some(model) = job_model(task) {
        patch.insert("model".to_string(), json!(model));
    }
    if patch.is_empty() {
        None
    } else {
        Some(Value::Object(patch))
    }
}

fn job_is_isolated(task: &Job) -> bool {
    matches!(
        &task.action,
        Action::AgentTurn {
            target: AgentTarget::Isolated,
            ..
        } | Action::Command {
            target: AgentTarget::Isolated,
            ..
        }
    )
}

fn command_job_has_non_chat_delivery(task: &Job) -> bool {
    matches!(&task.action, Action::Command { .. }) && !matches!(task.delivery, Delivery::Chat)
}

fn task_can_use_cron_lane(task: &Job) -> bool {
    matches!(task.action, Action::Command { .. }) || job_is_isolated(task)
}

fn job_model(task: &Job) -> Option<&str> {
    match &task.action {
        Action::AgentTurn { model, .. } => model.as_deref().filter(|model| !model.is_empty()),
        _ => None,
    }
}

#[cfg(test)]
fn set_job_isolated(task: &mut Job) {
    if let Action::AgentTurn { target, .. } = &mut task.action {
        *target = AgentTarget::Isolated;
    }
}

#[cfg(test)]
fn set_job_model(task: &mut Job, value: Option<String>) {
    if let Action::AgentTurn { model, .. } = &mut task.action {
        *model = value;
    }
}

fn cron_fire_message(task: &Job, final_fire: bool) -> ChatMessage {
    cron_fire_message_with_missed(task, final_fire, false)
}

fn cron_fire_message_with_missed(task: &Job, final_fire: bool, missed: bool) -> ChatMessage {
    let mut payload = json!({
        "task_id": task.id,
        "cron": task.cron_expr().unwrap_or_default(),
        "recurring": task.recurring,
        "fire_count": event_fire_count(task),
        "final": final_fire,
        "action_kind": task.action_kind(),
    });
    if missed {
        payload["missed"] = json!(true);
    }
    event(
        EventSubkind::CronFire,
        "scheduler.cron",
        payload,
        task.prompt().unwrap_or(&task.description).to_string(),
    )
}

fn event_fire_count(task: &Job) -> u32 {
    if task
        .last_fired_at_ms
        .is_some_and(|fired_at_ms| job_was_advanced_before_fire(task, fired_at_ms))
    {
        task.fire_count
    } else {
        task.fire_count.saturating_add(1)
    }
}

fn job_was_advanced_before_fire(task: &Job, fired_at_ms: u64) -> bool {
    task.recurring
        && task.last_status.as_deref() == Some("advanced")
        && task
            .last_fired_at_ms
            .is_some_and(|last_fired_at_ms| last_fired_at_ms <= fired_at_ms)
}

fn command_record_error(result: &CommandRunResult) -> String {
    command_error_summary(result)
}

fn command_failure_can_retry(result: &CommandRunResult) -> bool {
    classify(&command_retry_text(result)).is_some()
}

fn command_retry_text(result: &CommandRunResult) -> String {
    let mut text = String::new();
    if result.stderr.to_ascii_lowercase().contains("timed out") {
        text.push_str("timeout\n");
    }
    text.push_str(&result.stderr);
    if !result.stdout.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&result.stdout);
    }
    if text.trim().is_empty() {
        command_error_summary(result)
    } else {
        text
    }
}

fn catch_up_notice_message(missed_count: u64) -> ChatMessage {
    event(
        EventSubkind::SystemNotice,
        "scheduler.cron",
        json!({ "missed_count": missed_count }),
        format!("Caught up {missed_count} missed scheduled tasks"),
    )
}

fn auto_expired_notice_message(task: &Job) -> ChatMessage {
    event(
        EventSubkind::SystemNotice,
        "scheduler.cron",
        json!({
            "task_id": task.id,
            "reason": "auto_expired",
        }),
        format!(
            "Recurring task '{}' auto-expired after {}d",
            task.description,
            task.auto_expire_after_ms / DAY_MS
        ),
    )
}

pub(crate) fn scheduled_fire_at_ms(
    task: &Job,
    now: u64,
    jitter_cfg: &JitterConfig,
    grace_config: MissedRunGraceConfig,
) -> Option<u64> {
    if let Some(trigger_at_ms) = task.trigger_at_ms {
        if task.is_paused() {
            return Some(trigger_at_ms);
        }
    }
    let tz = scheduler_timezone();
    let from_ms = task.last_fired_at_ms.unwrap_or(task.created_at_ms);
    let recurring_state = if task.recurring
        && matches!(
            task.trigger,
            Trigger::Cron { .. } | Trigger::Interval { .. }
        ) {
        recurring_missed_grace_state(task, from_ms, now, tz, grace_config)
    } else {
        None
    };
    if recurring_state
        .as_ref()
        .is_some_and(|state| state.due_ms.is_some() && !state.should_fire)
    {
        return recurring_state.map(|state| state.next_future_ms);
    }
    let scheduled = match &task.trigger {
        Trigger::Cron { .. } if task.recurring => {
            jittered_next_run_ms_for(task, from_ms, &task.id, jitter_cfg, tz)
        }
        Trigger::Cron { .. } => {
            one_shot_jittered_next_run_ms_for(task, from_ms, &task.id, jitter_cfg, tz)
        }
        Trigger::Interval { .. } | Trigger::Once { .. } => next_run_ms(task, from_ms, tz),
        Trigger::Manual | Trigger::Webhook { .. } | Trigger::OnProcessExit { .. } => None,
    };
    let scheduled = if task.recurring || task.last_fired_at_ms.is_none() {
        scheduled
    } else {
        None
    };
    match (scheduled, task.trigger_at_ms) {
        (Some(scheduled), Some(trigger_at_ms)) => Some(scheduled.min(trigger_at_ms)),
        (Some(scheduled), None) => Some(scheduled),
        (None, Some(trigger_at_ms)) => Some(trigger_at_ms),
        (None, None) => None,
    }
}

async fn record_run(
    store: &Arc<dyn CronStore>,
    job_id: &str,
    status: &str,
    error: Option<String>,
    now_ms: u64,
    recent_runs_cap: usize,
) -> Result<(), String> {
    let mut task = store
        .get(job_id)
        .await
        .ok_or_else(|| format!("Scheduled task {job_id} not found"))?;
    let already_advanced = job_was_advanced_before_fire(&task, now_ms);
    if already_advanced {
        if let Some(run) = task
            .recent_runs
            .last_mut()
            .filter(|run| run.at_ms == now_ms)
        {
            run.status = status.to_string();
            run.error = error.clone();
        } else {
            task.recent_runs.push(CronRunRecord {
                at_ms: now_ms,
                status: status.to_string(),
                error: error.clone(),
            });
            cap_recent_runs(&mut task, recent_runs_cap);
        }
    } else {
        task.recent_runs.push(CronRunRecord {
            at_ms: now_ms,
            status: status.to_string(),
            error: error.clone(),
        });
        cap_recent_runs(&mut task, recent_runs_cap);
    }
    task.last_status = Some(status.to_string());
    task.last_error = error;
    if status == "fired" || status == "error" {
        if !already_advanced {
            task.last_fired_at_ms = Some(now_ms);
            task.fire_count = task.fire_count.saturating_add(1);
        }
        if status == "fired" {
            task.retry_attempts = 0;
        }
        if task
            .trigger_at_ms
            .is_some_and(|trigger_at_ms| trigger_at_ms <= now_ms)
        {
            task.trigger_at_ms = None;
        }
    }
    if store.replace(task).await? {
        Ok(())
    } else {
        Err(format!("Scheduled task {job_id} not found"))
    }
}

fn cap_recent_runs(task: &mut Job, cap: usize) {
    if cap == 0 {
        task.recent_runs.clear();
        return;
    }
    if task.recent_runs.len() > cap {
        task.recent_runs.drain(0..task.recent_runs.len() - cap);
    }
}

fn missed_one_shot_task(task: &Job, now: u64) -> bool {
    !task.recurring
        && matches!(task.trigger, Trigger::Cron { .. })
        && task.last_fired_at_ms.is_none()
        && task.fire_count == 0
        && next_run_ms(task, task.created_at_ms, scheduler_timezone())
            .is_some_and(|next| next < now)
}

fn task_final_after_fire(task: &Job, now: u64) -> bool {
    task.recurring
        && task.auto_expire_after_ms > 0
        && now.saturating_sub(task.created_at_ms) > task.auto_expire_after_ms
}

fn now_ms() -> u64 {
    Utc::now().timestamp_millis().max(0) as u64
}

async fn wait_for_shutdown(shutdown_flag: Arc<AtomicBool>) {
    while !shutdown_flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use chrono::TimeZone;
    use tokio::sync::Mutex as AMutex;

    use super::*;
    use crate::chat::internal_roles::EVENT_ROLE;
    use crate::chat::types::{ChatSession, SessionState};
    use crate::scheduler::store::InMemoryCronStore;
    use crate::scheduler::types::{DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS, RECENT_RUNS_CAP};

    fn task(id: &str, now: u64) -> Job {
        let mut task = Job::new_cron_agent_chat(
            "*/1 * * * *".to_string(),
            "scheduled prompt".to_string(),
            "scheduled prompt".to_string(),
            true,
            false,
            now - 120_000,
        );
        task.id = id.to_string();
        task.set_existing_chat(Some("chat-1".to_string()));
        task.set_mode(Some("agent".to_string()));
        task.last_fired_at_ms = Some(now - 120_000);
        task.auto_expire_after_ms = DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS;
        task
    }

    fn expired_task(id: &str, now: u64) -> Job {
        let mut task = task(id, now);
        task.created_at_ms = now - 2 * DAY_MS - 1;
        task.auto_expire_after_ms = 2 * DAY_MS;
        task
    }

    fn one_shot_task(id: &str, now: u64) -> Job {
        let mut task = task(id, now);
        task.recurring = false;
        task.last_fired_at_ms = None;
        task.auto_expire_after_ms = 0;
        task
    }

    fn due_task(id: &str, now: u64) -> Job {
        let mut task = task(id, now);
        task.created_at_ms = now - 120_000;
        task.last_fired_at_ms = Some(now - 120_000);
        task
    }

    fn interval_task(id: &str, now: u64, every_ms: u64) -> Job {
        let mut task = task(id, now);
        task.trigger = Trigger::Interval { every_ms };
        task.recurring = true;
        task.created_at_ms = now - every_ms * 2;
        task.last_fired_at_ms = Some(now - every_ms);
        task
    }

    fn once_trigger_task(id: &str, now: u64) -> Job {
        let mut task = task(id, now);
        task.trigger = Trigger::Once {
            at_ms: now.saturating_sub(1_000),
        };
        task.recurring = false;
        task.created_at_ms = now.saturating_sub(60_000);
        task.last_fired_at_ms = None;
        task.auto_expire_after_ms = 0;
        task
    }

    fn isolated_due_task(id: &str, now: u64) -> Job {
        let mut task = due_task(id, now);
        set_job_isolated(&mut task);
        set_job_model(&mut task, Some("model-1".to_string()));
        task
    }

    fn isolated_interval_task(id: &str, now: u64, every_ms: u64) -> Job {
        let mut task = interval_task(id, now, every_ms);
        set_job_isolated(&mut task);
        set_job_model(&mut task, Some("model-1".to_string()));
        task
    }

    fn command_due_task(id: &str, now: u64, argv: Vec<String>) -> Job {
        let mut task = due_task(id, now);
        task.description = format!("{id} command");
        task.action = Action::Command {
            argv,
            target: AgentTarget::ExistingChat {
                chat_id: "chat-1".to_string(),
            },
            cwd: None,
            env: None,
            timeout_secs: Some(5),
        };
        task
    }

    #[cfg(not(target_os = "windows"))]
    fn stdout_command(text: &str) -> Vec<String> {
        vec!["printf".to_string(), text.to_string()]
    }

    #[cfg(target_os = "windows")]
    fn stdout_command(text: &str) -> Vec<String> {
        vec!["Write-Output".to_string(), text.to_string()]
    }

    #[cfg(not(target_os = "windows"))]
    fn empty_command() -> Vec<String> {
        vec!["true".to_string()]
    }

    #[cfg(target_os = "windows")]
    fn empty_command() -> Vec<String> {
        vec!["cmd".to_string(), "/C".to_string(), "exit 0".to_string()]
    }

    #[cfg(not(target_os = "windows"))]
    fn slow_empty_command() -> Vec<String> {
        vec!["sh".to_string(), "-c".to_string(), "sleep 0.2".to_string()]
    }

    #[cfg(target_os = "windows")]
    fn slow_empty_command() -> Vec<String> {
        vec![
            "Start-Sleep".to_string(),
            "-Milliseconds".to_string(),
            "200".to_string(),
        ]
    }

    #[cfg(not(target_os = "windows"))]
    fn failing_command() -> Vec<String> {
        vec!["sh".to_string(), "-c".to_string(), "exit 7".to_string()]
    }

    #[cfg(target_os = "windows")]
    fn failing_command() -> Vec<String> {
        vec!["cmd".to_string(), "/C".to_string(), "exit 7".to_string()]
    }

    fn one_shot_command_due_task(id: &str, now: u64, argv: Vec<String>) -> Job {
        let mut task = command_due_task(id, now, argv);
        task.trigger = Trigger::Once {
            at_ms: now.saturating_sub(1_000),
        };
        task.recurring = false;
        task.last_fired_at_ms = None;
        task.auto_expire_after_ms = 0;
        task
    }

    #[cfg(not(target_os = "windows"))]
    fn transient_failing_command() -> Vec<String> {
        vec![
            "sh".to_string(),
            "-c".to_string(),
            "printf '429 Too Many Requests\\n' >&2; exit 7".to_string(),
        ]
    }

    #[cfg(target_os = "windows")]
    fn transient_failing_command() -> Vec<String> {
        vec![
            "cmd".to_string(),
            "/C".to_string(),
            "echo 429 Too Many Requests 1>&2 & exit 7".to_string(),
        ]
    }

    #[test]
    fn command_timeout_failure_is_retryable() {
        let result = CommandRunResult {
            status: "error".to_string(),
            stdout: String::new(),
            stderr: "command timed out after 1 seconds".to_string(),
            exit_code: None,
        };

        assert!(command_failure_can_retry(&result));
    }

    fn timezone_cron_task(id: &str, created_at_ms: u64) -> Job {
        let mut task = Job::new_cron_agent_chat(
            "0 9 * * *".to_string(),
            "scheduled prompt".to_string(),
            "scheduled prompt".to_string(),
            true,
            false,
            created_at_ms,
        );
        task.id = id.to_string();
        task.trigger = Trigger::Cron {
            expr: "0 9 * * *".to_string(),
            tz: Some("Asia/Kolkata".to_string()),
        };
        task.set_existing_chat(Some("chat-1".to_string()));
        task.set_mode(Some("agent".to_string()));
        task.auto_expire_after_ms = DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS;
        task
    }

    fn utc_ms(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> u64 {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64
    }

    async fn gcx_with_session(state: SessionState) -> SharedGlobalContext {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut session = ChatSession::new("chat-1".to_string());
        session.set_runtime_state(state, None);
        gcx.chat_sessions
            .write()
            .await
            .insert("chat-1".to_string(), Arc::new(AMutex::new(session)));
        gcx
    }

    async fn gcx_with_session_and_config<F>(
        state: SessionState,
        configure: F,
    ) -> SharedGlobalContext
    where
        F: FnOnce(&mut crate::scheduler::types::SchedulerConfig),
    {
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::get_mut(&mut gcx).unwrap().scheduler_config =
            crate::scheduler::types::test_scheduler_config_with(configure);
        let mut session = ChatSession::new("chat-1".to_string());
        session.set_runtime_state(state, None);
        gcx.chat_sessions
            .write()
            .await
            .insert("chat-1".to_string(), Arc::new(AMutex::new(session)));
        gcx
    }

    async fn gcx_with_closed_session() -> SharedGlobalContext {
        let gcx = gcx_with_session(SessionState::Idle).await;
        let session = session(&gcx).await;
        session.lock().await.close_event_channel();
        gcx
    }

    async fn session(gcx: &SharedGlobalContext) -> Arc<AMutex<ChatSession>> {
        gcx.chat_sessions
            .read()
            .await
            .get("chat-1")
            .cloned()
            .unwrap()
    }

    async fn isolated_session_ids(gcx: &SharedGlobalContext, task_id: &str) -> Vec<String> {
        let prefix = format!("cron_{task_id}_");
        let sessions = gcx.chat_sessions.read().await;
        let mut ids = sessions
            .keys()
            .filter(|id| id.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    async fn assert_isolated_prompt(gcx: &SharedGlobalContext, chat_id: &str, task_id: &str) {
        let session_arc = {
            let sessions = gcx.chat_sessions.read().await;
            sessions.get(chat_id).cloned().unwrap()
        };
        let deadline = TokioInstant::now() + Duration::from_secs(2);
        loop {
            {
                let session = session_arc.lock().await;
                let fire_event = session.messages.iter().any(|message| {
                    message.role == EVENT_ROLE
                        && message.extra["event"]["subkind"].as_str() == Some("cron_fire")
                        && message.extra["event"]["payload"]["task_id"].as_str() == Some(task_id)
                });
                let prompt_queued = session.command_queue.iter().any(|request| {
                    matches!(
                        &request.command,
                        ChatCommand::UserMessage { content, .. }
                            if content.as_str() == Some("scheduled prompt")
                    )
                });
                let prompt_added = session.messages.iter().any(|message| {
                    message.role == "user"
                        && message.content.content_text_only() == "scheduled prompt"
                });
                if fire_event && (prompt_queued || prompt_added) {
                    return;
                }
            }
            assert!(
                TokioInstant::now() < deadline,
                "isolated session must receive the scheduled prompt"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn event_message<'a>(
        session: &'a ChatSession,
        subkind: &str,
        task_id: &str,
    ) -> &'a crate::call_validation::ChatMessage {
        session
            .messages
            .iter()
            .find(|message| {
                message.role == EVENT_ROLE
                    && message.extra["event"]["subkind"].as_str() == Some(subkind)
                    && message.extra["event"]["payload"]["task_id"].as_str() == Some(task_id)
            })
            .unwrap()
    }

    async fn wait_for_fire(gcx: &SharedGlobalContext) {
        let deadline = TokioInstant::now() + Duration::from_secs(2);
        loop {
            {
                let session = session(gcx).await;
                let session = session.lock().await;
                let event_injected = session
                    .messages
                    .iter()
                    .any(|message| message.role == EVENT_ROLE);
                let prompt_queued = session.command_queue.iter().any(|request| {
                    matches!(
                        &request.command,
                        ChatCommand::UserMessage { content, .. }
                            if content.as_str() == Some("scheduled prompt")
                    )
                });
                let prompt_added = session.messages.iter().any(|message| {
                    message.role == "user"
                        && message.content.content_text_only() == "scheduled prompt"
                });
                if event_injected && (prompt_queued || prompt_added) {
                    return;
                }
            }
            assert!(
                TokioInstant::now() < deadline,
                "scheduled task did not fire"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn assert_no_fire(gcx: &SharedGlobalContext) -> impl std::future::Future<Output = ()> + '_ {
        async move {
            let session = session(gcx).await;
            let session = session.lock().await;
            assert!(session.messages.is_empty());
            assert!(session.command_queue.is_empty());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn fires_due_task() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store.add(task("cron_fire_due", now)).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let handle = spawn(store.clone(), gcx.clone());

        tokio::time::advance(Duration::from_secs(2)).await;
        wait_for_fire(&gcx).await;
        gcx.shutdown_flag.store(true, Ordering::Relaxed);
        handle.abort();

        let session = session(&gcx).await;
        let session = session.lock().await;
        let event_message = session
            .messages
            .iter()
            .find(|message| message.role == EVENT_ROLE)
            .unwrap();
        assert_eq!(
            event_message.extra["event"]["payload"]["task_id"],
            json!("cron_fire_due")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn session_store_runner_fires_session_task() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store.add(task("cron_session_fire", now)).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let handle = spawn(store, gcx.clone());

        tokio::time::advance(Duration::from_secs(2)).await;
        wait_for_fire(&gcx).await;
        gcx.shutdown_flag.store(true, Ordering::Relaxed);
        handle.abort();

        let session = session(&gcx).await;
        let session = session.lock().await;
        let event_message = event_message(&session, "cron_fire", "cron_session_fire");
        assert_eq!(
            event_message.extra["event"]["payload"]["recurring"],
            json!(true)
        );
    }

    #[tokio::test]
    async fn one_shot_removed_after_normal_fire() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(one_shot_task("cron_one_shot_removed", now))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        assert!(store.list().await.is_empty());
        let session = session(&gcx).await;
        let session = session.lock().await;
        let fire_event = event_message(&session, "cron_fire", "cron_one_shot_removed");
        assert_eq!(fire_event.extra["event"]["payload"]["final"], json!(false));
        assert_eq!(
            fire_event.extra["event"]["payload"]["recurring"],
            json!(false)
        );
    }

    #[tokio::test]
    async fn once_trigger_fires_and_is_removed() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(once_trigger_task("once_trigger_due", now))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        assert!(store.list().await.is_empty());
        let session = session(&gcx).await;
        let session = session.lock().await;
        let fire_event = event_message(&session, "cron_fire", "once_trigger_due");
        assert_eq!(
            fire_event.extra["event"]["payload"]["recurring"],
            json!(false)
        );
    }

    #[tokio::test]
    async fn disabled_scheduler_does_not_fire_due_task() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store.add(due_task("cron_disabled", now)).await.unwrap();
        let gcx = gcx_with_session_and_config(SessionState::Idle, |config| {
            config.enabled = false;
        })
        .await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("cron_disabled").await.unwrap();
        assert_eq!(stored.last_fired_at_ms, Some(now - 120_000));
        assert_eq!(stored.fire_count, 0);
        assert!(stored.recent_runs.is_empty());
        assert_no_fire(&gcx).await;
    }

    #[tokio::test]
    async fn one_shot_remains_when_enqueue_is_rejected() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(one_shot_task("cron_one_shot_queue_full", now))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        {
            let session_arc = session(&gcx).await;
            let mut session = session_arc.lock().await;
            for idx in 0..max_queue_size() {
                session.command_queue.push_back(CommandRequest {
                    client_request_id: format!("queued-priority-{idx}"),
                    priority: true,
                    command: ChatCommand::SetParams {
                        patch: json!({"temperature": idx}),
                    },
                });
            }
        }
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("cron_one_shot_queue_full").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("deferred"));
        assert_eq!(stored.fire_count, 0);
        assert_eq!(stored.last_fired_at_ms, None);
        assert!(runner
            .deferred_until_ms
            .contains_key("cron_one_shot_queue_full"));
        let session = session(&gcx).await;
        let session = session.lock().await;
        assert!(session.messages.iter().all(|m| m.role != EVENT_ROLE));
    }

    #[tokio::test]
    async fn enqueue_rejection_does_not_mark_recurring_fired() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(due_task("cron_recurring_queue_full", now))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        {
            let session_arc = session(&gcx).await;
            let mut session = session_arc.lock().await;
            for idx in 0..max_queue_size() {
                session.command_queue.push_back(CommandRequest {
                    client_request_id: format!("queued-recurring-{idx}"),
                    priority: true,
                    command: ChatCommand::SetParams {
                        patch: json!({"temperature": idx}),
                    },
                });
            }
        }
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("cron_recurring_queue_full").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("deferred"));
        assert_eq!(stored.fire_count, 0);
        assert_eq!(stored.last_fired_at_ms, Some(now - 120_000));
        let session = session(&gcx).await;
        let session = session.lock().await;
        assert!(session.messages.iter().all(|m| m.role != EVENT_ROLE));
    }

    #[tokio::test]
    async fn interval_job_fires_on_cadence() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(interval_task("interval_due", now, 60_000))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("interval_due").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert_eq!(stored.last_fired_at_ms, Some(now));
        assert_eq!(stored.fire_count, 1);
        assert!(!runner.task_is_due(&stored, now + 59_999));
        assert!(runner.task_is_due(&stored, now + 60_000));
    }

    #[tokio::test]
    async fn overdue_recurring_job_fast_forwards_without_burst() {
        let now = now_ms();
        let every_ms = 60_000;
        let store = Arc::new(InMemoryCronStore::new());
        let mut task = interval_task("interval_overdue_fast_forward", now, every_ms);
        task.last_fired_at_ms = Some(now - 10 * every_ms);
        store.add(task).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("interval_overdue_fast_forward").await.unwrap();
        assert_eq!(stored.last_fired_at_ms, Some(now));
        assert_eq!(stored.fire_count, 0);
        assert!(stored.recent_runs.is_empty());
        assert_no_fire(&gcx).await;
        assert!(!runner.task_is_due(&stored, now + every_ms - 1));
        assert!(runner.task_is_due(&stored, now + every_ms));
    }

    #[tokio::test]
    async fn command_job_runs_and_delivers_stdout() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(command_due_task(
                "command_stdout",
                now,
                stdout_command("hello command"),
            ))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("command_stdout").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert_eq!(stored.fire_count, 1);
        let session = session(&gcx).await;
        let session = session.lock().await;
        let fire_event = event_message(&session, "cron_fire", "command_stdout");
        assert_eq!(
            fire_event.extra["event"]["payload"]["action_kind"],
            json!("command")
        );
        let output = session
            .messages
            .iter()
            .find(|message| message.role == "plain_text")
            .expect("stdout message");
        assert_eq!(
            output.content.content_text_only().trim_end(),
            "hello command"
        );
    }

    #[tokio::test]
    async fn command_job_empty_stdout_is_silent_but_fired() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(command_due_task("command_silent", now, empty_command()))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("command_silent").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert_eq!(stored.fire_count, 1);
        let session = session(&gcx).await;
        let session = session.lock().await;
        assert!(session.messages.is_empty());
        assert!(session.command_queue.is_empty());
    }

    #[tokio::test]
    async fn recurring_command_advances_before_fire_without_double_counting() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(command_due_task(
                "command_advance_before_fire",
                now,
                stdout_command("advanced"),
            ))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("command_advance_before_fire").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert!(stored
            .last_fired_at_ms
            .is_some_and(|last_fired_at_ms| last_fired_at_ms <= now));
        assert_eq!(stored.fire_count, 1);
        assert_eq!(stored.recent_runs.len(), 1);
        assert_eq!(stored.recent_runs[0].status, "fired");
        let session = session(&gcx).await;
        let session = session.lock().await;
        let fire_event = event_message(&session, "cron_fire", "command_advance_before_fire");
        assert_eq!(fire_event.extra["event"]["payload"]["fire_count"], json!(1));
    }

    #[tokio::test]
    async fn concurrency_cap_defers_over_limit_command_runs() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        for id in ["command_cap_a", "command_cap_b", "command_cap_c"] {
            store
                .add(command_due_task(id, now, slow_empty_command()))
                .await
                .unwrap();
        }
        let gcx = gcx_with_session_and_config(SessionState::Idle, |config| {
            config.max_concurrent_runs = 2;
        })
        .await;
        let mut runner = CronRunner::new(store.clone(), gcx);

        runner.fire_due_tasks(now).await;

        let jobs = store.list().await;
        let fired = jobs
            .iter()
            .filter(|job| job.last_status.as_deref() == Some("fired"))
            .count();
        let pending = jobs
            .iter()
            .filter(|job| job.last_status.as_deref() != Some("fired"))
            .collect::<Vec<_>>();
        assert_eq!(fired, 2);
        assert_eq!(pending.len(), 1);
        assert_eq!(
            runner.deferred_until_ms.get(&pending[0].id),
            Some(&(now + IDLE_DEFER_MS))
        );
    }

    #[tokio::test]
    async fn command_job_nonzero_exit_records_error() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(command_due_task("command_error", now, failing_command()))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("command_error").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("error"));
        assert_eq!(stored.fire_count, 1);
        assert_eq!(stored.retry_attempts, 0);
        assert_eq!(stored.trigger_at_ms, None);
        assert!(stored
            .last_error
            .as_deref()
            .unwrap_or("")
            .contains("Scheduled command failed"));
        let session = session(&gcx).await;
        let session = session.lock().await;
        let notice = event_message(&session, "system_notice", "command_error");
        assert_eq!(
            notice.extra["event"]["payload"]["action_kind"],
            json!("command")
        );
    }

    #[tokio::test]
    async fn transient_command_failure_retries_until_max_then_gives_up() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(one_shot_command_due_task(
                "command_transient_retry",
                now,
                transient_failing_command(),
            ))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx);

        runner.fire_due_tasks(now).await;
        let stored = store.get("command_transient_retry").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("error"));
        assert_eq!(stored.fire_count, 1);
        assert_eq!(stored.retry_attempts, 1);
        assert_eq!(stored.trigger_at_ms, Some(now + 60_000));
        assert!(!runner.task_is_due(&stored, now + 59_999));

        runner.fire_due_tasks(now + 60_000).await;
        let stored = store.get("command_transient_retry").await.unwrap();
        assert_eq!(stored.fire_count, 2);
        assert_eq!(stored.retry_attempts, 2);
        assert_eq!(stored.trigger_at_ms, Some(now + 180_000));

        runner.fire_due_tasks(now + 180_000).await;
        let stored = store.get("command_transient_retry").await.unwrap();
        assert_eq!(stored.fire_count, 3);
        assert_eq!(stored.retry_attempts, 3);
        assert_eq!(stored.trigger_at_ms, Some(now + 480_000));

        runner.fire_due_tasks(now + 480_000).await;

        assert!(store.get("command_transient_retry").await.is_none());
    }

    #[tokio::test]
    async fn command_success_resets_retry_backoff() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut task = command_due_task("command_retry_reset", now, stdout_command("ok"));
        task.retry_attempts = 2;
        task.trigger_at_ms = Some(now);
        store.add(task).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx);

        runner.fire_due_tasks(now).await;

        let stored = store.get("command_retry_reset").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert_eq!(stored.retry_attempts, 0);
        assert_eq!(stored.trigger_at_ms, None);
    }

    #[tokio::test]
    async fn command_delivery_failure_records_separate_error_without_retry() {
        let router = axum::Router::new().route(
            "/hook",
            axum::routing::post(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = axum::Server::from_tcp(listener.into_std().unwrap())
            .unwrap()
            .serve(router.into_make_service());
        let server_task = tokio::spawn(server);
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut task = command_due_task("command_delivery_error", now, empty_command());
        task.delivery = Delivery::Webhook {
            url: format!("http://127.0.0.1:{port}/hook"),
            token: None,
        };
        store.add(task).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx);

        runner.fire_due_tasks(now).await;

        let stored = store.get("command_delivery_error").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert_eq!(stored.last_error, None);
        assert!(stored
            .last_delivery_error
            .as_deref()
            .unwrap_or_default()
            .contains("webhook delivery returned status"));
        assert_eq!(stored.retry_attempts, 0);
        assert_eq!(stored.trigger_at_ms, None);
        assert_eq!(stored.recent_runs.len(), 1);
        assert_eq!(stored.recent_runs[0].status, "fired");
        assert_eq!(stored.recent_runs[0].error, None);
        server_task.abort();
    }

    #[tokio::test]
    async fn timezone_cron_job_fires_at_local_time() {
        let now = utc_ms(2026, 1, 1, 3, 30);
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(timezone_cron_task("tz_cron_due", utc_ms(2026, 1, 1, 3, 29)))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());
        runner.jitter_cfg.recurring_frac = 0.0;

        runner.fire_due_tasks(now).await;

        let stored = store.get("tz_cron_due").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert_eq!(stored.last_fired_at_ms, Some(now));
        let session = session(&gcx).await;
        let session = session.lock().await;
        let fire_event = event_message(&session, "cron_fire", "tz_cron_due");
        assert_eq!(
            fire_event.extra["event"]["payload"]["task_id"],
            json!("tz_cron_due")
        );
    }

    #[tokio::test]
    async fn paused_job_never_fires_without_trigger_at() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut task = due_task("paused_cron", now);
        task.enabled = false;
        task.paused_at_ms = Some(now - 1_000);
        store.add(task).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("paused_cron").await.unwrap();
        assert_eq!(stored.fire_count, 0);
        assert!(stored.recent_runs.is_empty());
        assert_no_fire(&gcx).await;
    }

    #[tokio::test]
    async fn trigger_at_fires_paused_job_once_and_clears() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut task = due_task("run_now_paused", now);
        task.enabled = false;
        task.paused_at_ms = Some(now - 1_000);
        task.trigger_at_ms = Some(now);
        store.add(task).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("run_now_paused").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert_eq!(stored.fire_count, 1);
        assert_eq!(stored.trigger_at_ms, None);
        assert!(!runner.task_is_due(&stored, now + 60_000));
    }

    #[tokio::test]
    async fn trigger_at_overrides_existing_defer_once() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut task = due_task("run_now_deferred", now);
        task.trigger_at_ms = Some(now);
        store.add(task.clone()).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());
        runner
            .deferred_until_ms
            .insert(task.id.clone(), now + IDLE_DEFER_MS);

        runner.fire_due_tasks(now).await;

        let stored = store.get("run_now_deferred").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert_eq!(stored.trigger_at_ms, None);
        assert!(!runner.deferred_until_ms.contains_key("run_now_deferred"));
    }

    #[tokio::test]
    async fn record_run_caps_history_and_updates_status() {
        let now = now_ms();
        let store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let mut task = due_task("history_cap", now);
        task.fire_count = 7;
        task.recent_runs = (0..RECENT_RUNS_CAP)
            .map(|idx| CronRunRecord {
                at_ms: idx as u64,
                status: "old".to_string(),
                error: None,
            })
            .collect();
        store.add(task).await.unwrap();

        record_run(&store, "history_cap", "fired", None, now, RECENT_RUNS_CAP)
            .await
            .unwrap();
        record_run(
            &store,
            "history_cap",
            "deferred",
            Some("busy".to_string()),
            now + 1,
            RECENT_RUNS_CAP,
        )
        .await
        .unwrap();

        let stored = store.get("history_cap").await.unwrap();
        assert_eq!(stored.recent_runs.len(), RECENT_RUNS_CAP);
        assert_eq!(stored.recent_runs[0].at_ms, 2);
        assert_eq!(stored.recent_runs[RECENT_RUNS_CAP - 1].at_ms, now + 1);
        assert_eq!(stored.last_status.as_deref(), Some("deferred"));
        assert_eq!(stored.last_error.as_deref(), Some("busy"));
        assert_eq!(stored.last_fired_at_ms, Some(now));
        assert_eq!(stored.fire_count, 8);
    }

    #[tokio::test]
    async fn runner_uses_configurable_recent_run_cap() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(due_task("history_config_cap", now))
            .await
            .unwrap();
        let gcx = gcx_with_session_and_config(SessionState::Idle, |config| {
            config.recent_runs_cap = 1;
        })
        .await;
        let runner = CronRunner::new(store.clone(), gcx);

        runner
            .record_run("history_config_cap", "fired", None, now)
            .await
            .unwrap();
        runner
            .record_run("history_config_cap", "deferred", None, now + 1)
            .await
            .unwrap();

        let stored = store.get("history_config_cap").await.unwrap();
        assert_eq!(stored.recent_runs.len(), 1);
        assert_eq!(stored.recent_runs[0].at_ms, now + 1);
        assert_eq!(stored.recent_runs[0].status, "deferred");
    }

    #[tokio::test]
    async fn due_task_without_chat_id_does_not_spin() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut one_shot = one_shot_task("cron_no_chat_one_shot", now);
        one_shot.set_existing_chat(None);
        store.add(one_shot).await.unwrap();
        let mut recurring = due_task("cron_no_chat_recurring", now);
        recurring.set_existing_chat(None);
        store.add(recurring).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.list().await;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, "cron_no_chat_recurring");
        assert_eq!(
            runner.deferred_until_ms["cron_no_chat_recurring"],
            now + INVALID_TARGET_DEFER_MS
        );
        assert!(!runner.task_is_due(&stored[0], now + INVALID_TARGET_DEFER_MS - 1));
    }

    #[tokio::test]
    async fn missing_chat_one_shot_removed_or_deferred() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut one_shot = one_shot_task("cron_missing_chat_one_shot", now);
        one_shot.set_existing_chat(Some("missing-chat".to_string()));
        store.add(one_shot).await.unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut runner = CronRunner::new(store.clone(), gcx);

        runner.fire_due_tasks(now).await;

        assert!(store.list().await.is_empty());
        assert!(runner.deferred_until_ms.is_empty());
    }

    #[tokio::test]
    async fn closed_chat_task_does_not_hot_loop() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(due_task("cron_closed_chat_recurring", now))
            .await
            .unwrap();
        let gcx = gcx_with_closed_session().await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.list().await.into_iter().next().unwrap();
        assert_eq!(
            runner.deferred_until_ms["cron_closed_chat_recurring"],
            now + INVALID_TARGET_DEFER_MS
        );
        assert!(!runner.task_is_due(&stored, now + INVALID_TARGET_DEFER_MS - 1));
        assert!(!chat_is_idle(&gcx, "chat-1").await);
        let session = session(&gcx).await;
        let session = session.lock().await;
        assert!(session.messages.is_empty());
        assert!(session.command_queue.is_empty());
    }

    #[tokio::test]
    async fn spawn_from_active_project_starts_session_runner_without_project() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.shutdown_flag.store(true, Ordering::Relaxed);

        let handles = spawn_from_active_project(gcx).await;

        assert_eq!(handles.len(), 1);
        for handle in handles {
            handle.await.unwrap();
        }
    }

    #[tokio::test]
    async fn idle_gate_defers_when_generating() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store.add(task("cron_defer", now)).await.unwrap();
        let gcx = gcx_with_session(SessionState::Generating).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.list().await.into_iter().next().unwrap();
        assert_eq!(stored.last_fired_at_ms, Some(now - 120_000));
        assert_eq!(stored.fire_count, 0);
        assert!(runner.deferred_until_ms["cron_defer"] >= now + IDLE_DEFER_MS);
        let session = session(&gcx).await;
        let session = session.lock().await;
        assert!(session.messages.is_empty());
        assert!(session.command_queue.is_empty());
    }

    #[tokio::test]
    async fn isolated_due_job_creates_fresh_session_and_skips_idle_gate() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(isolated_due_task("cron_isolated_due", now))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Generating).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("cron_isolated_due").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert!(stored
            .last_fired_at_ms
            .is_some_and(|last_fired_at_ms| last_fired_at_ms <= now));
        assert_eq!(stored.fire_count, 1);
        let ids = isolated_session_ids(&gcx, "cron_isolated_due").await;
        assert_eq!(ids, vec![format!("cron_cron_isolated_due_{now}")]);
        assert_isolated_prompt(&gcx, &ids[0], "cron_isolated_due").await;
        let existing = session(&gcx).await;
        let existing = existing.lock().await;
        assert!(existing.messages.is_empty());
        assert!(existing.command_queue.is_empty());
    }

    #[tokio::test]
    async fn recurring_isolated_job_creates_distinct_session_per_fire() {
        let now = now_ms();
        let every_ms = 60_000;
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(isolated_interval_task(
                "cron_isolated_recurring",
                now,
                every_ms,
            ))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;
        runner.fire_due_tasks(now + every_ms).await;

        let ids = isolated_session_ids(&gcx, "cron_isolated_recurring").await;
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&format!("cron_cron_isolated_recurring_{now}")));
        assert!(ids.contains(&format!("cron_cron_isolated_recurring_{}", now + every_ms)));
        assert_ne!(ids[0], ids[1]);
        let stored = store.get("cron_isolated_recurring").await.unwrap();
        assert_eq!(stored.fire_count, 2);
    }

    #[tokio::test]
    async fn recurring_auto_expires_after_horizon() {
        let now = now_ms();
        let mut task = task("cron_expire", now);
        task.created_at_ms = now - DAY_MS;
        task.auto_expire_after_ms = DAY_MS;

        assert!(!task_final_after_fire(&task, now));
        task.created_at_ms -= 1;
        assert!(task_final_after_fire(&task, now));
        task.recurring = false;
        assert!(!task_final_after_fire(&task, now));
    }

    #[tokio::test]
    async fn webhook_trigger_with_zero_auto_expire_survives_fire() {
        let now = now_ms();
        let store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut task = command_due_task("cron_webhook_no_expire", now, empty_command());
        task.set_trigger(Trigger::Webhook {
            hook_id: "deploy".to_string(),
        });
        task.recurring = true;
        task.created_at_ms = now - 2 * DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS;
        task.auto_expire_after_ms = 0;
        task.trigger_at_ms = Some(now);
        store.add(task.clone()).await.unwrap();
        let mut runner = CronRunner::new(store.clone(), gcx);

        assert!(!task_final_after_fire(&task, now));
        assert!(runner.handle_due_task(task.clone(), now).await);

        let stored = store.get(&task.id).await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert_eq!(stored.auto_expire_after_ms, 0);
    }

    #[tokio::test]
    async fn final_fire_event_payload_has_final_true() {
        let now = now_ms();
        let message = cron_fire_message(&expired_task("cron_final", now), true);

        assert_eq!(message.extra["event"]["subkind"], json!("cron_fire"));
        assert_eq!(
            message.extra["event"]["payload"]["task_id"],
            json!("cron_final")
        );
        assert_eq!(message.extra["event"]["payload"]["final"], json!(true));
    }

    #[tokio::test]
    async fn expired_task_removed_from_store() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(expired_task("cron_expire_removed", now))
            .await
            .unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        assert!(store.list().await.is_empty());
        let session = session(&gcx).await;
        let session = session.lock().await;
        let fire_event = event_message(&session, "cron_fire", "cron_expire_removed");
        assert_eq!(fire_event.extra["event"]["payload"]["final"], json!(true));
        let notice_event = event_message(&session, "system_notice", "cron_expire_removed");
        assert_eq!(
            notice_event.extra["event"]["payload"],
            json!({"task_id": "cron_expire_removed", "reason": "auto_expired"})
        );
        assert_eq!(
            notice_event.content.content_text_only(),
            "Recurring task 'scheduled prompt' auto-expired after 2d"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_flag_cancels_runner() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store.add(task("cron_shutdown", now)).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        gcx.shutdown_flag.store(true, Ordering::Relaxed);

        let handle = spawn(store, gcx);
        tokio::time::advance(Duration::from_millis(200)).await;

        assert!(handle.is_finished());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn durable_one_shot_missing_trajectory_is_deferred_not_fired() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut one_shot = one_shot_task("cron_durable_no_traj", now);
        one_shot.set_existing_chat(Some("missing-traj-chat".to_string()));
        one_shot.durable = true;
        store.add(one_shot).await.unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        assert_eq!(
            store.list().await.len(),
            1,
            "durable one-shot must remain in store when no trajectory exists"
        );
        assert!(
            runner
                .deferred_until_ms
                .contains_key("cron_durable_no_traj"),
            "durable one-shot must be deferred when no trajectory exists"
        );
        let sessions = gcx.chat_sessions.read().await;
        assert!(
            !sessions.contains_key("missing-traj-chat"),
            "no empty session should be created for a durable task with no trajectory"
        );
    }

    #[tokio::test]
    async fn durable_recurring_missing_trajectory_does_not_hot_loop() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut recurring = due_task("cron_durable_recurring_no_traj", now);
        recurring.set_existing_chat(Some("missing-traj-recurring".to_string()));
        recurring.durable = true;
        recurring.trigger_at_ms = Some(now);
        store.add(recurring).await.unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.list().await;
        assert_eq!(stored.len(), 1, "recurring task should remain in store");
        assert_eq!(
            stored[0].fire_count, 0,
            "task must not fire when no trajectory exists"
        );
        assert!(
            runner
                .deferred_until_ms
                .contains_key("cron_durable_recurring_no_traj"),
            "task must be deferred when no trajectory exists"
        );
        let sessions = gcx.chat_sessions.read().await;
        assert!(
            !sessions.contains_key("missing-traj-recurring"),
            "no empty session should be created for a durable task with no trajectory"
        );
    }

    #[tokio::test]
    async fn durable_one_shot_catch_up_fires_with_missed_when_session_available() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut one_shot = one_shot_task("cron_catch_up_missed", now);
        one_shot.set_existing_chat(Some("catch-up-chat".to_string()));
        one_shot.durable = true;
        store.add(one_shot).await.unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut session = crate::chat::types::ChatSession::new("catch-up-chat".to_string());
        session.set_runtime_state(SessionState::Idle, None);
        gcx.chat_sessions
            .write()
            .await
            .insert("catch-up-chat".to_string(), Arc::new(AMutex::new(session)));

        let mut runner = CronRunner::new(store.clone(), gcx.clone());
        runner.catch_up().await;

        assert!(
            store.list().await.is_empty(),
            "missed durable one-shot should be removed after catch-up"
        );
        let sessions = gcx.chat_sessions.read().await;
        let session_arc = sessions.get("catch-up-chat").unwrap();
        let session = session_arc.lock().await;
        let fire_event = session.messages.iter().find(|m| {
            m.role == EVENT_ROLE && m.extra["event"]["subkind"].as_str() == Some("cron_fire")
        });
        assert!(
            fire_event.is_some(),
            "catch-up should inject cron_fire event"
        );
        assert_eq!(
            fire_event.unwrap().extra["event"]["payload"]["missed"],
            json!(true),
            "catch-up fire must carry missed=true"
        );
    }

    #[tokio::test]
    async fn durable_one_shot_catch_up_skips_when_no_trajectory() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut one_shot = one_shot_task("cron_catch_up_no_traj", now);
        one_shot.set_existing_chat(Some("no-traj-catch-up-chat".to_string()));
        one_shot.durable = true;
        store.add(one_shot).await.unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());
        runner.catch_up().await;

        assert_eq!(
            store.list().await.len(),
            1,
            "task should remain when no trajectory exists during catch-up"
        );
        let sessions = gcx.chat_sessions.read().await;
        assert!(
            !sessions.contains_key("no-traj-catch-up-chat"),
            "no empty session should be created during catch-up when no trajectory exists"
        );
    }

    #[tokio::test]
    async fn fire_mode_is_applied_as_set_params() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut task = due_task("cron_mode_apply", now);
        task.set_mode(Some("explore".to_string()));
        store.add(task).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let session = session(&gcx).await;
        let session = session.lock().await;
        let set_params_idx = session.command_queue.iter().position(|req| {
            matches!(&req.command, ChatCommand::SetParams { patch }
                if patch.get("mode").and_then(|v| v.as_str()) == Some("explore"))
        });
        let user_message_idx = session
            .command_queue
            .iter()
            .position(|req| matches!(&req.command, ChatCommand::UserMessage { .. }));
        assert!(
            set_params_idx.is_some(),
            "SetParams must be in queue for task with mode"
        );
        assert!(user_message_idx.is_some(), "UserMessage must be in queue");
        assert!(
            set_params_idx.unwrap() < user_message_idx.unwrap(),
            "SetParams must precede UserMessage in queue"
        );
    }

    #[tokio::test]
    async fn fire_without_mode_does_not_inject_set_params() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut task = due_task("cron_no_mode", now);
        task.set_mode(None);
        store.add(task).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let session = session(&gcx).await;
        let session = session.lock().await;
        let has_set_params = session
            .command_queue
            .iter()
            .any(|req| matches!(&req.command, ChatCommand::SetParams { .. }));
        assert!(
            !has_set_params,
            "No SetParams should be in queue when task has no mode"
        );
    }

    #[tokio::test]
    async fn fire_mode_with_non_empty_priority_queue_preserves_existing_order() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut task = due_task("cron_mode_nonempty_queue", now);
        task.set_mode(Some("explore".to_string()));
        store.add(task).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;

        {
            let session_arc = session(&gcx).await;
            let mut session = session_arc.lock().await;
            session.command_queue.push_back(CommandRequest {
                client_request_id: "pre-existing-priority".to_string(),
                priority: true,
                command: ChatCommand::SetParams {
                    patch: json!({"temperature": 0.5}),
                },
            });
        }

        let mut runner = CronRunner::new(store.clone(), gcx.clone());
        runner.fire_due_tasks(now).await;

        let session_arc = session(&gcx).await;
        let session = session_arc.lock().await;
        let queue: Vec<_> = session.command_queue.iter().collect();
        assert_eq!(
            queue.len(),
            3,
            "queue must have pre-existing + SetParams(mode) + UserMessage"
        );
        assert!(
            matches!(&queue[0].command, ChatCommand::SetParams { patch }
                if patch.get("temperature").is_some()),
            "pre-existing priority item must stay first"
        );
        assert!(
            matches!(&queue[1].command, ChatCommand::SetParams { patch }
                if patch.get("mode").and_then(|v| v.as_str()) == Some("explore")),
            "scheduled SetParams must follow existing priority items"
        );
        assert!(
            matches!(&queue[2].command, ChatCommand::UserMessage { .. }),
            "UserMessage must immediately follow SetParams"
        );
    }

    #[tokio::test]
    async fn fire_mode_is_persistent_no_auto_restore() {
        // SetParams applied at fire time permanently changes the chat mode.
        // No restore command is queued after the UserMessage — this is intentional:
        // a scheduled task that needs a specific mode changes the session mode for
        // all subsequent turns until the user or another command changes it back.
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut task = due_task("cron_mode_persist", now);
        task.set_mode(Some("agent".to_string()));
        store.add(task).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let session_arc = session(&gcx).await;
        let session = session_arc.lock().await;
        let set_params_count = session
            .command_queue
            .iter()
            .filter(|req| matches!(&req.command, ChatCommand::SetParams { .. }))
            .count();
        assert_eq!(
            set_params_count, 1,
            "exactly one SetParams (the mode change) must be queued; no auto-restore"
        );
    }

    #[tokio::test]
    async fn cron_defers_when_non_priority_user_message_queued() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut t = due_task("cron_defer_non_priority", now);
        t.set_mode(Some("agent".to_string()));
        store.add(t).await.unwrap();
        let gcx = gcx_with_session(SessionState::Idle).await;

        {
            let session_arc = session(&gcx).await;
            let mut sess = session_arc.lock().await;
            sess.command_queue.push_back(CommandRequest {
                client_request_id: "user-non-priority".to_string(),
                priority: false,
                command: ChatCommand::UserMessage {
                    content: serde_json::Value::String("user message".to_string()),
                    attachments: vec![],
                    context_files: vec![],
                    suppress_auto_enrichment: false,
                },
            });
        }

        let mut runner = CronRunner::new(store.clone(), gcx.clone());
        runner.fire_due_tasks(now).await;

        assert!(
            runner
                .deferred_until_ms
                .contains_key("cron_defer_non_priority"),
            "cron must be deferred when non-priority message is in queue"
        );

        let session_arc = session(&gcx).await;
        let sess = session_arc.lock().await;
        assert_eq!(
            sess.command_queue.len(),
            1,
            "cron must not inject commands ahead of non-priority user message"
        );
        assert!(
            matches!(&sess.command_queue[0].command, ChatCommand::UserMessage { content, .. }
                if content.as_str() == Some("user message")),
            "non-priority user message must remain first and only in queue"
        );
        assert!(
            sess.messages.iter().all(|m| m.role != EVENT_ROLE),
            "no cron event should be added when deferred"
        );
    }
}
