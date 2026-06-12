use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;
use std::sync::RwLock as StdRwLock;
use std::time::Duration;
use hyper::StatusCode;
use serde::Deserialize;
use structopt::StructOpt;
use tokio::signal;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tracing::{error, info};

use crate::ast::ast_indexer_thread::AstIndexService;
use crate::agents::registry::BackgroundAgentRegistry;
use crate::app_state::{
    AppActivitySink, AppBuddyEventSink, AppState, AppToolRegistry, BuddyServices, CapsState,
    ChatServices, EngineChatSessionFacade, IntegrationServices, ModelServices, PathServices,
    RuntimeServices, TokenizerState, WorkspaceServices,
};
use crate::buddy::conductor::wake::{ConductorWakeBus, ConductorWakeTargets};
use crate::caps::CodeAssistantCaps;
use crate::caps::providers::get_latest_provider_mtime;
use crate::providers::{ProviderRegistry, load_providers_from_config};
use crate::completion_cache::CompletionCache;
use crate::custom_error::ScratchError;
use crate::exec::ExecRegistry;
use crate::files_in_workspace::DocumentsState;
use crate::integrations::browser_runtime::BrowserRuntime;
use crate::integrations::sessions::IntegrationSession;
use crate::privacy::PrivacySettings;
use crate::background_tasks::BackgroundTasksHolder;
use crate::scheduler::SchedulerConfig;
use crate::voice::SharedVoiceService;
use crate::yaml_configs::customization_registry::RegistryCacheManager;
use crate::knowledge_index::KnowledgeIndex;
use refact_context_api::{HttpClientAccess, PathsAccess, ShutdownAccess};

#[derive(Debug, StructOpt, Clone)]
pub struct CommandLine {
    #[structopt(
        long,
        default_value = "pong",
        help = "A message to return in /v1/ping, useful to verify you're talking to the same process that you've started."
    )]
    pub ping_message: String,
    #[structopt(
        long,
        help = "Send logs to stderr, as opposed to ~/.cache/refact/logs, so it's easier to debug."
    )]
    pub logs_stderr: bool,
    #[structopt(long, default_value = "", help = "Send logs to a file.")]
    pub logs_to_file: String,
    #[structopt(
        long,
        help = "Trust self-signed SSL certificates, when connecting to an inference server."
    )]
    pub insecure: bool,

    #[structopt(
        long,
        short = "p",
        default_value = "0",
        help = "Bind <host>:<port> to listen for HTTP requests, such as /v1/code-completion, /v1/chat, /v1/caps."
    )]
    pub http_port: u16,
    #[structopt(
        long,
        help = "Host/address for the HTTP server. Defaults to 127.0.0.1, or 0.0.0.0 with --inside-container. Use 0.0.0.0 only on trusted networks."
    )]
    pub http_host: Option<String>,
    #[structopt(
        long,
        default_value = "0",
        help = "Bind 127.0.0.1:<port> and act as an LSP server. This is compatible with having an HTTP server at the same time."
    )]
    pub lsp_port: u16,
    #[structopt(
        long,
        default_value = "0",
        help = "Act as an LSP server, use stdin stdout for communication. This is compatible with having an HTTP server at the same time. But it's not compatible with LSP port."
    )]
    pub lsp_stdin_stdout: u16,

    #[structopt(
        long,
        short = "v",
        help = "Makes DEBUG log level visible, instead of the default INFO."
    )]
    pub verbose: bool,

    #[structopt(
        long,
        help = "Use AST, for it to start working, give it a jsonl files list or LSP workspace folders."
    )]
    pub ast: bool,
    // #[structopt(long, help="Use AST light mode, could be useful for large projects and little memory. Less information gets stored.")]
    // pub ast_light_mode: bool,
    #[structopt(
        long,
        default_value = "50000",
        help = "Maximum files for AST index, to avoid OOM on large projects."
    )]
    pub ast_max_files: usize,
    #[structopt(
        long,
        default_value = "",
        help = "Give it a path for AST database to make it permanent, if there is the database already, process starts without parsing all the files (careful). This quick start is helpful for automated solution search."
    )]
    pub ast_permanent: String,
    #[structopt(long, help = "Wait until AST is ready before responding requests.")]
    pub wait_ast: bool,

    #[structopt(
        long,
        help = "Use vector database. Give it LSP workspace folders or a jsonl, it also needs an embedding model."
    )]
    pub vecdb: bool,
    #[structopt(
        long,
        default_value = "15000",
        help = "Maximum files count for VecDB index, to avoid OOM."
    )]
    pub vecdb_max_files: usize,
    #[structopt(long, default_value = "", help = "Set VecDB storage path manually.")]
    pub vecdb_force_path: String,
    #[structopt(long, help = "Wait until VecDB is ready before responding requests.")]
    pub wait_vecdb: bool,

    #[structopt(
        long,
        short = "f",
        default_value = "",
        help = "A path to jsonl file with {\"path\": ...} on each line, files will immediately go to VecDB and AST."
    )]
    pub files_jsonl_path: String,
    #[structopt(
        long,
        short = "w",
        default_value = "",
        help = "Workspace folder to find all the files. An LSP or HTTP request can override this later."
    )]
    pub workspace_folder: String,

    #[structopt(
        long,
        help = "create yaml configs, like customization.yaml, privacy.yaml and exit."
    )]
    pub only_create_yaml_configs: bool,
    #[structopt(
        long,
        help = "Print combined project registry (modes, subagents, toolbox commands, code lens)."
    )]
    pub print_customization: bool,

    #[structopt(long, help = "Enable experimental features, such as new integrations.")]
    pub experimental: bool,

    #[structopt(
        long,
        help = "A way to tell this binary it can run more tools without confirmation."
    )]
    pub inside_container: bool,

    #[structopt(
        long,
        default_value = "",
        help = "Specify the integrations.yaml, this also disables the global integrations.d"
    )]
    pub integrations_yaml: String,

    #[structopt(
        long,
        default_value = "",
        help = "Specify the variables.yaml, disabling the global one"
    )]
    pub variables_yaml: String,
    #[structopt(
        long,
        default_value = "",
        help = "Specify the secrets.yaml, disabling the global one"
    )]
    pub secrets_yaml: String,
    #[structopt(
        long,
        default_value = "",
        help = "Specify the indexing.yaml, replacing the global one"
    )]
    pub indexing_yaml: String,
    #[structopt(
        long,
        default_value = "",
        help = "Specify the privacy.yaml, replacing the global one"
    )]
    pub privacy_yaml: String,
    #[structopt(long, help = "Disable the scheduler runner and cron scheduling tools.")]
    pub no_scheduler: bool,
}

