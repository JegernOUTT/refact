use super::super::{WebSearchRequest, WebSearchConfig, SearchResult};
use super::super::http::{client, request_error, text};
use super::legacy_common::*;
use serde_json::Value;

const SEARXNG_BACKEND_NAME: &str = "searxng";

fn parse_searxng_json(body: &str) -> Result<Vec<SearchResult>, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse SearxNG JSON response: {}", e))?;
    let Some(results) = value.get("results").and_then(|v| v.as_array()) else {
        return Err("SearxNG response did not contain a `results` array".to_string());
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
            .or_else(|| obj.get("snippet").and_then(|v| v.as_str()))
            .unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(title),
            url: url.to_string(),
            snippet: normalize_text(snippet),
            source: Some(SEARXNG_BACKEND_NAME.to_string()),
            sources: vec![SEARXNG_BACKEND_NAME.to_string()],
            published_date: obj
                .get("publishedDate")
                .and_then(Value::as_str)
                .and_then(super::common::normalize_date),
            ..Default::default()
        });
    }

    Ok(parsed)
}

pub(super) async fn search(
    request: &WebSearchRequest,
    config: &WebSearchConfig,
    _key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let endpoint = config
        .searxng_url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
        .ok_or_else(|| "SearxNG requires a configured URL".to_string())?;
    let mut url =
        url::Url::parse(endpoint).map_err(|error| format!("Invalid SearxNG URL: {error}"))?;
    if !url.path().trim_end_matches('/').ends_with("/search") {
        url.set_path(&format!("{}/search", url.path().trim_end_matches('/')));
    }
    url.query_pairs_mut()
        .append_pair("q", query(request)?)
        .append_pair("format", "json");
    if let Some(freshness) = &request.freshness {
        url.query_pairs_mut()
            .append_pair("time_range", freshness.as_str());
    }
    let response = client()?.get(url).send().await.map_err(request_error)?;
    let status = response.status();
    let is_json = response
        .headers()
        .get("content-type")
        .and_then(|header| header.to_str().ok())
        .is_some_and(|header| header.contains("json"));
    let body = text(response).await?;
    parse_response(status.as_u16(), is_json, &body)
        .map(|results| results.into_iter().take(request.num_results).collect())
}

fn parse_response(status: u16, is_json: bool, body: &str) -> Result<Vec<SearchResult>, String> {
    if !(200..300).contains(&status) {
        return Err(format!("SearxNG returned HTTP {status}"));
    }
    if !is_json {
        return Err("SearxNG did not return JSON".to_string());
    }
    parse_searxng_json(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_searxng_json_extracts_results() {
        let body = r#"{
          "results": [
            {
              "title": "Rust Book",
              "url": "https://doc.rust-lang.org/book/",
              "content": "Read the official Rust book."
            }
          ]
        }"#;

        let results = parse_searxng_json(body).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rust Book");
        assert_eq!(results[0].source.as_deref(), Some(SEARXNG_BACKEND_NAME));
    }
}

#[cfg(test)]
mod response_tests {
    use super::*;
    #[test]
    fn rejects_html_and_challenge_even_on_success() {
        let results = parse_response(200, true, r#"{"results":[{"title":"CAPTCHA challenge","url":"https://example.org","content":"too many requests; verify you are human"}]}"#).expect("ordinary search content");
        assert_eq!(results.len(), 1);
        assert!(parse_response(200, false, "<html>results</html>").is_err());
        assert!(parse_response(200, true, "<html>captcha challenge</html>").is_err());
        assert!(parse_response(429, true, r#"{"results":[]}"#).is_err());
        assert!(parse_response(200, true, r#"{"results":[]}"#)
            .expect("empty results")
            .is_empty());
    }
}
