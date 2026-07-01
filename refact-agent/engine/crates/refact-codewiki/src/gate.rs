use std::collections::HashSet;

pub fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Exact,
    Fuzzy,
    Unverified,
}

pub fn verify_quote(quote: &str, source_text: &str, fuzzy_threshold: f64) -> Verdict {
    let q = normalize_text(quote);
    let src = normalize_text(source_text);

    if q.is_empty() || src.is_empty() {
        return Verdict::Unverified;
    }

    if src.contains(q.as_str()) {
        return Verdict::Exact;
    }

    let q_tokens: HashSet<&str> = q.split_whitespace().collect();
    let src_tokens: HashSet<&str> = src.split_whitespace().collect();
    let overlap = q_tokens
        .iter()
        .filter(|token| src_tokens.contains(**token))
        .count() as f64
        / q_tokens.len() as f64;

    if overlap >= fuzzy_threshold {
        Verdict::Fuzzy
    } else {
        Verdict::Unverified
    }
}

pub fn verify_quote_default(quote: &str, source_text: &str) -> Verdict {
    verify_quote(quote, source_text, 0.6)
}

pub struct GateCandidate {
    pub decision: String,
    pub rationale: String,
    pub source_quote: String,
    pub source_text: String,
    pub verification: String,
}

pub fn apply_substring_gate(mut candidates: Vec<GateCandidate>) -> (Vec<GateCandidate>, usize) {
    let mut kept = Vec::new();
    let mut rejected_count = 0;

    for mut candidate in candidates.drain(..) {
        if candidate.source_text.trim().is_empty() {
            candidate.verification = "unverified".to_string();
            candidate.source_text.clear();
            kept.push(candidate);
            continue;
        }

        let source_text = candidate.source_text.clone();
        let mut produced_any = false;
        let mut grounded_any = false;
        let mut verdicts = Vec::new();

        for index in 0..3 {
            let field = match index {
                0 => &mut candidate.decision,
                1 => &mut candidate.rationale,
                _ => &mut candidate.source_quote,
            };
            let val = field.trim().to_string();

            if val.is_empty() {
                continue;
            }

            produced_any = true;
            let verdict = verify_quote(&val, &source_text, 0.6);

            if verdict == Verdict::Unverified {
                field.clear();
            } else {
                verdicts.push(verdict);
                grounded_any = true;
            }
        }

        if produced_any && !grounded_any {
            rejected_count += 1;
            continue;
        }

        candidate.verification = if verdicts.iter().any(|verdict| *verdict == Verdict::Exact) {
            "exact".to_string()
        } else if verdicts.iter().any(|verdict| *verdict == Verdict::Fuzzy) {
            "fuzzy".to_string()
        } else {
            "unverified".to_string()
        };
        candidate.source_text.clear();
        kept.push(candidate);
    }

    (kept, rejected_count)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    Exact,
    Fuzzy,
    Unverified,
}

pub fn compute_confidence(
    top_rank: u8,
    corroboration_count: u32,
    verification: Verification,
) -> f64 {
    let base = 0.4 + 0.5 * (top_rank as f64 / 9.0);
    let corroboration_bonus = (0.04 * corroboration_count.saturating_sub(1) as f64).min(0.12);
    let mut conf = base + corroboration_bonus;

    match verification {
        Verification::Exact => {}
        Verification::Fuzzy => conf *= 0.85,
        Verification::Unverified => conf *= 0.6,
    }

    (conf.clamp(0.0, 0.99) * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        decision: &str,
        rationale: &str,
        source_quote: &str,
        source_text: &str,
    ) -> GateCandidate {
        GateCandidate {
            decision: decision.to_string(),
            rationale: rationale.to_string(),
            source_quote: source_quote.to_string(),
            source_text: source_text.to_string(),
            verification: String::new(),
        }
    }

    #[test]
    fn decision_similarity_scores_overlap() {
        assert_eq!(
            decision_similarity("use postgres for storage", "use postgres for storage"),
            1.0
        );
        assert_eq!(decision_similarity("", "anything"), 0.0);
        let partial = decision_similarity("adopt redis cache layer", "adopt redis queue layer");
        assert!(partial > 0.0 && partial < 1.0);
    }

    #[test]
    fn normalize_collapses_trims_and_lowercases() {
        assert_eq!(normalize_text("  Hello\n\tWORLD  "), "hello world");
    }

    #[test]
    fn verify_exact_substring() {
        assert_eq!(
            verify_quote("quick brown", "The quick brown fox", 0.6),
            Verdict::Exact
        );
    }

    #[test]
    fn verify_is_whitespace_and_case_insensitive() {
        assert_eq!(
            verify_quote("QUICK\nBROWN", "the   quick brown fox", 0.6),
            Verdict::Exact
        );
    }

    #[test]
    fn verify_fuzzy_when_token_recall_meets_threshold() {
        assert_eq!(
            verify_quote(
                "alpha beta gamma delta epsilon",
                "alpha beta gamma zeta",
                0.6
            ),
            Verdict::Fuzzy
        );
    }

    #[test]
    fn verify_unverified_when_token_recall_below_threshold() {
        assert_eq!(
            verify_quote("alpha beta gamma delta epsilon", "alpha beta zeta", 0.6),
            Verdict::Unverified
        );
    }

    #[test]
    fn gate_keeps_empty_source_text_as_unverified() {
        let (kept, rejected) = apply_substring_gate(vec![candidate("made", "up", "quote", "   ")]);

        assert_eq!(rejected, 0);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].verification, "unverified");
        assert_eq!(kept[0].source_text, "");
    }

    #[test]
    fn gate_rejects_when_all_produced_fields_are_unverified() {
        let (kept, rejected) = apply_substring_gate(vec![candidate(
            "alpha",
            "beta",
            "gamma",
            "delta epsilon zeta",
        )]);

        assert_eq!(rejected, 1);
        assert!(kept.is_empty());
    }

    #[test]
    fn gate_drops_unverified_fields_and_keeps_grounded_candidate() {
        let (kept, rejected) = apply_substring_gate(vec![candidate(
            "keep this",
            "hallucinated field",
            "alpha beta gamma delta epsilon",
            "we keep this and alpha beta gamma zeta",
        )]);

        assert_eq!(rejected, 0);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].decision, "keep this");
        assert_eq!(kept[0].rationale, "");
        assert_eq!(kept[0].source_quote, "alpha beta gamma delta epsilon");
        assert_eq!(kept[0].verification, "exact");
        assert_eq!(kept[0].source_text, "");
    }

    #[test]
    fn confidence_exact_fuzzy_unverified_and_cap() {
        assert_eq!(compute_confidence(9, 1, Verification::Exact), 0.9);
        assert_eq!(compute_confidence(9, 1, Verification::Fuzzy), 0.765);
        assert_eq!(compute_confidence(9, 1, Verification::Unverified), 0.54);
        assert_eq!(compute_confidence(9, 5, Verification::Exact), 0.99);
    }
}

pub fn decision_similarity(a: &str, b: &str) -> f64 {
    let na = normalize_text(a);
    let nb = normalize_text(b);
    let sa: std::collections::HashSet<&str> = na.split_whitespace().collect();
    let sb: std::collections::HashSet<&str> = nb.split_whitespace().collect();
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count() as f64;
    let union = sa.union(&sb).count() as f64;
    inter / union
}
