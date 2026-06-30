// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use std::time::{Duration, Instant};

use crate::history::cells::HistoryCell;

use super::chunking::{AdaptiveChunkingPolicy, ChunkingMode, DrainPlan, QueueSnapshot};
use super::controller::{PlanStreamController, StreamController};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommitTickScope {
    AnyMode,
    CatchUpOnly,
}

pub(crate) struct CommitTickOutput {
    pub(crate) cells: Vec<Box<dyn HistoryCell>>,
    pub(crate) has_controller: bool,
    pub(crate) all_idle: bool,
}

impl Default for CommitTickOutput {
    fn default() -> Self {
        Self {
            cells: Vec::new(),
            has_controller: false,
            all_idle: true,
        }
    }
}

pub(crate) fn run_commit_tick(
    policy: &mut AdaptiveChunkingPolicy,
    stream_controller: Option<&mut StreamController>,
    plan_stream_controller: Option<&mut PlanStreamController>,
    scope: CommitTickScope,
    now: Instant,
) -> CommitTickOutput {
    let snapshot = stream_queue_snapshot(
        stream_controller.as_deref(),
        plan_stream_controller.as_deref(),
        now,
    );
    let decision = resolve_chunking_plan(policy, snapshot, now);
    if scope == CommitTickScope::CatchUpOnly && decision.mode != ChunkingMode::CatchUp {
        return CommitTickOutput::default();
    }
    apply_commit_tick_plan(
        decision.drain_plan,
        stream_controller,
        plan_stream_controller,
    )
}

fn resolve_chunking_plan(
    policy: &mut AdaptiveChunkingPolicy,
    snapshot: QueueSnapshot,
    now: Instant,
) -> super::chunking::ChunkingDecision {
    policy.decide(snapshot, now)
}

fn stream_queue_snapshot(
    stream_controller: Option<&StreamController>,
    plan_stream_controller: Option<&PlanStreamController>,
    now: Instant,
) -> QueueSnapshot {
    let mut queued_lines = 0usize;
    let mut oldest_age = None;
    if let Some(controller) = stream_controller {
        queued_lines += controller.queued_lines();
        oldest_age = max_duration(oldest_age, controller.oldest_queued_age(now));
    }
    if let Some(controller) = plan_stream_controller {
        queued_lines += controller.queued_lines();
        oldest_age = max_duration(oldest_age, controller.oldest_queued_age(now));
    }
    QueueSnapshot {
        queued_lines,
        oldest_age,
    }
}

fn apply_commit_tick_plan(
    drain_plan: DrainPlan,
    stream_controller: Option<&mut StreamController>,
    plan_stream_controller: Option<&mut PlanStreamController>,
) -> CommitTickOutput {
    let mut output = CommitTickOutput::default();
    if let Some(controller) = stream_controller {
        output.has_controller = true;
        let (cell, idle) = controller.drain_for_commit_tick(drain_plan);
        if let Some(cell) = cell {
            output.cells.push(cell);
        }
        output.all_idle &= idle;
    }
    if let Some(controller) = plan_stream_controller {
        output.has_controller = true;
        let (cell, idle) = controller.drain_for_commit_tick(drain_plan);
        if let Some(cell) = cell {
            output.cells.push(cell);
        }
        output.all_idle &= idle;
    }
    output
}

fn max_duration(lhs: Option<Duration>, rhs: Option<Duration>) -> Option<Duration> {
    match (lhs, rhs) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Line;
    use std::path::Path;

    fn drain_text(cells: &[Box<dyn HistoryCell>]) -> String {
        cells
            .iter()
            .flat_map(|cell| {
                text(&cell.render(80))
                    .lines()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn any_mode_drains_assistant_controller() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let mut stream = StreamController::new(None, Path::new("."));
        stream.push_delta("one\n");

        let output = run_commit_tick(
            &mut policy,
            Some(&mut stream),
            None,
            CommitTickScope::AnyMode,
            Instant::now(),
        );

        assert!(output.has_controller);
        assert!(output.all_idle);
        assert_eq!(drain_text(&output.cells), "• one");
    }

    #[test]
    fn any_mode_drains_plan_controller() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let mut plan = PlanStreamController::new(None, Path::new("."));
        plan.push_delta("- one\n");

        let output = run_commit_tick(
            &mut policy,
            None,
            Some(&mut plan),
            CommitTickScope::AnyMode,
            Instant::now(),
        );

        assert!(output.has_controller);
        assert!(output.all_idle);
        let rendered = drain_text(&output.cells);
        assert!(rendered.contains("Proposed Plan"));
        assert!(rendered.contains("- one"));
    }

    #[test]
    fn combined_snapshot_enters_catch_up_by_summed_depth() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let mut stream = StreamController::new(None, Path::new("."));
        let mut plan = PlanStreamController::new(None, Path::new("."));
        stream.push_delta("a\nb\nc\nd\n");
        plan.push_delta("e\nf\ng\nh\n");

        let output = run_commit_tick(
            &mut policy,
            Some(&mut stream),
            Some(&mut plan),
            CommitTickScope::CatchUpOnly,
            Instant::now(),
        );

        assert_eq!(output.cells.len(), 2);
        assert!(output.all_idle);
    }

    #[test]
    fn catch_up_only_suppresses_smooth_ticks() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let mut stream = StreamController::new(None, Path::new("."));
        stream.push_delta("one\n");

        let output = run_commit_tick(
            &mut policy,
            Some(&mut stream),
            None,
            CommitTickScope::CatchUpOnly,
            Instant::now(),
        );

        assert!(!output.has_controller);
        assert!(output.cells.is_empty());
        assert_eq!(stream.queued_lines(), 1);
    }

    #[test]
    fn no_controllers_reports_idle_without_controller() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let output = run_commit_tick(
            &mut policy,
            None,
            None,
            CommitTickScope::AnyMode,
            Instant::now(),
        );

        assert!(!output.has_controller);
        assert!(output.all_idle);
        assert!(output.cells.is_empty());
    }
}
