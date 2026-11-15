use super::super::{WebSearchRequest, WebSearchConfig, SearchResult};
use super::super::http::{client, json};
use super::legacy_common::*;
use serde_json::Value;

const CROSSREF_BACKEND_NAME: &str = "crossref";

fn parse(value: &Value, num_results: usize) -> Result<Vec<SearchResult>, String> {
    let Some(items) = value
        .get("message")
        .and_then(|v| v.get("items"))
        .and_then(|v| v.as_array())
    else {
        return Ok(vec![]);
    };

    let mut parsed = Vec::new();
    for item in items.iter().take(num_results) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_array())
            .and_then(|v| v.first())
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let doi = obj.get("DOI").and_then(|v| v.as_str()).unwrap_or_default();
        let url = obj
            .get("URL")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("https://doi.org/{}", doi));
        let publisher = obj
            .get("publisher")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let publication = obj
            .get("container-title")
            .and_then(|v| v.as_array())
            .and_then(|v| v.first())
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if title.is_empty() || doi.is_empty() {
            continue;
        }
        let snippet = [publication, publisher]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(". ");
        parsed.push(SearchResult {
            title: normalize_text(title),
            url,
            snippet,
            source: Some(CROSSREF_BACKEND_NAME.to_string()),
            sources: vec![CROSSREF_BACKEND_NAME.to_string()],
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
        .get("https://api.crossref.org/works")
        .header("User-Agent", "refact-agent")
        .query(&[
            (
                "query.bibliographic",
                &clean_query_for_source(query(request)?),
            ),
            ("rows", &request.num_results.min(1000).to_string()),
        ]);
    if let Some(freshness) = &request.freshness {
        builder = builder.query(&[("filter", format!("from-pub-date:{}", freshness.cutoff()))]);
    }
    parse(&json(builder).await?, request.num_results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_parses_provider_results() {
        let value: Value = serde_json::from_str(r#"{"message":{"items":[{"title":["Research"],"DOI":"10.1/test","publisher":"Publisher","container-title":["Journal"]}]}}"#).expect("fixture JSON");
        let results = parse(&value, 5).expect("fixture results");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://doi.org/10.1/test");
        assert_eq!(results[0].sources, vec!["crossref"]);
        assert!(!results[0].title.is_empty());
    }
    #[test]
    fn empty_fixture_is_safe() {
        assert!(parse(&serde_json::json!({}), 5)
            .map(|items| items.is_empty())
            .unwrap_or(true));
    }
}
