use std::collections::HashSet;

pub const DEFAULT_DEDUP_TAU: f64 = 0.83;
pub const DECISION_VECTOR_PREFIX: &str = "decision:";
pub const DECISION_PAGE_TYPE: &str = "decision_record";
pub const SEARCH_FETCH: usize = 50;

pub struct SearchHit {
    pub page_id: String,
    pub score: f64,
}

pub fn decision_match_text(title: &str, decision: &str) -> String {
    let t = title.trim();
    let d = decision.trim();
    if d.is_empty() {
        t.to_string()
    } else {
        format!("{}\n{}", t, d).trim().to_string()
    }
}

pub fn decision_page_id(decision_id: &str) -> String {
    format!("{}{}", DECISION_VECTOR_PREFIX, decision_id)
}

pub fn find_duplicate_decision(
    results: &[SearchHit],
    tau: f64,
    exclude_ids: &HashSet<String>,
) -> Option<String> {
    for hit in results {
        let Some(decision_id) = hit.page_id.strip_prefix(DECISION_VECTOR_PREFIX) else {
            continue;
        };
        if exclude_ids.contains(decision_id) {
            continue;
        }
        return if hit.score >= tau {
            Some(decision_id.to_string())
        } else {
            None
        };
    }
    None
}

pub fn find_related_decisions(
    results: &[SearchHit],
    lo: f64,
    hi: f64,
    exclude_ids: &HashSet<String>,
    limit: usize,
) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for hit in results {
        if out.len() == limit {
            break;
        }
        let Some(decision_id) = hit.page_id.strip_prefix(DECISION_VECTOR_PREFIX) else {
            continue;
        };
        if exclude_ids.contains(decision_id) {
            continue;
        }
        if lo <= hit.score && hit.score < hi {
            out.push((decision_id.to_string(), hit.score));
        }
    }
    out
}

pub fn is_duplicate_decision_text(a: &str, b: &str) -> bool {
    crate::evolution::text_similarity(a, b) >= DEFAULT_DEDUP_TAU
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(page_id: &str, score: f64) -> SearchHit {
        SearchHit {
            page_id: page_id.to_string(),
            score,
        }
    }

    #[test]
    fn decision_match_text_joins_title_and_decision() {
        assert_eq!(decision_match_text("T", "D"), "T\nD");
    }

    #[test]
    fn decision_match_text_returns_title_when_decision_empty() {
        assert_eq!(decision_match_text("T", ""), "T");
    }

    #[test]
    fn decision_page_id_prefixes_id() {
        assert_eq!(decision_page_id("abc"), "decision:abc");
    }

    #[test]
    fn find_duplicate_decision_returns_first_surviving_above_tau() {
        let results = vec![hit("other:skip", 1.0), hit("decision:abc", 0.84)];
        let exclude_ids = HashSet::new();
        assert_eq!(
            find_duplicate_decision(&results, DEFAULT_DEDUP_TAU, &exclude_ids),
            Some("abc".to_string())
        );
    }

    #[test]
    fn find_duplicate_decision_short_circuits_below_tau() {
        let results = vec![hit("decision:low", 0.82), hit("decision:high", 0.99)];
        let exclude_ids = HashSet::new();
        assert_eq!(
            find_duplicate_decision(&results, DEFAULT_DEDUP_TAU, &exclude_ids),
            None
        );
    }

    #[test]
    fn find_duplicate_decision_skips_non_decision_hits_and_excluded_ids() {
        let results = vec![
            hit("page:skip", 1.0),
            hit("decision:excluded", 0.99),
            hit("decision:kept", 0.83),
        ];
        let exclude_ids = HashSet::from(["excluded".to_string()]);
        assert_eq!(
            find_duplicate_decision(&results, DEFAULT_DEDUP_TAU, &exclude_ids),
            Some("kept".to_string())
        );
    }

    #[test]
    fn find_related_decisions_returns_hits_in_band_up_to_limit() {
        let results = vec![
            hit("page:skip", 0.9),
            hit("decision:excluded", 0.7),
            hit("decision:too_high", 0.83),
            hit("decision:first", 0.82),
            hit("decision:too_low", 0.49),
            hit("decision:second", 0.7),
            hit("decision:third", 0.6),
        ];
        let exclude_ids = HashSet::from(["excluded".to_string()]);
        assert_eq!(
            find_related_decisions(&results, 0.5, 0.83, &exclude_ids, 2),
            vec![("first".to_string(), 0.82), ("second".to_string(), 0.7)]
        );
    }

    #[test]
    fn find_related_decisions_with_zero_limit_returns_empty() {
        let results = vec![hit("decision:abc", 0.7)];
        let exclude_ids = HashSet::new();
        assert!(find_related_decisions(&results, 0.5, 0.83, &exclude_ids, 0).is_empty());
    }

    #[test]
    fn duplicate_decision_text_uses_default_tau() {
        assert!(is_duplicate_decision_text(
            "Use SQLite for storage",
            "Use SQLite for storage storage"
        ));
    }
}
