use std::path::Path;

use serde::{Deserialize, Serialize};
use refact_work_core::{CoordinationSettings, WorkKey};
use crate::tasks::types::{AbVariantInfo, BoardCard, TaskBoard};
use super::types::{Disclosure, Freshness, WorkKindTag, WorkMetadata, WorkRow, WorkStateTag};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct BoardProvenance {
    pub digest: String,
    pub records: Vec<refact_privacy::FileRecord>,
    pub complete: bool,
    #[serde(default)]
    pub authority_version: u32,
}

impl BoardProvenance {
    pub fn authority_valid_for(&self, board: &TaskBoard, settings: &CoordinationSettings) -> bool {
        self.authority_version == 1 && self.valid_for(board, settings)
    }

    pub fn valid_for(&self, board: &TaskBoard, settings: &CoordinationSettings) -> bool {
        self.complete
            && self.records.len() <= settings.max_work_provenance_records
            && self.records.iter().fold(0usize, |size, record| {
                size.saturating_add(record.path.len())
                    .saturating_add(record.zone.len())
            }) <= settings.max_work_provenance_bytes
            && board_digest(board).is_some_and(|digest| digest == self.digest)
    }
}

pub(crate) fn board_digest(board: &TaskBoard) -> Option<String> {
    if board.owner_epoch == 0 && board.owner_custody.is_empty() {
        return serde_json::to_string(board)
            .ok()
            .map(|body| crate::file_activity::digest_of(&body));
    }
    let mut projection = board.clone();
    projection.owner_epoch = 0;
    projection.owner_custody.clear();
    serde_json::to_string(&projection)
        .ok()
        .map(|body| crate::file_activity::digest_of(&body))
}

pub(crate) fn write_provenance(
    board: &TaskBoard,
    previous: Option<(&TaskBoard, &BoardProvenance)>,
    source: Option<(Vec<refact_privacy::FileRecord>, bool)>,
) -> BoardProvenance {
    let settings = crate::file_activity::settings();
    let mut provenance = BoardProvenance {
        digest: board_digest(board).unwrap_or_default(),
        records: previous
            .map(|(_, provenance)| provenance.records.clone())
            .unwrap_or_default(),
        complete: previous.is_none_or(|(board, provenance)| provenance.valid_for(board, &settings)),
        authority_version: previous.map_or(1, |(_, provenance)| provenance.authority_version),
    };
    let old_disclosure = previous
        .map(|(board, _)| disclosure_fingerprint(board))
        .unwrap_or_else(|| disclosure_fingerprint(&TaskBoard::default()));
    if old_disclosure != disclosure_fingerprint(board) {
        match source {
            Some((records, complete)) => {
                provenance.records.extend(records);
                provenance.complete &= complete;
            }
            None => provenance.complete = false,
        }
    }
    let mut seen = std::collections::HashSet::new();
    provenance
        .records
        .retain(|record| seen.insert(record.clone()));
    if provenance.records.len() > settings.max_work_provenance_records
        || provenance.records.iter().fold(0usize, |size, record| {
            size.saturating_add(record.path.len())
                .saturating_add(record.zone.len())
        }) > settings.max_work_provenance_bytes
    {
        provenance.complete = false;
        provenance.records.clear();
    }
    provenance
}

pub(crate) async fn actor_provenance(
    gcx: &crate::global_context::GlobalContext,
) -> Result<Option<(Vec<refact_privacy::FileRecord>, bool)>, String> {
    let Some(actor) = crate::file_activity::current_actor() else {
        return Ok(None);
    };
    let settings = crate::file_activity::settings();
    tokio::time::timeout(
        std::time::Duration::from_secs(settings.lock_timeout_secs),
        async {
            let session = gcx
                .chat_sessions
                .read()
                .await
                .get(&actor)
                .cloned()
                .ok_or("Board writer session unavailable")?;
            let session = session.lock().await;
            Ok(Some(
                session
                    .work_provenance
                    .snapshot(session.messages.len(), &settings),
            ))
        },
    )
    .await
    .map_err(|_| "Board writer provenance temporarily unavailable".to_string())?
}

