use super::super::{mcp, request::SearchCategory, SearchResult, WebSearchConfig, WebSearchRequest};
use super::common;
use serde_json::{json, Value};

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let mut args = json!({"query":common::query(request)?,"numResults":request.num_results.clamp(1,100),"type":"auto","enableHighlights":true,"highlightsMaxCharacters":600});
    if !request.allowed_domains.is_empty() {
        args["includeDomains"] = json!(request.allowed_domains);
    }
    if !request.blocked_domains.is_empty() {
        args["excludeDomains"] = json!(request.blocked_domains);
    }
    if let Some(freshness) = &request.freshness {
        args["startPublishedDate"] = json!(freshness.cutoff().to_string());
    }
    let category = match request.category {
        SearchCategory::News => Some("news"),
        SearchCategory::Papers => Some("publication"),
        SearchCategory::Code => Some("github"),
        _ => None,
    };
    if let Some(category) = category {
        args["category"] = json!(category);
    }
    let value = mcp::call(
        "https://mcp.exa.ai/mcp?tools=web_search_advanced_exa",
        common::headers("x-api-key", key, false)?,
        "web_search_advanced_exa",
        args,
    )
    .await;
    let value = match value {
        Ok(value) => value,
        Err(_) => mcp::call(
            "https://mcp.exa.ai/mcp",
            common::headers("x-api-key", key, false)?,
            "web_search_exa",
            json!({"query":common::query(request)?,"objective":common::query(request)?,"numResults":request.num_results}),
        ).await?,
    };
    parse(common::decode(value))
}

fn parse(value: Value) -> Result<Vec<SearchResult>, String> {
    if let Some(text) = value.as_str() {
        return parse_text(text);
    }
    Ok(common::rows(&value, &["/results", ""], "Exa")?
        .iter()
        .filter_map(|row| {
            common::result(
                row,
                "exa",
                &["title"],
                &["url"],
                &["highlights", "text", "summary"],
                &["publishedDate"],
            )
        })
        .collect())
}

fn parse_text(text: &str) -> Result<Vec<SearchResult>, String> {
    let mut results = Vec::new();
    let mut current = SearchResult {
        source: Some("exa".into()),
        sources: vec!["exa".into()],
        ..Default::default()
    };
    let mut content = false;
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("Title:") {
            if !current.url.is_empty() {
                results.push(current);
            }
            current = SearchResult {
                title: title.trim().into(),
                source: Some("exa".into()),
                sources: vec!["exa".into()],
                ..Default::default()
            };
            content = false;
        } else if let Some(url) = line.strip_prefix("URL:") {
            current.url = url.trim().into();
            content = false;
        } else if let Some(date) = line.strip_prefix("Published Date:") {
            current.published_date = common::normalize_date(date);
            content = false;
        } else if let Some(snippet) = line
            .strip_prefix("Highlights:")
            .or_else(|| line.strip_prefix("Text:"))
        {
            current.snippet = snippet.trim().into();
            content = true;
        } else if content && !line.trim().is_empty() && line.trim() != "---" {
            if !current.snippet.is_empty() {
                current.snippet.push('\n');
            }
            current.snippet.push_str(line.trim());
        }
    }
    if !current.url.is_empty() {
        results.push(current);
    }
    if results.is_empty()
        && !text.trim().is_empty()
        && !text.to_lowercase().contains("no results")
        && !text.to_lowercase().contains("no search results")
    {
        return Err("Invalid Exa response: missing result records".into());
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_records_and_json() {
        let results = parse(json!("Title: Rust\nURL: https://rust-lang.org\nPublished Date: 2025-01-01\nHighlights: Build reliable software\nMore content\n\nTitle: Docs\nURL: https://doc.rust-lang.org\nText: The book")).expect("text");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].snippet, "Build reliable software\nMore content");
        assert_eq!(results[0].published_date.as_deref(), Some("2025-01-01"));
        let results = parse(common::decode(json!(r#"{"results":[{"title":"Rust","url":"https://rust-lang.org","highlights":["First","Second"]}]}"#))).expect("json");
        assert_eq!(results[0].snippet, "First\nSecond");
        assert!(parse(json!("Error: rate limited")).is_err());
    }
}
