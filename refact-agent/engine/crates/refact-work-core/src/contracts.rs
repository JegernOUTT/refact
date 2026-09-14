use crate::identity::{RunKey, RuntimeKind, WorkKey, WorkScope};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkPhase {
    Queued,
    Admitted,
    Running,
    Parked,
    Waiting,
    Finished,
    Failed,
    Cancelled,
}

impl WorkPhase {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            WorkPhase::Finished | WorkPhase::Failed | WorkPhase::Cancelled
        )
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self,
            WorkPhase::Admitted | WorkPhase::Running | WorkPhase::Parked | WorkPhase::Waiting
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkOrigin {
    User,
    Subchat,
    Scheduler,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    Idle,
    Recent,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkSummary {
    pub work: WorkKey,
    pub run: Option<RunKey>,
    pub scope: WorkScope,
    pub phase: WorkPhase,
    pub origin: WorkOrigin,
    pub title: String,
    pub depth: usize,
    pub parent: Option<WorkKey>,
    pub updated_at_secs: u64,
    pub activity: ActivityState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl WorkSummary {
    pub fn new(
        work: WorkKey,
        scope: WorkScope,
        phase: WorkPhase,
        title: impl Into<String>,
    ) -> Self {
        WorkSummary {
            work,
            run: None,
            scope,
            phase,
            origin: WorkOrigin::User,
            title: title.into(),
            depth: 0,
            parent: None,
            updated_at_secs: 0,
            activity: ActivityState::Idle,
            detail: None,
        }
    }

    pub fn runtime(&self) -> RuntimeKind {
        self.work.runtime
    }

    pub fn classify_activity(&mut self, now_secs: u64, recent_window_secs: u64) {
        let age = now_secs.saturating_sub(self.updated_at_secs);
        self.activity = if self.phase.is_terminal() {
            ActivityState::Idle
        } else if age <= recent_window_secs {
            ActivityState::Recent
        } else {
            ActivityState::Stale
        };
    }
}

pub trait WorkerProjection {
    fn runtime(&self) -> RuntimeKind;
    fn list_work(&self) -> Vec<WorkSummary>;
    fn find_work(&self, work: &WorkKey) -> Option<WorkSummary> {
        self.list_work().into_iter().find(|s| &s.work == work)
    }
}