#[derive(Debug, Deserialize)]
struct EngineGlobalConfig {
    #[serde(default)]
    scheduler: SchedulerConfig,
}

impl Default for EngineGlobalConfig {
    fn default() -> Self {
        Self {
            scheduler: SchedulerConfig::default(),
        }
    }
}

pub struct AtCommandsPreviewCache {
    pub cache: HashMap<String, String>,
}

impl AtCommandsPreviewCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }
    pub fn get(&self, key: &str) -> Option<String> {
        let val = self.cache.get(key).cloned();
        // if val.is_some() {
        //     info!("AtCommandsPreviewCache: SOME: key={:?}", key);
        // } else {
        //     info!("AtCommandsPreviewCache: NONE: key={:?}", key);
        // }
        val
    }
    pub fn insert(&mut self, key: String, value: String) {
        self.cache.insert(key.clone(), value);
        // info!("AtCommandsPreviewCache: insert: key={:?}. new_len: {:?}", key, self.cache.len());
    }
    pub fn clear(&mut self) {
        self.cache.clear();
        // info!("AtCommandsPreviewCache: clear; new_len: {:?}", self.cache.len());
    }
}

pub struct GlobalContext {
    pub shutdown_flag: Arc<AtomicBool>,
    pub exec_registry: Arc<ExecRegistry>,
    pub cmdline: CommandLine,
    pub http_client: reqwest::Client,
    pub cache_dir: PathBuf,
    pub config_dir: PathBuf,
    pub caps_state: Arc<ARwLock<CapsState>>,
    pub tokenizer_state: Arc<StdRwLock<TokenizerState>>,
    pub completions_cache: Arc<StdRwLock<CompletionCache>>,
    pub vec_db: Arc<AMutex<Option<Arc<dyn refact_core::vecdb_types::VecdbSearch>>>>,
    pub vec_db_error: Arc<StdMutex<String>>,
    pub ast_service: Arc<StdMutex<Option<Arc<AMutex<AstIndexService>>>>>,
    pub ask_shutdown_sender: Arc<StdMutex<std::sync::mpsc::Sender<String>>>,
    pub documents_state: DocumentsState,
    pub at_commands_preview_cache: Arc<AMutex<AtCommandsPreviewCache>>,
    pub privacy_settings: Arc<StdRwLock<Arc<PrivacySettings>>>,
    pub indexing_everywhere: Arc<crate::files_blocklist::IndexingEverywhere>,
    pub integration_sessions:
        Arc<AMutex<HashMap<String, Arc<AMutex<Box<dyn IntegrationSession>>>>>>,
    pub browser_runtimes: Arc<AMutex<HashMap<String, Arc<AMutex<BrowserRuntime>>>>>,
    pub codelens_cache: Arc<AMutex<crate::http::routers::v1::code_lens::CodeLensCache>>,
    pub init_shadow_repos_background_task_holder: BackgroundTasksHolder,
    pub init_shadow_repos_lock: Arc<AMutex<bool>>,
    pub git_operations_abort_flag: Arc<AtomicBool>,
    pub app_searchable_id: Arc<StdMutex<String>>,
    pub trajectory_events_tx: Option<tokio::sync::broadcast::Sender<crate::chat::TrajectoryEvent>>,
    pub workspace_changed_tx: Option<tokio::sync::broadcast::Sender<()>>,
    pub task_events_tx:
        Option<tokio::sync::broadcast::Sender<crate::tasks::events::TaskEventEnvelope>>,
    pub task_events_seq: Option<Arc<std::sync::atomic::AtomicU64>>,
    pub notification_events_tx: Option<
        tokio::sync::broadcast::Sender<crate::http::routers::v1::sidebar::NotificationEvent>,
    >,
    pub chat_sessions: crate::chat::SessionsMap,
    pub voice_service: SharedVoiceService,
    pub project_registry_cache: Arc<StdRwLock<RegistryCacheManager>>,
    pub providers: Arc<ARwLock<ProviderRegistry>>,
    pub knowledge_index: Arc<AMutex<KnowledgeIndex>>,
    pub llm_stats_sender:
        Arc<StdMutex<Option<tokio::sync::mpsc::Sender<crate::stats::event::LlmCallEvent>>>>,
    pub ext_cache_generation: Arc<std::sync::atomic::AtomicU64>,
    pub buddy: Arc<AMutex<Option<crate::buddy::actor::BuddyService>>>,
    pub buddy_events_tx: Option<tokio::sync::broadcast::Sender<crate::buddy::events::BuddyEvent>>,
    pub user_activity: Arc<AMutex<crate::buddy::user_activity::UserActivityRing>>,
    pub conductor_wake_bus: Arc<AMutex<ConductorWakeBus>>,
    pub conductor_wake_targets: Arc<AMutex<ConductorWakeTargets>>,
    pub agents: Arc<BackgroundAgentRegistry>,
    pub scheduler_config: SchedulerConfig,
}

