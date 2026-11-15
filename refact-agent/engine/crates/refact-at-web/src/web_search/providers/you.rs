use super::super::{mcp, SearchResult, WebSearchConfig, WebSearchRequest};
use super::common;
use serde_json::{json, Value};

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let mut args = json!({"query":common::domain_query(common::query(request)?, request),"count":request.num_results.clamp(1,50),"extraction":"highlights"});
    if let Some(freshness) = &request.freshness {
        args["freshness"] = json!(freshness.as_str());
    }
    if !request.blocked_domains.is_empty() {
        args["exclude_domains"] = json!(request.blocked_domains);
    }
    let endpoint = if key.is_some_and(|key| !key.is_empty()) {
        "https://api.you.com/mcp"
    } else {
        "https://api.you.com/mcp?profile=free"
    };
    let value = mcp::call(
        endpoint,
        common::headers("authorization", key, true)?,
        "you-search",
        args,
    )
    .await?;
    parse(common::decode(value))
}

fn parse(value: Value) -> Result<Vec<SearchResult>, String> {
    let results = value.get("results").unwrap_or(&value);
    let mut parsed = Vec::new();
    let mut found = false;
    for field in ["web", "news"] {
        if let Some(rows) = results.get(field).and_then(Value::as_array) {
            found = true;
            parsed.extend(rows.iter().filter_map(|row| {
                common::result(
                    row,
                    "you",
                    &["title"],
                    &["url"],
                    &["highlights", "snippets", "description", "snippet"],
                    &["page_age", "published_date", "age"],
                )
            }));
        }
    }
    if !found {
        return Err("Invalid You response: missing web/news results".into());
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn web_and_news() {
        let value = json!({"results":{"web":[{"title":"Web","url":"https://example.org","snippets":["First","Second"]}],"news":[{"title":"News","url":"https://example.org/news","description":"News summary","page_age":"2025-06-01"}]}});
        let results = parse(common::decode(json!(value.to_string()))).expect("results");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].snippet, "First\nSecond");
        assert_eq!(results[1].published_date.as_deref(), Some("2025-06-01"));
        assert!(parse(json!({"results":{"web":[]}}))
            .expect("empty")
            .is_empty());
        assert!(parse(json!({"error":"quota"})).is_err());
    }
}
