//! Wire types for local work discovery (M1-03) and the significant-change
//! context capsule (M1-08/M1-09).
//!
//! Every cap in this module comes from [`refact_work_core::settings::CoordinationSettings`];
//! nothing here defines its own limit constants.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use refact_work_core::identity::{RuntimeKind, WorkKey};

/// What kind of work a catalog row describes.
///
/// The catalog is "same-project cross-root": top-level chats, background
/// agents, task planners and board agents all land in one list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkKindTag {
    /// A top-level user chat.
    Chat,
    /// A background subagent / delegate.
    Subagent,
    /// A chat carrying planner task metadata.
    TaskPlanner,
    /// A chat carrying non-planner task metadata (board executor).
    BoardAgent,
}

impl WorkKindTag {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkKindTag::Chat => "chat",
            WorkKindTag::Subagent => "subagent",
            WorkKindTag::TaskPlanner => "task_planner",
            WorkKindTag::BoardAgent => "board_agent",
        }
    }

    pub const ALL: [WorkKindTag; 4] = [
        WorkKindTag::Chat,
        WorkKindTag::Subagent,
        WorkKindTag::TaskPlanner,
        WorkKindTag::BoardAgent,
    ];
}

/// Coarse lifecycle state shared by trajectories and background agents.
///
/// Deliberately coarse: transitions between these values are the ones that
/// count as significant changes for the auto-context capsule. Token streaming,
/// step counters and tool chatter never move this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStateTag {
    /// Enqueued, not started yet.
    Queued,
    /// Actively generating or executing tools.
    Running,
    /// Explicitly paused / snoozed / deferred.
    Parked,
    /// Blocked on a human or an IDE round-trip.
    #[serde(rename = "waiting_user", alias = "waiting_approval")]
    WaitingApproval,
    /// Sitting idle with nothing queued.
    Idle,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
    /// Cancelled by a user or a parent.
    Cancelled,
    /// Lost to a restart.
    Interrupted,
    /// Deliberately closed; terminal, but the saved child can be resumed.
    Closed,
    /// The live record could not be read without blocking; row is stale.
    Unknown,
}

impl WorkStateTag {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkStateTag::Queued => "queued",
            WorkStateTag::Running => "running",
            WorkStateTag::Parked => "parked",
            WorkStateTag::WaitingApproval => "waiting_user",
            WorkStateTag::Idle => "idle",
            WorkStateTag::Completed => "completed",
            WorkStateTag::Failed => "failed",
            WorkStateTag::Cancelled => "cancelled",
            WorkStateTag::Interrupted => "interrupted",
            WorkStateTag::Closed => "closed",
            WorkStateTag::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<WorkStateTag> {
        Some(match value {
            "queued" => WorkStateTag::Queued,
            "running" => WorkStateTag::Running,
            "parked" => WorkStateTag::Parked,
            "waiting_user" | "waiting_approval" => WorkStateTag::WaitingApproval,
            "idle" => WorkStateTag::Idle,
            "completed" => WorkStateTag::Completed,
            "failed" => WorkStateTag::Failed,
            "cancelled" => WorkStateTag::Cancelled,
            "interrupted" => WorkStateTag::Interrupted,
            "closed" => WorkStateTag::Closed,
            "unknown" => WorkStateTag::Unknown,
            _ => return None,
        })
    }

    /// Terminal states never transition again.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            WorkStateTag::Completed
                | WorkStateTag::Failed
                | WorkStateTag::Cancelled
                | WorkStateTag::Interrupted
                | WorkStateTag::Closed
        )
    }

    /// States that mean "this work is live right now".
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            WorkStateTag::Queued | WorkStateTag::Running | WorkStateTag::WaitingApproval
        )
    }

    /// Sort weight: the most operationally interesting rows come first so that
    /// a truncated capsule keeps the rows that matter.
    pub fn priority(&self) -> u8 {
        match self {
            WorkStateTag::WaitingApproval => 0,
            WorkStateTag::Running => 1,
            WorkStateTag::Queued => 2,
            WorkStateTag::Parked => 3,
            WorkStateTag::Failed => 4,
            WorkStateTag::Interrupted => 5,
            WorkStateTag::Idle => 6,
            WorkStateTag::Cancelled => 7,
            WorkStateTag::Closed => 8,
            WorkStateTag::Completed => 9,
            WorkStateTag::Unknown => 10,
        }
    }

    pub const ALL: [WorkStateTag; 11] = [
        WorkStateTag::Queued,
        WorkStateTag::Running,
        WorkStateTag::Parked,
        WorkStateTag::WaitingApproval,
        WorkStateTag::Idle,
        WorkStateTag::Completed,
        WorkStateTag::Failed,
        WorkStateTag::Cancelled,
        WorkStateTag::Interrupted,
        WorkStateTag::Closed,
        WorkStateTag::Unknown,
    ];
}

