use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use axum::Extension;
use axum::response::Response;
use hyper::{Body, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock as ARwLock};
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarLoadingSection {
    Workspace,
    Trajectories,
    Tasks,
    Buddy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarLoadingStatus {
    Loading,
    Ready,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "category", rename_all = "snake_case")]
pub enum SidebarEvent {
    LoadingPhase {
        section: SidebarLoadingSection,
        status: SidebarLoadingStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u128>,
    },
    Snapshot {
        trajectories: Vec<TrajectoryMeta>,
        tasks: Vec<TaskMeta>,
        workspace_roots: Vec<String>,
        buddy: serde_json::Value,
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

async fn fetch_workspace_roots(gcx: Arc<ARwLock<GlobalContext>>) -> Vec<String> {
    let gcx_locked = gcx.read().await;
    let folders = gcx_locked.documents_state.workspace_folders.lock().unwrap();
    folders
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect()
}

async fn fetch_trajectories_snapshot(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<Vec<TrajectoryMeta>, String> {
    list_all_trajectories_meta(gcx).await
}

async fn fetch_tasks_snapshot(gcx: Arc<ARwLock<GlobalContext>>) -> Result<Vec<TaskMeta>, String> {
    list_tasks_with_session_state(gcx)
        .await
        .map_err(|e| e.to_string())
}

async fn fetch_snapshot(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<(Vec<TrajectoryMeta>, Vec<TaskMeta>, Vec<String>), String> {
    let (trajectories, tasks, workspace_roots) = tokio::try_join!(
        fetch_trajectories_snapshot(gcx.clone()),
        fetch_tasks_snapshot(gcx.clone()),
        async { Ok::<_, String>(fetch_workspace_roots(gcx.clone()).await) },
    )?;
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

#[derive(Debug)]
enum SidebarInitialPart {
    Workspace(Vec<String>),
    Trajectories(Result<Vec<TrajectoryMeta>, String>),
    Tasks(Result<Vec<TaskMeta>, String>),
    Buddy(serde_json::Value),
}

fn next_envelope(seq_counter: &AtomicU64, event: SidebarEvent) -> SidebarEventEnvelope {
    SidebarEventEnvelope {
        seq: seq_counter.fetch_add(1, Ordering::SeqCst),
        event,
    }
}

fn serialize_envelope(envelope: &SidebarEventEnvelope) -> Option<String> {
    serde_json::to_string(envelope)
        .ok()
        .map(|json| format!("data: {}\n\n", json))
}

fn loading_event(section: SidebarLoadingSection) -> SidebarEvent {
    SidebarEvent::LoadingPhase {
        section,
        status: SidebarLoadingStatus::Loading,
        message: None,
        elapsed_ms: None,
    }
}

fn ready_event(section: SidebarLoadingSection, elapsed_ms: u128) -> SidebarEvent {
    SidebarEvent::LoadingPhase {
        section,
        status: SidebarLoadingStatus::Ready,
        message: None,
        elapsed_ms: Some(elapsed_ms),
    }
}

fn error_event(section: SidebarLoadingSection, message: String, elapsed_ms: u128) -> SidebarEvent {
    SidebarEvent::LoadingPhase {
        section,
        status: SidebarLoadingStatus::Error,
        message: Some(message),
        elapsed_ms: Some(elapsed_ms),
    }
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

        if trajectory_rx.is_none() && task_rx.is_none() && notification_rx.is_none() {
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
        let initial_started_at = std::time::Instant::now();
        for section in [
            SidebarLoadingSection::Workspace,
            SidebarLoadingSection::Buddy,
            SidebarLoadingSection::Tasks,
            SidebarLoadingSection::Trajectories,
        ] {
            let envelope = next_envelope(&seq_counter, loading_event(section));
            if let Some(serialized) = serialize_envelope(&envelope) {
                yield Ok::<_, std::convert::Infallible>(serialized);
            }
        }

        let mut trajectories: Option<Vec<TrajectoryMeta>> = None;
        let mut tasks: Option<Vec<TaskMeta>> = None;
        let mut workspace_roots: Option<Vec<String>> = None;
        let mut buddy_snap: Option<serde_json::Value> = None;
        let mut initial_parts = JoinSet::new();

        initial_parts.spawn({
            let gcx = gcx.clone();
            async move { SidebarInitialPart::Workspace(fetch_workspace_roots(gcx).await) }
        });
        initial_parts.spawn({
            let gcx = gcx.clone();
            async move { SidebarInitialPart::Trajectories(fetch_trajectories_snapshot(gcx).await) }
        });
        initial_parts.spawn({
            let gcx = gcx.clone();
            async move { SidebarInitialPart::Tasks(fetch_tasks_snapshot(gcx).await) }
        });
        initial_parts.spawn({
            let gcx = gcx.clone();
            async move { SidebarInitialPart::Buddy(fetch_buddy_snapshot(gcx).await) }
        });

        while let Some(part) = initial_parts.join_next().await {
            match part {
                Ok(SidebarInitialPart::Workspace(roots)) => {
                    tracing::info!(
                        "sidebar snapshot: workspace roots ready in {:.3}s",
                        initial_started_at.elapsed().as_secs_f32(),
                    );
                    workspace_roots = Some(roots.clone());
                    let elapsed_ms = initial_started_at.elapsed().as_millis();
                    let events = vec![
                        SidebarEvent::WorkspaceSnapshot { workspace_roots: roots },
                        ready_event(SidebarLoadingSection::Workspace, elapsed_ms),
                    ];
                    for event in events {
                        let envelope = next_envelope(&seq_counter, event);
                        if let Some(serialized) = serialize_envelope(&envelope) {
                            yield Ok::<_, std::convert::Infallible>(serialized);
                        }
                    }
                }
                Ok(SidebarInitialPart::Trajectories(Ok(items))) => {
                    tracing::info!(
                        "sidebar snapshot: {} trajectories ready in {:.3}s",
                        items.len(),
                        initial_started_at.elapsed().as_secs_f32(),
                    );
                    trajectories = Some(items.clone());
                    let elapsed_ms = initial_started_at.elapsed().as_millis();
                    let events = vec![
                        SidebarEvent::TrajectoriesSnapshot { trajectories: items },
                        ready_event(SidebarLoadingSection::Trajectories, elapsed_ms),
                    ];
                    for event in events {
                        let envelope = next_envelope(&seq_counter, event);
                        if let Some(serialized) = serialize_envelope(&envelope) {
                            yield Ok::<_, std::convert::Infallible>(serialized);
                        }
                    }
                }
                Ok(SidebarInitialPart::Trajectories(Err(err))) => {
                    let elapsed_ms = initial_started_at.elapsed().as_millis();
                    let envelope = next_envelope(
                        &seq_counter,
                        error_event(SidebarLoadingSection::Trajectories, err, elapsed_ms),
                    );
                    if let Some(serialized) = serialize_envelope(&envelope) {
                        yield Ok::<_, std::convert::Infallible>(serialized);
                    }
                }
                Ok(SidebarInitialPart::Tasks(Ok(items))) => {
                    tracing::info!(
                        "sidebar snapshot: {} tasks ready in {:.3}s",
                        items.len(),
                        initial_started_at.elapsed().as_secs_f32(),
                    );
                    tasks = Some(items.clone());
                    let elapsed_ms = initial_started_at.elapsed().as_millis();
                    let events = vec![
                        SidebarEvent::TasksSnapshot { tasks: items },
                        ready_event(SidebarLoadingSection::Tasks, elapsed_ms),
                    ];
                    for event in events {
                        let envelope = next_envelope(&seq_counter, event);
                        if let Some(serialized) = serialize_envelope(&envelope) {
                            yield Ok::<_, std::convert::Infallible>(serialized);
                        }
                    }
                }
                Ok(SidebarInitialPart::Tasks(Err(err))) => {
                    let elapsed_ms = initial_started_at.elapsed().as_millis();
                    let envelope = next_envelope(
                        &seq_counter,
                        error_event(SidebarLoadingSection::Tasks, err, elapsed_ms),
                    );
                    if let Some(serialized) = serialize_envelope(&envelope) {
                        yield Ok::<_, std::convert::Infallible>(serialized);
                    }
                }
                Ok(SidebarInitialPart::Buddy(buddy)) => {
                    tracing::info!(
                        "sidebar snapshot: buddy ready in {:.3}s",
                        initial_started_at.elapsed().as_secs_f32(),
                    );
                    buddy_snap = Some(buddy.clone());
                    let elapsed_ms = initial_started_at.elapsed().as_millis();
                    let events = vec![
                        SidebarEvent::BuddySnapshot { buddy },
                        ready_event(SidebarLoadingSection::Buddy, elapsed_ms),
                    ];
                    for event in events {
                        let envelope = next_envelope(&seq_counter, event);
                        if let Some(serialized) = serialize_envelope(&envelope) {
                            yield Ok::<_, std::convert::Infallible>(serialized);
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!("sidebar snapshot: initial loader task failed: {}", err);
                }
            }
        }

        let envelope = next_envelope(
            &seq_counter,
            SidebarEvent::Snapshot {
                trajectories: trajectories.unwrap_or_default(),
                tasks: tasks.unwrap_or_default(),
                workspace_roots: workspace_roots.unwrap_or_default(),
                buddy: buddy_snap.unwrap_or_else(|| serde_json::json!({"enabled": false})),
            },
        );
        if let Some(serialized) = serialize_envelope(&envelope) {
            yield Ok::<_, std::convert::Infallible>(serialized);
        }

        let mut trajectory_rx = trajectory_rx;
        let mut workspace_changed_rx = workspace_changed_rx;
        let mut task_rx = task_rx;
        let mut notification_rx = notification_rx;
        let mut buddy_rx = buddy_rx;
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                result = async {
                    match &mut trajectory_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match result {
                        Ok(event) => {
                            let envelope = next_envelope(
                                &seq_counter,
                                SidebarEvent::Trajectory(event),
                            );
                            if let Some(serialized) = serialize_envelope(&envelope) {
                                yield Ok::<_, std::convert::Infallible>(serialized);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if let Ok((trajectories, tasks, workspace_roots)) =
                                fetch_snapshot(gcx_for_stream.clone()).await
                            {
                                let buddy = fetch_buddy_snapshot(gcx_for_stream.clone()).await;
                                let envelope = next_envelope(
                                    &seq_counter,
                                    SidebarEvent::Snapshot {
                                        trajectories,
                                        tasks,
                                        workspace_roots,
                                        buddy,
                                    },
                                );
                                if let Some(serialized) = serialize_envelope(&envelope) {
                                    yield Ok::<_, std::convert::Infallible>(serialized);
                                }
                            } else {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            trajectory_rx = None;
                            if task_rx.is_none() && notification_rx.is_none() {
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
                            if let Ok((trajectories, tasks, workspace_roots)) =
                                fetch_snapshot(gcx_for_stream.clone()).await
                            {
                                let buddy = fetch_buddy_snapshot(gcx_for_stream.clone()).await;
                                let envelope = next_envelope(
                                    &seq_counter,
                                    SidebarEvent::Snapshot {
                                        trajectories,
                                        tasks,
                                        workspace_roots,
                                        buddy,
                                    },
                                );
                                if let Some(serialized) = serialize_envelope(&envelope) {
                                    yield Ok::<_, std::convert::Infallible>(serialized);
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            workspace_changed_rx = None;
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
                            let envelope = next_envelope(
                                &seq_counter,
                                SidebarEvent::Task(task_envelope.event),
                            );
                            if let Some(serialized) = serialize_envelope(&envelope) {
                                yield Ok::<_, std::convert::Infallible>(serialized);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if let Ok((trajectories, tasks, workspace_roots)) =
                                fetch_snapshot(gcx_for_stream.clone()).await
                            {
                                let buddy = fetch_buddy_snapshot(gcx_for_stream.clone()).await;
                                let envelope = next_envelope(
                                    &seq_counter,
                                    SidebarEvent::Snapshot {
                                        trajectories,
                                        tasks,
                                        workspace_roots,
                                        buddy,
                                    },
                                );
                                if let Some(serialized) = serialize_envelope(&envelope) {
                                    yield Ok::<_, std::convert::Infallible>(serialized);
                                }
                            } else {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            task_rx = None;
                            if trajectory_rx.is_none() && notification_rx.is_none() {
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
                            let envelope = next_envelope(
                                &seq_counter,
                                SidebarEvent::Notification(event),
                            );
                            if let Some(serialized) = serialize_envelope(&envelope) {
                                yield Ok::<_, std::convert::Infallible>(serialized);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            notification_rx = None;
                            if trajectory_rx.is_none() && task_rx.is_none() {
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
                            let envelope = next_envelope(
                                &seq_counter,
                                SidebarEvent::Buddy { buddy_event: event },
                            );
                            if let Some(serialized) = serialize_envelope(&envelope) {
                                yield Ok::<_, std::convert::Infallible>(serialized);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            buddy_rx = None;
                        }
                    }
                }

                _ = heartbeat.tick() => {
                    yield Ok::<_, std::convert::Infallible>(": hb\n\n".to_string());
                }

                _ = async {
                    let shutdown_flag = gcx_for_stream.read().await.shutdown_flag.clone();
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
