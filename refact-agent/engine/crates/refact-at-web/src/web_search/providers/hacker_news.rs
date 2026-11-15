use super::super::{WebSearchRequest, WebSearchConfig, SearchResult};
use super::super::http::{client, json};
use super::legacy_common::*;
use serde_json::Value;

const HACKER_NEWS_BACKEND_NAME: &str = "hacker_news";

fn parse(value: &Value, num_results: usize) -> Result<Vec<SearchResult>, String> {
    let Some(hits) = value.get("hits").and_then(|v| v.as_array()) else {
        return Ok(vec![]);
    };

    let mut parsed = Vec::new();
    for item in hits.iter().take(num_results) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("story_title").and_then(|v| v.as_str()))
            .unwrap_or_default();
        let object_id = obj
            .get("objectID")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let points = obj
            .get("points")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let comments = obj
            .get("num_comments")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let linked_url = obj.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        if title.is_empty() || object_id.is_empty() {
            continue;
        }
        let url = if linked_url.is_empty() {
            format!("https://news.ycombinator.com/item?id={}", object_id)
        } else {
            linked_url.to_string()
        };
        parsed.push(SearchResult {
            title: normalize_text(&unescape_html_light(title)),
            url,
            snippet: format!("{} points. {} comments.", points, comments),
            source: Some(HACKER_NEWS_BACKEND_NAME.to_string()),
            sources: vec![HACKER_NEWS_BACKEND_NAME.to_string()],
            published_date: obj
                .get("created_at")
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
        .get("https://hn.algolia.com/api/v1/search")
        .header("User-Agent", "refact-agent")
        .query(&[
            ("query", clean_query_for_source(query(request)?).as_str()),
            ("tags", "story"),
            ("hitsPerPage", &request.num_results.min(100).to_string()),
        ]);
    if let Some(time) = timestamp(request) {
        builder = builder.query(&[("numericFilters", format!("created_at_i>{time}"))]);
    }
    parse(&json(builder).await?, request.num_results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_parses_provider_results() {
        let value: Value = serde_json::from_str(r#"{"hits":[{"title":"Hello &amp; world","objectID":"123","points":12,"num_comments":3}]}"#).expect("fixture JSON");
        let results = parse(&value, 5).expect("fixture results");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://news.ycombinator.com/item?id=123");
        assert_eq!(results[0].sources, vec!["hacker_news"]);
        assert!(!results[0].title.is_empty());
    }
    #[test]
    fn empty_fixture_is_safe() {
        assert!(parse(&serde_json::json!({}), 5)
            .map(|items| items.is_empty())
            .unwrap_or(true));
    }
}
