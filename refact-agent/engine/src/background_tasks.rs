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

pub async fn wait_for_shutdown(gcx: Arc<GlobalContext>) {
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
    gcx: Option<Arc<GlobalContext>>,
    tasks: Vec<JoinHandle<()>>,
    presence: Option<crate::coordination::presence::PresenceService>,
    reconciliation: Option<crate::coordination::reconcile::ReconciliationService>,
}

impl Default for BackgroundTasksHolder {
    fn default() -> Self {
        BackgroundTasksHolder::new(vec![])
    }
}

impl BackgroundTasksHolder {
    pub fn new(tasks: Vec<JoinHandle<()>>) -> Self {
        BackgroundTasksHolder {
            gcx: None,
            tasks,
            presence: None,
            reconciliation: None,
        }
    }

    pub fn with_gcx(gcx: Arc<GlobalContext>, tasks: Vec<JoinHandle<()>>) -> Self {
        BackgroundTasksHolder {
            gcx: Some(gcx),
            tasks,
            presence: None,
            reconciliation: None,
        }
    }

    fn set_presence(&mut self, presence: crate::coordination::presence::PresenceService) {
        self.presence = Some(presence);
    }

    fn set_reconciliation(
        &mut self,
        reconciliation: crate::coordination::reconcile::ReconciliationService,
    ) {
        self.reconciliation = Some(reconciliation);
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
        // Closing spawn-owner admission is a SHUTDOWN action, not an "abort this
        // holder" action: `abort` is also how a holder is recycled while the
        // process keeps running (see `vecdb_background_reload`). Only latch the
        // process-global gate when this engine is genuinely shutting down.
        let close_admission = self
            .gcx
            .as_ref()
            .is_some_and(|gcx| gcx.shutdown_flag.load(std::sync::atomic::Ordering::Relaxed));
        crate::agents::spawn::drain_spawn_owners(close_admission).await;
        if let Some(presence) = self.presence.take() {
            if let Err(error) = presence.shutdown().await {
                tracing::warn!("engine presence shutdown failed: {error}");
            }
        }
        if let Some(reconciliation) = self.reconciliation.take() {
            reconciliation.shutdown().await;
        }
        if let Some(gcx) = &self.gcx {
            crate::integrations::operator_probe::drain_cleanup(gcx).await;
        }
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
        // Aborting callers may have queued rollback finalizers after the first
        // drain; creation admission is already closed when shutting down.
        crate::agents::spawn::drain_spawn_owners(close_admission).await;
        if let Some(gcx) = &self.gcx {
            crate::integrations::operator_probe::drain_cleanup(gcx).await;
        }
    }
}

