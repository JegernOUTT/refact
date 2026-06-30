use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, USER_AGENT};
use reqwest::Client;
use select::document::Document;
use select::predicate::{Class, Name};
use serde_json::Value;
use tracing::{info, warn};
use url::Url;

pub use refact_core::chat_types::{SearchResult, format_search_results};

const DDG_HTML_URL: &str = "https://html.duckduckgo.com/html/";
const DDG_TIMEOUT_SECS: u64 = 10;
pub const DEFAULT_NUM_RESULTS: usize = 8;
pub const MAX_NUM_RESULTS: usize = 20;
const SEARXNG_BACKEND_NAME: &str = "searxng";
const DDG_BACKEND_NAME: &str = "duckduckgo";
const WIKIPEDIA_BACKEND_NAME: &str = "wikipedia";
const WIKIPEDIA_API_URL: &str = "https://en.wikipedia.org/w/api.php";
const KEENABLE_BACKEND_NAME: &str = "keenable";
const KEENABLE_SEARCH_API_URL: &str = "https://api.keenable.ai/v1/search";
const KEENABLE_MCP_URL: &str = "https://api.keenable.ai/mcp";
const TAVILY_BACKEND_NAME: &str = "tavily";
const TAVILY_SEARCH_API_URL: &str = "https://api.tavily.com/search";
const BRAVE_BACKEND_NAME: &str = "brave";
const BRAVE_WEB_SEARCH_API_URL: &str = "https://api.search.brave.com/res/v1/web/search";
const GITHUB_ISSUES_BACKEND_NAME: &str = "github_issues";
const GITHUB_REPOS_BACKEND_NAME: &str = "github_repositories";
const GITHUB_ISSUES_SEARCH_API_URL: &str = "https://api.github.com/search/issues";
const GITHUB_REPOS_SEARCH_API_URL: &str = "https://api.github.com/search/repositories";
const STACK_EXCHANGE_BACKEND_NAME: &str = "stack_overflow";
const STACK_EXCHANGE_SEARCH_API_URL: &str = "https://api.stackexchange.com/2.3/search/advanced";
const NPM_BACKEND_NAME: &str = "npm";
const NPM_SEARCH_API_URL: &str = "https://registry.npmjs.org/-/v1/search";
const OPENALEX_BACKEND_NAME: &str = "openalex";
const OPENALEX_WORKS_API_URL: &str = "https://api.openalex.org/works";
const CROSSREF_BACKEND_NAME: &str = "crossref";
const CROSSREF_WORKS_API_URL: &str = "https://api.crossref.org/works";
const HACKER_NEWS_BACKEND_NAME: &str = "hacker_news";
const HACKER_NEWS_SEARCH_API_URL: &str = "https://hn.algolia.com/api/v1/search";

const DEFAULT_SEARXNG_INSTANCES: &[&str] = &[
    "https://search.inetol.net/search",
    "https://priv.au/search",
    "https://searx.be/search",
];

const USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
];

#[derive(Debug, Clone, Copy)]
enum DdgRequestMethod {
    Post,
    Get,
}

impl DdgRequestMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Post => "POST",
            Self::Get => "GET",
        }
    }
}

#[derive(Debug, Clone)]
struct SearchBackendError {
    backend: String,
    detail: String,
}

impl SearchBackendError {
    fn new(backend: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            detail: detail.into(),
        }
    }
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_query(query: &str) -> String {
    normalize_text(query)
}

