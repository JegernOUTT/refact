use super::super::{http, SearchResult, WebSearchConfig, WebSearchRequest};
use super::common;
use serde_json::{json, Value};

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let mut body = json!({"query": common::domain_query(common::query(request)?, request), "max_results": request.num_results.clamp(1, 20)});
    let domains: Vec<String> = if request.allowed_domains.is_empty() {
        request
            .blocked_domains
            .iter()
            .map(|domain| format!("-{domain}"))
            .collect()
    } else {
        request.allowed_domains.clone()
    };
    if !domains.is_empty() {
        body["search_domain_filter"] = json!(domains.into_iter().take(20).collect::<Vec<_>>());
    }
    if let Some(freshness) = &request.freshness {
        body["search_recency_filter"] = json!(freshness.as_str());
    }
    let response = http::json(
        http::client()?
            .post("https://api.perplexity.ai/search")
            .headers(common::headers(
                "authorization",
                Some(common::key(key)?),
                true,
            )?)
            .json(&body),
    )
    .await?;
    parse(&response)
}

fn parse(value: &Value) -> Result<Vec<SearchResult>, String> {
    Ok(common::rows(value, &["/results"], "Perplexity")?
        .iter()
        .filter_map(|row| {
            common::result(
                row,
                "perplexity",
                &["title"],
                &["url"],
                &["snippet"],
                &["date"],
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_results() {
        let results = parse(&json!({"results":[{"title":"Rust","url":"https://rust-lang.org","snippet":"Systems programming","date":"2025-01-23","last_updated":"2025-09-25"},{"title":"Invalid"}]})).expect("results");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].published_date.as_deref(), Some("2025-01-23"));
        assert!(parse(&json!({"error":"unauthorized"})).is_err());
        assert!(parse(&json!({"results":[]})).expect("empty").is_empty());
    }
}