fn disclosure_fingerprint(board: &TaskBoard) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    let mut field = |value: &str| {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    };
    field(
        &serde_json::json!({
            "owner_chat_id": board.owner_chat_id,
            "reviewer_chat_ids": board.reviewer_chat_ids,
            "review_policy": board.review_policy,
            "claims": board.card_claims.iter().map(|(id, claim)| {
                (id, &claim.claimant_chat_id, &claim.claimed_at)
            }).collect::<Vec<_>>(),
            "dependency_overrides": board.dependency_overrides.iter().map(|(id, value)| {
                (id, &value.overridden_by, &value.overridden_at, &value.unsatisfied)
            }).collect::<Vec<_>>(),
        })
        .to_string(),
    );
    for card in &board.cards {
        for value in [&card.id, &card.title, &card.column, &card.created_at] {
            field(value);
        }
        for value in card
            .target_files
            .iter()
            .chain(card.agent_chat_id.iter())
            .chain(card.started_at.iter())
            .chain(card.completed_at.iter())
            .chain(card.agent_worktree.iter())
            .chain(card.agent_branch.iter())
            .chain(card.assignee.iter())
        {
            field(value);
        }
        for member in &card.team_members {
            if let Some(chat_id) = &member.agent_chat_id {
                field(chat_id);
            }
        }
        if let Some(variants) = &card.ab_variants {
            for variant in [&variants.a, &variants.b] {
                field(&variant.agent_id);
                field(&variant.chat_id);
                field(&variant.worktree);
                for value in variant.model.iter().chain(variant.branch.iter()) {
                    field(value);
                }
                if let Some(finish) = &variant.finish {
                    field(if finish.success {
                        "completed"
                    } else {
                        "failed"
                    });
                    field(&finish.completed_at);
                }
            }
        }
    }
    hex::encode(digest.finalize())
}

fn row_count(board: &TaskBoard) -> usize {
    board
        .cards
        .iter()
        .map(|card| {
            if card.ab_variants.is_some() {
                2
            } else if card.team_members.is_empty() {
                1
            } else {
                card.team_members
                    .iter()
                    .filter(|member| member.agent_chat_id.is_some())
                    .count()
                    .max(1)
            }
        })
        .fold(0usize, usize::saturating_add)
}

pub(crate) fn rows_from_board(
    root: &Path,
    task_id: &str,
    board: &TaskBoard,
    provenance: &BoardProvenance,
    settings: &CoordinationSettings,
) -> Option<Vec<WorkRow>> {
    let count = row_count(board);
    if count > settings.max_work_history_rows {
        return None;
    }
    let board_bytes = super::bounds::serialized_size(board, settings.max_work_history_bytes)?;
    let provenance_bytes =
        super::bounds::serialized_size(provenance, settings.max_work_provenance_bytes)?;
    if board_bytes.checked_add(provenance_bytes.checked_mul(count)?)?
        > settings.max_work_history_bytes
    {
        return None;
    }
    let complete = provenance.valid_for(board, settings);
    let mut rows = Vec::new();
    for card in &board.cards {
        if let Some(variants) = &card.ab_variants {
            for variant in [&variants.a, &variants.b] {
                rows.push(row(
                    root,
                    task_id,
                    card,
                    Some(variant),
                    Some(&variant.chat_id),
                ));
            }
        } else if !card.team_members.is_empty() {
            let start = rows.len();
            for member in &card.team_members {
                if let Some(chat_id) = &member.agent_chat_id {
                    rows.push(row(root, task_id, card, None, Some(chat_id)));
                }
            }
            if rows.len() == start {
                rows.push(row(root, task_id, card, None, None));
            }
        } else {
            rows.push(row(
                root,
                task_id,
                card,
                None,
                card.agent_chat_id.as_deref(),
            ));
        }
    }
    debug_assert_eq!(rows.len(), count);
    for row in &mut rows {
        row.metadata.source_revision = board.rev;
        row.provenance_complete = complete;
        if complete {
            row.source_records = provenance.records.clone();
        }
    }
    Some(rows)
}