fn unescape_html_light(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn strip_html_tags(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    normalize_text(&unescape_html_light(&output))
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let text = normalize_text(text);
    if text.chars().count() <= max_chars {
        return text;
    }

    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn clean_query_for_source(query: &str) -> String {
    let query = query
        .split_whitespace()
        .filter(|part| !part.trim_matches('"').starts_with("site:"))
        .collect::<Vec<_>>()
        .join(" ");
    normalize_query(query.trim_matches('"'))
}

fn query_contains_any(query: &str, needles: &[&str]) -> bool {
    let lower = query.to_ascii_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

fn should_search_developer_sources(query: &str) -> bool {
    query_contains_any(
        query,
        &[
            "api",
            "bug",
            "cli",
            "codex",
            "crate",
            "error",
            "exception",
            "github",
            "javascript",
            "library",
            "npm",
            "package",
            "python",
            "repo",
            "rust",
            "sdk",
            "site:github.com",
            "site:stackoverflow.com",
            "stack overflow",
            "stackoverflow",
            "timeout",
            "typescript",
        ],
    )
}

fn should_search_package_sources(query: &str) -> bool {
    query_contains_any(
        query,
        &[
            "javascript",
            "node",
            "node.js",
            "npm",
            "package",
            "typescript",
        ],
    )
}

fn should_search_research_sources(query: &str) -> bool {
    query_contains_any(
        query,
        &[
            "citation",
            "crossref",
            "doi",
            "journal",
            "openalex",
            "paper",
            "publication",
            "research",
            "study",
        ],
    )
}

fn should_search_hacker_news(query: &str) -> bool {
    should_search_developer_sources(query)
        || query_contains_any(query, &["hacker news", "hn.algolia", "news.ycombinator"])
}

fn github_repo_qualifier(query: &str) -> Option<String> {
    for raw_part in query.split_whitespace() {
        let part = raw_part.trim_matches('"');
        let Some(path) = part.strip_prefix("site:github.com/") else {
            continue;
        };
        let mut segments = path.split('/').filter(|segment| !segment.is_empty());
        let owner = segments.next()?;
        let repo = segments.next()?;
        return Some(format!("repo:{}/{}", owner, repo));
    }
    None
}

pub fn clamp_num_results(num_results: usize) -> usize {
    num_results.clamp(1, MAX_NUM_RESULTS)
}

fn normalize_title_key(title: &str) -> String {
    title
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_snippet_key(snippet: &str) -> String {
    normalize_title_key(snippet)
}

fn canonicalize_result_url(url: &str) -> String {
    let cleaned = clean_ddg_url(url);
    let Ok(mut parsed) = Url::parse(&cleaned) else {
        return cleaned.trim_end_matches('/').to_string();
    };

    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_fragment(None);

    let should_drop_port = matches!(parsed.scheme(), "http" | "https")
        && ((parsed.scheme() == "http" && parsed.port() == Some(80))
            || (parsed.scheme() == "https" && parsed.port() == Some(443)));
    if should_drop_port {
        let _ = parsed.set_port(None);
    }

    let normalized_path = parsed.path().trim_end_matches('/').to_string();
    parsed.set_path(if normalized_path.is_empty() {
        "/"
    } else {
        &normalized_path
    });

    parsed.to_string().trim_end_matches('/').to_string()
}

fn deduplicate_search_results(results: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut deduped: Vec<SearchResult> = Vec::new();
    let mut by_url: HashMap<String, usize> = HashMap::new();
    let mut by_title_snippet: HashMap<(String, String), usize> = HashMap::new();

    for mut result in results {
        result.title = normalize_text(&result.title);
        result.url = canonicalize_result_url(&result.url);
        result.snippet = normalize_text(&result.snippet);

        if result.title.is_empty() || result.url.is_empty() {
            continue;
        }

        let url_key = result.url.clone();
        if let Some(existing_idx) = by_url.get(&url_key).copied() {
            let existing = &mut deduped[existing_idx];
            if existing.snippet.is_empty() && !result.snippet.is_empty() {
                existing.snippet = result.snippet.clone();
            }
            if existing.source.is_none() {
                existing.source = result.source.clone();
            }
            continue;
        }

        let title_key = normalize_title_key(&result.title);
        let snippet_key = normalize_snippet_key(&result.snippet);
        let dedup_key = (title_key, snippet_key);
        if let Some(existing_idx) = by_title_snippet.get(&dedup_key).copied() {
            let existing = &mut deduped[existing_idx];
            if existing.snippet.is_empty() && !result.snippet.is_empty() {
                existing.snippet = result.snippet.clone();
            }
            if existing.source.is_none() {
                existing.source = result.source.clone();
            }
            continue;
        }

        let index = deduped.len();
        by_url.insert(url_key, index);
        by_title_snippet.insert(dedup_key, index);
        deduped.push(result);
    }

    deduped
}

fn contains_ddg_block_page(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("captcha")
        || lower.contains("verify you're human")
        || lower.contains("verify you are human")
        || lower.contains("automated requests")
        || lower.contains("unusual traffic")
        || lower.contains("please try again") && body.len() < 4000
        || lower.contains("bot") && body.len() < 4000
}

fn contains_searxng_block_page(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("too many requests")
        || lower.contains("rate limit")
        || lower.contains("sorry") && lower.contains("bot")
}

fn build_http_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(DDG_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

fn push_result(
    results: &mut Vec<SearchResult>,
    seen_urls: &mut HashSet<String>,
    title: String,
    url: String,
    snippet: String,
    source: &str,
) {
    let title = normalize_text(&title);
    let url = clean_ddg_url(&url);
    let url_key = canonicalize_result_url(&url);

    if title.is_empty() || url_key.is_empty() || !seen_urls.insert(url_key) {
        return;
    }

    results.push(SearchResult {
        title,
        url,
        snippet: normalize_text(&snippet),
        source: Some(source.to_string()),
    });
}

pub fn parse_ddg_html(html: &str) -> Vec<SearchResult> {
    let document = Document::from(html);
    let mut results = Vec::new();
    let mut seen_urls = HashSet::new();

    for node in document.find(Class("result__body")) {
        let anchor = node
            .find(Class("result__a"))
            .next()
            .or_else(|| node.find(Name("a")).next());

        let title = anchor.as_ref().map(|n| n.text()).unwrap_or_default();
        let url = anchor
            .as_ref()
            .and_then(|n| n.attr("href"))
            .unwrap_or_default()
            .to_string();
        let snippet = node
            .find(Class("result__snippet"))
            .next()
            .map(|n| n.text())
            .unwrap_or_default();

        push_result(
            &mut results,
            &mut seen_urls,
            title,
            url,
            snippet,
            DDG_BACKEND_NAME,
        );
    }

    for node in document.find(Class("results_links")) {
        let anchor = node
            .find(Class("result__a"))
            .next()
            .or_else(|| node.find(Name("a")).next());

        let title = anchor.as_ref().map(|n| n.text()).unwrap_or_default();
        let url = anchor
            .as_ref()
            .and_then(|n| n.attr("href"))
            .unwrap_or_default()
            .to_string();
        let snippet = node
            .find(Class("result__snippet"))
            .next()
            .or_else(|| node.find(Name("td")).last())
            .map(|n| n.text())
            .unwrap_or_default();

        push_result(
            &mut results,
            &mut seen_urls,
            title,
            url,
            snippet,
            DDG_BACKEND_NAME,
        );
    }

    for node in document.find(Class("result")) {
        let anchor = node
            .find(Class("result__a"))
            .next()
            .or_else(|| node.find(Name("a")).next());

        let title = anchor.as_ref().map(|n| n.text()).unwrap_or_default();
        let url = anchor
            .as_ref()
            .and_then(|n| n.attr("href"))
            .unwrap_or_default()
            .to_string();
        let snippet = node
            .find(Class("result__snippet"))
            .next()
            .map(|n| n.text())
            .unwrap_or_default();

        push_result(
            &mut results,
            &mut seen_urls,
            title,
            url,
            snippet,
            DDG_BACKEND_NAME,
        );
    }

    results
}

pub fn clean_ddg_url(href: &str) -> String {
    let trimmed = href.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let normalized = if trimmed.starts_with("//") {
        format!("https:{}", trimmed)
    } else if trimmed.starts_with('/') {
        format!("https://duckduckgo.com{}", trimmed)
    } else {
        trimmed.to_string()
    };

    if let Ok(parsed) = Url::parse(&normalized) {
        if let Some(domain) = parsed.domain() {
            if domain.ends_with("duckduckgo.com") {
                if let Some(decoded) = parsed
                    .query_pairs()
                    .find_map(|(key, value)| (key == "uddg").then(|| value.into_owned()))
                {
                    return decoded.trim().to_string();
                }
            }
        }

        if matches!(parsed.scheme(), "http" | "https") {
            return parsed.to_string();
        }
    }

    if let Some(encoded) = trimmed
        .split("uddg=")
        .nth(1)
        .and_then(|rest| rest.split('&').next())
    {
        return percent_encoding::percent_decode_str(encoded)
            .decode_utf8_lossy()
            .trim()
            .to_string();
    }

    normalized
}

fn parse_searxng_json(body: &str) -> Result<Vec<SearchResult>, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse SearxNG JSON response: {}", e))?;
    let Some(results) = value.get("results").and_then(|v| v.as_array()) else {
        return Err("SearxNG response did not contain a `results` array".to_string());
    };

    let mut parsed = Vec::new();
    for item in results {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let snippet = obj
            .get("content")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("snippet").and_then(|v| v.as_str()))
            .unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(title),
            url: url.to_string(),
            snippet: normalize_text(snippet),
            source: Some(SEARXNG_BACKEND_NAME.to_string()),
        });
    }

    Ok(parsed)
}

fn parse_keenable_json(body: &str) -> Result<Vec<SearchResult>, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse Keenable JSON response: {}", e))?;
    let Some(results) = value.get("results").and_then(|v| v.as_array()) else {
        return Err("Keenable response did not contain a `results` array".to_string());
    };

    let mut parsed = Vec::new();
    for item in results {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let snippet = obj
            .get("snippet")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("description").and_then(|v| v.as_str()))
            .unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(title),
            url: url.to_string(),
            snippet: normalize_text(snippet),
            source: Some(KEENABLE_BACKEND_NAME.to_string()),
        });
    }

    Ok(parsed)
}

fn parse_keenable_mcp_text(text: &str) -> Vec<SearchResult> {
    let mut parsed = Vec::new();
    for section in text.split("\n---\n") {
        let mut title = String::new();
        let mut url = String::new();
        let mut snippet_lines = Vec::new();
        let mut after_url = false;

        for line in section.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix("Title:") {
                title = normalize_text(rest);
                continue;
            }
            if let Some(rest) = line.strip_prefix("URL:") {
                url = rest.trim().to_string();
                after_url = true;
                continue;
            }
            if line.is_empty() || line.starts_with("Published:") || line.starts_with("Acquired:") {
                continue;
            }
            if after_url {
                snippet_lines.push(line);
            }
        }

        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title,
            url,
            snippet: truncate_chars(&snippet_lines.join(" "), 500),
            source: Some(KEENABLE_BACKEND_NAME.to_string()),
        });
    }

    parsed
}

