use at_tools::handle_v1_post_tools;
use axum::Router;
use axum::routing::{get, post, patch, delete};

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
use crate::http::routers::v1::git_branches::{
    handle_v1_git_branch_checkout, handle_v1_git_branch_create, handle_v1_git_branch_delete,
    handle_v1_git_branch_rename, handle_v1_git_fetch, handle_v1_git_pull, handle_v1_git_push,
};
use crate::http::routers::v1::git_read::{
    handle_v1_git_branches, handle_v1_git_commit_details, handle_v1_git_diff, handle_v1_git_log,
    handle_v1_git_stage, handle_v1_git_status, handle_v1_git_unstage,
};
use crate::http::routers::v1::git_write::{
    handle_v1_git_commit_v2, handle_v1_git_file, handle_v1_git_rollback,
};
use crate::http::routers::v1::graceful_shutdown::handle_v1_graceful_shutdown;
use crate::http::routers::v1::hooks::handle_v1_hooks_fire;
use crate::http::routers::v1::links::handle_v1_links;
use crate::http::routers::v1::lsp_like_handlers::{
    handle_v1_lsp_did_change, handle_v1_lsp_add_folder, handle_v1_lsp_initialize,
    handle_v1_lsp_remove_folder, handle_v1_set_active_document, handle_v1_git_branch_changed,
};
use crate::http::routers::v1::status::{
    handle_v1_codegraph_status, handle_v1_file_index_status, handle_v1_rag_status,
};
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
    handle_v1_project_defaults_get, handle_v1_project_defaults_update,
    handle_v1_provider_oauth_start, handle_v1_provider_remove_custom_model,
    handle_v1_provider_remove_custom_model_post, handle_v1_provider_schema,
    handle_v1_provider_quota, handle_v1_provider_quotas, handle_v1_provider_update,
    handle_v1_provider_usage, handle_v1_provider_usage_redeem, handle_v1_provider_usage_resets,
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
use crate::http::routers::v1::files::{
    handle_v1_files_browse, handle_v1_files_open, handle_v1_files_read, handle_v1_files_tree,
    handle_v1_files_write,
};
use crate::http::routers::v1::code_edit::handle_v1_code_edit;
use crate::http::routers::v1::workspace::handle_v1_get_app_searchable_id;
use crate::chat::{
    handle_v1_chats_subscribe_many, handle_v1_chat_subscribe, handle_v1_chat_command,
    handle_v1_chat_cancel_queued, handle_v1_chat_reprioritize_queued, handle_v1_trajectories_list,
    handle_v1_trajectories_all, handle_v1_trajectories_get, handle_v1_trajectory_path,
    handle_v1_trajectories_save, handle_v1_trajectories_delete, handle_v1_trajectories_subscribe,
};
use crate::http::routers::v1::voice::{
    handle_v1_voice_transcribe, handle_v1_voice_download, handle_v1_voice_status,
    handle_v1_voice_stream_subscribe, handle_v1_voice_stream_chunk,
};
use crate::http::routers::v1::tasks::{
    handle_get_task_goal, handle_list_tasks, handle_create_task, handle_get_task,
    handle_delete_task, handle_get_board, handle_create_card_comment, handle_patch_board,
    handle_create_card_comment_by_id, handle_get_planner_instructions,
    handle_set_planner_instructions, handle_get_ready_cards, handle_update_task_status,
    handle_update_task_meta, handle_list_task_trajectories, handle_create_planner_chat,
    handle_delete_planner_chat, handle_tasks_subscribe, handle_list_task_memories,
    handle_pin_task_memory, handle_archive_task_memory, handle_task_memories_triage_done,
    handle_task_memory_facets, handle_list_task_documents, handle_get_task_document,
    handle_create_task_document, handle_update_task_document, handle_append_task_document,
    handle_delete_task_document, handle_pin_task_document, handle_history_task_document,
};
use crate::http::routers::v1::trajectory_ops::{
    handle_transform_preview, handle_transform_apply, handle_mode_transition_apply,
    handle_planner_from_transition, handle_llm_compress_preview, handle_llm_compress_apply,
    handle_refresh_tools,
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
    handle_v1_worktrees_summary, handle_v1_worktrees_sync,
};
use crate::http::routers::v1::background_agents::{
    handle_v1_background_agents_cancel, handle_v1_background_agents_get,
    handle_v1_background_agents_message,
};

mod apps;
mod ast;
pub mod at_commands;
pub mod at_tools;
mod attention;
pub mod automation_credentials;
mod background_agents;
pub mod browser_settings;
pub mod buddy;
pub mod buddy_attention;
pub mod buddy_companion;
pub mod buddy_decisions;
pub mod buddy_drafts;
pub mod buddy_editorial;
pub mod buddy_frontend_error;
pub mod buddy_improvement;
pub mod buddy_native_approvals;
pub mod buddy_native_work;
pub mod buddy_observability;
pub mod buddy_operator;
pub mod buddy_opportunities;
pub mod buddy_pulse;
pub mod buddy_supervision;
pub mod bug_report;
pub mod caps;
mod causal;
mod chat_activity;
pub mod chat_based_handlers;
mod chat_modes;
pub mod code_completion;
mod code_edit;
pub mod code_intel;
pub mod code_lens;
mod commands_marketplace;
pub mod customization;
pub(crate) mod customization_editor;
pub mod engine_observation;
pub mod exec;
pub mod ext_management;
mod ext_marketplace_sources;
mod file_edit_tools;
mod files;
mod files_search;
mod git;
mod git_branches;
mod git_read;
mod git_write;
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
mod metrics;
mod organization;
mod performance;
mod plugins;
mod privacy;
mod project_configs;
pub mod project_information;
pub mod scheduler;
pub(crate) mod settings_config;
mod setup_status;
mod shell_policy;
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
pub mod v1_browser;
mod v1_integrations;
pub mod vecdb;
pub mod voice;
mod work;
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
    handle_v1_mcp_marketplace_installed, handle_v1_mcp_marketplace_update,
    handle_v1_mcp_marketplace_uninstall, handle_v1_mcp_auto_name, handle_v1_mcp_wizard_probe,
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
    handle_browser_element_pick_cancel, handle_browser_curl, handle_browser_eval,
    handle_browser_inject_css, handle_browser_remove_css, handle_browser_dom_snapshot,
    handle_browser_accessibility, handle_browser_record_animation, handle_browser_handoff,
    handle_browser_status, handle_browser_annotate_start, handle_browser_annotate_result,
    handle_browser_annotate_clear, handle_browser_action,
};

