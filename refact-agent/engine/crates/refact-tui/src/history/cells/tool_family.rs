use super::ToolCellType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolFamily {
    Shell,
    Process,
    Diff,
    Server,
    Mcp,
    WebSearch,
    WebFetch,
    CodeSearch,
    FileSearch,
    TreeSearch,
    DocumentationSearch,
    KnowledgeSearch,
    RequestInput,
    Unknown,
}

const EXACT_TOOL_FAMILIES: &[(&str, ToolFamily)] = &[
    ("shell", ToolFamily::Shell),
    ("clean_background_processes", ToolFamily::Process),
    ("shell_service", ToolFamily::Process),
    ("sleep", ToolFamily::Process),
    ("add_workspace_folder", ToolFamily::Process),
    ("worktree_merge", ToolFamily::Process),
    ("create_textdoc", ToolFamily::Diff),
    ("update_textdoc", ToolFamily::Diff),
    ("update_textdoc_regex", ToolFamily::Diff),
    ("update_textdoc_by_lines", ToolFamily::Diff),
    ("update_textdoc_anchored", ToolFamily::Diff),
    ("apply_patch", ToolFamily::Diff),
    ("undo_textdoc", ToolFamily::Diff),
    ("rm", ToolFamily::Diff),
    ("mv", ToolFamily::Diff),
    ("mcp_call", ToolFamily::Mcp),
    ("mcp_tool_search", ToolFamily::Mcp),
    ("web_search", ToolFamily::WebSearch),
    ("web", ToolFamily::WebFetch),
    ("chrome", ToolFamily::WebFetch),
    ("ui_probe", ToolFamily::WebFetch),
    ("mark_elements", ToolFamily::WebFetch),
    ("contrast_audit", ToolFamily::WebFetch),
    ("image_region", ToolFamily::WebFetch),
    ("visual_diff", ToolFamily::WebFetch),
    ("search_pattern", ToolFamily::CodeSearch),
    ("search_semantic", ToolFamily::CodeSearch),
    ("search_symbol_definition", ToolFamily::CodeSearch),
    ("codegraph_overview", ToolFamily::CodeSearch),
    ("code_health", ToolFamily::CodeSearch),
    ("git_risk", ToolFamily::CodeSearch),
    ("code_why", ToolFamily::CodeSearch),
    ("code_duplication", ToolFamily::CodeSearch),
    ("dead_code", ToolFamily::CodeSearch),
    ("security_scan", ToolFamily::CodeSearch),
    ("pr_blast", ToolFamily::CodeSearch),
    ("code_map", ToolFamily::CodeSearch),
    ("review", ToolFamily::CodeSearch),
    ("cat", ToolFamily::FileSearch),
    ("glob", ToolFamily::FileSearch),
    ("refact_engine_search", ToolFamily::CodeSearch),
    ("refact_engine_cat", ToolFamily::FileSearch),
    ("tree", ToolFamily::TreeSearch),
    ("design_system", ToolFamily::DocumentationSearch),
    ("ask_questions", ToolFamily::RequestInput),
    ("agent_ask_planner", ToolFamily::RequestInput),
    ("planner_reply", ToolFamily::RequestInput),
    ("subagent", ToolFamily::Server),
    ("delegate", ToolFamily::Server),
    ("tasks_set", ToolFamily::Server),
    ("task_done", ToolFamily::Server),
    ("buddy_say", ToolFamily::Server),
    ("buddy_render_controls", ToolFamily::Server),
    ("buddy_get_logs", ToolFamily::KnowledgeSearch),
    ("buddy_get_internal_context", ToolFamily::KnowledgeSearch),
    ("buddy_create_issue", ToolFamily::Server),
    ("buddy_open_view", ToolFamily::Server),
    ("buddy_open_setup_flow", ToolFamily::Server),
    ("buddy_create_draft", ToolFamily::Server),
    ("buddy_launch_investigation", ToolFamily::Server),
    ("buddy_log_activity", ToolFamily::Server),
    ("buddy_speak", ToolFamily::Server),
    ("buddy_runtime_event", ToolFamily::Server),
    ("buddy_user_pref_list", ToolFamily::Server),
    ("buddy_user_pref_upsert", ToolFamily::Server),
    ("buddy_user_pref_remove", ToolFamily::Server),
    ("refact_engine_clone", ToolFamily::Server),
    ("buddy_open_issue", ToolFamily::Server),
    ("set_trajectory_label", ToolFamily::Server),
    ("get_plan", ToolFamily::Server),
    ("set_plan", ToolFamily::Server),
    ("update_plan", ToolFamily::Server),
    ("get_goal", ToolFamily::Server),
    ("set_goal", ToolFamily::Server),
    ("update_goal", ToolFamily::Server),
    ("validate_goal", ToolFamily::Server),
    ("pause_goal", ToolFamily::Server),
    ("snooze_goal", ToolFamily::Server),
    ("compress_chat_probe", ToolFamily::Server),
    ("compress_chat_apply", ToolFamily::Server),
    ("handoff_to_mode", ToolFamily::Server),
    ("task_start", ToolFamily::Server),
    ("board_get", ToolFamily::Server),
    ("board_create", ToolFamily::Server),
    ("board_update", ToolFamily::Server),
    ("board_move", ToolFamily::Server),
    ("board_delete", ToolFamily::Server),
    ("ready_cards", ToolFamily::Server),
    ("board_create_batch", ToolFamily::Server),
    ("agent_update", ToolFamily::Server),
    ("agent_complete", ToolFamily::Server),
    ("agent_fail", ToolFamily::Server),
    ("assign_agent", ToolFamily::Server),
    ("spawn_agent", ToolFamily::Server),
    ("spawn_ab", ToolFamily::Server),
    ("pick_ab_winner", ToolFamily::Server),
    ("spawn_agents_batch", ToolFamily::Server),
    ("check_agents", ToolFamily::Server),
    ("task_overview", ToolFamily::Server),
    ("agent_diff", ToolFamily::Diff),
    ("agent_pulse", ToolFamily::Server),
    ("agent_chat_summary", ToolFamily::Server),
    ("agent_steer", ToolFamily::Server),
    ("task_questions_list", ToolFamily::Server),
    ("card_comment_add", ToolFamily::Server),
    ("card_comment_list", ToolFamily::Server),
    ("task_broadcast", ToolFamily::Server),
    ("cancel_agent", ToolFamily::Server),
    ("pause_agent", ToolFamily::Server),
    ("resume_agent", ToolFamily::Server),
    ("wait_agents", ToolFamily::Server),
    ("agent_finish", ToolFamily::Server),
    ("mark_done", ToolFamily::Server),
    ("mark_failed", ToolFamily::Server),
    ("mark_done_batch", ToolFamily::Server),
    ("mark_failed_batch", ToolFamily::Server),
    ("merge_agent", ToolFamily::Server),
    ("merge_ready_in_order", ToolFamily::Server),
    ("restart_agent", ToolFamily::Server),
    ("task_verify_card", ToolFamily::Server),
    ("agent_list", ToolFamily::Server),
    ("agent_status", ToolFamily::Server),
    ("agent_wait", ToolFamily::Server),
    ("agent_result", ToolFamily::Server),
    ("agent_cancel", ToolFamily::Server),
    ("activate_skill", ToolFamily::KnowledgeSearch),
    ("deactivate_skill", ToolFamily::KnowledgeSearch),
    ("knowledge", ToolFamily::KnowledgeSearch),
    ("create_knowledge", ToolFamily::KnowledgeSearch),
    ("get_trajectory_context", ToolFamily::KnowledgeSearch),
    ("search_trajectories", ToolFamily::KnowledgeSearch),
];

