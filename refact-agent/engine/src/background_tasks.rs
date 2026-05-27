use std::iter::IntoIterator;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::vec;
use tokio::task::JoinHandle;

const ABORT_TIMEOUT: Duration = Duration::from_secs(10);

use crate::global_context::GlobalContext;
use crate::knowledge_index::build_knowledge_index;

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
    let background_agent_monitor_app = app_state.clone();
    let background_agent_monitor_shutdown = gcx.shutdown_flag.clone();
    let mut bg = BackgroundTasksHolder::new(vec![
        tokio::spawn(crate::files_in_workspace::files_in_workspace_init_task(
            gcx.clone(),
        )),
        tokio::spawn(crate::vecdb::vdb_highlev::vecdb_background_reload(
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
        tokio::spawn(crate::trajectory_memos::trajectory_memos_background_task(
            gcx.clone(),
        )),
        crate::chat::notifications::spawn_notification_subscriber(gcx.clone()),
        tokio::spawn(crate::chat::start_agent_monitor(app_state)),
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
            // Build in-memory knowledge index in background (best-effort).
            let index = build_knowledge_index(gcx_for_knowledge_index.clone()).await;
            *gcx_for_knowledge_index.knowledge_index.lock().await = index;
            tracing::info!("knowledge_index: built");
        }),
        tokio::spawn({
            let gcx = gcx.clone();
            async move {
                let app = crate::app_state::AppState::from_gcx(gcx).await;
                crate::buddy::actor::buddy_background_task(app).await
            }
        }),
    ]);
    let ast = gcx.clone().ast_service.lock().unwrap().clone();
    if let Some(ast_service) = ast {
        bg.extend(
            crate::ast::ast_indexer_thread::ast_indexer_start(ast_service, gcx.clone()).await,
        );
    }
    if let Some(scheduler_task) = crate::scheduler::spawn_if_enabled(
        Arc::new(crate::scheduler::InMemoryCronStore::new()),
        gcx.scheduler_config,
    ) {
        bg.push_back(scheduler_task);
    }
    let files_jsonl_path = gcx.clone().cmdline.files_jsonl_path.clone();
    if !files_jsonl_path.is_empty() {
        bg.extend(vec![tokio::spawn(
            crate::files_in_jsonl::reload_if_jsonl_changes_background_task(gcx.clone()),
        )]);
    }
    bg
}