pub fn make_v1_router(app_state: AppState) -> Router<AppState> {
    let builder = Router::new()
        .route("/engine/identity", get(engine_observation::identity))
        .route("/chats/live", get(engine_observation::live))
        .merge(core_routes())
        .merge(coordination_routes())
        .merge(
            Router::new()
                .route("/organization/taxonomy", get(organization::taxonomy))
                .route("/organization/items", post(organization::items))
                .route("/organization/apply", post(organization::apply))
                .layer(axum::extract::DefaultBodyLimit::max(128 * 1024)),
        )
        .merge(tools_routes())
        .merge(lsp_routes())
        .merge(ast_routes())
        .merge(performance_routes())
        .merge(project_configs_routes())
        .merge(settings_routes())
        .merge(privacy_routes())
        .merge(worktrees_routes())
        .merge(background_agents_routes())
        .merge(at_commands_routes())
        .merge(integrations_routes())
        .merge(git_routes())
        .merge(files_routes())
        .merge(providers_routes())
        .merge(code_completion_routes())
        .merge(code_lens_routes())
        .merge(sync_files_routes())
        .merge(system_prompt_routes())
        .merge(code_intel_routes())
        .merge(knowledge_routes())
        .merge(trajectories_routes())
        .merge(chats_routes())
        .merge(voice_routes())
        .merge(tasks_routes())
        .merge(project_information_routes())
        .merge(exec_routes())
        .merge(scheduler_routes())
        .nest("/scheduler/credentials", automation_credentials::router())
        .merge(browser_routes())
        .merge(stats_routes())
        .merge(extensions_routes())
        .merge(marketplace_sources_routes())
        .merge(marketplace_skills_routes())
        .merge(marketplace_commands_routes())
        .merge(marketplace_subagents_routes())
        .merge(plugins_routes())
        .merge(mcp_routes())
        .merge(buddy_core_routes())
        .merge(buddy_opportunities_routes())
        .merge(buddy_drafts_routes())
        .merge(buddy_supervision_routes())
        .merge(buddy_improvement_routes())
        .merge(buddy_operator_routes())
        .merge(buddy_decisions_routes())
        .merge(buddy_editorial_routes())
        .merge(buddy_native_work_routes())
        .merge(buddy_frontend_error_routes())
        .merge(shell_policy_routes())
        .merge(bug_report_routes());

    let rl = buddy_frontend_error::FrontendErrorRateLimiter::new();
    builder
        .layer(axum::Extension(rl))
        .layer(axum::middleware::from_fn(request_logging_middleware))
        .with_state(app_state)
}
fn core_routes() -> Router<AppState> {
    Router::new()
        .route("/ping", get(handle_v1_ping))
        .route(
            "/graceful-shutdown",
            get(handle_v1_graceful_shutdown).post(handle_v1_graceful_shutdown),
        )
        .route("/caps", get(handle_v1_caps))
        .route("/build_info", get(crate::http::routers::info::handle_info))
        .route("/model-capabilities", get(handle_v1_model_capabilities))
        .route("/model-supported", get(handle_v1_model_supported))
        .route("/rag-status", get(handle_v1_rag_status))
        .route("/config-path", get(handle_v1_config_path))
        .route("/customization", get(handle_v1_customization))
        .route("/chat-modes", get(handle_v1_chat_modes))
        .route(
            "/customization/registry",
            get(handle_v1_customization_registry),
        )
        .route(
            "/customization/:kind/:id",
            get(handle_v1_customization_get)
                .put(handle_v1_customization_save)
                .delete(handle_v1_customization_delete),
        )
        .route("/customization/:kind", post(handle_v1_customization_create))
        .route("/links", post(handle_v1_links))
        .route(
            "/get-app-searchable-id",
            get(handle_v1_get_app_searchable_id),
        )
        .route(
            "/commit-message-from-diff",
            post(handle_v1_commit_message_from_diff),
        )
        .route("/sidebar/subscribe", get(sidebar::handle_sidebar_subscribe))
        .route("/hooks", get(hooks::handle_v1_hooks_surface))
        .route("/hooks/fire", post(handle_v1_hooks_fire))
        .route("/setup/status", get(handle_v1_setup_status))
}

fn coordination_routes() -> Router<AppState> {
    Router::new()
        .route("/apps", get(apps::list))
        .route("/apps/:app_id/document", get(apps::document))
        .route(
            "/apps/commands",
            post(apps::commands).layer(axum::extract::DefaultBodyLimit::max(9 * 1024 * 1024)),
        )
        .route("/apps/subscribe", get(apps::subscribe))
        .route("/work", get(work::list))
        .route("/work/subscribe", get(work::subscribe))
        .route("/work/wait-graph", get(work::wait_graph))
        .route("/work/provenance", delete(work::forget_provenance))
        .route("/work/causal", get(causal::trace))
        .route("/work/causal/subscribe", get(causal::subscribe))
        .route("/metrics", get(metrics::page))
        .route("/attention", get(attention::queue))
        .route("/work/:id", get(work::inspect))
}

fn tools_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/tools",
            get(handle_v1_get_tools).post(handle_v1_post_tools),
        )
        .route(
            "/tools-check-if-confirmation-needed",
            post(handle_v1_tools_check_if_confirmation_needed),
        )
        .route("/tools-execute", post(handle_v1_tools_execute))
}

fn lsp_routes() -> Router<AppState> {
    Router::new()
        .route("/lsp-initialize", post(handle_v1_lsp_initialize))
        .route("/lsp-did-changed", post(handle_v1_lsp_did_change))
        .route("/lsp-add-folder", post(handle_v1_lsp_add_folder))
        .route("/lsp-remove-folder", post(handle_v1_lsp_remove_folder))
        .route("/git-branch-changed", post(handle_v1_git_branch_changed))
        .route(
            "/lsp-set-active-document",
            post(handle_v1_set_active_document),
        )
}