const PROVIDER_NATIVE_TOOL_NAMES: &[&str] = &[
    "web_search_call",
    "file_search_call",
    "code_interpreter_call",
    "local_shell_call",
    "image_generation_call",
    "computer_use_call",
    "web_fetch",
    "code_execution",
];

const PREFIX_TOOL_FAMILIES: &[(&str, ToolFamily)] = &[
    ("process_", ToolFamily::Process),
    ("cron_", ToolFamily::Process),
    ("doc_", ToolFamily::DocumentationSearch),
    ("task_mem_", ToolFamily::KnowledgeSearch),
    ("buddy_memory_", ToolFamily::KnowledgeSearch),
    ("mcp", ToolFamily::Mcp),
    ("subagent_", ToolFamily::Server),
    ("srvtoolu_", ToolFamily::Server),
];

pub fn tool_family(name: &str) -> ToolFamily {
    EXACT_TOOL_FAMILIES
        .iter()
        .find_map(|(tool_name, family)| (*tool_name == name).then_some(*family))
        .or_else(|| {
            PROVIDER_NATIVE_TOOL_NAMES
                .contains(&name)
                .then_some(ToolFamily::Server)
        })
        .or_else(|| {
            PREFIX_TOOL_FAMILIES
                .iter()
                .find_map(|(prefix, family)| name.starts_with(prefix).then_some(*family))
        })
        .unwrap_or(ToolFamily::Unknown)
}

