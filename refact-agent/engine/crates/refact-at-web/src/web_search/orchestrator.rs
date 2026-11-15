use std::time::{Duration, Instant};
use futures::{stream::FuturesUnordered, StreamExt};
use serde::Serialize;
use super::{
    *,
    providers::{SearchProvider, CatalogProvider},
    circuit::{
        lease_backend_attempt, BackendAttemptLease, BackendAttemptGuard, record_backend_result,
    },
};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Ok,
    Empty,
    Error,
    Timeout,
    Skipped,
    NotConfigured,
    Disabled,
    NotApplicable,
}
#[derive(Debug, Clone, Serialize)]
pub struct ProviderTopResult {
    pub title: String,
    pub url: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct ProviderReport {
    pub provider: String,
    pub status: ProviderStatus,
    pub results: usize,
    pub latency_ms: u64,
    pub detail: Option<String>,
    pub top: Vec<ProviderTopResult>,
}
#[derive(Debug, Clone)]
pub struct WebSearchOutcome {
    pub text: String,
    pub results: Vec<SearchResult>,
    pub reports: Vec<ProviderReport>,
}
fn report(id: &str, status: ProviderStatus, detail: Option<String>) -> ProviderReport {
    ProviderReport {
        provider: id.into(),
        status,
        results: 0,
        latency_ms: 0,
        detail,
        top: vec![],
    }
}
fn inactive(
    info: &ProviderInfo,
    request: &WebSearchRequest,
    config: &WebSearchConfig,
) -> Option<ProviderStatus> {
    let runtime = config.providers.get(info.id);
    if !runtime.map(|c| c.enabled).unwrap_or(info.default_enabled) {
        return Some(ProviderStatus::Disabled);
    }
    if info.category.is_some_and(|c| c != request.category) {
        return Some(ProviderStatus::NotApplicable);
    }
    let has_key = runtime
        .and_then(|c| c.api_key.as_ref())
        .is_some_and(|k| !k.trim().is_empty());
    if info.key == KeyRequirement::Required && !has_key
        || info.id == "searxng"
            && !config
                .searxng_url
                .as_ref()
                .is_some_and(|u| !u.trim().is_empty())
    {
        return Some(ProviderStatus::NotConfigured);
    }
    None
}
async fn run(
    info: &ProviderInfo,
    request: &WebSearchRequest,
    config: &WebSearchConfig,
    provider: &dyn SearchProvider,
    end: tokio::time::Instant,
    bypass: bool,
) -> (ProviderReport, Vec<Vec<SearchResult>>) {
    if let Some(status) = inactive(info, request, config) {
        return (report(info.id, status, None), vec![]);
    }
    let guard = if bypass {
        None
    } else {
        match lease_backend_attempt(info.id, Instant::now()) {
            BackendAttemptLease::Run(attempt) => Some(BackendAttemptGuard {
                backend: info.id,
                attempt,
            }),
            BackendAttemptLease::Skip(detail) => {
                return (
                    report(info.id, ProviderStatus::Skipped, Some(detail)),
                    vec![],
                )
            }
        }
    };
    let key = config
        .providers
        .get(info.id)
        .and_then(|c| c.api_key.as_deref())
        .map(str::trim)
        .filter(|k| !k.is_empty());
    let started = Instant::now();
    let provider_end = tokio::time::Instant::now() + config.provider_timeout;
    let bound = end.min(provider_end);
    let mut calls = FuturesUnordered::new();
    for (index, query) in request.queries.iter().enumerate() {
        let mut single = request.clone();
        if info.id != "parallel" {
            single.queries = vec![query.clone()];
        }
        calls.push(async move { (index, provider.search(info.id, &single, config, key).await) });
        if info.id == "parallel" {
            break;
        }
    }
    let mut completed = Vec::new();
    let mut errors = Vec::new();
    let mut timed_out = false;
    while !calls.is_empty() {
        match tokio::time::timeout_at(bound, calls.next()).await {
            Ok(Some((index, Ok(values)))) => completed.push((index, values)),
            Ok(Some((index, Err(error)))) => errors.push((index, error)),
            Ok(None) => break,
            Err(_) => {
                timed_out = true;
                break;
            }
        }
    }
    completed.sort_by_key(|(index, _)| *index);
    errors.sort_by_key(|(index, _)| *index);
    let partial = !completed.is_empty() && (timed_out || !errors.is_empty());
    let result = if !completed.is_empty() {
        Ok(completed
            .into_iter()
            .map(|(_, values)| values)
            .collect::<Vec<_>>())
    } else if timed_out {
        Err("Provider timed out".into())
    } else {
        Err(errors
            .into_iter()
            .map(|(_, error)| error)
            .collect::<Vec<_>>()
            .join("; "))
    };
    let status = match &result {
        Ok(results) if results.iter().all(Vec::is_empty) => ProviderStatus::Empty,
        Ok(_) => ProviderStatus::Ok,
        Err(_) if timed_out => ProviderStatus::Timeout,
        Err(_) => ProviderStatus::Error,
    };
    if let Some(guard) = &guard {
        if result.is_ok() || !timed_out || provider_end <= end {
            record_backend_result(
                info.id,
                guard.attempt,
                &result
                    .as_ref()
                    .map(|lists| lists.iter().flatten().cloned().collect())
                    .map_err(Clone::clone),
            );
        }
    }
    let mut report = report(info.id, status, result.as_ref().err().cloned());
    if partial {
        report.detail = Some("Partial success: some queries failed or timed out".into());
    }
    report.latency_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    let results = result.unwrap_or_default();
    report.results = results.iter().map(Vec::len).sum();
    report.top = results
        .iter()
        .flatten()
        .take(3)
        .map(|r| ProviderTopResult {
            title: r.title.clone(),
            url: r.url.clone(),
        })
        .collect();
    (report, results)
}
async fn execute_with(
    request: &WebSearchRequest,
    config: &WebSearchConfig,
    provider: &dyn SearchProvider,
) -> Result<WebSearchOutcome, String> {
    let mut request = request.clone();
    request.normalize()?;
    let end = tokio::time::Instant::now() + config.deadline;
    let mut reports = Vec::new();
    let mut lists = Vec::new();
    for fallback in [false, true] {
        let enough = rank::fuse(&lists, &request).len() >= request.num_results;
        let should_run = !fallback
            || !enough
                && end.saturating_duration_since(tokio::time::Instant::now())
                    >= Duration::from_millis(1500);
        let mut calls = FuturesUnordered::new();
        for info in provider_catalog()
            .iter()
            .filter(|p| (p.tier == ProviderTier::Fallback) == fallback)
        {
            if should_run {
                calls.push(run(info, &request, config, provider, end, false));
            } else {
                reports.push(report(
                    info.id,
                    inactive(info, &request, config).unwrap_or(ProviderStatus::NotApplicable),
                    Some(
                        if enough {
                            "fallback not needed"
                        } else {
                            "insufficient deadline remaining"
                        }
                        .into(),
                    ),
                ));
            }
        }
        while let Some((report, results)) = calls.next().await {
            for results in results {
                lists.push((
                    report.provider.clone(),
                    if fallback { 0.6 } else { 1. },
                    results,
                ));
            }
            reports.push(report);
        }
    }
    reports.sort_by_key(|r| {
        provider_catalog()
            .iter()
            .position(|p| p.id == r.provider)
            .unwrap_or(usize::MAX)
    });
    let results = rank::fuse(&lists, &request);
    if results.is_empty()
        && !reports.iter().any(|r| {
            matches!(
                r.status,
                ProviderStatus::Ok
                    | ProviderStatus::Empty
                    | ProviderStatus::Error
                    | ProviderStatus::Timeout
            )
        })
    {
        return Err("Web search failed: No active search providers (disabled, not configured, or in cooldown)".into());
    }
    let attempted = reports
        .iter()
        .filter(|r| {
            matches!(
                r.status,
                ProviderStatus::Ok
                    | ProviderStatus::Empty
                    | ProviderStatus::Error
                    | ProviderStatus::Timeout
            )
        })
        .collect::<Vec<_>>();
    if results.is_empty()
        && !attempted.is_empty()
        && attempted
            .iter()
            .all(|r| matches!(r.status, ProviderStatus::Error | ProviderStatus::Timeout))
    {
        return Err(format!(
            "Web search failed: {}",
            attempted
                .iter()
                .map(|r| format!(
                    "{}: {}",
                    r.provider,
                    r.detail.as_deref().unwrap_or("failed")
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let text = format::outcome(&request, &results, &reports);
    Ok(WebSearchOutcome {
        text,
        results,
        reports,
    })
}
pub async fn execute_web_search(
    request: &WebSearchRequest,
    config: &WebSearchConfig,
) -> Result<WebSearchOutcome, String> {
    execute_with(request, config, &CatalogProvider).await
}
pub async fn test_providers(
    config: &WebSearchConfig,
    query: &str,
    only: Option<&[String]>,
) -> Vec<ProviderReport> {
    let mut calls = FuturesUnordered::new();
    for info in provider_catalog() {
        if only.is_some_and(|ids| !ids.iter().any(|id| id == info.id)) {
            continue;
        }
        let request = WebSearchRequest {
            queries: vec![query.into()],
            category: info.category.unwrap_or_default(),
            ..Default::default()
        };
        calls.push(async move {
            run(
                info,
                &request,
                config,
                &CatalogProvider,
                tokio::time::Instant::now() + config.provider_timeout,
                true,
            )
            .await
            .0
        });
    }
    let mut reports = Vec::new();
    while let Some(report) = calls.next().await {
        reports.push(report);
    }
    reports.sort_by(|a, b| a.provider.cmp(&b.provider));
    reports
}
#[cfg(test)]
mod tests {
    use super::*;
    use super::super::providers::SearchFuture;
    use std::{collections::BTreeMap, sync::Mutex};

    #[derive(Clone)]
    enum Reply {
        Results(Vec<SearchResult>),
        Error(&'static str),
        Pending,
        Delayed(Duration, Vec<SearchResult>),
        ByQuery(BTreeMap<String, Reply>),
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Call {
        id: String,
        queries: Vec<String>,
        key: Option<String>,
    }

    struct FakeProvider {
        replies: BTreeMap<String, Reply>,
        calls: Mutex<Vec<Call>>,
    }

    impl FakeProvider {
        fn new(replies: &[(&str, Reply)]) -> Self {
            Self {
                replies: replies
                    .iter()
                    .map(|(id, reply)| ((*id).into(), reply.clone()))
                    .collect(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl SearchProvider for FakeProvider {
        fn search<'a>(
            &'a self,
            id: &'a str,
            request: &'a WebSearchRequest,
            _config: &'a WebSearchConfig,
            key: Option<&'a str>,
        ) -> SearchFuture<'a> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(Call {
                    id: id.into(),
                    queries: request.queries.clone(),
                    key: key.map(str::to_owned),
                });
                let reply = self
                    .replies
                    .get(id)
                    .unwrap_or_else(|| panic!("Unexpected provider call: {id}"));
                let reply = match reply {
                    Reply::ByQuery(replies) => {
                        replies.get(&request.queries[0]).expect("Unexpected query")
                    }
                    reply => reply,
                };
                match reply {
                    Reply::Results(results) => Ok(results.clone()),
                    Reply::Error(error) => Err((*error).into()),
                    Reply::Pending => std::future::pending().await,
                    Reply::ByQuery(_) => panic!("Nested query replies are unsupported"),
                    Reply::Delayed(delay, results) => {
                        tokio::time::sleep(*delay).await;
                        Ok(results.clone())
                    }
                }
            })
        }
    }

    fn config(enabled: &[&str]) -> WebSearchConfig {
        let mut config = WebSearchConfig::default();
        for (id, runtime) in &mut config.providers {
            runtime.enabled = enabled.contains(&id.as_str());
        }
        config
    }

    fn request(num_results: usize) -> WebSearchRequest {
        WebSearchRequest {
            queries: vec!["rust".into()],
            num_results,
            ..Default::default()
        }
    }

    fn result(url: &str) -> SearchResult {
        SearchResult {
            title: url.into(),
            url: url.into(),
            snippet: "A result".into(),
            ..Default::default()
        }
    }

    fn status(outcome: &WebSearchOutcome, id: &str) -> ProviderStatus {
        outcome
            .reports
            .iter()
            .find(|report| report.provider == id)
            .unwrap()
            .status
    }

    async fn fallback_uses_fused_result_count() {
        let config = config(&["exa", "you"]);
        let provider = FakeProvider::new(&[
            (
                "exa",
                Reply::Results(vec![result("https://a.test"), result("https://b.test")]),
            ),
            ("you", Reply::Results(vec![result("https://c.test")])),
        ]);
        let outcome = execute_with(&request(2), &config, &provider).await.unwrap();
        assert_eq!(outcome.results.len(), 2);
        assert_eq!(status(&outcome, "exa"), ProviderStatus::Ok);
        assert_eq!(status(&outcome, "you"), ProviderStatus::NotApplicable);
        assert!(!outcome.text.contains("Provider notes:"));
        assert_eq!(
            outcome
                .reports
                .iter()
                .find(|r| r.provider == "you")
                .unwrap()
                .detail
                .as_deref(),
            Some("fallback not needed")
        );
        assert_eq!(provider.calls().len(), 1);
        let report = outcome
            .reports
            .iter()
            .find(|r| r.provider == "exa")
            .unwrap();
        assert_eq!(report.results, 2);
        assert_eq!(report.top.len(), 2);
        assert_eq!(report.top[0].title, "https://a.test");
        assert_eq!(report.top[0].url, "https://a.test");
        assert_eq!(
            outcome
                .reports
                .iter()
                .map(|r| r.provider.as_str())
                .collect::<Vec<_>>(),
            provider_catalog().iter().map(|p| p.id).collect::<Vec<_>>()
        );

        let provider = FakeProvider::new(&[
            (
                "exa",
                Reply::Results(vec![
                    result("https://a.test"),
                    result("https://a.test/#duplicate"),
                ]),
            ),
            ("you", Reply::Results(vec![result("https://b.test")])),
        ]);
        let outcome = execute_with(&request(2), &config, &provider).await.unwrap();
        assert_eq!(outcome.results.len(), 2);
        assert_eq!(status(&outcome, "you"), ProviderStatus::Ok);
        assert_eq!(
            provider
                .calls()
                .iter()
                .map(|call| call.id.as_str())
                .collect::<Vec<_>>(),
            ["exa", "you"]
        );
    }

    async fn empty_success_and_failures_are_distinct() {
        let provider = FakeProvider::new(&[("exa", Reply::Results(vec![]))]);
        let outcome = execute_with(&request(1), &config(&["exa"]), &provider)
            .await
            .unwrap();
        assert!(outcome.results.is_empty());
        assert_eq!(status(&outcome, "exa"), ProviderStatus::Empty);
        assert!(outcome
            .reports
            .iter()
            .find(|r| r.provider == "exa")
            .unwrap()
            .detail
            .is_none());

        let provider = FakeProvider::new(&[
            ("exa", Reply::Error("upstream unavailable")),
            ("you", Reply::Error("fallback unavailable")),
        ]);
        let error = execute_with(&request(1), &config(&["exa", "you"]), &provider)
            .await
            .unwrap_err();
        assert!(error.starts_with("Web search failed:"));
        assert!(error.contains("exa: upstream unavailable"));
        assert!(error.contains("you: fallback unavailable"));

        let provider = FakeProvider::new(&[
            ("exa", Reply::Error("upstream unavailable")),
            ("you", Reply::Results(vec![])),
        ]);
        let outcome = execute_with(&request(1), &config(&["exa", "you"]), &provider)
            .await
            .unwrap();
        assert!(outcome.results.is_empty());
        assert_eq!(status(&outcome, "exa"), ProviderStatus::Error);
        assert_eq!(status(&outcome, "you"), ProviderStatus::Empty);

        let provider = FakeProvider::new(&[("exa", Reply::Results(vec![]))]);
        execute_with(&request(1), &config(&["exa"]), &provider)
            .await
            .unwrap();
    }

    async fn inactive_providers_never_call_search() {
        let mut config = config(&["brave", "searxng", "github"]);
        config.providers.get_mut("brave").unwrap().api_key = Some("  ".into());
        config.searxng_url = Some("  ".into());
        let provider = FakeProvider::new(&[]);
        let error = execute_with(&request(1), &config, &provider)
            .await
            .unwrap_err();
        assert!(error.contains("No active search providers"));
        assert!(provider.calls().is_empty());
    }

    async fn multi_query_dispatch_and_validation() {
        let mut config = config(&["parallel", "serper"]);
        config.providers.get_mut("serper").unwrap().api_key = Some("test-key".into());
        let provider = FakeProvider::new(&[
            ("parallel", Reply::Results(vec![])),
            ("serper", Reply::Results(vec![])),
        ]);
        let mut request = request(1);
        request.queries = vec!["  rust   async ".into(), "tokio".into()];
        execute_with(&request, &config, &provider).await.unwrap();
        let calls = provider.calls();
        let parallel = calls
            .iter()
            .filter(|call| call.id == "parallel")
            .collect::<Vec<_>>();
        assert_eq!(parallel.len(), 1);
        assert_eq!(parallel[0].queries, ["rust async", "tokio"]);
        let mut singles = calls
            .iter()
            .filter(|call| call.id == "serper")
            .map(|call| {
                assert_eq!(call.key.as_deref(), Some("test-key"));
                assert_eq!(call.queries.len(), 1);
                call.queries[0].as_str()
            })
            .collect::<Vec<_>>();
        singles.sort();
        assert_eq!(singles, ["rust async", "tokio"]);
        let provider = FakeProvider::new(&[]);
        request.queries = vec!["  ".into()];
        assert!(execute_with(&request, &config, &provider).await.is_err());
        assert!(provider.calls().is_empty());
    }

    async fn query_rank_is_independent_of_completion_order() {
        let config = config(&["exa"]);
        let mut request = request(3);
        request.queries = vec!["first".into(), "second".into()];
        for delayed_query in ["first", "second"] {
            let replies = [
                (
                    "first",
                    vec![result("https://a.test"), result("https://shared.test")],
                ),
                (
                    "second",
                    vec![result("https://b.test"), result("https://shared.test")],
                ),
            ]
            .into_iter()
            .map(|(query, results)| {
                (
                    query.into(),
                    if query == delayed_query {
                        Reply::Delayed(Duration::from_millis(10), results)
                    } else {
                        Reply::Results(results)
                    },
                )
            })
            .collect();
            let provider = FakeProvider::new(&[("exa", Reply::ByQuery(replies))]);
            let outcome = execute_with(&request, &config, &provider).await.unwrap();
            assert_eq!(
                outcome
                    .results
                    .iter()
                    .map(|r| r.url.as_str())
                    .collect::<Vec<_>>(),
                ["https://shared.test", "https://a.test", "https://b.test"]
            );
            let report = outcome
                .reports
                .iter()
                .find(|r| r.provider == "exa")
                .unwrap();
            assert_eq!(report.results, 4);
            assert_eq!(
                report
                    .top
                    .iter()
                    .map(|r| r.url.as_str())
                    .collect::<Vec<_>>(),
                ["https://a.test", "https://shared.test", "https://b.test"]
            );
        }
        for success in [
            Reply::Results(vec![]),
            Reply::Results(vec![result("https://partial.test")]),
        ] {
            let provider = FakeProvider::new(&[(
                "exa",
                Reply::ByQuery(BTreeMap::from([
                    ("first".into(), Reply::Error("query failed")),
                    ("second".into(), success),
                ])),
            )]);
            let outcome = execute_with(&request, &config, &provider).await.unwrap();
            assert_eq!(
                status(&outcome, "exa"),
                if outcome.results.is_empty() {
                    ProviderStatus::Empty
                } else {
                    ProviderStatus::Ok
                }
            );
        }
        let provider = FakeProvider::new(&[(
            "exa",
            Reply::ByQuery(BTreeMap::from([
                ("first".into(), Reply::Error("first failed")),
                ("second".into(), Reply::Error("second failed")),
            ])),
        )]);
        assert!(execute_with(&request, &config, &provider)
            .await
            .unwrap_err()
            .contains("first failed; second failed"));
        let provider = FakeProvider::new(&[("exa", Reply::Results(vec![]))]);
        execute_with(&request, &config, &provider).await.unwrap();
    }

    async fn timeouts_respect_provider_and_global_budgets() {
        let mut config = config(&["tavily", "you"]);
        config.provider_timeout = Duration::from_secs(2);
        config.deadline = Duration::from_secs(10);
        let provider = FakeProvider::new(&[
            ("tavily", Reply::Pending),
            ("you", Reply::Results(vec![result("https://fallback.test")])),
        ]);
        let started = tokio::time::Instant::now();
        let outcome = execute_with(&request(1), &config, &provider).await.unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(2));
        assert_eq!(status(&outcome, "tavily"), ProviderStatus::Timeout);
        assert_eq!(status(&outcome, "you"), ProviderStatus::Ok);
        assert_eq!(
            outcome
                .reports
                .iter()
                .find(|r| r.provider == "tavily")
                .unwrap()
                .detail
                .as_deref(),
            Some("Provider timed out")
        );

        config.provider_timeout = Duration::from_secs(20);
        config.deadline = Duration::from_secs(3);
        let provider = FakeProvider::new(&[
            ("tavily", Reply::Delayed(Duration::from_secs(1), vec![])),
            ("you", Reply::Pending),
        ]);
        let started = tokio::time::Instant::now();
        let outcome = execute_with(&request(1), &config, &provider).await.unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(3));
        assert_eq!(status(&outcome, "tavily"), ProviderStatus::Empty);
        assert_eq!(status(&outcome, "you"), ProviderStatus::Timeout);

        config.deadline = Duration::from_secs(2);
        let provider =
            FakeProvider::new(&[("tavily", Reply::Delayed(Duration::from_secs(1), vec![]))]);
        let started = tokio::time::Instant::now();
        let outcome = execute_with(&request(1), &config, &provider).await.unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(1));
        assert_eq!(status(&outcome, "you"), ProviderStatus::NotApplicable);
        assert_eq!(provider.calls().len(), 1);

        let provider = FakeProvider::new(&[("tavily", Reply::Pending)]);
        let started = tokio::time::Instant::now();
        let error = execute_with(&request(1), &config, &provider)
            .await
            .unwrap_err();
        assert_eq!(started.elapsed(), Duration::from_secs(2));
        assert!(error.contains("tavily: Provider timed out"));
    }

    async fn circuit_skip_does_not_invoke_provider() {
        let mut config = config(&["langsearch"]);
        config.providers.get_mut("langsearch").unwrap().api_key = Some("test-key".into());
        let provider = FakeProvider::new(&[("langsearch", Reply::Error("429 rate limit"))]);
        assert!(execute_with(&request(1), &config, &provider)
            .await
            .unwrap_err()
            .contains("429 rate limit"));
        let provider = FakeProvider::new(&[]);
        let error = execute_with(&request(1), &config, &provider)
            .await
            .unwrap_err();
        assert!(error.contains("No active search providers"));
        assert!(provider.calls().is_empty());
        let BackendAttemptLease::Run(attempt) =
            lease_backend_attempt("langsearch", Instant::now() + Duration::from_secs(301))
        else {
            panic!("Expected recovery probe")
        };
        record_backend_result("langsearch", attempt, &Ok(vec![]));
    }

    #[tokio::test(start_paused = true)]
    async fn injected_orchestration_regressions() {
        fallback_uses_fused_result_count().await;
        empty_success_and_failures_are_distinct().await;
        inactive_providers_never_call_search().await;
        multi_query_dispatch_and_validation().await;
        query_rank_is_independent_of_completion_order().await;
        timeouts_respect_provider_and_global_budgets().await;
        circuit_skip_does_not_invoke_provider().await;
    }

    #[tokio::test(start_paused = true)]
    async fn partial_query_timeout_retains_success_and_trims_key() {
        let mut config = config(&["firecrawl"]);
        config.providers.get_mut("firecrawl").unwrap().api_key = Some(" key \n".into());
        config.provider_timeout = Duration::from_secs(1);
        let provider = FakeProvider::new(&[(
            "firecrawl",
            Reply::ByQuery(BTreeMap::from([
                (
                    "first".into(),
                    Reply::Results(vec![result("https://partial.test")]),
                ),
                ("second".into(), Reply::Pending),
            ])),
        )]);
        let mut request = request(1);
        request.queries = vec!["first".into(), "second".into()];
        let outcome = execute_with(&request, &config, &provider).await.unwrap();
        assert_eq!(outcome.results.len(), 1);
        assert_eq!(status(&outcome, "firecrawl"), ProviderStatus::Ok);
        assert!(outcome
            .reports
            .iter()
            .find(|r| r.provider == "firecrawl")
            .unwrap()
            .detail
            .as_deref()
            .unwrap()
            .contains("Partial"));
        assert!(provider
            .calls()
            .iter()
            .all(|call| call.key.as_deref() == Some("key")));
        assert_eq!(status(&outcome, "you"), ProviderStatus::Disabled);
    }

    #[tokio::test(start_paused = true)]
    async fn global_deadline_does_not_trip_provider_breaker() {
        let mut config = config(&["perplexity"]);
        config.providers.get_mut("perplexity").unwrap().api_key = Some("key".into());
        config.provider_timeout = Duration::from_secs(10);
        config.deadline = Duration::from_secs(1);
        let provider = FakeProvider::new(&[("perplexity", Reply::Pending)]);
        for _ in 0..4 {
            let error = execute_with(&request(1), &config, &provider)
                .await
                .unwrap_err();
            assert!(error.contains("perplexity: Provider timed out"));
        }
        assert_eq!(provider.calls().len(), 4);
    }

    #[tokio::test]
    async fn no_active_provider_is_an_error() {
        let error = execute_with(&request(1), &config(&[]), &FakeProvider::new(&[]))
            .await
            .unwrap_err();
        assert!(error.contains("No active search providers"));
    }

    #[tokio::test]
    #[ignore]
    async fn live_provider_health() {
        for r in test_providers(
            &WebSearchConfig::from_env(),
            "Rust programming language",
            None,
        )
        .await
        {
            println!(
                "{}\t{:?}\t{}\t{}ms\t{}",
                r.provider,
                r.status,
                r.results,
                r.latency_ms,
                r.detail.unwrap_or_default()
            );
        }
    }
}