fn ast_routes() -> Router<AppState> {
    Router::new()
        .route("/ast-file-symbols", post(handle_v1_ast_file_symbols))
        .route("/ast-file-dump", post(handle_v1_ast_file_dump))
        .route("/ast-status", get(handle_v1_ast_status))
}

fn performance_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/performance/telemetry",
            get(performance::handle_v1_performance_telemetry_get)
                .post(performance::handle_v1_performance_telemetry_post),
        )
        .route(
            "/performance/telemetry/reset",
            post(performance::handle_v1_performance_telemetry_reset),
        )
}

fn project_configs_routes() -> Router<AppState> {
    Router::new()
        .route("/project-configs", get(handle_v1_project_configs_get))
        .route(
            "/project-configs/rescan",
            post(handle_v1_project_configs_rescan),
        )
        .route(
            "/project-configs/bootstrap",
            post(handle_v1_project_configs_bootstrap),
        )
}

fn settings_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/indexing-settings",
            get(settings_config::handle_v1_indexing_settings_get)
                .post(settings_config::handle_v1_indexing_settings_post),
        )
        .route(
            "/skills/settings",
            get(settings_config::handle_v1_skills_settings_get)
                .post(settings_config::handle_v1_skills_settings_post),
        )
        .route(
            "/coordination-settings",
            get(settings_config::handle_v1_coordination_settings_get)
                .post(settings_config::handle_v1_coordination_settings_post),
        )
        .route(
            "/browser-settings",
            get(browser_settings::handle_v1_browser_settings_get)
                .post(browser_settings::handle_v1_browser_settings_post),
        )
}

fn privacy_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/privacy/policy",
            get(privacy::handle_v1_privacy_policy_get).post(privacy::handle_v1_privacy_policy_post),
        )
        .route("/privacy/status", get(privacy::handle_v1_privacy_status))
        .route("/privacy/inspect", post(privacy::handle_v1_privacy_inspect))
        .route("/privacy/derived", post(privacy::handle_v1_privacy_derived))
        .route(
            "/privacy/derived/clear",
            post(privacy::handle_v1_privacy_derived_clear),
        )
}

fn worktrees_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/worktrees",
            get(handle_v1_worktrees_list).post(handle_v1_worktrees_create),
        )
        .route("/worktrees/summary", get(handle_v1_worktrees_summary))
        .route(
            "/worktrees/cleanup-dry-run",
            post(handle_v1_worktrees_cleanup_dry_run),
        )
        .route("/worktrees/cleanup", post(handle_v1_worktrees_cleanup))
        .route(
            "/worktrees/:id",
            get(handle_v1_worktrees_get).delete(handle_v1_worktrees_delete),
        )
        .route("/worktrees/:id/diff", get(handle_v1_worktrees_diff))
        .route("/worktrees/:id/merge", post(handle_v1_worktrees_merge))
        .route("/worktrees/:id/sync", post(handle_v1_worktrees_sync))
        .route("/worktrees/:id/open", post(handle_v1_worktrees_open))
}

fn background_agents_routes() -> Router<AppState> {
    Router::new()
        .route("/background-agents", get(handle_v1_background_agents_get))
        .route(
            "/background-agents/:agent_id/cancel",
            post(handle_v1_background_agents_cancel),
        )
        .route(
            "/background-agents/:agent_id/message",
            post(handle_v1_background_agents_message),
        )
}

fn at_commands_routes() -> Router<AppState> {
    Router::new()
        .route("/at-command-completion", post(handle_v1_command_completion))
        .route("/at-command-preview", post(handle_v1_command_preview))
        .route("/at-command-execute", post(handle_v1_at_command_execute))
        .route("/slash-commands", get(handle_v1_slash_commands))
        .route("/fullpath", post(handle_v1_fullpath))
}

fn integrations_routes() -> Router<AppState> {
    Router::new()
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
}

fn git_routes() -> Router<AppState> {
    Router::new()
        .route("/git-commit", post(handle_v1_git_commit))
        .route("/git/status", get(handle_v1_git_status))
        .route("/git/diff", get(handle_v1_git_diff))
        .route("/git/log", get(handle_v1_git_log))
        .route("/git/branches", get(handle_v1_git_branches))
        .route("/git/stage", post(handle_v1_git_stage))
        .route("/git/unstage", post(handle_v1_git_unstage))
        .route(
            "/git/commit",
            get(handle_v1_git_commit_details).post(handle_v1_git_commit_v2),
        )
        .route("/git/rollback", post(handle_v1_git_rollback))
        .route("/git/file", get(handle_v1_git_file))
        .route("/git/branch/checkout", post(handle_v1_git_branch_checkout))
        .route("/git/branch/create", post(handle_v1_git_branch_create))
        .route("/git/branch/rename", post(handle_v1_git_branch_rename))
        .route("/git/branch/delete", post(handle_v1_git_branch_delete))
        .route("/git/fetch", post(handle_v1_git_fetch))
        .route("/git/pull", post(handle_v1_git_pull))
        .route("/git/push", post(handle_v1_git_push))
        .route("/checkpoints-preview", post(handle_v1_checkpoints_preview))
        .route("/checkpoints-restore", post(handle_v1_checkpoints_restore))
}

fn files_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/file_edit_tool_dry_run",
            post(handle_v1_file_edit_tool_dry_run),
        )
        .route("/files/search", get(files_search::handle_v1_files_search))
        .route("/files/tree", get(handle_v1_files_tree))
        .route("/files/read", get(handle_v1_files_read))
        .route("/files/open", post(handle_v1_files_open))
        .route("/files/browse", post(handle_v1_files_browse))
        .route("/files/write", post(handle_v1_files_write))
        .route("/code-edit", post(handle_v1_code_edit))
}

