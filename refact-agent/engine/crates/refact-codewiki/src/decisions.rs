use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::gate::{verify_quote_default, Verification};
use crate::semantic_match::is_duplicate_decision_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionStatus {
    Verified,
    Fuzzy,
    Unverified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provenance {
    Verbatim,
    Paraphrase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionSource {
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub statement: String,
    pub evidence: String,
    pub source_kind: String,
    pub status: DecisionStatus,
    pub provenance: Provenance,
    pub corroboration_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Relation {
    Supersedes,
    Refines,
    RelatesTo,
    ConflictsWith,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DecisionGraph {
    pub decisions: Vec<Decision>,
    pub edges: Vec<(usize, usize, Relation)>,
}

const DECISION_PATTERNS: &[&str] = &[
    "decided to",
    "we chose",
    "chosen",
    "choose",
    "chose",
    "instead of",
    "because",
    "rationale",
    "trade-off",
    "tradeoff",
    "switched to",
    "switch to",
    "we now",
    "no longer",
    "decision:",
];

fn split_sentences(text: &str) -> Vec<String> {
    text.split(|c| c == '.' || c == '\n' || c == ';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn words_after_marker(sentence: &str, marker: &str) -> usize {
    let lower = sentence.to_ascii_lowercase();
    let Some(start) = lower.find(marker) else {
        return 0;
    };
    word_count(&sentence[start + marker.len()..])
}

pub fn has_decision_language(sentence: &str) -> bool {
    let lower = sentence.to_ascii_lowercase();
    has_instead_of_decision(&lower)
        || has_action_with_rationale(&lower, "switch")
        || has_action_with_rationale(&lower, "use")
        || DECISION_PATTERNS
            .iter()
            .filter(|marker| **marker != "instead of")
            .any(|marker| lower.contains(marker) && words_after_marker(sentence, marker) >= 4)
        || chose_over_object(&lower)
}

fn has_instead_of_decision(lower: &str) -> bool {
    if let Some(index) = lower.find(" instead of ") {
        let before = &lower[..index];
        let after = &lower[index + " instead of ".len()..];
        return word_count(before) >= 2 && word_count(after) >= 4;
    }

    if let Some(index) = lower.find("instead of ") {
        let before = &lower[..index];
        let after = &lower[index + "instead of ".len()..];
        return word_count(before) >= 2 && word_count(after) >= 4;
    }

    false
}

fn has_action_with_rationale(lower: &str, verb: &str) -> bool {
    let Some(index) = lower.find(verb) else {
        return false;
    };
    let after = &lower[index + verb.len()..];
    word_count(after) >= 4 && (lower.contains(" because ") || lower.contains("because "))
}

fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
}

fn chose_over_object(lower: &str) -> bool {
    let Some(chose_pos) = lower.find("chose") else {
        return false;
    };
    let Some(over_pos) = lower[chose_pos..].find(" over ") else {
        return false;
    };
    let between = &lower[chose_pos + "chose".len()..chose_pos + over_pos];
    let after = &lower[chose_pos + over_pos + " over ".len()..];
    between.split_whitespace().count() >= 1 && after.split_whitespace().count() >= 1
}

pub fn classify_evidence(
    evidence: &str,
    source_text: &str,
    provenance: Provenance,
) -> DecisionStatus {
    if provenance == Provenance::Verbatim {
        return DecisionStatus::Verified;
    }

    match verify_quote_default(evidence, source_text) {
        Verification::Exact => DecisionStatus::Verified,
        Verification::Fuzzy => DecisionStatus::Fuzzy,
        Verification::Unverified => DecisionStatus::Unverified,
    }
}

pub fn extract_decisions(sources: &[DecisionSource]) -> Vec<Decision> {
    let mut out = Vec::new();
    for src in sources {
        for sentence in split_sentences(&src.text) {
            if has_decision_language(&sentence) {
                out.push(Decision {
                    statement: sentence.clone(),
                    evidence: sentence,
                    source_kind: src.kind.clone(),
                    status: DecisionStatus::Verified,
                    provenance: Provenance::Verbatim,
                    corroboration_count: 1,
                });
            }
        }
    }
    out
}

pub fn corroborate(decisions: &mut Vec<Decision>) {
    let mut survivors: Vec<Decision> = Vec::new();
    let mut source_sets: Vec<BTreeSet<String>> = Vec::new();

    for decision in decisions.drain(..) {
        if let Some((index, survivor)) = survivors.iter_mut().enumerate().find(|(_, survivor)| {
            is_duplicate_decision_text(&survivor.statement, &decision.statement)
        }) {
            source_sets[index].insert(decision.source_kind.clone());
            if decision.evidence.len() > survivor.evidence.len() {
                survivor.evidence = decision.evidence;
            }
            if status_rank(decision.status) > status_rank(survivor.status) {
                survivor.status = decision.status;
            }
            if provenance_rank(decision.provenance) > provenance_rank(survivor.provenance) {
                survivor.provenance = decision.provenance;
            }
        } else {
            let mut sources = BTreeSet::new();
            sources.insert(decision.source_kind.clone());
            source_sets.push(sources);
            survivors.push(decision);
        }
    }

    for (decision, sources) in survivors.iter_mut().zip(source_sets) {
        decision.corroboration_count = sources.len().max(1) as u32;
    }

    *decisions = survivors;
}

fn status_rank(status: DecisionStatus) -> u8 {
    match status {
        DecisionStatus::Verified => 3,
        DecisionStatus::Fuzzy => 2,
        DecisionStatus::Unverified => 1,
    }
}

fn provenance_rank(provenance: Provenance) -> u8 {
    match provenance {
        Provenance::Verbatim => 2,
        Provenance::Paraphrase => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(statement: &str, source_kind: &str) -> Decision {
        Decision {
            statement: statement.to_string(),
            evidence: statement.to_string(),
            source_kind: source_kind.to_string(),
            status: DecisionStatus::Verified,
            provenance: Provenance::Verbatim,
            corroboration_count: 1,
        }
    }

    #[test]
    fn evidence_gate_verified_fuzzy_unverified() {
        let source = "We decided to use SQLite because it is embedded and simple.";
        assert_eq!(
            classify_evidence("decided to use SQLite", source, Provenance::Paraphrase),
            DecisionStatus::Verified
        );
        assert_eq!(
            classify_evidence(
                "SQLite simple embedded decided",
                source,
                Provenance::Paraphrase
            ),
            DecisionStatus::Fuzzy
        );
        assert_eq!(
            classify_evidence("we picked Postgres", source, Provenance::Paraphrase),
            DecisionStatus::Unverified
        );
    }

    #[test]
    fn verbatim_classification_skips_quote_gate() {
        assert_eq!(
            classify_evidence("not in source", "source text", Provenance::Verbatim),
            DecisionStatus::Verified
        );
    }

    #[test]
    fn extracts_decisions_from_markers() {
        let sources = vec![
            DecisionSource {
                kind: "commit".to_string(),
                text: "Refactor parser. We decided to use tree-sitter instead of regex."
                    .to_string(),
            },
            DecisionSource {
                kind: "comment".to_string(),
                text: "Just a normal comment with no decision.".to_string(),
            },
        ];
        let decisions = extract_decisions(&sources);
        assert_eq!(
            decisions.len(),
            1,
            "only the decision sentence: {decisions:?}"
        );
        assert_eq!(decisions[0].source_kind, "commit");
        assert_eq!(decisions[0].status, DecisionStatus::Verified);
        assert_eq!(decisions[0].provenance, Provenance::Verbatim);
        assert!(decisions[0].statement.contains("tree-sitter"));
    }

    #[test]
    fn ignores_marker_without_object() {
        let sources = vec![DecisionSource {
            kind: "commit".to_string(),
            text: "because tests".to_string(),
        }];

        assert!(extract_decisions(&sources).is_empty());
    }

    #[test]
    fn instead_of_requires_decision_object() {
        let sources = vec![DecisionSource {
            kind: "commit".to_string(),
            text: "instead of tests".to_string(),
        }];

        assert!(extract_decisions(&sources).is_empty());
    }

    #[test]
    fn corroborate_counts_distinct_sources_after_dedup() {
        let mut decisions = vec![
            decision("Use SQLite for storage", "commit"),
            decision("Use SQLite for storage storage", "commit"),
            decision("Use SQLite for storage", "pr"),
        ];

        corroborate(&mut decisions);

        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].corroboration_count, 2);
    }

    #[test]
    fn decision_graph_holds_relations() {
        let mut g = DecisionGraph::default();
        g.decisions.push(Decision {
            statement: "use sqlite".to_string(),
            evidence: "use sqlite".to_string(),
            source_kind: "commit".to_string(),
            status: DecisionStatus::Verified,
            provenance: Provenance::Verbatim,
            corroboration_count: 1,
        });
        g.decisions.push(Decision {
            statement: "use sqlite with vec0".to_string(),
            evidence: "use sqlite with vec0".to_string(),
            source_kind: "commit".to_string(),
            status: DecisionStatus::Verified,
            provenance: Provenance::Verbatim,
            corroboration_count: 1,
        });
        g.edges.push((1, 0, Relation::Refines));
        assert_eq!(g.edges[0].2, Relation::Refines);
    }
}