fn parse_keenable_mcp_json(body: &str) -> Result<Vec<SearchResult>, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse Keenable MCP JSON response: {}", e))?;
    if let Some(error) = value.get("error") {
        let detail =
            json_detail(error).unwrap_or_else(|| "Keenable MCP returned an error".to_string());
        return Err(detail.to_string());
    }
    let Some(content) = value
        .get("result")
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_array())
    else {
        return Ok(vec![]);
    };

    let mut parsed = Vec::new();
    for item in content {
        let Some(text) = item.get("text").and_then(|v| v.as_str()) else {
            continue;
        };
        parsed.extend(parse_keenable_mcp_text(text));
    }

    Ok(parsed)
}

fn json_detail(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        Value::Null => None,
        Value::Array(items) if items.is_empty() => None,
        Value::Object(map) if map.is_empty() => None,
        _ => Some(value.to_string()),
    }
    .map(|text| truncate_chars(&text, 500))
}

fn provider_error_detail_from_json(value: &Value) -> Option<String> {
    value
        .get("error")
        .and_then(|error| {
            error
                .get("message")
                .and_then(json_detail)
                .or_else(|| error.get("detail").and_then(json_detail))
                .or_else(|| error.get("details").and_then(json_detail))
        })
        .or_else(|| value.get("message").and_then(json_detail))
        .or_else(|| value.get("detail").and_then(json_detail))
        .or_else(|| value.get("details").and_then(json_detail))
        .or_else(|| value.get("error").and_then(json_detail))
}

fn keenable_error_detail(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| provider_error_detail_from_json(&value))
        .unwrap_or_else(|| truncate_chars(body.trim(), 500))
}

fn keenable_api_key() -> Option<String> {
    env::var("KEENABLE_API_KEY")
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

fn parse_tavily_json(body: &str) -> Result<Vec<SearchResult>, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse Tavily JSON response: {}", e))?;
    if let Some(error) = value.get("error") {
        let detail = error
            .get("message")
            .and_then(|v| v.as_str())
            .or_else(|| error.as_str())
            .unwrap_or("Tavily returned an error");
        return Err(detail.to_string());
    }
    let Some(results) = value.get("results").and_then(|v| v.as_array()) else {
        return Err("Tavily response did not contain a `results` array".to_string());
    };

    let mut parsed = Vec::new();
    for item in results {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let snippet = obj
            .get("content")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("raw_content").and_then(|v| v.as_str()))
            .unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(title),
            url: url.to_string(),
            snippet: normalize_text(snippet),
            source: Some(TAVILY_BACKEND_NAME.to_string()),
        });
    }

    Ok(parsed)
}

fn tavily_error_detail(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| provider_error_detail_from_json(&value))
        .unwrap_or_else(|| truncate_chars(body.trim(), 500))
}

fn tavily_api_key() -> Option<String> {
    env::var("TAVILY_API_KEY")
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

async fn fetch_searxng_instance(
    client: &Client,
    instance_url: &str,
    query: &str,
    num_results: usize,
    user_agent: &str,
) -> Result<Vec<SearchResult>, String> {
    let mut url = Url::parse(instance_url)
        .map_err(|e| format!("Invalid SearxNG URL '{}': {}", instance_url, e))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("q", query);
        qp.append_pair("format", "json");
        qp.append_pair("language", "en-US");
        qp.append_pair("safesearch", "0");
        qp.append_pair("categories", "general");
        qp.append_pair("pageno", "1");
    }

    let response = client
        .get(url)
        .header("User-Agent", user_agent)
        .header("Accept", "application/json,text/html;q=0.9,*/*;q=0.8")
        .send()
        .await
        .map_err(|e| format!("SearxNG request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("SearxNG returned status: {}", response.status()));
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_default();

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read SearxNG response body: {}", e))?;

    if contains_searxng_block_page(&body) {
        return Err("SearxNG returned a rate-limit or bot-detection page".to_string());
    }

    if !content_type.contains("json") {
        return Err(format!(
            "SearxNG instance did not return JSON (content-type: {})",
            if content_type.is_empty() {
                "unknown"
            } else {
                &content_type
            }
        ));
    }

    let parsed = parse_searxng_json(&body)?;
    Ok(parsed.into_iter().take(num_results).collect())
}

async fn search_searxng(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    let mut errors = Vec::new();

    for (idx, instance) in DEFAULT_SEARXNG_INSTANCES.iter().enumerate() {
        let user_agent = USER_AGENTS[idx % USER_AGENTS.len()];
        match fetch_searxng_instance(client, instance, query, num_results, user_agent).await {
            Ok(results) if !results.is_empty() => return Ok(results),
            Ok(_) => errors.push(format!("{} returned 0 results", instance)),
            Err(err) => errors.push(format!("{}: {}", instance, err)),
        }
    }

    Err(SearchBackendError::new(
        SEARXNG_BACKEND_NAME,
        errors.join(" | "),
    ))
}

async fn search_wikipedia(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    let mut url = Url::parse(WIKIPEDIA_API_URL)
        .map_err(|e| SearchBackendError::new(WIKIPEDIA_BACKEND_NAME, e.to_string()))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("action", "query");
        qp.append_pair("list", "search");
        qp.append_pair("srsearch", query);
        qp.append_pair("srlimit", &num_results.to_string());
        qp.append_pair("utf8", "1");
        qp.append_pair("format", "json");
        qp.append_pair("formatversion", "2");
    }

    let response = client
        .get(url)
        .header("User-Agent", USER_AGENTS[0])
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| SearchBackendError::new(WIKIPEDIA_BACKEND_NAME, e.to_string()))?;

    if !response.status().is_success() {
        return Err(SearchBackendError::new(
            WIKIPEDIA_BACKEND_NAME,
            format!("Wikipedia returned status: {}", response.status()),
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(WIKIPEDIA_BACKEND_NAME, e.to_string()))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|e| SearchBackendError::new(WIKIPEDIA_BACKEND_NAME, e.to_string()))?;

    let Some(results) = value
        .get("query")
        .and_then(|v| v.get("search"))
        .and_then(|v| v.as_array())
    else {
        return Err(SearchBackendError::new(
            WIKIPEDIA_BACKEND_NAME,
            "Wikipedia response did not contain query.search",
        ));
    };

    let mut parsed = Vec::new();
    for item in results.iter().take(num_results) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let page_id = obj
            .get("pageid")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let snippet = obj
            .get("snippet")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .replace("<span class=\"searchmatch\">", "")
            .replace("</span>", "");
        if title.is_empty() || page_id == 0 {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(title),
            url: format!("https://en.wikipedia.org/?curid={}", page_id),
            snippet: normalize_text(&snippet),
            source: Some(WIKIPEDIA_BACKEND_NAME.to_string()),
        });
    }

    Ok(parsed)
}

