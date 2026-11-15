use super::super::{http, SearchResult, WebSearchConfig, WebSearchRequest};
use super::common;
use serde_json::{json, Value};

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let mut body = json!({"query":common::domain_query(common::query(request)?, request),"limit":request.num_results.clamp(1, 100)});
    if let Some(tbs) = common::qdr(request) {
        body["tbs"] = json!(tbs);
    }
    let response = http::json(
        http::client()?
            .post("https://api.firecrawl.dev/v2/search")
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
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        return Err("Firecrawl returned an API error".into());
    }
    Ok(common::rows(value, &["/data/web", "/data"], "Firecrawl")?
        .iter()
        .filter_map(|row| {
            common::result(
                row,
                "firecrawl",
                &["title"],
                &["url"],
                &["description", "snippet", "markdown"],
                &["publishedDate", "date"],
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_two_and_legacy() {
        let row = json!({"title":"Example","url":"https://example.org","description":"A page"});
        for value in [
            json!({"success":true,"data":{"web":[row.clone()]}}),
            json!({"success":true,"data":[row]}),
        ] {
            let results = parse(&value).expect("results");
            assert_eq!(results[0].snippet, "A page");
        }
        assert!(parse(&json!({"success":false})).is_err());
        assert!(parse(&json!({"data":{"web":[]}}))
            .expect("empty")
            .is_empty());
    }
}
