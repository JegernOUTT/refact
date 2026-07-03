use std::collections::{BTreeMap, BTreeSet};

use crate::decisions::*;
pub use crate::semantic_match::DEFAULT_DEDUP_TAU;

pub const RELATED_TAU: f64 = 0.6;
pub const SUPERSEDE_AUTOFLIP_CONFIDENCE: f64 = 0.85;

const REVERSAL_SIGNALS: &[&str] = &[
    "instead of",
    "no longer",
    "replaced",
    "replace",
    "supersedes",
    "supersede",
    "deprecated",
    "deprecate",
    "migrate",
    "migrated",
    "migration",
    "switch to",
    "switched to",
    "move away",
    "moved away",
    "move to",
    "moved to",
    "revert",
    "reverted",
    "stop using",
    "in favor of",
    "in favour of",
];

const OPPOSING_VERB_PAIRS: &[(&str, &str)] = &[
    ("use", "avoid"),
    ("adopt", "deprecate"),
    ("use", "deprecate"),
    ("add", "remove"),
    ("enable", "disable"),
    ("adopt", "abandon"),
    ("switch to", "revert"),
    ("migrate to", "roll back"),
    ("introduce", "drop"),
    ("sync", "async"),
    ("blocking", "non blocking"),
    ("monolith", "microservice"),
];

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "to", "of", "for", "and", "or", "in", "on", "with", "we", "our", "use",
    "using", "should", "this", "that", "is", "are", "be", "by", "as", "it", "its", "from", "at",
    "via", "into",
];

/// Cosine similarity over lowercased alphanumeric token-frequency vectors.
pub fn text_similarity(a: &str, b: &str) -> f64 {
    let a_counts = token_counts(a);
    let b_counts = token_counts(b);
    if a_counts.is_empty() || b_counts.is_empty() {
        return 0.0;
    }

    let dot: f64 = a_counts
        .iter()
        .filter_map(|(token, a_count)| {
            b_counts
                .get(token)
                .map(|b_count| (*a_count * *b_count) as f64)
        })
        .sum();
    let a_norm = (a_counts
        .values()
        .map(|count| (*count as f64).powi(2))
        .sum::<f64>())
    .sqrt();
    let b_norm = (b_counts
        .values()
        .map(|count| (*count as f64).powi(2))
        .sum::<f64>())
    .sqrt();

    if a_norm == 0.0 || b_norm == 0.0 {
        0.0
    } else {
        dot / (a_norm * b_norm)
    }
}

/// Build deterministic decision edges from token-cosine relatedness and
/// RepoWise-style contradiction/reversal heuristics.
pub fn build_decision_graph(mut decisions: Vec<Decision>) -> DecisionGraph {
    let mut edges = Vec::new();

    for i in 0..decisions.len() {
        for j in (i + 1)..decisions.len() {
            let statement_i = decisions[i].statement.clone();
            let statement_j = decisions[j].statement.clone();
            let statement_i = statement_i.as_str();
            let statement_j = statement_j.as_str();
            let sim = text_similarity(statement_i, statement_j);

            if sim >= DEFAULT_DEDUP_TAU {
                edges.push((j, i, Relation::RelatesTo));
                if !same_normalized(statement_i, statement_j) {
                    edges.push((j, i, Relation::Refines));
                }
                continue;
            }

            let opposing = has_opposing_signal(statement_i, statement_j);
            let reversal_i = has_reversal_signal(statement_i);
            let reversal_j = has_reversal_signal(statement_j);
            // Token cosine is intentionally simple; keep the same-topic gate from
            // RepoWise so explicit reversals such as "deprecate SQLite ..." still
            // land in the related band when they share a topic noun.
            let effective_sim = if sim < RELATED_TAU
                && opposing
                && (reversal_i || reversal_j)
                && shared_topic(statement_i, statement_j)
            {
                RELATED_TAU
            } else {
                sim
            };

            if (RELATED_TAU..DEFAULT_DEDUP_TAU).contains(&effective_sim) {
                if opposing {
                    if reversal_j {
                        let confidence = supersession_confidence(effective_sim, true);
                        edges.push((j, i, Relation::Supersedes));
                        if confidence >= SUPERSEDE_AUTOFLIP_CONFIDENCE {
                            decisions[i].status = DecisionStatus::Fuzzy;
                        }
                    } else if reversal_i {
                        let confidence = supersession_confidence(effective_sim, true);
                        edges.push((i, j, Relation::Supersedes));
                        if confidence >= SUPERSEDE_AUTOFLIP_CONFIDENCE {
                            decisions[j].status = DecisionStatus::Fuzzy;
                        }
                    } else {
                        edges.push((j, i, Relation::ConflictsWith));
                    }
                } else {
                    edges.push((j, i, Relation::RelatesTo));
                }
            }
        }
    }

    DecisionGraph { decisions, edges }
}

