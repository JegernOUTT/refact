//! Deterministic, bounded projection of an approved hidden base plus its
//! append-only deltas.
//!
//! A checkpoint here is a *projection*, never an edit: the approved base bytes
//! are copied verbatim, every delta stays on disk, and the folded range is
//! replaced only in the rendered text. The function is pure, so the same base
//! and deltas always yield byte-identical output.

use refact_core::chat_types::ChatMessage;
use serde::{Deserialize, Serialize};

pub const DEFAULT_CHECKPOINT_DELTA_THRESHOLD: usize = 24;
pub const DEFAULT_PROJECTION_MAX_CHARS: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionLimits {
    pub checkpoint_after_deltas: usize,
    pub max_chars: usize,
}

impl Default for ProjectionLimits {
    fn default() -> Self {
        Self {
            checkpoint_after_deltas: DEFAULT_CHECKPOINT_DELTA_THRESHOLD,
            max_chars: DEFAULT_PROJECTION_MAX_CHARS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectedRole {
    Plan,
    Goal,
}

impl ProjectedRole {
    pub fn updates_heading(self) -> &'static str {
        match self {
            ProjectedRole::Plan => "## Plan updates",
            ProjectedRole::Goal => "## Goal updates",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            ProjectedRole::Plan => "plan",
            ProjectedRole::Goal => "goal",
        }
    }
}

/// The approved base as stored on disk. Borrowed so a projection can never
/// rewrite it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovedBase<'a> {
    pub message_id: &'a str,
    pub mode: &'a str,
    pub version: u32,
    pub created_at_ms: u64,
    pub body: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectedDelta<'a> {
    pub seq: u64,
    pub note: &'a str,
}

/// Which revision the checkpoint folds up to, and the provenance of the
/// approval it was derived from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointProvenance {
    pub revision: u64,
    pub folded_deltas: usize,
    pub approved_message_id: String,
    pub approved_version: u32,
    pub approved_by: String,
    pub approved_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleProjection {
    pub text: String,
    pub delta_count: usize,
    pub checkpoint: Option<CheckpointProvenance>,
}

const BASE_SEPARATOR: &str = "\n\n---\n\n";
const NOTE_SEPARATOR: &str = "\n\n";

/// Revision of a delta event: the recorded `seq`, falling back to its position
/// for legacy trajectories that stored no seq.
pub fn delta_revision(message: &ChatMessage, index: usize) -> u64 {
    message
        .extra
        .get("event")
        .and_then(|event| event.get("payload"))
        .and_then(|payload| payload.get("seq"))
        .and_then(|seq| seq.as_u64())
        .unwrap_or(index as u64 + 1)
}

pub fn project_role(
    role: ProjectedRole,
    base: &ApprovedBase<'_>,
    deltas: &[ProjectedDelta<'_>],
    limits: ProjectionLimits,
) -> RoleProjection {
    let delta_count = deltas.len();
    if delta_count == 0 {
        return RoleProjection {
            text: base.body.to_string(),
            delta_count,
            checkpoint: None,
        };
    }
    let full = format!(
        "{}{}{}{}{}",
        base.body,
        BASE_SEPARATOR,
        role.updates_heading(),
        NOTE_SEPARATOR,
        join_notes(deltas)
    );
    if delta_count <= limits.checkpoint_after_deltas && full.chars().count() <= limits.max_chars {
        return RoleProjection {
            text: full,
            delta_count,
            checkpoint: None,
        };
    }
    checkpointed_projection(role, base, deltas, limits)
}

fn checkpointed_projection(
    role: ProjectedRole,
    base: &ApprovedBase<'_>,
    deltas: &[ProjectedDelta<'_>],
    limits: ProjectionLimits,
) -> RoleProjection {
    let delta_count = deltas.len();
    let base_chars = base.body.chars().count() + BASE_SEPARATOR.chars().count();
    let max_kept = limits.checkpoint_after_deltas.min(delta_count);
    let mut kept = 0usize;
    let mut kept_chars = 0usize;
    for next in 1..=max_kept {
        let note_chars = deltas[delta_count - next].note.chars().count();
        let grown = kept_chars + note_chars + if next == 1 { 0 } else { 2 };
        let folded = delta_count - next;
        let header = checkpoint_header(role, base, deltas, folded, delta_count);
        let total = base_chars + header.chars().count() + 2 + grown;
        if total > limits.max_chars {
            break;
        }
        kept = next;
        kept_chars = grown;
    }
    let folded = delta_count - kept;
    let mut text = format!(
        "{}{}{}",
        base.body,
        BASE_SEPARATOR,
        checkpoint_header(role, base, deltas, folded, delta_count)
    );
    if kept > 0 {
        text.push_str(NOTE_SEPARATOR);
        text.push_str(&join_notes(&deltas[folded..]));
    }
    RoleProjection {
        text,
        delta_count,
        checkpoint: Some(CheckpointProvenance {
            revision: revision_at(deltas, folded),
            folded_deltas: folded,
            approved_message_id: base.message_id.to_string(),
            approved_version: base.version,
            approved_by: base.mode.to_string(),
            approved_at_ms: base.created_at_ms,
        }),
    }
}

fn checkpoint_header(
    role: ProjectedRole,
    base: &ApprovedBase<'_>,
    deltas: &[ProjectedDelta<'_>],
    folded: usize,
    total: usize,
) -> String {
    format!(
        "{} (checkpoint at revision {} of {})\n\n[checkpoint: {} earlier {} updates folded; \
approved base v{} installed by {} at {}; the full delta history stays on disk and is available \
from get_{}(include_history=true)]",
        role.updates_heading(),
        revision_at(deltas, folded),
        total,
        folded,
        role.noun(),
        base.version,
        base.mode,
        base.created_at_ms,
        role.noun(),
    )
}

fn revision_at(deltas: &[ProjectedDelta<'_>], folded: usize) -> u64 {
    if folded == 0 {
        0
    } else {
        deltas[folded - 1].seq
    }
}

fn join_notes(deltas: &[ProjectedDelta<'_>]) -> String {
    deltas
        .iter()
        .map(|delta| delta.note)
        .collect::<Vec<_>>()
        .join(NOTE_SEPARATOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NON_GOAL: &str = "Non-goal: never rewrite the approved base.";

    fn base() -> ApprovedBase<'static> {
        ApprovedBase {
            message_id: "base-1",
            mode: "task_planner",
            version: 3,
            created_at_ms: 1_700_000_000_000,
            body: "# Approved plan\n\nShip M5.\n\nNon-goal: never rewrite the approved base.",
        }
    }

    fn notes(count: usize) -> Vec<String> {
        (1..=count)
            .map(|seq| format!("- update {seq}: touched another file"))
            .collect()
    }

    fn deltas<'a>(notes: &'a [String]) -> Vec<ProjectedDelta<'a>> {
        notes
            .iter()
            .enumerate()
            .map(|(index, note)| ProjectedDelta {
                seq: index as u64 + 1,
                note,
            })
            .collect()
    }

    #[test]
    fn plan_projection_without_deltas_is_the_approved_base_verbatim() {
        let projection = project_role(
            ProjectedRole::Plan,
            &base(),
            &[],
            ProjectionLimits::default(),
        );

        assert_eq!(projection.text, base().body);
        assert_eq!(projection.delta_count, 0);
        assert!(projection.checkpoint.is_none());
    }

    #[test]
    fn plan_projection_below_the_threshold_matches_plain_synthesis() {
        let notes = notes(3);
        let projection = project_role(
            ProjectedRole::Plan,
            &base(),
            &deltas(&notes),
            ProjectionLimits::default(),
        );

        assert_eq!(
            projection.text,
            format!(
                "{}\n\n---\n\n## Plan updates\n\n{}",
                base().body,
                notes.join("\n\n")
            )
        );
        assert!(projection.checkpoint.is_none());
    }

    #[test]
    fn plan_projection_is_deterministic_across_repeated_calls() {
        let notes = notes(200);
        let deltas = deltas(&notes);
        let limits = ProjectionLimits {
            checkpoint_after_deltas: 8,
            max_chars: 4_096,
        };

        let first = project_role(ProjectedRole::Plan, &base(), &deltas, limits);
        for _ in 0..5 {
            assert_eq!(
                project_role(ProjectedRole::Plan, &base(), &deltas, limits),
                first
            );
        }
    }

    #[test]
    fn plan_projection_bounds_two_hundred_deltas_and_keeps_approved_constraints() {
        let notes = notes(200);
        let deltas = deltas(&notes);
        let limits = ProjectionLimits {
            checkpoint_after_deltas: 12,
            max_chars: 2_048,
        };

        let projection = project_role(ProjectedRole::Plan, &base(), &deltas, limits);
        let checkpoint = projection.checkpoint.expect("checkpoint");

        assert!(projection.text.chars().count() <= limits.max_chars);
        assert!(projection.text.contains(NON_GOAL));
        assert!(projection.text.starts_with(base().body));
        assert_eq!(projection.delta_count, 200);
        assert_eq!(checkpoint.folded_deltas + 12, 200);
        assert_eq!(checkpoint.revision, 188);
        assert_eq!(checkpoint.approved_version, 3);
        assert_eq!(checkpoint.approved_by, "task_planner");
        assert_eq!(checkpoint.approved_at_ms, 1_700_000_000_000);
        assert_eq!(checkpoint.approved_message_id, "base-1");
        assert!(projection.text.ends_with(&notes[199]));
    }

    #[test]
    fn plan_projection_never_rewrites_or_truncates_the_approved_base() {
        let huge: String = "b".repeat(9_000);
        let base = ApprovedBase {
            body: &huge,
            ..base()
        };
        let notes = notes(50);
        let projection = project_role(
            ProjectedRole::Plan,
            &base,
            &deltas(&notes),
            ProjectionLimits {
                checkpoint_after_deltas: 4,
                max_chars: 1_024,
            },
        );

        assert!(projection.text.starts_with(&huge));
        assert!(!projection.text.contains("[truncated:"));
        assert_eq!(base.body, huge);
    }

    #[test]
    fn goal_projection_uses_goal_wording_and_folds_the_same_way() {
        let notes = notes(40);
        let projection = project_role(
            ProjectedRole::Goal,
            &base(),
            &deltas(&notes),
            ProjectionLimits {
                checkpoint_after_deltas: 5,
                max_chars: 4_096,
            },
        );

        assert!(projection
            .text
            .contains("## Goal updates (checkpoint at revision 35 of 40)"));
        assert!(projection.text.contains("get_goal(include_history=true)"));
        assert_eq!(projection.checkpoint.unwrap().folded_deltas, 35);
    }

    #[test]
    fn plan_projection_checkpoints_on_the_byte_cap_even_below_the_delta_threshold() {
        let long = "x".repeat(900);
        let notes = vec![long.clone(), long.clone(), long];
        let projection = project_role(
            ProjectedRole::Plan,
            &base(),
            &deltas(&notes),
            ProjectionLimits {
                checkpoint_after_deltas: 24,
                max_chars: 1_600,
            },
        );

        assert!(projection.checkpoint.is_some());
        assert!(projection.text.chars().count() <= 1_600);
        assert!(projection.text.contains(NON_GOAL));
    }
}
