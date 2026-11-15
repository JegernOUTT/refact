use super::super::SearchResult;
use std::collections::HashSet;
use url::Url;

pub(super) fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn unescape_html_light(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

pub(super) fn strip_html_tags(text: &str) -> String {
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

pub(super) fn truncate_chars(text: &str, max_chars: usize) -> String {
    let text = normalize_text(text);
    if text.chars().count() <= max_chars {
        return text;
    }

    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

pub(super) fn clean_query_for_source(query: &str) -> String {
    let query = query
        .split_whitespace()
        .filter(|part| !part.trim_matches('"').starts_with("site:"))
        .collect::<Vec<_>>()
        .join(" ");
    normalize_text(query.trim_matches('"'))
}

pub(super) fn github_repo_qualifier(query: &str) -> Option<String> {
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

pub(super) fn clean_ddg_url(href: &str) -> String {
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
            if domain == "duckduckgo.com" || domain.ends_with(".duckduckgo.com") {
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

pub(super) fn canonicalize_result_url(url: &str) -> String {
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

pub(super) fn push_result(
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
        sources: vec![source.to_string()],
        ..Default::default()
    });
}

pub(super) fn query(request: &super::super::WebSearchRequest) -> Result<&str, String> {
    request
        .queries
        .first()
        .map(String::as_str)
        .filter(|q| !q.trim().is_empty())
        .ok_or_else(|| "Search query cannot be empty".to_string())
}

pub(super) fn timestamp(request: &super::super::WebSearchRequest) -> Option<i64> {
    request
        .freshness
        .as_ref()
        .and_then(|freshness| freshness.cutoff().and_hms_opt(0, 0, 0))
        .map(|time| time.and_utc().timestamp())
}