pub type SharedGlobalContext = Arc<GlobalContext>; // TODO: remove this type alias, confusing

impl GlobalContext {
    pub fn app_state(&self, gcx: SharedGlobalContext) -> AppState {
        let activity_sink = Arc::new(AppActivitySink::new(self.user_activity.clone()));
        let buddy_event_sink = Arc::new(AppBuddyEventSink::new(gcx.clone(), self.buddy.clone()));
        let tool_registry = Arc::new(AppToolRegistry::new(gcx.clone()));
        AppState {
            gcx: gcx.clone(),
            runtime: RuntimeServices {
                shutdown_flag: self.shutdown_flag.clone(),
                cmdline: Arc::new(self.cmdline.clone()),
                http_client: self.http_client.clone(),
                ask_shutdown_sender: self.ask_shutdown_sender.clone(),
                exec_registry: self.exec_registry.clone(),
            },
            paths: PathServices {
                cache_dir: self.cache_dir.clone(),
                config_dir: self.config_dir.clone(),
                app_searchable_id: self.app_searchable_id.lock().unwrap().clone(),
            },
            model: ModelServices {
                caps: self.caps_state.clone(),
                tokenizers: self.tokenizer_state.clone(),
                providers: self.providers.clone(),
                llm_stats_sender: self.llm_stats_sender.lock().unwrap().clone(),
            },
            workspace: WorkspaceServices {
                documents_state: self.documents_state.clone(),
                privacy_settings: self.privacy_settings.read().unwrap().clone(),
                indexing_everywhere: self.indexing_everywhere.clone(),
                completions_cache: self.completions_cache.clone(),
                vec_db: self.vec_db.clone(),
                vec_db_error: self.vec_db_error.clone(),
                ast_service: self.ast_service.lock().unwrap().clone(),
                knowledge_index: self.knowledge_index.clone(),
                at_commands_preview_cache: self.at_commands_preview_cache.clone(),
            },
            chat: ChatServices {
                sessions: self.chat_sessions.clone(),
                facade: Arc::new(EngineChatSessionFacade::new(gcx.clone())),
                trajectory_events_tx: self
                    .trajectory_events_tx
                    .clone()
                    .expect("trajectory event sender is not initialized"),
                workspace_changed_tx: self
                    .workspace_changed_tx
                    .clone()
                    .expect("workspace changed sender is not initialized"),
                task_events_tx: self
                    .task_events_tx
                    .clone()
                    .expect("task event sender is not initialized"),
                task_events_seq: self
                    .task_events_seq
                    .clone()
                    .expect("task event sequence is not initialized"),
                notification_events_tx: self
                    .notification_events_tx
                    .clone()
                    .expect("notification event sender is not initialized"),
                voice_service: self.voice_service.clone(),
            },
            buddy: BuddyServices {
                buddy: self.buddy.clone(),
                buddy_events_tx: self
                    .buddy_events_tx
                    .clone()
                    .expect("buddy event sender is not initialized"),
                user_activity: self.user_activity.clone(),
                conductor_wake_bus: self.conductor_wake_bus.clone(),
                conductor_wake_targets: self.conductor_wake_targets.clone(),
            },
            integrations: IntegrationServices {
                integration_sessions: self.integration_sessions.clone(),
                browser_runtimes: self.browser_runtimes.clone(),
                ext_cache_generation: self.ext_cache_generation.clone(),
                project_registry_cache: self.project_registry_cache.clone(),
                codelens_cache: self.codelens_cache.clone(),
                init_shadow_repos_lock: self.init_shadow_repos_lock.clone(),
                git_operations_abort_flag: self.git_operations_abort_flag.clone(),
            },
            activity_sink,
            buddy_event_sink,
            tool_registry,
            agents: self.agents.clone(),
        }
    }
}

