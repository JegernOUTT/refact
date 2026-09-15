//! Ordering of cards that are known to touch the same files.
//!
//! **Ordering only.** Nothing here refuses, cancels, deprioritises across waves
//! or blocks admission. Every card handed in comes back out with a rank, so an
//! overlap cannot stall a wave: the rank of the card at input position `p` is
//! searched in `0..=p`, a range that always contains a free value because at
//! most `p` earlier cards can have taken one.
//!
//! **Declared is not observed.** A `target_files` entry is intent; a recorded
//! write is fact. They are carried as separate edges with separate sources and
//! are never merged into one undifferentiated signal.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The wording discipline shared with the work capsule: absence of a recorded
/// overlap is absence of evidence, not a safety guarantee.
pub const ABSENCE_NOTE: &str =
    "No observed overlap is not permission or a guarantee that editing is safe.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapSource {
    Declared,
    Observed,
}

impl OverlapSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            OverlapSource::Declared => "declared",
            OverlapSource::Observed => "observed",
        }
    }
}

/// File identities per card, kept apart by how they were learned. Keys are
/// produced by the caller from `file_activity::resolve_key`; this module never
/// resolves a path itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlapInputs {
    declared: BTreeMap<String, BTreeSet<String>>,
    observed: BTreeMap<String, BTreeSet<String>>,
}