/// Whether a work root is a plain project checkout or a registered worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkRootKind {
    Project,
    Worktree,
}

impl WorkRootKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkRootKind::Project => "project",
            WorkRootKind::Worktree => "worktree",
        }
    }
}

/// Whether the directory behind a work root still exists on disk.
///
/// A removed or merged checkout keeps its registry record and stays
/// inspectable, but it is no longer an active location. `Unknown` is the
/// fail-closed answer: a probe that could not conclude (permission denied, IO
/// error) never reports `Present` or `Missing`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum RootPresence {
    /// The checkout directory was observed on disk.
    Present,
    /// The recorded path is gone, or is no longer a directory.
    Missing,
    /// Not probed, or the probe could not conclude.
    #[default]
    Unknown,
}

impl RootPresence {
    pub fn as_str(&self) -> &'static str {
        match self {
            RootPresence::Present => "present",
            RootPresence::Missing => "missing",
            RootPresence::Unknown => "unknown",
        }
    }

    /// True only when the checkout was positively observed. A root that is
    /// missing or unproven is never counted as an active location.
    pub fn is_active_location(&self) -> bool {
        matches!(self, RootPresence::Present)
    }

    pub const ALL: [RootPresence; 3] = [
        RootPresence::Present,
        RootPresence::Missing,
        RootPresence::Unknown,
    ];
}

/// A location work happens in: the active worktree, or the source project
/// checkout when the work is not worktree-scoped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRootRef {
    /// Stable id: the worktree id, or the project hash for a plain checkout.
    pub id: String,
    pub kind: WorkRootKind,
    /// Absolute root path. Redacted (`None`) when privacy withholds the row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Project identity shared by every root of the same source checkout.
    /// This is what makes the catalog "same project, cross root".
    pub project: String,
    /// Absolute source workspace root. Redacted when privacy withholds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    /// Whether the checkout is still on disk. Defaults to `Unknown` so records
    /// written before this field existed load without claiming to be live.
    #[serde(default)]
    pub presence: RootPresence,
}

impl WorkRootRef {
    /// Drop everything that leaks a filesystem location, including whether the
    /// withheld path exists.
    pub fn redacted(&self) -> WorkRootRef {
        WorkRootRef {
            id: self.id.clone(),
            kind: self.kind,
            path: None,
            branch: None,
            project: self.project.clone(),
            source_path: None,
            presence: RootPresence::Unknown,
        }
    }
}

/// Outcome of the destination privacy check for one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disclosure {
    /// Everything about this row may be shown to the destination.
    Full,
    /// The row exists, but its title, summary, paths and location are withheld.
    Withheld,
}

impl Disclosure {
    pub const ALL: [Disclosure; 2] = [Disclosure::Full, Disclosure::Withheld];

    pub fn as_str(&self) -> &'static str {
        match self {
            Disclosure::Full => "full",
            Disclosure::Withheld => "withheld",
        }
    }
}

/// Placeholder substituted for any field a destination may not see.
pub const WITHHELD_PLACEHOLDER: &str = "<withheld by privacy policy>";

/// One unit of work in the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRow {
    pub key: WorkKey,
    #[serde(default, flatten)]
    pub metadata: WorkMetadata,
    pub kind: WorkKindTag,
    pub state: WorkStateTag,
    /// Needs a human: waiting on approval/input, failed, or has open questions.
    pub attention: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub root: WorkRootRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<String>,
    /// Files the work declared it intends to touch (target_files).
    /// Declared targets and observed edits are tracked separately on purpose:
    /// the file-activity source is not wired yet, so coverage is partial.
    #[serde(default)]
    pub declared_files: Vec<String>,
    /// Files the work is known to have edited.
    #[serde(default)]
    pub observed_files: Vec<String>,
    #[serde(default)]
    pub queue_size: usize,
    #[serde(default)]
    pub step_count: u32,
    #[serde(default)]
    pub open_questions: usize,
    /// Wall-clock milliseconds of the last known change.
    pub updated_at_ms: u64,
    /// True when the live record was locked and the row was built from
    /// whatever was available without blocking.
    #[serde(default)]
    pub stale: bool,
    pub disclosure: Disclosure,
    #[serde(skip)]
    pub(crate) source_records: Vec<refact_privacy::FileRecord>,
    #[serde(skip)]
    pub(crate) provenance_complete: bool,
}

