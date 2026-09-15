//! Overlap facts a board already has, in the form the wave scheduler wants.
//!
//! Declared overlap is what a card said it would touch; observed overlap is
//! what its agent was recorded writing. Both are collapsed onto file identities
//! by `file_activity::resolve_key`, so a symlink or a second spelling of one
//! path cannot hide a collision.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use refact_tasks::overlap_order::OverlapInputs;

use crate::global_context::GlobalContext;
use crate::tasks::types::TaskBoard;

fn file_key(root: &Path, entry: &str) -> String {
    let trimmed = entry.trim().trim_start_matches(['/', '\\']);
    crate::file_activity::resolve_key(&root.join(trimmed)).0
}

pub fn overlap_ordering_enabled() -> bool {
    refact_work_core::coordination_settings().overlap_ordering_enabled
}

/// Declared and observed file identities per card. An empty result orders
/// nothing, which leaves the wave exactly as the lanes declared it.
pub async fn overlap_inputs(gcx: Arc<GlobalContext>, board: &TaskBoard) -> OverlapInputs {
    let mut inputs = OverlapInputs::new();
    let roots = crate::files_correction::get_project_dirs(gcx).await;
    let Some(root) = roots.first() else {
        return inputs;
    };
    for card in &board.cards {
        let declared: BTreeSet<String> = card
            .target_files
            .iter()
            .filter(|entry| !entry.trim().is_empty())
            .map(|entry| file_key(root, entry))
            .collect();
        if !declared.is_empty() {
            inputs.declare(&card.id, declared);
        }
    }
    let writes = crate::file_activity::observations(None).writes;
    for card in &board.cards {
        let Some(chat_id) = card.agent_chat_id.as_deref() else {
            continue;
        };
        let observed: BTreeSet<String> = writes
            .iter()
            .filter(|write| write.actor.as_deref() == Some(chat_id))
            .map(|write| write.key.clone())
            .collect();
        if !observed.is_empty() {
            inputs.observe(&card.id, observed);
        }
    }
    inputs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::waves::{
        plan_schedule, plan_schedule_with_overlap, Lane, LaneKind, LaneStage, StageDecision,
        WaveLimits,
    };
    use refact_tasks::overlap_order::{order_wave, OverlapSource};

    fn card(id: &str, target_files: &[&str]) -> crate::tasks::types::BoardCard {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": id,
            "column": "planned",
            "depends_on": [],
            "target_files": target_files,
            "created_at": "2026-05-16T00:00:00Z",
        }))
        .unwrap()
    }

    fn limits() -> WaveLimits {
        WaveLimits {
            max_wave_width: 64,
            max_pipeline_depth: 16,
        }
    }

    fn lane(card_ids: &[&str]) -> Vec<Lane> {
        vec![Lane {
            name: "roots".to_string(),
            kind: LaneKind::Parallel,
            stages: card_ids
                .iter()
                .map(|card_id| LaneStage {
                    card_id: card_id.to_string(),
                    agent_id: None,
                    optional: false,
                })
                .collect(),
            optional: false,
        }]
    }

    async fn gcx_rooted_at(dir: &std::path::Path) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.to_path_buf()];
        gcx
    }

    #[tokio::test]
    async fn overlap_order_collapses_two_spellings_of_one_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn main() {}").unwrap();
        let gcx = gcx_rooted_at(dir.path()).await;
        let board = TaskBoard {
            cards: vec![
                card("T-1", &["src/lib.rs"]),
                card("T-2", &["/src/lib.rs"]),
                card("T-3", &["src/other.rs"]),
            ],
            ..TaskBoard::default()
        };

        let inputs = overlap_inputs(gcx, &board).await;
        let order = order_wave(
            0,
            &["T-1".to_string(), "T-2".to_string(), "T-3".to_string()],
            &inputs,
        );

        assert!(!inputs.is_empty());
        assert_eq!(order.rank_of("T-1"), Some(0));
        assert_eq!(
            order.rank_of("T-2"),
            Some(1),
            "a leading separator is the same file, not a second one"
        );
        assert_eq!(order.rank_of("T-3"), Some(0));
        assert_eq!(
            order
                .cards
                .iter()
                .find(|entry| entry.card_id == "T-2")
                .unwrap()
                .overlaps[0]
                .source,
            OverlapSource::Declared
        );
    }

    #[tokio::test]
    async fn overlap_order_is_absent_when_no_card_declares_or_touches_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx_rooted_at(dir.path()).await;
        let board = TaskBoard {
            cards: vec![card("T-1", &[]), card("T-2", &[])],
            ..TaskBoard::default()
        };

        let inputs = overlap_inputs(gcx, &board).await;
        let ordered =
            plan_schedule_with_overlap(&board, &lane(&["T-1", "T-2"]), &limits(), Some(&inputs))
                .unwrap();
        let unordered = plan_schedule(&board, &lane(&["T-1", "T-2"]), &limits()).unwrap();

        assert!(inputs.is_empty());
        assert_eq!(
            serde_json::to_string(&ordered).unwrap(),
            serde_json::to_string(&unordered).unwrap(),
            "with nothing known the wave runs exactly as it would have"
        );
        assert!(ordered.overlap_order.is_empty());
        assert_eq!(ordered.submission_groups(), vec![ordered.dispatch.clone()]);
    }

    #[tokio::test]
    async fn overlap_order_never_gates_a_pathological_all_to_all_graph() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/shared.rs"), "shared").unwrap();
        let gcx = gcx_rooted_at(dir.path()).await;
        let ids = ["T-1", "T-2", "T-3", "T-4", "T-5", "T-6"];
        let board = TaskBoard {
            cards: ids.iter().map(|id| card(id, &["src/shared.rs"])).collect(),
            ..TaskBoard::default()
        };

        let inputs = overlap_inputs(gcx, &board).await;
        let ordered =
            plan_schedule_with_overlap(&board, &lane(&ids), &limits(), Some(&inputs)).unwrap();
        let unordered = plan_schedule(&board, &lane(&ids), &limits()).unwrap();

        assert_eq!(
            ordered.dispatch, unordered.dispatch,
            "every card of a clique is still submitted; ordering refuses nothing"
        );
        assert_eq!(ordered.dispatch.len(), ids.len(), "no card is starved");
        assert!(
            ordered
                .lanes
                .iter()
                .flat_map(|outcome| outcome.stages.iter())
                .all(|stage| stage.decision == StageDecision::Dispatch),
            "ordering must never turn a dispatch into a refusal or a block"
        );
        let groups = ordered.submission_groups();
        assert_eq!(
            groups.len(),
            ids.len(),
            "a clique is fully serialised rather than deadlocked"
        );
        assert_eq!(
            groups.into_iter().flatten().collect::<Vec<String>>(),
            ordered.dispatch,
            "the submission groups partition the dispatch set exactly"
        );
    }

    #[tokio::test]
    async fn overlap_order_is_deterministic_across_repeated_runs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let gcx = gcx_rooted_at(dir.path()).await;
        let board = TaskBoard {
            cards: vec![
                card("T-1", &["src/a.rs", "src/b.rs"]),
                card("T-2", &["src/b.rs"]),
                card("T-3", &["src/c.rs"]),
                card("T-4", &["src/a.rs", "src/c.rs"]),
            ],
            ..TaskBoard::default()
        };
        let lanes = lane(&["T-1", "T-2", "T-3", "T-4"]);

        let inputs = overlap_inputs(gcx, &board).await;
        let first = plan_schedule_with_overlap(&board, &lanes, &limits(), Some(&inputs)).unwrap();
        for _ in 0..16 {
            let again =
                plan_schedule_with_overlap(&board, &lanes, &limits(), Some(&inputs)).unwrap();
            assert_eq!(again.dispatch, first.dispatch);
            assert_eq!(
                serde_json::to_string(&again).unwrap(),
                serde_json::to_string(&first).unwrap()
            );
        }
    }

    #[test]
    fn overlap_ordering_is_on_by_default() {
        assert!(refact_work_core::CoordinationSettings::default().overlap_ordering_enabled);
    }
}
