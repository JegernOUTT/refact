use at_tools::handle_v1_post_tools;
use axum::Router;
use axum::routing::{get, post, put, patch, delete};

use crate::http::utils::telemetry_middleware;
use crate::http::routers::v1::code_completion::{
    handle_v1_code_completion_web, handle_v1_code_completion_prompt,
};
use crate::http::routers::v1::code_lens::handle_v1_code_lens;
use crate::http::routers::v1::ast::{
    handle_v1_ast_file_dump, handle_v1_ast_file_symbols, handle_v1_ast_status,
};
use crate::http::routers::v1::at_commands::{
    handle_v1_command_completion, handle_v1_command_preview, handle_v1_at_command_execute,
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
use crate::http::routers::v1::dashboard::get_dashboard_plots;
use crate::http::routers::v1::docker::{
    handle_v1_docker_container_action, handle_v1_docker_container_list,
};
use crate::http::routers::v1::git::{
    handle_v1_git_commit, handle_v1_checkpoints_preview, handle_v1_checkpoints_restore,
};
use crate::http::routers::v1::graceful_shutdown::handle_v1_graceful_shutdown;
use crate::http::routers::v1::snippet_accepted::handle_v1_snippet_accepted;
use crate::http::routers::v1::telemetry_network::handle_v1_telemetry_network;
use crate::http::routers::v1::telemetry_chat::handle_v1_telemetry_chat;
use crate::http::routers::v1::links::handle_v1_links;
use crate::http::routers::v1::lsp_like_handlers::{
    handle_v1_lsp_did_change, handle_v1_lsp_add_folder, handle_v1_lsp_initialize,
    handle_v1_lsp_remove_folder, handle_v1_set_active_document,
};
use crate::http::routers::v1::status::handle_v1_rag_status;
use crate::http::routers::v1::customization::handle_v1_customization;
use crate::http::routers::v1::customization::handle_v1_config_path;
use crate::http::routers::v1::gui_help_handlers::handle_v1_fullpath;
use crate::http::routers::v1::sync_files::handle_v1_sync_files_extract_tar;
use crate::http::routers::v1::system_prompt::handle_v1_prepend_system_prompt_and_maybe_more_initial_messages;
use crate::providers::http::{
    handle_v1_providers_list, handle_v1_provider_get, handle_v1_provider_schema,
    handle_v1_provider_update, handle_v1_provider_delete, handle_v1_provider_models,
    handle_v1_provider_available_models, handle_v1_provider_model_toggle,
    handle_v1_provider_model_provider_update, handle_v1_openrouter_model_endpoints,
    handle_v1_provider_add_custom_model, handle_v1_provider_remove_custom_model,
    handle_v1_provider_remove_custom_model_post,
    handle_v1_defaults_get, handle_v1_defaults_update, handle_v1_models,
    handle_v1_provider_oauth_start, handle_v1_provider_oauth_exchange,
    handle_v1_provider_oauth_logout, handle_v1_provider_oauth_callback,
    handle_v1_openrouter_account_info, handle_v1_openrouter_health,
    handle_v1_google_gemini_health,
};

use crate::http::routers::v1::vecdb::{handle_v1_vecdb_search, handle_v1_vecdb_status};
use crate::http::routers::v1::knowledge_graph::handle_v1_knowledge_graph;
use crate::http::routers::v1::knowledge_ops::{
    handle_v1_knowledge_update_memory, handle_v1_knowledge_delete_memory,
};
use crate::http::routers::v1::v1_integrations::{
    handle_v1_integration_get, handle_v1_integration_icon, handle_v1_integration_save,
    handle_v1_integration_delete, handle_v1_integrations, handle_v1_integrations_filtered,
    handle_v1_integrations_mcp_logs,
};
use crate::http::routers::v1::file_edit_tools::handle_v1_file_edit_tool_dry_run;
use crate::http::routers::v1::code_edit::handle_v1_code_edit;
use crate::http::routers::v1::workspace::{
    handle_v1_get_app_searchable_id, handle_v1_set_active_group_id,
};
use crate::chat::{
    handle_v1_chat_subscribe, handle_v1_chat_command, handle_v1_chat_cancel_queued,
    handle_v1_trajectories_list, handle_v1_trajectories_all, handle_v1_trajectories_get,
    handle_v1_trajectories_save, handle_v1_trajectories_delete, handle_v1_trajectories_subscribe,
};
use crate::http::routers::v1::voice::{
    handle_v1_voice_transcribe, handle_v1_voice_download, handle_v1_voice_status,
    handle_v1_voice_stream_subscribe, handle_v1_voice_stream_chunk,
};
use crate::http::routers::v1::tasks::{
    handle_list_tasks, handle_create_task, handle_get_task, handle_delete_task, handle_get_board,
    handle_patch_board, handle_get_planner_instructions, handle_set_planner_instructions,
    handle_get_ready_cards, handle_update_task_status, handle_update_task_meta,
    handle_list_task_trajectories, handle_create_planner_chat, handle_tasks_subscribe,
};
use crate::http::routers::v1::trajectory_ops::{
    handle_transform_preview, handle_transform_apply, handle_handoff_preview, handle_handoff_apply,
    handle_mode_transition_apply,
};
use crate::http::routers::v1::project_configs::{
    handle_v1_project_configs_get, handle_v1_project_configs_rescan, handle_v1_project_configs_bootstrap,
};

mod ast;
pub mod at_commands;
pub mod at_tools;
pub mod caps;
pub mod chat_based_handlers;
pub mod code_completion;
mod code_edit;
pub mod code_lens;
pub mod customization;
mod dashboard;
mod docker;
mod file_edit_tools;
mod git;
pub mod graceful_shutdown;
mod gui_help_handlers;
pub mod knowledge_enrichment;
mod knowledge_graph;
pub mod knowledge_ops;
pub mod links;
pub mod lsp_like_handlers;
pub mod sidebar;
pub mod snippet_accepted;
pub mod status;
pub mod sync_files;
pub mod system_prompt;
pub mod tasks;
pub mod telemetry_chat;
pub mod telemetry_network;
mod trajectory_ops;
mod v1_integrations;
pub mod vecdb;
pub mod voice;
mod workspace;
mod project_configs;
mod chat_modes;
mod customization_editor;
pub mod project_information;
mod v1_browser;
mod stats;

use crate::http::routers::v1::chat_modes::handle_v1_chat_modes;
use crate::http::routers::v1::customization_editor::{
    handle_v1_customization_registry, handle_v1_customization_get,
    handle_v1_customization_save, handle_v1_customization_create,
    handle_v1_customization_delete,
};
use crate::http::routers::v1::project_information::{
    handle_v1_project_information_get, handle_v1_project_information_save,
    handle_v1_project_information_preview,
};
use crate::http::routers::v1::stats::{
    handle_v1_stats_llm_summary, handle_v1_stats_llm_events,
};
use crate::http::routers::v1::v1_browser::{
    handle_browser_start, handle_browser_stop, handle_browser_screenshot,
    handle_browser_context, handle_browser_context_commit,
    handle_browser_element_pick, handle_browser_element_pick_result,
    handle_browser_curl, handle_browser_eval,
    handle_browser_inject_css, handle_browser_remove_css,
    handle_browser_dom_snapshot, handle_browser_accessibility,
    handle_browser_record_animation, handle_browser_handoff,
    handle_browser_status,
};

pub fn make_v1_router() -> Router {
    let builder = Router::new()
        .route("/ping", get(handle_v1_ping))
        .route("/graceful-shutdown", get(handle_v1_graceful_shutdown))
        .route("/code-completion", post(handle_v1_code_completion_web))
        .route("/code-lens", post(handle_v1_code_lens))
        .route("/telemetry-network", post(handle_v1_telemetry_network))
        .route("/telemetry-chat", post(handle_v1_telemetry_chat))
        .route("/snippet-accepted", post(handle_v1_snippet_accepted))
        .route("/caps", get(handle_v1_caps))
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
        .route("/project-configs/rescan", post(handle_v1_project_configs_rescan))
        .route("/project-configs/bootstrap", post(handle_v1_project_configs_bootstrap))
        .route("/chat-modes", get(handle_v1_chat_modes))
        .route("/customization/registry", get(handle_v1_customization_registry))
        .route("/customization/:kind/:id", get(handle_v1_customization_get))
        .route("/customization/:kind/:id", put(handle_v1_customization_save))
        .route("/customization/:kind", post(handle_v1_customization_create))
        .route("/customization/:kind/:id", delete(handle_v1_customization_delete))
        .route(
            "/sync-files-extract-tar",
            post(handle_v1_sync_files_extract_tar),
        )
        .route("/git-commit", post(handle_v1_git_commit))
        .route(
            "/prepend-system-prompt-and-maybe-more-initial-messages",
            post(handle_v1_prepend_system_prompt_and_maybe_more_initial_messages),
        )
        .route("/at-command-completion", post(handle_v1_command_completion))
        .route("/at-command-preview", post(handle_v1_command_preview))
        .route("/at-command-execute", post(handle_v1_at_command_execute))
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
        .route(
            "/docker-container-list",
            post(handle_v1_docker_container_list),
        )
        .route(
            "/docker-container-action",
            post(handle_v1_docker_container_action),
        )
        .route("/checkpoints-preview", post(handle_v1_checkpoints_preview))
        .route("/checkpoints-restore", post(handle_v1_checkpoints_restore))
        .route("/links", post(handle_v1_links))
        .route(
            "/file_edit_tool_dry_run",
            post(handle_v1_file_edit_tool_dry_run),
        )
        .route("/code-edit", post(handle_v1_code_edit))
        .route("/models", get(handle_v1_models))
        .route("/providers", get(handle_v1_providers_list))
        .route("/providers/:name", get(handle_v1_provider_get))
        .route("/providers/:name", post(handle_v1_provider_update))
        .route("/providers/:name", delete(handle_v1_provider_delete))
        .route("/providers/:name/schema", get(handle_v1_provider_schema))
        .route("/providers/:name/models", get(handle_v1_provider_models))
        .route("/providers/:name/available-models", get(handle_v1_provider_available_models))
        .route(
            "/providers/:name/models/:model_id/endpoints",
            get(handle_v1_openrouter_model_endpoints),
        )
        .route("/providers/:name/models/toggle", post(handle_v1_provider_model_toggle))
        .route("/providers/:name/models/provider", post(handle_v1_provider_model_provider_update))
        .route("/providers/:name/custom-models", post(handle_v1_provider_add_custom_model))
        .route("/providers/:name/custom-models", delete(handle_v1_provider_remove_custom_model))
        .route("/providers/:name/custom-models/remove", post(handle_v1_provider_remove_custom_model_post))
        .route("/openrouter/account-info", get(handle_v1_openrouter_account_info))
        .route("/providers/:name/oauth/start", post(handle_v1_provider_oauth_start))
        .route("/providers/:name/oauth/exchange", post(handle_v1_provider_oauth_exchange))
        .route("/providers/:name/oauth/logout", post(handle_v1_provider_oauth_logout))
        .route("/providers/:name/oauth/callback", get(handle_v1_provider_oauth_callback))
        .route("/defaults", get(handle_v1_defaults_get))
        .route("/defaults", post(handle_v1_defaults_update))
        .route("/openrouter/health", get(handle_v1_openrouter_health))
        .route("/google-gemini/health", get(handle_v1_google_gemini_health))
        // cloud related
        .route("/set-active-group-id", post(handle_v1_set_active_group_id))
        .route(
            "/get-app-searchable-id",
            get(handle_v1_get_app_searchable_id),
        )
        // experimental
        .route("/get-dashboard-plots", get(get_dashboard_plots))
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
        .route("/knowledge-graph", get(handle_v1_knowledge_graph))
        .route(
            "/knowledge/update-memory",
            post(handle_v1_knowledge_update_memory),
        )
        .route(
            "/knowledge/delete-memory",
            post(handle_v1_knowledge_delete_memory),
        )
        .route("/trajectory-compress", post(handle_v1_trajectory_compress))
        .route("/trajectories", get(handle_v1_trajectories_list))
        .route("/trajectories/all", get(handle_v1_trajectories_all))
        .route(
            "/trajectories/subscribe",
            get(handle_v1_trajectories_subscribe),
        )
        .route("/trajectories/:id", get(handle_v1_trajectories_get))
        .route("/trajectories/:id", put(handle_v1_trajectories_save))
        .route("/trajectories/:id", delete(handle_v1_trajectories_delete))
        .route("/chats/subscribe", get(handle_v1_chat_subscribe))
        .route("/chats/:chat_id/commands", post(handle_v1_chat_command))
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
        .route("/tasks/:task_id/board", get(handle_get_board))
        .route("/tasks/:task_id/board", post(handle_patch_board))
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
        .route("/project-information", get(handle_v1_project_information_get))
        .route("/project-information", post(handle_v1_project_information_save))
        .route("/project-information/preview", post(handle_v1_project_information_preview))
        .route("/browser/start", post(handle_browser_start))
        .route("/browser/stop", post(handle_browser_stop))
        .route("/browser/screenshot", post(handle_browser_screenshot))
        .route("/browser/context", post(handle_browser_context))
        .route("/browser/context/commit", post(handle_browser_context_commit))
        .route("/browser/element-pick", post(handle_browser_element_pick))
        .route("/browser/element-pick/result", post(handle_browser_element_pick_result))
        .route("/browser/curl", post(handle_browser_curl))
        .route("/browser/eval", post(handle_browser_eval))
        .route("/browser/inject-css", post(handle_browser_inject_css))
        .route("/browser/remove-css", post(handle_browser_remove_css))
        .route("/browser/dom-snapshot", post(handle_browser_dom_snapshot))
        .route("/browser/accessibility", post(handle_browser_accessibility))
        .route("/browser/record-animation", post(handle_browser_record_animation))
        .route("/browser/handoff", post(handle_browser_handoff))
        .route("/browser/status", post(handle_browser_status))
        .route("/stats/llm/summary", get(handle_v1_stats_llm_summary))
        .route("/stats/llm/events", get(handle_v1_stats_llm_events));

    builder.layer(axum::middleware::from_fn(telemetry_middleware))
}
