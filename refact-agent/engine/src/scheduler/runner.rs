use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::Instant as TokioInstant;

use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::get_or_create_session_with_trajectory;
use crate::chat::session_runtime::SessionRestore;
use crate::chat::session_runtime::restore_session_if_trajectory_exists;
#[cfg(test)]
use crate::chat::types::max_queue_size;
use refact_core::chat_types::{DeliveryOutcome, PushMode};
use crate::files_correction::get_active_project_path;
use crate::global_context::SharedGlobalContext;
use crate::scheduler::scheduler_timezone;

use super::delivery::{deliver, deliver_notice, deliver_scheduled_turn};
use super::exec_action::{command_error_summary, run_command, CommandRunResult};
use super::jitter::{jittered_next_run_ms_for, one_shot_jittered_next_run_ms_for, JitterConfig};
use super::retry::{classify, retry_delay_ms};
use super::schedule::{next_run_ms, recurring_missed_grace_state, MissedRunGraceConfig};
use super::store::{CronStore, InMemoryCronStore, JsonFileCronStore};
use super::types::{
    Action, AgentTarget, CommandSpec, CronRunRecord, Delivery, Job, SchedulerConfig, Trigger,
};

const DEFAULT_SLEEP_MS: u64 = 1_000;
const IDLE_DEFER_MS: u64 = 30_000;
const INVALID_TARGET_DEFER_MS: u64 = 60_000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

enum DueFireOutcome {
    Fired,
    Deferred,
}

impl DueFireOutcome {
    fn did_fire(self) -> bool {
        matches!(self, Self::Fired)
    }
}

pub struct CronRunner {
    pub store: Arc<dyn CronStore>,
    pub gcx: SharedGlobalContext,
    pub shutdown_flag: Arc<AtomicBool>,
    pub change_notify: Arc<Notify>,
    pub jitter_cfg: JitterConfig,
    request_id: Option<String>,
    deferred_until_ms: HashMap<String, u64>,
}

impl CronRunner {
    pub fn new(store: Arc<dyn CronStore>, gcx: SharedGlobalContext) -> Self {
        let shutdown_flag = gcx.shutdown_flag.clone();
        let change_notify = store.change_notify();
        Self {
            store,
            gcx,
            shutdown_flag,
            change_notify,
            jitter_cfg: JitterConfig::default(),
            request_id: None,
            deferred_until_ms: HashMap::new(),
        }
    }

