use std::collections::HashMap;
use std::sync::Arc;
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
use crate::chat::types::{ChatCommand, CommandRequest};
use crate::files_correction::get_active_project_path;
use crate::global_context::SharedGlobalContext;

use super::jitter::{jittered_next_run_ms, one_shot_jittered_next_run_ms, JitterConfig};
use super::store::{CronStore, InMemoryCronStore, JsonFileCronStore};
use super::types::ScheduledTask;

const DEFAULT_SLEEP_MS: u64 = 60_000;
const IDLE_DEFER_MS: u64 = 30_000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const RUNNER_TZ: chrono_tz::Tz = chrono_tz::UTC;

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
            self.handle_due_task(task, now_ms()).await;
        }
    }

    async fn handle_due_task(&mut self, task: ScheduledTask, now: u64) {
        let Some(chat_id) = task.chat_id.clone() else {
            return;
        };

        if !chat_is_idle(&self.gcx, &chat_id).await {
            self.defer_task(&task, now);
            return;
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
        } else {
            let fire_count = task.fire_count.saturating_add(1);
            if let Err(error) = self.store.update_fired(&task.id, now, fire_count).await {
                tracing::warn!("failed to advance scheduled task {}: {}", task.id, error);
            }
        }
        self.deferred_until_ms.remove(&task.id);
    }

    fn defer_task(&mut self, task: &ScheduledTask, now: u64) {
        self.deferred_until_ms
            .insert(task.id.clone(), now + IDLE_DEFER_MS);
    }

    fn scheduled_fire_at_ms(&self, task: &ScheduledTask, now: u64) -> Option<u64> {
        self.deferred_until_ms
            .get(&task.id)
            .copied()
            .filter(|deferred_at| *deferred_at >= now)
            .or_else(|| scheduled_fire_at_ms(task, now, &self.jitter_cfg))
    }

    fn task_is_due(&self, task: &ScheduledTask, now: u64) -> bool {
        self.scheduled_fire_at_ms(task, now)
            .is_some_and(|fire_at| fire_at <= now)
    }

    async fn fire(&self, task: &ScheduledTask, final_fire: bool) -> Result<bool, String> {
        let chat_id = task
            .chat_id
            .as_ref()
            .ok_or_else(|| format!("Scheduled task {} has no chat_id", task.id))?;
        let session_arc = {
            let sessions = self.gcx.chat_sessions.read().await;
            sessions.get(chat_id).cloned()
        }
        .ok_or_else(|| format!("Chat session {chat_id} not found"))?;
        let app = AppState::from_gcx(self.gcx.clone()).await;
        let event_message = cron_fire_message(task, final_fire);
        let prompt = task.prompt.clone();
        let processor_flag = {
            let mut session = session_arc.lock().await;
            if session.closed {
                return Err(format!("Chat session {chat_id} is closed"));
            }
            if !session.is_idle() {
                return Ok(false);
            }
            session.add_message(event_message);
            session.command_queue.push_back(CommandRequest {
                client_request_id: format!("cron-fire-{}", Uuid::new_v4()),
                priority: false,
                command: ChatCommand::UserMessage {
                    content: serde_json::Value::String(prompt),
                    attachments: vec![],
                    context_files: vec![],
                    suppress_auto_enrichment: false,
                },
            });
            session.emit_queue_update();
            session.queue_notify.notify_one();
            session.queue_processor_running.clone()
        };

        if !processor_flag.swap(true, Ordering::SeqCst) {
            tokio::spawn(process_command_queue(app, session_arc, processor_flag));
        }
        Ok(true)
    }

    async fn emit_auto_expired_notice(&self, task: &ScheduledTask) {
        let Some(chat_id) = task.chat_id.as_ref() else {
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

pub async fn spawn_from_active_project(gcx: SharedGlobalContext) -> Option<JoinHandle<()>> {
    if !scheduler_enabled() {
        return None;
    }
    let project_root = get_active_project_path(gcx.clone()).await?;
    let store = match JsonFileCronStore::new(project_root) {
        Ok(store) => Arc::new(store),
        Err(error) => {
            tracing::warn!("scheduler runner disabled: {error}");
            return None;
        }
    };
    Some(spawn(store, gcx))
}

use std::sync::OnceLock;

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

pub async fn chat_is_idle(gcx: &SharedGlobalContext, chat_id: &str) -> bool {
    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return false;
    };
    let session = session_arc.lock().await;
    session.is_idle()
}

fn cron_fire_message(task: &ScheduledTask, final_fire: bool) -> ChatMessage {
    event(
        EventSubkind::CronFire,
        "scheduler.cron",
        json!({
            "task_id": task.id,
            "cron": task.cron,
            "recurring": task.recurring,
            "fire_count": task.fire_count.saturating_add(1),
            "final": final_fire,
        }),
        task.prompt.clone(),
    )
}

fn auto_expired_notice_message(task: &ScheduledTask) -> ChatMessage {
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

fn scheduled_fire_at_ms(task: &ScheduledTask, now: u64, jitter_cfg: &JitterConfig) -> Option<u64> {
    let from_ms = task.last_fired_at_ms.unwrap_or(task.created_at_ms);
    let next = if task.recurring {
        jittered_next_run_ms(&task.cron, from_ms, &task.id, jitter_cfg, RUNNER_TZ)
    } else {
        one_shot_jittered_next_run_ms(&task.cron, from_ms, &task.id, jitter_cfg, RUNNER_TZ)
    }?;
    if task.recurring || task.last_fired_at_ms.is_none() || next <= now {
        Some(next)
    } else {
        None
    }
}

fn task_is_due(task: &ScheduledTask, now: u64, jitter_cfg: &JitterConfig) -> bool {
    scheduled_fire_at_ms(task, now, jitter_cfg).is_some_and(|fire_at| fire_at <= now)
}

fn task_final_after_fire(task: &ScheduledTask, now: u64) -> bool {
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

    fn task(id: &str, now: u64) -> ScheduledTask {
        ScheduledTask {
            id: id.to_string(),
            cron: "*/1 * * * *".to_string(),
            prompt: "scheduled prompt".to_string(),
            description: "scheduled prompt".to_string(),
            recurring: true,
            durable: false,
            created_at_ms: now - 120_000,
            chat_id: Some("chat-1".to_string()),
            mode: Some("agent".to_string()),
            last_fired_at_ms: Some(now - 120_000),
            fire_count: 0,
            auto_expire_after_ms: DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS,
        }
    }

    fn expired_task(id: &str, now: u64) -> ScheduledTask {
        let mut task = task(id, now);
        task.created_at_ms = now - 2 * DAY_MS - 1;
        task.auto_expire_after_ms = 2 * DAY_MS;
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
}
