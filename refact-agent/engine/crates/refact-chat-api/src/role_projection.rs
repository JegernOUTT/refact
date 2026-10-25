//! Deterministic, bounded projection of an approved hidden base plus its
//! append-only deltas.
//!
//! A checkpoint here is a *projection*, never an edit: the approved base bytes
//! are copied verbatim, every delta stays on disk, and the folded range is
//! replaced only in the rendered text. The function is pure, so the same base
//! and deltas always yield byte-identical output.

use refact_core::chat_types::ChatMessage;
use serde::{Deserialize, Serialize};

use crate::internal_roles::{self, EventSubkind};

pub const DEFAULT_CHECKPOINT_DELTA_THRESHOLD: usize = 24;
pub const DEFAULT_PROJECTION_MAX_CHARS: usize = 32 * 1024;
pub const CHECKPOINT_SOURCE: &str = "context_rebuild.checkpoint";

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

pub fn checkpoint_provenance(message: &ChatMessage) -> Option<CheckpointProvenance> {
    if message.role != internal_roles::EVENT_ROLE {
        return None;
    }
    let event = message.extra.get("event")?;
    if !matches!(
        event.get("subkind").and_then(serde_json::Value::as_str),
        Some("plan_delta" | "goal_delta")
    ) || event.get("source").and_then(serde_json::Value::as_str) != Some(CHECKPOINT_SOURCE)
    {
        return None;
    }
    serde_json::from_value(event.get("payload")?.get("checkpoint")?.clone()).ok()
}