async fn load_engine_global_config(config_dir: &PathBuf, cmdline: &CommandLine) -> SchedulerConfig {
    let path = if cmdline.privacy_yaml.is_empty() {
        config_dir.join("privacy.yaml")
    } else {
        crate::files_correction::canonical_path(cmdline.privacy_yaml.clone())
    };
    let config = match tokio::fs::read_to_string(&path).await {
        Ok(content) => {
            serde_yaml::from_str::<EngineGlobalConfig>(&content).unwrap_or_else(|error| {
                tracing::warn!(
                    "failed to parse scheduler config from {}: {error}",
                    path.display()
                );
                EngineGlobalConfig::default()
            })
        }
        Err(error) => {
            tracing::warn!(
                "failed to read scheduler config from {}: {error}",
                path.display()
            );
            EngineGlobalConfig::default()
        }
    };
    config
        .scheduler
        .with_startup_overrides(cmdline.no_scheduler)
}

impl ShutdownAccess for GlobalContext {
    fn shutdown_flag(&self) -> Arc<AtomicBool> {
        self.shutdown_flag.clone()
    }
}

impl PathsAccess for GlobalContext {
    fn cache_dir(&self) -> PathBuf {
        self.cache_dir.clone()
    }

    fn config_dir(&self) -> PathBuf {
        self.config_dir.clone()
    }

    fn workspace_folders(&self) -> Vec<PathBuf> {
        self.documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .clone()
    }
}

impl HttpClientAccess for GlobalContext {
    fn http_client(&self) -> reqwest::Client {
        self.http_client.clone()
    }
}

const CAPS_RELOAD_BACKOFF: u64 = 60; // seconds
const CAPS_BACKGROUND_RELOAD: u64 = 3600; // seconds

