use std::collections::{BTreeMap, HashMap, HashSet};
use chrono::NaiveDate;
use url::Url;
use super::{SearchResult, WebSearchRequest};
fn canonical(url: &str) -> Option<String> {
    let mut url = Url::parse(url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let _ = url.set_scheme("https");
    if let Some(host) = url
        .host_str()
        .and_then(|host| host.strip_prefix("www."))
        .map(str::to_owned)
    {
        let _ = url.set_host(Some(&host));
    }
    url.set_fragment(None);
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(if path.is_empty() { "/" } else { &path });
    Some(url.to_string().trim_end_matches('/').to_owned())
}
fn matches(host: &str, domain: &str) -> bool {
    let domain = domain.strip_prefix("www.").unwrap_or(domain);
    host == domain || host.ends_with(&format!(".{domain}"))
}
fn accepted(result: &SearchResult, request: &WebSearchRequest) -> bool {
    let Some(host) = Url::parse(&canonical(&result.url).unwrap_or_default())
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
    else {
        return false;
    };
    if !request.allowed_domains.is_empty()
        && !request.allowed_domains.iter().any(|d| matches(&host, d))
    {
        return false;
    }
    if request.blocked_domains.iter().any(|d| matches(&host, d)) {
        return false;
    }
    if let (Some(freshness), Some(date)) = (
        request.freshness,
        result
            .published_date
            .as_ref()
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
    ) {
        if date < freshness.cutoff() {
            return false;
        }
    }
    true
}
pub(super) fn fuse(
    lists: &[(String, f64, Vec<SearchResult>)],
    request: &WebSearchRequest,
) -> Vec<SearchResult> {
    let mut entries: BTreeMap<String, (f64, f64, SearchResult)> = BTreeMap::new();
    let mut ordered = lists.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    for (provider, weight, results) in ordered {
        let mut seen = HashSet::new();
        for (rank, result) in results.iter().enumerate() {
            let Some(url) = canonical(&result.url) else {
                continue;
            };
            let mut result = result.clone();
            result.title = result
                .title
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if result.title.is_empty() {
                continue;
            }
            result.published_date = result.published_date.and_then(|date| {
                date.get(..10)
                    .and_then(|prefix| NaiveDate::parse_from_str(prefix, "%Y-%m-%d").ok())
                    .map(|date| date.to_string())
            });
            result.snippet = result
                .snippet
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(600)
                .collect();
            result.source = Some(provider.clone());
            result.sources = vec![provider.clone()];
            let score = if seen.insert(url.clone()) {
                weight / (60.0 + (rank + 1) as f64)
            } else {
                0.
            };
            match entries.get_mut(&url) {
                Some((total, best_score, existing)) => {
                    if score > *best_score {
                        *best_score = score;
                        existing.url = result.url.clone();
                        existing.title = result.title.clone();
                        existing.source = result.source.clone();
                    }
                    *total += score;
                    if !existing.sources.contains(provider) {
                        existing.sources.push(provider.clone());
                    }
                    if result.snippet.chars().count() > existing.snippet.chars().count() {
                        existing.snippet = result.snippet;
                    }
                    if existing.published_date.is_none() {
                        existing.published_date = result.published_date;
                    }
                }
                None => {
                    entries.insert(url, (score, score, result));
                }
            }
        }
    }
    let mut entries = entries.into_values().collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| canonical(&a.2.url).cmp(&canonical(&b.2.url)))
    });
    let mut domains = HashMap::<String, usize>::new();
    let mut results = Vec::new();
    for (_, _, result) in entries {
        if !accepted(&result, request) {
            continue;
        }
        let host = Url::parse(&canonical(&result.url).unwrap_or_default())
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        let count = domains.entry(host).or_default();
        if request.allowed_domains.len() != 1 && *count >= 3 {
            continue;
        }
        *count += 1;
        results.push(result);
        if results.len() >= request.num_results {
            break;
        }
    }
    results
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_metadata_precedes_freshness_and_same_list_scoring() {
        let mut old = result("https://a.test", "a longer snippet");
        old.published_date = Some("2000-01-01".into());
        let lists = vec![("a".into(), 1., vec![result("https://a.test", "s"), old])];
        let mut request = WebSearchRequest::default();
        let results = fuse(&lists, &request);
        assert_eq!(results[0].snippet, "a longer snippet");
        assert_eq!(results[0].published_date.as_deref(), Some("2000-01-01"));
        request.freshness = Some(super::super::Freshness::Day);
        assert!(fuse(&lists, &request).is_empty());
        let lists = vec![
            ("a".into(), 1., vec![result("https://a.test", "s")]),
            ("b".into(), 1., lists[0].2[1..].to_vec()),
        ];
        assert!(fuse(&lists, &request).is_empty());
    }
    #[test]
    fn same_list_duplicate_adds_no_rrf_score() {
        let lists = vec![(
            "a".into(),
            1.,
            vec![
                result("https://first.test", "s"),
                result("https://second.test", "s"),
                result("https://second.test#duplicate", "longer"),
            ],
        )];
        let results = fuse(&lists, &WebSearchRequest::default());
        assert_eq!(results[0].url, "https://first.test");
        assert_eq!(results[1].snippet, "longer");
    }
    #[test]
    fn canonical_variants_share_identity_and_domain_cap() {
        let lists = vec![(
            "a".into(),
            1.,
            vec![
                result("http://www.a.test/a", "s"),
                result("https://a.test/a", "longer"),
                result("https://www.a.test/b", "s"),
                result("https://a.test/c", "s"),
                result("https://www.a.test/d", "s"),
            ],
        )];
        let results = fuse(&lists, &WebSearchRequest::default());
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].url, "http://www.a.test/a");
        assert_eq!(results[0].snippet, "longer");
    }
    #[test]
    fn original_urls_follow_best_ranked_occurrence() {
        let lists = vec![
            (
                "a".into(),
                1.,
                vec![
                    result("http://only.test/", "s"),
                    result("http://www.a.test/page/", "s"),
                ],
            ),
            (
                "z".into(),
                1.,
                vec![result("https://www.a.test/page", "longer")],
            ),
        ];
        let results = fuse(&lists, &WebSearchRequest::default());
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://www.a.test/page");
        assert_eq!(results[1].url, "http://only.test/");
        let mut request = WebSearchRequest::default();
        request.allowed_domains = vec!["www.a.test".into()];
        assert_eq!(fuse(&lists, &request).len(), 1);
        request.blocked_domains = vec!["a.test".into()];
        assert!(fuse(&lists, &request).is_empty());
    }
    fn result(url: &str, snippet: &str) -> SearchResult {
        SearchResult {
            title: url.into(),
            url: url.into(),
            snippet: snippet.into(),
            ..Default::default()
        }
    }
    #[test]
    fn consensus_boost_and_metadata_merge() {
        let r = WebSearchRequest {
            queries: vec!["q".into()],
            ..Default::default()
        };
        let mut dated = result("https://b.test/", "longer snippet");
        dated.published_date = Some("2026-01-01".into());
        let out = fuse(
            &[
                (
                    "a".into(),
                    1.,
                    vec![
                        result("https://a.test", "s"),
                        result("https://b.test#x", "s"),
                    ],
                ),
                ("b".into(), 1., vec![dated]),
            ],
            &r,
        );
        assert_eq!(out[0].url, "https://b.test/");
        assert_eq!(out[0].sources, ["a", "b"]);
        assert_eq!(out[0].snippet, "longer snippet");
        assert_eq!(out[0].published_date.as_deref(), Some("2026-01-01"));
    }
    #[test]
    fn domain_cap_allow_block_and_freshness() {
        let mut req = WebSearchRequest {
            queries: vec!["q".into()],
            num_results: 20,
            ..Default::default()
        };
        let mut old = result("https://old.test", "s");
        old.published_date = Some("2000-01-01".into());
        let list = vec![(
            "a".into(),
            1.,
            vec![
                result("https://example.com/a", "s"),
                result("http://www.example.com/b/", "s"),
                result("https://www.example.com/c", "s"),
                result("http://example.com/d", "s"),
                result("https://sub.example.com/a", "s"),
                result("https://notexample.com", "s"),
                old,
            ],
        )];
        assert_eq!(fuse(&list, &req).len(), 6);
        req.allowed_domains = vec!["example.com".into()];
        assert_eq!(fuse(&list, &req).len(), 5);
        req.blocked_domains = vec!["sub.example.com".into()];
        assert_eq!(fuse(&list, &req).len(), 4);
        req.allowed_domains.clear();
        req.blocked_domains.clear();
        req.freshness = Some(super::super::Freshness::Day);
        assert_eq!(fuse(&list, &req).len(), 5);
    }
}
