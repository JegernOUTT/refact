use std::collections::{HashMap, HashSet};
use std::time::Duration;

use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use select::document::Document;
use select::predicate::{Class, Name};
use serde_json::Value;
use tracing::{info, warn};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

const DDG_HTML_URL: &str = "https://html.duckduckgo.com/html/";
const DDG_TIMEOUT_SECS: u64 = 10;
pub const DEFAULT_NUM_RESULTS: usize = 8;
pub const MAX_NUM_RESULTS: usize = 20;
const SEARXNG_BACKEND_NAME: &str = "searxng";
const DDG_BACKEND_NAME: &str = "duckduckgo";
const WIKIPEDIA_BACKEND_NAME: &str = "wikipedia";
const WIKIPEDIA_API_URL: &str = "https://en.wikipedia.org/w/api.php";

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

pub fn format_search_results(query: &str, results: &[SearchResult]) -> String {
    if results.is_empty() {
        return format!("No web search results found for \"{}\".", query);
    }

    let mut output = format!("Web search results for \"{}\":\n\n", query);
    for (i, result) in results.iter().enumerate() {
        output.push_str(&format!("{}. [{}]({})\n", i + 1, result.title, result.url));
        if !result.snippet.is_empty() {
            output.push_str(&format!("   {}\n", result.snippet));
        }
        output.push('\n');
    }
    output
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

    let (searxng_result, ddg_result, wikipedia_result) = tokio::join!(
        search_searxng(&client, &query, num_results),
        search_duckduckgo(&client, &query, num_results),
        search_wikipedia(&client, &query, num_results),
    );

    let mut merged_results = Vec::new();
    let mut errors = Vec::new();

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

    let deduped = deduplicate_search_results(merged_results)
        .into_iter()
        .take(num_results)
        .collect::<Vec<_>>();

    if !deduped.is_empty() {
        let text = format_search_results(&query, &deduped);
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