pub async fn migrate_to_config_folder(config_dir: &PathBuf, cache_dir: &PathBuf) -> io::Result<()> {
    let mut entries = tokio::fs::read_dir(cache_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
        let file_type = entry.file_type().await?;
        let is_yaml_cfg =
            file_type.is_file() && path.extension().and_then(|e| e.to_str()) == Some("yaml");
        if is_yaml_cfg {
            let new_path = config_dir.join(&file_name);
            if new_path.exists() {
                tracing::info!(
                    "cannot migrate {:?} to {:?}: destination exists",
                    path,
                    new_path
                );
                continue;
            }
            tokio::fs::rename(&path, &new_path).await?;
            tracing::info!("migrated {:?} to {:?}", path, new_path);
        }
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn get_app_searchable_id(workspace_folders: &[PathBuf]) -> String {
    let mac = pnet_datalink::interfaces()
        .into_iter()
        .find(|iface: &pnet_datalink::NetworkInterface| !iface.is_loopback() && iface.mac.is_some())
        .and_then(|iface| iface.mac)
        .map(|mac| mac.to_string().replace(":", ""))
        .unwrap_or_else(|| "no-mac".to_string());

    let folders = workspace_folders
        .iter()
        .map(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join(";");

    format!("{}-{}", mac, folders)
}

#[cfg(target_os = "windows")]
pub fn get_app_searchable_id(workspace_folders: &[PathBuf]) -> String {
    use winreg::enums::*;
    use winreg::RegKey;
    let machine_guid = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Microsoft\\Cryptography")
        .and_then(|key| key.get_value::<String, _>("MachineGuid"))
        .unwrap_or_else(|_| "no-machine-guid".to_string());
    let folders = workspace_folders
        .iter()
        .map(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join(";");
    format!("{}-{}", machine_guid, folders)
}

pub async fn try_load_caps_quickly_if_not_present(
    gcx: Arc<GlobalContext>,
    max_age_seconds: u64,
) -> Result<Arc<CodeAssistantCaps>, ScratchError> {
    let cmdline = gcx.cmdline.clone();
    let (caps_state, config_dir) = { (gcx.caps_state.clone(), gcx.config_dir.clone()) };
    let caps_reading_lock = caps_state.read().await.reading_lock.clone();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let caps_last_attempted_ts;
    let latest_provider_mtime = get_latest_provider_mtime(&config_dir).await.unwrap_or(0);

    {
        // gcx is not locked, but a specialized async mutex is, up until caps are saved
        let _caps_reading_locked = caps_reading_lock.lock().await;

        let max_age = if max_age_seconds > 0 {
            max_age_seconds
        } else {
            CAPS_BACKGROUND_RELOAD
        };
        {
            let mut caps_state = caps_state.write().await;
            if caps_state.last_attempted_ts + max_age < now
                || latest_provider_mtime >= caps_state.last_attempted_ts
            {
                caps_state.caps = None;
                caps_state.last_attempted_ts = 0;
                caps_last_attempted_ts = 0;
            } else {
                if let Some(caps_arc) = caps_state.caps.clone() {
                    return Ok(caps_arc.clone());
                }
                caps_last_attempted_ts = caps_state.last_attempted_ts;
            }
        }
        if caps_last_attempted_ts + CAPS_RELOAD_BACKOFF > now {
            let caps_state = caps_state.read().await;
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                caps_state.last_error.clone(),
            ));
        }

        let caps_result = crate::caps::load_caps(cmdline, gcx.clone()).await;

        {
            let mut caps_state = caps_state.write().await;
            caps_state.last_attempted_ts = now;
            match caps_result {
                Ok(caps) => {
                    caps_state.caps = Some(caps.clone());
                    caps_state.last_error = "".to_string();
                    Ok(caps)
                }
                Err(e) => {
                    error!("caps fetch failed: {:?}", e);
                    caps_state.last_error = format!("caps fetch failed: {}", e);
                    return Err(ScratchError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        caps_state.last_error.clone(),
                    ));
                }
            }
        }
    }
}

pub async fn look_for_piggyback_fields(
    gcx: Arc<GlobalContext>,
    anything_from_server: &serde_json::Value,
) {
    let caps_state = gcx.caps_state.clone();
    let mut caps_state = caps_state.write().await;
    if let Some(dict) = anything_from_server.as_object() {
        let new_caps_version = dict
            .get("caps_version")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if new_caps_version > 0 {
            if let Some(caps) = caps_state.caps.clone() {
                if caps.caps_version < new_caps_version {
                    info!(
                        "detected biggyback caps version {} is newer than the current version {}",
                        new_caps_version, caps.caps_version
                    );
                    caps_state.caps = None;
                    caps_state.last_attempted_ts = 0;
                }
            }
        }
    }
}

pub async fn block_until_signal(
    ask_shutdown_receiver: std::sync::mpsc::Receiver<String>,
    shutdown_flag: Arc<AtomicBool>,
) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let sigterm = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();

    #[cfg(unix)]
    let sigusr1 = async {
        signal::unix::signal(signal::unix::SignalKind::user_defined1())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let sigusr1 = std::future::pending::<()>();

    let shutdown_flag_clone = shutdown_flag.clone();
    tokio::select! {
        _ = ctrl_c => {
            info!("SIGINT signal received");
            shutdown_flag_clone.store(true, Ordering::SeqCst);
        },
        _ = sigterm => {
            info!("SIGTERM signal received");
            shutdown_flag_clone.store(true, Ordering::SeqCst);
        },
        _ = sigusr1 => {
            info!("SIGUSR1 signal received");
        },
        _ = tokio::task::spawn_blocking(move || {
            let _ = ask_shutdown_receiver.recv();
            shutdown_flag.store(true, Ordering::SeqCst);
        }) => {
            info!("graceful shutdown requested");
        }
    }
}

fn build_shared_http_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .tcp_keepalive(Some(Duration::from_secs(30)))
        .pool_idle_timeout(Some(Duration::from_secs(60)))
        .http2_keep_alive_interval(Some(Duration::from_secs(20)))
        .http2_keep_alive_timeout(Duration::from_secs(10))
        .http2_keep_alive_while_idle(true)
}

