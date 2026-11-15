use super::super::{http, request::SearchCategory, SearchResult, WebSearchConfig, WebSearchRequest};
use super::common;
use serde_json::{json, Value};

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let key = common::key(key)?;
    let news = matches!(request.category, SearchCategory::News);
    let endpoint = if news {
        "https://google.serper.dev/news"
    } else {
        "https://google.serper.dev/search"
    };
    let mut body = json!({"q": common::domain_query(common::query(request)?, request), "num": request.num_results.clamp(1, 100)});
    if let Some(tbs) = common::qdr(request) {
        body["tbs"] = json!(tbs);
    }
    let response = http::json(
        http::client()?
            .post(endpoint)
            .headers(common::headers("x-api-key", Some(key), false)?)
            .json(&body),
    )
    .await?;
    parse(&response, news)
}

fn parse(value: &Value, news: bool) -> Result<Vec<SearchResult>, String> {
    Ok(
        common::rows(value, &[if news { "/news" } else { "/organic" }], "Serper")?
            .iter()
            .filter_map(|row| {
                common::result(
                    row,
                    "serper",
                    &["title"],
                    &["link"],
                    &["snippet"],
                    &["date"],
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn organic_and_news() {
        let organic = parse(&json!({"organic":[{"title":"Rust","link":"https://rust-lang.org","snippet":"Build reliable software"},{"title":"Missing URL"}]}), false).expect("organic");
        assert_eq!(organic.len(), 1);
        assert_eq!(organic[0].snippet, "Build reliable software");
        let news = parse(&json!({"news":[{"title":"Release","link":"https://example.org/news","date":"2 hours ago"}]}), true).expect("news");
        assert!(chrono::NaiveDate::parse_from_str(
            news[0].published_date.as_deref().expect("normalized date"),
            "%Y-%m-%d"
        )
        .is_ok());
        assert!(parse(&json!({"error":"quota"}), false).is_err());
    }
}
