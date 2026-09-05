use std::iter::IntoIterator;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::vec;
use tokio::task::JoinHandle;

const ABORT_TIMEOUT: Duration = Duration::from_secs(10);
const CACHE_MAINTENANCE_START_DELAY: Duration = Duration::from_secs(2 * 60);
const CACHE_MAINTENANCE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

use crate::global_context::GlobalContext;
use crate::knowledge_index::{build_knowledge_index, knowledge_index_watcher_background_task};

async fn wait_for_shutdown(gcx: Arc<GlobalContext>) {
    while !gcx.shutdown_flag.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn cache_maintenance_background_task(
    gcx: Arc<GlobalContext>,
    app: crate::app_state::AppState,
) {
    let mut delay = CACHE_MAINTENANCE_START_DELAY;
    loop {
        if gcx.shutdown_flag.load(Ordering::SeqCst) {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = wait_for_shutdown(gcx.clone()) => return,
        }
        if gcx.shutdown_flag.load(Ordering::SeqCst) {
            return;
        }
        let spill_dir = gcx.cache_dir.join("exec");
        if let Err(error) = refact_exec::spill::sweep_spill_dir(&spill_dir).await {
            tracing::warn!("exec spill maintenance failed: {error}");
        }
        if let Err(error) = crate::cache_maintenance::sweep_logs_dir(&gcx.cache_dir).await {
            tracing::warn!("log cache maintenance failed: {error}");
        }
        let removed =
            crate::integrations::browser_runtime::sweep_registered_browser_profiles(&app).await;
        if removed > 0 {
            tracing::info!("removed {removed} stale browser profiles");
        }
        let trajectory_dirs = crate::chat::trajectories::list_trajectory_dirs(&gcx).await;
        let removed = crate::chat::trajectory_index::sweep_trajectory_index_tmp_files_in_roots(
            &trajectory_dirs,
            crate::chat::trajectory_index::TRAJECTORY_INDEX_TMP_SWEEP_MAX_AGE,
        )
        .await;
        if removed > 0 {
            tracing::info!("removed {removed} orphaned trajectory index tmp files");
        }
        delay = CACHE_MAINTENANCE_INTERVAL;
    }
}

pub struct BackgroundTasksHolder {
    tasks: Vec<JoinHandle<()>>,
}

impl Default for BackgroundTasksHolder {
    fn default() -> Self {
        BackgroundTasksHolder { tasks: vec![] }
    }
}

impl BackgroundTasksHolder {
    pub fn new(tasks: Vec<JoinHandle<()>>) -> Self {
        BackgroundTasksHolder { tasks }
    }

    pub fn push_back(&mut self, task: JoinHandle<()>) {
        self.tasks.push(task);
    }

    pub fn extend<T>(&mut self, tasks: T)
    where
        T: IntoIterator<Item = JoinHandle<()>>,
    {
        self.tasks.extend(tasks);
    }

    pub async fn abort(&mut self) {
        for task in self.tasks.iter_mut() {
            task.abort();
        }
        let join_all = futures::future::join_all(self.tasks.drain(..));
        if tokio::time::timeout(ABORT_TIMEOUT, join_all).await.is_err() {
            tracing::warn!(
                "background_tasks: some tasks did not finish within {:?} after abort, continuing shutdown",
                ABORT_TIMEOUT
            );
        }
    }
}

pub async fn start_full_soak_background_tasks(gcx: Arc<GlobalContext>) -> BackgroundTasksHolder {
    let app_state = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let goal_monitor_app = app_state.clone();
    let background_agent_monitor_app = app_state.clone();
    let cache_maintenance_app = app_state.clone();
    let background_agent_monitor_shutdown = gcx.shutdown_flag.clone();
    let trajectory_index_coordinator = gcx.trajectory_index_coordinator.clone();
    let trajectory_index_shutdown = gcx.shutdown_flag.clone();
    let mut bg = BackgroundTasksHolder::new(vec![
        tokio::spawn(crate::files_in_workspace::files_in_workspace_init_task(
            gcx.clone(),
        )),
        tokio::spawn(crate::codegraph::cg_highlev::codegraph_background_task(
            gcx.clone(),
        )),
        tokio::spawn(
            crate::integrations::sessions::remove_expired_sessions_background_task(gcx.clone()),
        ),
        tokio::spawn(crate::chat::start_agent_monitor(app_state)),
        tokio::spawn(crate::chat::start_goal_monitor(goal_monitor_app)),
        tokio::spawn(
            crate::chat::trajectory_index::trajectory_index_coordinator_background_task(
                trajectory_index_coordinator,
                trajectory_index_shutdown,
            ),
        ),
        tokio::spawn(crate::agents::monitor::run_background_agent_monitor(
            background_agent_monitor_app,
            background_agent_monitor_shutdown,
        )),
        tokio::spawn(cache_maintenance_background_task(
            gcx.clone(),
            cache_maintenance_app,
        )),
        tokio::spawn({
            let gcx = gcx.clone();
            async move {
                let app = crate::app_state::AppState::from_gcx(gcx).await;
                crate::buddy::actor::buddy_background_task(app).await
            }
        }),
    ]);
    bg.push_back(crate::scheduler::runner::spawn(
        crate::scheduler::runner::session_cron_store(),
        gcx,
    ));
    bg
}

pub async fn start_background_tasks(
    gcx: Arc<GlobalContext>,
    _config_dir: &PathBuf,
) -> BackgroundTasksHolder {
    let (stats_tx, stats_rx) = tokio::sync::mpsc::channel(1000);
    {
        *gcx.llm_stats_sender.lock().unwrap() = Some(stats_tx);
    }
    let gcx_for_knowledge_index = gcx.clone();
    let gcx_for_stats = gcx.clone();
    let app_state = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let goal_monitor_app = app_state.clone();
    let background_agent_monitor_app = app_state.clone();
    let cache_maintenance_app = app_state.clone();
    let background_agent_monitor_shutdown = gcx.shutdown_flag.clone();
    let trajectory_index_coordinator = gcx.trajectory_index_coordinator.clone();
    let trajectory_index_shutdown = gcx.shutdown_flag.clone();
    let mut bg = BackgroundTasksHolder::new(vec![
        tokio::spawn(crate::files_in_workspace::files_in_workspace_init_task(
            gcx.clone(),
        )),
        tokio::spawn(crate::vecdb::vdb_highlev::vecdb_background_reload(
            gcx.clone(),
        )),
        tokio::spawn(crate::codegraph::cg_highlev::codegraph_background_task(
            gcx.clone(),
        )),
        tokio::spawn(
            crate::integrations::sessions::remove_expired_sessions_background_task(gcx.clone()),
        ),
        tokio::spawn(crate::git::cleanup::git_shadow_cleanup_background_task(
            gcx.clone(),
        )),
        tokio::spawn(crate::knowledge_graph::knowledge_cleanup_background_task(
            gcx.clone(),
        )),
        tokio::spawn(crate::knowledge_graph::cleanup_inactive_memories_on_startup(gcx.clone())),
        tokio::spawn(cache_maintenance_background_task(
            gcx.clone(),
            cache_maintenance_app,
        )),
        tokio::spawn(crate::trajectory_memos::trajectory_memos_background_task(
            gcx.clone(),
        )),
        crate::chat::notifications::spawn_notification_subscriber(gcx.clone()),
        tokio::spawn(crate::chat::start_agent_monitor(app_state)),
        tokio::spawn(crate::chat::start_goal_monitor(goal_monitor_app)),
        tokio::spawn(
            crate::chat::trajectory_index::trajectory_index_coordinator_background_task(
                trajectory_index_coordinator,
                trajectory_index_shutdown,
            ),
        ),
        tokio::spawn(crate::agents::monitor::run_background_agent_monitor(
            background_agent_monitor_app,
            background_agent_monitor_shutdown,
        )),
        tokio::spawn(
            crate::providers::oauth_refresh::oauth_token_refresh_background_task(gcx.clone()),
        ),
        tokio::spawn(
            crate::integrations::browser_runtime::browser_monitor_background_task(
                crate::app_state::AppState::from_gcx(gcx.clone()).await,
            ),
        ),
        tokio::spawn(crate::stats::writer::stats_writer_task(
            gcx_for_stats,
            stats_rx,
        )),
        tokio::spawn(async move {
            let index = build_knowledge_index(gcx_for_knowledge_index.clone()).await;
            *gcx_for_knowledge_index.knowledge_index.lock().await = index;
            tracing::info!("knowledge_index: built");
            knowledge_index_watcher_background_task(gcx_for_knowledge_index).await;
        }),
        tokio::spawn({
            let gcx = gcx.clone();
            async move {
                let app = crate::app_state::AppState::from_gcx(gcx).await;
                crate::buddy::actor::buddy_background_task(app).await
            }
        }),
    ]);
    bg.extend(crate::scheduler::runner::spawn_from_active_project(gcx.clone()).await);
    if !gcx.cmdline.daemon_endpoint.is_empty() {
        bg.push_back(tokio::spawn(crate::daemon_link::daemon_link_task(
            gcx.clone(),
        )));
    }
    let files_jsonl_path = gcx.clone().cmdline.files_jsonl_path.clone();
    if !files_jsonl_path.is_empty() {
        bg.extend(vec![tokio::spawn(
            crate::files_in_jsonl::reload_if_jsonl_changes_background_task(gcx.clone()),
        )]);
    }
    bg
}
