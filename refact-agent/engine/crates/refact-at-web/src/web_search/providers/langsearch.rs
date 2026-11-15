use super::super::{http, SearchResult, WebSearchConfig, WebSearchRequest};
use super::common;
use serde_json::{json, Value};

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let freshness = request
        .freshness
        .as_ref()
        .map(|freshness| match freshness.as_str() {
            "day" => "oneDay",
            "week" => "oneWeek",
            "month" => "oneMonth",
            "year" => "oneYear",
            _ => "noLimit",
        })
        .unwrap_or("noLimit");
    let body = json!({"query": common::domain_query(common::query(request)?, request), "count": request.num_results.clamp(1, 50), "freshness": freshness, "summary": true});
    let response = http::json(
        http::client()?
            .post("https://api.langsearch.com/v1/web-search")
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
    if value
        .get("code")
        .and_then(Value::as_i64)
        .is_some_and(|code| code != 200)
    {
        return Err("LangSearch returned an API error".to_owned());
    }
    Ok(
        common::rows(value, &["/data/webPages/value"], "LangSearch")?
            .iter()
            .filter_map(|row| {
                common::result(
                    row,
                    "langsearch",
                    &["name"],
                    &["url"],
                    &["summary", "text", "snippet"],
                    &["datePublished"],
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summaries_and_text() {
        let results = parse(&json!({"code":200,"data":{"webPages":{"value":[{"name":"LangSearch","url":"https://langsearch.com","snippet":"Short","summary":"Long","datePublished":"2025-01-01"},{"name":"Text","url":"https://example.org","text":"New response"}]}}})).expect("results");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].snippet, "Long");
        assert_eq!(results[1].snippet, "New response");
        assert!(parse(&json!({"code":429})).is_err());
        assert!(parse(&json!({"data":{}})).is_err());
    }
}
