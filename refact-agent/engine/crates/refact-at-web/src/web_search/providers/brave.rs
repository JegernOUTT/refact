use super::super::{WebSearchRequest, WebSearchConfig, SearchResult};
use super::super::http::{client, json};
use super::legacy_common::*;
use serde_json::Value;

const BRAVE_BACKEND_NAME: &str = "brave";

fn parse(value: &Value, num_results: usize) -> Result<Vec<SearchResult>, String> {
    let Some(results) = value
        .get("web")
        .and_then(|v| v.get("results"))
        .and_then(|v| v.as_array())
    else {
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
            .unwrap_or_default();
        let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let snippet = obj
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(&unescape_html_light(title)),
            url: url.to_string(),
            snippet: strip_html_tags(snippet),
            source: Some(BRAVE_BACKEND_NAME.to_string()),
            sources: vec![BRAVE_BACKEND_NAME.to_string()],
            published_date: obj
                .get("page_age")
                .or_else(|| obj.get("age"))
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
    key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let key = key
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| "Brave requires an API key".to_string())?;
    let mut builder = client()?
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", key)
        .query(&[
            ("q", query(request)?),
            ("count", &request.num_results.min(20).to_string()),
            ("search_lang", "en"),
            ("spellcheck", "1"),
        ]);
    if let Some(freshness) = &request.freshness {
        let freshness = match freshness.as_str() {
            "day" => "pd",
            "week" => "pw",
            "month" => "pm",
            "year" => "py",
            _ => "",
        };
        if !freshness.is_empty() {
            builder = builder.query(&[("freshness", freshness)]);
        }
    }
    parse(&json(builder).await?, request.num_results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_parses_provider_results() {
        let value: Value = serde_json::from_str(r#"{"web":{"results":[{"title":"Rust &amp; friends","url":"https://example.com","description":"A <b>language</b>"}]}}"#).expect("fixture JSON");
        let results = parse(&value, 5).expect("fixture results");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].sources, vec!["brave"]);
        assert!(!results[0].title.is_empty());
    }
    #[test]
    fn empty_fixture_is_safe() {
        let result = parse(&serde_json::json!({"web":{"results":[{"title":"News", "url":"https://example.org", "age":"Jan 2, 2025"}]}}), 5).unwrap();
        assert_eq!(result[0].published_date.as_deref(), Some("2025-01-02"));
        assert!(parse(&serde_json::json!({}), 5)
            .map(|items| items.is_empty())
            .unwrap_or(true));
    }
}