async fn search_keenable_rest(
    client: &Client,
    query: &str,
    num_results: usize,
    api_key: &str,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    let url = Url::parse(KEENABLE_SEARCH_API_URL)
        .map_err(|e| SearchBackendError::new(KEENABLE_BACKEND_NAME, e.to_string()))?;
    let response = client
        .post(url)
        .header("X-API-Key", api_key)
        .header("Accept", "application/json")
        .json(&serde_json::json!({ "query": query }))
        .send()
        .await
        .map_err(|e| SearchBackendError::new(KEENABLE_BACKEND_NAME, e.to_string()))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let detail = keenable_error_detail(&body);
        return Err(SearchBackendError::new(
            KEENABLE_BACKEND_NAME,
            if detail.is_empty() {
                format!("Keenable returned status: {}", status)
            } else {
                format!("Keenable returned status: {} ({})", status, detail)
            },
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(KEENABLE_BACKEND_NAME, e.to_string()))?;
    let parsed = parse_keenable_json(&body)
        .map_err(|e| SearchBackendError::new(KEENABLE_BACKEND_NAME, e))?;
    Ok(parsed.into_iter().take(num_results).collect())
}

async fn search_keenable_mcp(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    let url = Url::parse(KEENABLE_MCP_URL)
        .map_err(|e| SearchBackendError::new(KEENABLE_BACKEND_NAME, e.to_string()))?;
    let initialize = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "refact-agent", "version": "0.0.0"}
        }
    });
    let init_response = client
        .post(url.clone())
        .header("Accept", "application/json, text/event-stream")
        .header(CONTENT_TYPE, "application/json")
        .json(&initialize)
        .send()
        .await
        .map_err(|e| SearchBackendError::new(KEENABLE_BACKEND_NAME, e.to_string()))?;

    if !init_response.status().is_success() {
        let status = init_response.status();
        let body = init_response.text().await.unwrap_or_default();
        let detail = keenable_error_detail(&body);
        return Err(SearchBackendError::new(
            KEENABLE_BACKEND_NAME,
            if detail.is_empty() {
                format!("Keenable MCP initialize returned status: {}", status)
            } else {
                format!(
                    "Keenable MCP initialize returned status: {} ({})",
                    status, detail
                )
            },
        ));
    }

    let session_id = init_response
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| {
            SearchBackendError::new(
                KEENABLE_BACKEND_NAME,
                "Keenable MCP did not return a session id",
            )
        })?;
    let init_body = init_response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(KEENABLE_BACKEND_NAME, e.to_string()))?;
    if let Ok(value) = serde_json::from_str::<Value>(&init_body) {
        if let Some(error) = value.get("error") {
            let detail = json_detail(error)
                .unwrap_or_else(|| "Keenable MCP initialize returned an error".to_string());
            return Err(SearchBackendError::new(KEENABLE_BACKEND_NAME, detail));
        }
    }

    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    });
    let initialized_response = client
        .post(url.clone())
        .header("Accept", "application/json, text/event-stream")
        .header(CONTENT_TYPE, "application/json")
        .header("mcp-session-id", session_id.as_str())
        .json(&initialized)
        .send()
        .await
        .map_err(|e| SearchBackendError::new(KEENABLE_BACKEND_NAME, e.to_string()))?;
    if !initialized_response.status().is_success() {
        let status = initialized_response.status();
        let body = initialized_response.text().await.unwrap_or_default();
        let detail = keenable_error_detail(&body);
        return Err(SearchBackendError::new(
            KEENABLE_BACKEND_NAME,
            if detail.is_empty() {
                format!(
                    "Keenable MCP initialized notification returned status: {}",
                    status
                )
            } else {
                format!(
                    "Keenable MCP initialized notification returned status: {} ({})",
                    status, detail
                )
            },
        ));
    }

    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "search_web_pages",
            "arguments": {"query": query}
        }
    });
    let call_response = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header(CONTENT_TYPE, "application/json")
        .header("mcp-session-id", session_id.as_str())
        .json(&call)
        .send()
        .await
        .map_err(|e| SearchBackendError::new(KEENABLE_BACKEND_NAME, e.to_string()))?;

    if !call_response.status().is_success() {
        let status = call_response.status();
        let body = call_response.text().await.unwrap_or_default();
        let detail = keenable_error_detail(&body);
        return Err(SearchBackendError::new(
            KEENABLE_BACKEND_NAME,
            if detail.is_empty() {
                format!("Keenable MCP search returned status: {}", status)
            } else {
                format!(
                    "Keenable MCP search returned status: {} ({})",
                    status, detail
                )
            },
        ));
    }

    let body = call_response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(KEENABLE_BACKEND_NAME, e.to_string()))?;
    let parsed = parse_keenable_mcp_json(&body)
        .map_err(|e| SearchBackendError::new(KEENABLE_BACKEND_NAME, e))?;
    Ok(parsed.into_iter().take(num_results).collect())
}

async fn search_keenable(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    if let Some(api_key) = keenable_api_key() {
        search_keenable_rest(client, query, num_results, &api_key).await
    } else {
        search_keenable_mcp(client, query, num_results).await
    }
}

async fn search_tavily(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    let url = Url::parse(TAVILY_SEARCH_API_URL)
        .map_err(|e| SearchBackendError::new(TAVILY_BACKEND_NAME, e.to_string()))?;
    let body = serde_json::json!({
        "query": query,
        "max_results": num_results.min(20),
        "search_depth": "basic",
        "include_answer": false,
        "include_raw_content": false,
    });
    let mut request = client
        .post(url)
        .header("Accept", "application/json")
        .header(CONTENT_TYPE, "application/json")
        .json(&body);
    if let Some(api_key) = tavily_api_key() {
        request = request.header("Authorization", format!("Bearer {}", api_key));
    } else {
        request = request.header("X-Tavily-Access-Mode", "keyless");
    }

    let response = request
        .send()
        .await
        .map_err(|e| SearchBackendError::new(TAVILY_BACKEND_NAME, e.to_string()))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let detail = tavily_error_detail(&body);
        return Err(SearchBackendError::new(
            TAVILY_BACKEND_NAME,
            if detail.is_empty() {
                format!("Tavily returned status: {}", status)
            } else {
                format!("Tavily returned status: {} ({})", status, detail)
            },
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(TAVILY_BACKEND_NAME, e.to_string()))?;
    let parsed =
        parse_tavily_json(&body).map_err(|e| SearchBackendError::new(TAVILY_BACKEND_NAME, e))?;
    Ok(parsed.into_iter().take(num_results).collect())
}

async fn search_brave(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    let Ok(api_key) = env::var("BRAVE_SEARCH_API_KEY") else {
        return Ok(vec![]);
    };
    if api_key.trim().is_empty() {
        return Ok(vec![]);
    }

    let mut url = Url::parse(BRAVE_WEB_SEARCH_API_URL)
        .map_err(|e| SearchBackendError::new(BRAVE_BACKEND_NAME, e.to_string()))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("q", query);
        qp.append_pair("count", &num_results.min(20).to_string());
        qp.append_pair("search_lang", "en");
        qp.append_pair("spellcheck", "1");
    }

    let response = client
        .get(url)
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| SearchBackendError::new(BRAVE_BACKEND_NAME, e.to_string()))?;

    if !response.status().is_success() {
        return Err(SearchBackendError::new(
            BRAVE_BACKEND_NAME,
            format!("Brave returned status: {}", response.status()),
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(BRAVE_BACKEND_NAME, e.to_string()))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|e| SearchBackendError::new(BRAVE_BACKEND_NAME, e.to_string()))?;
    let Some(results) = value
        .get("web")
        .and_then(|v| v.get("results"))
        .and_then(|v| v.as_array())
    else {
        return Ok(vec![]);
    };

    let mut parsed = Vec::new();
    for item in results.iter().take(num_results) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let snippet = obj
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(&unescape_html_light(title)),
            url: url.to_string(),
            snippet: strip_html_tags(snippet),
            source: Some(BRAVE_BACKEND_NAME.to_string()),
        });
    }

    Ok(parsed)
}

