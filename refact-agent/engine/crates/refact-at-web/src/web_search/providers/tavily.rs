use super::super::{WebSearchRequest, WebSearchConfig, SearchResult};
use super::super::http::{client, json};
use super::legacy_common::*;
use serde_json::Value;

const TAVILY_BACKEND_NAME: &str = "tavily";

fn parse_tavily_json(body: &str) -> Result<Vec<SearchResult>, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse Tavily JSON response: {}", e))?;
    if let Some(error) = value.get("error") {
        let detail = error
            .get("message")
            .and_then(|v| v.as_str())
            .or_else(|| error.as_str())
            .unwrap_or("Tavily returned an error");
        return Err(detail.to_string());
    }
    let Some(results) = value.get("results").and_then(|v| v.as_array()) else {
        return Err("Tavily response did not contain a `results` array".to_string());
    };

    let mut parsed = Vec::new();
    for item in results {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let snippet = obj
            .get("content")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("raw_content").and_then(|v| v.as_str()))
            .unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(title),
            url: url.to_string(),
            snippet: normalize_text(snippet),
            source: Some(TAVILY_BACKEND_NAME.to_string()),
            sources: vec![TAVILY_BACKEND_NAME.to_string()],
            published_date: obj
                .get("published_date")
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
    let body = request_body(request)?;
    let mut builder = client()?.post("https://api.tavily.com/search").json(&body);
    builder = match key.filter(|key| !key.trim().is_empty()) {
        Some(key) => builder.bearer_auth(key),
        None => builder.header("X-Tavily-Access-Mode", "keyless"),
    };
    let value = json(builder).await?;
    Ok(parse_tavily_json(&value.to_string())?
        .into_iter()
        .take(request.num_results)
        .collect())
}

fn request_body(request: &WebSearchRequest) -> Result<Value, String> {
    let mut body = serde_json::json!({"query": query(request)?, "max_results": request.num_results.min(20), "search_depth": "basic", "include_answer": false, "include_raw_content": false, "include_domains": request.allowed_domains, "exclude_domains": request.blocked_domains});
    if let Some(freshness) = &request.freshness {
        body["time_range"] = serde_json::json!(freshness.as_str());
    }
    if matches!(request.category, super::super::SearchCategory::News) {
        body["topic"] = serde_json::json!("news");
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_tavily_json_extracts_results() {
        let body = r#"{
          "query": "typescript best practices",
          "follow_up_questions": null,
          "answer": null,
          "images": [],
          "results": [
            {
              "url": "https://dev.to/example/typescript-best-practices",
              "title": "TypeScript Best Practices in 2025",
              "content": "TypeScript Best Practices in 2025: A Comprehensive Guide.",
              "score": 0.92857105,
              "raw_content": null
            }
          ],
          "response_time": 0.86,
          "request_id": "4245bea1-66ae-4ee9-a6b0-c36b3bf43404"
        }"#;

        let results = parse_tavily_json(body).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "TypeScript Best Practices in 2025");
        assert_eq!(
            results[0].url,
            "https://dev.to/example/typescript-best-practices"
        );
        assert_eq!(
            results[0].snippet,
            "TypeScript Best Practices in 2025: A Comprehensive Guide."
        );
        assert_eq!(results[0].source.as_deref(), Some(TAVILY_BACKEND_NAME));
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;
    #[test]
    fn passes_native_filters_and_news_topic() {
        let request = WebSearchRequest {
            queries: vec!["latest updates".to_string()],
            allowed_domains: vec!["example.com".to_string()],
            blocked_domains: vec!["blocked.com".to_string()],
            freshness: Some(super::super::super::Freshness::Week),
            category: super::super::super::SearchCategory::News,
            ..Default::default()
        };
        let body = request_body(&request).expect("request body");
        assert_eq!(body["include_domains"], serde_json::json!(["example.com"]));
        assert_eq!(body["exclude_domains"], serde_json::json!(["blocked.com"]));
        assert_eq!(body["time_range"], "week");
        assert_eq!(body["topic"], "news");
    }
}
