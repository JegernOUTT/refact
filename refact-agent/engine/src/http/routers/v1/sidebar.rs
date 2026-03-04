use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use axum::Extension;
use axum::response::Response;
use hyper::{Body, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock as ARwLock};

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
#[serde(tag = "category", rename_all = "snake_case")]
pub enum SidebarEvent {
    Snapshot {
        trajectories: Vec<TrajectoryMeta>,
        tasks: Vec<TaskMeta>,
    },
    Trajectory(TrajectoryEvent),
    Task(TaskEvent),
    Notification(NotificationEvent),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SidebarEventEnvelope {
    pub seq: u64,
    #[serde(flatten)]
    pub event: SidebarEvent,
}

async fn fetch_snapshot(gcx: Arc<ARwLock<GlobalContext>>) -> Result<(Vec<TrajectoryMeta>, Vec<TaskMeta>), String> {
    let trajectories = list_all_trajectories_meta(gcx.clone()).await?;
    let tasks = list_tasks_with_session_state(gcx.clone())
        .await
        .map_err(|e| e.to_string())?;
    Ok((trajectories, tasks))
}

pub async fn handle_sidebar_subscribe(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let (trajectory_rx, task_rx, notification_rx, seq_counter) = {
        let gcx_locked = gcx.read().await;

        let trajectory_rx = gcx_locked
            .trajectory_events_tx
            .as_ref()
            .map(|tx| tx.subscribe());

        let task_rx = gcx_locked
            .task_events_tx
            .as_ref()
            .map(|tx| tx.subscribe());

        let notification_rx = gcx_locked
            .notification_events_tx
            .as_ref()
            .map(|tx| tx.subscribe());

        if trajectory_rx.is_none() && task_rx.is_none() && notification_rx.is_none() {
            return Err(ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Sidebar events not available".to_string(),
            ));
        }

        let seq_counter = Arc::new(AtomicU64::new(0));
        (trajectory_rx, task_rx, notification_rx, seq_counter)
    };

    let (trajectories, tasks) = fetch_snapshot(gcx.clone())
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let gcx_for_stream = gcx.clone();
    let stream = async_stream::stream! {
        let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
        let envelope = SidebarEventEnvelope {
            seq,
            event: SidebarEvent::Snapshot { trajectories, tasks },
        };
        if let Ok(json) = serde_json::to_string(&envelope) {
            yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
        }

        let mut trajectory_rx = trajectory_rx;
        let mut task_rx = task_rx;
        let mut notification_rx = notification_rx;
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
                            let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
                            let envelope = SidebarEventEnvelope {
                                seq,
                                event: SidebarEvent::Trajectory(event),
                            };
                            if let Ok(json) = serde_json::to_string(&envelope) {
                                yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if let Ok((trajectories, tasks)) = fetch_snapshot(gcx_for_stream.clone()).await {
                                let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
                                let envelope = SidebarEventEnvelope {
                                    seq,
                                    event: SidebarEvent::Snapshot { trajectories, tasks },
                                };
                                if let Ok(json) = serde_json::to_string(&envelope) {
                                    yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
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
                    match &mut task_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match result {
                        Ok(task_envelope) => {
                            let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
                            let envelope = SidebarEventEnvelope {
                                seq,
                                event: SidebarEvent::Task(task_envelope.event),
                            };
                            if let Ok(json) = serde_json::to_string(&envelope) {
                                yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if let Ok((trajectories, tasks)) = fetch_snapshot(gcx_for_stream.clone()).await {
                                let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
                                let envelope = SidebarEventEnvelope {
                                    seq,
                                    event: SidebarEvent::Snapshot { trajectories, tasks },
                                };
                                if let Ok(json) = serde_json::to_string(&envelope) {
                                    yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
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
                            let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
                            let envelope = SidebarEventEnvelope {
                                seq,
                                event: SidebarEvent::Notification(event),
                            };
                            if let Ok(json) = serde_json::to_string(&envelope) {
                                yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
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
