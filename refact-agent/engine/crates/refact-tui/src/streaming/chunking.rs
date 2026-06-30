// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use std::time::{Duration, Instant};

const ENTER_QUEUE_DEPTH_LINES: usize = 8;
const ENTER_OLDEST_AGE: Duration = Duration::from_millis(120);
const EXIT_QUEUE_DEPTH_LINES: usize = 2;
const EXIT_OLDEST_AGE: Duration = Duration::from_millis(40);
const EXIT_HOLD: Duration = Duration::from_millis(250);
const REENTER_CATCH_UP_HOLD: Duration = Duration::from_millis(250);
const SEVERE_QUEUE_DEPTH_LINES: usize = 64;
const SEVERE_OLDEST_AGE: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ChunkingMode {
    #[default]
    Smooth,
    CatchUp,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct QueueSnapshot {
    pub(crate) queued_lines: usize,
    pub(crate) oldest_age: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrainPlan {
    Single,
    Batch(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChunkingDecision {
    pub(crate) mode: ChunkingMode,
    pub(crate) entered_catch_up: bool,
    pub(crate) drain_plan: DrainPlan,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct AdaptiveChunkingPolicy {
    mode: ChunkingMode,
    below_exit_threshold_since: Option<Instant>,
    last_catch_up_exit_at: Option<Instant>,
}

impl AdaptiveChunkingPolicy {
    pub(crate) fn reset(&mut self) {
        self.mode = ChunkingMode::Smooth;
        self.below_exit_threshold_since = None;
        self.last_catch_up_exit_at = None;
    }

    pub(crate) fn decide(&mut self, snapshot: QueueSnapshot, now: Instant) -> ChunkingDecision {
        if snapshot.queued_lines == 0 {
            self.note_catch_up_exit(now);
            self.mode = ChunkingMode::Smooth;
            self.below_exit_threshold_since = None;
            return ChunkingDecision {
                mode: self.mode,
                entered_catch_up: false,
                drain_plan: DrainPlan::Single,
            };
        }

        let entered_catch_up = match self.mode {
            ChunkingMode::Smooth => self.maybe_enter_catch_up(snapshot, now),
            ChunkingMode::CatchUp => {
                self.maybe_exit_catch_up(snapshot, now);
                false
            }
        };

        let drain_plan = match self.mode {
            ChunkingMode::Smooth => DrainPlan::Single,
            ChunkingMode::CatchUp => DrainPlan::Batch(snapshot.queued_lines.max(1)),
        };

        ChunkingDecision {
            mode: self.mode,
            entered_catch_up,
            drain_plan,
        }
    }

    fn maybe_enter_catch_up(&mut self, snapshot: QueueSnapshot, now: Instant) -> bool {
        if !should_enter_catch_up(snapshot) {
            return false;
        }
        if self.reentry_hold_active(now) && !is_severe_backlog(snapshot) {
            return false;
        }
        self.mode = ChunkingMode::CatchUp;
        self.below_exit_threshold_since = None;
        self.last_catch_up_exit_at = None;
        true
    }

    fn maybe_exit_catch_up(&mut self, snapshot: QueueSnapshot, now: Instant) {
        if !should_exit_catch_up(snapshot) {
            self.below_exit_threshold_since = None;
            return;
        }

        match self.below_exit_threshold_since {
            Some(since) if now.saturating_duration_since(since) >= EXIT_HOLD => {
                self.mode = ChunkingMode::Smooth;
                self.below_exit_threshold_since = None;
                self.last_catch_up_exit_at = Some(now);
            }
            Some(_) => {}
            None => self.below_exit_threshold_since = Some(now),
        }
    }

    fn note_catch_up_exit(&mut self, now: Instant) {
        if self.mode == ChunkingMode::CatchUp {
            self.last_catch_up_exit_at = Some(now);
        }
    }

    fn reentry_hold_active(&self, now: Instant) -> bool {
        self.last_catch_up_exit_at
            .is_some_and(|exit| now.saturating_duration_since(exit) < REENTER_CATCH_UP_HOLD)
    }
}

fn should_enter_catch_up(snapshot: QueueSnapshot) -> bool {
    snapshot.queued_lines >= ENTER_QUEUE_DEPTH_LINES
        || snapshot
            .oldest_age
            .is_some_and(|oldest| oldest >= ENTER_OLDEST_AGE)
}

fn should_exit_catch_up(snapshot: QueueSnapshot) -> bool {
    snapshot.queued_lines <= EXIT_QUEUE_DEPTH_LINES
        && snapshot
            .oldest_age
            .is_some_and(|oldest| oldest <= EXIT_OLDEST_AGE)
}

fn is_severe_backlog(snapshot: QueueSnapshot) -> bool {
    snapshot.queued_lines >= SEVERE_QUEUE_DEPTH_LINES
        || snapshot
            .oldest_age
            .is_some_and(|oldest| oldest >= SEVERE_OLDEST_AGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(queued_lines: usize, oldest_age_ms: u64) -> QueueSnapshot {
        QueueSnapshot {
            queued_lines,
            oldest_age: Some(Duration::from_millis(oldest_age_ms)),
        }
    }

    #[test]
    fn smooth_mode_drains_one_line() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let decision = policy.decide(snapshot(3, 10), Instant::now());
        assert_eq!(decision.mode, ChunkingMode::Smooth);
        assert!(!decision.entered_catch_up);
        assert_eq!(decision.drain_plan, DrainPlan::Single);
    }

    #[test]
    fn backlog_enters_catch_up_by_depth() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let decision = policy.decide(snapshot(8, 10), Instant::now());
        assert_eq!(decision.mode, ChunkingMode::CatchUp);
        assert!(decision.entered_catch_up);
        assert_eq!(decision.drain_plan, DrainPlan::Batch(8));
    }

    #[test]
    fn backlog_enters_catch_up_by_age() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let decision = policy.decide(snapshot(2, 120), Instant::now());
        assert_eq!(decision.mode, ChunkingMode::CatchUp);
        assert!(decision.entered_catch_up);
        assert_eq!(decision.drain_plan, DrainPlan::Batch(2));
    }

    #[test]
    fn catch_up_exits_after_hold() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let t0 = Instant::now();
        policy.decide(snapshot(8, 10), t0);
        let held = policy.decide(snapshot(2, 40), t0 + Duration::from_millis(200));
        assert_eq!(held.mode, ChunkingMode::CatchUp);
        let exited = policy.decide(snapshot(2, 40), t0 + Duration::from_millis(460));
        assert_eq!(exited.mode, ChunkingMode::Smooth);
        assert_eq!(exited.drain_plan, DrainPlan::Single);
    }

    #[test]
    fn idle_resets_to_smooth_and_holds_reentry() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let t0 = Instant::now();
        policy.decide(snapshot(8, 10), t0);
        let idle = policy.decide(
            QueueSnapshot {
                queued_lines: 0,
                oldest_age: None,
            },
            t0 + Duration::from_millis(20),
        );
        assert_eq!(idle.mode, ChunkingMode::Smooth);
        let held = policy.decide(snapshot(8, 20), t0 + Duration::from_millis(120));
        assert_eq!(held.mode, ChunkingMode::Smooth);
        let reentered = policy.decide(snapshot(8, 20), t0 + Duration::from_millis(320));
        assert_eq!(reentered.mode, ChunkingMode::CatchUp);
    }

    #[test]
    fn severe_backlog_bypasses_reentry_hold() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let t0 = Instant::now();
        policy.decide(snapshot(8, 10), t0);
        policy.decide(
            QueueSnapshot {
                queued_lines: 0,
                oldest_age: None,
            },
            t0 + Duration::from_millis(20),
        );
        let severe = policy.decide(snapshot(64, 20), t0 + Duration::from_millis(120));
        assert_eq!(severe.mode, ChunkingMode::CatchUp);
        assert_eq!(severe.drain_plan, DrainPlan::Batch(64));
    }
}
