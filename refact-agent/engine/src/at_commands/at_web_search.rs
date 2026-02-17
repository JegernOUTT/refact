use std::time::Duration;
use tracing::{info, warn};
use reqwest::Client;
use select::document::Document;
use select::predicate::{Class, Name};

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

const DDG_HTML_URL: &str = "https://html.duckduckgo.com/html/";
const DDG_TIMEOUT_SECS: u64 = 10;

const USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
];

pub fn parse_ddg_html(html: &str) -> Vec<SearchResult> {
    let document = Document::from(html);
    let mut results = Vec::new();

    for node in document.find(Class("result__body")) {
        let title = node
            .find(Class("result__a"))
            .next()
            .map(|n| n.text().trim().to_string())
            .unwrap_or_default();

        let url = node
            .find(Class("result__a"))
            .next()
            .and_then(|n| n.attr("href"))
            .map(|href| clean_ddg_url(href))
            .unwrap_or_default();

        let snippet = node
            .find(Class("result__snippet"))
            .next()
            .map(|n| n.text().trim().to_string())
            .unwrap_or_default();

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult { title, url, snippet });
        }
    }

    if results.is_empty() {
        for node in document.find(Class("results_links")) {
            let title = node
                .find(Name("a"))
                .next()
                .map(|n| n.text().trim().to_string())
                .unwrap_or_default();

            let url = node
                .find(Name("a"))
                .next()
                .and_then(|n| n.attr("href"))
                .map(|href| clean_ddg_url(href))
                .unwrap_or_default();

            let snippet = node
                .find(Class("result__snippet"))
                .next()
                .or_else(|| node.find(Name("td")).last())
                .map(|n| n.text().trim().to_string())
                .unwrap_or_default();

            if !title.is_empty() && !url.is_empty() {
                results.push(SearchResult { title, url, snippet });
            }
        }
    }

    results
}

fn clean_ddg_url(href: &str) -> String {
    if href.starts_with("//duckduckgo.com/l/?uddg=") {
        if let Some(encoded) = href.strip_prefix("//duckduckgo.com/l/?uddg=") {
            let encoded = encoded.split('&').next().unwrap_or(encoded);
            return percent_encoding::percent_decode_str(encoded)
                .decode_utf8_lossy()
                .to_string();
        }
    }
    if href.starts_with("http") {
        return href.to_string();
    }
    if href.starts_with("//") {
        return format!("https:{}", href);
    }
    href.to_string()
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

async fn fetch_ddg_html(query: &str, user_agent: &str) -> Result<String, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(DDG_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let params = [("q", query)];

    let response = client
        .post(DDG_HTML_URL)
        .header("User-Agent", user_agent)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.5")
        .header("Referer", "https://html.duckduckgo.com/")
        .header("DNT", "1")
        .header("Connection", "keep-alive")
        .header("Upgrade-Insecure-Requests", "1")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("DuckDuckGo request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("DuckDuckGo returned status: {}", response.status()));
    }

    let body = response.text().await.map_err(|e| format!("Failed to read response body: {}", e))?;

    if body.contains("Please try again") || body.contains("bot") && body.len() < 2000 {
        return Err("DuckDuckGo returned a captcha or rate-limit page".to_string());
    }

    Ok(body)
}

pub async fn execute_web_search(query: &str, num_results: usize) -> Result<String, String> {
    match fetch_ddg_html(query, USER_AGENTS[0]).await {
        Ok(html) => {
            let results = parse_ddg_html(&html);
            if results.is_empty() {
                warn!("DDG returned HTML but no results parsed for query: {}", query);
            } else {
                info!("DDG search for '{}': {} results", query, results.len());
            }
            let results: Vec<_> = results.into_iter().take(num_results).collect();
            Ok(format_search_results(query, &results))
        }
        Err(first_err) => {
            warn!("First DDG attempt failed: {}, retrying with different User-Agent", first_err);
            match fetch_ddg_html(query, USER_AGENTS[1]).await {
                Ok(html) => {
                    let results = parse_ddg_html(&html);
                    let results: Vec<_> = results.into_iter().take(num_results).collect();
                    Ok(format_search_results(query, &results))
                }
                Err(second_err) => {
                    Err(format!(
                        "Web search failed after retrying. First attempt: {}. Second attempt: {}. Please try again later.",
                        first_err, second_err
                    ))
                }
            }
        }
    }
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
        assert_eq!(results[0].snippet, "A language empowering everyone to build reliable and efficient software.");

        assert_eq!(results[1].title, "The Rust Programming Language - Book");
        assert_eq!(results[1].url, "https://doc.rust-lang.org/book/");
        assert_eq!(results[1].snippet, "The official book on the Rust programming language.");

        assert_eq!(results[2].title, "rust-lang/rust: The Rust compiler");
        assert_eq!(results[2].url, "https://github.com/rust-lang/rust");
        assert_eq!(results[2].snippet, "");
    }

    #[test]
    fn test_parse_ddg_html_empty() {
        let results = parse_ddg_html("<html><body></body></html>");
        assert!(results.is_empty());
    }

    #[test]
    fn test_format_search_results_with_results() {
        let results = vec![
            SearchResult {
                title: "Example".to_string(),
                url: "https://example.com".to_string(),
                snippet: "An example site.".to_string(),
            },
        ];
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
    fn test_num_results_limit() {
        let results = parse_ddg_html(DDG_HTML_FIXTURE);
        let limited: Vec<_> = results.into_iter().take(1).collect();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].title, "Rust Programming Language");
    }
}
