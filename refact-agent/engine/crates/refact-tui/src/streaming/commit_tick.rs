// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use super::chunking::{AdaptiveChunkingPolicy, DrainPlan, QueueSnapshot};
use super::controller::CommitDrain;

pub fn run_commit_tick(controller: &mut impl CommitDrain) -> Option<String> {
    controller.run_commit_tick()
}

pub(crate) fn drain_with_policy(
    policy: &mut AdaptiveChunkingPolicy,
    queued_lines: usize,
    drain: impl FnOnce(DrainPlan) -> Option<String>,
) -> Option<String> {
    let decision = policy.decide(QueueSnapshot { queued_lines });
    drain(decision.drain_plan)
}