fn providers_routes() -> Router<AppState> {
    Router::new()
        .route("/models", get(handle_v1_models))
        .route("/providers", get(handle_v1_providers_list))
        .route("/providers/quotas", get(handle_v1_provider_quotas))
        .route(
            "/providers/:name",
            get(handle_v1_provider_get)
                .post(handle_v1_provider_update)
                .delete(handle_v1_provider_delete),
        )
        .route(
            "/providers/:name/bedrock/profiles",
            get(crate::providers::bedrock_setup::profiles),
        )
        .route(
            "/providers/:name/bedrock/verify",
            post(crate::providers::bedrock_setup::verify),
        )
        .route(
            "/providers/:name/bedrock/probe",
            post(crate::providers::bedrock_setup::probe),
        )
        .route(
            "/providers/:name/bedrock/apply",
            post(crate::providers::bedrock_setup::apply),
        )
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
            post(handle_v1_provider_add_custom_model)
                .delete(handle_v1_provider_remove_custom_model),
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
        .route("/providers/:name/quota", get(handle_v1_provider_quota))
        .route(
            "/providers/:name/usage/redeem",
            post(handle_v1_provider_usage_redeem),
        )
        .route(
            "/providers/:name/usage/resets",
            get(handle_v1_provider_usage_resets),
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
        .route(
            "/defaults",
            get(handle_v1_defaults_get).post(handle_v1_defaults_update),
        )
        .route(
            "/project-defaults",
            get(handle_v1_project_defaults_get).post(handle_v1_project_defaults_update),
        )
        .route("/openrouter/health", get(handle_v1_openrouter_health))
        .route("/google-gemini/health", get(handle_v1_google_gemini_health))
}

fn code_completion_routes() -> Router<AppState> {
    Router::new()
        .route("/code-completion", post(handle_v1_code_completion_web))
        .route(
            "/code-completion-prompt",
            post(handle_v1_code_completion_prompt),
        )
}

fn code_lens_routes() -> Router<AppState> {
    Router::new().route("/code-lens", post(handle_v1_code_lens))
}

fn sync_files_routes() -> Router<AppState> {
    Router::new().route(
        "/sync-files-extract-tar",
        post(handle_v1_sync_files_extract_tar),
    )
}

fn system_prompt_routes() -> Router<AppState> {
    Router::new().route(
        "/prepend-system-prompt-and-maybe-more-initial-messages",
        post(handle_v1_prepend_system_prompt_and_maybe_more_initial_messages),
    )
}

fn code_intel_routes() -> Router<AppState> {
    Router::new()
        .route("/vdb-search", post(handle_v1_vecdb_search))
        .route("/vdb-status", get(handle_v1_vecdb_status))
        .route("/codegraph-search", post(handle_v1_codegraph_search))
        .route("/codegraph-status", get(handle_v1_codegraph_status))
        .route("/file-index-status", get(handle_v1_file_index_status))
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
}

fn knowledge_routes() -> Router<AppState> {
    Router::new()
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
}

fn trajectories_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/trajectory-settings",
            get(settings_config::handle_v1_trajectory_settings_get)
                .post(settings_config::handle_v1_trajectory_settings_post),
        )
        .route("/trajectory-compress", post(handle_v1_trajectory_compress))
        .route("/trajectory/refresh-tools", post(handle_refresh_tools))
        .route("/trajectories", get(handle_v1_trajectories_list))
        .route("/trajectories/all", get(handle_v1_trajectories_all))
        .route(
            "/trajectories/subscribe",
            get(handle_v1_trajectories_subscribe),
        )
        .route(
            "/trajectories/:id",
            get(handle_v1_trajectories_get)
                .put(handle_v1_trajectories_save)
                .delete(handle_v1_trajectories_delete),
        )
        .route("/trajectories/:id/path", get(handle_v1_trajectory_path))
}

fn chats_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/chats/activity",
            get(chat_activity::handle_v1_chats_activity),
        )
        .route("/chats/subscribe", get(handle_v1_chat_subscribe))
        .route("/chats/subscribe-many", get(handle_v1_chats_subscribe_many))
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
            delete(handle_v1_chat_cancel_queued).patch(handle_v1_chat_reprioritize_queued),
        )
        .route(
            "/chats/:chat_id/trajectory/transform/preview",
            post(handle_transform_preview),
        )
        .route(
            "/chats/:chat_id/trajectory/llm-compress/preview",
            post(handle_llm_compress_preview),
        )
        .route(
            "/chats/:chat_id/trajectory/llm-compress/apply",
            post(handle_llm_compress_apply),
        )
        .route(
            "/chats/:chat_id/trajectory/transform/apply",
            post(handle_transform_apply),
        )
        .route(
            "/chats/:chat_id/trajectory/mode-transition/apply",
            post(handle_mode_transition_apply),
        )
}

fn voice_routes() -> Router<AppState> {
    Router::new()
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
}

fn tasks_routes() -> Router<AppState> {
    Router::new()
        .route("/tasks", get(handle_list_tasks).post(handle_create_task))
        .route("/tasks/subscribe", get(handle_tasks_subscribe))
        .route(
            "/tasks/:task_id",
            get(handle_get_task).delete(handle_delete_task),
        )
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
        .route(
            "/task/:task_id/documents",
            get(handle_list_task_documents).post(handle_create_task_document),
        )
        .route(
            "/task/:task_id/documents/:slug",
            get(handle_get_task_document)
                .put(handle_update_task_document)
                .delete(handle_delete_task_document),
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
        .route(
            "/tasks/:task_id/board",
            get(handle_get_board).post(handle_patch_board),
        )
        .route(
            "/tasks/:task_id/cards/:card_id/comments",
            post(handle_create_card_comment),
        )
        .route(
            "/tasks/:task_id/cards/comments",
            post(handle_create_card_comment_by_id),
        )
        .route("/tasks/:task_id/board/ready", get(handle_get_ready_cards))
        .route("/tasks/:task_id/goal", get(handle_get_task_goal))
        .route(
            "/tasks/:task_id/planner-instructions",
            get(handle_get_planner_instructions).put(handle_set_planner_instructions),
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
}

fn project_information_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/project-information",
            get(handle_v1_project_information_get).post(handle_v1_project_information_save),
        )
        .route(
            "/project-information/preview",
            post(handle_v1_project_information_preview),
        )
}

