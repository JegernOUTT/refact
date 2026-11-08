use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::caps::resolve_chat_model;
use crate::chat::perf_diagnostics::{self, PerfComponent, PerfOutcome};
use crate::custom_error::YamlError;
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};
use crate::integrations::integr_abstract::IntegrationTrait;
use crate::integrations::running_integrations::load_integrations;
use crate::yaml_configs::customization_registry::{
    get_project_registry, should_expose_subagent_as_config_tool,
};

use super::tool_config_subagent::ToolConfigSubagent;
use super::tools_description::{Tool, ToolDesc, ToolGroup, ToolGroupCategory, ToolSourceType};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolGating {
    pub capability: &'static str,
    pub unavailable_reason: &'static str,
    pub enablement: &'static str,
}

/// When MCP tool count exceeds this threshold, lazy loading activates.
/// The full MCP schemas are replaced by two fixed proxy tools:
/// - `mcp_tool_search` — discover MCP tools by regex, returns schema text
/// - `mcp_call`        — execute any MCP tool by name + args JSON
///
/// The tool list is FIXED for the entire session (cache-safe).
const MCP_LAZY_THRESHOLD: usize = 15;

type LoadedIntegrations = indexmap::IndexMap<String, Box<dyn IntegrationTrait + Send + Sync>>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct IntegrationToolsCacheKey {
    config_dir: PathBuf,
    generation: u64,
}

#[derive(Default)]
struct IntegrationToolsCache {
    integrations: tokio::sync::RwLock<HashMap<IntegrationToolsCacheKey, Arc<LoadedIntegrations>>>,
    yaml_errors: tokio::sync::RwLock<HashMap<IntegrationToolsCacheKey, Arc<Vec<YamlError>>>>,
    build_locks: tokio::sync::Mutex<HashMap<IntegrationToolsCacheKey, Arc<tokio::sync::Mutex<()>>>>,
}

impl IntegrationToolsCache {
    async fn acquire_build_lock(
        &self,
        key: &IntegrationToolsCacheKey,
    ) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.build_locks.lock().await;
        locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    async fn get(&self, key: &IntegrationToolsCacheKey) -> Option<Arc<LoadedIntegrations>> {
        self.integrations.read().await.get(key).cloned()
    }

    async fn insert(
        &self,
        key: IntegrationToolsCacheKey,
        integrations: Arc<LoadedIntegrations>,
        yaml_errors: Arc<Vec<YamlError>>,
    ) {
        let mut cached = self.integrations.write().await;
        cached.retain(|existing, _| existing.config_dir != key.config_dir);
        cached.insert(key.clone(), integrations);
        drop(cached);
        let mut errors = self.yaml_errors.write().await;
        errors.retain(|existing, _| existing.config_dir != key.config_dir);
        errors.insert(key, yaml_errors);
    }
}

fn integration_tools_cache() -> &'static IntegrationToolsCache {
    static CACHE: std::sync::OnceLock<IntegrationToolsCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(IntegrationToolsCache::default)
}

#[cfg(test)]
type TestIntegrationFactory =
    Arc<dyn Fn(Arc<GlobalContext>) -> (LoadedIntegrations, Vec<YamlError>) + Send + Sync>;

#[cfg(test)]
static TEST_INTEGRATION_FACTORY: std::sync::OnceLock<
    tokio::sync::Mutex<HashMap<PathBuf, TestIntegrationFactory>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn test_integration_factory(
) -> &'static tokio::sync::Mutex<HashMap<PathBuf, TestIntegrationFactory>> {
    TEST_INTEGRATION_FACTORY.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

async fn load_integrations_cached(gcx: Arc<GlobalContext>) -> Arc<LoadedIntegrations> {
    let key = IntegrationToolsCacheKey {
        config_dir: gcx.config_dir.clone(),
        generation: gcx
            .tool_catalog_generations
            .integrations
            .load(Ordering::Acquire),
    };
    let cache = integration_tools_cache();
    if let Some(integrations) = cache.get(&key).await {
        return integrations;
    }
    let build_lock = cache.acquire_build_lock(&key).await;
    let _build_guard = build_lock.lock().await;
    if let Some(integrations) = cache.get(&key).await {
        return integrations;
    }
    #[cfg(test)]
    if let Some(factory) = test_integration_factory()
        .lock()
        .await
        .get(&gcx.config_dir)
        .cloned()
    {
        let (integrations, yaml_errors) = factory(gcx);
        let integrations = Arc::new(integrations);
        cache
            .insert(key, integrations.clone(), Arc::new(yaml_errors))
            .await;
        return integrations;
    }
    let (integrations, yaml_errors) = load_integrations(gcx, &["**/*".to_string()]).await;
    let integrations = Arc::new(integrations);
    cache
        .insert(key, integrations.clone(), Arc::new(yaml_errors))
        .await;
    integrations
}

/// Result of applying MCP lazy-loading logic on a tool list.
pub struct ToolsForMode {
    /// Tool list to send to the LLM as schemas. Fixed for the session lifetime.
    pub tools: Vec<Box<dyn Tool + Send>>,
    /// True when lazy mode replaced MCP schemas with the two proxy tools.
    pub mcp_lazy_mode: bool,
    /// Total count of all MCP tools (for the hint message).
    pub mcp_total_count: usize,
    /// (name, description) index for all MCP tools.
    ///
    /// Retained in the catalog for callers that need a local MCP index; the chat preamble
    /// deliberately keeps discovery on demand via `mcp_tool_search`.
    pub mcp_tool_index: Vec<(String, String)>,
}

fn is_integration_mcp_tool(t: &Box<dyn Tool + Send>) -> bool {
    let d = t.tool_description();
    d.name.starts_with("mcp") && matches!(d.source.source_type, ToolSourceType::Integration)
}

pub fn apply_mcp_lazy_filter(mut tools: Vec<Box<dyn Tool + Send>>) -> ToolsForMode {
    let mcp_tool_index: Vec<(String, String)> = tools
        .iter()
        .filter(|tool| is_integration_mcp_tool(tool))
        .map(|tool| {
            let description = tool.tool_description();
            (description.name, description.description)
        })
        .collect();
    let mcp_total_count = mcp_tool_index.len();
    let mcp_lazy_mode = mcp_total_count > MCP_LAZY_THRESHOLD;

    if mcp_lazy_mode {
        tools.retain(|t| !is_integration_mcp_tool(t));
        tools.push(Box::new(crate::tools::tool_mcp_search::ToolMcpSearch {}));
        tools.push(Box::new(crate::tools::tool_mcp_call::ToolMcpCall {}));
    }

    ToolsForMode {
        tools,
        mcp_lazy_mode,
        mcp_total_count,
        mcp_tool_index: if mcp_lazy_mode {
            mcp_tool_index
        } else {
            Vec::new()
        },
    }
}

pub fn provider_of_model(model_id: &str) -> &str {
    model_id
        .split_once('/')
        .map_or(model_id, |(provider, _)| provider)
}

pub async fn tool_access_policy(gcx: Arc<GlobalContext>) -> Arc<refact_privacy::PrivacyPolicy> {
    crate::privacy::load_privacy_if_needed(gcx.clone()).await;
    gcx.privacy_policy_load.read().unwrap().policy.clone()
}

pub fn mcp_tool_allowed(
    policy: &refact_privacy::PrivacyPolicy,
    provider: &str,
    desc: &ToolDesc,
) -> bool {
    if !(desc.name.starts_with("mcp")
        && matches!(desc.source.source_type, ToolSourceType::Integration))
    {
        return true;
    }
    if provider.is_empty() {
        return false;
    }
    let server = crate::integrations::mcp::mcp_interactions::server_name_from_config_path(
        &desc.source.config_path,
    );
    policy.tool_access.mcp_allowed(provider, &server)
}

pub async fn mcp_config_allowed_for_model(
    gcx: Arc<GlobalContext>,
    model_id: &str,
    config_path: &str,
) -> bool {
    let policy = tool_access_policy(gcx).await;
    if policy.tool_access.providers.is_empty() {
        return true;
    }
    if model_id.is_empty() {
        return false;
    }
    let server =
        crate::integrations::mcp::mcp_interactions::server_name_from_config_path(config_path);
    policy
        .tool_access
        .mcp_allowed(provider_of_model(model_id), &server)
}

fn tool_gating(tool: &Box<dyn Tool + Send>, availability: &ToolAvailability) -> Option<ToolGating> {
    let dependencies = tool.tool_depends_on();
    if dependencies
        .iter()
        .any(|dependency| dependency == "ast" || dependency == "codegraph")
        && !availability.codegraph_on
    {
        return Some(ToolGating {
            capability: "code graph index",
            unavailable_reason: "which is not enabled on this engine",
            enablement: "enable code graph indexing",
        });
    }
    if dependencies.iter().any(|dependency| dependency == "vecdb") && !availability.vecdb_on {
        return Some(ToolGating {
            capability: "vector database",
            unavailable_reason: "which is not enabled on this engine",
            enablement: "start it with --vecdb",
        });
    }
    if dependencies
        .iter()
        .any(|dependency| dependency == "thinking")
        && !availability.is_there_a_thinking_model
    {
        return Some(ToolGating {
            capability: "thinking model",
            unavailable_reason: "which is not configured on this engine",
            enablement: "configure a thinking-capable model",
        });
    }
    if dependencies
        .iter()
        .any(|dependency| dependency == "knowledge")
        && !availability.allow_knowledge
    {
        return Some(ToolGating {
            capability: "knowledge tools",
            unavailable_reason: "which are not enabled on this engine",
            enablement: "enable knowledge tools",
        });
    }
    if tool.tool_description().experimental && !availability.allow_experimental {
        return Some(ToolGating {
            capability: "experimental tools",
            unavailable_reason: "which are not enabled on this engine",
            enablement: "start it with --experimental",
        });
    }
    None
}

pub(crate) fn catalog_policy_for_tools(
    tools: &[Box<dyn Tool + Send>],
) -> Vec<refact_runtime_api::ToolPolicyInfo> {
    tools
        .iter()
        .map(|tool| {
            let desc = tool.tool_description();
            let config_override = if !desc.source.config_path.is_empty() {
                tool.config().ok().and_then(|config| config.allow_parallel)
            } else {
                None
            };
            refact_runtime_api::ToolPolicyInfo {
                name: desc.name,
                effective_allow_parallel: desc.allow_parallel && config_override.unwrap_or(true),
            }
        })
        .collect()
}

