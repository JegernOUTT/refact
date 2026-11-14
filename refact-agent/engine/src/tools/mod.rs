pub(crate) mod command_checkout;
pub(crate) mod native_enrichment;
pub(crate) mod planner_delivery;
pub mod scope_utils;
pub mod shell_gate;
pub mod shell_gate_llm;
pub mod tools_description;
pub mod tools_list;

#[cfg(test)]
mod cancel_agent_discard_tests;
#[cfg(test)]
mod cancel_agent_stop_tests;
pub mod design_tools;
#[cfg(test)]
mod merge_agent_force_stop_tests;
#[cfg(test)]
mod merge_agent_legacy_conflict_tests;
#[cfg(test)]
mod merge_agent_target_checkout_tests;
#[cfg(test)]
mod restart_capture_followup_tests;
#[cfg(test)]
mod restart_fresh_diff_tests;
pub mod subagent_phases;
pub(crate) mod task_tool_helpers;
#[cfg(test)]
mod tests_schema;
pub mod tool_helpers;
pub mod tool_name_alias;

mod role_history;
mod tool_activate_skill;
mod tool_add_workspace_folder;
pub mod tool_agent_approval;
mod tool_agent_chat_summary;
mod tool_agent_diff;
pub mod tool_agent_interact;
pub(crate) mod tool_agent_lifecycle;
pub(crate) mod tool_agent_planner_qna;
mod tool_agent_pulse;
mod tool_agent_steer;
pub(crate) mod tool_app;
#[cfg(test)]
mod tool_app_source;
mod tool_ast_definition;
pub mod tool_background_agents;
mod tool_card_comment;
mod tool_cat;
pub mod tool_causal_trace;
pub mod tool_chrome;
mod tool_clean_background_processes;
#[cfg(test)]
pub mod tool_cmdline;
mod tool_code_mode;
#[cfg(test)]
mod tool_code_mode_tests;
pub(crate) mod tool_codegraph;
pub mod tool_config_subagent;
mod tool_contrast_audit;
pub(crate) mod tool_cron_create;
pub(crate) mod tool_cron_delete;
mod tool_cron_list;
pub(crate) mod tool_cron_update;
mod tool_design_system;
pub(crate) mod tool_disagreement;
mod tool_fact_ledger;
mod tool_get_goal;
mod tool_glob;
pub mod tool_goal_pursuit_controls;
pub(crate) mod tool_handoff_to_mode;
mod tool_image_region;
mod tool_knowledge;
mod tool_manage_mcp;
mod tool_mark_elements;
mod tool_mcp_search;
pub mod tool_metrics;
mod tool_mv;
pub(crate) mod tool_pause;
mod tool_process;
mod tool_process_subscribe;
mod tool_process_write_stdin;
mod tool_regex_search;
pub(crate) mod tool_review;
mod tool_rm;
pub(crate) mod tool_schedule;
mod tool_search;
mod tool_search_trajectories;
mod tool_set_goal;
mod tool_set_trajectory_label;
mod tool_shell;
pub mod tool_team;
pub(crate) use tool_shell::approval_request as shell_approval_request;
pub(crate) use tool_process::approval_request as process_approval_request;
pub(crate) use tool_shell::approval_request_without_cache as shell_approval_request_without_cache;
pub(crate) use tool_process::approval_request_without_cache as process_approval_request_without_cache;
#[cfg(test)]
mod plan_history_archive_tests;
pub mod tool_organization;
pub(crate) mod tool_sleep;
pub(crate) mod tool_spawn_ab;
mod tool_subagent;
pub(crate) mod tool_tool_job;
mod tool_trajectory_context;
mod tool_tree;
mod tool_ui_probe;
pub(crate) mod tool_update_goal;
mod tool_validate_goal;
mod tool_visual_diff;
pub(crate) mod tool_wait;
mod tool_web;
mod tool_web_search;
pub mod tool_work_inspect;
pub mod tool_work_overview;

#[cfg(test)]
mod peer_authority_boundary_tests;
#[cfg(test)]
mod tool_causal_trace_tests;
#[cfg(test)]
mod tool_team_role_grant_root_tests;
#[cfg(test)]
mod tool_team_role_grant_tests;
#[cfg(test)]
mod tool_team_tests;

pub mod buddy;
pub mod file_edit;
#[cfg(test)]
pub(crate) mod settings_guard;
pub(crate) mod tool_ask_questions;
pub(crate) mod tool_buddy_create_draft;
pub(crate) mod tool_buddy_create_issue;
pub(crate) mod tool_buddy_get_context;
pub(crate) mod tool_buddy_get_logs;
mod tool_buddy_launch_investigation;
pub(crate) mod tool_buddy_open_setup_flow;
pub(crate) mod tool_buddy_open_view;
pub(crate) mod tool_buddy_say;
mod tool_create_knowledge;
pub(crate) mod tool_mcp_call;
pub(crate) mod tool_task_agent;
pub(crate) mod tool_task_agent_finish;
mod tool_task_batch;
mod tool_task_board;
#[cfg(test)]
mod tool_task_board_attest_tests;
mod tool_task_board_authority;
mod tool_task_broadcast;
pub(crate) mod tool_task_check_agents;
pub(crate) mod tool_task_documents;
mod tool_task_done;
mod tool_task_init;
mod tool_task_mark_card;
pub mod tool_task_memory;
pub(crate) mod tool_task_merge_agent;
mod tool_task_overview;
pub(crate) mod tool_task_restart_agent;
pub(crate) mod tool_task_spawn_agent;
mod tool_task_verify_card;
mod tool_task_wait_for_agents;
mod tool_tasks;
mod tool_worktree_create;
mod tool_worktree_merge;
