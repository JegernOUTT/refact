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

    #[test]
    fn an_observed_write_alone_is_enough_to_order_two_cards() {
        let mut inputs = OverlapInputs::new();
        inputs.observe("T-1", ["/repo/src/lib.rs".to_string()]);
        inputs.observe("T-2", ["/repo/src/lib.rs".to_string()]);

        let order = order_wave(0, &ids(&["T-1", "T-2"]), &inputs);

        assert_eq!(order.rank_of("T-2"), Some(1));
        assert_eq!(order.cards[1].overlaps[0].source, OverlapSource::Observed);
    }
}