impl OverlapInputs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn declare<I: IntoIterator<Item = String>>(&mut self, card_id: &str, keys: I) {
        self.declared
            .entry(card_id.to_string())
            .or_default()
            .extend(keys);
    }

    pub fn observe<I: IntoIterator<Item = String>>(&mut self, card_id: &str, keys: I) {
        self.observed
            .entry(card_id.to_string())
            .or_default()
            .extend(keys);
    }

    pub fn is_empty(&self) -> bool {
        self.declared.values().all(BTreeSet::is_empty)
            && self.observed.values().all(BTreeSet::is_empty)
    }

    fn shared(
        map: &BTreeMap<String, BTreeSet<String>>,
        left: &str,
        right: &str,
    ) -> Option<Vec<String>> {
        let left = map.get(left)?;
        let right = map.get(right)?;
        let shared: Vec<String> = left.intersection(right).cloned().collect();
        (!shared.is_empty()).then_some(shared)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlapEdge {
    pub with: String,
    pub source: OverlapSource,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedCard {
    pub card_id: String,
    /// Cards sharing a rank have no overlap known to this wave and may run at
    /// once; a higher rank is submitted after a lower one, never instead of it.
    pub serial_rank: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlaps: Vec<OverlapEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaveOrder {
    pub wave: usize,
    pub cards: Vec<OrderedCard>,
    pub note: String,
}

impl WaveOrder {
    pub fn rank_of(&self, card_id: &str) -> Option<usize> {
        self.cards
            .iter()
            .find(|card| card.card_id == card_id)
            .map(|card| card.serial_rank)
    }

    pub fn ranks(&self) -> BTreeMap<String, usize> {
        self.cards
            .iter()
            .map(|card| (card.card_id.clone(), card.serial_rank))
            .collect()
    }
}

/// Rank one wave's cards so that two cards known to touch the same file are
/// never submitted together.
///
/// `card_ids` is the wave's submission order as the lanes declared it, and it
/// is the only tie-break: equal ranks keep that order, so identical inputs
/// always produce an identical result.
pub fn order_wave(wave: usize, card_ids: &[String], inputs: &OverlapInputs) -> WaveOrder {
    let mut cards: Vec<OrderedCard> = Vec::with_capacity(card_ids.len());
    for (index, card_id) in card_ids.iter().enumerate() {
        let mut overlaps = Vec::new();
        let mut taken: BTreeSet<usize> = BTreeSet::new();
        for (earlier_index, earlier) in card_ids[..index].iter().enumerate() {
            let declared = OverlapInputs::shared(&inputs.declared, card_id, earlier);
            let observed = OverlapInputs::shared(&inputs.observed, card_id, earlier);
            if declared.is_none() && observed.is_none() {
                continue;
            }
            if let Some(files) = declared {
                overlaps.push(OverlapEdge {
                    with: earlier.clone(),
                    source: OverlapSource::Declared,
                    files,
                });
            }
            if let Some(files) = observed {
                overlaps.push(OverlapEdge {
                    with: earlier.clone(),
                    source: OverlapSource::Observed,
                    files,
                });
            }
            taken.insert(cards[earlier_index].serial_rank);
        }
        let serial_rank = (0..=index)
            .find(|rank| !taken.contains(rank))
            .unwrap_or(index);
        cards.push(OrderedCard {
            card_id: card_id.clone(),
            serial_rank,
            overlaps,
        });
    }
    cards.sort_by_key(|card| card.serial_rank);
    WaveOrder {
        wave,
        cards,
        note: ABSENCE_NOTE.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn declared(pairs: &[(&str, &[&str])]) -> OverlapInputs {
        let mut inputs = OverlapInputs::new();
        for (card_id, files) in pairs {
            inputs.declare(card_id, files.iter().map(|file| file.to_string()));
        }
        inputs
    }

    #[test]
    fn two_cards_declaring_the_same_path_are_ordered_not_raced() {
        let inputs = declared(&[
            ("T-1", &["/repo/src/lib.rs"]),
            ("T-2", &["/repo/src/lib.rs"]),
        ]);

        let order = order_wave(0, &ids(&["T-1", "T-2"]), &inputs);

        assert_eq!(order.rank_of("T-1"), Some(0));
        assert_eq!(
            order.rank_of("T-2"),
            Some(1),
            "a declared collision must not share a rank"
        );
        assert_eq!(
            order.cards[1].overlaps,
            vec![OverlapEdge {
                with: "T-1".to_string(),
                source: OverlapSource::Declared,
                files: vec!["/repo/src/lib.rs".to_string()],
            }]
        );
    }

    #[test]
    fn cards_touching_different_files_keep_one_rank_and_stay_parallel() {
        let inputs = declared(&[
            ("T-1", &["/repo/src/a.rs"]),
            ("T-2", &["/repo/src/b.rs"]),
            ("T-3", &["/repo/src/c.rs"]),
        ]);

        let order = order_wave(0, &ids(&["T-1", "T-2", "T-3"]), &inputs);

        assert!(order.cards.iter().all(|card| card.serial_rank == 0));
        assert!(order.cards.iter().all(|card| card.overlaps.is_empty()));
    }

    #[test]
    fn a_pathological_overlap_graph_still_ranks_every_card_without_stalling() {
        let everything: Vec<(&str, &[&str])> = vec![
            ("T-1", &["/repo/src/shared.rs"]),
            ("T-2", &["/repo/src/shared.rs"]),
            ("T-3", &["/repo/src/shared.rs"]),
            ("T-4", &["/repo/src/shared.rs"]),
            ("T-5", &["/repo/src/shared.rs"]),
        ];
        let inputs = declared(&everything);
        let wave = ids(&["T-1", "T-2", "T-3", "T-4", "T-5"]);

        let order = order_wave(0, &wave, &inputs);

        assert_eq!(
            order.cards.len(),
            wave.len(),
            "every card must survive ordering; ordering never drops work"
        );
        let ranks: Vec<usize> = order.cards.iter().map(|card| card.serial_rank).collect();
        assert_eq!(
            ranks,
            vec![0, 1, 2, 3, 4],
            "a clique is fully serialised, never refused"
        );
        let ordered: Vec<&str> = order
            .cards
            .iter()
            .map(|card| card.card_id.as_str())
            .collect();
        assert_eq!(ordered, vec!["T-1", "T-2", "T-3", "T-4", "T-5"]);
        assert!(
            order.cards.iter().all(|card| card.serial_rank < wave.len()),
            "a rank is bounded by the wave, so the search always terminates"
        );
    }

    #[test]
    fn ordering_is_deterministic_across_repeated_runs() {
        let mut inputs = declared(&[
            ("T-1", &["/repo/a.rs", "/repo/b.rs"]),
            ("T-2", &["/repo/b.rs"]),
            ("T-3", &["/repo/c.rs"]),
            ("T-4", &["/repo/a.rs", "/repo/c.rs"]),
        ]);
        inputs.observe("T-3", ["/repo/b.rs".to_string()]);
        let wave = ids(&["T-1", "T-2", "T-3", "T-4"]);

        let first = order_wave(1, &wave, &inputs);
        for _ in 0..16 {
            assert_eq!(order_wave(1, &wave, &inputs), first);
        }
    }

    #[test]
    fn declared_and_observed_overlap_remain_distinguishable() {
        let mut inputs = declared(&[
            ("T-1", &["/repo/src/lib.rs"]),
            ("T-2", &["/repo/src/lib.rs"]),
        ]);
        inputs.observe("T-1", ["/repo/src/lib.rs".to_string()]);
        inputs.observe("T-2", ["/repo/src/lib.rs".to_string()]);

        let order = order_wave(0, &ids(&["T-1", "T-2"]), &inputs);
        let sources: Vec<OverlapSource> = order.cards[1]
            .overlaps
            .iter()
            .map(|edge| edge.source)
            .collect();

        assert_eq!(
            sources,
            vec![OverlapSource::Declared, OverlapSource::Observed],
            "intent and fact must not collapse into one signal"
        );
        assert_eq!(OverlapSource::Declared.as_str(), "declared");
        assert_eq!(OverlapSource::Observed.as_str(), "observed");
    }

    #[test]
    fn an_observed_write_alone_is_enough_to_order_two_cards() {
        let mut inputs = OverlapInputs::new();
        inputs.observe("T-1", ["/repo/src/lib.rs".to_string()]);
        inputs.observe("T-2", ["/repo/src/lib.rs".to_string()]);

        let order = order_wave(0, &ids(&["T-1", "T-2"]), &inputs);

        assert_eq!(order.rank_of("T-2"), Some(1));
        assert_eq!(order.cards[1].overlaps[0].source, OverlapSource::Observed);
    }

    #[test]
    fn a_rank_is_reused_once_the_conflicting_card_is_behind_it() {
        let inputs = declared(&[
            ("T-1", &["/repo/a.rs"]),
            ("T-2", &["/repo/a.rs"]),
            ("T-3", &["/repo/b.rs"]),
        ]);

        let order = order_wave(0, &ids(&["T-1", "T-2", "T-3"]), &inputs);

        assert_eq!(order.rank_of("T-3"), Some(0));
        assert_eq!(
            order
                .cards
                .iter()
                .filter(|card| card.serial_rank == 0)
                .count(),
            2,
            "a card that collides with nothing keeps the first rank"
        );
    }

    #[test]
    fn absent_overlap_information_leaves_the_wave_untouched() {
        let inputs = OverlapInputs::new();
        let wave = ids(&["T-1", "T-2", "T-3"]);

        let order = order_wave(2, &wave, &inputs);

        assert!(inputs.is_empty());
        assert_eq!(order.wave, 2);
        assert!(order.cards.iter().all(|card| card.serial_rank == 0));
        assert_eq!(
            order
                .cards
                .iter()
                .map(|card| card.card_id.clone())
                .collect::<Vec<String>>(),
            wave
        );
    }

    #[test]
    fn the_order_states_that_absence_of_overlap_is_not_a_guarantee() {
        let order = order_wave(0, &ids(&["T-1"]), &OverlapInputs::new());

        assert_eq!(order.note, ABSENCE_NOTE);
        assert!(order.note.contains("not permission or a guarantee"));
    }
}
