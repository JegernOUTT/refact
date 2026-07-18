use at_tools::handle_v1_post_tools;
use axum::Router;
use axum::routing::{get, post, put, patch, delete};

use crate::app_state::AppState;
use crate::http::utils::request_logging_middleware;
use crate::http::routers::v1::code_completion::{
    handle_v1_code_completion_web, handle_v1_code_completion_prompt,
};
use crate::http::routers::v1::code_lens::handle_v1_code_lens;
use crate::http::routers::v1::code_intel::{
    handle_v1_code_intel_communities, handle_v1_code_intel_dead_code,
    handle_v1_code_intel_duplication, handle_v1_code_intel_git_risk, handle_v1_code_intel_graph,
    handle_v1_code_intel_health, handle_v1_code_intel_overview, handle_v1_code_intel_pr_blast,
    handle_v1_code_intel_security_scan,
};
use crate::http::routers::v1::ast::{
    handle_v1_ast_file_dump, handle_v1_ast_file_symbols, handle_v1_ast_status,
};
use crate::http::routers::v1::at_commands::{
    handle_v1_command_completion, handle_v1_command_preview, handle_v1_at_command_execute,
    handle_v1_slash_commands,
};
use crate::http::routers::v1::at_tools::{
    handle_v1_get_tools, handle_v1_tools_check_if_confirmation_needed, handle_v1_tools_execute,
};
use crate::http::routers::v1::caps::{
    handle_v1_caps, handle_v1_ping, handle_v1_model_capabilities, handle_v1_model_supported,
};

use crate::http::routers::v1::chat_based_handlers::{
    handle_v1_commit_message_from_diff, handle_v1_trajectory_compress,
};
use crate::http::routers::v1::git::{
    handle_v1_git_commit, handle_v1_checkpoints_preview, handle_v1_checkpoints_restore,
};
use crate::http::routers::v1::git_read::{
    handle_v1_git_branches, handle_v1_git_diff, handle_v1_git_log, handle_v1_git_stage,
    handle_v1_git_status, handle_v1_git_unstage,
};
use crate::http::routers::v1::graceful_shutdown::handle_v1_graceful_shutdown;
use crate::http::routers::v1::hooks::handle_v1_hooks_fire;
use crate::http::routers::v1::links::handle_v1_links;
use crate::http::routers::v1::lsp_like_handlers::{
    handle_v1_lsp_did_change, handle_v1_lsp_add_folder, handle_v1_lsp_initialize,
    handle_v1_lsp_remove_folder, handle_v1_set_active_document, handle_v1_git_branch_changed,
};
use crate::http::routers::v1::status::{handle_v1_codegraph_status, handle_v1_rag_status};
use crate::http::routers::v1::customization::handle_v1_customization;
use crate::http::routers::v1::customization::handle_v1_config_path;
use crate::http::routers::v1::gui_help_handlers::handle_v1_fullpath;
use crate::http::routers::v1::sync_files::handle_v1_sync_files_extract_tar;
use crate::http::routers::v1::system_prompt::handle_v1_prepend_system_prompt_and_maybe_more_initial_messages;
use crate::providers::http::{
    handle_v1_defaults_get, handle_v1_defaults_update, handle_v1_google_gemini_health,
    handle_v1_models, handle_v1_openrouter_account_info, handle_v1_openrouter_health,
    handle_v1_openrouter_model_endpoints, handle_v1_provider_account_info,
    handle_v1_provider_add_custom_model, handle_v1_provider_available_models,
    handle_v1_provider_delete, handle_v1_provider_get, handle_v1_provider_health,
    handle_v1_provider_model_provider_update, handle_v1_provider_model_toggle,
    handle_v1_provider_models, handle_v1_provider_oauth_callback,
    handle_v1_provider_oauth_exchange, handle_v1_provider_oauth_logout,
    handle_v1_provider_oauth_start, handle_v1_provider_remove_custom_model,
    handle_v1_provider_remove_custom_model_post, handle_v1_provider_schema,
    handle_v1_provider_update, handle_v1_provider_usage, handle_v1_provider_usage_redeem,
    handle_v1_providers_list,
};