impl ToolFamily {
    pub fn cell_type(self) -> ToolCellType {
        match self {
            Self::Shell | Self::Process => ToolCellType::Exec,
            Self::Diff => ToolCellType::Diff,
            Self::Server | Self::Mcp => ToolCellType::Server,
            Self::WebSearch
            | Self::WebFetch
            | Self::CodeSearch
            | Self::FileSearch
            | Self::TreeSearch
            | Self::DocumentationSearch
            | Self::KnowledgeSearch => ToolCellType::Search,
            Self::RequestInput => ToolCellType::RequestInput,
            Self::Unknown => ToolCellType::Generic,
        }
    }

    pub fn search_header(self, active: bool) -> &'static str {
        match (self, active) {
            (Self::WebSearch, true) => "Searching the web",
            (Self::WebSearch, false) => "Searched the web",
            (Self::WebFetch, true) => "Fetching the web",
            (Self::WebFetch, false) => "Fetched the web",
            (Self::CodeSearch, true) => "Searching code",
            (Self::CodeSearch, false) => "Searched code",
            (Self::FileSearch, true) => "Searching files",
            (Self::FileSearch, false) => "Searched files",
            (Self::TreeSearch, true) => "Inspecting the file tree",
            (Self::TreeSearch, false) => "Inspected the file tree",
            (Self::DocumentationSearch, true) => "Searching documentation",
            (Self::DocumentationSearch, false) => "Searched documentation",
            (Self::KnowledgeSearch, true) => "Searching knowledge",
            (Self::KnowledgeSearch, false) => "Searched knowledge",
            (_, true) => "Searching",
            (_, false) => "Searched",
        }
    }

    pub fn is_shell(self) -> bool {
        self == Self::Shell
    }

    pub fn is_mcp(self) -> bool {
        self == Self::Mcp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATIC_ENGINE_TOOL_NAMES: &[&str] = &[
        "shell",
        "process_start",
        "process_list",
        "process_read",
        "process_kill",
        "clean_background_processes",
        "process_wait",
        "process_subscribe",
        "process_write_stdin",
        "shell_service",
        "sleep",
        "cron_create",
        "cron_list",
        "cron_delete",
        "cron_update",
        "add_workspace_folder",
        "search_symbol_definition",
        "tree",
        "cat",
        "search_pattern",
        "glob",
        "search_semantic",
        "codegraph_overview",
        "code_health",
        "git_risk",
        "code_why",
        "code_duplication",
        "dead_code",
        "security_scan",
        "pr_blast",
        "code_map",
        "design_system",
        "create_textdoc",
        "update_textdoc",
        "update_textdoc_by_lines",
        "update_textdoc_regex",
        "update_textdoc_anchored",
        "apply_patch",
        "undo_textdoc",
        "rm",
        "mv",
        "web",
        "web_search",
        "chrome",
        "ui_probe",
        "mark_elements",
        "contrast_audit",
        "image_region",
        "visual_diff",
        "review",
        "subagent",
        "delegate",
        "tasks_set",
        "activate_skill",
        "deactivate_skill",
        "knowledge",
        "create_knowledge",
        "get_trajectory_context",
        "search_trajectories",
        "task_done",
        "ask_questions",
        "buddy_say",
        "buddy_render_controls",
        "buddy_get_logs",
        "buddy_get_internal_context",
        "buddy_create_issue",
        "buddy_open_view",
        "buddy_open_setup_flow",
        "buddy_create_draft",
        "buddy_launch_investigation",
        "buddy_log_activity",
        "buddy_speak",
        "buddy_runtime_event",
        "buddy_memory_search",
        "buddy_memory_create",
        "buddy_memory_archive",
        "buddy_memory_retag",
        "buddy_memory_merge",
        "buddy_user_pref_list",
        "buddy_user_pref_upsert",
        "buddy_user_pref_remove",
        "refact_engine_clone",
        "refact_engine_search",
        "refact_engine_cat",
        "buddy_open_issue",
        "set_trajectory_label",
        "get_plan",
        "set_plan",
        "update_plan",
        "get_goal",
        "set_goal",
        "update_goal",
        "validate_goal",
        "pause_goal",
        "snooze_goal",
        "compress_chat_probe",
        "compress_chat_apply",
        "handoff_to_mode",
        "task_start",
        "board_get",
        "board_create",
        "board_update",
        "board_move",
        "board_delete",
        "ready_cards",
        "board_create_batch",
        "agent_update",
        "agent_complete",
        "agent_fail",
        "assign_agent",
        "spawn_agent",
        "spawn_ab",
        "pick_ab_winner",
        "spawn_agents_batch",
        "check_agents",
        "task_overview",
        "agent_diff",
        "agent_pulse",
        "agent_chat_summary",
        "agent_steer",
        "agent_ask_planner",
        "planner_reply",
        "task_questions_list",
        "card_comment_add",
        "card_comment_list",
        "task_broadcast",
        "cancel_agent",
        "pause_agent",
        "resume_agent",
        "wait_agents",
        "agent_finish",
        "mark_done",
        "mark_failed",
        "mark_done_batch",
        "mark_failed_batch",
        "merge_agent",
        "merge_ready_in_order",
        "restart_agent",
        "task_verify_card",
        "doc_list",
        "doc_get",
        "doc_create",
        "doc_update",
        "doc_append",
        "doc_delete",
        "doc_pin",
        "doc_history",
        "task_mem_save",
        "task_mem_get",
        "task_mem_search",
        "task_mem_pin",
        "task_mem_archive",
        "task_mem_unarchive",
        "task_mem_inbox",
        "task_mem_triage_done",
        "agent_list",
        "agent_status",
        "agent_wait",
        "agent_result",
        "agent_cancel",
        "worktree_merge",
        "mcp_call",
    ];

    const LAZY_MCP_TOOL_NAMES: &[&str] = &["mcp_tool_search", "mcp_call"];

    #[test]
    fn static_engine_tool_registry_has_no_unknown_families() {
        let registered = STATIC_ENGINE_TOOL_NAMES
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(STATIC_ENGINE_TOOL_NAMES.len(), 162);
        assert_eq!(STATIC_ENGINE_TOOL_NAMES.len(), registered.len());
        let unknown = STATIC_ENGINE_TOOL_NAMES
            .iter()
            .filter(|name| tool_family(name) == ToolFamily::Unknown)
            .collect::<Vec<_>>();

        assert!(
            unknown.is_empty(),
            "unclassified static engine tools: {unknown:?}"
        );
    }

    #[test]
    fn retired_aliases_are_not_registered() {
        for alias in [
            "patch",
            "text_edit",
            "replace_textdoc",
            "search",
            "vecdb_search",
            "request_user_input",
            "request-user-input",
        ] {
            assert!(
                !EXACT_TOOL_FAMILIES.iter().any(|(name, _)| *name == alias),
                "retired alias remains registered: {alias}"
            );
            assert_eq!(tool_family(alias), ToolFamily::Unknown, "{alias}");
        }
    }

    #[test]
    fn provider_native_tools_are_explicitly_allowlisted() {
        assert_eq!(PROVIDER_NATIVE_TOOL_NAMES.len(), 8);
        for name in PROVIDER_NATIVE_TOOL_NAMES {
            assert_eq!(tool_family(name), ToolFamily::Server, "{name}");
        }
    }

    #[test]
    fn dynamic_tool_conventions_are_classified() {
        for name in LAZY_MCP_TOOL_NAMES {
            assert_eq!(tool_family(name), ToolFamily::Mcp, "{name}");
        }
        assert_eq!(tool_family("mcp_github_get_issue"), ToolFamily::Mcp);
        assert_eq!(tool_family("mcp-server-query"), ToolFamily::Mcp);
        assert_eq!(
            tool_family("subagent_project_researcher"),
            ToolFamily::Server
        );
        assert_eq!(tool_family("srvtoolu_123"), ToolFamily::Server);
    }
}