fn row(
    root: &Path,
    task_id: &str,
    card: &BoardCard,
    variant: Option<&AbVariantInfo>,
    chat_id: Option<&str>,
) -> WorkRow {
    let state = variant
        .and_then(|variant| variant.finish.as_ref())
        .map(|finish| {
            if finish.success {
                WorkStateTag::Completed
            } else {
                WorkStateTag::Failed
            }
        })
        .unwrap_or_else(|| match card.column.as_str() {
            "done" => WorkStateTag::Completed,
            "failed" | "regressed" => WorkStateTag::Failed,
            "doing" => WorkStateTag::Interrupted,
            _ => WorkStateTag::Idle,
        });
    let activity = variant
        .and_then(|variant| variant.finish.as_ref())
        .map(|finish| &finish.completed_at)
        .or(card.completed_at.as_ref())
        .or(card.started_at.as_ref())
        .unwrap_or(&card.created_at);
    let timestamp = chrono::DateTime::parse_from_rfc3339(activity)
        .ok()
        .map(|time| time.timestamp_millis().max(0) as u64);
    let mut location = super::roots::project_root_ref(root);
    let checkout = variant
        .map(|variant| variant.worktree.as_str())
        .filter(|path| !path.is_empty())
        .or(card.agent_worktree.as_deref());
    if checkout.is_some() {
        location.kind = super::types::WorkRootKind::Worktree;
        location.id = variant
            .map(|variant| variant.agent_id.clone())
            .or_else(|| card.assignee.clone())
            .unwrap_or_else(|| format!("board:{task_id}:{}", card.id));
        location.path = None;
        location.branch = variant
            .and_then(|variant| variant.branch.clone())
            .or_else(|| card.agent_branch.clone());
    }
    WorkRow {
        key: chat_id
            .map(WorkKey::trajectory)
            .unwrap_or_else(|| WorkKey::trajectory(format!("board:{task_id}:{}", card.id))),
        metadata: WorkMetadata {
            task_id: Some(task_id.into()),
            card_id: Some(card.id.clone()),
            runtime_status: Some(card.column.clone()),
            last_activity_at: timestamp,
            freshness: Freshness::Unknown,
            ..Default::default()
        },
        kind: WorkKindTag::BoardAgent,
        state,
        attention: matches!(state, WorkStateTag::Failed | WorkStateTag::Interrupted),
        title: Some(card.title.clone()),
        summary: None,
        root: location,
        parent_chat_id: None,
        root_chat_id: None,
        chat_id: chat_id.map(str::to_string),
        model: variant.and_then(|variant| variant.model.clone()),
        current_tool: None,
        declared_files: card.target_files.clone(),
        observed_files: vec![],
        queue_size: 0,
        step_count: 0,
        open_questions: 0,
        updated_at_ms: timestamp.unwrap_or(0),
        stale: true,
        disclosure: Disclosure::Full,
        source_records: vec![],
        provenance_complete: false,
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn board() -> TaskBoard {
        serde_json::from_value(json!({"rev":1,"cards":[{
            "id":"card-a","title":"Board title","column":"doing",
            "created_at":chrono::Utc::now().to_rfc3339(),"agent_chat_id":"agent-a"
        }]}))
        .unwrap()
    }

    #[test]
    fn unknown_writers_and_modified_disclosure_never_inherit_clearance() {
        let legacy_json = r#"{"schema_version":1,"rev":0,"columns":[],"cards":[],"owner_chat_id":"owner","review_policy":"owner_allowed"}"#;
        let legacy_board: TaskBoard = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(serde_json::to_string(&legacy_board).unwrap(), legacy_json);
        assert_eq!(
            board_digest(&legacy_board),
            Some(crate::file_activity::digest_of(legacy_json))
        );
        let default_json = serde_json::to_value(TaskBoard::default()).unwrap();
        assert!(default_json.get("owner_epoch").is_none());
        assert!(default_json.get("owner_custody").is_none());
        let mut transferred = legacy_board.clone();
        transferred.try_transfer_owner("successor", 1).unwrap();
        let mut legacy_successor = legacy_board.clone();
        legacy_successor.owner_chat_id = Some("successor".into());
        assert_eq!(transferred.owner_epoch, 1);
        assert_eq!(transferred.owner_custody.len(), 1);
        assert_eq!(board_digest(&transferred), board_digest(&legacy_successor));
        assert_ne!(board_digest(&legacy_board), board_digest(&transferred));
        let legacy_provenance = BoardProvenance {
            digest: crate::file_activity::digest_of(legacy_json),
            records: vec![],
            complete: true,
            authority_version: 1,
        };
        assert!(legacy_provenance.valid_for(&legacy_board, &CoordinationSettings::default()));
        assert!(!legacy_provenance.valid_for(&transferred, &CoordinationSettings::default()));
        let board = board();
        let settings = CoordinationSettings::default();
        let source = write_provenance(&board, None, Some((vec![], true)));
        assert!(source.valid_for(&board, &settings));
        let mut changed = board.clone();
        changed.rev += 1;
        changed.cards[0].title = "new private title".into();
        assert!(!source.valid_for(&changed, &settings));
        assert!(!write_provenance(&changed, Some((&board, &source)), None).complete);
        changed.cards[0].title = board.cards[0].title.clone();
        assert!(write_provenance(&changed, Some((&board, &source)), None).complete);
        assert!(!write_provenance(&board, None, None).complete);
        let legacy = BoardProvenance::default();
        assert!(
            !write_provenance(&changed, Some((&board, &legacy)), Some((vec![], true))).complete
        );
        let mut legacy = source.clone();
        legacy.authority_version = 0;
        assert!(legacy.valid_for(&board, &settings));
        assert!(!legacy.authority_valid_for(&board, &settings));
        assert!(
            !write_provenance(&board, Some((&board, &legacy)), Some((vec![], true)))
                .authority_valid_for(&board, &settings)
        );
        assert!(source.authority_valid_for(&board, &settings));
        for field in [
            "owner_chat_id",
            "reviewer_chat_ids",
            "review_policy",
            "card_claims",
            "dependency_overrides",
        ] {
            let mut value = serde_json::to_value(&board).unwrap();
            value[field] = match field {
                "owner_chat_id" => json!("private-owner"),
                "reviewer_chat_ids" => json!(["private-reviewer"]),
                "review_policy" => json!("independent_required"),
                "card_claims" => {
                    json!({"card-a":{"claimant_chat_id":"private-holder","idempotency_key":"secret","claimed_at":"t0"}})
                }
                _ => {
                    json!({"card-a":{"overridden_by":"private-owner","reason":"secret","overridden_at":"t0","unsatisfied":["hidden-card"]}})
                }
            };
            let changed: TaskBoard = serde_json::from_value(value).unwrap();
            assert!(
                !write_provenance(&changed, Some((&board, &source)), None).complete,
                "{field}"
            );
        }
        let mut empty = TaskBoard::default();
        let empty_source = write_provenance(&empty, None, None);
        assert!(empty_source.complete);
        let previous = empty.clone();
        empty.owner_chat_id = Some("private-owner".into());
        assert!(!write_provenance(&empty, Some((&previous, &empty_source)), None).complete);
    }

    #[test]
    fn non_live_board_rows_and_row_byte_limits_are_explicit() {
        let board = board();
        let settings = CoordinationSettings::default();
        let rows = rows_from_board(
            Path::new("/project"),
            "task-a",
            &board,
            &BoardProvenance::default(),
            &settings,
        )
        .unwrap();
        assert_eq!(rows[0].state, WorkStateTag::Interrupted);
        assert_ne!(rows[0].metadata.freshness, Freshness::Live);
        assert!(!rows[0].provenance_complete);
        for settings in [
            CoordinationSettings {
                max_work_history_rows: 0,
                ..settings.clone()
            },
            CoordinationSettings {
                max_work_history_bytes: 1,
                ..settings
            },
        ] {
            assert!(rows_from_board(
                Path::new("/project"),
                "task-a",
                &board,
                &BoardProvenance::default(),
                &settings
            )
            .is_none());
        }
    }
}
