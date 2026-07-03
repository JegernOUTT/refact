use std::collections::HashMap;

use serde::Serialize;

use crate::selection_scoring::PageCandidate;

const BM25_K1: f64 = 1.5;
const BM25_B: f64 = 0.75;
const RRF_K: f64 = 60.0;

#[derive(Debug, Clone, Serialize)]
pub struct ScoredDoc<'a> {
    pub page: &'a PageCandidate,
    pub score: f64,
    pub method: String,
}

#[derive(Debug, Clone)]
struct CorpusStats {
    avgdl: f64,
    df: HashMap<String, usize>,
    n: usize,
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() > 2)
        .map(str::to_string)
        .collect()
}

fn document_tokens(page: &PageCandidate) -> Vec<String> {
    tokenize(&format!(
        "{} {:?} {}",
        page.id,
        page.kind,
        page.paths.join(" ")
    ))
}

fn build_corpus_stats(docs: &[Vec<String>]) -> CorpusStats {
    let n = docs.len();
    let total_len: usize = docs.iter().map(Vec::len).sum();
    let avgdl = if n == 0 {
        0.0
    } else {
        total_len as f64 / n as f64
    };

    let mut df = HashMap::new();
    for doc in docs {
        let mut seen = std::collections::HashSet::new();
        for token in doc {
            if seen.insert(token) {
                *df.entry(token.clone()).or_insert(0) += 1;
            }
        }
    }

    CorpusStats { avgdl, df, n }
}

fn term_counts(tokens: &[String]) -> HashMap<&str, usize> {
    let mut counts = HashMap::new();
    for token in tokens {
        *counts.entry(token.as_str()).or_insert(0) += 1;
    }
    counts
}

fn bm25(query_tokens: &[String], doc_tokens: &[String], corpus_stats: &CorpusStats) -> f64 {
    if query_tokens.is_empty()
        || doc_tokens.is_empty()
        || corpus_stats.n == 0
        || corpus_stats.avgdl <= 0.0
    {
        return 0.0;
    }

    let tf = term_counts(doc_tokens);
    let mut query_terms = query_tokens.to_vec();
    query_terms.sort();
    query_terms.dedup();

    let doc_len = doc_tokens.len() as f64;
    let length_norm = 1.0 - BM25_B + BM25_B * (doc_len / corpus_stats.avgdl);

    query_terms
        .iter()
        .map(|term| {
            let Some(&freq) = tf.get(term.as_str()) else {
                return 0.0;
            };
            let df = *corpus_stats.df.get(term).unwrap_or(&0) as f64;
            let n = corpus_stats.n as f64;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            let freq = freq as f64;
            idf * (freq * (BM25_K1 + 1.0)) / (freq + BM25_K1 * length_norm)
        })
        .sum()
}

fn pagerank_prior(page: &PageCandidate, prior: &HashMap<String, f64>) -> f64 {
    let value = prior.get(&page.id).copied().unwrap_or(0.0);
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

pub fn rrf_fuse(rankings: &[Vec<usize>], k: f64) -> Vec<(usize, f64)> {
    let k = if k > 0.0 { k } else { RRF_K };
    let mut scores: HashMap<usize, f64> = HashMap::new();

    for ranking in rankings {
        for (rank, doc_idx) in ranking.iter().enumerate() {
            *scores.entry(*doc_idx).or_insert(0.0) += 1.0 / (k + rank as f64);
        }
    }

    let mut fused: Vec<(usize, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    fused
}

pub fn search_hybrid<'a>(
    query: &str,
    pages: &'a [PageCandidate],
    prior: &HashMap<String, f64>,
) -> Vec<ScoredDoc<'a>> {
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() || pages.is_empty() {
        return vec![];
    }

    let docs: Vec<Vec<String>> = pages.iter().map(document_tokens).collect();
    let stats = build_corpus_stats(&docs);

    let mut bm25_scores: Vec<(usize, f64)> = docs
        .iter()
        .enumerate()
        .map(|(idx, doc)| (idx, bm25(&query_tokens, doc, &stats)))
        .filter(|(_, score)| *score > 0.0)
        .collect();
    bm25_scores.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| pages[a.0].id.cmp(&pages[b.0].id))
    });
    let bm25_ranking: Vec<usize> = bm25_scores.into_iter().map(|(idx, _)| idx).collect();
    if bm25_ranking.is_empty() {
        return vec![];
    }

    let mut prior_ranking = bm25_ranking.clone();
    prior_ranking.sort_by(|a, b| {
        pagerank_prior(&pages[*b], prior)
            .total_cmp(&pagerank_prior(&pages[*a], prior))
            .then_with(|| pages[*a].id.cmp(&pages[*b].id))
    });

    let mut docs: Vec<ScoredDoc<'a>> = rrf_fuse(&[bm25_ranking, prior_ranking], RRF_K)
        .into_iter()
        .map(|(idx, score)| ScoredDoc {
            page: &pages[idx],
            score,
            method: "hybrid".to_string(),
        })
        .collect();
    docs.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| pagerank_prior(b.page, prior).total_cmp(&pagerank_prior(a.page, prior)))
            .then_with(|| a.page.id.cmp(&b.page.id))
    });
    docs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection_scoring::PageKind;

    fn page(id: &str, path: &str) -> PageCandidate {
        PageCandidate {
            id: id.to_string(),
            kind: PageKind::File,
            score: 1.0,
            paths: vec![path.to_string()],
        }
    }

    #[test]
    fn search_hybrid_ranks_auth_token_login_first() {
        let pages = vec![
            page(
                "file:src/render.rs",
                "src/render.rs draws widgets on screen",
            ),
            page(
                "file:src/auth.rs",
                "src/auth.rs handles token login auth validation",
            ),
            page(
                "file:src/storage.rs",
                "src/storage.rs persists user preferences",
            ),
        ];
        let prior = HashMap::from([
            ("file:src/render.rs".to_string(), 1.0),
            ("file:src/auth.rs".to_string(), 0.7),
            ("file:src/storage.rs".to_string(), 0.9),
        ]);

        let hits = search_hybrid("how does login token work", &pages, &prior);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].page.id, "file:src/auth.rs");
        assert_eq!(hits[0].method, "hybrid");
        assert!(search_hybrid("a an to", &pages, &prior).is_empty());
    }

    #[test]
    fn search_hybrid_returns_empty_when_bm25_has_no_hits() {
        let pages = vec![
            page(
                "file:src/render.rs",
                "src/render.rs draws widgets on screen",
            ),
            page(
                "file:src/storage.rs",
                "src/storage.rs persists user preferences",
            ),
        ];

        assert!(search_hybrid("oauth token refresh", &pages, &HashMap::new()).is_empty());
    }

    #[test]
    fn search_hybrid_uses_prior() {
        let pages = vec![
            page("file:src/a.rs", "src/a.rs auth token"),
            page("file:src/b.rs", "src/b.rs auth token"),
        ];
        let prior = HashMap::from([
            ("file:src/a.rs".to_string(), 0.1),
            ("file:src/b.rs".to_string(), 0.9),
        ]);

        let hits = search_hybrid("auth token", &pages, &prior);

        assert_eq!(hits[0].page.id, "file:src/b.rs");
    }

    #[test]
    fn rrf_fuse_middle_index_is_competitive_for_opposite_rankings() {
        let fused = rrf_fuse(&[vec![0, 1, 2], vec![2, 1, 0]], 60.0);
        let score_1 = fused.iter().find(|(idx, _)| *idx == 1).unwrap().1;
        let best = fused.first().unwrap().1;
        assert!(score_1 >= best * 0.99, "score_1={score_1}, best={best}");
    }
}
