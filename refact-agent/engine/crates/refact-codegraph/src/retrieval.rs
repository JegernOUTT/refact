use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::store::Store;

const RRF_K: f32 = 60.0;
const NEIGHBOR_DISCOUNT: f32 = 0.25;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeHit {
    pub path: String,
    pub line1: usize,
    pub line2: usize,
    pub symbol: Option<String>,
    pub score: f32,
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_lowercase())
        .collect()
}

fn fts_match_query(terms: &[String]) -> String {
    terms
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

pub fn search_hybrid(store: &Store, query: &str, limit: usize) -> Result<Vec<CodeHit>, String> {
    let terms = query_terms(query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let fetch = (limit * 3).max(10) as i64;

    let mut scores: HashMap<String, f32> = HashMap::new();
    let mut best_span: HashMap<String, (usize, usize, Option<String>)> = HashMap::new();

    let fts = store.fts_ranked(&fts_match_query(&terms), fetch)?;
    for (rank, (path, _bm25)) in fts.iter().enumerate() {
        *scores.entry(path.clone()).or_insert(0.0) += 1.0 / (RRF_K + rank as f32);
    }

    let mut symbol_rank = 0usize;
    for term in &terms {
        for (path, name, line1, line2) in store.symbol_name_ranked(term, fetch)? {
            *scores.entry(path.clone()).or_insert(0.0) += 1.0 / (RRF_K + symbol_rank as f32);
            best_span
                .entry(path.clone())
                .or_insert((line1 as usize, line2 as usize, Some(name)));
            symbol_rank += 1;
        }
    }

    let seeds: Vec<String> = scores.keys().cloned().collect();
    for seed in seeds {
        let seed_score = *scores.get(&seed).unwrap_or(&0.0);
        for neighbor in store.neighbor_paths(&seed)? {
            *scores.entry(neighbor).or_insert(0.0) += seed_score * NEIGHBOR_DISCOUNT;
        }
    }

    let mut hits: Vec<CodeHit> = scores
        .into_iter()
        .map(|(path, score)| {
            let (line1, line2, symbol) = best_span.get(&path).cloned().unwrap_or((1, 1, None));
            CodeHit {
                path,
                line1,
                line2,
                symbol,
                score,
            }
        })
        .collect();

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });
    hits.truncate(limit);
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_files() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .index_file_graph(
                "src/auth.rs",
                "fn authenticate_user(token: &str) -> bool { validate_token(token) }\nfn validate_token(t: &str) -> bool { true }\n",
                "rust",
            )
            .unwrap();
        store
            .index_file_graph(
                "src/widget.rs",
                "struct Widget;\nimpl Widget { fn render(&self) {} }\n",
                "rust",
            )
            .unwrap();
        store.connect_usages().unwrap();
        store
    }

    #[test]
    fn hybrid_search_ranks_symbol_name_match_first() {
        let store = store_with_files();
        let hits = search_hybrid(&store, "authenticate", 10).unwrap();
        assert!(!hits.is_empty(), "expected hits for 'authenticate'");
        assert_eq!(hits[0].path, "src/auth.rs");
        assert!(hits
            .iter()
            .any(|h| h.symbol.as_deref() == Some("authenticate_user")));
    }

    #[test]
    fn hybrid_search_matches_fts_content() {
        let store = store_with_files();
        let hits = search_hybrid(&store, "render", 10).unwrap();
        assert!(
            hits.iter().any(|h| h.path == "src/widget.rs"),
            "expected widget.rs for 'render', got {hits:?}"
        );
    }

    #[test]
    fn hybrid_search_empty_query_returns_empty() {
        let store = store_with_files();
        let hits = search_hybrid(&store, "  ", 10).unwrap();
        assert!(hits.is_empty());
    }
}
