use super::super::{WebSearchRequest, WebSearchConfig, SearchResult};
use super::super::http::{client, json};
use super::legacy_common::*;
use serde_json::Value;

const WIKIPEDIA_BACKEND_NAME: &str = "wikipedia";

fn parse(value: &Value, num_results: usize) -> Result<Vec<SearchResult>, String> {
    let Some(results) = value
        .get("query")
        .and_then(|v| v.get("search"))
        .and_then(|v| v.as_array())
    else {
        return Err("Wikipedia response did not contain query.search".to_string());
    };

    let mut parsed = Vec::new();
    for item in results.iter().take(num_results) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let page_id = obj
            .get("pageid")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let snippet = obj
            .get("snippet")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .replace("<span class=\"searchmatch\">", "")
            .replace("</span>", "");
        if title.is_empty() || page_id == 0 {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(title),
            url: format!("https://en.wikipedia.org/?curid={}", page_id),
            snippet: normalize_text(&snippet),
            source: Some(WIKIPEDIA_BACKEND_NAME.to_string()),
            sources: vec![WIKIPEDIA_BACKEND_NAME.to_string()],
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
    let builder = client()?
        .get("https://en.wikipedia.org/w/api.php")
        .header("User-Agent", "refact-agent")
        .query(&[
            ("action", "query"),
            ("list", "search"),
            ("srsearch", query(request)?),
            ("srlimit", &request.num_results.min(20).to_string()),
            ("utf8", "1"),
            ("format", "json"),
            ("formatversion", "2"),
        ]);

    parse(&json(builder).await?, request.num_results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_parses_provider_results() {
        let value: Value = serde_json::from_str(r#"{"query":{"search":[{"title":"Rust","pageid":42,"snippet":"A <span class=\"searchmatch\">language</span>"}]}}"#).expect("fixture JSON");
        let results = parse(&value, 5).expect("fixture results");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://en.wikipedia.org/?curid=42");
        assert_eq!(results[0].sources, vec!["wikipedia"]);
        assert!(!results[0].title.is_empty());
    }
    #[test]
    fn empty_fixture_is_safe() {
        assert!(parse(&serde_json::json!({}), 5)
            .map(|items| items.is_empty())
            .unwrap_or(true));
    }
}