#[cfg(test)]
fn availability_build_counters() -> &'static std::sync::Mutex<HashMap<PathBuf, usize>> {
    static COUNTERS: std::sync::OnceLock<std::sync::Mutex<HashMap<PathBuf, usize>>> =
        std::sync::OnceLock::new();
    COUNTERS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

#[cfg(test)]
fn availability_build_count(config_dir: &PathBuf) -> usize {
    availability_build_counters()
        .lock()
        .unwrap()
        .get(config_dir)
        .copied()
        .unwrap_or(0)
}

#[derive(Clone, Copy)]
struct ToolAvailability {
    codegraph_on: bool,
    vecdb_on: bool,
    is_there_a_thinking_model: bool,
    allow_knowledge: bool,
    allow_experimental: bool,
}

impl ToolAvailability {
    async fn from_gcx(gcx: Arc<GlobalContext>) -> Self {
        #[cfg(test)]
        {
            *availability_build_counters()
                .lock()
                .unwrap()
                .entry(gcx.config_dir.clone())
                .or_insert(0) += 1;
        }

        let span = perf_diagnostics::span(PerfComponent::ToolAvailabilityVecdbWait, None, None);
        let vecdb_on = gcx.vec_db.lock().await.is_some();
        span.finish_tool(PerfOutcome::Success, 1, 1, None);
        let span = perf_diagnostics::span(PerfComponent::ToolAvailabilityCodegraphWait, None, None);
        let codegraph_on = gcx.codegraph.lock().await.is_some();
        span.finish_tool(PerfOutcome::Success, 1, 1, None);

        let is_there_a_thinking_model =
            match try_load_caps_quickly_if_not_present(gcx.clone(), 0).await {
                Ok(caps) => caps
                    .chat_models
                    .get(&caps.defaults.chat_thinking_model)
                    .is_some(),
                Err(_) => false,
            };

        Self {
            codegraph_on,
            vecdb_on,
            is_there_a_thinking_model,
            allow_knowledge: true,
            allow_experimental: gcx.cmdline.experimental,
        }
    }

    fn allows(&self, tool: &Box<dyn Tool + Send>) -> bool {
        tool_gating(tool, self).is_none()
    }
}

impl ToolGroup {
    fn retain_available_tools(
        &mut self,
        availability: &ToolAvailability,
        gated_tools: Option<&mut HashMap<String, ToolGating>>,
    ) {
        let mut gated_tools = gated_tools;
        self.tools.retain(|tool| {
            let Some(gating) = tool_gating(tool, availability) else {
                return true;
            };
            if let Some(gated_tools) = gated_tools.as_deref_mut() {
                gated_tools.insert(tool.tool_description().name, gating);
            }
            false
        });
    }
}