impl WorkRow {
    /// Every path this row would disclose, used as the privacy audit input.
    pub fn disclosed_paths(&self) -> Vec<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        if let Some(path) = &self.root.path {
            out.insert(path.clone());
        }
        if let Some(path) = &self.root.source_path {
            out.insert(path.clone());
        }
        out.extend(self.declared_files.iter().cloned());
        out.extend(self.observed_files.iter().cloned());
        out.extend(self.metadata.observed_reads.iter().cloned());
        out.into_iter().collect()
    }

    /// Strip everything a destination may not see, keeping only the fact that
    /// work exists and its coarse state.
    pub fn withheld(&self) -> WorkRow {
        WorkRow {
            key: self.key.clone(),
            metadata: WorkMetadata::default(),
            kind: self.kind,
            state: self.state,
            attention: self.attention,
            title: Some(WITHHELD_PLACEHOLDER.to_string()),
            summary: None,
            root: self.root.redacted(),
            parent_chat_id: self.parent_chat_id.clone(),
            root_chat_id: self.root_chat_id.clone(),
            chat_id: self.chat_id.clone(),
            model: self.model.clone(),
            current_tool: None,
            declared_files: Vec::new(),
            observed_files: Vec::new(),
            queue_size: self.queue_size,
            step_count: self.step_count,
            open_questions: self.open_questions,
            updated_at_ms: self.updated_at_ms,
            stale: self.stale,
            disclosure: Disclosure::Withheld,
            source_records: self.source_records.clone(),
            provenance_complete: self.provenance_complete,
        }
    }

    /// Deterministic ordering key: attention first, then state priority, then
    /// most recently changed, then the work key for a total order.
    pub fn sort_key(&self) -> (u8, u8, std::cmp::Reverse<u64>, String) {
        (
            if self.attention { 0 } else { 1 },
            self.state.priority(),
            std::cmp::Reverse(self.updated_at_ms),
            self.key.to_string(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Live,
    Recent,
    Stale,
    #[default]
    Unknown,
}

impl Freshness {
    pub const ALL: [Freshness; 4] = [
        Freshness::Live,
        Freshness::Recent,
        Freshness::Stale,
        Freshness::Unknown,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Freshness::Live => "live",
            Freshness::Recent => "recent",
            Freshness::Stale => "stale",
            Freshness::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkMetadata {
    pub run: Option<refact_work_core::RunKey>,
    /// The admission class this run was launched with, so a queued row can say
    /// why it is waiting. Absent for work this engine no longer schedules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<refact_work_core::WorkPriority>,
    pub task_id: Option<String>,
    pub card_id: Option<String>,
    pub runtime_status: Option<String>,
    pub last_activity_at: Option<u64>,
    pub last_progress_at: Option<u64>,
    pub freshness: Freshness,
    pub observed_reads: Vec<String>,
    pub measured_tokens: Option<u64>,
    pub measured_cost_cents: Option<u64>,
    #[serde(skip)]
    pub source_revision: u64,
}

/// Which slice of the catalog a caller wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OverviewScope {
    /// Every root belonging to the current source project.
    #[default]
    Project,
    /// A single conversation root and its descendants.
    Root,
}

impl OverviewScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            OverviewScope::Project => "project",
            OverviewScope::Root => "root",
        }
    }

    pub fn parse(value: &str) -> Option<OverviewScope> {
        match value {
            "project" => Some(OverviewScope::Project),
            "root" => Some(OverviewScope::Root),
            _ => None,
        }
    }
}

/// `work_overview` request. Rows/cursor are clamped against
/// `CoordinationSettings::overview_default_rows` / `overview_max_rows`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkOverviewRequest {
    #[serde(default)]
    pub scope: OverviewScope,
    /// Required when `scope == Root`: the work-root id to restrict to.
    #[serde(skip)]
    pub root: Option<String>,
    /// Case-insensitive substring over declared/observed paths and the root path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Keep only these states. Empty/absent means every state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub states: Option<Vec<WorkStateTag>>,
    #[serde(default, rename = "limit", skip_serializing_if = "Option::is_none")]
    pub rows: Option<usize>,
    /// Opaque cursor returned by a previous page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// `work_overview` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkOverview {
    pub snapshot_id: String,
    pub seq: u64,
    pub project: String,
    pub coverage: String,
    pub page_unit: String,
    pub scope: OverviewScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    pub generated_at_ms: u64,
    #[serde(default)]
    pub snapshot_observed_at_ms: u64,
    #[serde(default)]
    pub snapshot_stale: bool,
    pub rows: Vec<WorkRow>,
    /// Rows matching the filters before paging.
    pub total: usize,
    pub returned: usize,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Work roots represented on this page; inventory is separately paginated.
    pub roots: Vec<WorkRootRef>,
    /// How many returned rows were reduced by the privacy gate.
    pub withheld_rows: usize,
    /// Non-fatal problems (locked sessions, unresolvable roots).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degraded: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub provenance_truncated: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_provenance_records: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_provenance_bytes: usize,
    #[serde(skip)]
    pub(crate) source_records: Vec<refact_privacy::FileRecord>,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

