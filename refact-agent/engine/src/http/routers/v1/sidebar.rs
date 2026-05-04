use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use axum::Extension;
use axum::response::Response;
use hyper::{Body, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock as ARwLock, broadcast, mpsc};
use tokio::task::JoinSet;

use crate::buddy::events::BuddyEvent;
use crate::chat::{TrajectoryEvent, TrajectoryMeta, list_all_trajectories_meta};
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::http::routers::v1::tasks::list_tasks_with_session_state;
use crate::tasks::events::TaskEvent;
use crate::tasks::types::TaskMeta;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationEvent {
    TaskDone {
        chat_id: String,
        tool_call_id: String,
        summary: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        knowledge_path: Option<String>,
    },
    AskQuestions {
        chat_id: String,
        tool_call_id: String,
        questions: Vec<NotificationQuestion>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationQuestion {
    pub id: String,
    #[serde(rename = "type")]
    pub question_type: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarLoadingSection {
    Workspace,
    Trajectories,
    Tasks,
    Buddy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "category", rename_all = "snake_case")]
pub enum SidebarEvent {
    Snapshot {
        trajectories: Vec<TrajectoryMeta>,
        tasks: Vec<TaskMeta>,
        workspace_roots: Vec<String>,
        buddy: serde_json::Value,
    },
    LoadingPhase {
        section: SidebarLoadingSection,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u128>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    WorkspaceSnapshot {
        workspace_roots: Vec<String>,
    },
    TrajectoriesSnapshot {
        trajectories: Vec<TrajectoryMeta>,
    },
    TasksSnapshot {
        tasks: Vec<TaskMeta>,
    },
    BuddySnapshot {
        buddy: serde_json::Value,
    },
    Trajectory(TrajectoryEvent),
    Task(TaskEvent),
    Notification(NotificationEvent),
    Buddy {
        buddy_event: BuddyEvent,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SidebarEventEnvelope {
    pub seq: u64,
    #[serde(flatten)]
    pub event: SidebarEvent,
}

#[derive(Debug)]
enum InitialSidebarPart {
    Workspace(Result<Vec<String>, String>),
    Trajectories(Result<Vec<TrajectoryMeta>, String>),
    Tasks(Result<Vec<TaskMeta>, String>),
    Buddy(serde_json::Value),
}

fn all_receivers_closed(
    trajectory_rx: &Option<broadcast::Receiver<TrajectoryEvent>>,
    workspace_changed_rx: &Option<broadcast::Receiver<()>>,
    task_rx: &Option<broadcast::Receiver<crate::tasks::events::TaskEventEnvelope>>,
    notification_rx: &Option<broadcast::Receiver<NotificationEvent>>,
    buddy_rx: &Option<broadcast::Receiver<BuddyEvent>>,
) -> bool {
    trajectory_rx.is_none()
        && workspace_changed_rx.is_none()
        && task_rx.is_none()
        && notification_rx.is_none()
        && buddy_rx.is_none()
}

async fn fetch_workspace_roots(gcx: Arc<ARwLock<GlobalContext>>) -> Vec<String> {
    let gcx_locked = gcx.read().await;
    let folders = gcx_locked.documents_state.workspace_folders.lock().unwrap();
    folders
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect()
}

async fn fetch_snapshot(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<(Vec<TrajectoryMeta>, Vec<TaskMeta>, Vec<String>), String> {
    let trajectories = list_all_trajectories_meta(gcx.clone()).await?;
    let tasks = list_tasks_with_session_state(gcx.clone())
        .await
        .map_err(|e| e.to_string())?;
    let workspace_roots = fetch_workspace_roots(gcx).await;
    Ok((trajectories, tasks, workspace_roots))
}

async fn fetch_buddy_snapshot(gcx: Arc<ARwLock<GlobalContext>>) -> serde_json::Value {
    let buddy_arc = gcx.read().await.buddy.clone();
    let locked = buddy_arc.lock().await;
    match locked.as_ref() {
        Some(svc) => {
            serde_json::to_value(&svc.snapshot()).unwrap_or(serde_json::json!({"enabled": false}))
        }
        None => serde_json::json!({"enabled": false}),
    }
}

fn make_event(seq_counter: &AtomicU64, event: SidebarEvent) -> Option<String> {
    let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
    let envelope = SidebarEventEnvelope { seq, event };
    serde_json::to_string(&envelope)
        .ok()
        .map(|json| format!("data: {}\n\n", json))
}

fn loading_event(
    section: SidebarLoadingSection,
    status: &str,
    elapsed_ms: Option<u128>,
    error: Option<String>,
) -> SidebarEvent {
    SidebarEvent::LoadingPhase {
        section,
        status: status.to_string(),
        elapsed_ms,
        error,
    }
}

async fn emit_full_snapshot(
    gcx: Arc<ARwLock<GlobalContext>>,
    seq_counter: &AtomicU64,
) -> Option<String> {
    match fetch_snapshot(gcx.clone()).await {
        Ok((trajectories, tasks, workspace_roots)) => {
            let buddy = fetch_buddy_snapshot(gcx).await;
            make_event(
                seq_counter,
                SidebarEvent::Snapshot {
                    trajectories,
                    tasks,
                    workspace_roots,
                    buddy,
                },
            )
        }
        Err(error) => {
            tracing::warn!("sidebar full snapshot recovery failed: {error}");
            None
        }
    }
}

fn spawn_initial_sidebar_loads(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> mpsc::UnboundedReceiver<InitialSidebarPart> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut jobs = JoinSet::new();

        jobs.spawn({
            let gcx = gcx.clone();
            async move {
                let roots = fetch_workspace_roots(gcx).await;
                InitialSidebarPart::Workspace(Ok(roots))
            }
        });

        jobs.spawn({
            let gcx = gcx.clone();
            async move { InitialSidebarPart::Trajectories(list_all_trajectories_meta(gcx).await) }
        });

        jobs.spawn({
            let gcx = gcx.clone();
            async move {
                InitialSidebarPart::Tasks(
                    list_tasks_with_session_state(gcx)
                        .await
                        .map_err(|e| e.to_string()),
                )
            }
        });

        jobs.spawn({
            let gcx = gcx.clone();
            async move { InitialSidebarPart::Buddy(fetch_buddy_snapshot(gcx).await) }
        });

        while let Some(result) = jobs.join_next().await {
            match result {
                Ok(part) => {
                    if tx.send(part).is_err() {
                        jobs.abort_all();
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!("sidebar initial load task failed: {e}");
                }
            }
        }
    });
    rx
}

pub async fn handle_sidebar_subscribe(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let (trajectory_rx, workspace_changed_rx, task_rx, notification_rx, buddy_rx, seq_counter) = {
        let gcx_locked = gcx.read().await;

        let trajectory_rx = gcx_locked
            .trajectory_events_tx
            .as_ref()
            .map(|tx| tx.subscribe());

        let workspace_changed_rx = gcx_locked
            .workspace_changed_tx
            .as_ref()
            .map(|tx| tx.subscribe());

        let task_rx = gcx_locked.task_events_tx.as_ref().map(|tx| tx.subscribe());

        let notification_rx = gcx_locked
            .notification_events_tx
            .as_ref()
            .map(|tx| tx.subscribe());

        let buddy_rx = gcx_locked.buddy_events_tx.as_ref().map(|tx| tx.subscribe());

        if trajectory_rx.is_none()
            && workspace_changed_rx.is_none()
            && task_rx.is_none()
            && notification_rx.is_none()
            && buddy_rx.is_none()
        {
            return Err(ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Sidebar events not available".to_string(),
            ));
        }

        let seq_counter = Arc::new(AtomicU64::new(0));
        (
            trajectory_rx,
            workspace_changed_rx,
            task_rx,
            notification_rx,
            buddy_rx,
            seq_counter,
        )
    };

    let gcx_for_stream = gcx.clone();
    let stream = async_stream::stream! {
        let mut trajectory_rx = trajectory_rx;
        let mut workspace_changed_rx = workspace_changed_rx;
        let mut task_rx = task_rx;
        let mut notification_rx = notification_rx;
        let mut buddy_rx = buddy_rx;
        let mut initial_rx = Some(spawn_initial_sidebar_loads(gcx_for_stream.clone()));
        let mut workspace_ready = false;
        let mut trajectories_ready = false;
        let mut tasks_ready = false;
        let mut buddy_ready = false;
        let mut live_events_started = false;
        let initial_started_at = Instant::now();
        let shutdown_flag = gcx_for_stream.read().await.shutdown_flag.clone();

        for section in [
            SidebarLoadingSection::Workspace,
            SidebarLoadingSection::Trajectories,
            SidebarLoadingSection::Tasks,
            SidebarLoadingSection::Buddy,
        ] {
            if let Some(event) = make_event(
                &seq_counter,
                loading_event(section, "started", Some(initial_started_at.elapsed().as_millis()), None),
            ) {
                yield Ok::<_, std::convert::Infallible>(event);
            }
        }

        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                part = async {
                    match &mut initial_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match part {
                        Some(InitialSidebarPart::Workspace(result)) => {
                            match result {
                                Ok(roots) => {
                                    tracing::info!("sidebar initial workspace ready in {}ms", initial_started_at.elapsed().as_millis());
                                    workspace_ready = true;
                                    if let Some(event) = make_event(&seq_counter, SidebarEvent::WorkspaceSnapshot { workspace_roots: roots }) {
                                        yield Ok::<_, std::convert::Infallible>(event);
                                    }
                                    if let Some(event) = make_event(&seq_counter, loading_event(SidebarLoadingSection::Workspace, "ready", Some(initial_started_at.elapsed().as_millis()), None)) {
                                        yield Ok::<_, std::convert::Infallible>(event);
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!("sidebar initial workspace failed: {error}");
                                    workspace_ready = true;
                                    if let Some(event) = make_event(&seq_counter, SidebarEvent::WorkspaceSnapshot { workspace_roots: Vec::new() }) {
                                        yield Ok::<_, std::convert::Infallible>(event);
                                    }
                                    if let Some(event) = make_event(&seq_counter, loading_event(SidebarLoadingSection::Workspace, "error", Some(initial_started_at.elapsed().as_millis()), Some(error))) {
                                        yield Ok::<_, std::convert::Infallible>(event);
                                    }
                                }
                            }
                        }
                        Some(InitialSidebarPart::Trajectories(result)) => {
                            match result {
                                Ok(items) => {
                                    tracing::info!("sidebar initial trajectories ready in {}ms", initial_started_at.elapsed().as_millis());
                                    trajectories_ready = true;
                                    if let Some(event) = make_event(&seq_counter, SidebarEvent::TrajectoriesSnapshot { trajectories: items }) {
                                        yield Ok::<_, std::convert::Infallible>(event);
                                    }
                                    if let Some(event) = make_event(&seq_counter, loading_event(SidebarLoadingSection::Trajectories, "ready", Some(initial_started_at.elapsed().as_millis()), None)) {
                                        yield Ok::<_, std::convert::Infallible>(event);
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!("sidebar initial trajectories failed: {error}");
                                    trajectories_ready = true;
                                    if let Some(event) = make_event(&seq_counter, SidebarEvent::TrajectoriesSnapshot { trajectories: Vec::new() }) {
                                        yield Ok::<_, std::convert::Infallible>(event);
                                    }
                                    if let Some(event) = make_event(&seq_counter, loading_event(SidebarLoadingSection::Trajectories, "error", Some(initial_started_at.elapsed().as_millis()), Some(error))) {
                                        yield Ok::<_, std::convert::Infallible>(event);
                                    }
                                }
                            }
                        }
                        Some(InitialSidebarPart::Tasks(result)) => {
                            match result {
                                Ok(items) => {
                                    tracing::info!("sidebar initial tasks ready in {}ms", initial_started_at.elapsed().as_millis());
                                    tasks_ready = true;
                                    if let Some(event) = make_event(&seq_counter, SidebarEvent::TasksSnapshot { tasks: items }) {
                                        yield Ok::<_, std::convert::Infallible>(event);
                                    }
                                    if let Some(event) = make_event(&seq_counter, loading_event(SidebarLoadingSection::Tasks, "ready", Some(initial_started_at.elapsed().as_millis()), None)) {
                                        yield Ok::<_, std::convert::Infallible>(event);
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!("sidebar initial tasks failed: {error}");
                                    tasks_ready = true;
                                    if let Some(event) = make_event(&seq_counter, SidebarEvent::TasksSnapshot { tasks: Vec::new() }) {
                                        yield Ok::<_, std::convert::Infallible>(event);
                                    }
                                    if let Some(event) = make_event(&seq_counter, loading_event(SidebarLoadingSection::Tasks, "error", Some(initial_started_at.elapsed().as_millis()), Some(error))) {
                                        yield Ok::<_, std::convert::Infallible>(event);
                                    }
                                }
                            }
                        }
                        Some(InitialSidebarPart::Buddy(snapshot)) => {
                            tracing::info!("sidebar initial buddy ready in {}ms", initial_started_at.elapsed().as_millis());
                            buddy_ready = true;
                            if let Some(event) = make_event(&seq_counter, SidebarEvent::BuddySnapshot { buddy: snapshot }) {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                            if let Some(event) = make_event(&seq_counter, loading_event(SidebarLoadingSection::Buddy, "ready", Some(initial_started_at.elapsed().as_millis()), None)) {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                        }
                        None => {
                            initial_rx = None;
                        }
                    }

                    if workspace_ready && trajectories_ready && tasks_ready && buddy_ready {
                        if !live_events_started {
                            if let Some(event) = emit_full_snapshot(gcx_for_stream.clone(), &seq_counter).await {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                        }
                        initial_rx = None;
                    }
                }

                result = async {
                    match &mut trajectory_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match result {
                        Ok(event) => {
                            live_events_started = true;
                            if let Some(event) = make_event(&seq_counter, SidebarEvent::Trajectory(event)) {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if let Some(event) = emit_full_snapshot(gcx_for_stream.clone(), &seq_counter).await {
                                yield Ok::<_, std::convert::Infallible>(event);
                            } else {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            trajectory_rx = None;
                            if all_receivers_closed(&trajectory_rx, &workspace_changed_rx, &task_rx, &notification_rx, &buddy_rx) {
                                break;
                            }
                        }
                    }
                }

                result = async {
                    match &mut workspace_changed_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match result {
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                            live_events_started = true;
                            if let Ok((_trajectories, _tasks, workspace_roots)) = fetch_snapshot(gcx_for_stream.clone()).await {
                                if let Some(event) = make_event(&seq_counter, SidebarEvent::WorkspaceSnapshot { workspace_roots }) {
                                    yield Ok::<_, std::convert::Infallible>(event);
                                }
                                if let Some(event) = emit_full_snapshot(gcx_for_stream.clone(), &seq_counter).await {
                                    yield Ok::<_, std::convert::Infallible>(event);
                                }
                            } else {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            workspace_changed_rx = None;
                            if all_receivers_closed(&trajectory_rx, &workspace_changed_rx, &task_rx, &notification_rx, &buddy_rx) {
                                break;
                            }
                        }
                    }
                }

                result = async {
                    match &mut task_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match result {
                        Ok(task_envelope) => {
                            live_events_started = true;
                            if let Some(event) = make_event(&seq_counter, SidebarEvent::Task(task_envelope.event)) {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if let Some(event) = emit_full_snapshot(gcx_for_stream.clone(), &seq_counter).await {
                                yield Ok::<_, std::convert::Infallible>(event);
                            } else {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            task_rx = None;
                            if all_receivers_closed(&trajectory_rx, &workspace_changed_rx, &task_rx, &notification_rx, &buddy_rx) {
                                break;
                            }
                        }
                    }
                }

                result = async {
                    match &mut notification_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match result {
                        Ok(event) => {
                            live_events_started = true;
                            if let Some(event) = make_event(&seq_counter, SidebarEvent::Notification(event)) {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            tracing::warn!("sidebar notification event receiver lagged; dropping unrecoverable notification events");
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            notification_rx = None;
                            if all_receivers_closed(&trajectory_rx, &workspace_changed_rx, &task_rx, &notification_rx, &buddy_rx) {
                                break;
                            }
                        }
                    }
                }

                result = async {
                    match &mut buddy_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match result {
                        Ok(event) => {
                            live_events_started = true;
                            if let Some(event) = make_event(&seq_counter, SidebarEvent::Buddy { buddy_event: event }) {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let buddy = fetch_buddy_snapshot(gcx_for_stream.clone()).await;
                            if let Some(event) = make_event(&seq_counter, SidebarEvent::BuddySnapshot { buddy }) {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            buddy_rx = None;
                            if all_receivers_closed(&trajectory_rx, &workspace_changed_rx, &task_rx, &notification_rx, &buddy_rx) {
                                break;
                            }
                        }
                    }
                }

                _ = heartbeat.tick() => {
                    yield Ok::<_, std::convert::Infallible>(": hb\n\n".to_string());
                }

                _ = async {
                    while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                } => {
                    break;
                }
            }
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::wrap_stream(stream))
        .unwrap())
}
