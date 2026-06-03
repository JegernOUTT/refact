use std::collections::HashMap;
use std::sync::Arc;

use crate::caps::resolve_chat_model;
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};
use crate::integrations::running_integrations::load_integrations;
use crate::yaml_configs::customization_registry::{
    get_project_registry, should_expose_subagent_as_config_tool,
};

use super::tools_description::{Tool, ToolGroup, ToolGroupCategory, ToolSourceType};
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

fn tool_available(
    tool: &Box<dyn Tool + Send>,
    ast_on: bool,
    vecdb_on: bool,
    is_there_a_thinking_model: bool,
    allow_knowledge: bool,
    allow_experimental: bool,
) -> bool {
    let dependencies = tool.tool_depends_on();
    if dependencies.contains(&"ast".to_string()) && !ast_on {
        return false;
    }
    if dependencies.contains(&"vecdb".to_string()) && !vecdb_on {
        return false;
    }
    if dependencies.contains(&"thinking".to_string()) && !is_there_a_thinking_model {
        return false;
    }
    if dependencies.contains(&"knowledge".to_string()) && !allow_knowledge {
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
    let (ast_on, vecdb_on, allow_experimental) = {
        let vecdb_on = gcx.vec_db.lock().await.is_some();
        let ast_on = gcx.ast_service.lock().unwrap().is_some();
        (ast_on, vecdb_on, gcx.cmdline.experimental)
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
            ast_on,
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
        Box::new(crate::tools::tool_search::ToolSearch {
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
    ];

    let system_tools = builtin_system_tools(config_path.clone());

    let deep_analysis_tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(
            crate::tools::tool_strategic_planning::ToolStrategicPlanning {
                config_path: config_path.clone(),
            },
        ),
        Box::new(crate::tools::tool_code_review::ToolCodeReview {
            config_path: config_path.clone(),
        }),
        Box::new(crate::tools::tool_deep_research::ToolDeepResearch {
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

    let conductor_tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(crate::tools::tool_conductor::ToolConductorSteerChat::new()),
        Box::new(crate::tools::tool_conductor::ToolConductorSpawnPlanner::new()),
        Box::new(crate::tools::tool_conductor::ToolConductorMemo::new()),
        Box::new(crate::tools::tool_conductor::ToolConductorSetAutonomy::new()),
        Box::new(crate::tools::tool_conductor::ToolConductorEscalate::new()),
        Box::new(crate::tools::tool_conductor::ToolConductorGoalStatus::new()),
        Box::new(crate::tools::tool_conductor::ToolConductorSay::new()),
        Box::new(crate::tools::tool_conductor::ToolConductorAsk::new()),
        Box::new(crate::tools::tool_conductor_trajectory::ToolConductorTrajectoryList::new()),
        Box::new(crate::tools::tool_conductor_trajectory::ToolConductorTrajectoryDigest::new()),
        Box::new(crate::tools::tool_conductor_trajectory::ToolConductorTrajectoryEdit::new()),
        Box::new(crate::tools::tool_conductor_trajectory::ToolConductorTrajectoryRemove::new()),
        Box::new(crate::tools::tool_conductor_trajectory::ToolConductorTrajectoryInsert::new()),
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
        Box::new(crate::tools::tool_swarm_investigate::ToolSwarmInvestigate {
            config_path: config_path.clone(),
        }),
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
            name: "Conductor".to_string(),
            description: "Buddy conductor goal orchestration tools".to_string(),
            category: ToolGroupCategory::Builtin,
            tools: conductor_tools,
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
    use std::sync::Arc;

    use crate::tools::tools_description::ToolSourceType;
    use crate::yaml_configs::customization_registry::RegistryCache;
    use crate::yaml_configs::customization_types::{
        ProjectRegistry, SubagentConfig, SubagentMessages, SubagentPrompts, SubchatConfig,
    };

    use super::*;

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
