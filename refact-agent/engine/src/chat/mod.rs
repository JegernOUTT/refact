pub mod browser_context;
pub(crate) mod browser_decisions;
pub mod cache_diagnostics;
pub mod cache_guard;
#[cfg(test)]
mod cache_stability_tests;
pub mod cloud_token_count;
#[cfg(test)]
mod compaction_hook_tests;
#[cfg(test)]
mod compression_vocabulary_tests;
pub mod config;
mod content;
pub mod context_rebuild;
#[cfg(test)]
mod context_rebuild_recovery_tests;
pub mod delivery;
pub mod diagnostics;
mod generation;
pub mod live_diagnostics;
pub(crate) use generation::{
    length_stop_continue_instruction, normalize_stop_reason, LengthStopKind, NormalizedStopReason,
};
#[cfg(test)]
mod generation_pressure_retry_tests;
#[cfg(test)]
mod goal_disagreement_gate_tests;
pub(crate) mod goal_validation_hook;
#[cfg(test)]
mod goal_validation_hook_tests;
#[cfg(test)]
mod notification_hook_tests;
#[cfg(test)]
mod plan_fold_compaction_tests;
#[cfg(test)]
mod planner_pause_edge_tests;
#[cfg(test)]
mod planner_pause_load_race_tests;
#[cfg(test)]
pub(crate) use generation::prepare_project_work_context;
#[cfg(test)]
pub(crate) use generation::{maybe_recover_after_length_stop, start_generation};
pub(crate) mod evidence;
pub(crate) mod evidence_cache;
pub(crate) mod evidence_revision;
#[cfg(test)]
mod goal_checkpoint_snapshot_tests;
pub mod goal_monitor;
pub mod goal_role;
pub mod goal_verifier;
pub mod goal_wait;
mod handlers;
pub mod history_limit;
pub mod internal_roles;
pub mod linearize;
pub mod notifications;
pub(crate) mod openai_codex_ws;
mod openai_merge;
pub mod perf_diagnostics;
#[cfg(any(test, feature = "bench"))]
pub mod perf_harness;
pub mod perf_telemetry;
pub(crate) mod permission_decision_hooks;
#[cfg(test)]
mod permission_decision_hooks_tests;
pub mod plan_role;
pub mod post_merge_check;
pub mod prepare;
pub mod prompt_snippets;
pub mod prompts;
#[cfg(test)]
pub(crate) mod questions_closure_tests;
pub(crate) mod queue;
pub(crate) mod refusal_followup;
pub(crate) mod retry_policy;
mod session;
pub(crate) mod worktree_lifecycle;
pub(crate) use session::goal_snapshot_from_messages;
pub mod session_runtime;
pub mod stream_core;
pub mod summarization;
pub mod system_context;
#[cfg(test)]
mod task_agent_attention_tests;
pub mod task_agent_monitor;
#[cfg(test)]
mod tests;
mod tool_call_recovery;
mod tool_call_recovery_oss;
pub mod tools;
pub mod trajectories;
pub mod trajectory_archive;
pub mod trajectory_index;
pub mod trajectory_ops;
pub mod types;
pub mod verifier;
mod verifier_diff;
#[cfg(test)]
mod verifier_false_fail_tests;
#[cfg(test)]
mod verifier_infra_tests;
pub(crate) mod verify_cmd;
#[cfg(test)]
mod verify_command_boundary_tests;
#[cfg(test)]
mod verify_command_readonly_tests;
#[cfg(test)]
mod verify_line_parsing_tests;

pub use session::{
    SessionsMap, create_sessions_map, start_session_cleanup_task,
    get_or_create_session_with_trajectory, close_all_chat_sessions,
    try_restore_session_if_trajectory_exists, snapshot_with_agents, snapshot_with_agents_mode,
};
pub use queue::process_command_queue;
#[cfg(test)]
pub(crate) use queue::handle_tool_decisions_guarded;
pub use delivery::{deliver_to_chat, drain_deliveries_at_boundary, update_pending_delivery_in_chat};
pub use trajectories::{
    start_trajectory_watcher, TrajectoryEvent, TrajectoryMeta, handle_v1_trajectories_list,
    handle_v1_trajectories_all, handle_v1_trajectories_get, handle_v1_trajectory_path,
    handle_v1_trajectories_save, handle_v1_trajectories_delete, handle_v1_trajectories_subscribe,
    maybe_save_trajectory, find_trajectory_path, find_trajectory_or_buddy_path,
    find_trajectory_path_for_active_chat, list_all_trajectories_meta, list_trajectories_page,
    list_trajectories_page_for,
};
pub use handlers::{
    handle_v1_chats_subscribe_many, handle_v1_chat_subscribe, handle_v1_chat_command,
    handle_v1_chat_cancel_queued, handle_v1_chat_reprioritize_queued,
};
pub use goal_monitor::start_goal_monitor;
pub use task_agent_monitor::start_agent_monitor;

pub(crate) mod recovery;