fn exec_routes() -> Router<AppState> {
    Router::new()
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
}

fn scheduler_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/scheduler/automation",
            get(scheduler::handle_v1_scheduler_automation_get)
                .post(scheduler::handle_v1_scheduler_automation_post),
        )
        .route(
            "/scheduler/automation/:id",
            get(scheduler::handle_v1_scheduler_automation_detail)
                .put(scheduler::handle_v1_scheduler_automation_put)
                .delete(scheduler::handle_v1_scheduler_automation_delete),
        )
        .route(
            "/scheduler/automation/:id/control",
            post(scheduler::handle_v1_scheduler_automation_control),
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
}

fn browser_routes() -> Router<AppState> {
    Router::new()
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
        .route(
            "/browser/element-pick/cancel",
            post(handle_browser_element_pick_cancel),
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
}

fn stats_routes() -> Router<AppState> {
    Router::new()
        .route("/stats/llm/summary", get(handle_v1_stats_llm_summary))
        .route("/stats/llm/events", get(handle_v1_stats_llm_events))
}

fn extensions_routes() -> Router<AppState> {
    Router::new()
        .route("/ext/registry", get(handle_v1_ext_registry))
        .route("/ext/skills", post(handle_v1_ext_skill_post))
        .route(
            "/ext/skills/:name",
            get(handle_v1_ext_skill_get)
                .put(handle_v1_ext_skill_put)
                .delete(handle_v1_ext_skill_delete),
        )
        .route("/ext/commands", post(handle_v1_ext_command_post))
        .route(
            "/ext/commands/:name",
            get(handle_v1_ext_command_get)
                .put(handle_v1_ext_command_put)
                .delete(handle_v1_ext_command_delete),
        )
        .route(
            "/ext/hooks",
            get(handle_v1_ext_hooks_get).put(handle_v1_ext_hooks_put),
        )
        .route(
            "/ext/competitor-import",
            get(handle_v1_ext_competitor_import_get).post(handle_v1_ext_competitor_import_post),
        )
        .route(
            "/ext/hooks/:index",
            delete(handle_v1_ext_hooks_delete_by_index),
        )
        .route(
            "/ext/marketplace/sources",
            get(handle_v1_ext_marketplace_sources_get).post(handle_v1_ext_marketplace_sources_post),
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
}

fn marketplace_sources_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/skills_marketplace/sources",
            get(handle_v1_ext_marketplace_sources_get).post(handle_v1_ext_marketplace_sources_post),
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
            get(handle_v1_ext_marketplace_sources_get).post(handle_v1_ext_marketplace_sources_post),
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
            get(handle_v1_ext_marketplace_sources_get).post(handle_v1_ext_marketplace_sources_post),
        )
        .route(
            "/subagents_marketplace/sources/:id",
            delete(handle_v1_ext_marketplace_sources_delete),
        )
        .route(
            "/subagents_marketplace/sources/:id/configure",
            post(handle_v1_ext_marketplace_sources_configure),
        )
        .route("/skills_marketplace", get(handle_v1_skills_marketplace_get))
        .route(
            "/skills_marketplace/install",
            post(handle_v1_skills_marketplace_install),
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
            "/subagents_marketplace",
            get(handle_v1_subagents_marketplace_get),
        )
        .route(
            "/subagents_marketplace/install",
            post(handle_v1_subagents_marketplace_install),
        )
}

fn marketplace_skills_routes() -> Router<AppState> {
    Router::new()
        .route("/skills/marketplace", get(handle_v1_skills_marketplace_get))
        .route(
            "/skills/marketplace/install",
            post(handle_v1_skills_marketplace_install),
        )
}

fn marketplace_commands_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/commands/marketplace",
            get(handle_v1_commands_marketplace_get),
        )
        .route(
            "/commands/marketplace/install",
            post(handle_v1_commands_marketplace_install),
        )
}

fn marketplace_subagents_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/subagents/marketplace",
            get(handle_v1_subagents_marketplace_get),
        )
        .route(
            "/subagents/marketplace/install",
            post(handle_v1_subagents_marketplace_install),
        )
}

fn plugins_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/plugins/marketplaces",
            get(handle_list_marketplaces).post(handle_add_marketplace),
        )
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
}

fn mcp_routes() -> Router<AppState> {
    Router::new()
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
        .route(
            "/mcp/marketplace/update",
            post(handle_v1_mcp_marketplace_update),
        )
        .route(
            "/mcp/marketplace/uninstall",
            post(handle_v1_mcp_marketplace_uninstall),
        )
        .route("/mcp/auto-name", post(handle_v1_mcp_auto_name))
        .route("/mcp/wizard-probe", post(handle_v1_mcp_wizard_probe))
        .route(
            "/mcp/marketplace/sources",
            get(handle_v1_mcp_marketplace_sources_get).post(handle_v1_mcp_marketplace_sources_post),
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
}

fn buddy_core_routes() -> Router<AppState> {
    Router::new()
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
        .route(
            "/buddy/settings",
            get(buddy::handle_v1_buddy_settings_get).post(buddy::handle_v1_buddy_settings_update),
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
            get(buddy::handle_v1_buddy_conversations_list)
                .post(buddy::handle_v1_buddy_conversations_create),
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
        .route("/buddy/briefing", get(buddy::handle_v1_buddy_briefing))
        .route(
            "/buddy/speech-decisions",
            get(buddy::handle_v1_buddy_speech_decisions),
        )
        .route("/buddy/pulse", get(buddy_pulse::handle_v1_buddy_pulse))
}

fn buddy_opportunities_routes() -> Router<AppState> {
    Router::new()
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
}

fn buddy_drafts_routes() -> Router<AppState> {
    Router::new()
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
            get(buddy_drafts::handle_v1_buddy_draft_get)
                .delete(buddy_drafts::handle_v1_buddy_draft_delete),
        )
}

fn buddy_supervision_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/buddy/supervision/context",
            get(buddy_supervision::context),
        )
        .route(
            "/buddy/supervision/mandates",
            post(buddy_supervision::create_mandate),
        )
        .route(
            "/buddy/supervision/mandates/:id",
            get(buddy_supervision::get_mandate),
        )
        .route(
            "/buddy/supervision/mandates/:id/revoke",
            post(buddy_supervision::revoke_mandate),
        )
        .route(
            "/buddy/supervision/mandates/:id/intake",
            post(buddy_supervision::intake_goal),
        )
        .route(
            "/buddy/supervision/mandates/:id/spawn",
            post(buddy_supervision::spawn_worker),
        )
        .route(
            "/buddy/supervision/mandates/:id/status",
            get(buddy_supervision::get_status),
        )
        .route(
            "/buddy/supervision/mandates/:id/reconcile",
            post(buddy_supervision::reconcile),
        )
        .route(
            "/buddy/supervision/mandates/:id/routine-question",
            post(buddy_supervision::routine_question),
        )
        .route(
            "/buddy/supervision/mandates/:id/message",
            post(buddy_supervision::message_worker),
        )
        .route(
            "/buddy/supervision/mandates/:id/pause",
            post(buddy_supervision::pause_worker),
        )
        .route(
            "/buddy/supervision/mandates/:id/resume",
            post(buddy_supervision::resume_worker),
        )
        .route(
            "/buddy/supervision/mandates/:id/cancel",
            post(buddy_supervision::cancel_worker),
        )
        .route(
            "/buddy/supervision/mandates/:id/assess-candidate",
            post(buddy_supervision::assess_candidate),
        )
}