fn github_request_builder(client: &Client, url: Url) -> reqwest::RequestBuilder {
    let builder = client
        .get(url)
        .header(USER_AGENT, "refact-agent")
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");

    match env::var("GITHUB_TOKEN") {
        Ok(token) if !token.trim().is_empty() => {
            builder.header("Authorization", format!("Bearer {}", token.trim()))
        }
        _ => builder,
    }
}

async fn search_github_issues(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    if !should_search_developer_sources(query) {
        return Ok(vec![]);
    }

    let source_query = clean_query_for_source(query);
    if source_query.is_empty() {
        return Ok(vec![]);
    }
    let mut gh_query = source_query;
    if let Some(repo) = github_repo_qualifier(query) {
        gh_query.push(' ');
        gh_query.push_str(&repo);
    }
    gh_query.push_str(" is:issue");

    let mut url = Url::parse(GITHUB_ISSUES_SEARCH_API_URL)
        .map_err(|e| SearchBackendError::new(GITHUB_ISSUES_BACKEND_NAME, e.to_string()))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("q", &gh_query);
        qp.append_pair("sort", "updated");
        qp.append_pair("order", "desc");
        qp.append_pair("per_page", &num_results.min(10).to_string());
    }

    let response = github_request_builder(client, url)
        .send()
        .await
        .map_err(|e| SearchBackendError::new(GITHUB_ISSUES_BACKEND_NAME, e.to_string()))?;
    if !response.status().is_success() {
        return Err(SearchBackendError::new(
            GITHUB_ISSUES_BACKEND_NAME,
            format!(
                "GitHub issues search returned status: {}",
                response.status()
            ),
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(GITHUB_ISSUES_BACKEND_NAME, e.to_string()))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|e| SearchBackendError::new(GITHUB_ISSUES_BACKEND_NAME, e.to_string()))?;
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
            source: Some(GITHUB_ISSUES_BACKEND_NAME.to_string()),
        });
    }

    Ok(parsed)
}

async fn search_github_repositories(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    if !should_search_developer_sources(query) || github_repo_qualifier(query).is_some() {
        return Ok(vec![]);
    }

    let source_query = clean_query_for_source(query);
    if source_query.is_empty() {
        return Ok(vec![]);
    }

    let mut url = Url::parse(GITHUB_REPOS_SEARCH_API_URL)
        .map_err(|e| SearchBackendError::new(GITHUB_REPOS_BACKEND_NAME, e.to_string()))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("q", &source_query);
        qp.append_pair("sort", "stars");
        qp.append_pair("order", "desc");
        qp.append_pair("per_page", &num_results.min(10).to_string());
    }

    let response = github_request_builder(client, url)
        .send()
        .await
        .map_err(|e| SearchBackendError::new(GITHUB_REPOS_BACKEND_NAME, e.to_string()))?;
    if !response.status().is_success() {
        return Err(SearchBackendError::new(
            GITHUB_REPOS_BACKEND_NAME,
            format!(
                "GitHub repository search returned status: {}",
                response.status()
            ),
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(GITHUB_REPOS_BACKEND_NAME, e.to_string()))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|e| SearchBackendError::new(GITHUB_REPOS_BACKEND_NAME, e.to_string()))?;
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
            source: Some(GITHUB_REPOS_BACKEND_NAME.to_string()),
        });
    }

    Ok(parsed)
}

async fn search_stack_overflow(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    if !should_search_developer_sources(query) {
        return Ok(vec![]);
    }

    let source_query = clean_query_for_source(query);
    if source_query.is_empty() {
        return Ok(vec![]);
    }

    let mut url = Url::parse(STACK_EXCHANGE_SEARCH_API_URL)
        .map_err(|e| SearchBackendError::new(STACK_EXCHANGE_BACKEND_NAME, e.to_string()))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("order", "desc");
        qp.append_pair("sort", "relevance");
        qp.append_pair("site", "stackoverflow");
        qp.append_pair("q", &source_query);
        qp.append_pair("pagesize", &num_results.min(10).to_string());
    }

    let response = client
        .get(url)
        .header(USER_AGENT, "refact-agent")
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| SearchBackendError::new(STACK_EXCHANGE_BACKEND_NAME, e.to_string()))?;
    if !response.status().is_success() {
        return Err(SearchBackendError::new(
            STACK_EXCHANGE_BACKEND_NAME,
            format!("Stack Exchange returned status: {}", response.status()),
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(STACK_EXCHANGE_BACKEND_NAME, e.to_string()))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|e| SearchBackendError::new(STACK_EXCHANGE_BACKEND_NAME, e.to_string()))?;
    if let Some(backoff) = value.get("backoff").and_then(|v| v.as_i64()) {
        warn!("Stack Exchange requested backoff for {} seconds", backoff);
    }
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
        let url = obj.get("link").and_then(|v| v.as_str()).unwrap_or_default();
        let score = obj
            .get("score")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let answers = obj
            .get("answer_count")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let accepted = obj
            .get("is_answered")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if title.is_empty() || url.is_empty() {
            continue;
        }
        parsed.push(SearchResult {
            title: normalize_text(&unescape_html_light(title)),
            url: url.to_string(),
            snippet: format!(
                "Score: {}. Answers: {}. Accepted answer: {}.",
                score,
                answers,
                if accepted { "yes" } else { "no" }
            ),
            source: Some(STACK_EXCHANGE_BACKEND_NAME.to_string()),
        });
    }

    Ok(parsed)
}

async fn search_npm(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    if !should_search_package_sources(query) {
        return Ok(vec![]);
    }

    let source_query = clean_query_for_source(query);
    if source_query.is_empty() {
        return Ok(vec![]);
    }

    let mut url = Url::parse(NPM_SEARCH_API_URL)
        .map_err(|e| SearchBackendError::new(NPM_BACKEND_NAME, e.to_string()))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("text", &source_query);
        qp.append_pair("size", &num_results.min(10).to_string());
    }

    let response = client
        .get(url)
        .header(USER_AGENT, "refact-agent")
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| SearchBackendError::new(NPM_BACKEND_NAME, e.to_string()))?;
    if !response.status().is_success() {
        return Err(SearchBackendError::new(
            NPM_BACKEND_NAME,
            format!("npm returned status: {}", response.status()),
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(NPM_BACKEND_NAME, e.to_string()))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|e| SearchBackendError::new(NPM_BACKEND_NAME, e.to_string()))?;
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
        });
    }

    Ok(parsed)
}