pub async fn start_full_soak_background_tasks(gcx: Arc<GlobalContext>) -> BackgroundTasksHolder {
    let app_state = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let presence_app = app_state.clone();
    let goal_monitor_app = app_state.clone();
    let background_agent_monitor_app = app_state.clone();
    let cache_maintenance_app = app_state.clone();
    let outbox_app = app_state.clone();
    let outbox_shutdown = gcx.shutdown_flag.clone();
    let provenance_app = app_state.clone();
    let provenance_shutdown = gcx.shutdown_flag.clone();
    let background_agent_monitor_shutdown = gcx.shutdown_flag.clone();
    let trajectory_index_coordinator = gcx.trajectory_index_coordinator.clone();
    let trajectory_index_shutdown = gcx.shutdown_flag.clone();
    let mut bg = BackgroundTasksHolder::with_gcx(
        gcx.clone(),
        vec![
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
            tokio::spawn(
                crate::coordination::outbox::outbox_recovery_background_task(
                    outbox_app,
                    outbox_shutdown,
                ),
            ),
            tokio::spawn(
                crate::coordination::file_provenance::file_provenance_background_task(
                    provenance_app,
                    provenance_shutdown,
                ),
            ),
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
        ],
    );
    if let Some(presence) =
        crate::coordination::presence::start_background_presence(&presence_app).await
    {
        bg.set_presence(presence);
    }
    if let Some(reconciliation) =
        crate::coordination::reconcile::start_background_reconciliation(&presence_app).await
    {
        bg.set_reconciliation(reconciliation);
    }
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
    crate::http::routers::v1::browser_settings::init_from_disk(&gcx.config_dir).await;
    let (stats_tx, stats_rx) = tokio::sync::mpsc::channel(1000);
    {
        *gcx.llm_stats_sender.lock().unwrap() = Some(stats_tx);
    }
    let gcx_for_knowledge_index = gcx.clone();
    let gcx_for_stats = gcx.clone();
    let app_state = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let presence_app = app_state.clone();
    let goal_monitor_app = app_state.clone();
    let background_agent_monitor_app = app_state.clone();
    let cache_maintenance_app = app_state.clone();
    let outbox_app = app_state.clone();
    let outbox_shutdown = gcx.shutdown_flag.clone();
    let provenance_app = app_state.clone();
    let provenance_shutdown = gcx.shutdown_flag.clone();
    let background_agent_monitor_shutdown = gcx.shutdown_flag.clone();
    let trajectory_index_coordinator = gcx.trajectory_index_coordinator.clone();
    let trajectory_index_shutdown = gcx.shutdown_flag.clone();
    let mut bg = BackgroundTasksHolder::with_gcx(
        gcx.clone(),
        vec![
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
                crate::coordination::outbox::outbox_recovery_background_task(
                    outbox_app,
                    outbox_shutdown,
                ),
            ),
            tokio::spawn(
                crate::coordination::file_provenance::file_provenance_background_task(
                    provenance_app,
                    provenance_shutdown,
                ),
            ),
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
        ],
    );
    if let Some(presence) =
        crate::coordination::presence::start_background_presence(&presence_app).await
    {
        bg.set_presence(presence);
    }
    if let Some(reconciliation) =
        crate::coordination::reconcile::start_background_reconciliation(&presence_app).await
    {
        bg.set_reconciliation(reconciliation);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn start_background_tasks_body() -> &'static str {
        include_str!("background_tasks.rs")
            .split_once("pub async fn start_background_tasks(")
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0
    }

    #[test]
    fn browser_settings_are_loaded_before_the_browser_monitor_starts() {
        let body = start_background_tasks_body();
        let init = body
            .find("browser_settings::init_from_disk(&gcx.config_dir)")
            .expect("startup must load browser settings from disk");
        let monitor = body
            .find("browser_monitor_background_task(")
            .expect("startup must spawn the browser monitor");
        assert!(
            init < monitor,
            "browser settings must be published before the browser monitor is spawned"
        );
    }

    #[test]
    fn browser_settings_init_is_awaited_at_startup() {
        assert!(start_background_tasks_body()
            .contains("browser_settings::init_from_disk(&gcx.config_dir).await;"));
    }

    fn wiring_body<'a>(source: &'a str, signature: &str) -> &'a str {
        let start = source
            .find(signature)
            .unwrap_or_else(|| panic!("{signature} is gone from background_tasks.rs"));
        let rest = &source[start + signature.len()..];
        let end = ["\npub ", "\nasync fn ", "\nfn ", "\n#[cfg(test)]"]
            .iter()
            .filter_map(|marker| rest.find(marker))
            .min()
            .unwrap_or(rest.len());
        &rest[..end]
    }

    fn background_agent_monitor_spawns(body: &str) -> usize {
        let compact: String = body
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .flat_map(str::chars)
            .filter(|c| !c.is_whitespace())
            .collect();
        compact
            .match_indices("run_background_agent_monitor(")
            .filter(|(at, _)| {
                compact[..*at]
                    .rsplit_once("tokio::spawn(")
                    .is_some_and(|(_, path)| {
                        path.chars()
                            .all(|c| c.is_alphanumeric() || c == '_' || c == ':')
                    })
            })
            .count()
    }

    #[test]
    fn both_wiring_paths_spawn_the_background_agent_monitor() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/background_tasks.rs"),
        )
        .unwrap();
        for signature in [
            "\npub async fn start_background_tasks(",
            "\npub async fn start_full_soak_background_tasks(",
        ] {
            assert_eq!(
                background_agent_monitor_spawns(wiring_body(&source, signature)),
                1,
                "{signature} must spawn agents::monitor::run_background_agent_monitor exactly once"
            );
        }
    }

    #[test]
    fn both_wiring_paths_own_engine_presence_until_shutdown() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/background_tasks.rs"),
        )
        .unwrap();
        for signature in [
            "\npub async fn start_background_tasks(",
            "\npub async fn start_full_soak_background_tasks(",
        ] {
            let body = wiring_body(&source, signature);
            assert_eq!(
                body.matches("start_background_presence(&presence_app)")
                    .count(),
                1,
                "{signature} must start presence exactly once"
            );
            assert!(
                body.contains("bg.set_presence(presence)"),
                "{signature} must retain the service for joined shutdown"
            );
        }
        let holder = source
            .split_once("pub struct BackgroundTasksHolder")
            .unwrap()
            .1
            .split_once("}\n")
            .unwrap()
            .0;
        assert!(holder.contains("presence: Option<crate::coordination::presence::PresenceService>"));
        assert!(source.contains("presence.shutdown().await"));
    }

    #[test]
    fn both_wiring_paths_own_crash_reconciliation_until_shutdown() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/background_tasks.rs"),
        )
        .unwrap();
        for signature in [
            "\npub async fn start_background_tasks(",
            "\npub async fn start_full_soak_background_tasks(",
        ] {
            let body = wiring_body(&source, signature);
            assert_eq!(
                body.matches("start_background_reconciliation(&presence_app)")
                    .count(),
                1,
                "{signature} must start reconciliation exactly once"
            );
            assert!(body.contains("bg.set_reconciliation(reconciliation)"));
        }
        let holder = source
            .split_once("pub struct BackgroundTasksHolder")
            .unwrap()
            .1
            .split_once("}\n")
            .unwrap()
            .0;
        assert!(holder.contains(
            "reconciliation: Option<crate::coordination::reconcile::ReconciliationService>"
        ));
        assert!(source.contains("reconciliation.shutdown().await"));
    }

    #[test]
    fn presence_service_uses_the_single_historical_engine_instance_id() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/coordination/mod.rs"),
        )
        .unwrap();
        let function = source
            .split_once("pub fn engine_instance_id()")
            .unwrap()
            .1
            .split_once("\n}")
            .unwrap()
            .0;
        assert!(function.contains("hostname::get()"));
        assert!(function.contains("std::process::id()"));

        let provenance = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/coordination/file_provenance.rs"),
        )
        .unwrap();
        let presence = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/coordination/presence.rs"),
        )
        .unwrap();
        assert!(provenance.contains("super::engine_instance_id()"));
        assert!(presence.contains("EngineIdentity::detect(super::engine_instance_id())"));
        assert_eq!(
            source.matches("pub fn engine_instance_id()").count()
                + provenance.matches("fn instance_id()").count()
                + presence.matches("fn instance_id()").count(),
            1
        );
    }

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn abort_cancels_every_owned_task_and_joins_before_returning() {
        let unwound = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let mut holder = BackgroundTasksHolder::default();
        let flag = DropFlag(unwound.clone());
        holder.push_back(tokio::spawn(async move {
            let _flag = flag;
            std::future::pending::<()>().await;
        }));
        let done = finished.clone();
        let completed = tokio::spawn(async move {
            done.store(true, Ordering::SeqCst);
        });
        while !finished.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        holder.extend([completed]);
        holder.push_back(tokio::spawn(std::future::pending::<()>()));
        let started = std::time::Instant::now();
        holder.abort().await;
        assert!(started.elapsed() < ABORT_TIMEOUT);
        assert!(holder.tasks.is_empty());
        assert!(unwound.load(Ordering::SeqCst));
        assert!(finished.load(Ordering::SeqCst));
        holder.abort().await;
        assert!(holder.tasks.is_empty());
    }
}
