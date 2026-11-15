use super::super::{mcp, SearchResult, WebSearchConfig, WebSearchRequest};
use super::common;
use serde_json::{json, Value};

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let query = common::query(request)?;
    let headers = common::headers("authorization", key, true)?;
    let resolved = mcp::call(
        "https://mcp.context7.com/mcp",
        headers.clone(),
        "resolve-library-id",
        json!({"libraryName":query,"query":query}),
    )
    .await?;
    let library = library_id(common::decode(resolved))?;
    let docs = mcp::call(
        "https://mcp.context7.com/mcp",
        headers,
        "query-docs",
        json!({"libraryId":library,"query":query}),
    )
    .await?;
    parse(common::decode(docs), &library)
}

fn library_id(value: Value) -> Result<String, String> {
    if let Some(id) = value
        .get("id")
        .or_else(|| value.get("libraryId"))
        .and_then(Value::as_str)
        .filter(|id| id.starts_with('/'))
    {
        return Ok(id.into());
    }
    for pointer in ["/results", "/libraries", ""] {
        if let Some(rows) = value.pointer(pointer).and_then(Value::as_array) {
            if let Some(id) = rows.iter().find_map(|row| {
                row.get("id")
                    .or_else(|| row.get("libraryId"))
                    .and_then(Value::as_str)
                    .filter(|id| id.starts_with('/'))
            }) {
                return Ok(id.into());
            }
        }
    }
    if let Some(text) = value.as_str() {
        for line in text.lines() {
            if let Some((_, id)) = line
                .split_once("Context7-compatible library ID:")
                .or_else(|| line.split_once("Library ID:"))
            {
                let id = id.trim().trim_matches('`');
                if id.starts_with('/') && !id.contains(char::is_whitespace) {
                    return Ok(id.into());
                }
            }
        }
    }
    Err("Context7 could not resolve a library ID".into())
}

fn parse(value: Value, library: &str) -> Result<Vec<SearchResult>, String> {
    if let Some(rows) = value
        .get("results")
        .or_else(|| value.get("snippets"))
        .and_then(Value::as_array)
    {
        return Ok(rows
            .iter()
            .map(|row| {
                let url = common::text(row, &["url", "source"]);
                SearchResult {
                    title: common::text(row, &["title"]),
                    url: if url.is_empty() {
                        format!("https://context7.com{}", library)
                    } else {
                        url
                    },
                    snippet: common::text(row, &["content", "text", "snippet"]),
                    source: Some("context7".into()),
                    sources: vec!["context7".into()],
                    ..Default::default()
                }
            })
            .collect());
    }
    let text = value
        .as_str()
        .ok_or_else(|| "Invalid Context7 documentation response".to_owned())?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut results = Vec::new();
    for block in text.split("### ").filter(|block| !block.trim().is_empty()) {
        let source = block
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("Source:")
                    .or_else(|| line.trim().strip_prefix("SOURCE:"))
            })
            .map(str::trim)
            .filter(|source| source.starts_with("https://") || source.starts_with("http://"));
        results.push(SearchResult {
            title: block.lines().next().unwrap_or(library).trim().into(),
            url: source
                .map(str::to_owned)
                .unwrap_or_else(|| format!("https://context7.com{}", library)),
            snippet: block.trim().into(),
            source: Some("context7".into()),
            sources: vec!["context7".into()],
            ..Default::default()
        });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolve_and_documentation() {
        assert_eq!(library_id(json!("- Title: Next.js\n- Context7-compatible library ID: /vercel/next.js\n- Description: Framework")).expect("id"), "/vercel/next.js");
        assert_eq!(
            library_id(json!({"results":[{"id":"/rust-lang/rust"}]})).expect("id"),
            "/rust-lang/rust"
        );
        assert!(library_id(json!("No matching libraries")).is_err());
        let results = parse(json!("### Routing\nSource: https://nextjs.org/docs/routing\nUse routes\n### Cache\nCached docs"), "/vercel/next.js").expect("docs");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://nextjs.org/docs/routing");
        assert_eq!(results[1].url, "https://context7.com/vercel/next.js");
        let results = parse(json!({"snippets":[{"title":"Routing","content":"Use routes","source":"https://nextjs.org/docs"}]}), "/vercel/next.js").expect("structured");
        assert_eq!(results[0].snippet, "Use routes");
    }
}
