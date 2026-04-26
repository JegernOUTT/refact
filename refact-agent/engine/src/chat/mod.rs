pub mod browser_context;
pub mod cache_guard;
pub mod config;
mod content;
mod generation;
mod handlers;
pub mod history_limit;
mod openai_merge;
pub mod prepare;
pub mod prompt_snippets;
pub mod prompts;
mod queue;
mod session;
pub mod stream_core;
pub mod system_context;
pub mod task_agent_monitor;
#[cfg(test)]
mod tests;
mod tool_call_recovery;
mod tool_call_recovery_oss;
pub mod tools;
pub mod trajectories;
pub mod trajectory_ops;
pub mod types;

pub use session::{
    SessionsMap, create_sessions_map, start_session_cleanup_task,
    get_or_create_session_with_trajectory, close_all_chat_sessions,
};
pub use queue::process_command_queue;
pub use trajectories::{
    start_trajectory_watcher, TrajectoryEvent, TrajectoryMeta, handle_v1_trajectories_list,
    handle_v1_trajectories_all, handle_v1_trajectories_get, handle_v1_trajectories_save,
    handle_v1_trajectories_delete, handle_v1_trajectories_subscribe, maybe_save_trajectory,
    find_trajectory_path, list_all_trajectories_meta,
};
pub use handlers::{handle_v1_chat_subscribe, handle_v1_chat_command, handle_v1_chat_cancel_queued};
pub use task_agent_monitor::start_agent_monitor;