fn buddy_improvement_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/buddy/improvement/candidates",
            get(buddy_improvement::handle_list).post(buddy_improvement::handle_create),
        )
        .route(
            "/buddy/improvement/candidates/:id",
            get(buddy_improvement::handle_get),
        )
        .route(
            "/buddy/improvement/specs",
            post(buddy_improvement::handle_spec_create),
        )
        .route(
            "/buddy/improvement/candidates/:id/proofs",
            post(buddy_improvement::handle_proof),
        )
        .route(
            "/buddy/improvement/candidates/:id/spawn",
            post(buddy_improvement::handle_spawn),
        )
        .route(
            "/buddy/improvement/candidates/:id/activation-check",
            post(buddy_improvement::handle_activation_check),
        )
        .route(
            "/buddy/improvement/candidates/:id/activate",
            post(buddy_improvement::handle_activate),
        )
        .route(
            "/buddy/improvement/attempts",
            get(buddy_improvement::handle_attempts),
        )
        .route(
            "/buddy/improvement/inspect",
            post(buddy_improvement::handle_inspect),
        )
        .route(
            "/buddy/improvement/artifacts/inspect",
            post(buddy_improvement::handle_artifact_inspect),
        )
        .route(
            "/buddy/improvement/candidates/:id/evaluate",
            post(buddy_improvement::handle_evaluate),
        )
        .route(
            "/buddy/improvement/evaluations",
            get(buddy_improvement::handle_evaluations),
        )
        .route(
            "/buddy/improvement/evaluations/:id",
            get(buddy_improvement::handle_evaluation_get),
        )
        .route(
            "/buddy/improvement/evaluations/:id/cancel",
            post(buddy_improvement::handle_evaluation_cancel),
        )
}

fn buddy_operator_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/buddy/operator/status",
            get(buddy_operator::handle_operator_status),
        )
        .route(
            "/buddy/operator/reopen",
            post(buddy_operator::handle_operator_reopen),
        )
        .route(
            "/buddy/operator/pending-publications",
            post(buddy_operator::handle_pending_publications),
        )
        .route(
            "/buddy/operator/pending-publications/:id/retry",
            post(buddy_operator::handle_retry_publication),
        )
        .route(
            "/buddy/operator/legacy-drafts/:id/work",
            get(buddy_operator::handle_legacy_draft_work),
        )
        .route(
            "/buddy/work",
            get(buddy_operator::handle_v1_buddy_operator_work_list),
        )
        .route(
            "/buddy/work/actions",
            get(buddy_operator::handle_v1_buddy_operator_actions_batch),
        )
        .route(
            "/buddy/operator/diagnostics",
            get(buddy_observability::handle_summary),
        )
        .route(
            "/buddy/operator/retention/preview",
            get(buddy_observability::handle_retention_preview),
        )
        .route(
            "/buddy/operator/policy/preview",
            post(buddy_observability::handle_policy_preview),
        )
        .route(
            "/buddy/operator/policy/confirm",
            post(buddy_observability::handle_policy_confirm),
        )
        .route(
            "/buddy/actions/preview",
            post(buddy_operator::handle_resource_preview),
        )
        .route(
            "/buddy/actions/draft",
            post(buddy_operator::handle_resource_draft),
        )
        .route(
            "/buddy/actions/rights",
            post(buddy_operator::handle_resource_rights),
        )
        .route(
            "/buddy/actions/prepare",
            post(buddy_operator::handle_resource_prepare),
        )
        .route(
            "/buddy/actions/grant",
            post(buddy_operator::handle_resource_grant),
        )
        .route(
            "/buddy/actions/execute",
            post(buddy_operator::handle_resource_execute),
        )
        .route(
            "/buddy/executions/:id/recover",
            post(buddy_operator::handle_resource_recover),
        )
        .route(
            "/buddy/grants/:id/revoke",
            post(buddy_operator::handle_resource_grant_revoke),
        )
        .route(
            "/buddy/work/:id",
            get(buddy_operator::handle_v1_buddy_operator_work_get),
        )
        .route(
            "/buddy/work/:id/actions",
            get(buddy_operator::handle_v1_buddy_operator_actions),
        )
        .route(
            "/buddy/work/:id/actions/attach",
            post(buddy_operator::handle_resource_attach),
        )
        .route(
            "/buddy/work/:id/outcomes",
            get(buddy_operator::handle_v1_buddy_operator_work_outcomes),
        )
        .route(
            "/buddy/work/:id/read",
            get(buddy_operator::handle_v1_buddy_operator_read_get)
                .post(buddy_operator::handle_v1_buddy_operator_read_mark),
        )
        .route(
            "/buddy/work/:id/snooze",
            get(buddy_operator::handle_v1_buddy_operator_snooze_get)
                .post(buddy_operator::handle_v1_buddy_operator_snooze_set),
        )
        .route(
            "/buddy/work/:id/actions/:action_id/execute",
            post(buddy_operator::handle_v1_buddy_operator_action_execute),
        )
        .route(
            "/buddy/artifacts/:id",
            get(buddy_operator::handle_v1_buddy_operator_artifact_get),
        )
        .route(
            "/buddy/artifacts/:id/revisions",
            get(buddy_operator::handle_v1_buddy_operator_artifact_history),
        )
        .route(
            "/buddy/outcomes/:id",
            get(buddy_operator::handle_v1_buddy_operator_outcome_get),
        )
        .route(
            "/buddy/executions/:id",
            get(buddy_operator::handle_v1_buddy_operator_execution_get),
        )
}

