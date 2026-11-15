use super::super::{WebSearchRequest, WebSearchConfig, SearchResult};
use super::super::http::{client, request_error, text};
use super::legacy_common::*;

use html5ever::tendril::TendrilSink;
use html5ever::parse_document;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use std::collections::HashSet;
use crate::html_dom::{
    attribute as node_attribute, descendant_nodes, element_name as node_element_name,
};
const DDG_BACKEND_NAME: &str = "duckduckgo";
fn contains_ddg_block_page(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    if lower.contains("anomaly.js")
        || lower.contains("anomaly-modal")
        || lower.contains("challenge-form")
        || lower.contains("select all squares containing a duck")
    {
        return true;
    }
    let concrete_block_phrases = [
        "captcha",
        "verify you're human",
        "verify you are human",
        "verify-human",
        "automated requests",
        "automated-requests",
        "unusual traffic",
        "unusual-traffic",
    ];
    let generic_bot_phrases = [
        "are you a bot",
        "are you a robot",
        "not a robot",
        "bot detection",
        "bot-detection",
        "bot challenge",
        "bot-challenge",
        "detected as a bot",
        "bots use duckduckgo",
    ];
    let supporting_challenge_markers = [
        "access denied",
        "complete the challenge",
        "please try again",
        "security check",
    ];
    let contains_generic_bot_phrase = generic_bot_phrases
        .iter()
        .any(|phrase| lower.contains(phrase));

    concrete_block_phrases
        .iter()
        .any(|phrase| lower.contains(phrase))
        || contains_generic_bot_phrase
            && (body.len() < 4000
                || supporting_challenge_markers
                    .iter()
                    .any(|marker| lower.contains(marker)))
        || lower.contains("please try again") && body.len() < 4000
}