use crate::http::routers::v1::vecdb::{
    handle_v1_codegraph_search, handle_v1_vecdb_search, handle_v1_vecdb_status,
};
use crate::http::routers::v1::knowledge_graph::handle_v1_knowledge_graph;
use crate::http::routers::v1::knowledge_ops::{
    handle_v1_knowledge_update_memory, handle_v1_knowledge_delete_memory,
    handle_v1_knowledge_relink_memories,
};
use crate::http::routers::v1::v1_integrations::{
    handle_v1_integration_get, handle_v1_integration_icon, handle_v1_integration_save,
    handle_v1_integration_delete, handle_v1_integrations, handle_v1_integrations_filtered,
    handle_v1_integrations_mcp_logs,
};
use crate::http::routers::v1::file_edit_tools::handle_v1_file_edit_tool_dry_run;
use crate::http::routers::v1::files::{handle_v1_files_read, handle_v1_files_tree};
use crate::http::routers::v1::code_edit::handle_v1_code_edit;
use crate::http::routers::v1::workspace::handle_v1_get_app_searchable_id;
use crate::chat::{
    handle_v1_chat_subscribe, handle_v1_chat_command, handle_v1_chat_cancel_queued,
    handle_v1_trajectories_list, handle_v1_trajectories_all, handle_v1_trajectories_get,
    handle_v1_trajectory_path, handle_v1_trajectories_save, handle_v1_trajectories_delete,
    handle_v1_trajectories_subscribe,
};
use crate::http::routers::v1::voice::{
    handle_v1_voice_transcribe, handle_v1_voice_download, handle_v1_voice_status,
    handle_v1_voice_stream_subscribe, handle_v1_voice_stream_chunk,
};
use crate::http::routers::v1::tasks::{
    handle_list_tasks, handle_create_task, handle_get_task, handle_delete_task, handle_get_board,
    handle_create_card_comment, handle_patch_board, handle_get_planner_instructions,
    handle_set_planner_instructions, handle_get_ready_cards, handle_update_task_status,
    handle_update_task_meta, handle_list_task_trajectories, handle_create_planner_chat,
    handle_delete_planner_chat, handle_tasks_subscribe, handle_list_task_memories,
    handle_pin_task_memory, handle_archive_task_memory, handle_task_memories_triage_done,
    handle_task_memory_facets, handle_list_task_documents, handle_get_task_document,
    handle_create_task_document, handle_update_task_document, handle_append_task_document,
    handle_delete_task_document, handle_pin_task_document, handle_history_task_document,
};
use crate::http::routers::v1::trajectory_ops::{
    handle_transform_preview, handle_transform_apply, handle_handoff_preview, handle_handoff_apply,
    handle_mode_transition_apply, handle_planner_from_transition,
};
use crate::http::routers::v1::project_configs::{
    handle_v1_project_configs_get, handle_v1_project_configs_rescan,
    handle_v1_project_configs_bootstrap,
};
use crate::http::routers::v1::scheduler::{
    handle_v1_scheduler_cron_delete, handle_v1_scheduler_cron_get, handle_v1_scheduler_cron_patch,
    handle_v1_scheduler_cron_post, handle_v1_scheduler_cron_run,
};
use crate::http::routers::v1::worktrees::{
    handle_v1_worktrees_cleanup, handle_v1_worktrees_cleanup_dry_run, handle_v1_worktrees_create,
    handle_v1_worktrees_delete, handle_v1_worktrees_diff, handle_v1_worktrees_get,
    handle_v1_worktrees_list, handle_v1_worktrees_merge, handle_v1_worktrees_open,
    handle_v1_worktrees_summary,
};

mod ast;
pub mod at_commands;
pub mod at_tools;
pub mod buddy;
pub mod buddy_drafts;
pub mod buddy_frontend_error;
pub mod buddy_opportunities;
pub mod buddy_pulse;
pub mod bug_report;
pub mod caps;
pub mod chat_based_handlers;
mod chat_modes;
pub mod code_completion;
mod code_edit;
pub mod code_intel;
pub mod code_lens;
mod commands_marketplace;
pub mod customization;
mod customization_editor;
pub mod exec;
pub mod ext_management;
mod ext_marketplace_sources;
mod file_edit_tools;
mod files;
mod git;
mod git_read;
pub mod graceful_shutdown;
mod gui_help_handlers;
mod hooks;
pub mod knowledge_enrichment;
mod knowledge_graph;
pub mod knowledge_ops;
pub mod links;
pub mod lsp_like_handlers;
mod mcp_config_sharing;
mod mcp_interactions;
mod mcp_marketplace;
mod mcp_marketplace_sources;
mod mcp_oauth;
mod mcp_server_info;
mod plugins;
mod project_configs;
pub mod project_information;
pub mod scheduler;
mod setup_status;
pub mod sidebar;
mod skills_marketplace;
mod skills_status;
mod stats;
pub mod status;
mod subagents_marketplace;
pub mod sync_files;
pub mod system_prompt;
pub mod tasks;
mod trajectory_ops;
mod v1_browser;
mod v1_integrations;
pub mod vecdb;
pub mod voice;
mod workspace;
mod worktrees;

use crate::http::routers::v1::ext_management::{
    handle_v1_ext_registry, handle_v1_ext_skill_get, handle_v1_ext_skill_put,
    handle_v1_ext_skill_post, handle_v1_ext_skill_delete, handle_v1_ext_command_get,
    handle_v1_ext_command_put, handle_v1_ext_command_post, handle_v1_ext_command_delete,
    handle_v1_ext_hooks_get, handle_v1_ext_hooks_put, handle_v1_ext_hooks_delete_by_index,
    handle_v1_ext_competitor_import_get, handle_v1_ext_competitor_import_post,
};
use crate::http::routers::v1::chat_modes::handle_v1_chat_modes;
use crate::http::routers::v1::customization_editor::{
    handle_v1_customization_registry, handle_v1_customization_get, handle_v1_customization_save,
    handle_v1_customization_create, handle_v1_customization_delete,
};
use crate::http::routers::v1::project_information::{
    handle_v1_project_information_get, handle_v1_project_information_save,
    handle_v1_project_information_preview,
};
use crate::http::routers::v1::stats::{handle_v1_stats_llm_summary, handle_v1_stats_llm_events};
use crate::http::routers::v1::plugins::{
    handle_list_marketplaces, handle_add_marketplace, handle_delete_marketplace,
    handle_list_marketplace_plugins, handle_install_plugin, handle_list_installed,
    handle_uninstall_plugin,
};
use crate::http::routers::v1::ext_marketplace_sources::{
    handle_v1_ext_marketplace_sources_get, handle_v1_ext_marketplace_sources_post,
    handle_v1_ext_marketplace_sources_delete, handle_v1_ext_marketplace_sources_configure,
    handle_v1_ext_marketplace_sources_refresh,
};
use crate::http::routers::v1::skills_marketplace::{
    handle_v1_skills_marketplace_get, handle_v1_skills_marketplace_install,
};
use crate::http::routers::v1::commands_marketplace::{
    handle_v1_commands_marketplace_get, handle_v1_commands_marketplace_install,
};
use crate::http::routers::v1::subagents_marketplace::{
    handle_v1_subagents_marketplace_get, handle_v1_subagents_marketplace_install,
};
use crate::http::routers::v1::skills_status::handle_v1_skills_status;
use crate::http::routers::v1::knowledge_enrichment::handle_v1_memory_enrichment_preview;
use crate::http::routers::v1::mcp_server_info::{
    handle_v1_mcp_server_info, handle_v1_mcp_server_reconnect,
};
use crate::http::routers::v1::setup_status::handle_v1_setup_status;

