use std::collections::HashMap;
use std::sync::Arc;

use crate::caps::resolve_chat_model;
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};
use crate::integrations::running_integrations::load_integrations;
use crate::yaml_configs::customization_registry::{
    get_project_registry, should_expose_subagent_as_config_tool,
};

use super::tools_description::{Tool, ToolDesc, ToolGroup, ToolGroupCategory, ToolSourceType};
use super::tool_config_subagent::ToolConfigSubagent;

/// When MCP tool count exceeds this threshold, lazy loading activates.
/// The full MCP schemas are replaced by two fixed proxy tools:
/// - `mcp_tool_search` — discover MCP tools by regex, returns schema text
/// - `mcp_call`        — execute any MCP tool by name + args JSON
///
/// The tool list is FIXED for the entire session (cache-safe).
const MCP_LAZY_THRESHOLD: usize = 15;

/// Result of applying MCP lazy-loading logic on a tool list.
pub struct ToolsForMode {
    /// Tool list to send to the LLM as schemas. Fixed for the session lifetime.
    pub tools: Vec<Box<dyn Tool + Send>>,
    /// True when lazy mode replaced MCP schemas with the two proxy tools.
    pub mcp_lazy_mode: bool,
    /// Total count of all MCP tools (for the hint message).
    pub mcp_total_count: usize,
    /// (name, description) index for ALL MCP tools — used to build the `cd_instruction` hint.
    /// Empty when lazy mode is inactive.
    pub mcp_tool_index: Vec<(String, String)>,
}

/// Returns true for real MCP integration tools, false for the proxy builtins
/// (`mcp_call`, `mcp_tool_search`) which share the "mcp" name prefix but have
/// `ToolSourceType::Builtin`. This makes `apply_mcp_lazy_filter` idempotent.
fn is_integration_mcp_tool(t: &Box<dyn Tool + Send>) -> bool {
    let d = t.tool_description();
    d.name.starts_with("mcp") && matches!(d.source.source_type, ToolSourceType::Integration)
}