    pub fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            self.run().await;
        })
    }

    pub(crate) fn validate_definition_legacy(task: &Job) -> Result<(), String> {
        if task.command_spec().is_some() {
            runnable_command_job(task).map_err(str::to_string)
        } else if job_is_isolated(task) {
            runnable_isolated_job(task).map_err(str::to_string)
        } else {
            runnable_chat_id(task).map(|_| ()).map_err(str::to_string)
        }
    }

    /// The generic runtime already holds exact-definition durable admission and capacity.
    pub(crate) async fn execute_definition_legacy(
        gcx: SharedGlobalContext,
        task: &Job,
        occurrence: &str,
    ) -> Result<Value, String> {
        let app = AppState::from_gcx(gcx.clone()).await;
        if let Some(cmd) = task.command_spec() {
            runnable_command_job(task).map_err(str::to_string)?;
            let result = run_command(&app, task, &cmd).await;
            if result.status != "fired" {
                return Err(command_error_summary(&result));
            }
            return Ok(
                json!({"stdout":result.stdout.chars().take(32768).collect::<String>(),"exit_code":result.exit_code}),
            );
        }
        let mut runner = Self::new(session_cron_store(), gcx);
        runner.request_id = Some(occurrence.into());
        let now = now_ms();
        if job_is_isolated(task) {
            let mut task = task.clone();
            task.id = format!("{}_{}", task.id, uuid::Uuid::new_v4());
            return runner
                .fire_isolated_with_missed(&task, false, now, false)
                .await
                .map(|accepted| json!({"accepted":accepted}));
        }
        let chat = runnable_chat_id(task).map_err(str::to_string)?;
        let outcome = deliver_scheduled_turn(
            &app,
            chat,
            task,
            now,
            Some(occurrence),
            vec![
                cron_fire_message(task, false),
                scheduled_prompt_message(task.prompt().unwrap_or_default().into()),
            ],
        )
        .await?;
        Ok(
            json!({"accepted":matches!(outcome,DeliveryOutcome::Delivered|DeliveryOutcome::Queued|DeliveryOutcome::Duplicate)}),
        )
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
        store.add(task.clone()).await?;
        let mut runner = Self::new(store, gcx);
        runner.request_id = Some(uuid::Uuid::new_v4().to_string());
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
        let task = store
            .mutate(
                job_id,
                Box::new(move |task| {
                    task.trigger_at_ms = Some(now);
                    Ok(())
                }),
            )
            .await?
            .ok_or_else(|| format!("Scheduled task {job_id} not found"))?;
        let mut runner = Self::new(store.clone(), gcx);
        runner.request_id = Some(uuid::Uuid::new_v4().to_string());
        let fired = runner.handle_due_task(task, now).await;
        Ok(fired)
    }

    async fn run(mut self) {
        if !self.enabled() {
            return;
        }
        let shutdown = self.shutdown_flag.clone();
        tokio::select! {
            biased;
            _ = wait_for_shutdown(shutdown.clone()) => return,
            _ = self.catch_up() => {}
        }
        let generic = tokio::select! {
            biased;
            _ = wait_for_shutdown(shutdown.clone()) => return,
            generic = async {
                if let Some(root) = get_active_project_path(self.gcx.clone()).await {
                    match super::generic_runtime::runtime(self.gcx.clone(), root).await {
                        Ok(runtime) => Some(runtime),
                        Err(error) => {
                            tracing::warn!("generic automation startup failed: {error}");
                            None
                        }
                    }
                } else {
                    None
                }
            } => generic,
        };

        loop {
            if self.shutdown_flag.load(Ordering::Relaxed) {
                break;
            }

            if let Some(generic) = &generic {
                tokio::select! {
                    biased;
                    _ = wait_for_shutdown(shutdown.clone()) => break,
                    result = generic.dispatch_due() => {
                        if let Err(error) = result {
                            tracing::warn!("automation reconcile failed: {error}");
                        }
                    }
                }
            }
            let notify = self.change_notify.clone();
            let changed = notify.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let now = now_ms();
            let tasks = self.store.list().await;
            let next = tasks
                .iter()
                .filter_map(|task| self.scheduled_fire_at_ms(task, now))
                .min()
                .unwrap_or(now + DEFAULT_SLEEP_MS)
                .min(now + DEFAULT_SLEEP_MS);
            let next = if generic.is_some() {
                next.min(now + 1000)
            } else {
                next
            };
            let sleep_until = TokioInstant::now() + Duration::from_millis(next.saturating_sub(now));

            let global_change = runner_change_notify();
            tokio::select! {
                _ = global_change.notified() => continue,
                _ = tokio::time::sleep_until(sleep_until) => {}
                _ = &mut changed => continue,
                _ = wait_for_shutdown(self.shutdown_flag.clone()) => break,
            }

            self.fire_due_tasks(now_ms()).await;
        }
    }

    async fn catch_up(&mut self) {
        let now = now_ms();
        let mut missed_counts = HashMap::<String, u64>::new();

        for selected in self.store.list().await {
            let Some(task) = self.store.get(&selected.id).await else {
                continue;
            };
            if task.is_paused() && task.trigger_at_ms.is_none() {
                continue;
            }
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
                if !self.restore_durable_chat_target(&task, chat_id, now).await {
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
                    if let Err(error) = self.remove_completed_task(&task).await {
                        tracing::warn!(
                            "failed to remove caught-up scheduled task {}: {}",
                            task.id,
                            error
                        );
                    }
                }
                Ok(false) => {
                    let _ = self
                        .record_run(&task, "deferred", Some("delivery not accepted".into()), now)
                        .await;
                    self.defer_task(&task, now);
                }
                Err(error) => {
                    tracing::warn!("failed to catch up scheduled task {}: {}", task.id, error);
                    let _ = self.record_run(&task, "deferred", Some(error), now).await;
                    self.defer_task(&task, now);
                }
            }
        }

        for (chat_id, missed_count) in missed_counts {
            self.emit_catch_up_notice(&chat_id, missed_count).await;
        }
    }

    async fn resume_recurring_task(&self, task: &Job, now: u64) {
        if let Err(error) = self.fast_forward_recurring_if_missed(task, now).await {
            tracing::warn!("failed to resume scheduled task {}: {}", task.id, error);
        }
    }

    /// `REFACT_DISABLE_SCHEDULER` is a startup gate, resolved into
    /// `SchedulerConfig::enabled` by `with_startup_overrides`. A live runner
    /// reads that snapshot: re-reading the process environment on every tick
    /// drops a due fire with no run record when another thread flips it.
    fn enabled(&self) -> bool {
        self.gcx.scheduler_config.enabled
    }

    fn missed_grace_config(&self) -> MissedRunGraceConfig {
        MissedRunGraceConfig::from(&self.gcx.scheduler_config)
    }

    fn recent_runs_cap(&self) -> usize {
        self.gcx.scheduler_config.recent_runs_cap
    }

    async fn fire_due_tasks(&mut self, now: u64) {
        if !self.enabled() {
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

        let owner = match super::automation_runtime::execution_owner(&self.gcx).await {
            Ok(owner) => owner,
            Err(error) => {
                tracing::warn!("scheduler admission unavailable: {error}");
                return;
            }
        };
        owner.retry_deliveries(&self.gcx).await;
        // Own children for the entire dispatch future: cancelling/recycling
        // the parent must abort its runs rather than detach their effects.
        let mut handles = tokio::task::JoinSet::new();
        let max_concurrent_runs = self.gcx.scheduler_config.max_concurrent_runs.max(1);
        for selected in due_tasks {
            let Some(task) = self.store.get(&selected.id).await else {
                continue;
            };
            if !self.task_is_due(&task, now) && !self.task_needs_missed_fast_forward(&task, now) {
                continue;
            }
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
                self.defer_for_backpressure(&task, now).await;
                continue;
            }
            if task_can_use_cron_lane(&task) && owner.capacity.available_permits() == 0 {
                self.defer_for_backpressure(&task, now).await;
                continue;
            }
            if !task_can_use_cron_lane(&task) {
                self.handle_due_task(task, now).await;
                continue;
            }
            let store = self.store.clone();
            let gcx = self.gcx.clone();
            let shutdown_flag = self.shutdown_flag.clone();
            let change_notify = self.change_notify.clone();
            let jitter_cfg = self.jitter_cfg.clone();
            let task_id = task.id.clone();
            handles.spawn(async move {
                let mut runner = CronRunner {
                    store,
                    gcx,
                    shutdown_flag,
                    change_notify,
                    jitter_cfg,
                    request_id: None,
                    deferred_until_ms: HashMap::new(),
                };
                runner.handle_due_task(task, now).await;
                (task_id, runner.deferred_until_ms)
            });
        }
        while let Some(result) = handles.join_next().await {
            if let Ok((task_id, deferred)) = result {
                if deferred.contains_key(&task_id) {
                    self.deferred_until_ms.extend(deferred);
                } else {
                    self.deferred_until_ms.remove(&task_id);
                }
            }
        }
    }

    async fn handle_due_task(&mut self, task: Job, now: u64) -> bool {
        let Some(task) = self.store.get(&task.id).await else {
            return false;
        };
        if !self.task_is_due(&task, now) {
            return false;
        }
        if matches!(&task.action, Action::Command { .. }) {
            return self.handle_due_command_task(task, now).await;
        }
        if job_is_isolated(&task) {
            return self.handle_due_isolated_task(task, now).await;
        }

        let chat_id = match runnable_chat_id(&task) {
            Ok(chat_id) => chat_id,
            Err(reason) => {
                self.handle_unfireable_task(&task, now, reason).await;
                return false;
            }
        };
        if !self.ensure_chat_target_fireable(&task, chat_id, now).await {
            return false;
        }

        let final_fire = task_final_after_fire(&task, now);
        self.fire_agent_task(&task, final_fire, now)
            .await
            .did_fire()
    }

    async fn ensure_chat_target_fireable(&mut self, task: &Job, chat_id: &str, now: u64) -> bool {
        match chat_fire_status(&self.gcx, chat_id).await {
            ChatFireStatus::Fireable | ChatFireStatus::Busy => true,
            ChatFireStatus::Missing if task.durable => {
                if !self.restore_durable_chat_target(task, chat_id, now).await {
                    return false;
                }
                match chat_fire_status(&self.gcx, chat_id).await {
                    ChatFireStatus::Fireable | ChatFireStatus::Busy => true,
                    status => {
                        tracing::warn!(
                            "durable task {} deferred after session restore ({:?})",
                            task.id,
                            status
                        );
                        if let Err(error) = self
                            .record_run(
                                &task,
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
                        self.defer_invalid_target_task(task, now);
                        false
                    }
                }
            }
            ChatFireStatus::Missing => {
                self.handle_unfireable_task(task, now, "chat session not found")
                    .await;
                false
            }
            ChatFireStatus::Closed => {
                self.handle_unfireable_task(task, now, "chat session is closed")
                    .await;
                false
            }
        }
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
        if !self
            .ensure_command_chat_target_fireable(&task, &cmd, now)
            .await
        {
            return false;
        }

        let final_fire = task_final_after_fire(&task, now);
        let Some(result) = self.run_due_command(&task, &cmd, final_fire, now).await else {
            return false;
        };
        self.complete_command_fire(&task, result, final_fire, now)
            .await
            .did_fire()
    }

    async fn ensure_command_chat_target_fireable(
        &mut self,
        task: &Job,
        cmd: &CommandSpec,
        now: u64,
    ) -> bool {
        if !matches!(task.delivery, Delivery::Chat) {
            return true;
        }
        let AgentTarget::ExistingChat {
            chat_id,
        } = &cmd.target
        else {
            return true;
        };
        if chat_id.is_empty() {
            self.handle_unfireable_task(task, now, "missing chat_id")
                .await;
            return false;
        }
        match chat_fire_status(&self.gcx, chat_id).await {
            ChatFireStatus::Fireable | ChatFireStatus::Busy => true,
            ChatFireStatus::Missing if task.durable => {
                self.restore_durable_chat_target(task, chat_id, now).await
            }
            ChatFireStatus::Missing => {
                self.handle_unfireable_task(task, now, "chat session not found")
                    .await;
                false
            }
            ChatFireStatus::Closed => {
                self.handle_unfireable_task(task, now, "chat session is closed")
                    .await;
                false
            }
        }
    }

    async fn run_due_command(
        &mut self,
        task: &Job,
        cmd: &CommandSpec,
        final_fire: bool,
        now: u64,
    ) -> Option<CommandRunResult> {
        match self
            .fire_command_with_missed(task, cmd, final_fire, now, false)
            .await
        {
            Ok(result) => Some(result),
            Err(error) => {
                tracing::warn!("failed to fire scheduled task {}: {}", task.id, error);
                if !self
                    .handle_classifiable_fire_error(task, final_fire, now, &error)
                    .await
                {
                    self.handle_unfireable_task(task, now, &error).await;
                }
                None
            }
        }
    }

    async fn complete_command_fire(
        &mut self,
        task: &Job,
        result: CommandRunResult,
        final_fire: bool,
        now: u64,
    ) -> DueFireOutcome {
        let error = (result.status == "error").then(|| command_record_error(&result));
        if let Err(store_error) = self.record_run(&task, &result.status, error, now).await {
            tracing::warn!(
                "failed to record scheduled task {} status {}: {}",
                task.id,
                result.status,
                store_error
            );
        }
        if result.status == "error" && command_failure_can_retry(&result) {
            match self
                .schedule_retry(task, now, &command_retry_text(&result))
                .await
            {
                Ok(true) => {
                    self.deferred_until_ms.remove(&task.id);
                    return DueFireOutcome::Deferred;
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!("failed to schedule retry for task {}: {}", task.id, error);
                }
            }
        }
        self.finish_task_after_fire(task, final_fire).await;
        if result.status == "fired" {
            DueFireOutcome::Fired
        } else {
            DueFireOutcome::Deferred
        }
    }

    async fn handle_due_isolated_task(&mut self, task: Job, now: u64) -> bool {
        if let Err(reason) = runnable_isolated_job(&task) {
            self.handle_unfireable_task(&task, now, reason).await;
            return false;
        }

        let final_fire = task_final_after_fire(&task, now);
        self.fire_agent_task(&task, final_fire, now)
            .await
            .did_fire()
    }

    async fn fire_agent_task(&mut self, task: &Job, final_fire: bool, now: u64) -> DueFireOutcome {
        match self.fire(task, final_fire, now).await {
            Ok(true) => {
                if let Err(error) = self.record_run(&task, "fired", None, now).await {
                    tracing::warn!(
                        "failed to record fired scheduled task {}: {}",
                        task.id,
                        error
                    );
                }
                self.finish_task_after_fire(task, final_fire).await;
                DueFireOutcome::Fired
            }
            Ok(false) => {
                if self.record_skipped_after_advance(task, now).await {
                    return DueFireOutcome::Deferred;
                }
                if let Err(error) = self.record_run(&task, "deferred", None, now).await {
                    tracing::warn!(
                        "failed to record deferred scheduled task {}: {}",
                        task.id,
                        error
                    );
                }
                self.defer_task(task, now);
                DueFireOutcome::Deferred
            }
            Err(error) => {
                tracing::warn!("failed to fire scheduled task {}: {}", task.id, error);
                if let Err(store_error) = self.record_run(&task, "deferred", Some(error), now).await
                {
                    tracing::warn!(
                        "failed to record deferred task {}: {}",
                        task.id,
                        store_error
                    );
                }
                self.defer_task(task, now);
                DueFireOutcome::Deferred
            }
        }
    }

    async fn finish_task_after_fire(&mut self, task: &Job, final_fire: bool) {
        if final_fire {
            match self.remove_completed_task(task).await {
                Ok(true) => self.emit_auto_expired_notice(task).await,
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
            if let Err(error) = self.remove_task(task).await {
                tracing::warn!(
                    "failed to remove fired one-shot scheduled task {}: {}",
                    task.id,
                    error
                );
            }
        }
        self.deferred_until_ms.remove(&task.id);
    }

    async fn handle_unfireable_task(&mut self, task: &Job, now: u64, reason: &str) {
        if task.recurring || task.durable {
            tracing::warn!(
                "deferred scheduled task {} because it is not fireable: {}",
                task.id,
                reason
            );
            if let Err(error) = self
                .record_run(&task, "deferred", Some(reason.to_string()), now)
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
            .record_run(&task, "skipped", Some(reason.to_string()), now)
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
            .record_run(&task, "error", Some(error.to_string()), now)
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
            match self.remove_completed_task(task).await {
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
                &task,
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
        let retry_cfg = self.gcx.scheduler_config.retry.clone();
        let (scheduled_tx, scheduled_rx) = tokio::sync::oneshot::channel();
        let stored = self
            .store
            .mutate(
                &task.id,
                Box::new(move |stored| {
                    if stored.is_paused() {
                        let _ = scheduled_tx.send(false);
                        return Ok(());
                    }
                    if let Some(delay_ms) = retry_delay_ms(&retry_cfg, stored.retry_attempts) {
                        stored.retry_attempts = stored.retry_attempts.saturating_add(1);
                        stored.trigger_at_ms = Some(now.saturating_add(delay_ms));
                        let _ = scheduled_tx.send(true);
                    } else {
                        let _ = scheduled_tx.send(false);
                    }
                    Ok(())
                }),
            )
            .await?;
        let Some(stored) = stored else {
            return Ok(false);
        };
        if !scheduled_rx.await.unwrap_or(false) {
            return Ok(false);
        }
        let next_retry_at_ms = stored.trigger_at_ms;
        let retry_attempts = stored.retry_attempts;
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
        let grace = self.missed_grace_config();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.store
            .mutate(
                &task.id,
                Box::new(move |current| {
                    let mut advanced = false;
                    if !current.is_paused()
                        && current.recurring
                        && current.trigger_at_ms.is_none()
                        && current.retry_attempts == 0
                    {
                        if let Some(state) = recurring_missed_grace_state(
                            current,
                            current.last_fired_at_ms.unwrap_or(current.created_at_ms),
                            now,
                            scheduler_timezone(),
                            grace,
                        ) {
                            if state.due_ms.is_some() && !state.should_fire {
                                if let Some(at) = state.advance_last_fired_at_ms {
                                    current.last_fired_at_ms =
                                        Some(current.last_fired_at_ms.unwrap_or(0).max(at));
                                    advanced = true;
                                }
                            }
                        }
                    }
                    let _ = tx.send(advanced);
                    Ok(())
                }),
            )
            .await?;
        Ok(rx.await.unwrap_or(false))
    }

    async fn record_run(
        &self,
        selected: &Job,
        status: &str,
        error: Option<String>,
        now_ms: u64,
    ) -> Result<(), String> {
        record_run(
            &self.store,
            &selected.id,
            status,
            error,
            now_ms,
            self.recent_runs_cap(),
            Some(selected.clone()),
        )
        .await
    }

    async fn remove_completed_task(&self, task: &Job) -> Result<bool, String> {
        let selected = task.clone();
        self.store
            .remove_if(
                &task.id,
                Box::new(move |current| completion_controls_match(&selected, current)),
            )
            .await
    }

    async fn remove_task(&mut self, task: &Job) -> Result<(), String> {
        let _ = self.remove_completed_task(task).await?;
        self.deferred_until_ms.remove(&task.id);
        Ok(())
    }

    fn defer_task(&mut self, task: &Job, now: u64) {
        self.deferred_until_ms
            .insert(task.id.clone(), now + IDLE_DEFER_MS);
    }

    /// Backpressure is a deferral, not a failure: it reuses the delivery
    /// rejection's `"deferred"` record so a throttled task is distinguishable
    /// from an idle one, and it consumes no retry attempt.
    async fn defer_for_backpressure(&mut self, task: &Job, now: u64) {
        if let Err(error) = self
            .record_run(
                &task,
                "deferred",
                Some("scheduler at capacity".to_string()),
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
        self.defer_task(task, now);
    }

    /// Restores a durable task's chat from its trajectory. A missing trajectory is an
    /// invalid target; a refused open (jobs still draining, another open of the chat in
    /// progress, shutdown) is temporary and goes through the ordinary deferral, never a
    /// tight retry.
    async fn restore_durable_chat_target(&mut self, task: &Job, chat_id: &str, now: u64) -> bool {
        let app = AppState::from_gcx(self.gcx.clone()).await;
        let (reason, temporary) =
            match restore_session_if_trajectory_exists(app, &self.gcx.chat_sessions, chat_id).await
            {
                SessionRestore::Open => return true,
                SessionRestore::NoTrajectory => ("no trajectory found".to_string(), false),
                SessionRestore::Refused(error) => {
                    (format!("chat temporarily unavailable: {error}"), true)
                }
            };
        tracing::warn!(
            "durable task {} deferred for chat {}: {}",
            task.id,
            chat_id,
            reason
        );
        if let Err(error) = self.record_run(task, "deferred", Some(reason), now).await {
            tracing::warn!(
                "failed to record deferred scheduled task {}: {}",
                task.id,
                error
            );
        }
        if temporary {
            self.defer_task(task, now);
        } else {
            self.defer_invalid_target_task(task, now);
        }
        false
    }

    fn defer_invalid_target_task(&mut self, task: &Job, now: u64) {
        self.deferred_until_ms
            .insert(task.id.clone(), now + INVALID_TARGET_DEFER_MS);
    }

    fn scheduled_fire_at_ms(&self, task: &Job, now: u64) -> Option<u64> {
        if task.trigger_at_ms.is_none() && task.is_paused() {
            return None;
        }
        if let Some(trigger_at_ms) = task.trigger_at_ms.filter(|at| *at <= now) {
            // A fresh explicit run-now overrides an older backoff; retrying the
            // same rejected admission must still honor its defer timer. Recency
            // decides that, not `last_status`: a retry carries "error", and
            // reading that as a fresh run-now skipped the window and hot-looped.
            let new_trigger = task
                .recent_runs
                .last()
                .is_none_or(|run| trigger_at_ms > run.at_ms);
            if new_trigger || !self.deferred_until_ms.contains_key(&task.id) {
                return Some(trigger_at_ms);
            }
        }
        if let Some(deferred_at) = self.deferred_until_ms.get(&task.id).copied() {
            return Some(
                deferred_at.max(
                    scheduled_fire_at_ms(task, now, &self.jitter_cfg, self.missed_grace_config())
                        .unwrap_or(deferred_at),
                ),
            );
        }
        if task.retry_attempts > 0 && task.trigger_at_ms.is_some() {
            return task.trigger_at_ms;
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
        if task.is_paused()
            || !task.recurring
            || task.trigger_at_ms.is_some()
            || task.retry_attempts > 0
        {
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
            let result = self
                .fire_command_with_missed(task, &cmd, final_fire, fired_at_ms, missed)
                .await?;
            return Ok(result.status == "fired");
        }
        let owner = super::automation_runtime::execution_owner(&self.gcx).await?;
        let occurrence = self
            .request_id
            .clone()
            .unwrap_or_else(|| occurrence_key(task));
        if owner.agent_enqueue_accepted(task, &occurrence)? {
            return Ok(true);
        }
        let admission = owner.admit(task, &occurrence)?;
        if job_is_isolated(&task) {
            let chat_id = match self.open_isolated_chat(task).await {
                Ok(chat_id) => chat_id,
                Err(error) => {
                    admission.rejected_agent_enqueue()?;
                    return Err(error);
                }
            };
            admission.started()?;
            let result = self
                .deliver_isolated(task, &chat_id, final_fire, fired_at_ms, missed)
                .await;
            match result.as_ref() {
                Ok(accepted) => admission.finish(*accepted, None)?,
                Err(error) if error == "chat delivery queue is full" => {
                    admission.rejected_agent_enqueue()?;
                }
                Err(_) => {}
            }
            return result;
        }
        admission.started()?;

        let chat_id = runnable_chat_id(task).map_err(|reason| {
            format!(
                "Scheduled task {} is not a runnable chat job: {reason}",
                task.id
            )
        })?;
        let _session_arc = {
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
        let outcome = deliver_scheduled_turn(
            &app,
            chat_id,
            task,
            fired_at_ms,
            self.request_id.as_deref(),
            vec![event_message, scheduled_prompt_message(prompt)],
        )
        .await;
        // This exact queue refusal is a known non-effect. All other errors
        // remain ambiguous and Admission::drop holds them for review.
        let outcome = match outcome {
            Err(error) if error == "chat delivery queue is full" => {
                admission.rejected_agent_enqueue()?;
                return Err(error);
            }
            result => result?,
        };
        let accepted = matches!(
            outcome,
            DeliveryOutcome::Delivered | DeliveryOutcome::Queued | DeliveryOutcome::Duplicate
        );
        admission.finish(accepted, None)?;
        Ok(accepted)
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
        let owner = super::automation_runtime::execution_owner(&self.gcx).await?;
        let admission = owner.admit(
            task,
            &self
                .request_id
                .clone()
                .unwrap_or_else(|| occurrence_key(task)),
        )?;
        admission.started()?;
        let result = run_command(&app, task, cmd).await;
        let mut delivery_snapshot = task.clone();
        if let Delivery::Webhook {
            token, ..
        } = &mut delivery_snapshot.delivery
        {
            *token = None;
        }
        let delivery_payload = json!({
            "job": delivery_snapshot,
            "job_id": task.id,
            "run_id": admission.claim.as_ref().map(|claim| claim.run_id.as_str()),
            "config_digest": refact_scheduler_core::automation::legacy_job_digest(task)?,
            "status": result.status,
            "output": if result.status == "error" { command_error_summary(&result) } else { result.stdout.chars().take(32_768).collect::<String>() },
            "exit_code": result.exit_code,
            "fired_at_ms": fired_at_ms,
        });
        let run_status = if result.stderr.to_ascii_lowercase().contains("timed out") {
            refact_scheduler_core::automation::RunStatus::Unknown
        } else if result.status == "fired" {
            refact_scheduler_core::automation::RunStatus::Succeeded
        } else {
            refact_scheduler_core::automation::RunStatus::Failed
        };
        let _delivery_guard = owner.delivery_lock.lock().await;
        let delivery_seq = admission.complete_with_delivery(run_status, &delivery_payload)?;
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
        } else {
            admission.delivered(delivery_seq)?;
            if let Err(store_error) = self.clear_delivery_error(&task.id).await {
                tracing::warn!(
                    "failed to clear delivery error for scheduled task {}: {}",
                    task.id,
                    store_error
                );
            }
        }
        Ok(result)
    }

    async fn clear_delivery_error(&self, job_id: &str) -> Result<(), String> {
        self.store
            .mutate(
                job_id,
                Box::new(|task| {
                    task.last_delivery_error = None;
                    Ok(())
                }),
            )
            .await?;
        Ok(())
    }

    async fn record_delivery_error(&self, job_id: &str, error: String) -> Result<(), String> {
        self.store
            .mutate(
                job_id,
                Box::new(move |task| {
                    task.last_delivery_error = Some(error);
                    Ok(())
                }),
            )
            .await?;
        Ok(())
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
        let chat_id = self.open_isolated_chat(task).await?;
        self.deliver_isolated(task, &chat_id, final_fire, fired_at_ms, missed)
            .await
    }

    /// Opens the fresh isolated chat. Nothing has run when this fails, so callers
    /// record the refusal as a rejected enqueue, never as an interrupted effect.
    async fn open_isolated_chat(&self, task: &Job) -> Result<String, String> {
        runnable_isolated_job(task).map_err(|reason| {
            format!(
                "Scheduled task {} is not a runnable isolated chat job: {reason}",
                task.id
            )
        })?;
        let chat_id = format!("cron_{}_{}", task.id, task.fire_count);
        let app = AppState::from_gcx(self.gcx.clone()).await;
        get_or_create_session_with_trajectory(app.clone(), &app.chat.sessions, &chat_id)
            .await
            .map_err(|error| {
                format!("isolated chat {chat_id} unavailable, nothing was started: {error}")
            })?;
        Ok(chat_id)
    }

    async fn deliver_isolated(
        &self,
        task: &Job,
        chat_id: &str,
        final_fire: bool,
        fired_at_ms: u64,
        missed: bool,
    ) -> Result<bool, String> {
        let app = AppState::from_gcx(self.gcx.clone()).await;
        let event_message = if missed {
            cron_fire_message_with_missed(task, final_fire, missed)
        } else {
            cron_fire_message(task, final_fire)
        };
        let prompt = task.prompt().unwrap_or_default().to_string();
        deliver_scheduled_turn(
            &app,
            chat_id,
            task,
            fired_at_ms,
            self.request_id.as_deref(),
            vec![event_message, scheduled_prompt_message(prompt)],
        )
        .await?;
        Ok(true)
    }

    /// Pure notices: routed through the delivery layer (default append) and
    /// never waking a chat.
    async fn emit_catch_up_notice(&self, chat_id: &str, missed_count: u64) {
        if !self.gcx.chat_sessions.read().await.contains_key(chat_id) {
            return;
        }
        let app = AppState::from_gcx(self.gcx.clone()).await;
        if let Err(error) = deliver_notice(
            &app,
            chat_id,
            format!("cron-catch-up-{chat_id}-{}", now_ms()),
            catch_up_notice_message(missed_count),
            PushMode::Append,
        )
        .await
        {
            tracing::warn!("failed to deliver scheduler catch-up notice: {error}");
        }
    }

    async fn emit_auto_expired_notice(&self, task: &Job) {
        let Some(chat_id) = task.chat_id() else {
            return;
        };
        if !self.gcx.chat_sessions.read().await.contains_key(chat_id) {
            return;
        }
        let app = AppState::from_gcx(self.gcx.clone()).await;
        if let Err(error) = deliver_notice(
            &app,
            chat_id,
            format!("cron-auto-expired-{}", task.id),
            auto_expired_notice_message(task),
            PushMode::Append,
        )
        .await
        {
            tracing::warn!(
                "failed to deliver auto-expiry notice for scheduled task {}: {error}",
                task.id
            );
        }
    }
}

fn occurrence_key(task: &Job) -> String {
    match task.trigger_at_ms {
        Some(at) => format!("request:{at}:{}", task.fire_count),
        None => format!(
            "slot:{}:{}",
            task.last_fired_at_ms.unwrap_or(task.created_at_ms),
            task.fire_count
        ),
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
        match tokio::task::spawn_blocking(move || JsonFileCronStore::new(project_root)).await {
            Ok(Ok(store)) => handles.push(spawn(Arc::new(store), gcx)),
            Ok(Err(error)) => tracing::warn!("durable scheduler runner disabled: {error}"),
            Err(error) => tracing::warn!("durable scheduler storage task failed: {error}"),
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
                target: AgentTarget::ExistingChat {
                    chat_id,
                },
                ..
            },
            Delivery::Chat,
        ) if !chat_id.is_empty() => Ok(chat_id),
        (
            Action::AgentTurn {
                ..
            },
            Delivery::Chat,
        ) => Err("missing chat_id"),
        (
            Action::AgentTurn {
                ..
            },
            _,
        ) => Err("non-chat delivery is not supported yet"),
        (
            Action::Command {
                ..
            },
            _,
        ) => Err("command actions use the command runner"),
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
        (
            Action::AgentTurn {
                ..
            },
            Delivery::Chat,
        ) => Err("target is not isolated"),
        (
            Action::AgentTurn {
                ..
            },
            _,
        ) => Err("non-chat delivery is not supported yet"),
        (
            Action::Command {
                ..
            },
            _,
        ) => Err("command actions use the command runner"),
    }
}

fn runnable_command_job(job: &Job) -> Result<(), &'static str> {
    if !runner_supported_trigger(&job.trigger) {
        return Err("trigger is not supported by the command runner yet");
    }
    match (&job.action, &job.delivery) {
        (
            Action::Command {
                argv, ..
            },
            _,
        ) if argv.is_empty() => Err("command argv is empty"),
        (
            Action::Command {
                target: AgentTarget::ExistingChat {
                    chat_id,
                },
                ..
            },
            Delivery::Chat,
        ) if chat_id.is_empty() => Err("missing chat_id"),
        (
            Action::Command {
                ..
            },
            Delivery::Chat,
        ) => Ok(()),
        (
            Action::Command {
                ..
            },
            _,
        ) => Ok(()),
        (
            Action::AgentTurn {
                ..
            },
            _,
        ) => Err("action is not command"),
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

pub(crate) fn scheduled_thread_patch(task: &Job) -> Option<Value> {
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
        Action::AgentTurn {
            model, ..
        } => model.as_deref().filter(|model| !model.is_empty()),
        _ => None,
    }
}

#[cfg(test)]
fn set_job_isolated(task: &mut Job) {
    if let Action::AgentTurn {
        target, ..
    } = &mut task.action
    {
        *target = AgentTarget::Isolated;
    }
}

#[cfg(test)]
fn set_job_model(task: &mut Job, value: Option<String>) {
    if let Action::AgentTurn {
        model, ..
    } = &mut task.action
    {
        *model = value;
    }
}

fn scheduled_prompt_message(prompt: String) -> ChatMessage {
    ChatMessage {
        role: "user".to_string(),
        content: crate::call_validation::ChatContent::SimpleText(prompt),
        ..Default::default()
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

fn command_failure_can_retry(_result: &CommandRunResult) -> bool {
    false
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
    if task.trigger_at_ms.is_none()
        && recurring_state
            .as_ref()
            .is_some_and(|state| state.due_ms.is_some() && !state.should_fire)
    {
        return recurring_state.map(|state| state.next_future_ms);
    }
    let scheduled = match &task.trigger {
        Trigger::Cron {
            ..
        } if task.recurring => jittered_next_run_ms_for(task, from_ms, &task.id, jitter_cfg, tz),
        Trigger::Cron {
            ..
        } => one_shot_jittered_next_run_ms_for(task, from_ms, &task.id, jitter_cfg, tz),
        Trigger::Interval {
            ..
        }
        | Trigger::Once {
            ..
        } => next_run_ms(task, from_ms, tz),
        Trigger::Manual
        | Trigger::Webhook {
            ..
        }
        | Trigger::OnProcessExit {
            ..
        } => None,
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

fn completion_controls_match(selected: &Job, current: &Job) -> bool {
    selected.id == current.id
        && selected.description == current.description
        && selected.enabled == current.enabled
        && selected.paused_at_ms == current.paused_at_ms
        && selected.created_at_ms == current.created_at_ms
        && selected.recurring == current.recurring
        && selected.trigger == current.trigger
        && selected.action == current.action
        && selected.delivery == current.delivery
        && selected.push == current.push
        && selected.auto_expire_after_ms == current.auto_expire_after_ms
        && (current.trigger_at_ms.is_none() || current.trigger_at_ms == selected.trigger_at_ms)
}

async fn record_run(
    store: &Arc<dyn CronStore>,
    job_id: &str,
    status: &str,
    error: Option<String>,
    now_ms: u64,
    recent_runs_cap: usize,
    selected: Option<Job>,
) -> Result<(), String> {
    let status = status.to_string();
    store
        .mutate(
            job_id,
            Box::new(move |task| {
                let same_occurrence = selected.as_ref().is_none_or(|selected| {
                    selected.id == task.id
                        && selected.created_at_ms == task.created_at_ms
                        && selected.recurring == task.recurring
                        && selected.trigger == task.trigger
                        && (task.trigger_at_ms.is_none()
                            || selected.trigger_at_ms == task.trigger_at_ms)
                });
                let already_advanced =
                    same_occurrence && job_was_advanced_before_fire(task, now_ms);
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
                        cap_recent_runs(task, recent_runs_cap);
                    }
                } else {
                    task.recent_runs.push(CronRunRecord {
                        at_ms: now_ms,
                        status: status.to_string(),
                        error: error.clone(),
                    });
                    cap_recent_runs(task, recent_runs_cap);
                }
                task.last_status = Some(status.to_string());
                task.last_error = error;
                // A rejected agent prompt remains the same outstanding fire even after a
                // restart or a long backoff. Do not let missed-run grace fast-forward it.
                if status == "deferred"
                    && same_occurrence
                    && !task.is_paused()
                    && matches!(task.action, Action::AgentTurn { .. })
                {
                    task.trigger_at_ms.get_or_insert(now_ms);
                }
                if status == "fired" || status == "error" {
                    if same_occurrence && !already_advanced {
                        task.last_fired_at_ms = Some(now_ms);
                        task.fire_count = task.fire_count.saturating_add(1);
                    }
                    if same_occurrence && status == "fired" {
                        task.retry_attempts = 0;
                    }
                    if same_occurrence
                        && task
                            .trigger_at_ms
                            .is_some_and(|trigger_at_ms| trigger_at_ms <= now_ms)
                    {
                        task.trigger_at_ms = None;
                    }
                }
                Ok(())
            }),
        )
        .await?;
    Ok(())
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

/// Recurring schedules have no lifetime cutoff: neither the legacy implicit
/// 30-day default nor an explicitly configured `auto_expire_after_ms` ever makes
/// a fire final. One-shot removal is handled separately by the one-shot paths.
fn task_final_after_fire(_task: &Job, _now: u64) -> bool {
    false
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
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;

    use refact_work_core::DeliveryState;
    use chrono::TimeZone;
    use tokio::sync::Mutex as AMutex;

    use super::*;
    use crate::chat::internal_roles::EVENT_ROLE;
    use crate::chat::types::{ChatSession, SessionState};
    use crate::coordination::outbox;
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
        task.trigger = Trigger::Interval {
            every_ms,
        };
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
        vec!["cmd".to_string(), "/C".to_string(), format!("echo {text}")]
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
            "cmd".to_string(),
            "/C".to_string(),
            "ping -n 2 127.0.0.1 >nul".to_string(),
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

    #[tokio::test]
    async fn command_timeout_failure_is_not_replayed() {
        let result = CommandRunResult {
            status: "error".to_string(),
            stdout: String::new(),
            stderr: "command timed out after 1 seconds".to_string(),
            exit_code: None,
        };

        assert!(!command_failure_can_retry(&result));
        #[cfg(unix)]
        {
            let now = now_ms();
            let store = Arc::new(InMemoryCronStore::new());
            let (workspace, gcx) = gcx_with_session(SessionState::Idle).await;
            let app = AppState::from_gcx(gcx.clone()).await;
            let attempts = workspace.path().join("timeout-attempts");
            let argv = vec![
                "sh".to_string(),
                "-c".to_string(),
                r#"printf 'attempt\n' >> "$1"; printf '429 Too Many Requests\n'; exec sleep 60"#
                    .to_string(),
                "timeout-fixture".to_string(),
                attempts.to_string_lossy().into_owned(),
            ];
            let mut job = one_shot_command_due_task("actual_pending_timeout", now, argv);
            if let Action::Command {
                timeout_secs, ..
            } = &mut job.action
            {
                *timeout_secs = Some(2);
            }
            store.add(job).await.unwrap();
            let mut runner = CronRunner::new(store.clone(), gcx);
            let mut running = tokio::spawn(async move {
                runner.fire_due_tasks(now).await;
                runner
            });
            let mut joined = false;
            let outcome: Result<CronRunner, String> = async {
                tokio::time::timeout(Duration::from_secs(20), async {
                    loop {
                        if running.is_finished() {
                            return Err(
                                "runner completed before pending child was observed".to_string()
                            );
                        }
                        if tokio::fs::read_to_string(&attempts)
                            .await
                            .unwrap_or_default()
                            == "attempt\n"
                        {
                            return if running.is_finished() {
                                Err("child handshake did not observe pending runner".to_string())
                            } else {
                                Ok(())
                            };
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .map_err(|error| format!("child handshake: {error}"))??;
                let result = tokio::time::timeout(Duration::from_secs(20), &mut running)
                    .await
                    .map_err(|error| format!("configured timeout: {error}"))?;
                joined = true;
                result.map_err(|error| format!("runner task: {error}"))
            }
            .await;
            if !joined {
                running.abort();
                let _ = (&mut running).await;
            }
            let processes = app.runtime.exec_registry.list(Default::default()).await;
            let owned: Vec<_> = processes
                .into_iter()
                .filter(|process| {
                    process.meta.owner.tool_call_id.as_deref()
                        == Some("cron:actual_pending_timeout")
                })
                .collect();
            let timed_out =
                owned.len() == 1 && matches!(owned[0].status, crate::exec::ExecStatus::TimedOut);
            let mut cleanup_errors = Vec::new();
            for process in owned {
                if let Err(error) = app
                    .runtime
                    .exec_registry
                    .remove(&process.meta.process_id)
                    .await
                {
                    cleanup_errors.push(error);
                }
            }
            let absent_after_timeout = store.get("actual_pending_timeout").await.is_none();
            let mut first_error = None;
            let mut second_error = None;
            if let Ok(mut runner) = outcome {
                let mut second = tokio::spawn(async move {
                    runner.fire_due_tasks(now + 600_000).await;
                });
                match tokio::time::timeout(Duration::from_secs(20), &mut second).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => second_error = Some(format!("second runner task: {error}")),
                    Err(error) => {
                        second.abort();
                        let _ = (&mut second).await;
                        second_error = Some(format!("second runner timeout: {error}"));
                    }
                }
            } else if let Err(error) = outcome {
                first_error = Some(error);
            }
            for process in app.runtime.exec_registry.list(Default::default()).await {
                if process.meta.owner.tool_call_id.as_deref() == Some("cron:actual_pending_timeout")
                {
                    if let Err(error) = app
                        .runtime
                        .exec_registry
                        .remove(&process.meta.process_id)
                        .await
                    {
                        cleanup_errors.push(error);
                    }
                }
            }
            let attempts_result = tokio::fs::read_to_string(&attempts).await;
            assert!(
                cleanup_errors.is_empty(),
                "fixture cleanup failed: {cleanup_errors:?}"
            );
            assert!(
                first_error.is_none(),
                "actual child handshake and configured timeout must finish: {first_error:?}"
            );
            assert!(
                second_error.is_none(),
                "second runner drive must finish: {second_error:?}"
            );
            assert!(
                timed_out,
                "configured deadline must produce an actual timeout, not early exit"
            );
            assert!(
                absent_after_timeout,
                "timed-out started command must not be scheduled for replay"
            );
            assert_eq!(
                attempts_result.unwrap(),
                "attempt\n",
                "second runner drive must not replay timed-out command"
            );
        }
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

    async fn gcx_with_session(state: SessionState) -> (tempfile::TempDir, SharedGlobalContext) {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let mut session = ChatSession::new("chat-1".to_string());
        session.set_runtime_state(state, None);
        gcx.chat_sessions
            .write()
            .await
            .insert("chat-1".to_string(), Arc::new(AMutex::new(session)));
        (workspace, gcx)
    }

    async fn gcx_with_session_and_config<F>(
        state: SessionState,
        configure: F,
    ) -> (tempfile::TempDir, SharedGlobalContext)
    where
        F: FnOnce(&mut crate::scheduler::types::SchedulerConfig),
    {
        let workspace = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        Arc::get_mut(&mut gcx).unwrap().scheduler_config =
            crate::scheduler::types::test_scheduler_config_with(configure);
        let mut session = ChatSession::new("chat-1".to_string());
        session.set_runtime_state(state, None);
        gcx.chat_sessions
            .write()
            .await
            .insert("chat-1".to_string(), Arc::new(AMutex::new(session)));
        (workspace, gcx)
    }

    /// A gcx whose cache directory is the test's own, so the coordination store
    /// this engine would use can be made unusable without touching any other
    /// test.
    async fn gcx_with_session_in_own_cache(
        state: SessionState,
    ) -> (tempfile::TempDir, SharedGlobalContext) {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            workspace.path().join("cache"),
            workspace.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let mut session = ChatSession::new("chat-1".to_string());
        session.set_runtime_state(state, None);
        gcx.chat_sessions
            .write()
            .await
            .insert("chat-1".to_string(), Arc::new(AMutex::new(session)));
        (workspace, gcx)
    }

    /// Park a plain file where the store's directory belongs, so opening it
    /// fails the way a real deployment without a usable store does.
    fn block_coordination_store(gcx: &SharedGlobalContext) -> PathBuf {
        let blocked = gcx.cache_dir.join("coordination");
        std::fs::write(&blocked, "not a directory").unwrap();
        blocked
    }

    async fn gcx_with_closed_session() -> (tempfile::TempDir, SharedGlobalContext) {
        let (workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        let session = session(&gcx).await;
        session.lock().await.close_event_channel();
        (workspace, gcx)
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
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            {
                let session = session_arc.lock().await;
                let fire_event = session.messages.iter().any(|message| {
                    message.role == EVENT_ROLE
                        && message.extra["event"]["subkind"].as_str() == Some("cron_fire")
                        && message.extra["event"]["payload"]["task_id"].as_str() == Some(task_id)
                });
                let prompt_queued = session.pending_deliveries.iter().any(|delivery| {
                    delivery.messages.iter().any(|message| {
                        message.role == "user"
                            && message.content.content_text_only() == "scheduled prompt"
                    })
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
                std::time::Instant::now() < deadline,
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

    /// The fire this waits for crosses a real thread (the coordination store
    /// runs its own), so the budget is wall clock. A paused virtual clock
    /// auto-advances through two seconds in well under a millisecond and would
    /// time out before any of that work could land.

    fn assert_no_fire(gcx: &SharedGlobalContext) -> impl std::future::Future<Output = ()> + '_ {
        async move {
            let session = session(gcx).await;
            let session = session.lock().await;
            assert!(session.messages.is_empty());
            assert!(session.command_queue.is_empty());
        }
    }

    #[tokio::test]
    async fn one_shot_removed_after_normal_fire() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(one_shot_task("cron_one_shot_removed", now))
            .await
            .unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
        let (_workspace, gcx) = gcx_with_session_and_config(SessionState::Idle, |config| {
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
        let mut job = one_shot_task("cron_one_shot_queue_full", now);
        job.set_mode(Some("explore".into()));
        set_job_model(&mut job, Some("scheduled-model".into()));
        store.add(job).await.unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        {
            let session_arc = session(&gcx).await;
            let mut session = session_arc.lock().await;
            for idx in 0..max_queue_size() {
                session.pending_deliveries.push_back(
                    refact_core::chat_types::PendingDelivery::with_id(
                        format!("queued-{idx}"),
                        vec![scheduled_prompt_message("queued".into())],
                        PushMode::Append,
                        "test",
                        false,
                    ),
                );
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
        assert!(stored
            .last_error
            .as_deref()
            .unwrap()
            .contains("queue is full"));
        assert!(!runner.task_is_due(&stored, now + 1));
        let arc = session(&gcx).await;
        {
            let mut session = arc.lock().await;
            assert!(session.messages.iter().all(|m| m.role != EVENT_ROLE));
            assert_ne!(session.thread.mode, "explore");
            assert_ne!(session.thread.model, "scheduled-model");
            session.pending_deliveries.clear();
        }
        runner.fire_due_tasks(now + IDLE_DEFER_MS).await;
        assert!(store.get("cron_one_shot_queue_full").await.is_none());
        assert_eq!(arc.lock().await.thread.mode, "explore");
        assert_eq!(arc.lock().await.thread.model, "scheduled-model");
        assert_eq!(
            arc.lock()
                .await
                .messages
                .iter()
                .filter(|m| m.role == "user")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn fires_without_durability_when_the_store_cannot_be_opened() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store.add(due_task("cron_no_outbox", now)).await.unwrap();
        let (_workspace, gcx) = gcx_with_session_in_own_cache(SessionState::Idle).await;
        let blocked = block_coordination_store(&gcx);
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("cron_no_outbox").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert_eq!(stored.fire_count, 1);
        assert_eq!(stored.last_fired_at_ms, Some(now));
        {
            let session = session(&gcx).await;
            let session = session.lock().await;
            let _ = event_message(&session, "cron_fire", "cron_no_outbox");
            assert!(session.messages.iter().any(|message| message.role == "user"
                && message.content.content_text_only() == "scheduled prompt"));
        }

        let reported = outbox::durability_warnings();
        assert!(
            reported
                .iter()
                .any(|warning| warning.to_string().contains(&blocked.display().to_string())),
            "a store that cannot be opened must be reported, not passed off as durable: {reported:?}"
        );
    }

    #[tokio::test]
    async fn fired_delivery_is_persisted_when_the_store_is_available() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(due_task("cron_outbox_persist", now))
            .await
            .unwrap();
        let (_workspace, gcx) = gcx_with_session_in_own_cache(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("cron_outbox_persist").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        let app = AppState::from_gcx(gcx).await;
        let project = outbox::project_key_for_root(&app, None).await;
        let outbox = outbox::outbox_for(&app, project)
            .await
            .expect("a usable store must yield an outbox");
        assert_eq!(
            outbox
                .status_of("cron-cron_outbox_persist-0-turn")
                .await
                .unwrap(),
            Some(DeliveryState::Delivered),
            "a fired scheduled turn stays durable when the store is available"
        );
    }

    #[tokio::test]
    async fn rejected_enqueue_without_durability_keeps_rejection_semantics() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(one_shot_task("cron_degraded_one_shot", now))
            .await
            .unwrap();
        store
            .add(due_task("cron_degraded_recurring", now))
            .await
            .unwrap();
        let (_workspace, gcx) = gcx_with_session_in_own_cache(SessionState::Idle).await;
        block_coordination_store(&gcx);
        {
            let session_arc = session(&gcx).await;
            let mut session = session_arc.lock().await;
            for idx in 0..max_queue_size() {
                session.pending_deliveries.push_back(
                    refact_core::chat_types::PendingDelivery::with_id(
                        format!("queued-{idx}"),
                        vec![scheduled_prompt_message("queued".into())],
                        PushMode::Append,
                        "test",
                        false,
                    ),
                );
            }
        }
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let one_shot = store.get("cron_degraded_one_shot").await.unwrap();
        assert_eq!(one_shot.last_status.as_deref(), Some("deferred"));
        assert_eq!(one_shot.fire_count, 0);
        assert_eq!(one_shot.last_fired_at_ms, None);
        let recurring = store.get("cron_degraded_recurring").await.unwrap();
        assert_eq!(recurring.last_status.as_deref(), Some("deferred"));
        assert_eq!(recurring.fire_count, 0);
        assert_eq!(recurring.last_fired_at_ms, Some(now - 120_000));
        let session = session(&gcx).await;
        let session = session.lock().await;
        assert!(session.messages.iter().all(|m| m.role != EVENT_ROLE));
    }

    #[tokio::test]
    async fn rejected_recurring_occurrence_is_retried_not_skipped() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(due_task("cron_recurring_retry_after_rejection", now))
            .await
            .unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        {
            let session_arc = session(&gcx).await;
            let mut session = session_arc.lock().await;
            for idx in 0..max_queue_size() {
                session.pending_deliveries.push_back(
                    refact_core::chat_types::PendingDelivery::with_id(
                        format!("queued-{idx}"),
                        vec![scheduled_prompt_message("queued".into())],
                        PushMode::Append,
                        "test",
                        false,
                    ),
                );
            }
        }
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store
            .get("cron_recurring_retry_after_rejection")
            .await
            .unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("deferred"));
        assert_eq!(stored.fire_count, 0);
        assert!(!runner.task_is_due(&stored, now + IDLE_DEFER_MS - 1));
        assert!(runner.task_is_due(&stored, now + IDLE_DEFER_MS));

        let arc = session(&gcx).await;
        arc.lock().await.pending_deliveries.clear();

        runner.fire_due_tasks(now + IDLE_DEFER_MS).await;

        let stored = store
            .get("cron_recurring_retry_after_rejection")
            .await
            .unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert_eq!(stored.fire_count, 1);
        assert_eq!(stored.last_fired_at_ms, Some(now + IDLE_DEFER_MS));
        assert!(stored.recent_runs.iter().all(|run| run.status != "skipped"));
        assert_eq!(
            arc.lock()
                .await
                .messages
                .iter()
                .filter(|m| m.role == "user")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn interval_job_fires_on_cadence() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(interval_task("interval_due", now, 60_000))
            .await
            .unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
    async fn independent_manual_and_store_entrypoints_share_project_capacity() {
        let now = now_ms();
        let (_workspace, gcx) =
            gcx_with_session_and_config(SessionState::Idle, |c| c.max_concurrent_runs = 1).await;
        let owner = super::super::automation_runtime::execution_owner(&gcx)
            .await
            .unwrap();
        let held = owner.capacity.clone().try_acquire_owned().unwrap();
        let first = command_due_task("manual_capacity_a", now, empty_command());
        let second = command_due_task("manual_capacity_b", now, empty_command());
        assert!(CronRunner::fire_manual_job(gcx.clone(), first, now)
            .await
            .is_err());
        assert!(
            CronRunner::fire_manual_job(gcx.clone(), second.clone(), now)
                .await
                .is_err()
        );
        let store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        store.add(second.clone()).await.unwrap();
        assert!(
            !CronRunner::fire_store_job_now(store.clone(), gcx.clone(), &second.id, now)
                .await
                .unwrap()
        );
        assert_eq!(store.get(&second.id).await.unwrap().fire_count, 0);
        drop(held);
        assert!(CronRunner::fire_manual_job(gcx, second, now).await.unwrap());
    }

    #[tokio::test]
    async fn interrupted_effect_holds_even_a_new_manual_request() {
        let now = now_ms();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        let owner = super::super::automation_runtime::execution_owner(&gcx)
            .await
            .unwrap();
        let job = command_due_task("interrupted_effect", now, empty_command());
        let admission = owner.admit(&job, "manual:before-crash").unwrap();
        admission.started().unwrap();
        drop(admission);
        assert!(owner.admit(&job, "manual:new-request").is_err());
        assert_eq!(
            owner.capacity.available_permits(),
            gcx.scheduler_config.max_concurrent_runs.max(1)
        );
    }

    #[tokio::test]
    async fn refused_isolated_open_is_a_rejected_enqueue_not_an_interrupted_effect() {
        let now = now_ms();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        let task = isolated_due_task("isolated_open_refused", now);
        gcx.tool_jobs.scopes.begin_shutdown();
        let runner = CronRunner::new(Arc::new(InMemoryCronStore::new()), gcx.clone());
        let error = runner
            .fire_with_missed(&task, false, now, false)
            .await
            .expect_err("the isolated chat cannot be opened during shutdown");
        assert!(error.contains("nothing was started"), "{error}");
        let owner = super::super::automation_runtime::execution_owner(&gcx)
            .await
            .unwrap();
        let retry = owner.admit(&task, &occurrence_key(&task));
        assert!(
            retry.is_ok(),
            "a refused open must release the claim, not hold it as unknown: {:?}",
            retry.err()
        );
    }

    #[tokio::test]
    async fn duplicate_job_admission_does_not_consume_remaining_capacity() {
        let now = now_ms();
        let (_workspace, gcx) =
            gcx_with_session_and_config(SessionState::Idle, |c| c.max_concurrent_runs = 2).await;
        let owner = super::super::automation_runtime::execution_owner(&gcx)
            .await
            .unwrap();
        let job = command_due_task("duplicate_manual", now, empty_command());
        let admission = owner.admit(&job, "manual:first").unwrap();
        assert!(owner.admit(&job, "manual:second").is_err());
        assert_eq!(owner.capacity.available_permits(), 1);
        let other = command_due_task("different_manual", now, empty_command());
        let _other = owner.admit(&other, "manual:third").unwrap();
        drop(admission);
        assert_eq!(owner.capacity.available_permits(), 1);
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
        let (_workspace, gcx) = gcx_with_session_and_config(SessionState::Idle, |config| {
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
    async fn started_command_failure_is_not_automatically_replayed() {
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        let owner = super::super::automation_runtime::execution_owner(&gcx)
            .await
            .unwrap();
        let mut runner = CronRunner::new(store.clone(), gcx);
        runner.fire_due_tasks(now).await;
        assert!(store.get("command_transient_retry").await.is_none());
        runner.fire_due_tasks(now + 480_000).await;
        assert_eq!(
            owner.capacity.available_permits(),
            runner.gcx.scheduler_config.max_concurrent_runs.max(1)
        );
    }

    /// Cron-lane backpressure holds a retry back without touching its backoff
    /// chain: the same attempt fires once the defer window elapses.
    ///
    /// F-7 moved three assertions here because they encoded the defects this
    /// test was written alongside, not the contract: the deferral used to leave
    /// `last_status` at `"error"` and add no run record (invisible backpressure),
    /// and the retry used to be re-attempted at the very instant it had just
    /// been deferred (the busy spin). The chain 60s -> 120s is what is
    /// load-bearing; its absolute instants shift by one defer window.
    #[tokio::test]
    async fn saturated_cron_lane_defers_retry_without_losing_backoff() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(one_shot_command_due_task(
                "command_saturated_retry",
                now,
                transient_failing_command(),
            ))
            .await
            .unwrap();
        let (_workspace, gcx) = gcx_with_session_and_config(SessionState::Idle, |config| {
            config.max_concurrent_runs = 1;
        })
        .await;
        let mut runner = CronRunner::new(store.clone(), gcx);

        let mut pending = store.get("command_saturated_retry").await.unwrap();
        pending.retry_attempts = 1;
        pending.trigger_at_ms = Some(now + 60_000);
        store.replace(pending).await.unwrap();
        let after_first_fire = store.get("command_saturated_retry").await.unwrap();
        let permit = super::super::automation_runtime::execution_owner(&runner.gcx)
            .await
            .unwrap()
            .capacity
            .clone()
            .try_acquire_owned()
            .unwrap();
        runner.fire_due_tasks(now + 60_000).await;

        let deferred = store.get("command_saturated_retry").await.unwrap();
        assert_eq!(deferred.fire_count, 0);
        assert_eq!(deferred.retry_attempts, 1);
        assert_eq!(deferred.trigger_at_ms, Some(now + 60_000));
        assert_eq!(deferred.last_status.as_deref(), Some("deferred"));
        assert_eq!(
            deferred.recent_runs[..after_first_fire.recent_runs.len()],
            after_first_fire.recent_runs[..]
        );
        assert_eq!(
            deferred.recent_runs.len(),
            after_first_fire.recent_runs.len() + 1
        );
        assert_eq!(deferred.recent_runs.last().unwrap().status, "deferred");
        assert_eq!(
            runner.deferred_until_ms.get("command_saturated_retry"),
            Some(&(now + 60_000 + IDLE_DEFER_MS))
        );

        drop(permit);
        runner.fire_due_tasks(now + 60_000 + IDLE_DEFER_MS).await;

        assert!(store.get("command_saturated_retry").await.is_none());
    }

    /// The spin is ruled out by a *count*, never by a duration: a wall-clock
    /// bound would measure the machine. This mirrors `run()` -- take the
    /// earliest `scheduled_fire_at_ms`, advance a fake clock to it, fire -- and
    /// a defer window that is not honoured returns an instant already in the
    /// past, so the clock stands still while the wake-ups run away.
    #[tokio::test]
    async fn saturated_cron_lane_does_not_busy_spin_on_a_retrying_task() {
        const WINDOW_MS: u64 = 5 * 60_000;
        const MAX_WAKEUPS: u64 = WINDOW_MS / IDLE_DEFER_MS + 2;

        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(one_shot_command_due_task(
                "command_spin_guard",
                now,
                transient_failing_command(),
            ))
            .await
            .unwrap();
        let (_workspace, gcx) = gcx_with_session_and_config(SessionState::Idle, |config| {
            config.max_concurrent_runs = 1;
        })
        .await;
        let mut runner = CronRunner::new(store.clone(), gcx);

        let mut pending = store.get("command_spin_guard").await.unwrap();
        pending.retry_attempts = 1;
        pending.trigger_at_ms = Some(now + 60_000);
        store.replace(pending).await.unwrap();
        let _permit = super::super::automation_runtime::execution_owner(&runner.gcx)
            .await
            .unwrap()
            .capacity
            .clone()
            .try_acquire_owned()
            .unwrap();
        let mut clock = now;
        let mut wakeups = 0u64;
        while clock < now + WINDOW_MS {
            let next = store
                .list()
                .await
                .iter()
                .filter_map(|task| runner.scheduled_fire_at_ms(task, clock))
                .min()
                .unwrap_or(clock + DEFAULT_SLEEP_MS);
            clock = clock.max(next);
            wakeups += 1;
            assert!(
                wakeups <= MAX_WAKEUPS,
                "runner woke {wakeups} times within a {WINDOW_MS}ms window while the lane stayed \
                 saturated (bound {MAX_WAKEUPS}): the defer window is not held and the loop spins"
            );
            runner.fire_due_tasks(clock).await;
        }

        let throttled = store.get("command_spin_guard").await.unwrap();
        assert_eq!(throttled.last_status.as_deref(), Some("deferred"));
        assert_eq!(throttled.fire_count, 0);
        assert_eq!(throttled.retry_attempts, 1);
        assert_eq!(throttled.trigger_at_ms, Some(now + 60_000));
    }

    /// The case that was broken: a retry carries `"error"`, and its defer window
    /// must hold exactly as a rejected delivery's does. Without a defer entry
    /// the same task stays due at its retry instant.
    #[tokio::test]
    async fn defer_window_is_honoured_while_last_status_is_error() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut task = one_shot_command_due_task("command_defer_after_error", now, empty_command());
        task.retry_attempts = 1;
        task.trigger_at_ms = Some(now);
        task.last_status = Some("error".to_string());
        task.recent_runs = vec![CronRunRecord {
            at_ms: now,
            status: "error".to_string(),
            error: Some("429 Too Many Requests".to_string()),
        }];
        store.add(task.clone()).await.unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx);

        assert!(runner.task_is_due(&task, now));

        runner.defer_task(&task, now);

        assert_eq!(
            runner.scheduled_fire_at_ms(&task, now),
            Some(now + IDLE_DEFER_MS)
        );
        assert!(!runner.task_is_due(&task, now + IDLE_DEFER_MS - 1));
        assert!(runner.task_is_due(&task, now + IDLE_DEFER_MS));
    }

    #[tokio::test]
    async fn command_success_resets_retry_backoff() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut task = command_due_task("command_retry_reset", now, stdout_command("ok"));
        task.retry_attempts = 2;
        task.trigger_at_ms = Some(now);
        store.add(task).await.unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
    async fn durable_runner_refreshes_selected_controls_and_discovers_new_jobs() {
        let now = now_ms();
        let temp = tempfile::tempdir().unwrap();
        let store: Arc<dyn CronStore> = Arc::new(JsonFileCronStore::new(temp.path()).unwrap());
        let controls = JsonFileCronStore::new(temp.path()).unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());
        for id in ["deleted", "paused", "rescheduled"] {
            let selected = due_task(id, now);
            controls.add(selected.clone()).await.unwrap();
            if id == "deleted" {
                controls.remove(id).await.unwrap();
            } else {
                controls
                    .mutate(
                        id,
                        Box::new(move |job| {
                            if id == "paused" {
                                job.enabled = false;
                                job.paused_at_ms = Some(now);
                            } else {
                                job.set_trigger(Trigger::Once {
                                    at_ms: now + 60_000,
                                });
                            }
                            Ok(())
                        }),
                    )
                    .await
                    .unwrap();
            }
            assert!(
                !runner.handle_due_task(selected, now).await,
                "stale selection {id} fired"
            );
        }
        {
            let session = session(&gcx).await;
            let session = session.lock().await;
            assert!(session.messages.is_empty());
            assert!(session.pending_deliveries.is_empty());
        }
        controls.add(due_task("new_job", now)).await.unwrap();
        runner.fire_due_tasks(now).await;
        assert_eq!(controls.get("new_job").await.unwrap().fire_count, 1);
        assert_eq!(controls.get("paused").await.unwrap().fire_count, 0);
        assert_eq!(controls.get("rescheduled").await.unwrap().fire_count, 0);
        assert!(controls.get("deleted").await.is_none());
    }

    #[tokio::test]
    async fn old_completion_does_not_delete_rescheduled_one_shot() {
        let now = now_ms();
        let store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let selected = one_shot_task("rescheduled_completion", now);
        store.add(selected.clone()).await.unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx);
        store
            .mutate(
                &selected.id,
                Box::new(move |job| {
                    job.set_trigger(Trigger::Once {
                        at_ms: now + 60_000,
                    });
                    Ok(())
                }),
            )
            .await
            .unwrap();
        runner
            .record_run(&selected, "fired", None, now)
            .await
            .unwrap();
        runner.finish_task_after_fire(&selected, false).await;
        let current = store.get(&selected.id).await.unwrap();
        assert_eq!(
            current.trigger,
            Trigger::Once {
                at_ms: now + 60_000
            }
        );
        assert_eq!(current.last_fired_at_ms, None);
        assert_eq!(current.fire_count, 0);
        assert_eq!(current.recent_runs.last().unwrap().status, "fired");
        assert_eq!(
            runner.scheduled_fire_at_ms(&current, now),
            Some(now + 60_000)
        );
        runner.fire_due_tasks(now + 59_999).await;
        assert_no_fire(&runner.gcx).await;
        runner.fire_due_tasks(now + 60_000).await;
        assert!(store.get(&selected.id).await.is_none());
        runner.fire_due_tasks(now + 120_000).await;
        let session = session(&runner.gcx).await;
        let session = session.lock().await;
        assert_eq!(
            session.messages.iter().filter(|m| m.role == "user").count(),
            1
        );
        let fire = event_message(&session, "cron_fire", &selected.id);
        assert_eq!(fire.extra["event"]["payload"]["fire_count"], json!(1));
    }

    #[tokio::test]
    async fn old_completion_preserves_new_run_now_marker() {
        let now = now_ms();
        let store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let selected = due_task("new_run_now_completion", now);
        store.add(selected.clone()).await.unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        let runner = CronRunner::new(store.clone(), gcx);
        store
            .mutate(
                &selected.id,
                Box::new(move |job| {
                    job.trigger_at_ms = Some(now);
                    Ok(())
                }),
            )
            .await
            .unwrap();
        runner
            .record_run(&selected, "fired", None, now)
            .await
            .unwrap();
        let current = store.get(&selected.id).await.unwrap();
        assert_eq!(current.trigger_at_ms, Some(now));
        assert_eq!(current.last_fired_at_ms, selected.last_fired_at_ms);
        assert_eq!(current.fire_count, 0);
        assert!(runner.task_is_due(&current, now));
    }

    #[tokio::test]
    async fn missed_advance_uses_current_controls_and_preserves_newer_count() {
        let now = now_ms();
        let store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let mut selected = interval_task("stale_advance", now, 60_000);
        selected.last_fired_at_ms = Some(now - 600_000);
        store.add(selected.clone()).await.unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        let runner = CronRunner::new(store.clone(), gcx);
        store
            .mutate(
                &selected.id,
                Box::new(move |job| {
                    job.enabled = false;
                    job.fire_count = 9;
                    job.last_fired_at_ms = Some(now);
                    Ok(())
                }),
            )
            .await
            .unwrap();
        assert!(!runner
            .fast_forward_recurring_if_missed(&selected, now)
            .await
            .unwrap());
        let current = store.get(&selected.id).await.unwrap();
        assert_eq!(current.last_fired_at_ms, Some(now));
        assert_eq!(current.fire_count, 9);
        store
            .mutate(
                &selected.id,
                Box::new(move |job| {
                    job.enabled = true;
                    job.last_fired_at_ms = Some(now - 600_000);
                    Ok(())
                }),
            )
            .await
            .unwrap();
        assert!(runner
            .fast_forward_recurring_if_missed(&selected, now)
            .await
            .unwrap());
        assert_eq!(store.get(&selected.id).await.unwrap().fire_count, 9);
    }

    #[tokio::test]
    async fn durable_completion_preserves_edits_and_deleted_jobs() {
        let now = now_ms();
        let temp = tempfile::tempdir().unwrap();
        let store: Arc<dyn CronStore> = Arc::new(JsonFileCronStore::new(temp.path()).unwrap());
        let controls = JsonFileCronStore::new(temp.path()).unwrap();
        controls.add(due_task("completion", now)).await.unwrap();
        let selected = controls.get("completion").await.unwrap();
        controls
            .mutate(
                "completion",
                Box::new(move |job| {
                    job.enabled = false;
                    job.paused_at_ms = Some(now);
                    if let Action::AgentTurn {
                        prompt, ..
                    } = &mut job.action
                    {
                        *prompt = "edited prompt".into();
                    }
                    Ok(())
                }),
            )
            .await
            .unwrap();
        record_run(&store, "completion", "fired", None, now, 20, Some(selected))
            .await
            .unwrap();
        let job = controls.get("completion").await.unwrap();
        assert!(!job.enabled);
        assert_eq!(job.prompt(), Some("edited prompt"));
        assert_eq!(job.fire_count, 1);
        controls.remove("completion").await.unwrap();
        record_run(&store, "completion", "fired", None, now, 20, None)
            .await
            .unwrap();
        assert!(controls.list().await.is_empty());
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

        record_run(
            &store,
            "history_cap",
            "fired",
            None,
            now,
            RECENT_RUNS_CAP,
            None,
        )
        .await
        .unwrap();
        record_run(
            &store,
            "history_cap",
            "deferred",
            Some("busy".to_string()),
            now + 1,
            RECENT_RUNS_CAP,
            None,
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
    async fn due_task_without_chat_id_does_not_spin() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut one_shot = one_shot_task("cron_no_chat_one_shot", now);
        one_shot.set_existing_chat(None);
        store.add(one_shot).await.unwrap();
        let mut recurring = due_task("cron_no_chat_recurring", now);
        recurring.set_existing_chat(None);
        store.add(recurring).await.unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
        let (_workspace, gcx) = gcx_with_closed_session().await;
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
    async fn default_push_accepts_while_generating() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store.add(task("cron_defer", now)).await.unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Generating).await;
        session(&gcx).await.lock().await.start_stream().unwrap();
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.list().await.into_iter().next().unwrap();
        assert_eq!(stored.last_fired_at_ms, Some(now));
        assert_eq!(stored.fire_count, 1);
        assert!(!runner.deferred_until_ms.contains_key("cron_defer"));
        let session = session(&gcx).await;
        let session = session.lock().await;
        assert!(session.messages.is_empty());
        assert_eq!(session.pending_deliveries.len(), 1);
        assert_eq!(session.pending_deliveries[0].push, PushMode::Append);
    }

    #[tokio::test]
    async fn isolated_due_job_creates_fresh_session_and_skips_idle_gate() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        store
            .add(isolated_due_task("cron_isolated_due", now))
            .await
            .unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Generating).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("cron_isolated_due").await.unwrap();
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        assert!(stored
            .last_fired_at_ms
            .is_some_and(|last_fired_at_ms| last_fired_at_ms <= now));
        assert_eq!(stored.fire_count, 1);
        let ids = isolated_session_ids(&gcx, "cron_isolated_due").await;
        assert_eq!(ids, vec!["cron_cron_isolated_due_0".to_string()]);
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;
        runner.fire_due_tasks(now + every_ms).await;

        let ids = isolated_session_ids(&gcx, "cron_isolated_recurring").await;
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"cron_cron_isolated_recurring_0".to_string()));
        assert!(ids.contains(&"cron_cron_isolated_recurring_1".to_string()));
        assert_ne!(ids[0], ids[1]);
        let stored = store.get("cron_isolated_recurring").await.unwrap();
        assert_eq!(stored.fire_count, 2);
    }

    #[tokio::test]
    async fn webhook_trigger_with_zero_auto_expire_survives_fire() {
        let now = now_ms();
        let store: Arc<dyn CronStore> = Arc::new(InMemoryCronStore::new());
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
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
    async fn recurring_task_with_explicit_expiry_survives_due_execution() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        // Direct Rust fields bypass serde normalization: even an explicit,
        // long-elapsed lifetime must not cut a recurring schedule off.
        let task = expired_task("cron_expire_survives", now);
        assert!(task.auto_expire_after_ms > 0);
        assert!(!task_final_after_fire(&task, now));
        store.add(task).await.unwrap();
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("cron_expire_survives").await.unwrap();
        assert!(stored.recurring);
        assert_eq!(stored.fire_count, 1);
        assert_eq!(stored.last_status.as_deref(), Some("fired"));
        let session = session(&gcx).await;
        let session = session.lock().await;
        let fire_event = event_message(&session, "cron_fire", "cron_expire_survives");
        assert_eq!(fire_event.extra["event"]["payload"]["final"], json!(false));
        assert!(!session.messages.iter().any(|message| {
            message.role == EVENT_ROLE
                && message.extra["event"]["subkind"].as_str() == Some("system_notice")
                && message.extra["event"]["payload"]["reason"].as_str() == Some("auto_expired")
        }));
    }

    #[test]
    fn auto_expired_notice_message_shape_is_stable() {
        let now = now_ms();
        let task = expired_task("cron_expire_notice", now);
        let notice_event = auto_expired_notice_message(&task);
        assert_eq!(
            notice_event.extra["event"]["payload"],
            json!({"task_id": "cron_expire_notice", "reason": "auto_expired"})
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
        let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
        gcx.shutdown_flag.store(true, Ordering::Relaxed);

        let handle = spawn(store, gcx.clone());
        tokio::time::advance(Duration::from_millis(200)).await;

        assert!(handle.is_finished());
        handle.await.unwrap();

        // Exercise cancellation while shipping startup is actually waiting on
        // store I/O, rather than relying only on an already-set shutdown flag.
        struct WaitingStore {
            entered: Arc<tokio::sync::Notify>,
        }
        #[async_trait::async_trait]
        impl CronStore for WaitingStore {
            async fn add(&self, _: Job) -> Result<(), String> {
                unreachable!()
            }
            async fn mutate(
                &self,
                _: &str,
                _: refact_scheduler_core::store::JobMutation,
            ) -> Result<Option<Job>, String> {
                unreachable!()
            }
            async fn remove_if(
                &self,
                _: &str,
                _: refact_scheduler_core::store::JobPredicate,
            ) -> Result<bool, String> {
                unreachable!()
            }
            async fn get(&self, _: &str) -> Option<Job> {
                unreachable!()
            }
            async fn replace(&self, _: Job) -> Result<bool, String> {
                unreachable!()
            }
            async fn remove(&self, _: &str) -> Result<bool, String> {
                unreachable!()
            }
            async fn update_fired(&self, _: &str, _: u64, _: u32) -> Result<(), String> {
                unreachable!()
            }
            fn change_notify(&self) -> Arc<tokio::sync::Notify> {
                Arc::new(tokio::sync::Notify::new())
            }
            async fn list(&self) -> Vec<Job> {
                self.entered.notify_one();
                std::future::pending().await
            }
        }
        gcx.shutdown_flag.store(false, Ordering::Relaxed);
        let entered = Arc::new(tokio::sync::Notify::new());
        let handle = spawn(
            Arc::new(WaitingStore {
                entered: entered.clone(),
            }),
            gcx.clone(),
        );
        entered.notified().await;
        gcx.shutdown_flag.store(true, Ordering::Relaxed);
        // Advancing the paused clock wakes the shutdown timer but does not
        // guarantee its task has been polled to completion. Join it within the
        // same 200 ms virtual-time bound instead of inspecting that race.
        // Without cancellation of catch_up, list() never resolves and this
        // timeout must fail; no store notification can accidentally release it.
        tokio::time::timeout(Duration::from_millis(200), handle)
            .await
            .expect("runner must cancel pending startup I/O within 200 ms")
            .unwrap();

        // A recycled dispatch owner must not detach its command children.
        // Block the real command child's missing-target bookkeeping before any
        // external command runs, then observe that its future is dropped.
        struct ChildStore {
            job: Job,
            entered: Arc<tokio::sync::Notify>,
            dropped: Arc<tokio::sync::Notify>,
        }
        struct ChildDrop(Arc<tokio::sync::Notify>);
        impl Drop for ChildDrop {
            fn drop(&mut self) {
                self.0.notify_one();
            }
        }
        #[async_trait::async_trait]
        impl CronStore for ChildStore {
            async fn add(&self, _: Job) -> Result<(), String> {
                unreachable!()
            }
            async fn mutate(
                &self,
                _: &str,
                _: refact_scheduler_core::store::JobMutation,
            ) -> Result<Option<Job>, String> {
                let _drop = ChildDrop(self.dropped.clone());
                self.entered.notify_one();
                std::future::pending().await
            }
            async fn remove_if(
                &self,
                _: &str,
                _: refact_scheduler_core::store::JobPredicate,
            ) -> Result<bool, String> {
                unreachable!()
            }
            async fn get(&self, _: &str) -> Option<Job> {
                let _drop = ChildDrop(self.dropped.clone());
                self.entered.notify_one();
                std::future::pending().await
            }
            async fn replace(&self, _: Job) -> Result<bool, String> {
                unreachable!()
            }
            async fn remove(&self, _: &str) -> Result<bool, String> {
                unreachable!()
            }
            async fn update_fired(&self, _: &str, _: u64, _: u32) -> Result<(), String> {
                unreachable!()
            }
            fn change_notify(&self) -> Arc<tokio::sync::Notify> {
                Arc::new(tokio::sync::Notify::new())
            }
            async fn list(&self) -> Vec<Job> {
                vec![self.job.clone()]
            }
        }
        gcx.shutdown_flag.store(false, Ordering::Relaxed);
        let entered = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(tokio::sync::Notify::new());
        let mut job = command_due_task("cancel_owned_command", now, empty_command());
        job.set_existing_chat(Some("missing-command-target".into()));
        let store = Arc::new(ChildStore {
            job,
            entered: entered.clone(),
            dropped: dropped.clone(),
        });
        let mut runner = CronRunner::new(store, gcx.clone());
        let handle = tokio::spawn(async move { runner.fire_due_tasks(now).await });
        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .expect("real command child must reach controlled store wait");
        handle.abort();
        assert!(handle.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_millis(200), dropped.notified())
            .await
            .expect("aborting dispatch must drop its blocked command child");
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
    async fn durable_task_with_refused_chat_open_is_deferred_as_temporarily_unavailable() {
        let now = now_ms();
        let store = Arc::new(InMemoryCronStore::new());
        let mut one_shot = one_shot_task("cron_durable_refused", now);
        one_shot.set_existing_chat(Some("refused-open-chat".to_string()));
        one_shot.durable = true;
        store.add(one_shot).await.unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let dir = workspace.path().join(".refact").join("trajectories");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("refused-open-chat.json"),
            json!({ "id": "refused-open-chat", "messages": [] }).to_string(),
        )
        .unwrap();
        AppState::from_gcx(gcx.clone())
            .await
            .runtime
            .tool_jobs
            .scopes
            .begin_shutdown();
        let mut runner = CronRunner::new(store.clone(), gcx.clone());

        runner.fire_due_tasks(now).await;

        let stored = store.get("cron_durable_refused").await.unwrap();
        assert_eq!(stored.fire_count, 0);
        let last = stored.recent_runs.last().unwrap();
        assert_eq!(last.status, "deferred");
        let reason = last.error.clone().unwrap_or_default();
        assert!(
            reason.starts_with("chat temporarily unavailable"),
            "refusal must not be reported as a missing trajectory: {reason}"
        );
        assert_eq!(
            runner.deferred_until_ms.get("cron_durable_refused"),
            Some(&(now + IDLE_DEFER_MS)),
            "a refusal uses the ordinary retry deferral, not a tight loop"
        );
        let runs = stored.recent_runs.len();
        runner.fire_due_tasks(now + 1).await;
        assert_eq!(
            store
                .get("cron_durable_refused")
                .await
                .unwrap()
                .recent_runs
                .len(),
            runs,
            "no retry before the deferral expires"
        );
        assert!(!gcx
            .chat_sessions
            .read()
            .await
            .contains_key("refused-open-chat"));
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

        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let mut session = crate::chat::types::ChatSession::new("catch-up-chat".to_string());
        session.set_runtime_state(SessionState::Idle, None);
        gcx.chat_sessions
            .write()
            .await
            .insert("catch-up-chat".to_string(), Arc::new(AMutex::new(session)));

        let durable_job = store.get("cron_catch_up_missed").await.unwrap();
        refact_scheduler_core::automation::AutomationJournal::open(workspace.path())
            .unwrap()
            .put_legacy_job(&durable_job)
            .unwrap();
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
    async fn mode_and_model_land_atomically_at_each_push_boundary() {
        for push in [PushMode::Append, PushMode::WhenIdle, PushMode::Preempt] {
            let now = now_ms();
            let store = Arc::new(InMemoryCronStore::new());
            let mut job = due_task("boundary", now);
            job.push = push;
            job.set_mode(Some("explore".into()));
            set_job_model(&mut job, Some("scheduled-model".into()));
            store.add(job.clone()).await.unwrap();
            let (_workspace, gcx) = gcx_with_session(SessionState::Idle).await;
            let arc = session(&gcx).await;
            {
                let mut session = arc.lock().await;
                session.thread.model = "original".into();
                session.start_stream().unwrap();
                session.turn_depth = 1;
            }
            let mut runner = CronRunner::new(store.clone(), gcx.clone());
            runner.handle_due_task(job.clone(), now).await;
            assert_eq!(store.get(&job.id).await.unwrap().fire_count, 1);
            // Same fire retried at another wall-clock instant is accepted once.
            assert!(runner.fire(&job, false, now + 500).await.unwrap());
            let mut session = arc.lock().await;
            if push != PushMode::Preempt {
                assert_eq!(session.thread.model, "original");
                assert_eq!(session.pending_deliveries.len(), 1);
                assert_eq!(session.pending_deliveries[0].messages.len(), 2);
                session.finish_stream(Some("stop".into()));
                session.drain_pending_deliveries();
                if push == PushMode::WhenIdle {
                    assert_eq!(session.thread.model, "original");
                    session.turn_depth = 0;
                    session.drain_pending_deliveries();
                }
            }
            assert_eq!(session.thread.model, "scheduled-model");
            assert_eq!(session.thread.mode, "explore");
            let events: Vec<_> = session
                .messages
                .iter()
                .filter(|m| {
                    m.role == "user"
                        || m.extra
                            .get("event")
                            .is_some_and(|event| event["subkind"] == "cron_fire")
                })
                .collect();
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].extra["event"]["subkind"], "cron_fire");
            assert_eq!(events[1].role, "user");
            assert!(session.command_queue.is_empty());
        }
    }
}