fn openalex_abstract_from_inverted_index(value: &Value) -> String {
    let Some(index) = value.as_object() else {
        return String::new();
    };

    let mut words_by_position: BTreeMap<usize, &str> = BTreeMap::new();
    for (word, positions) in index {
        let Some(positions) = positions.as_array() else {
            continue;
        };
        for position in positions {
            if let Some(position) = position.as_u64() {
                words_by_position.insert(position as usize, word);
            }
        }
    }

    words_by_position
        .values()
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

async fn search_openalex(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    if !should_search_research_sources(query) {
        return Ok(vec![]);
    }

    let source_query = clean_query_for_source(query);
    if source_query.is_empty() {
        return Ok(vec![]);
    }

    let mut url = Url::parse(OPENALEX_WORKS_API_URL)
        .map_err(|e| SearchBackendError::new(OPENALEX_BACKEND_NAME, e.to_string()))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("search", &source_query);
        qp.append_pair("per-page", &num_results.min(10).to_string());
    }

    let response = client
        .get(url)
        .header(USER_AGENT, "refact-agent")
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| SearchBackendError::new(OPENALEX_BACKEND_NAME, e.to_string()))?;
    if !response.status().is_success() {
        return Err(SearchBackendError::new(
            OPENALEX_BACKEND_NAME,
            format!("OpenAlex returned status: {}", response.status()),
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(OPENALEX_BACKEND_NAME, e.to_string()))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|e| SearchBackendError::new(OPENALEX_BACKEND_NAME, e.to_string()))?;
    let Some(results) = value.get("results").and_then(|v| v.as_array()) else {
        return Ok(vec![]);
    };

    let mut parsed = Vec::new();
    for item in results.iter().take(num_results) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("display_name").and_then(|v| v.as_str()))
            .unwrap_or_default();
        let url = obj
            .get("doi")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("id").and_then(|v| v.as_str()))
            .unwrap_or_default();
        let year = obj
            .get("publication_year")
            .and_then(|v| v.as_i64())
            .map(|v| v.to_string())
            .unwrap_or_default();
        let abstract_text = obj
            .get("abstract_inverted_index")
            .map(openalex_abstract_from_inverted_index)
            .unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let snippet = if abstract_text.is_empty() {
            year
        } else if year.is_empty() {
            truncate_chars(&abstract_text, 280)
        } else {
            format!("{}. {}", year, truncate_chars(&abstract_text, 260))
        };
        parsed.push(SearchResult {
            title: normalize_text(title),
            url: url.to_string(),
            snippet: normalize_text(&snippet),
            source: Some(OPENALEX_BACKEND_NAME.to_string()),
        });
    }

    Ok(parsed)
}

async fn search_crossref(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    if !should_search_research_sources(query) {
        return Ok(vec![]);
    }

    let source_query = clean_query_for_source(query);
    if source_query.is_empty() {
        return Ok(vec![]);
    }

    let mut url = Url::parse(CROSSREF_WORKS_API_URL)
        .map_err(|e| SearchBackendError::new(CROSSREF_BACKEND_NAME, e.to_string()))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("query.bibliographic", &source_query);
        qp.append_pair("rows", &num_results.min(10).to_string());
    }

    let response = client
        .get(url)
        .header(USER_AGENT, "refact-agent")
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| SearchBackendError::new(CROSSREF_BACKEND_NAME, e.to_string()))?;
    if !response.status().is_success() {
        return Err(SearchBackendError::new(
            CROSSREF_BACKEND_NAME,
            format!("Crossref returned status: {}", response.status()),
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(CROSSREF_BACKEND_NAME, e.to_string()))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|e| SearchBackendError::new(CROSSREF_BACKEND_NAME, e.to_string()))?;
    let Some(items) = value
        .get("message")
        .and_then(|v| v.get("items"))
        .and_then(|v| v.as_array())
    else {
        return Ok(vec![]);
    };

    let mut parsed = Vec::new();
    for item in items.iter().take(num_results) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_array())
            .and_then(|v| v.first())
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let doi = obj.get("DOI").and_then(|v| v.as_str()).unwrap_or_default();
        let url = obj
            .get("URL")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("https://doi.org/{}", doi));
        let publisher = obj
            .get("publisher")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let publication = obj
            .get("container-title")
            .and_then(|v| v.as_array())
            .and_then(|v| v.first())
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if title.is_empty() || doi.is_empty() {
            continue;
        }
        let snippet = [publication, publisher]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(". ");
        parsed.push(SearchResult {
            title: normalize_text(title),
            url,
            snippet,
            source: Some(CROSSREF_BACKEND_NAME.to_string()),
        });
    }

    Ok(parsed)
}

async fn search_hacker_news(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    if !should_search_hacker_news(query) {
        return Ok(vec![]);
    }

    let source_query = clean_query_for_source(query);
    if source_query.is_empty() {
        return Ok(vec![]);
    }

    let mut url = Url::parse(HACKER_NEWS_SEARCH_API_URL)
        .map_err(|e| SearchBackendError::new(HACKER_NEWS_BACKEND_NAME, e.to_string()))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("query", &source_query);
        qp.append_pair("tags", "story");
        qp.append_pair("hitsPerPage", &num_results.min(10).to_string());
    }

    let response = client
        .get(url)
        .header(USER_AGENT, "refact-agent")
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| SearchBackendError::new(HACKER_NEWS_BACKEND_NAME, e.to_string()))?;
    if !response.status().is_success() {
        return Err(SearchBackendError::new(
            HACKER_NEWS_BACKEND_NAME,
            format!("Hacker News search returned status: {}", response.status()),
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| SearchBackendError::new(HACKER_NEWS_BACKEND_NAME, e.to_string()))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|e| SearchBackendError::new(HACKER_NEWS_BACKEND_NAME, e.to_string()))?;
    let Some(hits) = value.get("hits").and_then(|v| v.as_array()) else {
        return Ok(vec![]);
    };

    let mut parsed = Vec::new();
    for item in hits.iter().take(num_results) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("story_title").and_then(|v| v.as_str()))
            .unwrap_or_default();
        let object_id = obj
            .get("objectID")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let points = obj
            .get("points")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let comments = obj
            .get("num_comments")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let linked_url = obj.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        if title.is_empty() || object_id.is_empty() {
            continue;
        }
        let url = if linked_url.is_empty() {
            format!("https://news.ycombinator.com/item?id={}", object_id)
        } else {
            linked_url.to_string()
        };
        parsed.push(SearchResult {
            title: normalize_text(&unescape_html_light(title)),
            url,
            snippet: format!("{} points. {} comments.", points, comments),
            source: Some(HACKER_NEWS_BACKEND_NAME.to_string()),
        });
    }

    Ok(parsed)
}

async fn fetch_ddg_html_with_method(
    client: &Client,
    query: &str,
    user_agent: &str,
    method: DdgRequestMethod,
) -> Result<String, String> {
    let request = match method {
        DdgRequestMethod::Post => client.post(DDG_HTML_URL).form(&[("q", query)]),
        DdgRequestMethod::Get => {
            let mut url = Url::parse(DDG_HTML_URL)
                .map_err(|e| format!("Invalid DuckDuckGo HTML URL: {}", e))?;
            url.query_pairs_mut().append_pair("q", query);
            client.get(url)
        }
    };

    let response = request
        .header("User-Agent", user_agent)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "en-US,en;q=0.5")
        .header("Referer", "https://html.duckduckgo.com/")
        .header("DNT", "1")
        .header("Connection", "keep-alive")
        .header("Upgrade-Insecure-Requests", "1")
        .send()
        .await
        .map_err(|e| format!("DuckDuckGo {} request failed: {}", method.as_str(), e))?;

    if !response.status().is_success() {
        return Err(format!(
            "DuckDuckGo {} returned status: {}",
            method.as_str(),
            response.status()
        ));
    }

    if let Some(content_type) = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
    {
        if !content_type.to_ascii_lowercase().contains("text/html") {
            warn!(
                "DuckDuckGo {} returned unexpected content-type '{}' for query '{}'",
                method.as_str(),
                content_type,
                query
            );
        }
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    if contains_ddg_block_page(&body) {
        return Err("DuckDuckGo returned a captcha or rate-limit page".to_string());
    }

    Ok(body)
}

