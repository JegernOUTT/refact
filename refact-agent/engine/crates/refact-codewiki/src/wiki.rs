use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Abstraction over the LLM used to summarize code into wiki prose.
/// The engine wires a real implementation; tests use a mock.
pub trait SummarizerLlm {
    fn summarize(&self, module: &str, code: &str) -> String;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WikiEntry {
    pub module: String,
    pub summary: String,
    pub source_hash: u64,
    pub freshness: f64,
}

fn cheap_hash(s: &str) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

pub fn generate_entry(llm: &dyn SummarizerLlm, module: &str, code: &str) -> WikiEntry {
    WikiEntry {
        module: module.to_string(),
        summary: llm.summarize(module, code),
        source_hash: cheap_hash(code),
        freshness: 1.0,
    }
}

/// Returns true if the entry is stale relative to the current code.
pub fn is_stale(entry: &WikiEntry, current_code: &str) -> bool {
    entry.source_hash != cheap_hash(current_code)
}

fn tokenize(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(|t| t.to_string())
        .collect()
}

/// Keyword-overlap RAG retrieval over wiki entries (module name + summary).
#[deprecated(since = "0.1.0", note = "use rag::search_hybrid")]
pub fn search_wiki<'a>(entries: &'a [WikiEntry], query: &str, top_n: usize) -> Vec<&'a WikiEntry> {
    let q = tokenize(query);
    if q.is_empty() {
        return vec![];
    }
    let mut scored: Vec<(usize, &WikiEntry)> = entries
        .iter()
        .map(|e| {
            let doc = tokenize(&format!("{} {}", e.module, e.summary));
            let overlap = q.intersection(&doc).count();
            (overlap, e)
        })
        .filter(|(overlap, _)| *overlap > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.module.cmp(&b.1.module)));
    scored.into_iter().take(top_n).map(|(_, e)| e).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockLlm;
    impl SummarizerLlm for MockLlm {
        fn summarize(&self, module: &str, code: &str) -> String {
            format!(
                "Module {module} handles {} lines of logic.",
                code.lines().count()
            )
        }
    }

    #[test]
    fn generate_entry_uses_llm_and_hashes_source() {
        let entry = generate_entry(&MockLlm, "auth", "fn a() {}\nfn b() {}\n");
        assert_eq!(entry.module, "auth");
        assert!(entry.summary.contains("Module auth"));
        assert!(!is_stale(&entry, "fn a() {}\nfn b() {}\n"));
        assert!(is_stale(&entry, "changed code"));
    }

    #[test]
    #[allow(deprecated)]
    fn wiki_search_ranks_by_keyword_overlap() {
        let entries = vec![
            WikiEntry {
                module: "auth".to_string(),
                summary: "handles login and token validation".to_string(),
                source_hash: 0,
                freshness: 1.0,
            },
            WikiEntry {
                module: "render".to_string(),
                summary: "draws widgets on screen".to_string(),
                source_hash: 0,
                freshness: 1.0,
            },
        ];
        let hits = search_wiki(&entries, "how does token login work", 5);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].module, "auth", "auth matches token/login: {hits:?}");
    }

    #[test]
    #[allow(deprecated)]
    fn wiki_search_empty_query_is_safe() {
        let entries: Vec<WikiEntry> = vec![];
        assert!(search_wiki(&entries, "anything", 5).is_empty());
    }
}