use crate::http::routers::v1::mcp_marketplace::{
    handle_v1_mcp_marketplace_get, handle_v1_mcp_marketplace_install,
    handle_v1_mcp_marketplace_installed, handle_v1_mcp_auto_name,
};
use crate::http::routers::v1::mcp_marketplace_sources::{
    handle_v1_mcp_marketplace_sources_get, handle_v1_mcp_marketplace_sources_post,
    handle_v1_mcp_marketplace_sources_delete, handle_v1_mcp_marketplace_sources_configure,
};
use crate::http::routers::v1::mcp_config_sharing::{
    handle_v1_mcp_export, handle_v1_mcp_import, handle_v1_mcp_project_config,
};
use crate::http::routers::v1::mcp_oauth::{
    handle_v1_mcp_oauth_start, handle_v1_mcp_oauth_exchange, handle_v1_mcp_oauth_callback,
    handle_v1_mcp_oauth_logout, handle_v1_mcp_oauth_status, handle_v1_mcp_oauth_cancel,
};
use crate::http::routers::v1::mcp_interactions::{
    handle_v1_mcp_interactions_list, handle_v1_mcp_interactions_respond,
};
use crate::http::routers::v1::v1_browser::{
    handle_browser_start, handle_browser_stop, handle_browser_screenshot, handle_browser_context,
    handle_browser_context_commit, handle_browser_element_pick, handle_browser_element_pick_result,
    handle_browser_curl, handle_browser_eval, handle_browser_inject_css, handle_browser_remove_css,
    handle_browser_dom_snapshot, handle_browser_accessibility, handle_browser_record_animation,
    handle_browser_handoff, handle_browser_status, handle_browser_annotate_start,
    handle_browser_annotate_result, handle_browser_annotate_clear, handle_browser_action,
};