async fn search_duckduckgo(
    client: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>, SearchBackendError> {
    let attempts = [
        (USER_AGENTS[0], DdgRequestMethod::Post),
        (USER_AGENTS[0], DdgRequestMethod::Get),
        (USER_AGENTS[1], DdgRequestMethod::Post),
        (USER_AGENTS[1], DdgRequestMethod::Get),
    ];

    let mut errors = Vec::new();
    let mut saw_successful_empty_response = false;

    for (idx, (user_agent, method)) in attempts.iter().enumerate() {
        match fetch_ddg_html_with_method(client, query, user_agent, *method).await {
            Ok(html) => {
                let results = parse_ddg_html(&html);
                if results.is_empty() {
                    warn!(
                        "DDG attempt {} ({}) returned HTML but no results parsed for query: {}",
                        idx + 1,
                        method.as_str(),
                        query
                    );
                    saw_successful_empty_response = true;
                    continue;
                }

                info!(
                    "DDG search for '{}': {} results from attempt {} ({})",
                    query,
                    results.len(),
                    idx + 1,
                    method.as_str()
                );

                return Ok(results.into_iter().take(num_results).collect());
            }
            Err(err) => errors.push(format!(
                "attempt {} ({}): {}",
                idx + 1,
                method.as_str(),
                err
            )),
        }
    }

    if saw_successful_empty_response {
        return Ok(vec![]);
    }

    Err(SearchBackendError::new(
        DDG_BACKEND_NAME,
        format!(
            "failed after {} attempts: {}",
            attempts.len(),
            errors.join(" | ")
        ),
    ))
}

pub async fn execute_web_search_results(
    query: &str,
    num_results: usize,
) -> Result<(String, Vec<SearchResult>), String> {
    let query = normalize_query(query);
    if query.is_empty() {
        return Err("Search query cannot be empty".to_string());
    }

    let num_results = clamp_num_results(num_results);
    let client = build_http_client()?;

    let (
        keenable_result,
        tavily_result,
        brave_result,
        searxng_result,
        ddg_result,
        wikipedia_result,
        github_issues_result,
        github_repos_result,
        stack_overflow_result,
        npm_result,
        openalex_result,
        crossref_result,
        hacker_news_result,
    ) = tokio::join!(
        search_keenable(&client, &query, num_results),
        search_tavily(&client, &query, num_results),
        search_brave(&client, &query, num_results),
        search_searxng(&client, &query, num_results),
        search_duckduckgo(&client, &query, num_results),
        search_wikipedia(&client, &query, num_results),
        search_github_issues(&client, &query, num_results),
        search_github_repositories(&client, &query, num_results),
        search_stack_overflow(&client, &query, num_results),
        search_npm(&client, &query, num_results),
        search_openalex(&client, &query, num_results),
        search_crossref(&client, &query, num_results),
        search_hacker_news(&client, &query, num_results),
    );

    let mut merged_results = Vec::new();
    let mut errors = Vec::new();

    match keenable_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }
    match tavily_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }
    match brave_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }
    match searxng_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }
    match ddg_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }
    match wikipedia_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }
    match github_issues_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }
    match github_repos_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }
    match stack_overflow_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }
    match npm_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }
    match openalex_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }
    match crossref_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }
    match hacker_news_result {
        Ok(results) => merged_results.extend(results),
        Err(err) => errors.push(format!("{}: {}", err.backend, err.detail)),
    }

    let deduped = deduplicate_search_results(merged_results)
        .into_iter()
        .take(num_results)
        .collect::<Vec<_>>();

    if !deduped.is_empty() {
        let mut text = format_search_results(&query, &deduped);
        let primary_errors = errors
            .iter()
            .filter(|err| {
                err.starts_with(KEENABLE_BACKEND_NAME) || err.starts_with(TAVILY_BACKEND_NAME)
            })
            .cloned()
            .collect::<Vec<_>>();
        if !primary_errors.is_empty() {
            text.push_str("\n\nProvider notes:\n");
            for err in primary_errors {
                text.push_str("- ");
                text.push_str(&err);
                text.push('\n');
            }
        }
        return Ok((text, deduped));
    }

    if !errors.is_empty() {
        return Err(format!(
            "Web search failed across all configured backends. {}",
            errors.join(" | ")
        ));
    }

    Ok((format_search_results(&query, &[]), vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DDG_HTML_FIXTURE: &str = r#"
<!DOCTYPE html>
<html>
<body>
<div id="links">
    <div class="result results_links results_links_deep web-result">
        <div class="result__body">
            <h2 class="result__title">
                <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F&amp;rut=abc">
                    Rust Programming Language
                </a>
            </h2>
            <a class="result__snippet">A language empowering everyone to build reliable and efficient software.</a>
        </div>
    </div>
    <div class="result results_links results_links_deep web-result">
        <div class="result__body">
            <h2 class="result__title">
                <a class="result__a" href="https://doc.rust-lang.org/book/">
                    The Rust Programming Language - Book
                </a>
            </h2>
            <a class="result__snippet">The official book on the Rust programming language.</a>
        </div>
    </div>
    <div class="result results_links results_links_deep web-result">
        <div class="result__body">
            <h2 class="result__title">
                <a class="result__a" href="https://github.com/rust-lang/rust">
                    rust-lang/rust: The Rust compiler
                </a>
            </h2>
            <a class="result__snippet"></a>
        </div>
    </div>
</div>
</body>
</html>
    "#;

    #[test]
    fn test_parse_ddg_html_extracts_results() {
        let results = parse_ddg_html(DDG_HTML_FIXTURE);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].title, "Rust Programming Language");
        assert_eq!(results[0].url, "https://www.rust-lang.org/");
        assert_eq!(results[0].source.as_deref(), Some(DDG_BACKEND_NAME));
    }

    #[test]
    fn test_parse_ddg_html_empty() {
        let results = parse_ddg_html("<html><body></body></html>");
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_ddg_html_deduplicates_repeated_results() {
        let html = r#"
<!DOCTYPE html>
<html>
<body>
  <div class="result results_links">
    <div class="result__body">
      <a class="result__a" href="/l/?uddg=https%3A%2F%2Fexample.com%2Fguide">Example Guide</a>
      <div class="result__snippet">Primary result snippet</div>
    </div>
  </div>
  <div class="result results_links">
    <a href="https://example.com/guide">Example Guide</a>
    <div class="result__snippet">Duplicate fallback result</div>
  </div>
</body>
</html>
        "#;

        let results = parse_ddg_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example Guide");
        assert_eq!(results[0].url, "https://example.com/guide");
        assert_eq!(results[0].snippet, "Primary result snippet");
    }

    #[test]
    fn test_deduplicate_search_results_by_url_and_title() {
        let deduped = deduplicate_search_results(vec![
            SearchResult {
                title: "Example Result".to_string(),
                url: "https://example.com/page/".to_string(),
                snippet: "Primary snippet".to_string(),
                source: Some("a".to_string()),
            },
            SearchResult {
                title: "Example Result".to_string(),
                url: "https://example.com/page".to_string(),
                snippet: "".to_string(),
                source: Some("b".to_string()),
            },
            SearchResult {
                title: "Example Result".to_string(),
                url: "https://another.example.com/page".to_string(),
                snippet: "Primary snippet".to_string(),
                source: Some("c".to_string()),
            },
        ]);

        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].url, "https://example.com/page");
        assert_eq!(deduped[0].snippet, "Primary snippet");
    }

    #[test]
    fn test_parse_searxng_json_extracts_results() {
        let body = r#"{
          "results": [
            {
              "title": "Rust Book",
              "url": "https://doc.rust-lang.org/book/",
              "content": "Read the official Rust book."
            }
          ]
        }"#;

        let results = parse_searxng_json(body).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rust Book");
        assert_eq!(results[0].source.as_deref(), Some(SEARXNG_BACKEND_NAME));
    }

    #[test]
    fn test_parse_keenable_json_extracts_results() {
        let body = r#"{
          "query": "typescript best practices",
          "results": [
            {
              "title": "TypeScript Best Practices 2026",
              "url": "https://example.com/ts-best-practices",
              "description": "A comprehensive guide to modern TypeScript patterns.",
              "snippet": "TypeScript Best Practices 2026 Use strict mode...",
              "published_at": "2026-01-15T10:30:00Z",
              "acquired_at": "2026-01-16T08:12:34Z"
            }
          ]
        }"#;

        let results = parse_keenable_json(body).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "TypeScript Best Practices 2026");
        assert_eq!(results[0].url, "https://example.com/ts-best-practices");
        assert_eq!(
            results[0].snippet,
            "TypeScript Best Practices 2026 Use strict mode..."
        );
        assert_eq!(results[0].source.as_deref(), Some(KEENABLE_BACKEND_NAME));
    }

    #[test]
    fn test_parse_keenable_json_uses_description_when_snippet_missing() {
        let body = r#"{
          "query": "rust",
          "results": [
            {
              "title": "Rust",
              "url": "https://www.rust-lang.org/",
              "description": "A language empowering everyone."
            }
          ]
        }"#;

        let results = parse_keenable_json(body).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].snippet, "A language empowering everyone.");
    }

    #[test]
    fn test_parse_keenable_mcp_text_extracts_results() {
        let text = r#"Title: W3Schools.com
URL: https://www.w3schools.com/typescript/typescript_best_practices.php
Acquired: 2026-05-16

TypeScript Best Practices This guide covers essential TypeScript best practices.

---

Title: TypeScript Best Practices for Production Code in 2026
URL: https://dev.to/example/typescript-best-practices
Published: 2026-03-25
Acquired: 2026-05-11

This guide covers 15 best practices that separate production-quality TypeScript.
"#;

        let results = parse_keenable_mcp_text(text);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "W3Schools.com");
        assert_eq!(
            results[0].url,
            "https://www.w3schools.com/typescript/typescript_best_practices.php"
        );
        assert_eq!(
            results[0].snippet,
            "TypeScript Best Practices This guide covers essential TypeScript best practices."
        );
        assert_eq!(results[0].source.as_deref(), Some(KEENABLE_BACKEND_NAME));
        assert_eq!(
            results[1].title,
            "TypeScript Best Practices for Production Code in 2026"
        );
    }

    #[test]
    fn test_parse_keenable_mcp_json_extracts_text_content() {
        let body = serde_json::json!({
            "result": {
                "content": [{
                    "type": "text",
                    "text": "Title: Rust\nURL: https://www.rust-lang.org/\nAcquired: 2026-05-16\n\nA language empowering everyone."
                }]
            },
            "jsonrpc": "2.0",
            "id": 2
        })
        .to_string();

        let results = parse_keenable_mcp_json(&body).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rust");
        assert_eq!(results[0].url, "https://www.rust-lang.org/");
        assert_eq!(results[0].snippet, "A language empowering everyone.");
    }

    #[test]
    fn test_parse_tavily_json_extracts_results() {
        let body = r#"{
          "query": "typescript best practices",
          "follow_up_questions": null,
          "answer": null,
          "images": [],
          "results": [
            {
              "url": "https://dev.to/example/typescript-best-practices",
              "title": "TypeScript Best Practices in 2025",
              "content": "TypeScript Best Practices in 2025: A Comprehensive Guide.",
              "score": 0.92857105,
              "raw_content": null
            }
          ],
          "response_time": 0.86,
          "request_id": "4245bea1-66ae-4ee9-a6b0-c36b3bf43404"
        }"#;

        let results = parse_tavily_json(body).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "TypeScript Best Practices in 2025");
        assert_eq!(
            results[0].url,
            "https://dev.to/example/typescript-best-practices"
        );
        assert_eq!(
            results[0].snippet,
            "TypeScript Best Practices in 2025: A Comprehensive Guide."
        );
        assert_eq!(results[0].source.as_deref(), Some(TAVILY_BACKEND_NAME));
    }

    #[test]
    fn test_parse_tavily_json_uses_raw_content_when_content_missing() {
        let body = r#"{
          "query": "rust",
          "results": [
            {
              "url": "https://www.rust-lang.org/",
              "title": "Rust",
              "raw_content": "A language empowering everyone."
            }
          ]
        }"#;

        let results = parse_tavily_json(body).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].snippet, "A language empowering everyone.");
    }

    #[test]
    fn test_clean_query_for_source_removes_site_filters() {
        let query = r#""site:github.com/openai/codex/issues" timeout slow codex"#;
        assert_eq!(clean_query_for_source(query), "timeout slow codex");
    }

    #[test]
    fn test_github_repo_qualifier_extracts_repo_from_site_filter() {
        let query = r#""site:github.com/openai/codex/issues" timeout slow codex"#;
        assert_eq!(
            github_repo_qualifier(query).as_deref(),
            Some("repo:openai/codex")
        );
    }

    #[test]
    fn test_openalex_abstract_from_inverted_index_orders_words() {
        let value = serde_json::json!({
            "reliable": [1],
            "Search": [0],
            "results": [2]
        });
        assert_eq!(
            openalex_abstract_from_inverted_index(&value),
            "Search reliable results"
        );
    }

    #[test]
    fn test_format_search_results_with_results() {
        let results = vec![SearchResult {
            title: "Example".to_string(),
            url: "https://example.com".to_string(),
            snippet: "An example site.".to_string(),
            source: None,
        }];
        let output = format_search_results("test", &results);
        assert!(output.contains("Web search results for \"test\""));
        assert!(output.contains("1. [Example](https://example.com)"));
        assert!(output.contains("An example site."));
    }

    #[test]
    fn test_format_search_results_empty() {
        let output = format_search_results("test", &[]);
        assert!(output.contains("No web search results found"));
    }

    #[test]
    fn test_clean_ddg_url_encoded() {
        let url = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F&rut=abc";
        assert_eq!(clean_ddg_url(url), "https://www.rust-lang.org/");
    }

    #[test]
    fn test_clean_ddg_url_absolute_redirect() {
        let url = "https://duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Farticle%3Fid%3D1";
        assert_eq!(clean_ddg_url(url), "https://example.com/article?id=1");
    }

    #[test]
    fn test_clean_ddg_url_root_relative_redirect() {
        let url = "/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&rut=abc";
        assert_eq!(clean_ddg_url(url), "https://example.com/page");
    }

    #[test]
    fn test_clean_ddg_url_direct() {
        let url = "https://example.com/page";
        assert_eq!(clean_ddg_url(url), "https://example.com/page");
    }

    #[test]
    fn test_clean_ddg_url_protocol_relative() {
        let url = "//example.com/page";
        assert_eq!(clean_ddg_url(url), "https://example.com/page");
    }

    #[test]
    fn test_clamp_num_results_limits_bounds() {
        assert_eq!(clamp_num_results(0), 1);
        assert_eq!(clamp_num_results(5), 5);
        assert_eq!(clamp_num_results(999), MAX_NUM_RESULTS);
    }

    #[test]
    fn test_num_results_limit() {
        let results = parse_ddg_html(DDG_HTML_FIXTURE);
        let limited: Vec<_> = results.into_iter().take(1).collect();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].title, "Rust Programming Language");
    }
}
