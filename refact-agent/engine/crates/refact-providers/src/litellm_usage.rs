use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::litellm_account::{
    fetch_key_info, fetch_self_user, permits_self_user, non_negative, now_rfc3339, safe_text,
    LiteLLMFailure, LiteLLMFeatureState, NativeSession,
};

const MAX_RANGE_DAYS: i64 = 92;
const MAX_PAGE: u32 = 1000;
const MAX_PAGE_SIZE: u32 = 100;
const MAX_USAGE_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_DAYS: usize = 400;
const MAX_MODELS: usize = 200;
const MAX_KEYS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiteLLMUsageScope {
    Key,
    User,
    Team,
}

impl LiteLLMUsageScope {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "key" => Some(Self::Key),
            "user" => Some(Self::User),
            "team" => Some(Self::Team),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiteLLMUsageQuery {
    start: NaiveDate,
    end: NaiveDate,
    scope: LiteLLMUsageScope,
    page: u32,
    page_size: u32,
}

impl LiteLLMUsageQuery {
    pub fn new(
        start: NaiveDate,
        end: NaiveDate,
        scope: LiteLLMUsageScope,
        page: u32,
        page_size: u32,
    ) -> Result<Self, LiteLLMFailure> {
        let invalid =
            |message| LiteLLMFailure::new(LiteLLMFeatureState::MalformedResponse, message);
        if end < start {
            return Err(invalid("End date must not precede start date"));
        }
        if (end - start).num_days() >= MAX_RANGE_DAYS {
            return Err(invalid("Date range is too long"));
        }
        if !(1..=MAX_PAGE).contains(&page) || !(1..=MAX_PAGE_SIZE).contains(&page_size) {
            return Err(invalid("Invalid page or page size"));
        }
        Ok(Self {
            start,
            end,
            scope,
            page,
            page_size,
        })
    }