pub fn make_v1_router(app_state: AppState) -> Router<AppState> {
    let builder = Router::new()
        .route("/ping", get(handle_v1_ping))
        .route(
            "/graceful-shutdown",
            get(handle_v1_graceful_shutdown).post(handle_v1_graceful_shutdown),
        )
        .route("/code-completion", post(handle_v1_code_completion_web))
        .route("/code-lens", post(handle_v1_code_lens))
        .route("/caps", get(handle_v1_caps))
        .route("/build_info", get(crate::http::routers::info::handle_info))
        .route(
            "/bug-report/context",
            get(bug_report::handle_v1_bug_report_context),
        )
        .route(
            "/bug-report/logs",
            get(bug_report::handle_v1_bug_report_logs),
        )
        .route(
            "/bug-report/errors",
            get(bug_report::handle_v1_bug_report_errors),
        )
        .route(
            "/bug-report/bundle",
            post(bug_report::handle_v1_bug_report_bundle),
        )
        .route("/model-capabilities", get(handle_v1_model_capabilities))
        .route("/model-supported", get(handle_v1_model_supported))
        .route("/tools", get(handle_v1_get_tools))
        .route("/tools", post(handle_v1_post_tools))
        .route(
            "/tools-check-if-confirmation-needed",
            post(handle_v1_tools_check_if_confirmation_needed),
        )
        .route("/tools-execute", post(handle_v1_tools_execute))
        .route("/lsp-initialize", post(handle_v1_lsp_initialize))
        .route("/lsp-did-changed", post(handle_v1_lsp_did_change))
        .route("/lsp-add-folder", post(handle_v1_lsp_add_folder))
        .route("/lsp-remove-folder", post(handle_v1_lsp_remove_folder))
        .route("/git-branch-changed", post(handle_v1_git_branch_changed))
        .route(
            "/lsp-set-active-document",
            post(handle_v1_set_active_document),
        )
        .route("/ast-file-symbols", post(handle_v1_ast_file_symbols))
        .route("/ast-file-dump", post(handle_v1_ast_file_dump))
        .route("/ast-status", get(handle_v1_ast_status))
        .route("/rag-status", get(handle_v1_rag_status))
        .route("/config-path", get(handle_v1_config_path))
        .route("/customization", get(handle_v1_customization))
        .route("/project-configs", get(handle_v1_project_configs_get))
        .route(
            "/project-configs/rescan",
            post(handle_v1_project_configs_rescan),
        )
        .route(
            "/project-configs/bootstrap",
            post(handle_v1_project_configs_bootstrap),
        )
        .route("/worktrees", get(handle_v1_worktrees_list))
        .route("/worktrees", post(handle_v1_worktrees_create))
        .route("/worktrees/summary", get(handle_v1_worktrees_summary))
        .route(
            "/worktrees/cleanup-dry-run",
            post(handle_v1_worktrees_cleanup_dry_run),
        )
        .route("/worktrees/cleanup", post(handle_v1_worktrees_cleanup))
        .route("/worktrees/:id", get(handle_v1_worktrees_get))
        .route("/worktrees/:id", delete(handle_v1_worktrees_delete))
        .route("/worktrees/:id/diff", get(handle_v1_worktrees_diff))
        .route("/worktrees/:id/merge", post(handle_v1_worktrees_merge))
        .route("/worktrees/:id/open", post(handle_v1_worktrees_open))
        .route("/chat-modes", get(handle_v1_chat_modes))
        .route(
            "/customization/registry",
            get(handle_v1_customization_registry),
        )
        .route("/customization/:kind/:id", get(handle_v1_customization_get))
        .route(
            "/customization/:kind/:id",
            put(handle_v1_customization_save),
        )
        .route("/customization/:kind", post(handle_v1_customization_create))
        .route(
            "/customization/:kind/:id",
            delete(handle_v1_customization_delete),
        )
        .route(
            "/sync-files-extract-tar",
            post(handle_v1_sync_files_extract_tar),
        )
        .route("/git-commit", post(handle_v1_git_commit))
        .route("/git/status", get(handle_v1_git_status))
        .route("/git/diff", get(handle_v1_git_diff))
        .route("/git/log", get(handle_v1_git_log))
        .route("/git/branches", get(handle_v1_git_branches))
        .route("/git/stage", post(handle_v1_git_stage))
        .route("/git/unstage", post(handle_v1_git_unstage))
        .route(
            "/prepend-system-prompt-and-maybe-more-initial-messages",
            post(handle_v1_prepend_system_prompt_and_maybe_more_initial_messages),
        )
        .route("/at-command-completion", post(handle_v1_command_completion))
        .route("/at-command-preview", post(handle_v1_command_preview))
        .route("/at-command-execute", post(handle_v1_at_command_execute))
        .route("/slash-commands", get(handle_v1_slash_commands))
        .route("/fullpath", post(handle_v1_fullpath))
        .route("/integrations", get(handle_v1_integrations))
        .route(
            "/integrations-filtered/:integr_name",
            get(handle_v1_integrations_filtered),
        )
        .route("/integration-get", post(handle_v1_integration_get))
        .route("/integration-save", post(handle_v1_integration_save))
        .route("/integration-delete", delete(handle_v1_integration_delete))
        .route(
            "/integration-icon/:icon_name",
            get(handle_v1_integration_icon),
        )
        .route(
            "/integrations-mcp-logs",
            post(handle_v1_integrations_mcp_logs),
        )
        .route("/checkpoints-preview", post(handle_v1_checkpoints_preview))
        .route("/checkpoints-restore", post(handle_v1_checkpoints_restore))
        .route("/links", post(handle_v1_links))
        .route(
            "/file_edit_tool_dry_run",
            post(handle_v1_file_edit_tool_dry_run),
        )
        .route("/files/tree", get(handle_v1_files_tree))
        .route("/files/read", get(handle_v1_files_read))
        .route("/code-edit", post(handle_v1_code_edit))
        .route("/models", get(handle_v1_models))
        .route("/providers", get(handle_v1_providers_list))
        .route("/providers/:name", get(handle_v1_provider_get))
        .route("/providers/:name", post(handle_v1_provider_update))
        .route("/providers/:name", delete(handle_v1_provider_delete))
        .route("/providers/:name/schema", get(handle_v1_provider_schema))
        .route("/providers/:name/models", get(handle_v1_provider_models))
        .route(
            "/providers/:name/available-models",
            get(handle_v1_provider_available_models),
        )
        .route(
            "/providers/:name/models/:model_id/endpoints",
            get(handle_v1_openrouter_model_endpoints),
        )
        .route(
            "/providers/:name/models/toggle",
            post(handle_v1_provider_model_toggle),
        )
        .route(
            "/providers/:name/models/provider",
            post(handle_v1_provider_model_provider_update),
        )
        .route(
            "/providers/:name/custom-models",
            post(handle_v1_provider_add_custom_model),
        )
        .route(
            "/providers/:name/custom-models",
            delete(handle_v1_provider_remove_custom_model),
        )
        .route(
            "/providers/:name/custom-models/remove",
            post(handle_v1_provider_remove_custom_model_post),
        )
        .route(
            "/openrouter/account-info",
            get(handle_v1_openrouter_account_info),
        )
        .route(
            "/providers/:name/account-info",
            get(handle_v1_provider_account_info),
        )
        .route("/providers/:name/health", get(handle_v1_provider_health))
        .route("/providers/:name/usage", get(handle_v1_provider_usage))
        .route(
            "/providers/:name/usage/redeem",
            post(handle_v1_provider_usage_redeem),
        )
        .route(
            "/providers/:name/oauth/start",
            post(handle_v1_provider_oauth_start),
        )
        .route(
            "/providers/:name/oauth/exchange",
            post(handle_v1_provider_oauth_exchange),
        )
        .route(
            "/providers/:name/oauth/logout",
            post(handle_v1_provider_oauth_logout),
        )
        .route(
            "/providers/:name/oauth/callback",
            get(handle_v1_provider_oauth_callback),
        )
        .route("/defaults", get(handle_v1_defaults_get))
        .route("/defaults", post(handle_v1_defaults_update))
        .route("/openrouter/health", get(handle_v1_openrouter_health))
        .route("/google-gemini/health", get(handle_v1_google_gemini_health))
        .route(
            "/get-app-searchable-id",
            get(handle_v1_get_app_searchable_id),
        )
        // experimental
        .route(
            "/code-completion-prompt",
            post(handle_v1_code_completion_prompt),
        )
        .route(
            "/commit-message-from-diff",
            post(handle_v1_commit_message_from_diff),
        );
    let builder = builder
        .route("/vdb-search", post(handle_v1_vecdb_search))
        .route("/vdb-status", get(handle_v1_vecdb_status))
        .route("/codegraph-search", post(handle_v1_codegraph_search))
        .route("/codegraph-status", get(handle_v1_codegraph_status))
        .route("/code-intel/overview", get(handle_v1_code_intel_overview))
        .route("/code-intel/graph", get(handle_v1_code_intel_graph))
        .route(
            "/code-intel/communities",
            get(handle_v1_code_intel_communities),
        )
        .route("/code-intel/dead-code", get(handle_v1_code_intel_dead_code))
        .route("/code-intel/health", get(handle_v1_code_intel_health))
        .route("/code-intel/git-risk", get(handle_v1_code_intel_git_risk))
        .route(
            "/code-intel/duplication",
            get(handle_v1_code_intel_duplication),
        )
        .route("/code-intel/pr-blast", post(handle_v1_code_intel_pr_blast))
        .route(
            "/code-intel/security-scan",
            post(handle_v1_code_intel_security_scan),
        )
        .route("/knowledge-graph", get(handle_v1_knowledge_graph))
        .route(
            "/knowledge/update-memory",
            post(handle_v1_knowledge_update_memory),
        )
        .route(
            "/knowledge/delete-memory",
            post(handle_v1_knowledge_delete_memory),
        )
        .route(
            "/knowledge/relink-memories",
            post(handle_v1_knowledge_relink_memories),
        )
        .route("/trajectory-compress", post(handle_v1_trajectory_compress))
        .route("/trajectories", get(handle_v1_trajectories_list))
        .route("/trajectories/all", get(handle_v1_trajectories_all))
        .route(
            "/trajectories/subscribe",
            get(handle_v1_trajectories_subscribe),
        )
        .route("/trajectories/:id", get(handle_v1_trajectories_get))
        .route("/trajectories/:id/path", get(handle_v1_trajectory_path))
        .route("/trajectories/:id", put(handle_v1_trajectories_save))
        .route("/trajectories/:id", delete(handle_v1_trajectories_delete))
        .route("/chats/subscribe", get(handle_v1_chat_subscribe))
        .route("/chats/:chat_id/commands", post(handle_v1_chat_command))
        .route(
            "/chats/:chat_id/skills-status",
            get(handle_v1_skills_status),
        )
        .route(
            "/chats/:chat_id/memory-enrichment/preview",
            post(handle_v1_memory_enrichment_preview),
        )
        .route(
            "/chats/:chat_id/queue/:client_request_id",
            delete(handle_v1_chat_cancel_queued),
        )
        .route("/voice/transcribe", post(handle_v1_voice_transcribe))
        .route("/voice/download", post(handle_v1_voice_download))
        .route("/voice/status", get(handle_v1_voice_status))
        .route(
            "/voice/stream/:session_id/subscribe",
            get(handle_v1_voice_stream_subscribe),
        )
        .route(
            "/voice/stream/:session_id/chunk",
            post(handle_v1_voice_stream_chunk),
        )
        .route("/sidebar/subscribe", get(sidebar::handle_sidebar_subscribe))
        .route("/tasks", get(handle_list_tasks))
        .route("/tasks", post(handle_create_task))
        .route("/tasks/subscribe", get(handle_tasks_subscribe))
        .route("/tasks/:task_id", get(handle_get_task))
        .route("/tasks/:task_id", delete(handle_delete_task))
        .route("/tasks/:task_id/status", post(handle_update_task_status))
        .route("/tasks/:task_id/meta", patch(handle_update_task_meta))
        .route("/task/:task_id/memories", get(handle_list_task_memories))
        .route(
            "/task/:task_id/memories/facets",
            get(handle_task_memory_facets),
        )
        .route(
            "/task/:task_id/memories/triage-done",
            post(handle_task_memories_triage_done),
        )
        .route(
            "/task/:task_id/memories/:filename/pin",
            post(handle_pin_task_memory),
        )
        .route(
            "/task/:task_id/memories/:filename/archive",
            post(handle_archive_task_memory),
        )
        .route("/task/:task_id/documents", get(handle_list_task_documents))
        .route(
            "/task/:task_id/documents",
            post(handle_create_task_document),
        )
        .route(
            "/task/:task_id/documents/:slug",
            get(handle_get_task_document),
        )
        .route(
            "/task/:task_id/documents/:slug",
            put(handle_update_task_document),
        )
        .route(
            "/task/:task_id/documents/:slug",
            delete(handle_delete_task_document),
        )
        .route(
            "/task/:task_id/documents/:slug/append",
            post(handle_append_task_document),
        )
        .route(
            "/task/:task_id/documents/:slug/pin",
            post(handle_pin_task_document),
        )
        .route(
            "/task/:task_id/documents/:slug/history",
            get(handle_history_task_document),
        )
        .route("/tasks/:task_id/board", get(handle_get_board))
        .route("/tasks/:task_id/board", post(handle_patch_board))
        .route(
            "/tasks/:task_id/cards/:card_id/comments",
            post(handle_create_card_comment),
        )
        .route("/tasks/:task_id/board/ready", get(handle_get_ready_cards))
        .route(
            "/tasks/:task_id/planner-instructions",
            get(handle_get_planner_instructions),
        )
        .route(
            "/tasks/:task_id/planner-instructions",
            put(handle_set_planner_instructions),
        )
        .route(
            "/tasks/:task_id/trajectories/:role",
            get(handle_list_task_trajectories),
        )
        .route(
            "/tasks/:task_id/planner-chats",
            post(handle_create_planner_chat),
        )
        .route(
            "/tasks/:task_id/planner-chats/:chat_id",
            delete(handle_delete_planner_chat),
        )
        .route(
            "/tasks/:task_id/planner-chats/from-transition",
            post(handle_planner_from_transition),
        )
        .route(
            "/chats/:chat_id/trajectory/transform/preview",
            post(handle_transform_preview),
        )
        .route(
            "/chats/:chat_id/trajectory/transform/apply",
            post(handle_transform_apply),
        )
        .route(
            "/chats/:chat_id/trajectory/handoff/preview",
            post(handle_handoff_preview),
        )
        .route(
            "/chats/:chat_id/trajectory/handoff/apply",
            post(handle_handoff_apply),
        )
        .route(
            "/chats/:chat_id/trajectory/mode-transition/apply",
            post(handle_mode_transition_apply),
        )
        .route(
            "/project-information",
            get(handle_v1_project_information_get),
        )
        .route(
            "/project-information",
            post(handle_v1_project_information_save),
        )
        .route(
            "/project-information/preview",
            post(handle_v1_project_information_preview),
        )
        .route("/exec/spawn", post(exec::handle_v1_exec_spawn))
        .route("/exec/list", get(exec::handle_v1_exec_list))
        .route("/exec/:process_id/read", get(exec::handle_v1_exec_read))
        .route("/exec/:process_id/kill", post(exec::handle_v1_exec_kill))
        .route(
            "/exec/:process_id/resize",
            post(exec::handle_v1_exec_resize),
        )
        .route("/exec/:process_id/stdin", post(exec::handle_v1_exec_stdin))
        .route(
            "/exec/:process_id/subscribe",
            get(exec::handle_v1_exec_subscribe),
        )
        .route(
            "/scheduler/cron",
            get(handle_v1_scheduler_cron_get).post(handle_v1_scheduler_cron_post),
        )
        .route(
            "/scheduler/cron/:id",
            delete(handle_v1_scheduler_cron_delete).patch(handle_v1_scheduler_cron_patch),
        )
        .route(
            "/scheduler/cron/:id/run",
            post(handle_v1_scheduler_cron_run),
        )
        .route("/hooks/fire", post(handle_v1_hooks_fire))
        .route("/setup/status", get(handle_v1_setup_status))
        .route("/browser/start", post(handle_browser_start))
        .route("/browser/stop", post(handle_browser_stop))
        .route("/browser/screenshot", post(handle_browser_screenshot))
        .route("/browser/context", post(handle_browser_context))
        .route(
            "/browser/context/commit",
            post(handle_browser_context_commit),
        )
        .route("/browser/element-pick", post(handle_browser_element_pick))
        .route(
            "/browser/element-pick/result",
            post(handle_browser_element_pick_result),
        )
        .route("/browser/curl", post(handle_browser_curl))
        .route("/browser/eval", post(handle_browser_eval))
        .route("/browser/inject-css", post(handle_browser_inject_css))
        .route("/browser/remove-css", post(handle_browser_remove_css))
        .route("/browser/dom-snapshot", post(handle_browser_dom_snapshot))
        .route("/browser/accessibility", post(handle_browser_accessibility))
        .route(
            "/browser/record-animation",
            post(handle_browser_record_animation),
        )
        .route("/browser/handoff", post(handle_browser_handoff))
        .route("/browser/status", post(handle_browser_status))
        .route(
            "/browser/annotate/start",
            post(handle_browser_annotate_start),
        )
        .route(
            "/browser/annotate/result",
            post(handle_browser_annotate_result),
        )
        .route(
            "/browser/annotate/clear",
            post(handle_browser_annotate_clear),
        )
        .route("/browser/action", post(handle_browser_action))
        .route("/stats/llm/summary", get(handle_v1_stats_llm_summary))
        .route("/stats/llm/events", get(handle_v1_stats_llm_events))
        .route("/ext/registry", get(handle_v1_ext_registry))
        .route("/ext/skills", post(handle_v1_ext_skill_post))
        .route("/ext/skills/:name", get(handle_v1_ext_skill_get))
        .route("/ext/skills/:name", put(handle_v1_ext_skill_put))
        .route("/ext/skills/:name", delete(handle_v1_ext_skill_delete))
        .route("/ext/commands", post(handle_v1_ext_command_post))
        .route("/ext/commands/:name", get(handle_v1_ext_command_get))
        .route("/ext/commands/:name", put(handle_v1_ext_command_put))
        .route("/ext/commands/:name", delete(handle_v1_ext_command_delete))
        .route("/ext/hooks", get(handle_v1_ext_hooks_get))
        .route("/ext/hooks", put(handle_v1_ext_hooks_put))
        .route(
            "/ext/competitor-import",
            get(handle_v1_ext_competitor_import_get),
        )
        .route(
            "/ext/competitor-import",
            post(handle_v1_ext_competitor_import_post),
        )
        .route(
            "/ext/hooks/:index",
            delete(handle_v1_ext_hooks_delete_by_index),
        )
        .route(
            "/ext/marketplace/sources",
            get(handle_v1_ext_marketplace_sources_get),
        )
        .route(
            "/ext/marketplace/sources",
            post(handle_v1_ext_marketplace_sources_post),
        )
        .route(
            "/ext/marketplace/sources/:id",
            delete(handle_v1_ext_marketplace_sources_delete),
        )
        .route(
            "/ext/marketplace/sources/:id/configure",
            post(handle_v1_ext_marketplace_sources_configure),
        )
        .route(
            "/ext/marketplace/sources/:id/refresh",
            post(handle_v1_ext_marketplace_sources_refresh),
        )
        .route(
            "/skills_marketplace/sources",
            get(handle_v1_ext_marketplace_sources_get),
        )
        .route(
            "/skills_marketplace/sources",
            post(handle_v1_ext_marketplace_sources_post),
        )
        .route(
            "/skills_marketplace/sources/:id",
            delete(handle_v1_ext_marketplace_sources_delete),
        )
        .route(
            "/skills_marketplace/sources/:id/configure",
            post(handle_v1_ext_marketplace_sources_configure),
        )
        .route(
            "/commands_marketplace/sources",
            get(handle_v1_ext_marketplace_sources_get),
        )
        .route(
            "/commands_marketplace/sources",
            post(handle_v1_ext_marketplace_sources_post),
        )
        .route(
            "/commands_marketplace/sources/:id",
            delete(handle_v1_ext_marketplace_sources_delete),
        )
        .route(
            "/commands_marketplace/sources/:id/configure",
            post(handle_v1_ext_marketplace_sources_configure),
        )
        .route(
            "/subagents_marketplace/sources",
            get(handle_v1_ext_marketplace_sources_get),
        )
        .route(
            "/subagents_marketplace/sources",
            post(handle_v1_ext_marketplace_sources_post),
        )
        .route(
            "/subagents_marketplace/sources/:id",
            delete(handle_v1_ext_marketplace_sources_delete),
        )
        .route(
            "/subagents_marketplace/sources/:id/configure",
            post(handle_v1_ext_marketplace_sources_configure),
        )
        .route("/skills/marketplace", get(handle_v1_skills_marketplace_get))
        .route(
            "/skills/marketplace/install",
            post(handle_v1_skills_marketplace_install),
        )
        .route("/skills_marketplace", get(handle_v1_skills_marketplace_get))
        .route(
            "/skills_marketplace/install",
            post(handle_v1_skills_marketplace_install),
        )
        .route(
            "/commands/marketplace",
            get(handle_v1_commands_marketplace_get),
        )
        .route(
            "/commands/marketplace/install",
            post(handle_v1_commands_marketplace_install),
        )
        .route(
            "/commands_marketplace",
            get(handle_v1_commands_marketplace_get),
        )
        .route(
            "/commands_marketplace/install",
            post(handle_v1_commands_marketplace_install),
        )
        .route(
            "/subagents/marketplace",
            get(handle_v1_subagents_marketplace_get),
        )
        .route(
            "/subagents/marketplace/install",
            post(handle_v1_subagents_marketplace_install),
        )
        .route(
            "/subagents_marketplace",
            get(handle_v1_subagents_marketplace_get),
        )
        .route(
            "/subagents_marketplace/install",
            post(handle_v1_subagents_marketplace_install),
        )
        .route("/plugins/marketplaces", get(handle_list_marketplaces))
        .route("/plugins/marketplaces", post(handle_add_marketplace))
        .route(
            "/plugins/marketplaces/:name",
            delete(handle_delete_marketplace),
        )
        .route(
            "/plugins/marketplace/:name/plugins",
            get(handle_list_marketplace_plugins),
        )
        .route("/plugins/install", post(handle_install_plugin))
        .route("/plugins/installed", get(handle_list_installed))
        .route("/plugins/installed/:name", delete(handle_uninstall_plugin))
        .route("/mcp-server-info", get(handle_v1_mcp_server_info))
        .route(
            "/mcp-server-reconnect",
            post(handle_v1_mcp_server_reconnect),
        )
        .route("/mcp/marketplace", get(handle_v1_mcp_marketplace_get))
        .route(
            "/mcp/marketplace/install",
            post(handle_v1_mcp_marketplace_install),
        )
        .route(
            "/mcp/marketplace/installed",
            get(handle_v1_mcp_marketplace_installed),
        )
        .route("/mcp/auto-name", post(handle_v1_mcp_auto_name))
        .route(
            "/mcp/marketplace/sources",
            get(handle_v1_mcp_marketplace_sources_get),
        )
        .route(
            "/mcp/marketplace/sources",
            post(handle_v1_mcp_marketplace_sources_post),
        )
        .route(
            "/mcp/marketplace/sources/:id",
            delete(handle_v1_mcp_marketplace_sources_delete),
        )
        .route(
            "/mcp/marketplace/sources/:id/configure",
            post(handle_v1_mcp_marketplace_sources_configure),
        )
        .route("/mcp/export", post(handle_v1_mcp_export))
        .route("/mcp/import", post(handle_v1_mcp_import))
        .route("/mcp/project-config", get(handle_v1_mcp_project_config))
        .route("/mcp/oauth/start", post(handle_v1_mcp_oauth_start))
        .route("/mcp/oauth/exchange", post(handle_v1_mcp_oauth_exchange))
        .route("/mcp/oauth/callback", get(handle_v1_mcp_oauth_callback))
        .route("/mcp/oauth/logout", post(handle_v1_mcp_oauth_logout))
        .route("/mcp/oauth/status", get(handle_v1_mcp_oauth_status))
        .route("/mcp/oauth/cancel", post(handle_v1_mcp_oauth_cancel))
        .route("/mcp/interactions", get(handle_v1_mcp_interactions_list))
        .route(
            "/mcp/interactions/respond",
            post(handle_v1_mcp_interactions_respond),
        )
        .route("/buddy", get(buddy::handle_v1_buddy_snapshot))
        .route(
            "/buddy/user_action",
            post(buddy::handle_v1_buddy_user_action),
        )
        .route(
            "/buddy/user_activity",
            get(buddy::handle_v1_buddy_user_activity),
        )
        .route(
            "/buddy/pulse_preview",
            get(buddy::handle_v1_buddy_pulse_preview),
        )
        .route("/buddy/settings", get(buddy::handle_v1_buddy_settings_get))
        .route(
            "/buddy/settings",
            post(buddy::handle_v1_buddy_settings_update),
        )
        .route("/buddy/care", post(buddy::handle_v1_buddy_care))
        .route(
            "/buddy/quest/accept",
            post(buddy::handle_v1_buddy_quest_accept),
        )
        .route(
            "/buddy/quest/dismiss",
            post(buddy::handle_v1_buddy_quest_dismiss),
        )
        .route(
            "/buddy/personality/reroll",
            post(buddy::handle_v1_buddy_personality_reroll),
        )
        .route("/buddy/activities", get(buddy::handle_v1_buddy_activities))
        .route(
            "/buddy/conversations",
            get(buddy::handle_v1_buddy_conversations_list),
        )
        .route(
            "/buddy/conversations",
            post(buddy::handle_v1_buddy_conversations_create),
        )
        .route(
            "/buddy/conversations/setup",
            post(buddy::handle_v1_buddy_conversations_create_setup),
        )
        .route(
            "/buddy/suggestions/:id/dismiss",
            post(buddy::handle_v1_buddy_suggestion_dismiss),
        )
        .route(
            "/buddy/runtime/:id/dismiss",
            post(buddy::handle_v1_buddy_runtime_dismiss),
        )
        .route(
            "/buddy/diagnostics",
            get(buddy::handle_v1_buddy_diagnostics_list),
        )
        .route(
            "/buddy/diagnostics/collect",
            post(buddy::handle_v1_buddy_diagnostics_collect),
        )
        .route(
            "/buddy/investigation-context",
            post(buddy::handle_v1_buddy_investigation_context),
        )
        .route(
            "/buddy/issues/create",
            post(buddy::handle_v1_buddy_issues_create),
        )
        .route(
            "/buddy/opportunities",
            get(buddy_opportunities::handle_v1_buddy_opportunities_list),
        )
        .route(
            "/buddy/opportunities/:id/accept",
            post(buddy_opportunities::handle_v1_buddy_opportunity_accept),
        )
        .route(
            "/buddy/opportunities/:id/dismiss",
            post(buddy_opportunities::handle_v1_buddy_opportunity_dismiss),
        )
        .route(
            "/buddy/actions/undo",
            post(buddy_opportunities::handle_v1_buddy_action_undo),
        )
        .route(
            "/buddy/receipts",
            get(buddy_opportunities::handle_v1_buddy_receipts),
        )
        .route(
            "/buddy/rules/unmute",
            post(buddy_opportunities::handle_v1_buddy_rule_unmute),
        )
        .route("/buddy/briefing", get(buddy::handle_v1_buddy_briefing))
        .route(
            "/buddy/speech-decisions",
            get(buddy::handle_v1_buddy_speech_decisions),
        )
        .route("/buddy/pulse", get(buddy_pulse::handle_v1_buddy_pulse))
        .route(
            "/buddy/drafts/skill",
            post(buddy_drafts::handle_v1_buddy_draft_create_skill),
        )
        .route(
            "/buddy/drafts/command",
            post(buddy_drafts::handle_v1_buddy_draft_create_command),
        )
        .route(
            "/buddy/drafts/subagent",
            post(buddy_drafts::handle_v1_buddy_draft_create_subagent),
        )
        .route(
            "/buddy/drafts/mode",
            post(buddy_drafts::handle_v1_buddy_draft_create_mode),
        )
        .route(
            "/buddy/drafts/agents_md",
            post(buddy_drafts::handle_v1_buddy_draft_create_agents_md),
        )
        .route(
            "/buddy/drafts/defaults",
            post(buddy_drafts::handle_v1_buddy_draft_create_defaults),
        )
        .route(
            "/buddy/drafts/hook",
            post(buddy_drafts::handle_v1_buddy_draft_create_hook),
        )
        .route(
            "/buddy/drafts/pulse_report",
            post(buddy_drafts::handle_v1_buddy_draft_create_pulse_report),
        )
        .route(
            "/buddy/drafts/:id",
            get(buddy_drafts::handle_v1_buddy_draft_get),
        )
        .route(
            "/buddy/drafts/:id",
            delete(buddy_drafts::handle_v1_buddy_draft_delete),
        )
        .route(
            "/buddy/frontend-error",
            post(buddy_frontend_error::handle_v1_buddy_frontend_error),
        );

    let rl = buddy_frontend_error::FrontendErrorRateLimiter::new();
    builder
        .layer(axum::Extension(rl))
        .layer(axum::middleware::from_fn(request_logging_middleware))
        .with_state(app_state)
}
