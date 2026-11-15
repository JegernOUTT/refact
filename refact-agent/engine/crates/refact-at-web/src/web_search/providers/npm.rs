use super::super::{WebSearchRequest, WebSearchConfig, SearchResult};
use super::super::http::{client, json};
use super::legacy_common::*;
use serde_json::Value;

const NPM_BACKEND_NAME: &str = "npm";

fn parse(value: &Value, num_results: usize) -> Result<Vec<SearchResult>, String> {
    let Some(objects) = value.get("objects").and_then(|v| v.as_array()) else {
        return Ok(vec![]);
    };

    let mut parsed = Vec::new();
    for item in objects.iter().take(num_results) {
        let Some(package) = item.get("package").and_then(|v| v.as_object()) else {
            continue;
        };
        let name = package
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let version = package
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let description = package
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let url = package
            .get("links")
            .and_then(|v| v.get("npm"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("https://www.npmjs.com/package/{}", name));
        parsed.push(SearchResult {
            title: if version.is_empty() {
                name.to_string()
            } else {
                format!("{} {}", name, version)
            },
            url,
            snippet: normalize_text(description),
            source: Some(NPM_BACKEND_NAME.to_string()),
            sources: vec![NPM_BACKEND_NAME.to_string()],
            ..Default::default()
        });
    }

    Ok(parsed)
}

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    _key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let builder = client()?
        .get("https://registry.npmjs.org/-/v1/search")
        .header("User-Agent", "refact-agent")
        .query(&[
            ("text", &clean_query_for_source(query(request)?)),
            ("size", &request.num_results.min(250).to_string()),
        ]);

    parse(&json(builder).await?, request.num_results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_parses_provider_results() {
        let value: Value = serde_json::from_str(r#"{"objects":[{"package":{"name":"@scope/pkg","version":"1.0","description":"Useful"}}]}"#).expect("fixture JSON");
        let results = parse(&value, 5).expect("fixture results");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://www.npmjs.com/package/@scope/pkg");
        assert_eq!(results[0].sources, vec!["npm"]);
        assert!(!results[0].title.is_empty());
    }
    #[test]
    fn empty_fixture_is_safe() {
        assert!(parse(&serde_json::json!({}), 5)
            .map(|items| items.is_empty())
            .unwrap_or(true));
    }
}