pub async fn create_global_context(
    cache_dir: PathBuf,
    config_dir: PathBuf,
) -> (
    Arc<GlobalContext>,
    std::sync::mpsc::Receiver<String>,
    CommandLine,
) {
    let cmdline = CommandLine::from_args();
    let (ask_shutdown_sender, ask_shutdown_receiver) = std::sync::mpsc::channel::<String>();
    let mut http_client_builder = build_shared_http_client_builder();
    if cmdline.insecure {
        http_client_builder = http_client_builder.danger_accept_invalid_certs(true)
    }
    let http_client = http_client_builder.build().unwrap();
    let scheduler_config = load_engine_global_config(&config_dir, &cmdline).await;

    let mut workspace_dirs: Vec<PathBuf> = vec![];
    if !cmdline.workspace_folder.is_empty() {
        let path = crate::files_correction::canonical_path(&cmdline.workspace_folder);
        workspace_dirs = vec![path];
    }
    let user_activity_project_root = workspace_dirs
        .first()
        .cloned()
        .unwrap_or_else(|| cache_dir.clone());
    let user_activity =
        crate::buddy::user_activity::UserActivityRing::load(user_activity_project_root.as_path())
            .await;
    let agents_root = user_activity_project_root
        .join(".refact")
        .join("background_agents");
    let agents = BackgroundAgentRegistry::new(agents_root)
        .await
        .unwrap_or_else(|error| panic!("failed to initialize background agent registry: {error}"));
    let cx = GlobalContext {
        shutdown_flag: Arc::new(AtomicBool::new(false)),
        exec_registry: Arc::new(ExecRegistry::new()),
        cmdline: cmdline.clone(),
        http_client: http_client.clone(),
        cache_dir,
        config_dir: config_dir.clone(),
        caps_state: Arc::new(ARwLock::new(CapsState {
            caps: None,
            reading_lock: Arc::new(AMutex::<bool>::new(false)),
            last_error: String::new(),
            last_attempted_ts: 0,
            models_dev_startup_refresh_attempted: false,
        })),
        tokenizer_state: Arc::new(StdRwLock::new(TokenizerState {
            map: HashMap::new(),
            download_lock: Arc::new(AMutex::<bool>::new(false)),
        })),
        completions_cache: Arc::new(StdRwLock::new(CompletionCache::new())),
        vec_db: Arc::new(AMutex::new(None)),
        vec_db_error: Arc::new(StdMutex::new(String::new())),
        ast_service: Arc::new(StdMutex::new(None)),
        ask_shutdown_sender: Arc::new(StdMutex::new(ask_shutdown_sender)),
        documents_state: DocumentsState::new(workspace_dirs.clone()).await,
        at_commands_preview_cache: Arc::new(AMutex::new(AtCommandsPreviewCache::new())),
        privacy_settings: Arc::new(StdRwLock::new(Arc::new(PrivacySettings::default()))),
        indexing_everywhere: Arc::new(crate::files_blocklist::IndexingEverywhere::default()),
        integration_sessions: Arc::new(AMutex::new(HashMap::new())),
        browser_runtimes: Arc::new(AMutex::new(HashMap::new())),
        codelens_cache: Arc::new(AMutex::new(
            crate::http::routers::v1::code_lens::CodeLensCache::default(),
        )),
        init_shadow_repos_background_task_holder: BackgroundTasksHolder::new(vec![]),
        init_shadow_repos_lock: Arc::new(AMutex::new(false)),
        git_operations_abort_flag: Arc::new(AtomicBool::new(false)),
        app_searchable_id: Arc::new(StdMutex::new(get_app_searchable_id(&workspace_dirs))),
        trajectory_events_tx: Some(tokio::sync::broadcast::channel(1024).0),
        workspace_changed_tx: Some(tokio::sync::broadcast::channel(16).0),
        task_events_tx: Some(tokio::sync::broadcast::channel(1024).0),
        task_events_seq: Some(Arc::new(std::sync::atomic::AtomicU64::new(0))),
        notification_events_tx: Some(tokio::sync::broadcast::channel(256).0),
        chat_sessions: crate::chat::create_sessions_map(),
        voice_service: crate::voice::VoiceService::new(),
        project_registry_cache: Arc::new(StdRwLock::new(RegistryCacheManager::new())),
        providers: Arc::new(ARwLock::new(
            load_providers_from_config(&config_dir, &http_client)
                .await
                .unwrap_or_default(),
        )),
        knowledge_index: Arc::new(AMutex::new(KnowledgeIndex::empty())),
        llm_stats_sender: Arc::new(StdMutex::new(None)),
        ext_cache_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        buddy: Arc::new(AMutex::new(None)),
        buddy_events_tx: Some(tokio::sync::broadcast::channel(256).0),
        user_activity: Arc::new(AMutex::new(user_activity)),
        conductor_wake_bus: Arc::new(AMutex::new(ConductorWakeBus::new())),
        conductor_wake_targets: Arc::new(AMutex::new(ConductorWakeTargets::default())),
        agents,
        scheduler_config,
    };
    let gcx = Arc::new(cx);
    crate::files_in_workspace::watcher_init(gcx.clone()).await;
    let app_state = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    crate::chat::start_session_cleanup_task(app_state);
    crate::chat::start_trajectory_watcher(gcx.clone());
    (gcx, ask_shutdown_receiver, cmdline)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[tokio::test]
    async fn app_state_from_test_gcx_clones() {
        let gcx = make_test_gcx().await;
        let app_state = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let second_app_state = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let cloned = app_state.clone();

        assert_eq!(cloned.paths.app_searchable_id, "test");
        assert!(Arc::ptr_eq(
            &app_state.runtime.shutdown_flag,
            &cloned.runtime.shutdown_flag
        ));
        assert!(Arc::ptr_eq(
            &gcx.exec_registry,
            &app_state.runtime.exec_registry
        ));
        assert!(Arc::ptr_eq(
            &app_state.runtime.exec_registry,
            &second_app_state.runtime.exec_registry
        ));
        assert!(Arc::ptr_eq(
            &app_state.model.caps,
            &second_app_state.model.caps
        ));
        assert!(Arc::ptr_eq(
            &app_state.model.tokenizers,
            &second_app_state.model.tokenizers
        ));
        assert!(Arc::ptr_eq(&app_state.agents, &second_app_state.agents));
        assert!(Arc::ptr_eq(
            &app_state.buddy.conductor_wake_bus,
            &second_app_state.buddy.conductor_wake_bus
        ));
        assert!(Arc::ptr_eq(
            &app_state.buddy.conductor_wake_targets,
            &second_app_state.buddy.conductor_wake_targets
        ));
    }

    #[tokio::test]
    async fn make_test_gcx_initializes_exec_registry() {
        let gcx = make_test_gcx().await;
        let snapshot = gcx
            .exec_registry
            .register(
                crate::exec::ExecProcessMeta::new(
                    crate::exec::ExecMode::Foreground,
                    "true".to_string(),
                ),
                1024,
            )
            .await;

        assert_eq!(gcx.exec_registry.list(Default::default()).await.len(), 1);
        assert_eq!(
            gcx.exec_registry.get(&snapshot.meta.process_id).await,
            Some(snapshot)
        );
    }

    pub async fn make_test_gcx() -> Arc<GlobalContext> {
        let cache_dir = std::env::temp_dir().join(format!("refact-test-{}", uuid::Uuid::new_v4()));
        let config_dir = std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4()));
        make_test_gcx_with_dirs(cache_dir, config_dir).await
    }

    pub async fn make_test_gcx_with_dirs(
        cache_dir: PathBuf,
        config_dir: PathBuf,
    ) -> Arc<GlobalContext> {
        let (ask_shutdown_sender, _) = std::sync::mpsc::channel::<String>();

        let _ = std::fs::create_dir_all(&cache_dir);
        let _ = std::fs::create_dir_all(&config_dir);
        let user_activity = crate::buddy::user_activity::UserActivityRing::load(&cache_dir).await;
        let agents =
            BackgroundAgentRegistry::new(cache_dir.join(".refact").join("background_agents"))
                .await
                .unwrap_or_else(|error| {
                    panic!("failed to initialize test background agent registry: {error}")
                });

        let cmdline = CommandLine {
            ping_message: "pong".to_string(),
            logs_stderr: true,
            logs_to_file: String::new(),
            insecure: true,
            http_port: 0,
            http_host: None,
            lsp_port: 0,
            lsp_stdin_stdout: 0,
            verbose: false,
            ast: false,
            ast_max_files: 0,
            ast_permanent: String::new(),
            wait_ast: false,
            vecdb: false,
            vecdb_max_files: 0,
            vecdb_force_path: String::new(),
            wait_vecdb: false,
            files_jsonl_path: String::new(),
            workspace_folder: String::new(),
            only_create_yaml_configs: false,
            print_customization: false,
            experimental: false,
            inside_container: true,
            integrations_yaml: String::new(),
            variables_yaml: String::new(),
            secrets_yaml: String::new(),
            indexing_yaml: String::new(),
            privacy_yaml: String::new(),
            no_scheduler: false,
        };

        let http_client = build_shared_http_client_builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();

        let cx = GlobalContext {
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            exec_registry: Arc::new(ExecRegistry::new()),
            cmdline,
            http_client,
            cache_dir,
            config_dir,
            caps_state: Arc::new(ARwLock::new(CapsState {
                caps: None,
                reading_lock: Arc::new(AMutex::<bool>::new(false)),
                last_error: String::new(),
                last_attempted_ts: 0,
                models_dev_startup_refresh_attempted: true,
            })),
            tokenizer_state: Arc::new(StdRwLock::new(TokenizerState {
                map: HashMap::new(),
                download_lock: Arc::new(AMutex::<bool>::new(false)),
            })),
            completions_cache: Arc::new(StdRwLock::new(CompletionCache::new())),
            vec_db: Arc::new(AMutex::new(None)),
            vec_db_error: Arc::new(StdMutex::new(String::new())),
            ast_service: Arc::new(StdMutex::new(None)),
            ask_shutdown_sender: Arc::new(StdMutex::new(ask_shutdown_sender)),
            documents_state: DocumentsState::new(vec![]).await,
            at_commands_preview_cache: Arc::new(AMutex::new(AtCommandsPreviewCache::new())),
            privacy_settings: Arc::new(StdRwLock::new(Arc::new(PrivacySettings::default()))),
            indexing_everywhere: Arc::new(crate::files_blocklist::IndexingEverywhere::default()),
            integration_sessions: Arc::new(AMutex::new(HashMap::new())),
            browser_runtimes: Arc::new(AMutex::new(HashMap::new())),
            codelens_cache: Arc::new(AMutex::new(
                crate::http::routers::v1::code_lens::CodeLensCache::default(),
            )),
            init_shadow_repos_background_task_holder: BackgroundTasksHolder::new(vec![]),
            init_shadow_repos_lock: Arc::new(AMutex::new(false)),
            git_operations_abort_flag: Arc::new(AtomicBool::new(false)),
            app_searchable_id: Arc::new(StdMutex::new("test".to_string())),
            trajectory_events_tx: Some(tokio::sync::broadcast::channel(1024).0),
            workspace_changed_tx: Some(tokio::sync::broadcast::channel(16).0),
            task_events_tx: Some(tokio::sync::broadcast::channel(1024).0),
            task_events_seq: Some(Arc::new(std::sync::atomic::AtomicU64::new(0))),
            notification_events_tx: Some(tokio::sync::broadcast::channel(256).0),
            chat_sessions: crate::chat::create_sessions_map(),
            voice_service: crate::voice::VoiceService::new(),
            project_registry_cache: Arc::new(StdRwLock::new(RegistryCacheManager::new())),
            providers: Arc::new(ARwLock::new(ProviderRegistry::default())),
            knowledge_index: Arc::new(AMutex::new(KnowledgeIndex::empty())),
            llm_stats_sender: Arc::new(StdMutex::new(None)),
            ext_cache_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            buddy: Arc::new(AMutex::new(None)),
            buddy_events_tx: Some(tokio::sync::broadcast::channel(256).0),
            user_activity: Arc::new(AMutex::new(user_activity)),
            conductor_wake_bus: Arc::new(AMutex::new(ConductorWakeBus::new())),
            conductor_wake_targets: Arc::new(AMutex::new(ConductorWakeTargets::default())),
            agents,
            scheduler_config: SchedulerConfig::default(),
        };
        Arc::new(cx)
    }
}