fn buddy_decisions_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/buddy/companion/:source_id",
            post(buddy_companion::handle_v1_buddy_companion),
        )
        .route(
            "/buddy/attention",
            get(buddy_attention::handle_current_attention),
        )
        .route(
            "/buddy/decisions/current",
            get(buddy_decisions::handle_current_decisions),
        )
        .route(
            "/buddy/approvals/current",
            get(buddy_native_approvals::handle_current_approvals),
        )
        .route(
            "/buddy/approvals/:chat_id/response",
            post(buddy_native_approvals::handle_approval_response),
        )
        .route(
            "/buddy/approvals/:chat_id/outcomes/:client_request_id",
            get(buddy_native_approvals::handle_approval_outcome),
        )
        .route(
            "/buddy/decisions/questions/:chat_id/:tool_call_id",
            get(buddy_decisions::handle_question_state)
                .post(buddy_decisions::handle_question_answer),
        )
}

fn buddy_editorial_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/buddy/editorial/claim",
            post(buddy_editorial::handle_claim),
        )
        .route(
            "/buddy/editorial/feedback",
            post(buddy_editorial::handle_feedback),
        )
        .route(
            "/buddy/editorial/metrics",
            get(buddy_editorial::handle_metrics),
        )
}

fn buddy_native_work_routes() -> Router<AppState> {
    Router::new().route(
        "/buddy/native-work",
        get(buddy_native_work::handle_native_work),
    )
}

fn buddy_frontend_error_routes() -> Router<AppState> {
    Router::new().route(
        "/buddy/frontend-error",
        post(buddy_frontend_error::handle_v1_buddy_frontend_error),
    )
}
fn shell_policy_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/shell-policy",
            get(shell_policy::handle_v1_shell_policy_get)
                .post(shell_policy::handle_v1_shell_policy_post),
        )
        .route(
            "/shell-policy/test",
            post(shell_policy::handle_v1_shell_policy_test),
        )
        .route(
            "/shell-policy/audit",
            get(shell_policy::handle_v1_shell_policy_audit),
        )
}

fn bug_report_routes() -> Router<AppState> {
    Router::new()
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
}

