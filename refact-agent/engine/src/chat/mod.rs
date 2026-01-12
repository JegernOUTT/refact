pub mod config;
mod content;
mod generation;
mod handlers;
pub mod history_limit;
pub mod openai_convert;
mod openai_merge;
pub mod prepare;
pub mod prompts;
mod queue;
mod session;
pub mod stream_core;
pub mod system_context;
#[cfg(test)]
mod tests;
pub mod tools;
pub mod trajectories;
pub mod trajectory_ops;
pub mod types;

pub use session::{SessionsMap, create_sessions_map, start_session_cleanup_task, get_or_create_session_with_trajectory};
pub use queue::process_command_queue;
pub use trajectories::{
    start_trajectory_watcher, TrajectoryEvent, handle_v1_trajectories_list,
    handle_v1_trajectories_all, handle_v1_trajectories_get, handle_v1_trajectories_save,
    handle_v1_trajectories_delete, handle_v1_trajectories_subscribe, maybe_save_trajectory,
};
pub use handlers::{handle_v1_chat_subscribe, handle_v1_chat_command, handle_v1_chat_cancel_queued};
