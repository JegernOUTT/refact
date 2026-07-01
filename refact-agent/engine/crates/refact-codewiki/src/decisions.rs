use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionStatus {
    Verified,
    Fuzzy,
    Unverified,
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

const MARKERS: &[&str] = &[
    "decided to",
    "we chose",
    "chosen",
    "instead of",
    "because",
    "rationale",
    "trade-off",
    "tradeoff",
    "note:",
    "decision:",
];

fn split_sentences(text: &str) -> Vec<String> {
    text.split(|c| c == '.' || c == '\n' || c == ';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Evidence gate: Verified if the evidence appears verbatim in the source text;
/// Fuzzy if every whitespace token of the evidence appears (in any order);
/// Unverified otherwise.
pub fn classify_evidence(evidence: &str, source_text: &str) -> DecisionStatus {
    let ev = evidence.trim();
    if ev.is_empty() {
        return DecisionStatus::Unverified;
    }
    let hay = source_text.to_lowercase();
    let needle = ev.to_lowercase();
    if hay.contains(&needle) {
        return DecisionStatus::Verified;
    }
    let all_tokens_present = needle
        .split_whitespace()
        .all(|tok| hay.split(|c: char| !c.is_alphanumeric()).any(|w| w == tok));
    if all_tokens_present {
        DecisionStatus::Fuzzy
    } else {
        DecisionStatus::Unverified
    }
}

pub fn extract_decisions(sources: &[DecisionSource]) -> Vec<Decision> {
    let mut out = Vec::new();
    for src in sources {
        for sentence in split_sentences(&src.text) {
            let lower = sentence.to_lowercase();
            if MARKERS.iter().any(|m| lower.contains(m)) {
                let status = classify_evidence(&sentence, &src.text);
                out.push(Decision {
                    statement: sentence.clone(),
                    evidence: sentence,
                    source_kind: src.kind.clone(),
                    status,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_gate_verified_fuzzy_unverified() {
        let source = "We decided to use SQLite because it is embedded and simple.";
        assert_eq!(
            classify_evidence("decided to use SQLite", source),
            DecisionStatus::Verified
        );
        assert_eq!(
            classify_evidence("SQLite simple embedded decided", source),
            DecisionStatus::Fuzzy
        );
        assert_eq!(
            classify_evidence("we picked Postgres", source),
            DecisionStatus::Unverified
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
        assert!(decisions[0].statement.contains("tree-sitter"));
    }

    #[test]
    fn decision_graph_holds_relations() {
        let mut g = DecisionGraph::default();
        g.decisions.push(Decision {
            statement: "use sqlite".to_string(),
            evidence: "use sqlite".to_string(),
            source_kind: "commit".to_string(),
            status: DecisionStatus::Verified,
        });
        g.decisions.push(Decision {
            statement: "use sqlite with vec0".to_string(),
            evidence: "use sqlite with vec0".to_string(),
            source_kind: "commit".to_string(),
            status: DecisionStatus::Verified,
        });
        g.edges.push((1, 0, Relation::Refines));
        assert_eq!(g.edges[0].2, Relation::Refines);
    }
}
