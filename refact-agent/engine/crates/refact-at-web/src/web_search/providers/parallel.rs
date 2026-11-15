use super::super::{mcp, SearchResult, WebSearchConfig, WebSearchRequest};
use super::common;
use serde_json::{json, Value};

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let queries: Vec<String> = request
        .queries
        .iter()
        .map(|query| common::domain_query(query, request))
        .collect();
    let mut objective = common::query(request)?.to_owned();
    if let Some(freshness) = &request.freshness {
        objective.push_str(&format!(
            "; prefer sources published after {}",
            freshness.cutoff()
        ));
    }
    let value = mcp::call(
        "https://search.parallel.ai/mcp",
        common::headers("authorization", key, true)?,
        "web_search",
        json!({"objective":objective,"search_queries":queries}),
    )
    .await?;
    let mut results = parse(common::decode(value))?;
    results.truncate(request.num_results);
    Ok(results)
}

fn parse(value: Value) -> Result<Vec<SearchResult>, String> {
    Ok(
        common::rows(&value, &["/results", "/search_results", ""], "Parallel")?
            .iter()
            .filter_map(|row| {
                common::result(
                    row,
                    "parallel",
                    &["title"],
                    &["url"],
                    &["excerpts", "snippet"],
                    &["publish_date"],
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structured_and_json_text() {
        let value = json!({"results":[{"url":"https://example.org","title":"Example","publish_date":"2025-06-01","excerpts":["First","Second"]}]});
        let results = parse(common::decode(json!(value.to_string()))).expect("results");
        assert_eq!(results[0].snippet, "First\nSecond");
        assert_eq!(results[0].published_date.as_deref(), Some("2025-06-01"));
        assert!(parse(json!({"error":"quota"})).is_err());
    }
}