/// Apply MCP lazy-loading to a flat tool list returned by `get_tools_for_mode`.
///
/// When there are more than `MCP_LAZY_THRESHOLD` MCP tools, ALL individual MCP
/// schemas are replaced by two fixed proxy tools (`mcp_tool_search` + `mcp_call`).
/// The tool list produced here NEVER changes during the session — cache-safe.
///
/// Safe to call multiple times: proxy tools have `ToolSourceType::Builtin` so they
/// are never counted or removed by subsequent calls.
pub fn apply_mcp_lazy_filter(mut tools: Vec<Box<dyn Tool + Send>>) -> ToolsForMode {
    // Collect the index of ALL real MCP integration tools before filtering.
    // Proxy builtins (mcp_call / mcp_tool_search) are excluded via source_type check.
    let mcp_tool_index: Vec<(String, String)> = tools
        .iter()
        .filter(|t| is_integration_mcp_tool(t))
        .map(|t| {
            let d = t.tool_description();
            (d.name, d.description)
        })
        .collect();

    let mcp_total_count = mcp_tool_index.len();
    let mcp_lazy_mode = mcp_total_count > MCP_LAZY_THRESHOLD;

    if mcp_lazy_mode {
        // Drop ALL individual MCP tool schemas (integration tools only).
        tools.retain(|t| !is_integration_mcp_tool(t));
        // Inject two fixed proxies — tool list is now stable for the session.
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
            vec![]
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

fn tool_available(
    tool: &Box<dyn Tool + Send>,
    codegraph_on: bool,
    vecdb_on: bool,
    is_there_a_thinking_model: bool,
    allow_knowledge: bool,
    allow_experimental: bool,
) -> bool {
    let dependencies = tool.tool_depends_on();
    if dependencies
        .iter()
        .any(|dependency| dependency == "ast" || dependency == "codegraph")
        && !codegraph_on
    {
        return false;
    }
    if dependencies.iter().any(|dependency| dependency == "vecdb") && !vecdb_on {
        return false;
    }
    if dependencies
        .iter()
        .any(|dependency| dependency == "thinking")
        && !is_there_a_thinking_model
    {
        return false;
    }
    if dependencies
        .iter()
        .any(|dependency| dependency == "knowledge")
        && !allow_knowledge
    {
        return false;
    }
    if tool.tool_description().experimental && !allow_experimental {
        return false;
    }
    true
}

async fn tool_available_from_gcx(
    gcx: Arc<GlobalContext>,
) -> impl Fn(&Box<dyn Tool + Send>) -> bool {
    let (codegraph_on, vecdb_on, allow_experimental) = {
        let vecdb_on = gcx.vec_db.lock().await.is_some();
        let codegraph_on = gcx.codegraph.lock().await.is_some();
        (codegraph_on, vecdb_on, gcx.cmdline.experimental)
    };

    let is_there_a_thinking_model = match try_load_caps_quickly_if_not_present(gcx.clone(), 0).await
    {
        Ok(caps) => caps
            .chat_models
            .get(&caps.defaults.chat_thinking_model)
            .is_some(),
        Err(_) => false,
    };
    let allow_knowledge = true;

    move |tool: &Box<dyn Tool + Send>| {
        tool_available(
            tool,
            codegraph_on,
            vecdb_on,
            is_there_a_thinking_model,
            allow_knowledge,
            allow_experimental,
        )
    }
}

impl ToolGroup {
    pub async fn retain_available_tools(&mut self, gcx: Arc<GlobalContext>) {
        let tool_available = tool_available_from_gcx(gcx.clone()).await;
        self.tools.retain(|tool| tool_available(tool));
    }
}

pub(crate) fn builtin_system_tools(config_path: String) -> Vec<Box<dyn Tool + Send>> {
    vec![
        Box::new(crate::tools::tool_shell::ToolShell {
            cfg: crate::tools::tool_shell::SettingsShell {
                timeout: "10".to_string(),
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
        Box::new(crate::tools::tool_process::ToolProcessWait {
            config_path: config_path.clone(),
        }),
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
        Box::new(crate::tools::tool_sleep::ToolSleep {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_cron_create::ToolCronCreate::new(
            config_path.clone(),
        )),
        Box::new(crate::tools::tool_cron_list::ToolCronList::new(
            config_path.clone(),
        )),
        Box::new(crate::tools::tool_cron_delete::ToolCronDelete::new(
            config_path.clone(),
        )),
        Box::new(crate::tools::tool_cron_update::ToolCronUpdate::new(
            config_path.clone(),
        )),
        Box::new(
            crate::tools::tool_add_workspace_folder::ToolAddWorkspaceFolder {
                config_path: config_path.clone(),
            },
        ),
    ]
}

async fn get_builtin_tools(gcx: Arc<GlobalContext>) -> Vec<ToolGroup> {
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
        Box::new(crate::tools::tool_review::ToolCodeReview {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_subagent::ToolSubagent {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_delegate::ToolDelegate {
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
        Box::new(crate::tools::tool_activate_skill::ToolDeactivateSkill {
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
        Box::new(crate::tools::tool_get_plan::ToolGetPlan::new(
            config_path.clone(),
        )),
        Box::new(crate::tools::tool_set_plan::ToolSetPlan {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_update_plan::ToolUpdatePlan {
            config_path: config_path.clone(),
        }),
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
        Box::new(crate::tools::tool_goal_pursuit_controls::ToolPauseGoal {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_goal_pursuit_controls::ToolSnoozeGoal {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_compress_chat::ToolCompressChatProbe {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_compress_chat::ToolCompressChatApply {
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
        Box::new(crate::tools::tool_agent_lifecycle::ToolPauseAgent::new()),
        Box::new(crate::tools::tool_agent_lifecycle::ToolResumeAgent::new()),
        Box::new(crate::tools::tool_task_wait_for_agents::ToolTaskWaitForAgents::new()),
        Box::new(crate::tools::tool_task_agent_finish::ToolTaskAgentFinish::new()),
        Box::new(crate::tools::tool_task_mark_card::ToolTaskMarkCardDone::new()),
        Box::new(crate::tools::tool_task_mark_card::ToolTaskMarkCardFailed::new()),
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
        Box::new(crate::tools::tool_background_agents::ToolAgentList {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_background_agents::ToolAgentStatus {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_background_agents::ToolAgentWait {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_background_agents::ToolAgentResult {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_background_agents::ToolAgentCancel {
            config_path: config_path.clone(),
        }),
    ];

    let worktree_tools: Vec<Box<dyn Tool + Send>> = vec![Box::new(
        crate::tools::tool_worktree_merge::ToolWorktreeMerge::new(),
    )];

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

    for tool_group in tool_groups.iter_mut() {
        tool_group.retain_available_tools(gcx.clone()).await;
    }

    tool_groups
}

pub async fn get_integration_tools(gcx: Arc<GlobalContext>) -> Vec<ToolGroup> {
    let mut integrations_group = ToolGroup {
        name: "Integrations".to_string(),
        description: "Integration tools".to_string(),
        category: ToolGroupCategory::Integration,
        tools: vec![],
    };

    let mut mcp_groups = HashMap::new();

    let (integrations_map, _yaml_errors) =
        load_integrations(gcx.clone(), &["**/*".to_string()]).await;
    for (name, integr) in integrations_map {
        for tool in integr.integr_tools(&name).await {
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

    for tool_group in tool_groups.iter_mut() {
        tool_group.retain_available_tools(gcx.clone()).await;
    }

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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::collections::HashSet;
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
    fn provider_of_model_takes_the_prefix_before_the_slash() {
        assert_eq!(provider_of_model("openai_codex/gpt-5"), "openai_codex");
        assert_eq!(provider_of_model("ollama/llama3:8b"), "ollama");
        assert_eq!(provider_of_model("bare-model"), "bare-model");
        assert_eq!(provider_of_model(""), "");
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
    async fn config_subagent_tools_skip_builtin_ids() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let registry = ProjectRegistry {
            subagents: HashMap::from_iter([
                ("subagent".to_string(), exposed_subagent("subagent")),
                (
                    "project_researcher".to_string(),
                    exposed_subagent("project_researcher"),
                ),
            ]),
            ..Default::default()
        };
        insert_registry(&gcx, registry);

        let group = get_config_subagent_tools(gcx).await;
        let names = group
            .tools
            .iter()
            .map(|tool| tool.tool_description().name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["project_researcher"]);
    }

    #[tokio::test]
    async fn available_tools_include_only_builtin_background_subagent_for_default_config() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();

        let tools = get_available_tools(gcx).await;
        let subagents = tools
            .iter()
            .filter_map(|tool| {
                let desc = tool.tool_description();
                (desc.name == "subagent")
                    .then_some(matches!(desc.source.source_type, ToolSourceType::Builtin))
            })
            .collect::<Vec<_>>();

        assert_eq!(subagents, vec![true]);
    }

    #[tokio::test]
    async fn task_planner_includes_web_tools() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();

        let tool_names = get_tools_for_mode(gcx.clone(), "task_planner", None)
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect::<Vec<_>>();

        assert!(tool_names.contains(&"web".to_string()), "{tool_names:?}");
        assert!(
            tool_names.contains(&"web_search".to_string()),
            "{tool_names:?}"
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

    #[tokio::test]
    async fn task_planner_prompt_references_only_available_tools() {
        assert_task_mode_prompt_tool_contract("task_planner").await;
    }

    #[tokio::test]
    async fn task_agent_prompt_references_only_available_tools() {
        assert_task_mode_prompt_tool_contract("task_agent").await;
    }

    /// `get_tools_for_mode` filters mode YAML `tools:` lists by exact match against
    /// registered `ToolDesc.name` values and silently drops anything that does not
    /// match. A YAML entry with a wrong name (e.g. a Claude-Code-facing alias such
    /// as `regex_search` instead of the canonical `search_pattern`) therefore
    /// removes the tool from the mode without any warning. This test covers every
    /// default mode, not just the task modes.
    #[tokio::test]
    async fn all_default_mode_tools_reference_registered_tools() {
        let modes_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("crates")
            .join("refact-yaml-configs")
            .join("src")
            .join("defaults")
            .join("modes");
        let gcx = task_prompt_contract_gcx().await;
        let registered = get_available_tools(gcx)
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect::<HashSet<_>>();

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
            // Model-specific overlays patch a base mode and carry no standalone tools list.
            if value
                .get("specific")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                && value.get("tools").is_none()
            {
                continue;
            }
            let Some(tools) = value.get("tools").and_then(|v| v.as_sequence()) else {
                continue;
            };
            checked_modes += 1;
            for tool in tools {
                let Some(name) = tool.as_str() else {
                    failures.push(format!("{file_name}: non-string tools entry {tool:?}"));
                    continue;
                };
                // Dynamically-registered tool families that depend on user config.
                if name.starts_with("cmdline_")
                    || name.starts_with("service_")
                    || name.starts_with("mcp_")
                {
                    continue;
                }
                if !registered.contains(name) {
                    failures.push(format!("{file_name}: {name}"));
                }
            }
        }
        assert!(
            checked_modes > 10,
            "expected to check most default modes, only found {checked_modes}"
        );
        assert!(
            failures.is_empty(),
            "mode YAML tools lists reference unregistered tool names (get_tools_for_mode silently drops them): {failures:?}"
        );
    }

    #[tokio::test]
    async fn yaml_configs_design_workflows_load_and_reference_registered_tools() {
        let gcx = task_prompt_contract_gcx().await;
        let registry =
            crate::yaml_configs::customization_registry::load_registry_from_dir(&gcx.config_dir)
                .await;
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);

        let registered = get_available_tools(gcx)
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect::<HashSet<_>>();
        let design = registry
            .modes
            .get("design")
            .expect("design mode should load");
        let review = registry
            .subagents
            .get("design_review")
            .expect("design_review subagent should load");
        let visual_qa = registry
            .subagents
            .get("visual_qa")
            .expect("visual_qa subagent should load");

        for (id, tools) in [
            ("design", &design.tools),
            ("design_review", &review.tools),
            ("visual_qa", &visual_qa.tools),
        ] {
            let missing = tools
                .iter()
                .filter(|name| !registered.contains(*name))
                .cloned()
                .collect::<Vec<_>>();
            assert!(
                missing.is_empty(),
                "{id} references unregistered tools: {missing:?}"
            );
        }

        let review_prompt = review
            .messages
            .system_prompt
            .as_deref()
            .expect("design_review system prompt");
        for required in [
            "numeric evidence",
            "proposed fix",
            "scrollWidth",
            "clientWidth",
            "fenced `html` block",
            "capture_element_states",
            "capture_frames",
            "accessibility_snapshot",
            "page_context",
            "scanned 0 elements",
        ] {
            assert!(review_prompt.contains(required), "missing `{required}`");
        }

        let visual_qa_prompt = visual_qa
            .messages
            .system_prompt
            .as_deref()
            .expect("visual_qa system prompt");
        for required in [
            "Verdict: PASS",
            "Verdict: FAIL",
            "diff artifact",
            "console error",
            "page error",
            "layout shift",
            "capture_element_states",
            "capture_frames",
            "accessibility_snapshot",
            "page_context",
            "elements_scanned",
        ] {
            assert!(visual_qa_prompt.contains(required), "missing `{required}`");
        }
        assert!(
            visual_qa.tools.iter().any(|tool| tool == "contrast_audit"),
            "the visual_qa gate needs contrast_audit to enforce fail-closed contrast evidence"
        );

        let design_prompt = &design.prompt;
        for required in [
            "capture_element_states",
            "capture_frames",
            "accessibility_snapshot",
            "page_context",
            "scanned 0 elements",
            "verbatim",
        ] {
            assert!(design_prompt.contains(required), "missing `{required}`");
        }
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

    #[tokio::test]
    async fn design_prompt_chrome_examples_parse_through_the_real_browser_dialect() {
        let gcx = task_prompt_contract_gcx().await;
        let registry =
            crate::yaml_configs::customization_registry::load_registry_from_dir(&gcx.config_dir)
                .await;
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);

        let design = registry
            .modes
            .get("design")
            .expect("design mode should load");
        let prompts = [
            ("design", design.prompt.clone()),
            (
                "design_review",
                registry
                    .subagents
                    .get("design_review")
                    .and_then(|subagent| subagent.messages.system_prompt.clone())
                    .expect("design_review system prompt"),
            ),
            (
                "visual_qa",
                registry
                    .subagents
                    .get("visual_qa")
                    .and_then(|subagent| subagent.messages.system_prompt.clone())
                    .expect("visual_qa system prompt"),
            ),
        ];

        let mut parsed = 0usize;
        for (id, prompt) in &prompts {
            for example in extract_chrome_examples(prompt) {
                let value: serde_json::Value =
                    serde_json::from_str(&example).unwrap_or_else(|error| {
                        panic!("{id} example is not JSON: {example} ({error})")
                    });
                crate::integrations::browser_models::parse_browser_action_request(value)
                    .unwrap_or_else(|error| {
                        panic!(
                            "{id} example does not match the chrome dialect: {example} ({error})"
                        )
                    });
                assert!(
                    !example.contains("\"type\":"),
                    "{id} example uses the non-existent `type` step key: {example}"
                );
                parsed += 1;
            }
        }
        assert_eq!(
            parsed, 6,
            "expected every documented chrome example to be round-tripped, parsed {parsed}"
        );
    }

    #[test]
    fn browser_locators_only_parse_in_by_value_form() {
        let by_value = serde_json::json!({
            "steps":[{"action":"click","locator":{"by":"css","value":"#save"}}]
        });
        assert!(
            crate::integrations::browser_models::parse_browser_action_request(by_value).is_ok()
        );
        for wrong in [
            serde_json::json!({"steps":[{"type":"click","locator":{"by":"css","value":"#save"}}]}),
            serde_json::json!({"steps":[{"action":"click","locator":{"css":"#save"}}]}),
            serde_json::json!({"steps":[{"action":"click","locator":{"ref":"e14"}}]}),
        ] {
            assert!(
                crate::integrations::browser_models::parse_browser_action_request(wrong.clone())
                    .is_err(),
                "{wrong} must not parse; documenting it would teach an unusable dialect"
            );
        }
    }
}

pub async fn get_available_tool_groups(gcx: Arc<GlobalContext>) -> Vec<ToolGroup> {
    let mut tools_all = get_builtin_tools(gcx.clone()).await;
    tools_all.extend(get_integration_tools(gcx.clone()).await);

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

pub async fn get_tools_for_mode(
    gcx: Arc<GlobalContext>,
    mode_id: &str,
    model_id: Option<&str>,
) -> Vec<Box<dyn Tool + Send>> {
    use crate::yaml_configs::customization_registry::{get_mode_config, map_legacy_mode_to_id};
    use std::collections::HashSet;

    let mode_id = map_legacy_mode_to_id(mode_id);

    let mode_config = match get_mode_config(gcx.clone(), mode_id, model_id).await {
        Some(config) => config,
        None => {
            tracing::warn!("Mode '{}' not found, returning empty tools", mode_id);
            return vec![];
        }
    };

    if mode_config.tools.is_empty() {
        return vec![];
    }

    let allowed_tools: HashSet<&str> = mode_config.tools.iter().map(|s| s.as_str()).collect();

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

    let allow_integrations = mode_config.allow_integrations;
    let allow_mcp = mode_config.allow_mcp;
    let allow_subagents = mode_config.allow_subagents;

    let all_tool_groups: Vec<(ToolGroupCategory, Box<dyn Tool + Send>)> =
        get_available_tool_groups(gcx.clone())
            .await
            .into_iter()
            .flat_map(|g| {
                let cat = g.category;
                g.tools.into_iter().map(move |t| (cat, t))
            })
            .collect();

    let registered_tool_names: HashSet<String> = all_tool_groups
        .iter()
        .map(|(_, tool)| tool.tool_description().name)
        .collect();
    crate::yaml_configs::mode_validation::warn_unknown_mode_tools(
        gcx.clone(),
        mode_id,
        &mode_config.tools,
        &registered_tool_names,
    )
    .await;

    let all_tools: Vec<(ToolGroupCategory, Box<dyn Tool + Send>)> = all_tool_groups
        .into_iter()
        .filter(|(_, tool)| tool.config().unwrap_or_default().enabled)
        .filter(|(_, tool)| {
            if tool.tool_description().name == "web_search" && model_supports_web_search {
                return false;
            }
            true
        })
        .collect();

    let tool_order: HashMap<&str, usize> = mode_config
        .tools
        .iter()
        .enumerate()
        .map(|(i, name)| (name.as_str(), i))
        .collect();

    let mut result: Vec<Box<dyn Tool + Send>> = all_tools
        .into_iter()
        .filter(|(cat, tool)| match cat {
            ToolGroupCategory::Integration if allow_integrations => true,
            ToolGroupCategory::MCP if allow_mcp => true,
            ToolGroupCategory::ConfigSubagent if allow_subagents => true,
            _ => allowed_tools.contains(tool.tool_description().name.as_str()),
        })
        .map(|(_, tool)| tool)
        .collect();

    if let Some(mid) = model_id {
        let policy = tool_access_policy(gcx.clone()).await;
        if !policy.tool_access.providers.is_empty() {
            let provider = provider_of_model(mid).to_string();
            result.retain(|tool| mcp_tool_allowed(&policy, &provider, &tool.tool_description()));
        }
    }

    result.sort_by(|a, b| {
        let a_order = tool_order
            .get(a.tool_description().name.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        let b_order = tool_order
            .get(b.tool_description().name.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        a_order
            .cmp(&b_order)
            .then_with(|| a.tool_description().name.cmp(&b.tool_description().name))
    });

    result
}