pub(crate) fn builtin_system_tools(config_path: String) -> Vec<Box<dyn Tool + Send>> {
    vec![
        Box::new(crate::tools::tool_shell::ToolShell {
            cfg: crate::tools::tool_shell::SettingsShell {
                output_filter: crate::postprocessing::pp_command_output::OutputFilter::default(),
            },
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_process::ToolProcessStart {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_process::ToolProcessList {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_process::ToolProcessRead {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_process::ToolProcessKill {
            config_path: config_path.clone(),
        }),
        Box::new(
            crate::tools::tool_clean_background_processes::ToolCleanBackgroundProcesses {
                config_path: config_path.clone(),
            },
        ),
        Box::new(crate::tools::tool_process_subscribe::ToolProcessSubscribe {
            config_path: config_path.clone(),
        }),
        Box::new(
            crate::tools::tool_process_write_stdin::ToolProcessWriteStdin {
                config_path: config_path.clone(),
            },
        ),
        Box::new(crate::tools::tool_process::ToolShellServiceAlias {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_wait::ToolWait {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_schedule::ToolSchedule::new(
            config_path.clone(),
        )),
        Box::new(crate::tools::tool_pause::ToolPause {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_pause::ToolResume {
            config_path: config_path.clone(),
        }),
        Box::new(
            crate::tools::tool_add_workspace_folder::ToolAddWorkspaceFolder {
                config_path: config_path.clone(),
            },
        ),
    ]
}

async fn get_builtin_tools(
    gcx: Arc<GlobalContext>,
    availability: &ToolAvailability,
    gated_tools: Option<&mut HashMap<String, ToolGating>>,
) -> Vec<ToolGroup> {
    let config_dir = gcx.config_dir.clone();
    let config_path = config_dir
        .join("builtin_tools.yaml")
        .to_string_lossy()
        .to_string();

    let codebase_search_tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(crate::tools::tool_ast_definition::ToolAstDefinition {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_tree::ToolTree {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_cat::ToolCat {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_regex_search::ToolRegexSearch {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_glob::ToolGlob {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_search::ToolSearch {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_codegraph::ToolCodegraphOverview {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_codegraph::ToolCodeHealth {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_codegraph::ToolGitRisk {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_codegraph::ToolCodeWhy {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_codegraph::ToolCodeDuplication {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_codegraph::ToolDeadCode {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_codegraph::ToolSecurityScan {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_codegraph::ToolPrBlast {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_codegraph::ToolCodeMap {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_design_system::ToolDesignSystem {
            config_path: config_path.clone(),
        }),
    ];

    let codebase_change_tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(
            crate::tools::file_edit::tool_create_textdoc::ToolCreateTextDoc {
                config_path: config_path.clone(),
            },
        ),
        Box::new(
            crate::tools::file_edit::tool_update_textdoc::ToolUpdateTextDoc {
                config_path: config_path.clone(),
            },
        ),
        Box::new(
            crate::tools::file_edit::tool_update_textdoc_by_lines::ToolUpdateTextDocByLines {
                config_path: config_path.clone(),
            },
        ),
        Box::new(
            crate::tools::file_edit::tool_update_textdoc_regex::ToolUpdateTextDocRegex {
                config_path: config_path.clone(),
            },
        ),
        Box::new(
            crate::tools::file_edit::tool_update_textdoc_anchored::ToolUpdateTextDocAnchored {
                config_path: config_path.clone(),
            },
        ),
        Box::new(crate::tools::file_edit::tool_apply_patch::ToolApplyPatch {
            config_path: config_path.clone(),
        }),
        Box::new(
            crate::tools::file_edit::tool_undo_textdoc::ToolUndoTextDoc {
                config_path: config_path.clone(),
            },
        ),
        Box::new(crate::tools::tool_rm::ToolRm {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_mv::ToolMv {
            config_path: config_path.clone(),
        }),
    ];

    let web_tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(crate::tools::tool_web::ToolWeb {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_web_search::ToolWebSearch {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_chrome::ToolChrome {
            config_path: config_path.clone(),
            ..Default::default()
        }),
        Box::new(crate::tools::tool_ui_probe::ToolUiProbe {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_mark_elements::ToolMarkElements {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_contrast_audit::ToolContrastAudit {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_image_region::ToolImageRegion {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_visual_diff::ToolVisualDiff {
            config_path: config_path.clone(),
        }),
    ];

    let system_tools = builtin_system_tools(config_path.clone());

    let deep_analysis_tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(crate::tools::tool_subagent::ToolSubagent {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_tasks::ToolTasksSet {
            config_path: config_path.clone(),
        }),
    ];

    let knowledge_tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(crate::tools::tool_activate_skill::ToolActivateSkill {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_knowledge::ToolGetKnowledge {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_create_knowledge::ToolCreateKnowledge {
            config_path: config_path.clone(),
        }),
        Box::new(
            crate::tools::tool_trajectory_context::ToolTrajectoryContext {
                config_path: config_path.clone(),
            },
        ),
        Box::new(
            crate::tools::tool_search_trajectories::ToolSearchTrajectories {
                config_path: config_path.clone(),
            },
        ),
        Box::new(crate::tools::tool_task_done::ToolTaskDone {
            config_path: config_path.clone(),
        }),
    ];

    let interaction_tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(crate::tools::tool_ask_questions::ToolAskQuestions {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_buddy_say::ToolBuddySay {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_buddy_say::ToolBuddyRenderControls {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_buddy_get_logs::ToolBuddyGetLogs {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_buddy_get_context::ToolBuddyGetContext {
            config_path: config_path.clone(),
        }),
        Box::new(
            crate::tools::tool_buddy_create_issue::ToolBuddyCreateIssue {
                config_path: config_path.clone(),
            },
        ),
        Box::new(crate::tools::tool_buddy_open_view::ToolBuddyOpenView {
            config_path: config_path.clone(),
        }),
        Box::new(
            crate::tools::tool_buddy_open_setup_flow::ToolBuddyOpenSetupFlow {
                config_path: config_path.clone(),
            },
        ),
        Box::new(
            crate::tools::tool_buddy_create_draft::ToolBuddyCreateDraft {
                config_path: config_path.clone(),
            },
        ),
        Box::new(
            crate::tools::tool_buddy_launch_investigation::ToolBuddyLaunchInvestigation {
                config_path: config_path.clone(),
            },
        ),
        Box::new(crate::tools::buddy::surface::ToolBuddyLogActivity {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::surface::ToolBuddySpeak {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::surface::ToolBuddyRuntimeEvent {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::memory::ToolBuddyMemorySearch {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::memory::ToolBuddyMemoryCreate {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::memory::ToolBuddyMemoryArchive {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::memory::ToolBuddyMemoryRetag {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::memory::ToolBuddyMemoryMerge {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::user_prefs::ToolBuddyUserPrefList {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::user_prefs::ToolBuddyUserPrefUpsert {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::user_prefs::ToolBuddyUserPrefRemove {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::refact_engine::ToolRefactEngineClone {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::refact_engine::ToolRefactEngineSearch {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::refact_engine::ToolRefactEngineCat {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::buddy::integrations::ToolBuddyOpenIssue {
            config_path: config_path.clone(),
        }),
    ];

    let chat_management_tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(
            crate::tools::tool_set_trajectory_label::ToolSetTrajectoryLabel {
                config_path: config_path.clone(),
            },
        ),
        Box::new(crate::tools::tool_get_goal::ToolGetGoal::new(
            config_path.clone(),
        )),
        Box::new(crate::tools::tool_set_goal::ToolSetGoal {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_update_goal::ToolUpdateGoal {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_validate_goal::ToolValidateGoal {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_handoff_to_mode::ToolHandoffToMode {
            config_path: config_path.clone(),
        }),
    ];

    let task_tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(crate::tools::tool_task_init::ToolTaskInit::new()),
        Box::new(crate::tools::tool_task_board::ToolTaskBoardGet::new()),
        Box::new(crate::tools::tool_task_board::ToolTaskBoardCreateCard::new()),
        Box::new(crate::tools::tool_task_board::ToolTaskBoardUpdateCard::new()),
        Box::new(crate::tools::tool_task_board::ToolTaskBoardMoveCard::new()),
        Box::new(crate::tools::tool_task_board::ToolTaskBoardDeleteCard::new()),
        Box::new(crate::tools::tool_task_board::ToolTaskReadyCards::new()),
        Box::new(crate::tools::tool_task_board::ToolTaskBoardClaimCard::new()),
        Box::new(crate::tools::tool_task_board::ToolTaskBoardDiscover::new()),
        Box::new(crate::tools::tool_task_board_authority::ToolTaskBoardAuthority::new()),
        Box::new(crate::tools::tool_task_batch::ToolBoardCreateBatch::new()),
        Box::new(crate::tools::tool_task_agent::ToolTaskAgentUpdate::new()),
        Box::new(crate::tools::tool_task_agent::ToolTaskAgentComplete::new()),
        Box::new(crate::tools::tool_task_agent::ToolTaskAgentFail::new()),
        Box::new(crate::tools::tool_task_agent::ToolTaskAssignAgent::new()),
        Box::new(crate::tools::tool_task_spawn_agent::ToolTaskSpawnAgent::new()),
        Box::new(crate::tools::tool_spawn_ab::ToolSpawnAb::new()),
        Box::new(crate::tools::tool_spawn_ab::ToolPickAbWinner::new()),
        Box::new(crate::tools::tool_task_batch::ToolSpawnAgentsBatch::new()),
        Box::new(crate::tools::tool_task_check_agents::ToolTaskCheckAgents::new()),
        Box::new(crate::tools::tool_task_overview::ToolTaskOverview::new()),
        Box::new(crate::tools::tool_agent_diff::ToolAgentDiff::new()),
        Box::new(crate::tools::tool_agent_pulse::ToolAgentPulse::new()),
        Box::new(crate::tools::tool_agent_chat_summary::ToolAgentChatSummary::new()),
        Box::new(crate::tools::tool_agent_steer::ToolAgentSteer::new()),
        Box::new(crate::tools::tool_agent_planner_qna::ToolAgentAskPlanner::new()),
        Box::new(crate::tools::tool_agent_planner_qna::ToolPlannerReply::new()),
        Box::new(crate::tools::tool_agent_planner_qna::ToolTaskQuestionsList::new()),
        Box::new(crate::tools::tool_card_comment::ToolCardCommentAdd::new()),
        Box::new(crate::tools::tool_card_comment::ToolCardCommentList::new()),
        Box::new(crate::tools::tool_task_broadcast::ToolTaskBroadcast::new()),
        Box::new(crate::tools::tool_agent_lifecycle::ToolCancelAgent::new()),
        Box::new(crate::tools::tool_task_agent_finish::ToolTaskAgentFinish::new()),
        Box::new(crate::tools::tool_task_mark_card::ToolTaskMarkCardDone::new()),
        Box::new(crate::tools::tool_task_mark_card::ToolTaskMarkCardFailed::new()),
        Box::new(crate::tools::tool_task_mark_card::ToolTaskFinalizeCard::new()),
        Box::new(crate::tools::tool_task_batch::ToolMarkDoneBatch::new()),
        Box::new(crate::tools::tool_task_batch::ToolMarkFailedBatch::new()),
        Box::new(crate::tools::tool_task_merge_agent::ToolTaskMergeAgent::new()),
        Box::new(crate::tools::tool_task_batch::ToolMergeReadyInOrder::new()),
        Box::new(crate::tools::tool_task_restart_agent::ToolTaskRestartAgent::new()),
        Box::new(crate::tools::tool_task_verify_card::ToolTaskVerifyCard::new()),
        Box::new(crate::tools::tool_task_documents::ToolDocList::new()),
        Box::new(crate::tools::tool_task_documents::ToolDocGet::new()),
        Box::new(crate::tools::tool_task_documents::ToolDocCreate::new()),
        Box::new(crate::tools::tool_task_documents::ToolDocUpdate::new()),
        Box::new(crate::tools::tool_task_documents::ToolDocAppend::new()),
        Box::new(crate::tools::tool_task_documents::ToolDocDelete::new()),
        Box::new(crate::tools::tool_task_documents::ToolDocPin::new()),
        Box::new(crate::tools::tool_task_documents::ToolDocHistory::new()),
        Box::new(crate::tools::tool_task_memory::ToolTaskMemorySave::new()),
        Box::new(crate::tools::tool_task_memory::ToolTaskMemoriesGet::new()),
        Box::new(crate::tools::tool_task_memory::ToolTaskMemorySearch::new()),
        Box::new(crate::tools::tool_task_memory::ToolTaskMemoryPin::new()),
        Box::new(crate::tools::tool_task_memory::ToolTaskMemoryArchive::new()),
        Box::new(crate::tools::tool_task_memory::ToolTaskMemoryUnarchive::new()),
        Box::new(crate::tools::tool_task_memory::ToolTaskMemoryInbox::new()),
        Box::new(crate::tools::tool_task_memory::ToolTaskMemoryTriageDone::new()),
    ];

    let background_agent_tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(crate::tools::tool_agent_approval::ToolAgentApproval {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_work_overview::ToolWorkOverview {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_organization::ToolOrganization {
            name: "organization_list".into(),
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_organization::ToolOrganization {
            name: "organization_candidates".into(),
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_organization::ToolOrganization {
            name: "organization_apply".into(),
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_organization::ToolOrganization {
            name: "organization_status".into(),
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_work_inspect::ToolWorkInspect {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_metrics::ToolMetrics {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_causal_trace::ToolCausalTrace {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_disagreement::ToolDisagreementFile {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_disagreement::ToolDisagreementList {
            config_path: config_path.clone(),
        }),
        Box::new(
            crate::tools::tool_disagreement::ToolDisagreementAdjudicate {
                config_path: config_path.clone(),
            },
        ),
        Box::new(crate::tools::tool_fact_ledger::ToolClaimRecord {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_fact_ledger::ToolClaimList {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_agent_interact::ToolAgentsOverview {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_agent_interact::ToolAgentMessage {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_agent_interact::ToolAgentPeerGrant {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_team::ToolTeamSeat {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_team::ToolTeamRoleGrant {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_code_mode::ToolCodeMode {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_agent_interact::ToolProgressReport {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_agent_interact::ToolAgentMailbox {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_background_agents::ToolAgentList {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_background_agents::ToolAgentStatus {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_background_agents::ToolAgentResult {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_background_agents::ToolAgentLifecycle {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_background_agents::ToolAgentCancel {
            config_path: config_path.clone(),
        }),
    ];

    let worktree_tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(crate::tools::tool_worktree_create::ToolWorktreeCreate),
        Box::new(crate::tools::tool_worktree_merge::ToolWorktreeMerge::new()),
        Box::new(crate::tools::tool_worktree_sync::ToolWorktreeSync),
    ];

    let mut tool_groups = vec![
        ToolGroup {
            name: "Codebase Search".to_string(),
            description: "Codebase search tools".to_string(),
            category: ToolGroupCategory::Builtin,
            tools: codebase_search_tools,
        },
        ToolGroup {
            name: "Codebase Change".to_string(),
            description: "Codebase modification tools".to_string(),
            category: ToolGroupCategory::Builtin,
            tools: codebase_change_tools,
        },
        ToolGroup {
            name: "Web".to_string(),
            description: "Web tools".to_string(),
            category: ToolGroupCategory::Builtin,
            tools: web_tools,
        },
        ToolGroup {
            name: "System".to_string(),
            description: "System tools".to_string(),
            category: ToolGroupCategory::Builtin,
            tools: system_tools,
        },
        ToolGroup {
            name: "Strategic Planning".to_string(),
            description: "Strategic planning tools".to_string(),
            category: ToolGroupCategory::Builtin,
            tools: deep_analysis_tools,
        },
        ToolGroup {
            name: "Knowledge".to_string(),
            description: "Knowledge tools".to_string(),
            category: ToolGroupCategory::Builtin,
            tools: knowledge_tools,
        },
        ToolGroup {
            name: "Interaction".to_string(),
            description: "User interaction tools".to_string(),
            category: ToolGroupCategory::Builtin,
            tools: interaction_tools,
        },
        ToolGroup {
            name: "Chat Management".to_string(),
            description: "Chat compression and handoff tools".to_string(),
            category: ToolGroupCategory::Builtin,
            tools: chat_management_tools,
        },
        ToolGroup {
            name: "Task Management".to_string(),
            description: "Task workspace and kanban board tools".to_string(),
            category: ToolGroupCategory::Builtin,
            tools: task_tools,
        },
        ToolGroup {
            name: "Background Agents".to_string(),
            description: "Inspect and control background agents spawned by chats".to_string(),
            category: ToolGroupCategory::Builtin,
            tools: background_agent_tools,
        },
        ToolGroup {
            name: "Worktrees".to_string(),
            description: "Worktree lifecycle tools".to_string(),
            category: ToolGroupCategory::Builtin,
            tools: worktree_tools,
        },
    ];

    let mut gated_tools = gated_tools;
    for tool_group in tool_groups.iter_mut() {
        tool_group.retain_available_tools(availability, gated_tools.as_deref_mut());
    }

    tool_groups
}

pub async fn get_integration_tools(gcx: Arc<GlobalContext>) -> Vec<ToolGroup> {
    let availability = ToolAvailability::from_gcx(gcx.clone()).await;
    build_integration_tool_groups(gcx, &availability, None).await
}

async fn build_integration_tool_groups(
    gcx: Arc<GlobalContext>,
    availability: &ToolAvailability,
    gated_tools: Option<&mut HashMap<String, ToolGating>>,
) -> Vec<ToolGroup> {
    let mut integrations_group = ToolGroup {
        name: "Integrations".to_string(),
        description: "Integration tools".to_string(),
        category: ToolGroupCategory::Integration,
        tools: vec![],
    };

    let mut mcp_groups = HashMap::new();

    let span = perf_diagnostics::span(PerfComponent::ToolIntegrationToolsBuild, None, None);
    let integrations_map = load_integrations_cached(gcx.clone()).await;
    for (name, integr) in integrations_map.iter() {
        for tool in integr.integr_tools(name).await {
            let tool_desc = tool.tool_description();
            if tool_desc.name.starts_with("mcp") {
                let mcp_server_name = std::path::Path::new(&tool_desc.source.config_path)
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or("unknown");

                if !mcp_groups.contains_key(mcp_server_name) {
                    mcp_groups.insert(
                        mcp_server_name.to_string(),
                        ToolGroup {
                            name: format!("MCP {}", mcp_server_name),
                            description: format!("MCP tools for {}", mcp_server_name),
                            category: ToolGroupCategory::MCP,
                            tools: vec![],
                        },
                    );
                }
                mcp_groups
                    .entry(mcp_server_name.to_string())
                    .and_modify(|group| group.tools.push(tool));
            } else {
                integrations_group.tools.push(tool);
            }
        }
    }

    let mut sorted_mcp: Vec<(String, ToolGroup)> = mcp_groups.into_iter().collect();
    sorted_mcp.sort_by(|(a, _), (b, _)| a.cmp(b));

    let mut tool_groups = vec![integrations_group];
    tool_groups.extend(sorted_mcp.into_iter().map(|(_, group)| group));

    let mut gated_tools = gated_tools;
    for tool_group in tool_groups.iter_mut() {
        tool_group.retain_available_tools(availability, gated_tools.as_deref_mut());
    }

    span.finish_tool(PerfOutcome::Success, 1, tool_groups.len() as u64, None);

    tool_groups
}

async fn get_config_subagent_tools(gcx: Arc<GlobalContext>) -> ToolGroup {
    let mut subagent_tools: Vec<Box<dyn Tool + Send>> = vec![];

    if let Some(registry) = get_project_registry(gcx.clone()).await {
        let mut subagents: Vec<(String, _)> = registry.subagents.into_iter().collect();
        subagents.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (_, subagent_config) in subagents {
            if should_expose_subagent_as_config_tool(&subagent_config) {
                subagent_tools.push(Box::new(ToolConfigSubagent::new(subagent_config)));
            }
        }
    }

    ToolGroup {
        name: "Config Subagents".to_string(),
        description: "Subagent tools from project config".to_string(),
        category: ToolGroupCategory::ConfigSubagent,
        tools: subagent_tools,
    }
}

enum McpToolQuery<'a> {
    ByName(&'a str),
    All,
}

async fn collect_mcp_integration_tools(
    gcx: Arc<GlobalContext>,
    query: McpToolQuery<'_>,
) -> Vec<Box<dyn Tool + Send>> {
    let span = perf_diagnostics::span(PerfComponent::ToolIntegrationToolsBuild, None, None);
    let integrations_map = load_integrations_cached(gcx.clone()).await;
    let availability = ToolAvailability::from_gcx(gcx.clone()).await;

    let mut integr_names: Vec<&String> = integrations_map.keys().collect();
    integr_names.sort();

    let mut collected: Vec<(String, Box<dyn Tool + Send>)> = vec![];
    'outer: for integr_name in integr_names {
        let integr = match integrations_map.get(integr_name) {
            Some(integr) => integr,
            None => continue,
        };
        for tool in integr.integr_tools(integr_name).await {
            let tool_desc = tool.tool_description();
            if !tool_desc.name.starts_with("mcp") || !availability.allows(&tool) {
                continue;
            }
            let server = std::path::Path::new(&tool_desc.source.config_path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("unknown")
                .to_string();
            match query {
                McpToolQuery::ByName(requested) => {
                    if crate::tools::tool_mcp_call::mcp_tool_name_matches(
                        &tool_desc.name,
                        requested,
                    ) {
                        collected.push((server, tool));
                        break 'outer;
                    }
                }
                McpToolQuery::All => collected.push((server, tool)),
            }
        }
    }

    collected.sort_by(|(a, _), (b, _)| a.cmp(b));
    let tools: Vec<Box<dyn Tool + Send>> = collected.into_iter().map(|(_, tool)| tool).collect();
    span.finish_tool(PerfOutcome::Success, 1, tools.len() as u64, None);
    tools
}

/// Resolve a single MCP tool by the name the model used, stopping at the first integration
/// that provides it instead of materialising every MCP tool box.
pub async fn get_integration_mcp_tool_by_name(
    gcx: Arc<GlobalContext>,
    tool_name: &str,
) -> Option<Box<dyn Tool + Send>> {
    collect_mcp_integration_tools(gcx, McpToolQuery::ByName(tool_name))
        .await
        .pop()
}

/// Every available MCP tool as a flat list, ordered by server the same way
/// `get_integration_tools` groups them.
pub async fn get_integration_mcp_tools(gcx: Arc<GlobalContext>) -> Vec<Box<dyn Tool + Send>> {
    collect_mcp_integration_tools(gcx, McpToolQuery::All).await
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::collections::HashSet;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::tools::tools_description::{ToolSource, ToolSourceType};
    use crate::yaml_configs::customization_registry::RegistryCache;
    use crate::yaml_configs::customization_types::{
        ProjectRegistry, SubagentConfig, SubagentMessages, SubagentPrompts, SubchatConfig,
    };

    use super::*;

    fn desc_with_source(name: &str, source: ToolSource) -> ToolDesc {
        ToolDesc {
            name: name.to_string(),
            experimental: false,
            allow_parallel: false,
            description: String::new(),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
            output_schema: None,
            annotations: None,
            display_name: name.to_string(),
            source,
        }
    }

    fn mcp_desc(config_path: &str, name: &str) -> ToolDesc {
        desc_with_source(
            name,
            ToolSource {
                source_type: ToolSourceType::Integration,
                config_path: config_path.to_string(),
            },
        )
    }

    fn builtin_desc(name: &str) -> ToolDesc {
        desc_with_source(
            name,
            ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
        )
    }

    fn embedded_contract_modes() -> Vec<crate::yaml_configs::customization_types::ModeConfig> {
        let mut modes = crate::yaml_configs::project_configs_bootstrap::embedded_defaults("modes")
            .into_iter()
            .map(|(filename, raw)| {
                let mode: crate::yaml_configs::customization_types::ModeConfig =
                    serde_yaml::from_str(&raw)
                        .unwrap_or_else(|error| panic!("{filename}: {error}"));
                assert_eq!(filename, format!("{}.yaml", mode.id));
                mode
            })
            .collect::<Vec<_>>();
        modes.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(
            modes.len(),
            24,
            "review contracts when shipped modes change"
        );
        assert_eq!(
            modes
                .iter()
                .map(|mode| &mode.id)
                .collect::<HashSet<_>>()
                .len(),
            24
        );
        modes
    }

    fn contract_availability() -> ToolAvailability {
        // A registration ceiling, not a claim about any running engine's capabilities.
        ToolAvailability {
            codegraph_on: true,
            vecdb_on: true,
            is_there_a_thinking_model: true,
            allow_knowledge: true,
            allow_experimental: true,
        }
    }

    async fn contract_described_tools(
        gcx: Arc<GlobalContext>,
        availability: &ToolAvailability,
    ) -> Vec<DescribedTool> {
        get_builtin_tools(gcx, availability, None)
            .await
            .into_iter()
            .flat_map(|group| {
                let category = group.category;
                group.tools.into_iter().map(move |tool| {
                    let desc = tool.tool_description();
                    (category, desc, tool)
                })
            })
            .collect()
    }

    #[tokio::test]
    async fn all_embedded_modes_reference_unique_registered_builtin_descriptors() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let tools = contract_described_tools(gcx, &contract_availability()).await;
        let registered = tools
            .iter()
            .map(|(_, desc, _)| desc.name.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(
            registered.len(),
            tools.len(),
            "duplicate builtin registration"
        );
        // No optional-integration exemption: every current shipped tools entry is builtin.
        for mode in embedded_contract_modes() {
            let mut seen = HashSet::new();
            for name in &mode.tools {
                assert!(seen.insert(name), "{}: duplicate tool {name}", mode.id);
                assert!(
                    registered.contains(name.as_str()),
                    "{}: unregistered tool {name}",
                    mode.id
                );
            }
        }
    }

    #[tokio::test]
    async fn all_embedded_modes_effective_filter_respects_capabilities_order_and_categories() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ceiling = contract_described_tools(gcx.clone(), &contract_availability()).await;
        for mode in embedded_contract_modes() {
            for codegraph_on in [false, true] {
                for vecdb_on in [false, true] {
                    for native_web in [false, true] {
                        let availability = ToolAvailability {
                            codegraph_on,
                            vecdb_on,
                            ..contract_availability()
                        };
                        // Expected entries are derived from actual registered dependencies,
                        // not a second catalog copied from implementation module names.
                        let expected = mode
                            .tools
                            .iter()
                            .filter(|name| {
                                ceiling.iter().any(|(_, desc, tool)| {
                                    &desc.name == *name
                                        && availability.allows(tool)
                                        && tool.config().unwrap_or_default().enabled
                                        && !(native_web && desc.name == "web_search")
                                })
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        let mut described =
                            contract_described_tools(gcx.clone(), &availability).await;
                        for (category, name) in [
                            (ToolGroupCategory::Integration, "contract_integration_probe"),
                            (ToolGroupCategory::MCP, "mcp_contract_probe"),
                            (
                                ToolGroupCategory::ConfigSubagent,
                                "contract_config_subagent_probe",
                            ),
                        ] {
                            let desc = builtin_desc(name);
                            described.push((category, desc.clone(), Box::new(TestTool(desc))));
                        }
                        let actual = selected_names(select_and_order_mode_tools(
                            described,
                            &mode.tools,
                            mode.allow_integrations,
                            mode.allow_mcp,
                            mode.allow_subagents,
                            native_web,
                            None,
                        ));
                        let mut expected = expected;
                        let mut extras = Vec::new();
                        for (allowed, name) in [
                            (mode.allow_integrations, "contract_integration_probe"),
                            (mode.allow_mcp, "mcp_contract_probe"),
                            (mode.allow_subagents, "contract_config_subagent_probe"),
                        ] {
                            if allowed {
                                extras.push(name.to_string());
                            }
                        }
                        extras.sort();
                        expected.extend(extras);
                        assert_eq!(actual, expected,
                            "{}: codegraph={codegraph_on}, vecdb={vecdb_on}, native_web={native_web}", mode.id);
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn builtin_availability_gates_report_real_descriptor_dependencies() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ceiling = contract_described_tools(gcx.clone(), &contract_availability()).await;
        for availability in [
            ToolAvailability {
                codegraph_on: false,
                ..contract_availability()
            },
            ToolAvailability {
                vecdb_on: false,
                ..contract_availability()
            },
            ToolAvailability {
                is_there_a_thinking_model: false,
                ..contract_availability()
            },
            ToolAvailability {
                allow_knowledge: false,
                ..contract_availability()
            },
            ToolAvailability {
                allow_experimental: false,
                ..contract_availability()
            },
        ] {
            let mut gated = HashMap::new();
            let actual = get_builtin_tools(gcx.clone(), &availability, Some(&mut gated))
                .await
                .into_iter()
                .flat_map(|group| group.tools)
                .map(|tool| tool.tool_description().name)
                .collect::<HashSet<_>>();
            // Anchor the two indexed tool families independently of tool_gating,
            // so dropping their dependency metadata cannot make this a tautology.
            if !availability.codegraph_on {
                for name in ["codegraph_overview", "search_symbol_definition"] {
                    assert!(!actual.contains(name), "codegraph-off still exposes {name}");
                    assert_eq!(
                        gated.get(name).map(|gate| gate.capability),
                        Some("code graph index")
                    );
                }
            }
            if !availability.vecdb_on {
                assert!(!actual.contains("search_semantic"));
                assert_eq!(
                    gated.get("search_semantic").map(|gate| gate.capability),
                    Some("vector database")
                );
            }
            for (_, desc, tool) in &ceiling {
                let reason = tool_gating(tool, &availability);
                assert_eq!(
                    actual.contains(&desc.name),
                    reason.is_none(),
                    "{}",
                    desc.name
                );
                assert_eq!(gated.get(&desc.name), reason.as_ref(), "{}", desc.name);
            }
        }
    }

    #[tokio::test]
    async fn embedded_mode_workflows_have_effective_native_tools_and_preserve_boundaries() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        for mode in embedded_contract_modes() {
            let names = selected_names(select_and_order_mode_tools(
                contract_described_tools(gcx.clone(), &contract_availability()).await,
                &mode.tools,
                mode.allow_integrations,
                mode.allow_mcp,
                mode.allow_subagents,
                false,
                None,
            ))
            .into_iter()
            .collect::<HashSet<_>>();
            let require = |required: &[&str]| {
                for name in required {
                    assert!(names.contains(*name), "{} workflow missing {name}", mode.id);
                }
            };
            if names.contains("worktree_merge") {
                require(&["worktree_sync"]);
            }
            // allow_subagents controls ConfigSubagent category expansion, not this
            // explicitly allowlisted builtin launcher or its owned-child controls.
            if names.contains("subagent") {
                require(&[
                    "agents_overview",
                    "agent_message",
                    "agent_list",
                    "agent_status",
                    "wait",
                    "agent_result",
                    "agent_cancel",
                    "agent_lifecycle",
                ]);
                assert_eq!(
                    crate::yaml_configs::customization_registry::match_tool_confirm_action(
                        &mode.tool_confirm.rules,
                        "agent_lifecycle",
                    )
                    .as_deref(),
                    Some("ask"),
                    "{} child recovery must retain explicit confirmation",
                    mode.id
                );
            }
            match mode.id.as_str() {
                "design" => require(&[
                    "design_system",
                    "ui_probe",
                    "chrome",
                    "contrast_audit",
                    "visual_diff",
                    "apply_patch",
                    "worktree_create",
                    "worktree_merge",
                    "worktree_sync",
                    "shell",
                    "process_start",
                    "process_list",
                    "process_read",
                    "process_kill",
                    "process_write_stdin",
                    "wait",
                    "ask_questions",
                ]),
                "task_planner" => {
                    require(&[
                        "spawn_agent",
                        "check_agents",
                        "merge_agent",
                        "restart_agent",
                        "worktree_sync",
                    ]);
                    assert!(
                        !names.contains("worktree_create"),
                        "planner must use task-owned checkout lifecycle"
                    );
                }
                "configurator" => require(&[
                    "ask_questions",
                    "create_textdoc",
                    "update_textdoc",
                    "apply_patch",
                ]),
                "learn" => require(&[
                    "set_trajectory_label",
                    "shell",
                    "process_start",
                    "process_list",
                    "process_read",
                    "process_kill",
                    "process_write_stdin",
                    "wait",
                    "schedule",
                ]),
                _ => {}
            }
            if ["ask", "brainstorming", "explore", "past_work", "plan"].contains(&mode.id.as_str())
            {
                for name in &names {
                    assert!(
                        !name.starts_with("worktree_")
                            && !name.starts_with("process_")
                            && !name.starts_with("update_textdoc")
                            && ![
                                "shell",
                                "shell_service",
                                "schedule",
                                "clean_background_processes",
                                "apply_patch",
                                "create_textdoc",
                                "undo_textdoc",
                                "rm",
                                "mv"
                            ]
                            .contains(&name.as_str()),
                        "{} must not expose core file/worktree/shell mutator {name}",
                        mode.id,
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn builtin_catalog_registers_set_trajectory_label() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let availability = ToolAvailability::from_gcx(gcx.clone()).await;
        let names = get_builtin_tools(gcx, &availability, None)
            .await
            .into_iter()
            .flat_map(|group| group.tools)
            .map(|tool| tool.tool_description().name)
            .collect::<HashSet<_>>();
        for name in [
            "set_trajectory_label",
            "wait",
            "schedule",
            "pause",
            "resume",
            "agent_lifecycle",
        ] {
            assert!(names.contains(name), "missing builtin {name}");
        }
    }

    #[tokio::test]
    async fn builtin_catalog_excludes_retired_compression_tools() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let availability = ToolAvailability::from_gcx(gcx.clone()).await;
        let descriptions = get_builtin_tools(gcx, &availability, None)
            .await
            .into_iter()
            .flat_map(|group| group.tools)
            .map(|tool| tool.tool_description())
            .collect::<Vec<_>>();
        assert!(!descriptions.is_empty());
        let subagent_config =
            include_str!("../../crates/refact-yaml-configs/src/defaults/subagents/subagent.yaml");
        for name in [
            "compress_chat_probe",
            "compress_chat_apply",
            "ctx_probe",
            "ctx_apply",
        ] {
            assert!(!descriptions.iter().any(|desc| desc.name == name));
            assert!(!subagent_config.contains(name));
        }
        assert!(descriptions
            .iter()
            .any(|desc| desc.name == "handoff_to_mode"));
    }

    struct TestTool(ToolDesc);

    struct AuthorityTestTool {
        desc: ToolDesc,
        executions: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Tool for AuthorityTestTool {
        async fn tool_execute(
            &mut self,
            _ccx: Arc<tokio::sync::Mutex<crate::at_commands::at_commands::AtCommandsContext>>,
            tool_call_id: &String,
            _args: &HashMap<String, serde_json::Value>,
        ) -> Result<(bool, Vec<crate::call_validation::ContextEnum>), String> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            Ok((
                false,
                vec![crate::call_validation::ContextEnum::ChatMessage(
                    crate::call_validation::ChatMessage {
                        role: "tool".into(),
                        tool_call_id: tool_call_id.clone(),
                        content: crate::call_validation::ChatContent::SimpleText(
                            "fixture result".into(),
                        ),
                        ..Default::default()
                    },
                )],
            ))
        }

        fn tool_description(&self) -> ToolDesc {
            self.desc.clone()
        }
    }

    struct AuthorityTestIntegration(Arc<AtomicUsize>);

    #[async_trait::async_trait]
    impl IntegrationTrait for AuthorityTestIntegration {
        fn integr_schema(&self) -> &str {
            "type: object"
        }

        async fn integr_settings_apply(
            &mut self,
            _gcx: Arc<GlobalContext>,
            _config_path: String,
            _value: &serde_json::Value,
        ) -> Result<(), serde_json::Error> {
            Ok(())
        }

        fn integr_settings_as_json(&self) -> serde_json::Value {
            serde_json::json!({})
        }

        fn integr_common(&self) -> crate::integrations::integr_abstract::IntegrationCommon {
            Default::default()
        }

        async fn integr_tools(&self, _integr_name: &str) -> Vec<Box<dyn Tool + Send>> {
            ["mcp_fixture_read", "mcp_fixture_write"]
                .into_iter()
                .map(|name| {
                    Box::new(AuthorityTestTool {
                        desc: mcp_desc("mcp_fixture.yaml", name),
                        executions: self.0.clone(),
                    }) as Box<dyn Tool + Send>
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn mcp_public_search_and_proxy_enforce_concrete_child_targets() {
        use crate::subchat::ToolsPolicy;
        use crate::tools::tools_description::MatchConfirmDenyResult;
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let executions = Arc::new(AtomicUsize::new(0));
        let counter = executions.clone();
        let factory: TestIntegrationFactory = Arc::new(move |_| {
            let mut integrations: LoadedIntegrations = indexmap::IndexMap::new();
            integrations.insert(
                "mcp_fixture".into(),
                Box::new(AuthorityTestIntegration(counter.clone())),
            );
            (integrations, vec![])
        });
        test_integration_factory()
            .lock()
            .await
            .insert(gcx.config_dir.clone(), factory);
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let context = Arc::new(tokio::sync::Mutex::new(
            crate::at_commands::at_commands::AtCommandsContext::new_from_app(
                app,
                4096,
                1,
                false,
                vec![],
                "mcp-child".into(),
                None,
                "model".into(),
                None,
                None,
            )
            .await,
        ));
        context.lock().await.child_tool_ceiling =
            Some(ToolsPolicy::Only(vec!["mcp_fixture_read".into()]));
        let mut search = crate::tools::tool_mcp_search::ToolMcpSearch {};
        let result = search
            .tool_execute(
                context.clone(),
                &"search".into(),
                &HashMap::from([("query".into(), serde_json::json!("."))]),
            )
            .await
            .unwrap();
        let serialized = result
            .1
            .iter()
            .map(|item| match item {
                crate::call_validation::ContextEnum::ChatMessage(message) => {
                    message.content.content_text_only()
                }
                _ => panic!("expected tool message"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(serialized.contains("mcp_fixture_read"));
        assert!(!serialized.contains("mcp_fixture_write"));
        let mut proxy = crate::tools::tool_mcp_call::ToolMcpCall {};
        for name in ["mcp_fixture_write", "fixture_write"] {
            let args = HashMap::from([
                ("tool_name".into(), serde_json::json!(name)),
                ("args".into(), serde_json::json!({})),
            ]);
            assert_eq!(
                proxy
                    .match_against_confirm_deny(context.clone(), &args)
                    .await
                    .unwrap()
                    .result,
                MatchConfirmDenyResult::DENY
            );
            assert!(proxy
                .tool_execute(context.clone(), &"denied".into(), &args)
                .await
                .is_err());
        }
        assert_eq!(executions.load(Ordering::SeqCst), 0);
        let allowed = HashMap::from([
            ("tool_name".into(), serde_json::json!("fixture_read")),
            ("args".into(), serde_json::json!({})),
        ]);
        proxy
            .tool_execute(context.clone(), &"allowed".into(), &allowed)
            .await
            .unwrap();
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        for ceiling in [
            ToolsPolicy::None,
            ToolsPolicy::Only(vec!["mcp_call".into(), "mcp_tool_search".into()]),
        ] {
            context.lock().await.child_tool_ceiling = Some(ceiling);
            assert!(proxy
                .tool_execute(context.clone(), &"denied".into(), &allowed)
                .await
                .is_err());
            let result = search
                .tool_execute(
                    context.clone(),
                    &"search".into(),
                    &HashMap::from([("query".into(), serde_json::json!("."))]),
                )
                .await
                .unwrap();
            let serialized = result
                .1
                .iter()
                .map(|item| match item {
                    crate::call_validation::ContextEnum::ChatMessage(message) => {
                        message.content.content_text_only()
                    }
                    _ => panic!("expected tool message"),
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(!serialized.contains("mcp_fixture_read"));
            assert!(!serialized.contains("mcp_fixture_write"));
            assert!(serialized.contains("all 0 available"));
        }
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        test_integration_factory()
            .lock()
            .await
            .remove(&gcx.config_dir);
    }

    #[derive(Clone)]
    struct IntegrationTestState {
        loads: Arc<AtomicUsize>,
        tool_builds: Arc<AtomicUsize>,
    }

    struct IntegrationTestTool {
        desc: ToolDesc,
    }

    #[async_trait::async_trait]
    impl Tool for IntegrationTestTool {
        async fn tool_execute(
            &mut self,
            _ccx: Arc<tokio::sync::Mutex<crate::at_commands::at_commands::AtCommandsContext>>,
            _tool_call_id: &String,
            _args: &HashMap<String, serde_json::Value>,
        ) -> Result<(bool, Vec<crate::call_validation::ContextEnum>), String> {
            unreachable!("test tools are only used to build the catalog")
        }

        fn tool_description(&self) -> ToolDesc {
            self.desc.clone()
        }
    }

    struct TestIntegration {
        state: IntegrationTestState,
        config_path: String,
        tool_names: Vec<String>,
    }

    #[async_trait::async_trait]
    impl IntegrationTrait for TestIntegration {
        fn integr_schema(&self) -> &str {
            "type: object"
        }

        async fn integr_settings_apply(
            &mut self,
            _gcx: Arc<GlobalContext>,
            config_path: String,
            _value: &serde_json::Value,
        ) -> Result<(), serde_json::Error> {
            self.config_path = config_path;
            Ok(())
        }

        fn integr_settings_as_json(&self) -> serde_json::Value {
            serde_json::json!({})
        }

        fn integr_common(&self) -> crate::integrations::integr_abstract::IntegrationCommon {
            Default::default()
        }

        async fn integr_tools(&self, _integr_name: &str) -> Vec<Box<dyn Tool + Send>> {
            let build_number = self.state.tool_builds.fetch_add(1, Ordering::SeqCst);
            self.tool_names
                .iter()
                .map(|tool_name| {
                    Box::new(IntegrationTestTool {
                        desc: ToolDesc {
                            name: tool_name.clone(),
                            experimental: false,
                            allow_parallel: true,
                            description: format!("build {build_number}"),
                            input_schema: serde_json::json!({"type":"object"}),
                            output_schema: None,
                            annotations: None,
                            display_name: tool_name.clone(),
                            source: ToolSource {
                                source_type: ToolSourceType::Integration,
                                config_path: self.config_path.clone(),
                            },
                        },
                    }) as Box<dyn Tool + Send>
                })
                .collect()
        }
    }

    fn write_test_integration_config(config_dir: &std::path::Path) {
        let dir = config_dir.join("integrations.d");
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|error| panic!("failed to create config dir: {error}"));
        std::fs::write(
            dir.join("cmdline.yaml"),
            "name: cmdline\ncommand: echo test\ntools:\n  cmdline:\n    enabled: true\n",
        )
        .unwrap_or_else(|error| panic!("failed to write integration config: {error}"));
    }

    async fn integration_test_gcx() -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_test_integration_config(&gcx.config_dir);
        gcx
    }

    async fn with_test_integrations<T>(
        config_dir: PathBuf,
        state: IntegrationTestState,
        specs: Vec<(String, Vec<String>)>,
        f: impl std::future::Future<Output = T>,
    ) -> T {
        let factory: TestIntegrationFactory = Arc::new(move |_gcx: Arc<GlobalContext>| {
            state.loads.fetch_add(1, Ordering::SeqCst);
            let mut integrations: LoadedIntegrations = indexmap::IndexMap::new();
            for (integr_name, tool_names) in &specs {
                integrations.insert(
                    integr_name.clone(),
                    Box::new(TestIntegration {
                        state: state.clone(),
                        config_path: format!("{integr_name}.yaml"),
                        tool_names: tool_names.clone(),
                    }),
                );
            }
            (integrations, Vec::new())
        });
        test_integration_factory()
            .lock()
            .await
            .insert(config_dir.clone(), factory);
        let result = f.await;
        test_integration_factory().lock().await.remove(&config_dir);
        result
    }

    async fn with_test_integration_factory<T>(
        config_dir: PathBuf,
        state: IntegrationTestState,
        f: impl std::future::Future<Output = T>,
    ) -> T {
        with_test_integrations(
            config_dir,
            state,
            vec![("cmdline".to_string(), vec!["cmdline_tool".to_string()])],
            f,
        )
        .await
    }

    #[async_trait::async_trait]
    impl Tool for TestTool {
        async fn tool_execute(
            &mut self,
            _ccx: Arc<tokio::sync::Mutex<crate::at_commands::at_commands::AtCommandsContext>>,
            _tool_call_id: &String,
            _args: &HashMap<String, serde_json::Value>,
        ) -> Result<(bool, Vec<crate::call_validation::ContextEnum>), String> {
            unreachable!("test tools are only used to build the lazy catalog")
        }

        fn tool_description(&self) -> ToolDesc {
            self.0.clone()
        }
    }

    fn policy_limiting(provider: &str, servers: &[&str]) -> refact_privacy::PrivacyPolicy {
        refact_privacy::PrivacyPolicy {
            tool_access: refact_privacy::ToolAccess {
                providers: std::collections::BTreeMap::from([(
                    provider.to_string(),
                    refact_privacy::ProviderToolAccess {
                        mcp: servers.iter().map(|s| (*s).to_string()).collect(),
                    },
                )]),
            },
            ..Default::default()
        }
    }

    #[test]
    fn blocked_server_tools_are_denied_and_allowed_ones_survive() {
        let policy = policy_limiting("openai_codex", &["github"]);

        assert!(mcp_tool_allowed(
            &policy,
            "openai_codex",
            &mcp_desc("mcp_stdio_github.yaml", "mcp_github_create_issue")
        ));
        assert!(!mcp_tool_allowed(
            &policy,
            "openai_codex",
            &mcp_desc("mcp_stdio_postgres.yaml", "mcp_postgres_query")
        ));
    }

    #[test]
    fn non_mcp_tools_are_never_gated() {
        let policy = policy_limiting("openai_codex", &[]);

        assert!(mcp_tool_allowed(
            &policy,
            "openai_codex",
            &builtin_desc("cat")
        ));
        assert!(mcp_tool_allowed(
            &policy,
            "openai_codex",
            &builtin_desc("mcp_call")
        ));
    }

    fn counting_state() -> IntegrationTestState {
        IntegrationTestState {
            loads: Arc::new(AtomicUsize::new(0)),
            tool_builds: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn mcp_integration_specs() -> Vec<(String, Vec<String>)> {
        vec![
            (
                "alpha".to_string(),
                vec!["mcp_alpha_one".to_string(), "mcp_alpha_two".to_string()],
            ),
            ("beta".to_string(), vec!["mcp_beta_one".to_string()]),
            ("plain".to_string(), vec!["plain_shell".to_string()]),
            ("zeta".to_string(), vec!["mcp_zeta_one".to_string()]),
        ]
    }

    #[test]
    fn unlisted_provider_and_wildcard_keep_every_server() {
        let policy = policy_limiting("openai_codex", &["*"]);
        let tool = mcp_desc("mcp_stdio_postgres.yaml", "mcp_postgres_query");

        assert!(mcp_tool_allowed(&policy, "openai_codex", &tool));
        assert!(mcp_tool_allowed(&policy, "ollama", &tool));
    }

    #[test]
    fn unknown_caller_is_denied_when_an_allowlist_exists() {
        let policy = policy_limiting("openai_codex", &["github"]);

        assert!(!mcp_tool_allowed(
            &policy,
            "",
            &mcp_desc("mcp_stdio_github.yaml", "mcp_github_create_issue")
        ));
    }

    fn exposed_subagent(id: &str) -> SubagentConfig {
        SubagentConfig {
            schema_version: 1,
            id: id.to_string(),
            title: id.to_string(),
            description: String::new(),
            specific: false,
            expose_as_tool: true,
            has_code: false,
            tool: None,
            subchat: SubchatConfig::default(),
            messages: SubagentMessages::default(),
            prompts: SubagentPrompts::default(),
            gather_files: Default::default(),
            tools: Vec::new(),
            base: None,
            match_models: None,
            extra: HashMap::new(),
        }
    }

    fn insert_registry(gcx: &Arc<GlobalContext>, registry: ProjectRegistry) {
        gcx.project_registry_cache.write().unwrap().cache.insert(
            gcx.config_dir.clone(),
            RegistryCache {
                project_root: gcx.config_dir.clone(),
                registry,
                last_scan: std::time::SystemTime::now(),
            },
        );
    }

    #[tokio::test]
    async fn agent_and_work_tool_names_stay_registered_as_builtins() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();
        let tools = get_available_tools(gcx).await;
        let builtin = |name: &str| {
            tools
                .iter()
                .filter(|tool| tool.tool_description().name == name)
                .map(|tool| {
                    let desc = tool.tool_description();
                    (
                        matches!(desc.source.source_type, ToolSourceType::Builtin),
                        desc.allow_parallel,
                    )
                })
                .collect::<Vec<_>>()
        };
        for name in [
            "subagent",
            "agents_overview",
            "agent_list",
            "agent_status",
            "wait",
            "agent_result",
            "agent_cancel",
            "agent_lifecycle",
            "agent_message",
            "agent_mailbox",
            "agent_peer_grant",
            "team_seat",
            "team_role_grant",
            "progress_report",
            "board_claim",
            "board_discover",
        ] {
            assert_eq!(
                builtin(name).len(),
                1,
                "{name} must stay registered exactly once"
            );
            assert!(builtin(name)[0].0, "{name} must remain a builtin");
        }
        for name in ["work_overview", "work_inspect", "metrics", "board_discover"] {
            assert_eq!(
                builtin(name),
                vec![(true, true)],
                "{name} is read-only and parallel"
            );
        }
        assert_eq!(builtin("team_seat"), vec![(true, false)]);
        assert_eq!(builtin("team_role_grant"), vec![(true, false)]);
    }

    #[tokio::test]
    async fn board_tools_are_available_only_to_modes_with_board_authority() {
        let gcx = task_prompt_contract_gcx().await;

        for mode in ["task_planner", "task_agent"] {
            let tool_names = get_tools_for_mode(gcx.clone(), mode, None)
                .await
                .into_iter()
                .map(|tool| tool.tool_description().name)
                .collect::<Vec<_>>();
            assert!(
                tool_names.contains(&"board_claim".to_string()),
                "{mode}: {tool_names:?}"
            );
            assert!(
                tool_names.contains(&"board_discover".to_string()),
                "{mode}: {tool_names:?}"
            );
        }

        let agent_tools = get_tools_for_mode(gcx, "agent", None)
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect::<Vec<_>>();
        assert!(
            agent_tools.contains(&"board_discover".to_string()),
            "{agent_tools:?}"
        );
        assert!(
            !agent_tools.contains(&"board_claim".to_string()),
            "{agent_tools:?}"
        );
    }

    struct PromptContractVecdb;

    #[async_trait::async_trait]
    impl crate::vecdb::vdb_structs::VecdbSearch for PromptContractVecdb {
        async fn vecdb_search(
            &self,
            query: String,
            _top_n: usize,
            _filter_mb: Option<String>,
        ) -> Result<crate::vecdb::vdb_structs::SearchResult, String> {
            Ok(crate::vecdb::vdb_structs::SearchResult {
                query_text: query,
                results: Vec::new(),
            })
        }

        async fn get_status(&self) -> Result<crate::vecdb::vdb_structs::VecDbStatus, String> {
            Ok(crate::vecdb::vdb_structs::VecDbStatus {
                files_unprocessed: 0,
                files_total: 0,
                requests_made_since_start: 0,
                vectors_made_since_start: 0,
                db_size: 0,
                db_cache_size: 0,
                state: "done".to_string(),
                queue_additions: false,
                vecdb_max_files_hit: false,
                vecdb_errors: Default::default(),
                cache_writes_failed: 0,
                vectors_missing_from_cache: 0,
            })
        }

        async fn remove_file(&self, _file_path: &PathBuf) -> Result<(), String> {
            Ok(())
        }

        async fn vectorizer_enqueue_files(
            &self,
            _documents: &[String],
            _process_immediately: bool,
            _roots: refact_core::memory_plane::MemoryPlaneRoots,
        ) {
        }

        fn current_constants(&self) -> (crate::vecdb::vdb_structs::EmbeddingModelConfig, usize) {
            (
                crate::vecdb::vdb_structs::EmbeddingModelConfig {
                    model_id: String::new(),
                    endpoint: String::new(),
                    endpoint_style: String::new(),
                    embedding_endpoint_style: String::new(),
                    api_key: String::new(),
                    model_name: String::new(),
                    embedding_size: 0,
                    dimensions: None,
                    query_prefix: String::new(),
                    document_prefix: String::new(),
                    rejection_threshold: 0.0,
                    embedding_batch: 1,
                    n_ctx: 0,
                },
                0,
            )
        }

        async fn embed_query(&self, _query: &str) -> Result<Vec<f32>, String> {
            Ok(Vec::new())
        }

        async fn vecdb_search_with_embedding(
            &self,
            _embedding: &Vec<f32>,
            _top_n: usize,
            _filter_mb: Option<String>,
        ) -> Result<Vec<crate::vecdb::vdb_structs::VecdbRecord>, String> {
            Ok(Vec::new())
        }
    }

    async fn task_prompt_contract_gcx() -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();
        *gcx.codegraph.lock().await = Some(Arc::new(
            refact_codegraph::CodeGraphService::open_in_memory().unwrap(),
        ));
        *gcx.vec_db.lock().await = Some(Arc::new(PromptContractVecdb));
        gcx
    }

    fn read_default_task_mode(
        mode_id: &str,
    ) -> crate::yaml_configs::customization_types::ModeConfig {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("crates")
            .join("refact-yaml-configs")
            .join("src")
            .join("defaults")
            .join("modes")
            .join(format!("{mode_id}.yaml"));
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        serde_yaml::from_str(&raw)
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
    }

    fn task_prompt_tool_references(prompt: &str) -> HashSet<String> {
        let mut refs = HashSet::new();
        let call_re = regex::Regex::new(r"\b([a-z][a-z0-9_]*)\(").unwrap();
        for caps in call_re.captures_iter(prompt) {
            refs.insert(caps[1].to_string());
        }

        let non_tool_code_terms = HashSet::from([
            "agent_branch",
            "cache_control",
            "content",
            "depends_on",
            "description",
            "executing_tools",
            "exit_code",
            "files_changed",
            "followup_cards",
            "node_modules",
            "output_tail",
            "path",
            "relevant_documents",
            "retain_worktree",
            "run_in_background",
            "suggested_steps",
            "t_shell",
            "target_files",
            "test_name",
            "test_pattern",
            "tests_added_or_updated",
            "worktree_path",
        ]);
        let code_span_re = regex::Regex::new(r"`([^`]+)`").unwrap();
        let snake_re = regex::Regex::new(r"\b[a-z][a-z0-9_]*_[a-z0-9_]+\b").unwrap();
        let toolish_span_re = regex::Regex::new(r"^([a-z][a-z0-9_]*)(?:\s*\(|$)").unwrap();
        for caps in code_span_re.captures_iter(prompt) {
            let span = &caps[1];
            if span.contains('\n') {
                continue;
            }
            if span.trim() != span {
                continue;
            }
            if span.contains(" / ") {
                for token in snake_re.find_iter(span) {
                    let token = token.as_str();
                    if !non_tool_code_terms.contains(token) {
                        refs.insert(token.to_string());
                    }
                }
                continue;
            }
            let span = span.trim();
            if let Some(caps) = toolish_span_re.captures(span) {
                let token = caps[1].to_string();
                if !non_tool_code_terms.contains(token.as_str()) {
                    refs.insert(token);
                }
            }
        }

        refs
    }

    async fn assert_task_mode_prompt_tool_contract(mode_id: &str) {
        let mode = read_default_task_mode(mode_id);
        let gcx = task_prompt_contract_gcx().await;
        let registered_tools = get_available_tools(gcx.clone())
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect::<HashSet<_>>();
        let mode_tools = get_tools_for_mode(gcx, mode_id, None)
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect::<HashSet<_>>();

        let missing_registered = mode
            .tools
            .iter()
            .filter(|name| !registered_tools.contains(*name))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            missing_registered.is_empty(),
            "{mode_id} tools list references unregistered tools: {missing_registered:?}"
        );

        let missing_from_mode = mode
            .tools
            .iter()
            .filter(|name| !mode_tools.contains(*name))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            missing_from_mode.is_empty(),
            "{mode_id} tools list contains tools unavailable in the mode: {missing_from_mode:?}; available={mode_tools:?}"
        );

        let prompt_refs = task_prompt_tool_references(&mode.prompt);
        let missing_prompt_refs = prompt_refs
            .iter()
            .filter(|name| !mode_tools.contains(*name))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            missing_prompt_refs.is_empty(),
            "{mode_id} prompt references tools unavailable in the mode: {missing_prompt_refs:?}; refs={prompt_refs:?}; available={mode_tools:?}"
        );
    }

    #[test]
    fn default_mode_prompts_include_models_info_and_no_delegate_tool_policy() {
        let modes_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("crates")
            .join("refact-yaml-configs")
            .join("src")
            .join("defaults")
            .join("modes");
        let mut failures = Vec::new();
        let mut checked_modes = 0usize;
        for entry in std::fs::read_dir(&modes_dir).expect("failed to read default modes dir") {
            let path = entry.expect("failed to read modes dir entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
                continue;
            }
            let file_name = path.file_name().unwrap().to_string_lossy().to_string();
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
            let value: serde_yaml::Value = serde_yaml::from_str(&raw)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
            for retired in [
                "compress_chat_*",
                "compress_chat_probe",
                "compress_chat_apply",
                "ctx_probe",
                "ctx_apply",
                "%COMPRESS_HANDOFF_INSTRUCTIONS%",
            ] {
                assert!(!raw.contains(retired), "{file_name} references {retired}");
            }
            checked_modes += 1;
            if !value
                .get("prompt")
                .and_then(|value| value.as_str())
                .is_some_and(|prompt| prompt.contains("%MODELS_INFO%"))
            {
                failures.push(format!("{file_name}: missing %MODELS_INFO% in prompt"));
            }
            if value
                .get("tools")
                .and_then(|value| value.as_sequence())
                .is_some_and(|tools| tools.iter().any(|tool| tool.as_str() == Some("delegate")))
            {
                failures.push(format!("{file_name}: tools includes delegate"));
            }
            if value
                .get("tool_confirm")
                .and_then(|value| value.get("rules"))
                .and_then(|value| value.as_sequence())
                .is_some_and(|rules| {
                    rules.iter().any(|rule| {
                        rule.get("match").and_then(|value| value.as_str()) == Some("delegate")
                    })
                })
            {
                failures.push(format!("{file_name}: tool_confirm includes delegate"));
            }
        }
        assert!(
            checked_modes > 10,
            "expected to check most default modes, only found {checked_modes}"
        );
        assert!(
            failures.is_empty(),
            "default mode prompt/tool policy regressions: {failures:?}"
        );
    }

    fn extract_chrome_examples(prompt: &str) -> Vec<String> {
        let mut examples = Vec::new();
        let mut rest = prompt;
        while let Some(start) = rest.find("chrome({") {
            let payload = &rest[start + "chrome(".len()..];
            let mut depth = 0usize;
            let mut end = None;
            for (index, character) in payload.char_indices() {
                match character {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(index + character.len_utf8());
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(end) = end else {
                panic!(
                    "unbalanced chrome example near: {}",
                    &payload[..40.min(payload.len())]
                );
            };
            examples.push(payload[..end].to_string());
            rest = &payload[end..];
        }
        examples
    }

    struct CountingTool {
        desc: ToolDesc,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Tool for CountingTool {
        async fn tool_execute(
            &mut self,
            _ccx: Arc<tokio::sync::Mutex<crate::at_commands::at_commands::AtCommandsContext>>,
            _tool_call_id: &String,
            _args: &HashMap<String, serde_json::Value>,
        ) -> Result<(bool, Vec<crate::call_validation::ContextEnum>), String> {
            unreachable!("test tools are only used to build the catalog")
        }

        fn tool_description(&self) -> ToolDesc {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.desc.clone()
        }

        fn config(&self) -> Result<crate::tools::tools_description::ToolConfig, String> {
            Ok(crate::tools::tools_description::ToolConfig::default())
        }
    }

    fn described_builtin(name: &str) -> DescribedTool {
        let tool_desc = builtin_desc(name);
        (
            ToolGroupCategory::Builtin,
            tool_desc.clone(),
            Box::new(TestTool(tool_desc)) as Box<dyn Tool + Send>,
        )
    }

    fn described_integration(name: &str) -> DescribedTool {
        let tool_desc = mcp_desc("stdio_test.yaml", name);
        (
            ToolGroupCategory::Integration,
            tool_desc.clone(),
            Box::new(TestTool(tool_desc)) as Box<dyn Tool + Send>,
        )
    }

    fn selected_names(tools: Vec<Box<dyn Tool + Send>>) -> Vec<String> {
        tools
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect()
    }
}

pub async fn get_available_tool_groups(gcx: Arc<GlobalContext>) -> Vec<ToolGroup> {
    let availability = ToolAvailability::from_gcx(gcx.clone()).await;
    get_available_tool_groups_with_availability(gcx, &availability, None).await
}

async fn get_available_tool_groups_with_availability(
    gcx: Arc<GlobalContext>,
    availability: &ToolAvailability,
    gated_tools: Option<&mut HashMap<String, ToolGating>>,
) -> Vec<ToolGroup> {
    let mut gated_tools = gated_tools;
    let mut tools_all =
        get_builtin_tools(gcx.clone(), availability, gated_tools.as_deref_mut()).await;
    let integration_tools =
        build_integration_tool_groups(gcx.clone(), availability, gated_tools.as_deref_mut()).await;
    tools_all.extend(integration_tools);

    let config_subagent_group = get_config_subagent_tools(gcx).await;
    if !config_subagent_group.tools.is_empty() {
        tools_all.push(config_subagent_group);
    }

    tools_all
}

pub async fn get_available_tools(gcx: Arc<GlobalContext>) -> Vec<Box<dyn Tool + Send>> {
    get_available_tool_groups(gcx)
        .await
        .into_iter()
        .flat_map(|g| g.tools)
        .collect()
}

type DescribedTool = (ToolGroupCategory, ToolDesc, Box<dyn Tool + Send>);

pub(crate) async fn chat_has_parent_agent(app: &crate::app_state::AppState, chat_id: &str) -> bool {
    let session = app.chat.sessions.read().await.get(chat_id).cloned();
    let Some(session) = session else {
        return app.agents.child_chat_status(chat_id).await.is_some();
    };
    let has_parent = session.lock().await.tool_exposure_has_parent.clone();
    *has_parent
        .get_or_init(|| async { app.agents.child_chat_status(chat_id).await.is_some() })
        .await
}

pub(crate) fn filter_chat_tools(tools: &mut Vec<ToolDesc>, has_parent: bool) {
    tools.retain(|tool| has_parent || tool.name != "progress_report");
}

pub(crate) async fn filter_tool_catalog_for_chat(
    app: &crate::app_state::AppState,
    chat_id: &str,
    catalog: Arc<refact_runtime_api::ToolCatalogSnapshot>,
) -> Arc<refact_runtime_api::ToolCatalogSnapshot> {
    if chat_has_parent_agent(app, chat_id).await
        || !catalog
            .index
            .tools
            .iter()
            .any(|tool| tool.name == "progress_report")
    {
        return catalog;
    }
    let mut filtered = (*catalog).clone();
    filter_chat_tools(&mut filtered.index.tools, false);
    filtered
        .policy
        .retain(|tool| tool.name != "progress_report");
    let names = filtered
        .index
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    filtered.aliases = refact_tool_api::build_registry_from_names(&names);
    Arc::new(filtered)
}

fn select_and_order_mode_tools(
    described_tools: Vec<DescribedTool>,
    mode_tools: &[String],
    allow_integrations: bool,
    allow_mcp: bool,
    allow_subagents: bool,
    model_supports_web_search: bool,
    mcp_access: Option<(&refact_privacy::PrivacyPolicy, &str)>,
) -> Vec<Box<dyn Tool + Send>> {
    let allowed_tools: std::collections::HashSet<&str> =
        mode_tools.iter().map(|name| name.as_str()).collect();
    let tool_order: HashMap<&str, usize> = mode_tools
        .iter()
        .enumerate()
        .map(|(index, name)| (name.as_str(), index))
        .collect();

    let mut selected: Vec<(ToolDesc, Box<dyn Tool + Send>)> = described_tools
        .into_iter()
        .filter(|(_, _, tool)| tool.config().unwrap_or_default().enabled)
        .filter(|(_, desc, _)| !(desc.name == "web_search" && model_supports_web_search))
        .filter(|(category, desc, _)| match category {
            ToolGroupCategory::Integration if allow_integrations => true,
            ToolGroupCategory::MCP if allow_mcp => true,
            ToolGroupCategory::ConfigSubagent if allow_subagents => true,
            _ => allowed_tools.contains(desc.name.as_str()),
        })
        .filter(|(_, desc, _)| match mcp_access {
            Some((policy, provider)) => mcp_tool_allowed(policy, provider, desc),
            None => true,
        })
        .map(|(_, desc, tool)| (desc, tool))
        .collect();

    selected.sort_by(|(a, _), (b, _)| {
        let a_order = tool_order
            .get(a.name.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        let b_order = tool_order
            .get(b.name.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        a_order.cmp(&b_order).then_with(|| a.name.cmp(&b.name))
    });

    selected.into_iter().map(|(_, tool)| tool).collect()
}

pub async fn get_tools_for_mode(
    gcx: Arc<GlobalContext>,
    mode_id: &str,
    model_id: Option<&str>,
) -> Vec<Box<dyn Tool + Send>> {
    use crate::yaml_configs::customization_registry::{get_mode_config, map_legacy_mode_to_id};
    use std::collections::HashSet;

    let mode_id = map_legacy_mode_to_id(mode_id);

    let span = perf_diagnostics::span(PerfComponent::ToolModeConfig, None, None);
    let mode_config = get_mode_config(gcx.clone(), mode_id, model_id).await;
    span.finish_tool(PerfOutcome::Success, 1, 1, None);
    let mode_config = match mode_config {
        Some(config) => config,
        None => {
            tracing::warn!("Mode '{}' not found, returning empty tools", mode_id);
            return vec![];
        }
    };

    if mode_config.tools.is_empty() {
        return vec![];
    }

    let span = perf_diagnostics::span(PerfComponent::ToolModelCaps, None, None);
    let model_supports_web_search = if let Some(mid) = model_id {
        match try_load_caps_quickly_if_not_present(gcx.clone(), 0).await {
            Ok(caps) => resolve_chat_model(caps, mid)
                .map(|rec| rec.base.supports_web_search)
                .unwrap_or(false),
            Err(_) => false,
        }
    } else {
        false
    };

    span.finish_tool(PerfOutcome::Success, 1, 1, None);
    let availability = ToolAvailability::from_gcx(gcx.clone()).await;
    let mut gated_tools = HashMap::new();
    let described_tools: Vec<DescribedTool> = get_available_tool_groups_with_availability(
        gcx.clone(),
        &availability,
        Some(&mut gated_tools),
    )
    .await
    .into_iter()
    .flat_map(|group| {
        let category = group.category;
        group.tools.into_iter().map(move |tool| {
            let desc = tool.tool_description();
            (category, desc, tool)
        })
    })
    .collect();

    let registered_tool_names: HashSet<String> = described_tools
        .iter()
        .map(|(_, desc, _)| desc.name.clone())
        .collect();
    crate::yaml_configs::mode_validation::warn_unavailable_mode_tools(
        gcx.clone(),
        mode_id,
        &mode_config.tools,
        &registered_tool_names,
        &gated_tools,
    )
    .await;

    let mcp_access = match model_id {
        Some(mid) => {
            let span = perf_diagnostics::span(PerfComponent::ToolAccessPolicy, None, None);
            let policy = tool_access_policy(gcx.clone()).await;
            span.finish_tool(PerfOutcome::Success, 1, 1, None);
            if policy.tool_access.providers.is_empty() {
                None
            } else {
                Some((policy, provider_of_model(mid).to_string()))
            }
        }
        None => None,
    };

    select_and_order_mode_tools(
        described_tools,
        &mode_config.tools,
        mode_config.allow_integrations,
        mode_config.allow_mcp,
        mode_config.allow_subagents,
        model_supports_web_search,
        mcp_access
            .as_ref()
            .map(|(policy, provider)| (policy.as_ref(), provider.as_str())),
    )
}