/// Confidence that an evolution edge is a supersession, bounded to [0, 0.99].
pub fn supersession_confidence(sim: f64, has_reversal_signal: bool) -> f64 {
    let bounded_sim = sim.clamp(0.0, 1.0);
    let mut confidence = 0.45 + 0.4 * bounded_sim;
    if has_reversal_signal {
        confidence += 0.2;
    }
    confidence.clamp(0.0, 0.99)
}

pub fn opposing_verbs() -> &'static [(&'static str, &'static str)] {
    OPPOSING_VERB_PAIRS
}

fn token_counts(text: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for token in tokens(text) {
        *counts.entry(token).or_insert(0) += 1;
    }
    counts
}

fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();

    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }

    out
}

fn normalized_text(text: &str) -> String {
    tokens(text).join(" ")
}

fn same_normalized(a: &str, b: &str) -> bool {
    normalized_text(a) == normalized_text(b)
}

fn has_reversal_signal(text: &str) -> bool {
    let normalized = normalized_text(text);
    REVERSAL_SIGNALS
        .iter()
        .any(|signal| normalized.contains(signal))
}

fn has_opposing_signal(a: &str, b: &str) -> bool {
    let normalized_a = normalized_text(a);
    let normalized_b = normalized_text(b);

    OPPOSING_VERB_PAIRS.iter().any(|(left, right)| {
        (contains_phrase(&normalized_a, left) && contains_phrase(&normalized_b, right))
            || (contains_phrase(&normalized_a, right) && contains_phrase(&normalized_b, left))
    }) || ((has_reversal_signal(a) || has_reversal_signal(b)) && shared_topic(a, b))
}

fn contains_phrase(normalized_text: &str, phrase: &str) -> bool {
    let normalized_phrase = normalized_text_from_phrase(phrase);
    normalized_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(normalized_phrase.split_whitespace().count())
        .any(|window| window.join(" ") == normalized_phrase)
}

fn normalized_text_from_phrase(phrase: &str) -> String {
    normalized_text(phrase)
}

fn shared_topic(a: &str, b: &str) -> bool {
    let stopwords: BTreeSet<&str> = STOPWORDS.iter().copied().collect();
    let content_a: BTreeSet<String> = tokens(a)
        .into_iter()
        .filter(|token| token.len() > 2 && !stopwords.contains(token.as_str()))
        .collect();
    let content_b: BTreeSet<String> = tokens(b)
        .into_iter()
        .filter(|token| token.len() > 2 && !stopwords.contains(token.as_str()))
        .collect();

    content_a.intersection(&content_b).count() >= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(statement: &str) -> Decision {
        Decision {
            statement: statement.to_string(),
            evidence: statement.to_string(),
            source_kind: "test".to_string(),
            status: DecisionStatus::Verified,
            provenance: Provenance::Verbatim,
            corroboration_count: 1,
        }
    }

    #[test]
    fn near_identical_statements_relate_and_refine() {
        let graph = build_decision_graph(vec![
            decision("Use SQLite for storage"),
            decision("Use SQLite for storage storage"),
        ]);

        assert!(graph.edges.iter().any(|edge| edge.2 == Relation::RelatesTo));
        assert!(graph.edges.iter().any(|edge| edge.2 == Relation::Refines));
    }

    #[test]
    fn opposing_reversal_supersedes_with_high_confidence() {
        let sim = text_similarity(
            "We adopt SQLite for storage",
            "We deprecate SQLite, switch to Postgres instead",
        );
        let graph = build_decision_graph(vec![
            decision("We adopt SQLite for storage"),
            decision("We deprecate SQLite, switch to Postgres instead"),
        ]);

        assert!(graph
            .edges
            .iter()
            .any(|edge| *edge == (1, 0, Relation::Supersedes)));
        assert!(
            supersession_confidence(sim.max(RELATED_TAU), true) >= SUPERSEDE_AUTOFLIP_CONFIDENCE
        );
    }

    #[test]
    fn low_similarity_statements_have_no_edge() {
        let graph = build_decision_graph(vec![
            decision("Use Redis for caching"),
            decision("Use Kafka for messaging"),
        ]);

        assert!(graph.edges.is_empty());
    }

    #[test]
    fn opposing_verbs_table_contains_use_avoid_pair() {
        assert!(!opposing_verbs().is_empty());
        assert!(opposing_verbs().contains(&("use", "avoid")));
    }
}
