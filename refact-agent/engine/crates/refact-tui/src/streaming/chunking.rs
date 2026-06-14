// Adapted from openai/codex codex-rs/tui, Apache-2.0.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ChunkingMode {
    #[default]
    Smooth,
    CatchUp,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct QueueSnapshot {
    pub(crate) queued_lines: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrainPlan {
    Single,
    Batch(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChunkingDecision {
    pub(crate) mode: ChunkingMode,
    pub(crate) drain_plan: DrainPlan,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct AdaptiveChunkingPolicy {
    mode: ChunkingMode,
}

impl AdaptiveChunkingPolicy {
    pub(crate) fn reset(&mut self) {
        self.mode = ChunkingMode::Smooth;
    }

    pub(crate) fn decide(&mut self, snapshot: QueueSnapshot) -> ChunkingDecision {
        if snapshot.queued_lines == 0 {
            self.mode = ChunkingMode::Smooth;
        } else if snapshot.queued_lines >= 8 {
            self.mode = ChunkingMode::CatchUp;
        } else if self.mode == ChunkingMode::CatchUp && snapshot.queued_lines <= 1 {
            self.mode = ChunkingMode::Smooth;
        }

        let drain_plan = match self.mode {
            ChunkingMode::Smooth => DrainPlan::Single,
            ChunkingMode::CatchUp => DrainPlan::Batch(snapshot.queued_lines),
        };

        ChunkingDecision {
            mode: self.mode,
            drain_plan,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_mode_drains_one_line() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let decision = policy.decide(QueueSnapshot { queued_lines: 3 });
        assert_eq!(decision.mode, ChunkingMode::Smooth);
        assert_eq!(decision.drain_plan, DrainPlan::Single);
    }

    #[test]
    fn backlog_enters_catch_up() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let decision = policy.decide(QueueSnapshot { queued_lines: 8 });
        assert_eq!(decision.mode, ChunkingMode::CatchUp);
        assert_eq!(decision.drain_plan, DrainPlan::Batch(8));
    }
}
