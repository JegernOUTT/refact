use super::super::{WebSearchRequest, WebSearchConfig, SearchResult};
use super::super::http::{client, json};
use super::legacy_common::*;
use serde_json::Value;

const STACK_EXCHANGE_BACKEND_NAME: &str = "stack_overflow";

fn parse(value: &Value, num_results: usize) -> Result<Vec<SearchResult>, String> {
    if let Some(error) = value.get("error_message").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    if let Some(backoff) = value.get("backoff").and_then(Value::as_u64) {
        return Err(format!(
            "Stack Exchange rate limit: backoff {backoff} seconds"
        ));
    }
    let Some(items) = value.get("items").and_then(|v| v.as_array()) else {
        return Ok(vec![]);
    };

    let mut parsed = Vec::new();
    for item in items.iter().take(num_results) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let url = obj.get("link").and_then(|v| v.as_str()).unwrap_or_default();
        let score = obj
            .get("score")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let answers = obj
            .get("answer_count")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let accepted = obj
            .get("is_answered")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(&unescape_html_light(title)),
            url: url.to_string(),
            snippet: format!(
                "Score: {}. Answers: {}. Accepted answer: {}.",
                score,
                answers,
                if accepted { "yes" } else { "no" }
            ),
            source: Some(STACK_EXCHANGE_BACKEND_NAME.to_string()),
            sources: vec![STACK_EXCHANGE_BACKEND_NAME.to_string()],
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
        .get("https://api.stackexchange.com/2.3/search/advanced")
        .header("User-Agent", "refact-agent")
        .query(&[
            ("order", "desc"),
            ("sort", "relevance"),
            ("site", "stackoverflow"),
            ("q", &clean_query_for_source(query(request)?)),
            ("pagesize", &request.num_results.min(100).to_string()),
        ]);
    if let Some(time) = timestamp(request) {
        builder = builder.query(&[("fromdate", time.to_string())]);
    }
    parse(&json(builder).await?, request.num_results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_parses_provider_results() {
        let value: Value = serde_json::from_str(r#"{"items":[{"title":"Rust &amp; friends","link":"https://stackoverflow.com/q/1","score":10,"answer_count":2,"is_answered":true}]}"#).expect("fixture JSON");
        let results = parse(&value, 5).expect("fixture results");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://stackoverflow.com/q/1");
        assert_eq!(results[0].sources, vec!["stack_overflow"]);
        assert!(!results[0].title.is_empty());
    }
    #[test]
    fn empty_fixture_is_safe() {
        assert!(parse(&serde_json::json!({}), 5)
            .map(|items| items.is_empty())
            .unwrap_or(true));
    }
}
