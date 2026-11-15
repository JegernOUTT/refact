use super::super::{WebSearchRequest, WebSearchConfig, SearchResult};
use super::super::http::{client, json};
use super::legacy_common::*;
use serde_json::Value;

const KEENABLE_BACKEND_NAME: &str = "keenable";

fn parse_keenable_json(body: &str) -> Result<Vec<SearchResult>, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse Keenable JSON response: {}", e))?;
    let Some(results) = value.get("results").and_then(|v| v.as_array()) else {
        return Err("Keenable response did not contain a `results` array".to_string());
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
            .get("snippet")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("description").and_then(|v| v.as_str()))
            .unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(title),
            url: url.to_string(),
            snippet: normalize_text(snippet),
            source: Some(KEENABLE_BACKEND_NAME.to_string()),
            sources: vec![KEENABLE_BACKEND_NAME.to_string()],
            published_date: obj
                .get("published_at")
                .and_then(Value::as_str)
                .and_then(super::common::normalize_date),
            ..Default::default()
        });
    }

    Ok(parsed)
}

fn parse_keenable_mcp_text(text: &str) -> Vec<SearchResult> {
    let mut parsed = Vec::new();
    for section in text.split("\n---\n") {
        let mut title = String::new();
        let mut url = String::new();
        let mut snippet_lines = Vec::new();
        let mut after_url = false;
        let mut published_date = None;

        for line in section.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix("Title:") {
                title = normalize_text(rest);
                continue;
            }
            if let Some(rest) = line.strip_prefix("URL:") {
                url = rest.trim().to_string();
                after_url = true;
                continue;
            }
            if let Some(date) = line.strip_prefix("Published:") {
                published_date = super::common::normalize_date(date);
                continue;
            }
            if line.is_empty() || line.starts_with("Acquired:") {
                continue;
            }
            if after_url {
                snippet_lines.push(line);
            }
        }

        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title,
            url,
            snippet: truncate_chars(&snippet_lines.join(" "), 500),
            source: Some(KEENABLE_BACKEND_NAME.to_string()),
            sources: vec![KEENABLE_BACKEND_NAME.to_string()],
            published_date,
            ..Default::default()
        });
    }

    parsed
}

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let parsed = if let Some(key) = key.filter(|key| !key.trim().is_empty()) {
        let value = json(
            client()?
                .post("https://api.keenable.ai/v1/search")
                .header("X-API-Key", key)
                .json(&serde_json::json!({"query": query(request)?})),
        )
        .await?;
        parse_keenable_json(&value.to_string())?
    } else {
        let value = super::super::mcp::call(
            "https://api.keenable.ai/mcp",
            reqwest::header::HeaderMap::new(),
            "search_web_pages",
            serde_json::json!({"query": query(request)?}),
        )
        .await?;
        match value {
            Value::String(text) => parse_keenable_mcp_text(&text),
            value => parse_keenable_json(&value.to_string())?,
        }
    };
    Ok(parsed.into_iter().take(request.num_results).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_keenable_json_extracts_results() {
        let body = r#"{
          "query": "typescript best practices",
          "results": [
            {
              "title": "TypeScript Best Practices 2026",
              "url": "https://example.com/ts-best-practices",
              "description": "A comprehensive guide to modern TypeScript patterns.",
              "snippet": "TypeScript Best Practices 2026 Use strict mode...",
              "published_at": "2026-01-15T10:30:00Z",
              "acquired_at": "2026-01-16T08:12:34Z"
            }
          ]
        }"#;

        let results = parse_keenable_json(body).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "TypeScript Best Practices 2026");
        assert_eq!(results[0].url, "https://example.com/ts-best-practices");
        assert_eq!(
            results[0].snippet,
            "TypeScript Best Practices 2026 Use strict mode..."
        );
        assert_eq!(results[0].source.as_deref(), Some(KEENABLE_BACKEND_NAME));
    }

    #[test]
    fn test_parse_keenable_mcp_text_extracts_results() {
        let text = r#"Title: W3Schools.com
URL: https://www.w3schools.com/typescript/typescript_best_practices.php
Acquired: 2026-05-16

TypeScript Best Practices This guide covers essential TypeScript best practices.

---

Title: TypeScript Best Practices for Production Code in 2026
URL: https://dev.to/example/typescript-best-practices
Published: 2026-03-25
Acquired: 2026-05-11

This guide covers 15 best practices that separate production-quality TypeScript.
"#;

        let results = parse_keenable_mcp_text(text);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "W3Schools.com");
        assert_eq!(
            results[0].url,
            "https://www.w3schools.com/typescript/typescript_best_practices.php"
        );
        assert_eq!(
            results[0].snippet,
            "TypeScript Best Practices This guide covers essential TypeScript best practices."
        );
        assert_eq!(results[0].source.as_deref(), Some(KEENABLE_BACKEND_NAME));
        assert_eq!(
            results[1].title,
            "TypeScript Best Practices for Production Code in 2026"
        );
    }
}