#[cfg(test)]
mod router_tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::make_v1_router;
    use crate::app_state::AppState;

    #[tokio::test]
    async fn v1_router_routes_each_domain_and_keeps_shared_layers() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let mut sources =
            crate::ext::extensions_marketplace::load_all_sources(&app.paths.config_dir)
                .await
                .unwrap();
        for source in &mut sources {
            source.enabled = matches!(
                source.source_kind,
                crate::ext::extensions_marketplace::MarketplaceSourceKind::BuiltinEmbedded
            );
        }
        crate::ext::extensions_marketplace::save_sources_db(
            &app.paths.config_dir,
            &crate::ext::extensions_marketplace::ExtensionsMarketplaceSourcesDb {
                sources,
            },
        )
        .await
        .unwrap();
        for source in super::mcp_marketplace_sources::load_sources(&app.paths.config_dir)
            .await
            .sources
        {
            super::mcp_marketplace_sources::handle_v1_mcp_marketplace_sources_configure(
                axum::extract::State(app.clone()),
                axum::extract::Path(source.id),
                hyper::body::Bytes::from_static(br#"{"enabled":false}"#),
            )
            .await
            .unwrap();
        }
        let router = make_v1_router(app.clone()).with_state(app);
        let routes = [
            ("GET", "/ping"),
            ("GET", "/build_info"),
            ("GET", "/bug-report/context"),
            ("GET", "/tools"),
            ("POST", "/lsp-initialize"),
            ("GET", "/ast-status"),
            ("GET", "/performance/telemetry"),
            ("GET", "/project-configs"),
            ("GET", "/browser-settings"),
            ("GET", "/privacy/status"),
            ("GET", "/worktrees"),
            ("GET", "/background-agents"),
            ("GET", "/slash-commands"),
            ("GET", "/integrations"),
            ("GET", "/git/status"),
            ("GET", "/files/tree"),
            ("GET", "/models"),
            ("POST", "/code-completion"),
            ("POST", "/code-lens"),
            ("POST", "/sync-files-extract-tar"),
            (
                "POST",
                "/prepend-system-prompt-and-maybe-more-initial-messages",
            ),
            ("GET", "/codegraph-status"),
            ("GET", "/knowledge-graph"),
            ("GET", "/trajectories/all"),
            ("GET", "/chats/subscribe"),
            ("GET", "/voice/status"),
            ("GET", "/tasks"),
            ("GET", "/project-information"),
            ("GET", "/exec/list"),
            ("GET", "/scheduler/cron"),
            ("GET", "/scheduler/automation"),
            ("POST", "/scheduler/automation"),
            ("PUT", "/scheduler/automation/test"),
            ("DELETE", "/scheduler/automation/test"),
            ("POST", "/scheduler/automation/test/control"),
            ("POST", "/browser/status"),
            ("GET", "/stats/llm/summary"),
            ("GET", "/ext/registry"),
            ("GET", "/skills_marketplace"),
            ("GET", "/skills/marketplace"),
            ("GET", "/commands/marketplace"),
            ("GET", "/subagents/marketplace"),
            ("GET", "/plugins/installed"),
            ("GET", "/mcp/marketplace"),
            ("GET", "/buddy"),
            ("GET", "/buddy/opportunities"),
            ("GET", "/buddy/native-work"),
            ("GET", "/buddy/work/actions?ids=missing"),
            ("GET", "/shell-policy"),
        ];

        for (method, uri) in routes {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_ne!(response.status(), StatusCode::NOT_FOUND, "{method} {uri}");
            assert_ne!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} {uri}"
            );
        }

        let wrong_method = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ping")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);

        let frontend_error_request = || {
            Request::builder()
                .method("POST")
                .uri("/buddy/frontend-error")
                .header("content-type", "application/json")
                .header("x-forwarded-for", "192.0.2.10")
                .body(Body::from(
                    r#"{"message":"test error","stack":"","url":"test","kind":"test"}"#,
                ))
                .unwrap()
        };
        for _ in 0..60 {
            let response = router
                .clone()
                .oneshot(frontend_error_request())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let limited = router.oneshot(frontend_error_request()).await.unwrap();
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);

        let dir = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        let cmdline = &mut std::sync::Arc::get_mut(&mut gcx).unwrap().cmdline;
        cmdline.daemon_auth_token = Some("automation-router-token".into());
        cmdline.http_host = Some("127.0.0.1".into());
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        let app = AppState::from_gcx(gcx).await;
        let runtime = crate::scheduler::automation_api::active(&app)
            .await
            .unwrap();
        let definition = serde_json::json!({"schema_version":1,"id":"private-automation","revision":1,
            "owner":{"kind":"project"},"description":"private automation","enabled":false,
            "trigger":{"kind":"continuous"},"action":{"kind":"collect","driver":{"type":"manual"}},
            "deliveries":[],"lifecycle":{"kind":"on_demand"},
            "policy":{"idempotent":false,"max_pending_events":10,"max_event_bytes":1024}});
        let initial = runtime
            .save(0, vec![serde_json::from_value(definition.clone()).unwrap()])
            .await
            .unwrap();
        assert_eq!(initial.definitions.len(), 1);
        let revision =
            serde_json::json!({"expected_catalog_revision":initial.revision,"expected_revision":1});
        let save = serde_json::json!({"expected_catalog_revision":initial.revision,
            "expected_revision":1,"definition":definition});
        let control = serde_json::json!({"expected_catalog_revision":initial.revision,
            "expected_revision":1,"operation":"enable"});
        let router = make_v1_router(app.clone()).with_state(app.clone());
        for (method, uri, body) in [
            ("GET", "/scheduler/automation", serde_json::Value::Null),
            (
                "GET",
                "/scheduler/automation/private-automation",
                serde_json::Value::Null,
            ),
            ("POST", "/scheduler/automation", save.clone()),
            ("PUT", "/scheduler/automation/private-automation", save),
            (
                "DELETE",
                "/scheduler/automation/private-automation",
                revision.clone(),
            ),
            (
                "POST",
                "/scheduler/automation/private-automation/control",
                control,
            ),
        ] {
            let (status, body) = automation_request(
                &router,
                method,
                uri,
                body,
                None,
                Some("127.0.0.1:7"),
                Some("http://evil.example"),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {body}");
            assert!(!body.to_string().contains("private-automation"));
            assert_eq!(
                serde_json::to_value(runtime.list().unwrap()).unwrap(),
                serde_json::to_value(&initial).unwrap()
            );
        }
        let (status, _) = automation_request(
            &router,
            "GET",
            "/scheduler/automation",
            serde_json::Value::Null,
            Some("wrong-token"),
            Some("127.0.0.1:7"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        for uri in [
            "/scheduler/automation",
            "/scheduler/automation/private-automation",
        ] {
            let (status, body) = automation_request(
                &router,
                "GET",
                uri,
                serde_json::Value::Null,
                None,
                Some("192.0.2.1:7"),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["definitions"].as_array().unwrap().len(), 1);
            assert_eq!(
                body["definitions"][0]["definition"]["id"],
                "private-automation"
            );
        }
        let (status, _) = automation_request(
            &router,
            "DELETE",
            "/scheduler/automation/private-automation",
            revision.clone(),
            Some("automation-router-token"),
            Some("127.0.0.1:7"),
            Some("http://evil.example"),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(runtime.list().unwrap().definitions.len(), 1);
        let (status, body) = automation_request(
            &router,
            "DELETE",
            "/scheduler/automation/private-automation",
            revision,
            None,
            None,
            Some("http://127.0.0.1:8001"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["removed"], true);
        assert!(runtime.list().unwrap().definitions.is_empty());
        runtime.shutdown().await;
        runtime.sources.shutdown().await;

        let dir = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("local-cache"),
            dir.path().join("local-config"),
        )
        .await;
        std::sync::Arc::get_mut(&mut gcx).unwrap().cmdline.http_host = Some("127.0.0.1".into());
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        let app = AppState::from_gcx(gcx).await;
        let router = make_v1_router(app.clone()).with_state(app.clone());
        for (peer, origin, expected) in [
            (None, None, StatusCode::OK),
            (
                Some("127.0.0.1:7"),
                Some("http://127.0.0.1:8001"),
                StatusCode::OK,
            ),
        ] {
            let (status, _) = automation_request(
                &router,
                "GET",
                "/scheduler/automation",
                serde_json::Value::Null,
                None,
                peer,
                origin,
            )
            .await;
            assert_eq!(status, expected, "peer={peer:?} origin={origin:?}");
        }
        let runtime = crate::scheduler::automation_api::active(&app)
            .await
            .unwrap();
        runtime.shutdown().await;
        runtime.sources.shutdown().await;
    }

    async fn automation_request(
        router: &axum::Router,
        method: &str,
        uri: &str,
        body: serde_json::Value,
        token: Option<&str>,
        peer: Option<&str>,
        origin: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:8001")
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let mut request = request.body(Body::from(body.to_string())).unwrap();
        if let Some(peer) = peer {
            request.extensions_mut().insert(axum::extract::ConnectInfo(
                peer.parse::<std::net::SocketAddr>().unwrap(),
            ));
        }
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }
}