pub fn parse_ddg_html(html: &str) -> Vec<SearchResult> {
    let document = parse_document(RcDom::default(), Default::default()).one(html);
    let document_nodes = descendant_nodes(&document.document);
    let mut results = Vec::new();
    let mut seen_urls = HashSet::new();

    for node in matching_nodes(&document_nodes, "result__body") {
        let descendants = descendant_nodes(&node);
        let anchor = find_class_node(&descendants, "result__a")
            .or_else(|| find_element_node(&descendants, "a"));

        let title = anchor.as_ref().map(node_text).unwrap_or_default();
        let url = anchor
            .as_ref()
            .and_then(|node| node_attribute(node, "href"))
            .unwrap_or_default()
            .to_owned();
        let snippet = find_class_node(&descendants, "result__snippet")
            .map(|node| node_text(&node))
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

    for node in matching_nodes(&document_nodes, "results_links") {
        let descendants = descendant_nodes(&node);
        let anchor = find_class_node(&descendants, "result__a")
            .or_else(|| find_element_node(&descendants, "a"));

        let title = anchor.as_ref().map(node_text).unwrap_or_default();
        let url = anchor
            .as_ref()
            .and_then(|node| node_attribute(node, "href"))
            .unwrap_or_default()
            .to_owned();
        let snippet = find_class_node(&descendants, "result__snippet")
            .or_else(|| find_element_nodes(&descendants, "td").pop())
            .map(|node| node_text(&node))
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

    for node in matching_nodes(&document_nodes, "result") {
        let descendants = descendant_nodes(&node);
        let anchor = find_class_node(&descendants, "result__a")
            .or_else(|| find_element_node(&descendants, "a"));

        let title = anchor.as_ref().map(node_text).unwrap_or_default();
        let url = anchor
            .as_ref()
            .and_then(|node| node_attribute(node, "href"))
            .unwrap_or_default()
            .to_owned();
        let snippet = find_class_node(&descendants, "result__snippet")
            .map(|node| node_text(&node))
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

fn matching_nodes(nodes: &[Handle], class: &str) -> Vec<Handle> {
    nodes
        .iter()
        .filter(|node| node_has_class(node, class))
        .cloned()
        .collect()
}

fn find_class_node(nodes: &[Handle], class: &str) -> Option<Handle> {
    nodes
        .iter()
        .find(|node| node_has_class(node, class))
        .cloned()
}

fn find_element_node(nodes: &[Handle], name: &str) -> Option<Handle> {
    nodes
        .iter()
        .find(|node| node_element_name(node) == Some(name))
        .cloned()
}

fn find_element_nodes(nodes: &[Handle], name: &str) -> Vec<Handle> {
    nodes
        .iter()
        .filter(|node| node_element_name(node) == Some(name))
        .cloned()
        .collect()
}

fn node_has_class(node: &Handle, class: &str) -> bool {
    node_attribute(node, "class")
        .is_some_and(|classes| classes.split_whitespace().any(|token| token == class))
}

fn node_text(node: &Handle) -> String {
    let mut text = String::new();
    let mut pending = vec![node.clone()];
    while let Some(node) = pending.pop() {
        match &node.data {
            NodeData::Text {
                contents,
            } => text.push_str(contents.borrow().as_ref()),
            _ => pending.extend(node.children.borrow().iter().rev().cloned()),
        }
    }
    text
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
    fn test_parse_ddg_result_body_layout() {
        let html = r#"
            <div class="extra result__body">
                <a href="https://ignored.example">ignored anchor</a>
                <a class="result__a" href="https://first.example"> First <b>result</b> </a>
                <a class="result__a" href="https://second.example">Second result</a>
                <span class="result__snippet extra"> First <b>snippet</b> </span>
                <span class="result__snippet">Second snippet</span>
            </div>
        "#;

        let results = parse_ddg_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "First result");
        assert_eq!(results[0].url, "https://first.example/");
        assert_eq!(results[0].snippet, "First snippet");
    }

    #[test]
    fn test_parse_ddg_results_links_layout_uses_last_table_cell_fallback() {
        let html = r#"
            <div class="extra results_links">
                <a href="https://first.example"> First <b>link</b> </a>
                <a href="https://second.example">Second link</a>
                <table><tr><td>First cell</td><td> Last <b>cell</b> </td></tr></table>
            </div>
        "#;

        let results = parse_ddg_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "First link");
        assert_eq!(results[0].url, "https://first.example/");
        assert_eq!(results[0].snippet, "Last cell");
    }

    #[test]
    fn test_ddg_block_page_allows_ordinary_bot_substrings() {
        for body in [
            "<html><body>Robots help automate warehouse work.</body></html>",
            "<html><body>See /robots.txt for crawler rules.</body></html>",
            "<html><body>Our robotics club meets every Tuesday.</body></html>",
            "<html><body>Continue at the bottom of the page.</body></html>",
        ] {
            assert!(!contains_ddg_block_page(body), "unexpected block: {body}");
        }
    }

    #[test]
    fn test_ddg_block_page_detects_concrete_challenges() {
        for body in [
            "<html><body>Please complete this CAPTCHA.</body></html>",
            "<html><body>Verify you are human to continue.</body></html>",
            "<html><body>We detected automated requests.</body></html>",
            "<html><body>Unusual traffic was detected.</body></html>",
            "<html><body>Bot-detection challenge in progress.</body></html>",
            "<html><body>Are you a robot?</body></html>",
            "<html><body>Please try again later.</body></html>",
        ] {
            assert!(contains_ddg_block_page(body), "missed block: {body}");
        }

        for phrase in [
            "Please complete this CAPTCHA.",
            "Verify you're human.",
            "A verify-human interstitial.",
            "Automated requests were detected.",
            "Automated-requests protection.",
            "Unusual traffic was detected.",
            "Unusual-traffic protection.",
        ] {
            let large_concrete_challenge = format!(
                "<html><body>{phrase}{}</body></html>",
                " challenge padding".repeat(300)
            );
            assert!(contains_ddg_block_page(&large_concrete_challenge));
        }

        let large_generic_challenge = format!(
            "<html><body><h1>Security check</h1><p>Are you a bot?</p>{}</body></html>",
            " challenge padding".repeat(300)
        );
        assert!(contains_ddg_block_page(&large_generic_challenge));

        for phrase in [
            "Bot detection",
            "bot-detection",
            "bot challenge",
            "bot-challenge",
            "detected as a bot",
            "bots use DuckDuckGo",
        ] {
            let large_supported_challenge = format!(
                "<html><body><h1>Access denied</h1><p>{phrase}</p>{}</body></html>",
                " challenge padding".repeat(300)
            );
            assert!(
                contains_ddg_block_page(&large_supported_challenge),
                "missed supported challenge: {phrase}"
            );
        }
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
    fn detects_duck_challenge() {
        assert!(contains_ddg_block_page(
            "<form id='challenge-form'>Select all squares containing a duck</form>"
        ));
        assert!(contains_ddg_block_page(
            "<script src='/anomaly.js'></script>"
        ));
    }
    #[test]
    fn redirects_are_decoded() {
        assert_eq!(
            clean_ddg_url("https://duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa"),
            "https://example.com/a"
        );
    }
}

pub(super) async fn search(
    request: &WebSearchRequest,
    _config: &WebSearchConfig,
    _key: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    let client = client()?;
    let mut errors = Vec::new();
    for post in [true, false] {
        let builder = if post {
            client
                .post("https://html.duckduckgo.com/html/")
                .form(&[("q", query(request)?)])
        } else {
            client
                .get("https://html.duckduckgo.com/html/")
                .query(&[("q", query(request)?)])
        };
        let response = match builder
            .header(
                "User-Agent",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36",
            )
            .header("Accept", "text/html")
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                errors.push(request_error(error));
                continue;
            }
        };
        let status = response.status().as_u16();
        if status == 202 {
            return Err("DuckDuckGo blocked by captcha".to_string());
        }
        let body = text(response).await?;
        if !(200..300).contains(&status) {
            errors.push(format!("DuckDuckGo returned HTTP {status}"));
            continue;
        }
        return Ok(parse_response(status, &body)?
            .into_iter()
            .take(request.num_results)
            .collect());
    }
    Err(errors.join(" | "))
}

fn parse_response(status: u16, body: &str) -> Result<Vec<SearchResult>, String> {
    if status == 202 {
        return Err("DuckDuckGo blocked by captcha".into());
    }
    if !(200..300).contains(&status) {
        return Err(format!("DuckDuckGo returned HTTP {status}"));
    }
    let results = parse_ddg_html(body);
    if results.is_empty() && contains_ddg_block_page(body) {
        return Err("DuckDuckGo blocked by captcha".into());
    }
    Ok(results)
}

#[cfg(test)]
mod response_tests {
    use super::*;
    #[test]
    fn ordinary_captcha_results_are_not_challenges() {
        let html = r#"<div class="result__body"><a class="result__a" href="https://example.org">CAPTCHA challenge</a><span class="result__snippet">Verify you are human</span></div>"#;
        assert_eq!(parse_response(200, html).unwrap().len(), 1);
        assert!(parse_response(202, html).is_err());
        assert!(parse_response(200, "<form id=challenge-form>captcha</form>").is_err());
    }
}
