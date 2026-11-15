use super::super::{WebSearchRequest, WebSearchConfig, SearchResult};
use super::super::http::{client, json};
use super::legacy_common::*;
use serde_json::Value;

const OPENALEX_BACKEND_NAME: &str = "openalex";

use std::collections::BTreeMap;

fn openalex_abstract_from_inverted_index(value: &Value) -> String {
    let Some(index) = value.as_object() else {
        return String::new();
    };

    let mut words_by_position: BTreeMap<usize, &str> = BTreeMap::new();
    for (word, positions) in index {
        let Some(positions) = positions.as_array() else {
            continue;
        };
        for position in positions {
            if let Some(position) = position.as_u64() {
                words_by_position.insert(position as usize, word);
            }
        }
    }

    words_by_position
        .values()
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse(value: &Value, num_results: usize) -> Result<Vec<SearchResult>, String> {
    let Some(results) = value.get("results").and_then(|v| v.as_array()) else {
        return Ok(vec![]);
    };

    let mut parsed = Vec::new();
    for item in results.iter().take(num_results) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("display_name").and_then(|v| v.as_str()))
            .unwrap_or_default();
        let url = obj
            .get("doi")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("id").and_then(|v| v.as_str()))
            .unwrap_or_default();
        let year = obj
            .get("publication_year")
            .and_then(|v| v.as_i64())
            .map(|v| v.to_string())
            .unwrap_or_default();
        let abstract_text = obj
            .get("abstract_inverted_index")
            .map(openalex_abstract_from_inverted_index)
            .unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let snippet = if abstract_text.is_empty() {
            year
        } else if year.is_empty() {
            truncate_chars(&abstract_text, 280)
        } else {
            format!("{}. {}", year, truncate_chars(&abstract_text, 260))
        };
        parsed.push(SearchResult {
            title: normalize_text(title),
            url: url.to_string(),
            snippet: normalize_text(&snippet),
            source: Some(OPENALEX_BACKEND_NAME.to_string()),
            sources: vec![OPENALEX_BACKEND_NAME.to_string()],
            published_date: obj
                .get("publication_date")
                .and_then(Value::as_str)
                .and_then(super::common::normalize_date),
            ..Default::default()
        });
    }

    Ok(parsed)
}

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    _key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    #[allow(unused_mut)]
    let mut builder = client()?
        .get("https://api.openalex.org/works")
        .header("User-Agent", "refact-agent")
        .query(&[
            ("search", &clean_query_for_source(query(request)?)),
            ("per-page", &request.num_results.min(200).to_string()),
        ]);
    if let Some(freshness) = &request.freshness {
        builder = builder.query(&[(
            "filter",
            format!("from_publication_date:{}", freshness.cutoff()),
        )]);
    }
    parse(&json(builder).await?, request.num_results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_parses_provider_results() {
        let value: Value = serde_json::from_str(r#"{"results":[{"display_name":"Research","doi":"https://doi.org/10.1/test","publication_year":2026,"abstract_inverted_index":{"world":[1],"Hello":[0]}}]}"#).expect("fixture JSON");
        let results = parse(&value, 5).expect("fixture results");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://doi.org/10.1/test");
        assert_eq!(results[0].sources, vec!["openalex"]);
        assert!(!results[0].title.is_empty());
    }
    #[test]
    fn empty_fixture_is_safe() {
        assert!(parse(&serde_json::json!({}), 5)
            .map(|items| items.is_empty())
            .unwrap_or(true));
    }
}
