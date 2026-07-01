use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::response::Response;
use axum::extract::State;
use hyper::{Body, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinSet;
use tokio::time::timeout;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::global_context::GlobalContext;
use crate::buddy::events::BuddyEvent;
use crate::chat::trajectories::trajectory_event_is_displayable_chat;
use crate::chat::{list_trajectories_page, TrajectoryEvent, TrajectoryMeta};
use crate::custom_error::ScratchError;
use crate::http::routers::v1::tasks::list_tasks_with_session_state;
use crate::tasks::events::TaskEvent;
use crate::tasks::types::TaskMeta;

const SIDEBAR_PROTOCOL_VERSION: u8 = 2;
const SIDEBAR_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(60);
const SIDEBAR_RETRY_DELAY: Duration = Duration::from_secs(5);
const SIDEBAR_CHATS_PAGE_SIZE: usize = 50;

pub use refact_chat_api::notification_event::{NotificationEvent, NotificationQuestion};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarSection {
    Workspace,
    Chats,
    Tasks,
    Buddy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarSectionStatus {
    Ready,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SidebarSectionSnapshot {
    Workspace {
        workspace_roots: Vec<String>,
    },
    Chats {
        trajectories: Vec<TrajectoryMeta>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pagination: Option<SidebarPagination>,
    },
    Tasks {
        tasks: Vec<TaskMeta>,
    },
    Buddy {
        buddy: serde_json::Value,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SidebarPagination {
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub total_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SidebarSectionUpdate {
    Trajectory(TrajectoryEvent),
    Task(TaskEvent),
    Buddy(BuddyEvent),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SidebarEvent {
    SectionSnapshot {
        section: SidebarSection,
        status: SidebarSectionStatus,
        snapshot: SidebarSectionSnapshot,
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u128>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    SectionUpdate {
        section: SidebarSection,
        update: SidebarSectionUpdate,
    },
    Notification {
        notification: NotificationEvent,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SidebarEventEnvelope {
    pub protocol_version: u8,
    pub seq: u64,
    pub subscription_id: String,
    pub event: SidebarEvent,
}

#[derive(Debug)]
enum InitialSidebarPart {
    Workspace {
        workspace_roots: Vec<String>,
        status: SidebarSectionStatus,
        error: Option<String>,
    },
    Chats {
        trajectories: Vec<TrajectoryMeta>,
        pagination: Option<SidebarPagination>,
        status: SidebarSectionStatus,
        error: Option<String>,
    },
    Tasks {
        tasks: Vec<TaskMeta>,
        status: SidebarSectionStatus,
        error: Option<String>,
    },
    Buddy {
        buddy: serde_json::Value,
        status: SidebarSectionStatus,
        error: Option<String>,
    },
}

#[derive(Debug)]
struct InitialSidebarPartMessage {
    generation: u64,
    part: InitialSidebarPart,
}

fn current_initial_sidebar_part(
    message: InitialSidebarPartMessage,
    load_generation: u64,
) -> Option<InitialSidebarPart> {
    if message.generation == load_generation {
        Some(message.part)
    } else {
        None
    }
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

async fn fetch_workspace_roots(gcx: Arc<GlobalContext>) -> Vec<String> {
    let folders = gcx.documents_state.workspace_folders.lock().unwrap();
    folders
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect()
}

async fn fetch_buddy_snapshot(gcx: Arc<GlobalContext>) -> serde_json::Value {
    let buddy_arc = gcx.buddy.clone();
    let locked = buddy_arc.lock().await;
    match locked.as_ref() {
        Some(svc) => serde_json::to_value(&svc.snapshot()).unwrap_or(serde_json::Value::Null),
        None => serde_json::Value::Null,
    }
}

fn make_event(
    seq_counter: &AtomicU64,
    subscription_id: &str,
    event: SidebarEvent,
) -> Option<String> {
    let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
    let envelope = SidebarEventEnvelope {
        protocol_version: SIDEBAR_PROTOCOL_VERSION,
        seq,
        subscription_id: subscription_id.to_string(),
        event,
    };
    serde_json::to_string(&envelope)
        .ok()
        .map(|json| format!("data: {}\n\n", json))
}

fn section_snapshot_event(
    section: SidebarSection,
    status: SidebarSectionStatus,
    snapshot: SidebarSectionSnapshot,
    elapsed_ms: Option<u128>,
    error: Option<String>,
) -> SidebarEvent {
    SidebarEvent::SectionSnapshot {
        section,
        status,
        snapshot,
        elapsed_ms,
        error,
    }
}

impl InitialSidebarPart {
    fn section(&self) -> SidebarSection {
        match self {
            InitialSidebarPart::Workspace { .. } => SidebarSection::Workspace,
            InitialSidebarPart::Chats { .. } => SidebarSection::Chats,
            InitialSidebarPart::Tasks { .. } => SidebarSection::Tasks,
            InitialSidebarPart::Buddy { .. } => SidebarSection::Buddy,
        }
    }

    fn status(&self) -> SidebarSectionStatus {
        match self {
            InitialSidebarPart::Workspace { status, .. }
            | InitialSidebarPart::Chats { status, .. }
            | InitialSidebarPart::Tasks { status, .. }
            | InitialSidebarPart::Buddy { status, .. } => *status,
        }
    }

    fn into_event(self, elapsed_ms: u128) -> SidebarEvent {
        match self {
            InitialSidebarPart::Workspace {
                workspace_roots,
                status,
                error,
            } => section_snapshot_event(
                SidebarSection::Workspace,
                status,
                SidebarSectionSnapshot::Workspace { workspace_roots },
                Some(elapsed_ms),
                error,
            ),
            InitialSidebarPart::Chats {
                trajectories,
                pagination,
                status,
                error,
            } => section_snapshot_event(
                SidebarSection::Chats,
                status,
                SidebarSectionSnapshot::Chats {
                    trajectories,
                    pagination,
                },
                Some(elapsed_ms),
                error,
            ),
            InitialSidebarPart::Tasks {
                tasks,
                status,
                error,
            } => section_snapshot_event(
                SidebarSection::Tasks,
                status,
                SidebarSectionSnapshot::Tasks { tasks },
                Some(elapsed_ms),
                error,
            ),
            InitialSidebarPart::Buddy {
                buddy,
                status,
                error,
            } => section_snapshot_event(
                SidebarSection::Buddy,
                status,
                SidebarSectionSnapshot::Buddy { buddy },
                Some(elapsed_ms),
                error,
            ),
        }
    }
}

async fn load_workspace_part(gcx: Arc<GlobalContext>) -> InitialSidebarPart {
    match timeout(SIDEBAR_BOOTSTRAP_TIMEOUT, fetch_workspace_roots(gcx)).await {
        Ok(workspace_roots) => InitialSidebarPart::Workspace {
            workspace_roots,
            status: SidebarSectionStatus::Ready,
            error: None,
        },
        Err(_) => InitialSidebarPart::Workspace {
            workspace_roots: Vec::new(),
            status: SidebarSectionStatus::Error,
            error: Some("Timed out loading workspace".to_string()),
        },
    }
}

async fn load_chats_part(gcx: Arc<GlobalContext>) -> InitialSidebarPart {
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    match timeout(
        SIDEBAR_BOOTSTRAP_TIMEOUT,
        list_trajectories_page(app, SIDEBAR_CHATS_PAGE_SIZE, None, true),
    )
    .await
    {
        Ok(Ok(page)) => {
            let pagination = SidebarPagination {
                next_cursor: page.next_cursor,
                has_more: page.has_more,
                total_count: page.total_count,
            };
            InitialSidebarPart::Chats {
                trajectories: page.items,
                pagination: Some(pagination),
                status: SidebarSectionStatus::Ready,
                error: None,
            }
        }
        Ok(Err(error)) => InitialSidebarPart::Chats {
            trajectories: Vec::new(),
            pagination: None,
            status: SidebarSectionStatus::Error,
            error: Some(error),
        },
        Err(_) => InitialSidebarPart::Chats {
            trajectories: Vec::new(),
            pagination: None,
            status: SidebarSectionStatus::Error,
            error: Some("Timed out loading chats".to_string()),
        },
    }
}

async fn load_tasks_part(gcx: Arc<GlobalContext>) -> InitialSidebarPart {
    match timeout(
        SIDEBAR_BOOTSTRAP_TIMEOUT,
        list_tasks_with_session_state(gcx),
    )
    .await
    {
        Ok(Ok(tasks)) => InitialSidebarPart::Tasks {
            tasks,
            status: SidebarSectionStatus::Ready,
            error: None,
        },
        Ok(Err(error)) => InitialSidebarPart::Tasks {
            tasks: Vec::new(),
            status: SidebarSectionStatus::Error,
            error: Some(error.to_string()),
        },
        Err(_) => InitialSidebarPart::Tasks {
            tasks: Vec::new(),
            status: SidebarSectionStatus::Error,
            error: Some("Timed out loading tasks".to_string()),
        },
    }
}

async fn load_buddy_part(gcx: Arc<GlobalContext>) -> InitialSidebarPart {
    match timeout(SIDEBAR_BOOTSTRAP_TIMEOUT, fetch_buddy_snapshot(gcx)).await {
        Ok(buddy) => InitialSidebarPart::Buddy {
            buddy,
            status: SidebarSectionStatus::Ready,
            error: None,
        },
        Err(_) => InitialSidebarPart::Buddy {
            buddy: serde_json::Value::Null,
            status: SidebarSectionStatus::Error,
            error: Some("Timed out loading buddy".to_string()),
        },
    }
}

fn load_section_part(
    gcx: Arc<GlobalContext>,
    section: SidebarSection,
) -> impl std::future::Future<Output = InitialSidebarPart> {
    async move {
        match section {
            SidebarSection::Workspace => load_workspace_part(gcx).await,
            SidebarSection::Chats => load_chats_part(gcx).await,
            SidebarSection::Tasks => load_tasks_part(gcx).await,
            SidebarSection::Buddy => load_buddy_part(gcx).await,
        }
    }
}

fn spawn_sidebar_section_retry(
    gcx: Arc<GlobalContext>,
    section: SidebarSection,
    generation: u64,
    tx: mpsc::UnboundedSender<InitialSidebarPartMessage>,
) {
    tokio::spawn(async move {
        tokio::time::sleep(SIDEBAR_RETRY_DELAY).await;
        let part = load_section_part(gcx, section).await;
        let _ = tx.send(InitialSidebarPartMessage { generation, part });
    });
}

fn spawn_initial_sidebar_loads(
    gcx: Arc<GlobalContext>,
    generation: u64,
) -> mpsc::UnboundedReceiver<InitialSidebarPartMessage> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let workspace_part = load_workspace_part(gcx.clone()).await;
        if tx
            .send(InitialSidebarPartMessage {
                generation,
                part: workspace_part,
            })
            .is_err()
        {
            return;
        }

        let mut jobs = JoinSet::new();

        for section in [
            SidebarSection::Chats,
            SidebarSection::Tasks,
            SidebarSection::Buddy,
        ] {
            let gcx = gcx.clone();
            jobs.spawn(async move { load_section_part(gcx, section).await });
        }

        while let Some(result) = jobs.join_next().await {
            match result {
                Ok(part) => {
                    if tx
                        .send(InitialSidebarPartMessage { generation, part })
                        .is_err()
                    {
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

async fn chats_snapshot_event(gcx: Arc<GlobalContext>) -> SidebarEvent {
    load_chats_part(gcx).await.into_event(0)
}

async fn tasks_snapshot_event(gcx: Arc<GlobalContext>) -> SidebarEvent {
    load_tasks_part(gcx).await.into_event(0)
}

async fn buddy_snapshot_event(gcx: Arc<GlobalContext>) -> SidebarEvent {
    load_buddy_part(gcx).await.into_event(0)
}

fn mark_section_settled(
    section: SidebarSection,
    workspace_settled: &mut bool,
    chats_settled: &mut bool,
    tasks_settled: &mut bool,
    buddy_settled: &mut bool,
) {
    match section {
        SidebarSection::Workspace => *workspace_settled = true,
        SidebarSection::Chats => *chats_settled = true,
        SidebarSection::Tasks => *tasks_settled = true,
        SidebarSection::Buddy => *buddy_settled = true,
    }
}

fn section_settled(
    section: SidebarSection,
    workspace_settled: bool,
    chats_settled: bool,
    tasks_settled: bool,
    buddy_settled: bool,
) -> bool {
    match section {
        SidebarSection::Workspace => workspace_settled,
        SidebarSection::Chats => chats_settled,
        SidebarSection::Tasks => tasks_settled,
        SidebarSection::Buddy => buddy_settled,
    }
}

fn buffered_event_ready(
    event: &SidebarEvent,
    workspace_settled: bool,
    chats_settled: bool,
    tasks_settled: bool,
    buddy_settled: bool,
    bootstrap_complete: bool,
) -> bool {
    match event {
        SidebarEvent::SectionSnapshot { section, .. }
        | SidebarEvent::SectionUpdate { section, .. } => section_settled(
            *section,
            workspace_settled,
            chats_settled,
            tasks_settled,
            buddy_settled,
        ),
        SidebarEvent::Notification { .. } => bootstrap_complete,
    }
}

fn try_finish_bootstrap(
    workspace_settled: bool,
    chats_settled: bool,
    tasks_settled: bool,
    buddy_settled: bool,
    bootstrap_complete: &mut bool,
    initial_rx: &mut Option<mpsc::UnboundedReceiver<InitialSidebarPartMessage>>,
    buffered_live_events: &mut VecDeque<SidebarEvent>,
    seq_counter: &AtomicU64,
    subscription_id: &str,
) -> Vec<String> {
    if workspace_settled && chats_settled && tasks_settled && buddy_settled && !*bootstrap_complete
    {
        *bootstrap_complete = true;
        *initial_rx = None;
    }

    let mut emitted = Vec::new();
    let mut pending = VecDeque::new();
    while let Some(event) = buffered_live_events.pop_front() {
        if buffered_event_ready(
            &event,
            workspace_settled,
            chats_settled,
            tasks_settled,
            buddy_settled,
            *bootstrap_complete,
        ) {
            if let Some(event) = make_event(seq_counter, subscription_id, event) {
                emitted.push(event);
            }
        } else {
            pending.push_back(event);
        }
    }
    *buffered_live_events = pending;

    emitted
}

fn push_or_emit_live_event(
    event: SidebarEvent,
    workspace_settled: bool,
    chats_settled: bool,
    tasks_settled: bool,
    buddy_settled: bool,
    bootstrap_complete: bool,
    buffered_live_events: &mut VecDeque<SidebarEvent>,
    seq_counter: &AtomicU64,
    subscription_id: &str,
) -> Option<String> {
    if buffered_event_ready(
        &event,
        workspace_settled,
        chats_settled,
        tasks_settled,
        buddy_settled,
        bootstrap_complete,
    ) {
        make_event(seq_counter, subscription_id, event)
    } else {
        buffered_live_events.push_back(event);
        None
    }
}

pub async fn handle_sidebar_subscribe(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let (trajectory_rx, workspace_changed_rx, task_rx, notification_rx, buddy_rx, seq_counter) = {
        let trajectory_rx = gcx.trajectory_events_tx.as_ref().map(|tx| tx.subscribe());

        let workspace_changed_rx = gcx.workspace_changed_tx.as_ref().map(|tx| tx.subscribe());

        let task_rx = gcx.task_events_tx.as_ref().map(|tx| tx.subscribe());

        let notification_rx = gcx.notification_events_tx.as_ref().map(|tx| tx.subscribe());

        let buddy_rx = gcx.buddy_events_tx.as_ref().map(|tx| tx.subscribe());

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
    let subscription_id = Uuid::new_v4().to_string();
    let stream = async_stream::stream! {
        let mut trajectory_rx = trajectory_rx;
        let mut workspace_changed_rx = workspace_changed_rx;
        let mut task_rx = task_rx;
        let mut notification_rx = notification_rx;
        let mut buddy_rx = buddy_rx;
        let mut load_generation = 0_u64;
        let mut initial_rx = Some(spawn_initial_sidebar_loads(
            gcx_for_stream.clone(),
            load_generation,
        ));
        let (retry_tx, mut retry_rx) = mpsc::unbounded_channel::<InitialSidebarPartMessage>();
        let mut workspace_settled = false;
        let mut chats_settled = false;
        let mut tasks_settled = false;
        let mut buddy_settled = false;
        let mut bootstrap_complete = false;
        let mut buffered_live_events = VecDeque::new();
        let mut initial_started_at = Instant::now();
        let shutdown_flag = gcx_for_stream.shutdown_flag.clone();

        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
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
                        Some(message) => {
                            let Some(part) = current_initial_sidebar_part(message, load_generation) else {
                                continue;
                            };
                            let section = part.section();
                            let status = part.status();
                            let elapsed_ms = initial_started_at.elapsed().as_millis();
                            let event = part.into_event(elapsed_ms);
                            mark_section_settled(
                                section,
                                &mut workspace_settled,
                                &mut chats_settled,
                                &mut tasks_settled,
                                &mut buddy_settled,
                            );
                            tracing::info!("sidebar initial {:?} finished with {:?} in {}ms", section, status, elapsed_ms);
                            if let Some(event) = make_event(&seq_counter, &subscription_id, event) {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                            if status == SidebarSectionStatus::Error {
                                spawn_sidebar_section_retry(
                                    gcx_for_stream.clone(),
                                    section,
                                    load_generation,
                                    retry_tx.clone(),
                                );
                            }
                        }
                        None => {
                            initial_rx = None;
                        }
                    }

                    for event in try_finish_bootstrap(
                        workspace_settled,
                        chats_settled,
                        tasks_settled,
                        buddy_settled,
                        &mut bootstrap_complete,
                        &mut initial_rx,
                        &mut buffered_live_events,
                        &seq_counter,
                        &subscription_id,
                    ) {
                        yield Ok::<_, std::convert::Infallible>(event);
                    }
                }

                retry_part = retry_rx.recv() => {
                    if let Some(message) = retry_part {
                        let Some(part) = current_initial_sidebar_part(message, load_generation) else {
                            continue;
                        };
                        let section = part.section();
                        let status = part.status();
                        let event = part.into_event(0);
                        mark_section_settled(
                            section,
                            &mut workspace_settled,
                            &mut chats_settled,
                            &mut tasks_settled,
                            &mut buddy_settled,
                        );
                        if let Some(event) = make_event(&seq_counter, &subscription_id, event) {
                            yield Ok::<_, std::convert::Infallible>(event);
                        }
                        if status == SidebarSectionStatus::Error {
                            spawn_sidebar_section_retry(
                                gcx_for_stream.clone(),
                                section,
                                load_generation,
                                retry_tx.clone(),
                            );
                        }
                    }

                    for event in try_finish_bootstrap(
                        workspace_settled,
                        chats_settled,
                        tasks_settled,
                        buddy_settled,
                        &mut bootstrap_complete,
                        &mut initial_rx,
                        &mut buffered_live_events,
                        &seq_counter,
                        &subscription_id,
                    ) {
                        yield Ok::<_, std::convert::Infallible>(event);
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
                            if !trajectory_event_is_displayable_chat(&event) {
                                continue;
                            }
                            let event = SidebarEvent::SectionUpdate {
                                section: SidebarSection::Chats,
                                update: SidebarSectionUpdate::Trajectory(event),
                            };
                            if let Some(event) = push_or_emit_live_event(
                                event,
                                workspace_settled,
                                chats_settled,
                                tasks_settled,
                                buddy_settled,
                                bootstrap_complete,
                                &mut buffered_live_events,
                                &seq_counter,
                                &subscription_id,
                            ) {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let event = chats_snapshot_event(gcx_for_stream.clone()).await;
                            if let Some(event) = push_or_emit_live_event(
                                event,
                                workspace_settled,
                                chats_settled,
                                tasks_settled,
                                buddy_settled,
                                bootstrap_complete,
                                &mut buffered_live_events,
                                &seq_counter,
                                &subscription_id,
                            ) {
                                yield Ok::<_, std::convert::Infallible>(event);
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
                            load_generation = load_generation.wrapping_add(1);
                            workspace_settled = false;
                            chats_settled = false;
                            tasks_settled = false;
                            buddy_settled = false;
                            bootstrap_complete = false;
                            buffered_live_events.clear();
                            initial_started_at = Instant::now();
                            initial_rx = Some(spawn_initial_sidebar_loads(
                                gcx_for_stream.clone(),
                                load_generation,
                            ));
                            tracing::info!("sidebar workspace changed; restarting section snapshots");
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
                            let event = SidebarEvent::SectionUpdate {
                                section: SidebarSection::Tasks,
                                update: SidebarSectionUpdate::Task(task_envelope.event),
                            };
                            if let Some(event) = push_or_emit_live_event(
                                event,
                                workspace_settled,
                                chats_settled,
                                tasks_settled,
                                buddy_settled,
                                bootstrap_complete,
                                &mut buffered_live_events,
                                &seq_counter,
                                &subscription_id,
                            ) {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let event = tasks_snapshot_event(gcx_for_stream.clone()).await;
                            if let Some(event) = push_or_emit_live_event(
                                event,
                                workspace_settled,
                                chats_settled,
                                tasks_settled,
                                buddy_settled,
                                bootstrap_complete,
                                &mut buffered_live_events,
                                &seq_counter,
                                &subscription_id,
                            ) {
                                yield Ok::<_, std::convert::Infallible>(event);
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
                        Ok(notification) => {
                            let event = SidebarEvent::Notification { notification };
                            if let Some(event) = push_or_emit_live_event(
                                event,
                                workspace_settled,
                                chats_settled,
                                tasks_settled,
                                buddy_settled,
                                bootstrap_complete,
                                &mut buffered_live_events,
                                &seq_counter,
                                &subscription_id,
                            ) {
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
                            let event = SidebarEvent::SectionUpdate {
                                section: SidebarSection::Buddy,
                                update: SidebarSectionUpdate::Buddy(event),
                            };
                            if let Some(event) = push_or_emit_live_event(
                                event,
                                workspace_settled,
                                chats_settled,
                                tasks_settled,
                                buddy_settled,
                                bootstrap_complete,
                                &mut buffered_live_events,
                                &seq_counter,
                                &subscription_id,
                            ) {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let event = buddy_snapshot_event(gcx_for_stream.clone()).await;
                            if let Some(event) = push_or_emit_live_event(
                                event,
                                workspace_settled,
                                chats_settled,
                                tasks_settled,
                                buddy_settled,
                                bootstrap_complete,
                                &mut buffered_live_events,
                                &seq_counter,
                                &subscription_id,
                            ) {
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
                        tokio::time::sleep(Duration::from_millis(200)).await;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_part(section: SidebarSection, status: SidebarSectionStatus) -> InitialSidebarPart {
        let error = (status == SidebarSectionStatus::Error).then(|| "boom".to_string());
        match section {
            SidebarSection::Workspace => InitialSidebarPart::Workspace {
                workspace_roots: Vec::new(),
                status,
                error,
            },
            SidebarSection::Chats => InitialSidebarPart::Chats {
                trajectories: Vec::new(),
                pagination: None,
                status,
                error,
            },
            SidebarSection::Tasks => InitialSidebarPart::Tasks {
                tasks: Vec::new(),
                status,
                error,
            },
            SidebarSection::Buddy => InitialSidebarPart::Buddy {
                buddy: serde_json::Value::Null,
                status,
                error,
            },
        }
    }

    fn buffered_notification() -> SidebarEvent {
        SidebarEvent::Notification {
            notification: NotificationEvent::TaskDone {
                chat_id: "chat".to_string(),
                tool_call_id: "tool".to_string(),
                summary: "done".to_string(),
                knowledge_path: None,
            },
        }
    }

    #[test]
    fn sidebar_event_envelope_uses_v2_nested_event_shape() {
        let seq = AtomicU64::new(0);
        let raw = make_event(
            &seq,
            "sub-1",
            section_snapshot_event(
                SidebarSection::Tasks,
                SidebarSectionStatus::Ready,
                SidebarSectionSnapshot::Tasks { tasks: Vec::new() },
                Some(7),
                None,
            ),
        )
        .expect("event should serialize");
        let json = raw
            .trim_start_matches("data: ")
            .trim()
            .parse::<serde_json::Value>()
            .expect("valid json");

        assert_eq!(json["protocol_version"], 2);
        assert_eq!(json["seq"], 0);
        assert_eq!(json["subscription_id"], "sub-1");
        assert_eq!(json["event"]["type"], "section_snapshot");
        assert_eq!(json["event"]["section"], "tasks");
        assert_eq!(json["event"]["status"], "ready");
        assert_eq!(
            json["event"]["snapshot"]["tasks"].as_array().unwrap().len(),
            0
        );
        assert!(json.get("category").is_none());
    }

    #[test]
    fn sidebar_error_snapshot_is_terminal_and_carries_empty_data() {
        let event = section_snapshot_event(
            SidebarSection::Chats,
            SidebarSectionStatus::Error,
            SidebarSectionSnapshot::Chats {
                trajectories: Vec::new(),
                pagination: None,
            },
            None,
            Some("boom".to_string()),
        );
        let json = serde_json::to_value(event).expect("event should serialize");

        assert_eq!(json["type"], "section_snapshot");
        assert_eq!(json["section"], "chats");
        assert_eq!(json["status"], "error");
        assert_eq!(json["error"], "boom");
        assert_eq!(
            json["snapshot"]["trajectories"].as_array().unwrap().len(),
            0
        );
    }

    #[test]
    fn sidebar_bootstrap_settles_for_all_ready_error_combinations_and_flushes() {
        let statuses = [SidebarSectionStatus::Ready, SidebarSectionStatus::Error];

        for workspace_status in statuses {
            for chats_status in statuses {
                for tasks_status in statuses {
                    for buddy_status in statuses {
                        let mut workspace_settled = false;
                        let mut chats_settled = false;
                        let mut tasks_settled = false;
                        let mut buddy_settled = false;

                        for part in [
                            empty_part(SidebarSection::Workspace, workspace_status),
                            empty_part(SidebarSection::Chats, chats_status),
                            empty_part(SidebarSection::Tasks, tasks_status),
                            empty_part(SidebarSection::Buddy, buddy_status),
                        ] {
                            assert!(matches!(
                                part.status(),
                                SidebarSectionStatus::Ready | SidebarSectionStatus::Error
                            ));
                            mark_section_settled(
                                part.section(),
                                &mut workspace_settled,
                                &mut chats_settled,
                                &mut tasks_settled,
                                &mut buddy_settled,
                            );
                        }

                        let (_tx, rx) = mpsc::unbounded_channel();
                        let mut initial_rx = Some(rx);
                        let mut bootstrap_complete = false;
                        let mut buffered_live_events = VecDeque::from([buffered_notification()]);
                        let seq = AtomicU64::new(0);
                        let emitted = try_finish_bootstrap(
                            workspace_settled,
                            chats_settled,
                            tasks_settled,
                            buddy_settled,
                            &mut bootstrap_complete,
                            &mut initial_rx,
                            &mut buffered_live_events,
                            &seq,
                            "sub-1",
                        );

                        assert!(bootstrap_complete);
                        assert!(initial_rx.is_none());
                        assert!(buffered_live_events.is_empty());
                        assert_eq!(emitted.len(), 1);
                        assert_eq!(seq.load(Ordering::SeqCst), 1);
                    }
                }
            }
        }
    }

    #[test]
    fn settled_section_live_updates_flush_before_full_bootstrap() {
        let mut initial_rx = Some(mpsc::unbounded_channel().1);
        let mut bootstrap_complete = false;
        let mut buffered_live_events = VecDeque::from([
            SidebarEvent::SectionUpdate {
                section: SidebarSection::Tasks,
                update: SidebarSectionUpdate::Task(TaskEvent::BoardChanged {
                    task_id: "task-1".to_string(),
                    rev: 2,
                    board: Default::default(),
                }),
            },
            buffered_notification(),
        ]);
        let seq = AtomicU64::new(0);

        let emitted = try_finish_bootstrap(
            false,
            false,
            true,
            false,
            &mut bootstrap_complete,
            &mut initial_rx,
            &mut buffered_live_events,
            &seq,
            "sub-1",
        );

        assert!(!bootstrap_complete);
        assert!(initial_rx.is_some());
        assert_eq!(emitted.len(), 1);
        assert_eq!(seq.load(Ordering::SeqCst), 1);
        assert_eq!(buffered_live_events.len(), 1);
        assert!(matches!(
            buffered_live_events.front(),
            Some(SidebarEvent::Notification { .. })
        ));
    }

    #[test]
    fn sidebar_stale_initial_part_generation_is_ignored() {
        let stale = InitialSidebarPartMessage {
            generation: 1,
            part: empty_part(SidebarSection::Workspace, SidebarSectionStatus::Ready),
        };
        let current = InitialSidebarPartMessage {
            generation: 2,
            part: empty_part(SidebarSection::Workspace, SidebarSectionStatus::Error),
        };

        assert!(current_initial_sidebar_part(stale, 2).is_none());
        let part = current_initial_sidebar_part(current, 2).expect("current generation accepted");

        assert_eq!(part.section(), SidebarSection::Workspace);
        assert_eq!(part.status(), SidebarSectionStatus::Error);
    }
}