    pub fn scope(&self) -> LiteLLMUsageScope {
        self.scope
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiteLLMUsageTotalsSource {
    #[default]
    Page,
    GatewaySummary,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LiteLLMUsageTotals {
    pub spend: f64,
    pub requests: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub successful_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_requests: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMUsageDailyModel {
    pub model: String,
    pub spend: f64,
    pub requests: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMUsageDay {
    pub date: String,
    pub spend: f64,
    pub requests: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub models: Vec<LiteLLMUsageDailyModel>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMUsageModel {
    pub model: String,
    pub spend: f64,
    pub requests: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMUsageApiKey {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub current: bool,
    pub spend: f64,
    pub requests: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMUsageReport {
    pub state: LiteLLMFeatureState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub fetched_at: String,
    pub scope: LiteLLMUsageScope,
    pub start: String,
    pub end: String,
    pub page: u32,
    pub page_size: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_pages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    pub truncated: bool,
    pub partial: bool,
    pub totals_source: LiteLLMUsageTotalsSource,
    pub totals: LiteLLMUsageTotals,
    pub daily: Vec<LiteLLMUsageDay>,
    pub models: Vec<LiteLLMUsageModel>,
    pub api_keys: Vec<LiteLLMUsageApiKey>,
}

impl LiteLLMUsageReport {
    pub fn failed(query: &LiteLLMUsageQuery, failure: LiteLLMFailure) -> Self {
        Self {
            state: failure.state,
            message: Some(failure.message),
            fetched_at: now_rfc3339(),
            scope: query.scope,
            start: query.start.to_string(),
            end: query.end.to_string(),
            page: query.page,
            page_size: query.page_size,
            total_pages: None,
            has_more: None,
            truncated: false,
            partial: true,
            totals_source: LiteLLMUsageTotalsSource::Page,
            totals: LiteLLMUsageTotals::default(),
            daily: Vec::new(),
            models: Vec::new(),
            api_keys: Vec::new(),
        }
    }
}

fn count(value: Option<&Value>) -> Option<u64> {
    non_negative(value).map(|n| n.min(u64::MAX as f64) as u64)
}

#[derive(Default, Clone, Copy)]
struct Metrics {
    spend: f64,
    requests: u64,
    prompt: u64,
    completion: u64,
    total: u64,
}

fn malformed(message: &str) -> LiteLLMFailure {
    LiteLLMFailure::new(LiteLLMFeatureState::MalformedResponse, message)
}

fn metrics(object: &Value) -> Result<Metrics, LiteLLMFailure> {
    let m = object
        .get("metrics")
        .filter(|m| m.is_object())
        .ok_or_else(|| malformed("Usage row has no metrics"))?;
    let field = |name: &str| -> Result<Option<u64>, LiteLLMFailure> {
        match m.get(name) {
            None | Some(Value::Null) => Ok(None),
            value => count(value)
                .map(Some)
                .ok_or_else(|| malformed("Usage row has invalid metrics")),
        }
    };
    let spend = match m.get("spend") {
        None | Some(Value::Null) => return Err(malformed("Usage row has no spend")),
        value => non_negative(value).ok_or_else(|| malformed("Usage row has invalid spend"))?,
    };
    let requests =
        field("api_requests")?.ok_or_else(|| malformed("Usage row has no request count"))?;
    let prompt_reported = field("prompt_tokens")?;
    let completion_reported = field("completion_tokens")?;
    let total = match (field("total_tokens")?, prompt_reported, completion_reported) {
        (Some(total), _, _) => total,
        (None, Some(prompt), Some(completion)) => prompt.saturating_add(completion),
        _ => return Err(malformed("Usage row has no token counts")),
    };
    let prompt = prompt_reported.unwrap_or(0);
    let completion = completion_reported.unwrap_or(0);
    Ok(Metrics {
        spend,
        requests,
        prompt,
        completion,
        total,
    })
}

fn add(into: &mut Metrics, from: Metrics) {
    into.spend += from.spend;
    into.requests = into.requests.saturating_add(from.requests);
    into.prompt = into.prompt.saturating_add(from.prompt);
    into.completion = into.completion.saturating_add(from.completion);
    into.total = into.total.saturating_add(from.total);
}

fn breakdown<'a>(row: &'a Value, name: &str) -> Option<&'a Map<String, Value>> {
    row.get("breakdown")?.get(name)?.as_object()
}

pub(crate) fn parse_usage(
    query: &LiteLLMUsageQuery,
    value: &Value,
    own_key_hash: Option<&str>,
) -> Result<LiteLLMUsageReport, LiteLLMFailure> {
    let rows = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            LiteLLMFailure::new(
                LiteLLMFeatureState::MalformedResponse,
                "Missing usage results",
            )
        })?;
    let mut report =
        LiteLLMUsageReport::failed(query, LiteLLMFailure::new(LiteLLMFeatureState::Ok, ""));
    report.message = None;
    let mut truncated = rows.len() > MAX_DAYS;
    let mut totals = Metrics::default();
    let mut models: BTreeMap<String, Metrics> = BTreeMap::new();
    let mut keys: BTreeMap<String, (Option<String>, bool, Metrics)> = BTreeMap::new();
    for row in rows.iter().take(MAX_DAYS) {
        let date = row
            .get("date")
            .and_then(Value::as_str)
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .ok_or_else(|| malformed("Usage row has an invalid date"))?;
        if date < query.start || date > query.end {
            return Err(malformed("Usage row is outside the requested range"));
        }
        let day = metrics(row)?;
        add(&mut totals, day);
        let mut day_models = Vec::new();
        for (name, entry) in breakdown(row, "models").into_iter().flatten() {
            let Some(model) = safe_text(Some(&Value::String(name.clone()))) else {
                continue;
            };
            let m = metrics(entry)?;
            if models.len() < MAX_MODELS || models.contains_key(&model) {
                add(models.entry(model.clone()).or_default(), m);
            } else {
                truncated = true;
            }
            if day_models.len() < MAX_MODELS {
                day_models.push(LiteLLMUsageDailyModel {
                    model,
                    spend: m.spend,
                    requests: m.requests,
                    total_tokens: m.total,
                });
            }
        }
        for (hash, entry) in breakdown(row, "api_keys").into_iter().flatten() {
            let current = own_key_hash == Some(hash.as_str());
            let alias = entry
                .get("metadata")
                .and_then(|m| safe_text(m.get("key_alias")));
            if keys.len() >= MAX_KEYS && !keys.contains_key(hash) {
                truncated = true;
                continue;
            }
            let slot =
                keys.entry(hash.clone())
                    .or_insert((alias.clone(), current, Metrics::default()));
            if slot.0.is_none() {
                slot.0 = alias;
            }
            add(&mut slot.2, metrics(entry)?);
        }
        report.daily.push(LiteLLMUsageDay {
            date: date.to_string(),
            spend: day.spend,
            requests: day.requests,
            prompt_tokens: day.prompt,
            completion_tokens: day.completion,
            total_tokens: day.total,
            models: day_models,
        });
    }
    report.daily.sort_by(|a, b| a.date.cmp(&b.date));
    let meta = value.get("metadata").filter(|m| m.is_object());
    let meta_get = |name| meta.and_then(|m| m.get(name));
    let summary = (
        non_negative(meta_get("total_spend")),
        count(meta_get("total_api_requests")),
        count(meta_get("total_tokens")),
    );
    report.totals = match summary {
        (Some(spend), Some(requests), Some(total_tokens)) => {
            report.totals_source = LiteLLMUsageTotalsSource::GatewaySummary;
            LiteLLMUsageTotals {
                spend,
                requests,
                prompt_tokens: count(meta_get("total_prompt_tokens")).unwrap_or(totals.prompt),
                completion_tokens: count(meta_get("total_completion_tokens"))
                    .unwrap_or(totals.completion),
                total_tokens,
                successful_requests: count(meta_get("total_successful_requests")),
                failed_requests: count(meta_get("total_failed_requests")),
            }
        }
        _ => LiteLLMUsageTotals {
            spend: totals.spend,
            requests: totals.requests,
            prompt_tokens: totals.prompt,
            completion_tokens: totals.completion,
            total_tokens: totals.total,
            successful_requests: None,
            failed_requests: None,
        },
    };
    let page_field = |name| meta_get(name).or_else(|| value.get(name));
    report.total_pages = count(page_field("total_pages")).map(|n| n.min(u32::MAX as u64) as u32);
    report.has_more = page_field("has_more").and_then(Value::as_bool);
    if let Some(page) = count(page_field("page")) {
        if page != query.page as u64 {
            return Err(malformed("Usage page does not match the request"));
        }
    }
    report.models = models
        .into_iter()
        .map(|(model, m)| LiteLLMUsageModel {
            model,
            spend: m.spend,
            requests: m.requests,
            prompt_tokens: m.prompt,
            completion_tokens: m.completion,
            total_tokens: m.total,
        })
        .collect();
    let mut other_index = 0;
    report.api_keys = keys
        .into_values()
        .map(|(alias, current, m)| {
            let label = match (&alias, current) {
                (Some(alias), _) => alias.clone(),
                (None, true) => "Current API key".to_string(),
                (None, false) => {
                    other_index += 1;
                    format!("Key {other_index}")
                }
            };
            LiteLLMUsageApiKey {
                label,
                alias,
                current,
                spend: m.spend,
                requests: m.requests,
                total_tokens: m.total,
            }
        })
        .collect();
    let page_complete = report.has_more == Some(false)
        || report.total_pages.is_some_and(|pages| pages <= query.page);
    report.truncated = truncated;
    report.partial = truncated || query.page > 1 || !page_complete;
    Ok(report)
}

pub(crate) async fn fetch_usage(
    session: &mut NativeSession,
    query: &LiteLLMUsageQuery,
) -> LiteLLMUsageReport {
    let identity = match fetch_key_info(session).await {
        Ok((identity, _)) => identity,
        Err(_) if query.scope != LiteLLMUsageScope::Key && permits_self_user(session) => {
            match fetch_self_user(session).await {
                Ok((identity, _)) => identity,
                Err(failure) => return LiteLLMUsageReport::failed(query, failure),
            }
        }
        Err(failure) => return LiteLLMUsageReport::failed(query, failure),
    };
    let not_associated = |what| {
        LiteLLMUsageReport::failed(
            query,
            LiteLLMFailure::new(LiteLLMFeatureState::NotConfigured, what),
        )
    };
    let mut params = vec![
        ("start_date", query.start.to_string()),
        ("end_date", query.end.to_string()),
        ("page", query.page.to_string()),
        ("page_size", query.page_size.to_string()),
    ];
    let path = match query.scope {
        LiteLLMUsageScope::Key => {
            let Some(hash) = identity.token_hash.clone() else {
                return not_associated("Key identity is unavailable");
            };
            params.push(("api_key", hash));
            if let Some(user_id) = identity.user_id.clone() {
                params.push(("user_id", user_id));
            }
            "/user/daily/activity"
        }
        LiteLLMUsageScope::User => {
            let Some(user_id) = identity.user_id.clone() else {
                return not_associated("Key is not associated with a user");
            };
            params.push(("user_id", user_id));
            "/user/daily/activity"
        }
        LiteLLMUsageScope::Team => {
            let Some(team_id) = identity.team_id.clone() else {
                return not_associated("Key is not associated with a team");
            };
            params.push(("team_ids", team_id));
            "/team/daily/activity"
        }
    };
    match session.get_json(path, &params, MAX_USAGE_BODY_BYTES).await {
        Ok(value) => parse_usage(query, &value, identity.token_hash.as_deref())
            .unwrap_or_else(|failure| LiteLLMUsageReport::failed(query, failure)),
        Err(failure) => LiteLLMUsageReport::failed(query, failure),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::litellm::LiteLLMProvider;
    use crate::litellm_account::test_server::{json, request_line, serve};
    use serde_json::json as j;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn query(scope: LiteLLMUsageScope) -> LiteLLMUsageQuery {
        LiteLLMUsageQuery::new(date("2026-01-01"), date("2026-01-31"), scope, 1, 50).unwrap()
    }

    fn missing_key() -> crate::litellm_account::test_server::Reply {
        json(
            "404 Not Found",
            j!({"error": {"type": "not_found_error",
            "message": "Key not found in database", "code": "404"}})
            .to_string(),
        )
    }

    fn self_user(team: Value) -> crate::litellm_account::test_server::Reply {
        json(
            "200 OK",
            j!({"user_id": "self-user", "user_info": {
            "user_id": "self-user", "team_id": team},
            "keys": [{"token": "listed-secret", "team_id": "unproven-team"}],
            "teams": []})
            .to_string(),
        )
    }

    #[tokio::test]
    async fn jwt_key_usage_refuses_before_self_or_activity_request() {
        let (base, server) = serve(vec![missing_key()]).await;
        let p = LiteLLMProvider {
            endpoint: base,
            api_key: "synthetic-jwt-bearer".into(),
            ..Default::default()
        };
        let report = p
            .fetch_usage("jwt-key", &query(LiteLLMUsageScope::Key))
            .await;
        assert_eq!(report.state, LiteLLMFeatureState::Unavailable);
        assert_eq!(report.scope, LiteLLMUsageScope::Key);
        assert_eq!(server.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn jwt_explicit_user_usage_is_narrowed_without_api_key() {
        let (base, server) = serve(vec![
            missing_key(),
            self_user(Value::Null),
            json("200 OK", activity().to_string()),
        ])
        .await;
        let p = LiteLLMProvider {
            endpoint: base,
            api_key: "synthetic-jwt-bearer".into(),
            ..Default::default()
        };
        let report = p
            .fetch_usage("jwt-user", &query(LiteLLMUsageScope::User))
            .await;
        assert_eq!(report.state, LiteLLMFeatureState::Ok);
        assert_eq!(report.scope, LiteLLMUsageScope::User);
        assert!(report.api_keys.iter().all(|k| !k.current));
        let requests = server.await.unwrap();
        assert_eq!(request_line(&requests[1]), "GET /user/info HTTP/1.1");
        assert_eq!(request_line(&requests[2]), "GET /user/daily/activity?start_date=2026-01-01&end_date=2026-01-31&page=1&page_size=50&user_id=self-user HTTP/1.1");
        assert!(requests.iter().all(
            |r| crate::litellm_account::test_server::bearer(r).as_deref()
                == Some("synthetic-jwt-bearer")
        ));
        let output = serde_json::to_string(&report).unwrap();
        for secret in [
            "synthetic-jwt-bearer",
            "listed-secret",
            "own-hash",
            "other-hash",
        ] {
            assert!(!output.contains(secret));
        }
    }

    #[tokio::test]
    async fn jwt_team_usage_requires_server_returned_association() {
        for team in [Value::Null, j!("associated-team")] {
            let associated = team.is_string();
            let mut replies = vec![missing_key(), self_user(team)];
            if associated {
                replies.push(json("200 OK", r#"{"results": []}"#));
            }
            let (base, server) = serve(replies).await;
            let p = LiteLLMProvider {
                endpoint: base,
                api_key: "synthetic-jwt-bearer".into(),
                ..Default::default()
            };
            let report = p
                .fetch_usage("jwt-team", &query(LiteLLMUsageScope::Team))
                .await;
            let requests = server.await.unwrap();
            if associated {
                assert_eq!(report.state, LiteLLMFeatureState::Ok);
                assert!(request_line(&requests[2]).contains("team_ids=associated-team"));
            } else {
                assert_eq!(report.state, LiteLLMFeatureState::NotConfigured);
                assert_eq!(requests.len(), 2);
            }
        }
    }

    #[tokio::test]
    async fn jwt_usage_refuses_invalid_identity_and_does_not_fallback_on_auth_or_other_404() {
        let (base, server) = serve(vec![
            missing_key(),
            json(
                "200 OK",
                r#"{"user_id":"u","user_info":{"user_id":"other"}}"#,
            ),
        ])
        .await;
        let p = LiteLLMProvider {
            endpoint: base,
            api_key: "synthetic-jwt-bearer".into(),
            ..Default::default()
        };
        assert_eq!(
            p.fetch_usage("jwt-invalid", &query(LiteLLMUsageScope::User))
                .await
                .state,
            LiteLLMFeatureState::MalformedResponse
        );
        assert_eq!(server.await.unwrap().len(), 2);
        for (status, state) in [
            ("401 Unauthorized", LiteLLMFeatureState::AuthRequired),
            ("403 Forbidden", LiteLLMFeatureState::PermissionDenied),
            ("404 Not Found", LiteLLMFeatureState::Unsupported),
        ] {
            let (base, server) = serve(vec![json(status, "{}")]).await;
            let p = LiteLLMProvider {
                endpoint: base,
                api_key: "synthetic-jwt-bearer".into(),
                ..Default::default()
            };
            assert_eq!(
                p.fetch_usage("jwt-errors", &query(LiteLLMUsageScope::User))
                    .await
                    .state,
                state
            );
            assert_eq!(server.await.unwrap().len(), 1);
        }
    }

    #[test]
    fn query_bounds_are_enforced() {
        let s = date("2026-01-01");
        assert!(
            LiteLLMUsageQuery::new(s, date("2025-12-31"), LiteLLMUsageScope::Key, 1, 10).is_err()
        );
        assert!(
            LiteLLMUsageQuery::new(s, date("2026-06-01"), LiteLLMUsageScope::Key, 1, 10).is_err()
        );
        assert!(LiteLLMUsageQuery::new(s, s, LiteLLMUsageScope::Key, 0, 10).is_err());
        assert!(LiteLLMUsageQuery::new(s, s, LiteLLMUsageScope::Key, 1, 101).is_err());
        assert!(LiteLLMUsageQuery::new(s, s, LiteLLMUsageScope::Key, 1000, 100).is_ok());
    }

    fn activity() -> Value {
        j!({
            "results": [
                {"date": "2026-01-02", "metrics": {"spend": 1.5, "api_requests": 3,
                    "prompt_tokens": 100, "completion_tokens": 50, "total_tokens": 150},
                 "breakdown": {
                    "models": {"gpt-4o": {"metrics": {"spend": 1.5, "api_requests": 3, "total_tokens": 150}}},
                    "api_keys": {
                        "own-hash": {"metrics": {"spend": 1.0, "api_requests": 2, "total_tokens": 100},
                                     "metadata": {"key_alias": null}},
                        "other-hash": {"metrics": {"spend": 0.5, "api_requests": 1, "total_tokens": 50},
                                       "metadata": {"key_alias": "ci"}}
                    }},
                 "messages": [{"content": "secret prompt"}]},
                {"date": "2026-01-01", "metrics": {"spend": 0.5, "api_requests": 1,
                    "prompt_tokens": 10, "completion_tokens": 5}}
            ],
            "metadata": {"total_spend": 2.0, "total_api_requests": 4, "total_prompt_tokens": 110,
                "total_completion_tokens": 55, "total_tokens": 165,
                "total_successful_requests": 3, "total_failed_requests": 1,
                "page": 1, "total_pages": 2, "has_more": true}
        })
    }

    #[test]
    fn aggregates_safely_without_hashes_or_prompts() {
        let report = parse_usage(
            &query(LiteLLMUsageScope::Key),
            &activity(),
            Some("own-hash"),
        )
        .unwrap();
        assert_eq!(report.state, LiteLLMFeatureState::Ok);
        assert_eq!(report.totals.spend, 2.0);
        assert_eq!(
            report.totals_source,
            LiteLLMUsageTotalsSource::GatewaySummary
        );
        assert!(report.partial);
        assert_eq!(report.totals.failed_requests, Some(1));
        assert_eq!(report.total_pages, Some(2));
        assert_eq!(report.has_more, Some(true));
        assert_eq!(report.daily[0].date, "2026-01-01");
        assert_eq!(report.daily[0].total_tokens, 15);
        assert_eq!(report.models[0].model, "gpt-4o");
        let current = report.api_keys.iter().find(|k| k.current).unwrap();
        assert_eq!(current.label, "Current API key");
        assert!(report
            .api_keys
            .iter()
            .any(|k| k.label == "ci" && !k.current));
        let serialized = serde_json::to_string(&report).unwrap();
        assert!(!serialized.contains("own-hash") && !serialized.contains("other-hash"));
        assert!(!serialized.contains("secret prompt"));
        assert_eq!(
            parse_usage(&query(LiteLLMUsageScope::Key), &j!({}), None)
                .unwrap_err()
                .state,
            LiteLLMFeatureState::MalformedResponse
        );
    }

    #[test]
    fn malformed_rows_and_out_of_range_dates_are_refused() {
        let q = query(LiteLLMUsageScope::Key);
        for bad in [
            j!({"results": [{"date": "2026-01-02"}]}),
            j!({"results": [{"date": "2026-01-02", "metrics": {"spend": 1, "api_requests": 1}}]}),
            j!({"results": [{"date": "2026-01-02", "metrics": {"spend": null, "api_requests": 1, "total_tokens": 1}}]}),
            j!({"results": [{"date": "2026-01-02", "metrics": {"spend": 1, "api_requests": null, "total_tokens": 1}}]}),
            j!({"results": [{"date": "2026-01-02", "metrics": {"spend": 1, "api_requests": 1, "total_tokens": 1},
                "breakdown": {"models": {"m": {"metrics": {"spend": 1}}}}}]}),
            j!({"results": [{"date": "2026-01-02", "metrics": {"spend": "abc"}}]}),
            j!({"results": [{"date": "2026-01-02", "metrics": {"api_requests": -1}}]}),
            j!({"results": [{"date": "not-a-date", "metrics": {}}]}),
            j!({"results": [{"date": "2025-12-31", "metrics": {"spend": 1}}]}),
            j!({"results": [{"date": "2026-01-02", "metrics": {},
                "breakdown": {"models": {"m": {"metrics": "x"}}}}]}),
            j!({"results": [], "metadata": {"page": 2}}),
        ] {
            assert_eq!(
                parse_usage(&q, &bad, None).unwrap_err().state,
                LiteLLMFeatureState::MalformedResponse,
                "{bad}"
            );
        }
        let empty = parse_usage(&q, &j!({"results": []}), None).unwrap();
        assert_eq!(empty.totals.spend, 0.0);
        assert!(empty.daily.is_empty());
        assert_eq!(empty.totals_source, LiteLLMUsageTotalsSource::Page);
        assert!(empty.partial);
        let full = parse_usage(
            &q,
            &j!({"results": [{"date": "2026-01-02", "metrics": {"spend": 1, "api_requests": 1,
                "prompt_tokens": 2, "completion_tokens": 3}}],
                "metadata": {"page": 1, "total_pages": 1, "has_more": false}}),
            None,
        )
        .unwrap();
        assert!(!full.partial && !full.truncated);
        assert_eq!(full.totals.total_tokens, 5);
        assert_eq!(full.totals_source, LiteLLMUsageTotalsSource::Page);
    }

    #[test]
    fn pagination_reads_native_metadata_and_top_level_shapes() {
        let q = query(LiteLLMUsageScope::Key);
        let nested = parse_usage(
            &q,
            &j!({"results": [], "metadata": {"page": 1, "total_pages": 3, "has_more": true}}),
            None,
        )
        .unwrap();
        assert_eq!((nested.total_pages, nested.has_more), (Some(3), Some(true)));
        let top = parse_usage(
            &q,
            &j!({"results": [], "page": 1, "total_pages": 1, "has_more": false}),
            None,
        )
        .unwrap();
        assert_eq!((top.total_pages, top.has_more), (Some(1), Some(false)));
    }

    #[tokio::test]
    async fn key_scope_uses_own_hash_and_user_id() {
        let (base, server) = serve(vec![
            json(
                "200 OK",
                j!({"info": {"token": "own-hash", "user_id": "u1", "team_id": "t1"}}).to_string(),
            ),
            json("200 OK", activity().to_string()),
        ])
        .await;
        let p = LiteLLMProvider {
            endpoint: base,
            api_key: "sk-a".into(),
            ..Default::default()
        };
        let report = p
            .fetch_usage("litellm", &query(LiteLLMUsageScope::Key))
            .await;
        let requests = server.await.unwrap();
        assert_eq!(report.state, LiteLLMFeatureState::Ok);
        assert_eq!(
            request_line(&requests[1]),
            "GET /user/daily/activity?start_date=2026-01-01&end_date=2026-01-31&page=1&page_size=50&api_key=own-hash&user_id=u1 HTTP/1.1"
        );
    }

    #[tokio::test]
    async fn team_scope_uses_only_associated_team() {
        let (base, server) = serve(vec![
            json(
                "200 OK",
                j!({"info": {"token": "h", "team_id": "t1"}}).to_string(),
            ),
            json("403 Forbidden", "{}"),
        ])
        .await;
        let p = LiteLLMProvider {
            endpoint: base,
            api_key: "sk-a".into(),
            ..Default::default()
        };
        let report = p
            .fetch_usage("litellm", &query(LiteLLMUsageScope::Team))
            .await;
        let requests = server.await.unwrap();
        assert!(request_line(&requests[1]).starts_with("GET /team/daily/activity?"));
        assert!(request_line(&requests[1]).contains("team_ids=t1"));
        assert_eq!(report.state, LiteLLMFeatureState::PermissionDenied);
    }

    #[tokio::test]
    async fn user_scope_without_association_makes_no_activity_request() {
        let (base, server) = serve(vec![json(
            "200 OK",
            j!({"info": {"token": "h"}}).to_string(),
        )])
        .await;
        let p = LiteLLMProvider {
            endpoint: base,
            api_key: "sk-a".into(),
            ..Default::default()
        };
        let report = p
            .fetch_usage("litellm", &query(LiteLLMUsageScope::User))
            .await;
        assert_eq!(server.await.unwrap().len(), 1);
        assert_eq!(report.state, LiteLLMFeatureState::NotConfigured);
    }
}
