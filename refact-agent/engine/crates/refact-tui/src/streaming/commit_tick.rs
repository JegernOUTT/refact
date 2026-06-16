// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use std::time::Instant;

use super::chunking::{AdaptiveChunkingPolicy, DrainPlan, QueueSnapshot};
use super::controller::CommitDrain;

pub fn run_commit_tick(controller: &mut impl CommitDrain) -> Option<String> {
    controller.run_commit_tick()
}

pub(crate) fn drain_with_policy(
    policy: &mut AdaptiveChunkingPolicy,
    snapshot: QueueSnapshot,
    now: Instant,
    drain: impl FnOnce(DrainPlan) -> Option<String>,
) -> Option<String> {
    let decision = policy.decide(snapshot, now);
    drain(decision.drain_plan)
}