/// Tagged selector for `work_inspect`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkInspectSelector {
    /// One work item by id.
    Work {
        work: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        runtime: Option<RuntimeKind>,
    },
    /// Declared vs observed files for one work item, plus overlap with peers.
    Files { work: String },
    /// One work root and the work running inside it.
    Worktree { worktree: String },
    /// Every work root of the current project.
    ProjectWorktrees,
}

/// `work_inspect` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkInspection {
    Work {
        row: WorkRow,
        /// Direct children (subagents spawned by this work).
        children: Vec<WorkRow>,
    },
    Files {
        key: WorkKey,
        declared_files: Vec<String>,
        observed_files: Vec<String>,
        /// Peers touching at least one of the same paths.
        overlaps: Vec<FileOverlap>,
        /// True when privacy withheld the file lists.
        withheld: bool,
    },
    Worktree {
        root: WorkRootRef,
        rows: Vec<WorkRow>,
    },
    ProjectWorktrees {
        project: String,
        roots: Vec<WorkRootRef>,
    },
}

/// Two work items competing for the same files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileOverlap {
    pub key: WorkKey,
    pub state: WorkStateTag,
    pub paths: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(state: WorkStateTag, attention: bool, updated_at_ms: u64, id: &str) -> WorkRow {
        WorkRow {
            key: WorkKey::trajectory(id),
            metadata: Default::default(),
            kind: WorkKindTag::Chat,
            state,
            attention,
            title: Some("t".to_string()),
            summary: None,
            root: WorkRootRef {
                id: "root".to_string(),
                kind: WorkRootKind::Project,
                path: Some("/p".to_string()),
                branch: None,
                project: "proj".to_string(),
                source_path: Some("/p".to_string()),
                presence: RootPresence::Present,
            },
            parent_chat_id: None,
            root_chat_id: None,
            chat_id: Some(id.to_string()),
            model: None,
            current_tool: Some("cat".to_string()),
            declared_files: vec!["/p/a.rs".to_string()],
            observed_files: vec!["/p/b.rs".to_string()],
            queue_size: 0,
            step_count: 0,
            open_questions: 0,
            updated_at_ms,
            stale: false,
            disclosure: Disclosure::Full,
            source_records: Vec::new(),
            provenance_complete: true,
        }
    }

    #[test]
    fn withholding_removes_every_path_and_free_text_field() {
        let withheld = row(WorkStateTag::Running, false, 1, "a").withheld();

        assert_eq!(withheld.title.as_deref(), Some(WITHHELD_PLACEHOLDER));
        assert_eq!(withheld.summary, None);
        assert_eq!(withheld.current_tool, None);
        assert!(withheld.declared_files.is_empty());
        assert!(withheld.observed_files.is_empty());
        assert_eq!(withheld.root.path, None);
        assert_eq!(withheld.root.source_path, None);
        assert_eq!(withheld.root.branch, None);
        assert_eq!(withheld.disclosure, Disclosure::Withheld);
        // Identity and coarse state survive so callers still see that work exists.
        assert_eq!(withheld.key.id, "a");
        assert_eq!(withheld.state, WorkStateTag::Running);
        assert_eq!(withheld.root.project, "proj");
    }

    #[test]
    fn selector_uses_a_tagged_representation() {
        let json = serde_json::to_value(WorkInspectSelector::Files {
            work: "chat-1".to_string(),
        })
        .unwrap();
        assert_eq!(json["type"], "files");
        assert_eq!(json["work"], "chat-1");

        let parsed: WorkInspectSelector =
            serde_json::from_value(serde_json::json!({"type": "project_worktrees"})).unwrap();
        assert_eq!(parsed, WorkInspectSelector::ProjectWorktrees);
    }

    #[test]
    fn a_root_stored_before_presence_existed_loads_without_claiming_to_be_live() {
        let legacy = serde_json::json!({
            "id": "wt-1",
            "kind": "worktree",
            "path": "/p/wt-1",
            "project": "proj",
            "source_path": "/p"
        });
        let root: WorkRootRef = serde_json::from_value(legacy).unwrap();

        assert_eq!(root.presence, RootPresence::Unknown);
        assert!(!root.presence.is_active_location());
    }
}
