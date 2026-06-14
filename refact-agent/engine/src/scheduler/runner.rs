use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::Instant as TokioInstant;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::process_command_queue;
use crate::chat::try_restore_session_if_trajectory_exists;
use crate::chat::types::{ChatCommand, CommandRequest, EnqueueCommandOutcome, max_queue_size};
use crate::files_correction::get_active_project_path;
use crate::global_context::SharedGlobalContext;
use crate::scheduler::scheduler_timezone;

use super::jitter::{jittered_next_run_ms_for, one_shot_jittered_next_run_ms_for, JitterConfig};
use super::schedule::next_run_ms;
use super::store::{CronStore, InMemoryCronStore, JsonFileCronStore};
use super::types::{Action, AgentTarget, Delivery, Job, Trigger};

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
            deferred_until_ms: HashMap::new(),
        }
    }

    pub fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            self.run().await;
        })
    }

    async fn run(mut self) {
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
            let Some(chat_id) = task.chat_id().map(str::to_string) else {
                continue;
            };
            let app = AppState::from_gcx(self.gcx.clone()).await;
            let restored =
                try_restore_session_if_trajectory_exists(app, &self.gcx.chat_sessions, &chat_id)
                    .await;
            if !restored {
                tracing::warn!(
                    "skipping missed durable one-shot {}: no trajectory found for chat {}",
                    task.id,
                    chat_id
                );
                continue;
            }
            match self.fire_with_missed(&task, true, true).await {
                Ok(true) => {
                    *missed_counts.entry(chat_id).or_default() += 1;
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
        if self
            .scheduled_fire_at_ms(task, now)
            .is_some_and(|fire_at| fire_at < now)
        {
            if let Err(error) = self
                .store
                .update_fired(&task.id, now, task.fire_count)
                .await
            {
                tracing::warn!("failed to resume scheduled task {}: {}", task.id, error);
            }
        }
    }

    async fn fire_due_tasks(&mut self, now: u64) {
        let due_tasks = self
            .store
            .list()
            .await
            .into_iter()
            .filter(|task| self.task_is_due(task, now))
            .collect::<Vec<_>>();

        for task in due_tasks {
            if self.shutdown_flag.load(Ordering::Relaxed) {
                break;
            }
            self.handle_due_task(task, now).await;
        }
    }

    async fn handle_due_task(&mut self, task: Job, now: u64) {
        let chat_id = match runnable_chat_id(&task) {
            Ok(chat_id) => chat_id,
            Err("missing chat_id") => {
                self.handle_unfireable_task(&task, now, "missing chat_id")
                    .await;
                return;
            }
            Err(reason) => {
                tracing::warn!("skipping scheduled task {} because {}", task.id, reason);
                self.defer_invalid_target_task(&task, now);
                return;
            }
        };

        match chat_fire_status(&self.gcx, chat_id).await {
            ChatFireStatus::Fireable => {}
            ChatFireStatus::Busy => {
                self.defer_task(&task, now);
                return;
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
                        self.defer_invalid_target_task(&task, now);
                        return;
                    }
                    match chat_fire_status(&self.gcx, chat_id).await {
                        ChatFireStatus::Fireable => {}
                        status => {
                            tracing::warn!(
                                "durable task {} deferred after session restore ({:?})",
                                task.id,
                                status
                            );
                            self.defer_invalid_target_task(&task, now);
                            return;
                        }
                    }
                } else {
                    self.handle_unfireable_task(&task, now, "chat session not found")
                        .await;
                    return;
                }
            }
            ChatFireStatus::Closed => {
                self.handle_unfireable_task(&task, now, "chat session is closed")
                    .await;
                return;
            }
        }

        let final_fire = task_final_after_fire(&task, now);
        match self.fire(&task, final_fire).await {
            Ok(true) => {}
            Ok(false) => {
                self.defer_task(&task, now);
                return;
            }
            Err(error) => {
                tracing::warn!("failed to fire scheduled task {}: {}", task.id, error);
                self.handle_unfireable_task(&task, now, &error).await;
                return;
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
        } else {
            let fire_count = task.fire_count.saturating_add(1);
            if let Err(error) = self.store.update_fired(&task.id, now, fire_count).await {
                tracing::warn!("failed to advance scheduled task {}: {}", task.id, error);
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
            self.defer_invalid_target_task(task, now);
            return;
        }

        tracing::warn!(
            "removing one-shot scheduled task {} because it is not fireable: {}",
            task.id,
            reason
        );
        if let Err(error) = self.remove_task(task).await {
            tracing::warn!(
                "failed to remove unfireable one-shot scheduled task {}: {}",
                task.id,
                error
            );
        }
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
        self.deferred_until_ms
            .get(&task.id)
            .copied()
            .filter(|deferred_at| *deferred_at >= now)
            .or_else(|| scheduled_fire_at_ms(task, now, &self.jitter_cfg))
    }

    fn task_is_due(&self, task: &Job, now: u64) -> bool {
        self.scheduled_fire_at_ms(task, now)
            .is_some_and(|fire_at| fire_at <= now)
    }

    async fn fire(&self, task: &Job, final_fire: bool) -> Result<bool, String> {
        self.fire_with_missed(task, final_fire, false).await
    }

    async fn fire_with_missed(
        &self,
        task: &Job,
        final_fire: bool,
        missed: bool,
    ) -> Result<bool, String> {
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
    if !scheduler_enabled() {
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
    config: crate::scheduler::types::SchedulerConfig,
) -> Option<JoinHandle<()>> {
    if !config.enabled || !scheduler_enabled() {
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
    if !matches!(job.trigger, Trigger::Cron { .. }) {
        return Err("non-cron trigger is not supported yet");
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
        (Action::Command { .. }, _) => Err("command actions are not supported yet"),
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
        "fire_count": task.fire_count.saturating_add(1),
        "final": final_fire,
    });
    if missed {
        payload["missed"] = json!(true);
    }
    event(
        EventSubkind::CronFire,
        "scheduler.cron",
        payload,
        task.prompt().unwrap_or_default().to_string(),
    )
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

fn scheduled_fire_at_ms(task: &Job, _now: u64, jitter_cfg: &JitterConfig) -> Option<u64> {
    let tz = scheduler_timezone();
    let from_ms = task.last_fired_at_ms.unwrap_or(task.created_at_ms);
    let next = if task.recurring {
        jittered_next_run_ms_for(task, from_ms, &task.id, jitter_cfg, tz)
    } else {
        one_shot_jittered_next_run_ms_for(task, from_ms, &task.id, jitter_cfg, tz)
    }?;
    if task.recurring || task.last_fired_at_ms.is_none() {
        Some(next)
    } else {
        None
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

    use tokio::sync::Mutex as AMutex;

    use super::*;
    use crate::chat::internal_roles::EVENT_ROLE;
    use crate::chat::types::{ChatSession, SessionState};
    use crate::scheduler::store::InMemoryCronStore;
    use crate::scheduler::types::DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS;

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
