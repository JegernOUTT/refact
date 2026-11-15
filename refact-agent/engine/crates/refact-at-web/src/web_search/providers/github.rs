use super::super::{WebSearchRequest, WebSearchConfig, SearchResult};
use super::super::http::{client, json};
use super::legacy_common::*;
use serde_json::Value;

fn parse_issues(value: &Value, num_results: usize) -> Result<Vec<SearchResult>, String> {
    let Some(items) = value.get("items").and_then(|v| v.as_array()) else {
        return Ok(vec![]);
    };

    let mut parsed = Vec::new();
    for item in items.iter().take(num_results) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let url = obj
            .get("html_url")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let body = obj.get("body").and_then(|v| v.as_str()).unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(title),
            url: url.to_string(),
            snippet: truncate_chars(body, 280),
            source: Some("github".to_string()),
            sources: vec!["github".to_string()],
            published_date: obj
                .get("created_at")
                .and_then(Value::as_str)
                .and_then(super::common::normalize_date),
            ..Default::default()
        });
    }

    Ok(parsed)
}

fn parse_repositories(value: &Value, num_results: usize) -> Result<Vec<SearchResult>, String> {
    let Some(items) = value.get("items").and_then(|v| v.as_array()) else {
        return Ok(vec![]);
    };

    let mut parsed = Vec::new();
    for item in items.iter().take(num_results) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let full_name = obj
            .get("full_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let url = obj
            .get("html_url")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let description = obj
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let language = obj.get("language").and_then(|v| v.as_str()).unwrap_or("");
        let stars = obj
            .get("stargazers_count")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        if full_name.is_empty() || url.is_empty() {
            continue;
        }
        let metadata = if language.is_empty() {
            format!("{} stars", stars)
        } else {
            format!("{} stars. Language: {}", stars, language)
        };
        let snippet = if description.is_empty() {
            metadata
        } else {
            format!("{}. {}", description, metadata)
        };
        parsed.push(SearchResult {
            title: full_name.to_string(),
            url: url.to_string(),
            snippet: normalize_text(&snippet),
            source: Some("github".to_string()),
            sources: vec!["github".to_string()],
            published_date: obj
                .get("created_at")
                .and_then(Value::as_str)
                .and_then(super::common::normalize_date),
            ..Default::default()
        });
    }

    Ok(parsed)
}

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let mut issue_query = clean_query_for_source(query(request)?);
    if let Some(repo) = github_repo_qualifier(query(request)?) {
        issue_query.push_str(&format!(" {repo}"));
    }
    let mut repo_query = clean_query_for_source(query(request)?);
    if let Some(freshness) = &request.freshness {
        let filter = format!(" created:>{}", freshness.cutoff());
        issue_query.push_str(&filter);
        repo_query.push_str(&filter);
    }
    issue_query.push_str(" is:issue");
    let issues = fetch("issues", &issue_query, request.num_results, key);
    let repos = fetch("repositories", &repo_query, request.num_results, key);
    let (issues, repos) = tokio::join!(issues, repos);
    match (issues, repos) {
        (Ok(issues), Ok(repos)) => {
            let mut combined = Vec::new();
            let mut issues = issues.into_iter();
            let mut repos = repos.into_iter();
            loop {
                let issue = issues.next();
                let repo = repos.next();
                if issue.is_none() && repo.is_none() {
                    break;
                }
                combined.extend(issue);
                combined.extend(repo);
            }
            Ok(combined.into_iter().take(request.num_results).collect())
        }
        (Ok(results), Err(_)) | (Err(_), Ok(results)) if !results.is_empty() => Ok(results),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

async fn fetch(
    kind: &str,
    query: &str,
    count: usize,
    key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let mut builder = client()?
        .get(format!("https://api.github.com/search/{kind}"))
        .header("User-Agent", "refact-agent")
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .query(&[
            ("q", query),
            ("per_page", &count.min(100).to_string()),
            ("sort", if kind == "issues" { "updated" } else { "stars" }),
            ("order", "desc"),
        ]);
    if let Some(key) = key.filter(|key| !key.trim().is_empty()) {
        builder = builder.bearer_auth(key);
    }
    let value = json(builder).await?;
    if kind == "issues" {
        parse_issues(&value, count)
    } else {
        parse_repositories(&value, count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn issue_and_repository_fixtures_share_provider_id() {
        let issue = parse_issues(&serde_json::json!({"items":[{"title":"A bug","html_url":"https://github.com/a/b/issues/1","body":"Details"}]}), 5).expect("issues");
        let repo = parse_repositories(&serde_json::json!({"items":[{"full_name":"a/b","html_url":"https://github.com/a/b","language":"Rust","stargazers_count":42}]}), 5).expect("repos");
        assert_eq!(issue[0].sources, vec!["github"]);
        assert_eq!(repo[0].sources, vec!["github"]);
        assert!(repo[0].snippet.contains("42 stars"));
        assert_eq!(
            github_repo_qualifier("topic site:github.com/a/b/issues"),
            Some("repo:a/b".to_string())
        );
    }
}