pub fn project_role(
    role: ProjectedRole,
    base: &ApprovedBase<'_>,
    deltas: &[ProjectedDelta<'_>],
    limits: ProjectionLimits,
) -> RoleProjection {
    project_role_with_prefold(role, base, None, deltas, limits)
}

pub fn project_role_with_prefold(
    role: ProjectedRole,
    base: &ApprovedBase<'_>,
    prefold: Option<&CheckpointProvenance>,
    deltas: &[ProjectedDelta<'_>],
    limits: ProjectionLimits,
) -> RoleProjection {
    let prefold_count = prefold.map_or(0, |checkpoint| checkpoint.folded_deltas);
    let delta_count = prefold_count + deltas.len();
    if delta_count == 0 {
        return RoleProjection {
            text: base.body.to_string(),
            delta_count,
            checkpoint: None,
        };
    }
    if prefold.is_none() && !deltas.is_empty() {
        let full = format!(
            "{}{}{}{}{}",
            base.body,
            BASE_SEPARATOR,
            role.updates_heading(),
            NOTE_SEPARATOR,
            join_notes(deltas)
        );
        if delta_count <= limits.checkpoint_after_deltas && full.chars().count() <= limits.max_chars
        {
            return RoleProjection {
                text: full,
                delta_count,
                checkpoint: None,
            };
        }
    }
    checkpointed_projection(role, base, prefold, deltas, limits)
}

fn checkpointed_projection(
    role: ProjectedRole,
    base: &ApprovedBase<'_>,
    prefold: Option<&CheckpointProvenance>,
    deltas: &[ProjectedDelta<'_>],
    limits: ProjectionLimits,
) -> RoleProjection {
    let prefold_count = prefold.map_or(0, |checkpoint| checkpoint.folded_deltas);
    let delta_count = prefold_count + deltas.len();
    let base_chars = base.body.chars().count() + BASE_SEPARATOR.chars().count();
    let max_kept = limits.checkpoint_after_deltas.min(deltas.len());
    let mut kept = 0usize;
    let mut kept_chars = 0usize;
    for next in 1..=max_kept {
        let note_chars = deltas[deltas.len() - next].note.chars().count();
        let grown = kept_chars + note_chars + if next == 1 { 0 } else { 2 };
        let folded_new = deltas.len() - next;
        let folded = prefold_count + folded_new;
        let revision = revision_for_fold(prefold, deltas, folded_new);
        let header = checkpoint_header(role, base, revision, folded, delta_count);
        let total = base_chars + header.chars().count() + 2 + grown;
        if total > limits.max_chars {
            break;
        }
        kept = next;
        kept_chars = grown;
    }
    let folded_new = deltas.len() - kept;
    let folded = prefold_count + folded_new;
    let revision = revision_for_fold(prefold, deltas, folded_new);
    let mut text = format!(
        "{}{}{}",
        base.body,
        BASE_SEPARATOR,
        checkpoint_header(role, base, revision, folded, delta_count)
    );
    if kept > 0 {
        text.push_str(NOTE_SEPARATOR);
        text.push_str(&join_notes(&deltas[folded_new..]));
    }
    RoleProjection {
        text,
        delta_count,
        checkpoint: Some(CheckpointProvenance {
            revision,
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
    revision: u64,
    folded: usize,
    total: usize,
) -> String {
    format!(
        "{} (checkpoint at revision {} of {})\n\n[checkpoint: {} earlier {} updates folded; \
approved base v{} installed by {} at {}; the full delta history stays on disk and is available \
from get_goal(include_history=true)]",
        role.updates_heading(),
        revision,
        total,
        folded,
        role.noun(),
        base.version,
        base.mode,
        base.created_at_ms,
    )
}

fn revision_for_fold(
    prefold: Option<&CheckpointProvenance>,
    deltas: &[ProjectedDelta<'_>],
    folded_new: usize,
) -> u64 {
    if folded_new == 0 {
        prefold.map_or(0, |checkpoint| checkpoint.revision)
    } else {
        revision_at(deltas, folded_new)
    }
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

pub fn fold_role_messages(
    role: ProjectedRole,
    messages: &[ChatMessage],
    limits: ProjectionLimits,
) -> Vec<ChatMessage> {
    let Some((base_index, base_message)) = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            role_base_version(role, message).map(|version| (index, version, message))
        })
        .max_by_key(|(index, version, _)| (*version, *index))
        .map(|(index, _, message)| (index, message))
    else {
        return messages.to_vec();
    };
    let base_meta = base_message.extra.get(role.noun());
    let body = base_message.content.content_text_only();
    let base = ApprovedBase {
        message_id: &base_message.message_id,
        mode: base_meta
            .and_then(|meta| meta.get("mode"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
        version: role_base_version(role, base_message).unwrap_or(1),
        created_at_ms: base_meta
            .and_then(|meta| meta.get("created_at_ms"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        body: &body,
    };
    let checkpoint_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(index, message)| {
            *index > base_index
                && is_role_checkpoint(role, message)
                && checkpoint_provenance(message)
                    .is_some_and(|checkpoint| checkpoint.approved_message_id == base.message_id)
        })
        .map(|(index, _)| index)
        .collect();
    let checkpoint_index = checkpoint_indices.last().copied();
    let prefold = checkpoint_index.and_then(|index| checkpoint_provenance(&messages[index]));
    let delta_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(index, message)| {
            *index > base_index
                && checkpoint_index.map_or(true, |checkpoint_index| *index > checkpoint_index)
                && is_delta_kind(message, role.delta_subkind())
                && (role != ProjectedRole::Goal || !crate::goal_role::is_plan_delta(message))
                && checkpoint_provenance(message).is_none()
        })
        .map(|(index, _)| index)
        .collect();
    let notes: Vec<String> = delta_indices
        .iter()
        .map(|index| messages[*index].content.content_text_only())
        .collect();
    let deltas: Vec<ProjectedDelta<'_>> = delta_indices
        .iter()
        .enumerate()
        .map(|(index, message_index)| ProjectedDelta {
            seq: delta_revision(
                &messages[*message_index],
                prefold
                    .as_ref()
                    .map_or(0, |checkpoint| checkpoint.folded_deltas)
                    + index,
            ),
            note: &notes[index],
        })
        .collect();
    let projection = project_role_with_prefold(role, &base, prefold.as_ref(), &deltas, limits);
    let Some(provenance) = projection.checkpoint else {
        return messages.to_vec();
    };
    let folded_before = prefold
        .as_ref()
        .map_or(0, |checkpoint| checkpoint.folded_deltas);
    let newly_folded = provenance.folded_deltas - folded_before;
    if newly_folded == 0 {
        return messages.to_vec();
    }
    let folded_indices = &delta_indices[..newly_folded];
    let last_folded_index = *folded_indices.last().expect("newly folded deltas");
    let previous_payload = checkpoint_index.map(|index| &messages[index].extra["event"]["payload"]);
    let first_folded_message_id = previous_payload
        .and_then(|payload| payload.get("first_folded_message_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| messages[folded_indices[0]].message_id.clone());
    let checkpoint_message = make_checkpoint_message(
        role,
        &base,
        &provenance,
        first_folded_message_id,
        messages[last_folded_index].message_id.clone(),
    );
    let folded_indices: std::collections::HashSet<usize> = folded_indices.iter().copied().collect();
    let checkpoint_indices: std::collections::HashSet<usize> =
        checkpoint_indices.into_iter().collect();
    let mut output = Vec::with_capacity(messages.len() - newly_folded + 1);
    for (index, message) in messages.iter().enumerate() {
        if index == last_folded_index {
            output.push(checkpoint_message.clone());
        }
        if folded_indices.contains(&index) || checkpoint_indices.contains(&index) {
            continue;
        }
        output.push(message.clone());
    }
    output
}

impl ProjectedRole {
    fn delta_subkind(self) -> &'static str {
        match self {
            ProjectedRole::Plan => "plan_delta",
            ProjectedRole::Goal => "goal_delta",
        }
    }
}

fn role_base_version(role: ProjectedRole, message: &ChatMessage) -> Option<u32> {
    let expected_role = match role {
        ProjectedRole::Plan => "plan",
        ProjectedRole::Goal => "goal",
    };
    if message.role != expected_role {
        return None;
    }
    message
        .extra
        .get(role.noun())?
        .get("version")?
        .as_u64()
        .and_then(|version| u32::try_from(version).ok())
}

fn is_delta_kind(message: &ChatMessage, subkind: &str) -> bool {
    message.role == internal_roles::EVENT_ROLE
        && message
            .extra
            .get("event")
            .and_then(|event| event.get("subkind"))
            .and_then(serde_json::Value::as_str)
            == Some(subkind)
}

fn is_role_checkpoint(role: ProjectedRole, message: &ChatMessage) -> bool {
    is_delta_kind(message, role.delta_subkind()) && checkpoint_provenance(message).is_some()
}

fn make_checkpoint_message(
    role: ProjectedRole,
    base: &ApprovedBase<'_>,
    provenance: &CheckpointProvenance,
    first_folded_message_id: String,
    last_folded_message_id: String,
) -> ChatMessage {
    let (subkind, id_prefix) = match role {
        ProjectedRole::Plan => (EventSubkind::PlanDelta, "plan"),
        ProjectedRole::Goal => (EventSubkind::GoalDelta, "goal"),
    };
    let mut message = internal_roles::event(
        subkind,
        CHECKPOINT_SOURCE,
        serde_json::json!({
            "seq": provenance.revision,
            "checkpoint": provenance,
            "first_folded_message_id": first_folded_message_id,
            "last_folded_message_id": last_folded_message_id,
        }),
        format!(
            "[checkpoint: {} earlier {} updates folded at revision {}; approved base v{} installed by {} at {}; full history: get_goal(include_history=true)]",
            provenance.folded_deltas,
            role.noun(),
            provenance.revision,
            base.version,
            base.mode,
            base.created_at_ms,
        ),
    );
    message.message_id = format!(
        "{id_prefix}-checkpoint-{}-{}",
        base.message_id, provenance.revision
    );
    message
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
