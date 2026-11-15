use super::super::{SearchResult, WebSearchRequest};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

pub(super) fn headers(
    name: &'static str,
    key: Option<&str>,
    bearer: bool,
) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    if let Some(key) = key.filter(|key| !key.is_empty()) {
        let value = if bearer {
            format!("Bearer {key}")
        } else {
            key.to_owned()
        };
        let mut value = HeaderValue::from_str(&value)
            .map_err(|_| "Invalid provider API key header".to_owned())?;
        value.set_sensitive(true);
        headers.insert(HeaderName::from_static(name), value);
    }
    Ok(headers)
}

pub(super) fn key(key: Option<&str>) -> Result<&str, String> {
    key.filter(|key| !key.is_empty())
        .ok_or_else(|| "Provider requires an API key".to_owned())
}

pub(super) fn query(request: &WebSearchRequest) -> Result<&str, String> {
    request
        .queries
        .first()
        .map(String::as_str)
        .filter(|query| !query.trim().is_empty())
        .ok_or_else(|| "Search query is empty".to_owned())
}

pub(super) fn domain_query(query: &str, request: &WebSearchRequest) -> String {
    let mut query = query.to_owned();
    if !request.allowed_domains.is_empty() {
        let sites = request
            .allowed_domains
            .iter()
            .map(|domain| format!("site:{domain}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        query.push_str(&format!(" ({sites})"));
    }
    for domain in &request.blocked_domains {
        query.push_str(&format!(" -site:{domain}"));
    }
    query
}

pub(super) fn decode(value: Value) -> Value {
    match value {
        Value::String(ref text) => serde_json::from_str(text).unwrap_or(value),
        _ => value,
    }
}

pub(super) fn text(value: &Value, fields: &[&str]) -> String {
    fields
        .iter()
        .find_map(|field| {
            let value = value.get(*field)?;
            if let Some(text) = value.as_str().filter(|text| !text.trim().is_empty()) {
                return Some(text.to_owned());
            }
            value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .filter(|text| !text.is_empty())
        })
        .unwrap_or_default()
}

pub(super) fn result(
    value: &Value,
    source: &str,
    titles: &[&str],
    urls: &[&str],
    snippets: &[&str],
    dates: &[&str],
) -> Option<SearchResult> {
    let url = text(value, urls);
    if url.is_empty() {
        return None;
    }
    Some(SearchResult {
        title: text(value, titles),
        url,
        snippet: text(value, snippets),
        source: Some(source.to_owned()),
        sources: vec![source.to_owned()],
        published_date: normalize_date(&text(value, dates)),
    })
}

pub(super) fn rows<'a>(
    value: &'a Value,
    pointers: &[&str],
    provider: &str,
) -> Result<&'a Vec<Value>, String> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_array))
        .ok_or_else(|| format!("Invalid {provider} response: missing results array"))
}

pub(super) fn qdr(request: &WebSearchRequest) -> Option<String> {
    request.freshness.as_ref().map(|freshness| {
        format!(
            "qdr:{}",
            match freshness.as_str() {
                "day" => "d",
                "week" => "w",
                "month" => "m",
                "year" => "y",
                _ => "y",
            }
        )
    })
}

pub(super) fn normalize_date(text: &str) -> Option<String> {
    normalize_date_at(text, chrono::Utc::now())
}

fn normalize_date_at(text: &str, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    use chrono::{DateTime, Duration, Months, NaiveDate};
    let text = text.trim();
    if let Ok(date) = DateTime::parse_from_rfc3339(text) {
        return Some(date.with_timezone(&chrono::Utc).date_naive().to_string());
    }
    for format in ["%Y-%m-%d", "%b %e, %Y", "%e %b %Y", "%B %e, %Y", "%e %B %Y"] {
        if let Ok(date) = NaiveDate::parse_from_str(text, format) {
            return Some(date.to_string());
        }
    }
    let lower = text.to_ascii_lowercase();
    let parts: Vec<_> = lower.split_whitespace().collect();
    if parts.len() != 3 || parts[2] != "ago" {
        return None;
    }
    let count = parts[0].parse::<u32>().ok()?;
    let date = match parts[1].trim_end_matches('s') {
        "minute" => now.checked_sub_signed(Duration::try_minutes(i64::from(count))?)?,
        "hour" => now.checked_sub_signed(Duration::try_hours(i64::from(count))?)?,
        "day" => now.checked_sub_signed(Duration::try_days(i64::from(count))?)?,
        "week" => now.checked_sub_signed(Duration::try_weeks(i64::from(count))?)?,
        "month" => now.checked_sub_months(Months::new(count))?,
        "year" => now.checked_sub_months(Months::new(count.checked_mul(12)?))?,
        _ => return None,
    };
    Some(date.date_naive().to_string())
}

#[cfg(test)]
mod date_tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn normalizes_relative_and_human_dates_in_utc() {
        let now = Utc.with_ymd_and_hms(2026, 3, 31, 0, 30, 0).unwrap();
        for (input, expected) in [
            ("31 minutes ago", "2026-03-30"),
            ("2 hours ago", "2026-03-30"),
            ("2 days ago", "2026-03-29"),
            ("1 week ago", "2026-03-24"),
            ("1 month ago", "2026-02-28"),
            ("2 years ago", "2024-03-31"),
            ("Jan 2, 2025", "2025-01-02"),
            ("2 Jan 2025", "2025-01-02"),
            ("2026-03-31T00:30:00+02:00", "2026-03-30"),
        ] {
            assert_eq!(
                normalize_date_at(input, now).as_deref(),
                Some(expected),
                "{input}"
            );
        }
        for input in [
            "",
            "unknown",
            "999999999999 years ago",
            "-2 days ago",
            "Feb 30, 2025",
        ] {
            assert_eq!(normalize_date_at(input, now), None);
        }
    }

    #[test]
    fn result_normalizes_human_dates() {
        let row = serde_json::json!({"url":"https://example.org", "date":"2 Jan 2025"});
        assert_eq!(
            result(&row, "serper", &[], &["url"], &[], &["date"])
                .unwrap()
                .published_date
                .as_deref(),
            Some("2025-01-02")
        );
    }
}
