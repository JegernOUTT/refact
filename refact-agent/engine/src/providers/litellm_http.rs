use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::providers::litellm::LiteLLMProvider;
use crate::providers::quota::{
    ProviderQuotaCache, ProviderQuotaFact, ProviderQuotaFactValue, ProviderQuotaSnapshot,
    ProviderQuotaSource, ProviderQuotaWindow,
};

const CACHE_TTL: Duration = Duration::from_secs(60);
const CACHE_CAPACITY: usize = 100;

#[derive(Clone)]
struct AccountEntry {
    configuration: String,
    identity: String,
    value: Value,
    inserted: Instant,
}

type AccountCache = HashMap<(PathBuf, String), AccountEntry>;

fn account_cache() -> &'static Mutex<AccountCache> {
    static CACHE: OnceLock<Mutex<AccountCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn configuration(provider: &LiteLLMProvider) -> Result<String, ScratchError> {
    let bytes = serde_json::to_vec(provider).map_err(|_| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "LiteLLM configuration is unavailable".to_string(),
        )
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub(super) async fn resolve_provider(
    gcx: &Arc<GlobalContext>,
    instance: &str,
) -> Result<LiteLLMProvider, ScratchError> {
    refact_providers::identity::validate_provider_instance_id(instance).map_err(|_| {
        ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Invalid provider instance".to_string(),
        )
    })?;
    let registry = gcx.providers.read().await;
    let provider = registry.get(instance).ok_or_else(|| {
        ScratchError::new(
            StatusCode::NOT_FOUND,
            "LiteLLM provider is not configured".to_string(),
        )
    })?;
    if provider.base_provider_name() != "litellm" {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Provider is not a LiteLLM instance".to_string(),
        ));
    }
    provider
        .as_any()
        .downcast_ref::<LiteLLMProvider>()
        .cloned()
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "LiteLLM provider is unavailable".to_string(),
            )
        })
}

fn unavailable(state: &str, message: &str) -> Value {
    json!({"state": state, "message": message, "fetched_at": Utc::now().to_rfc3339(), "stale": false, "subjects": []})
}

async fn identity_is_current(
    gcx: &Arc<GlobalContext>,
    instance: &str,
    revision: &str,
    fingerprint: &str,
) -> bool {
    let Ok(current) = resolve_provider(gcx, instance).await else {
        return false;
    };
    configuration(&current).is_ok_and(|value| value == revision)
        && current
            .identity_fingerprint(instance)
            .await
            .is_ok_and(|value| value == fingerprint)
}

pub(super) fn invalidate(gcx: &Arc<GlobalContext>, instance: &str) {
    if let Ok(mut cache) = account_cache().lock() {
        cache.remove(&(gcx.config_dir.clone(), instance.to_string()));
    }
    super::litellm_observed::invalidate(&gcx.config_dir, instance);
}

pub(super) async fn account(
    gcx: &Arc<GlobalContext>,
    instance: &str,
    refresh: bool,
) -> Result<Value, ScratchError> {
    let provider = resolve_provider(gcx, instance).await?;
    let revision = configuration(&provider)?;
    let fingerprint = match provider.identity_fingerprint(instance).await {
        Ok(value) => value,
        Err(error) => {
            return Ok(unavailable(
                serde_json::to_value(error.state)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .as_deref()
                    .unwrap_or("unavailable"),
                &error.message,
            ))
        }
    };
    let key = (gcx.config_dir.clone(), instance.to_string());
    let prior = account_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
        .filter(|entry| entry.configuration == revision && entry.identity == fingerprint);
    if !refresh {
        if let Some(entry) = prior
            .as_ref()
            .filter(|entry| entry.inserted.elapsed() < CACHE_TTL)
        {
            let mut value = entry.value.clone();
            attach_observed(&provider, instance, gcx, &mut value).await;
            if !identity_is_current(gcx, instance, &revision, &fingerprint).await {
                return Ok(unavailable(
                    "unavailable",
                    "LiteLLM identity changed while loading account data; refresh to retry",
                ));
            }
            return Ok(value);
        }
    }
    let snapshot = provider.fetch_account(instance).await;
    let current = resolve_provider(gcx, instance).await?;
    if configuration(&current)? != revision
        || current.identity_fingerprint(instance).await.ok().as_deref()
            != Some(fingerprint.as_str())
    {
        return Ok(unavailable(
            "unavailable",
            "LiteLLM identity changed while loading account data; refresh to retry",
        ));
    }
    let mut value = serde_json::to_value(snapshot).map_err(|_| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "LiteLLM account response is unavailable".to_string(),
        )
    })?;
    if value.get("state").and_then(Value::as_str) == Some("unavailable") {
        if let Some(entry) =
            prior.filter(|entry| entry.value.get("state").and_then(Value::as_str) == Some("ok"))
        {
            let message = value.get("message").cloned();
            value = entry.value;
            value["stale"] = json!(true);
            value["message"] =
                message.unwrap_or_else(|| json!("LiteLLM account refresh is unavailable"));
        }
    } else if let Ok(mut cache) = account_cache().lock() {
        if cache.len() >= CACHE_CAPACITY && !cache.contains_key(&key) {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.inserted)
                .map(|(key, _)| key.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(
            key,
            AccountEntry {
                configuration: revision.clone(),
                identity: fingerprint.clone(),
                value: value.clone(),
                inserted: Instant::now(),
            },
        );
    }
    attach_observed(&provider, instance, gcx, &mut value).await;
    if !identity_is_current(gcx, instance, &revision, &fingerprint).await {
        return Ok(unavailable(
            "unavailable",
            "LiteLLM identity changed while loading account data; refresh to retry",
        ));
    }
    Ok(value)
}

async fn attach_observed(
    provider: &LiteLLMProvider,
    instance: &str,
    gcx: &Arc<GlobalContext>,
    value: &mut Value,
) {
    let Ok(base) = provider.validated_base_url() else {
        return;
    };
    let Ok(bearer) = provider.resolve_bearer(instance).await else {
        return;
    };
    let identity =
        super::litellm_observed::identity(&base, bearer.expose(), &provider.extra_headers);
    if let Some(sample) = super::litellm_observed::get(&gcx.config_dir, instance, &identity) {
        if let Ok(sample) = serde_json::to_value(sample) {
            value["observed_headroom"] = sample;
        }
    }
}

#[derive(Default, Deserialize)]
pub struct AccountQuery {
    #[serde(default)]
    refresh: bool,
}

pub async fn get_account(
    State(app): State<AppState>,
    Path(instance): Path<String>,
    Query(query): Query<AccountQuery>,
) -> Result<Json<Value>, ScratchError> {
    account(&app.gcx, &instance, query.refresh).await.map(Json)
}

#[derive(Default, Deserialize)]
pub struct UsageQuery {
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
    scope: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
}

pub(super) async fn usage(
    gcx: &Arc<GlobalContext>,
    instance: &str,
    query: UsageQuery,
) -> Result<Value, ScratchError> {
    use refact_providers::litellm_usage::{LiteLLMUsageQuery, LiteLLMUsageScope};
    let today = Utc::now().date_naive();
    let scope = match query.scope.as_deref().unwrap_or("key") {
        "key" => LiteLLMUsageScope::Key,
        "user" => LiteLLMUsageScope::User,
        "team" => LiteLLMUsageScope::Team,
        _ => {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                "Invalid LiteLLM usage scope".to_string(),
            ))
        }
    };
    let query = LiteLLMUsageQuery::new(
        query.start.unwrap_or(today - chrono::TimeDelta::days(6)),
        query.end.unwrap_or(today),
        scope,
        query.page.unwrap_or(1),
        query.page_size.unwrap_or(50),
    )
    .map_err(|error| ScratchError::new(StatusCode::BAD_REQUEST, error.message))?;
    let provider = resolve_provider(gcx, instance).await?;
    let revision = configuration(&provider)?;
    let identity = provider.identity_fingerprint(instance).await.ok();
    let report = provider.fetch_usage(instance, &query).await;
    let current = resolve_provider(gcx, instance).await?;
    if revision != configuration(&current)?
        || identity != current.identity_fingerprint(instance).await.ok()
    {
        return Err(ScratchError::new(
            StatusCode::CONFLICT,
            "LiteLLM identity changed while loading usage".to_string(),
        ));
    }
    serde_json::to_value(report).map_err(|_| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "LiteLLM usage response is unavailable".to_string(),
        )
    })
}

pub async fn get_usage(
    State(app): State<AppState>,
    Path(instance): Path<String>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<Value>, ScratchError> {
    usage(&app.gcx, &instance, query).await.map(Json)
}

pub(super) fn quota(instance: &str, details: Value) -> ProviderQuotaSnapshot {
    let ok = details.get("state").and_then(Value::as_str) == Some("ok");
    let subject = details
        .get("subjects")
        .and_then(Value::as_array)
        .filter(|_| ok)
        .and_then(|subjects| {
            let scope = |subject: &Value, expected: &str| {
                subject.get("scope").and_then(Value::as_str) == Some(expected)
            };
            let state = |subject: &Value, expected: &str| {
                subject.get("state").and_then(Value::as_str) == Some(expected)
            };
            subjects
                .iter()
                .find(|subject| scope(subject, "key") && state(subject, "ok"))
                .or_else(|| {
                    subjects
                        .iter()
                        .filter(|subject| scope(subject, "key"))
                        .all(|subject| state(subject, "unavailable"))
                        .then(|| {
                            subjects
                                .iter()
                                .find(|subject| scope(subject, "user") && state(subject, "ok"))
                        })
                        .flatten()
                })
        });
    let available = subject.is_some();
    let mut facts = Vec::new();
    let mut windows = Vec::new();
    if let Some(subject) = subject {
        let is_user = subject.get("scope").and_then(Value::as_str) == Some("user");
        let spend = subject.pointer("/spend/value").and_then(Value::as_f64);
        let budget = subject.pointer("/max_budget/value").and_then(Value::as_f64);
        let remaining = subject.pointer("/remaining/value").and_then(Value::as_f64);
        for (name, value) in [
            ("spend", spend),
            ("max_budget", budget),
            ("remaining", remaining),
        ] {
            if let Some(number) = value.and_then(serde_json::Number::from_f64) {
                facts.push(ProviderQuotaFact {
                    id: name.to_string(),
                    label: name.replace('_', " "),
                    value: ProviderQuotaFactValue::Number(number),
                    unit: Some("USD".to_string()),
                });
            }
        }
        facts.push(ProviderQuotaFact {
            id: "scope".to_string(),
            label: "Quota scope".to_string(),
            value: ProviderQuotaFactValue::String(
                if is_user {
                    "Configured LiteLLM user"
                } else {
                    "Configured LiteLLM key"
                }
                .to_string(),
            ),
            unit: None,
        });
        for name in [
            "budget_reset_at",
            "expires_at",
            "blocked",
            "rpm_limit",
            "tpm_limit",
        ] {
            let Some(value) = subject.get(name) else {
                continue;
            };
            if let Ok(value) = serde_json::from_value(value.clone()) {
                facts.push(ProviderQuotaFact {
                    id: name.to_string(),
                    label: name.replace('_', " "),
                    value,
                    unit: None,
                });
            }
        }
        if subject.pointer("/max_budget/kind").and_then(Value::as_str) == Some("unlimited") {
            facts.push(ProviderQuotaFact {
                id: "max_budget".to_string(),
                label: "Budget".to_string(),
                value: ProviderQuotaFactValue::String("Unlimited".to_string()),
                unit: None,
            });
        }
        if let Some(limit) = budget {
            windows.push(ProviderQuotaWindow {
                id: if is_user { "user_budget" } else { "key_budget" }.to_string(),
                label: if is_user { "User budget" } else { "Key budget" }.to_string(),
                used_percent: spend.and_then(|used| {
                    (limit > 0.0).then_some((used / limit * 100.0).clamp(0.0, 100.0))
                }),
                limit: Some(limit),
                used: spend,
                remaining,
                reset_at: subject
                    .get("budget_reset_at")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                reset_after_seconds: None,
                window_seconds: None,
                status: subject
                    .pointer("/remaining/overage")
                    .and_then(Value::as_bool)
                    .filter(|overage| *overage)
                    .map(|_| "over_budget".to_string()),
            });
        }
    }
    if let Some(observed) = details.get("observed_headroom") {
        for name in [
            "observed_at",
            "expires_at",
            "model",
            "requests_remaining",
            "tokens_remaining",
            "requests_limit",
            "tokens_limit",
            "requests_reset_at",
            "tokens_reset_at",
        ] {
            let Some(value) = observed.get(name) else {
                continue;
            };
            if let Ok(value) = serde_json::from_value(value.clone()) {
                facts.push(ProviderQuotaFact {
                    id: format!("observed_{name}"),
                    label: format!("Observed {}", name.replace('_', " ")),
                    value,
                    unit: None,
                });
            }
        }
        facts.push(ProviderQuotaFact {
            id: "observed_scope".to_string(),
            label: "Headroom source".to_string(),
            value: ProviderQuotaFactValue::String(
                "Last inference response; key or upstream scope unknown".to_string(),
            ),
            unit: None,
        });
    }
    ProviderQuotaSnapshot {
        provider_name: instance.to_string(),
        base_provider: "litellm".to_string(),
        source: ProviderQuotaSource::Litellm,
        available,
        error: details
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned),
        plan: None,
        windows,
        facts,
        fetched_at: details
            .get("fetched_at")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(ProviderQuotaCache::now_iso_timestamp),
        stale: details
            .get("stale")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        litellm: Some(details),
    }
}

#[cfg(test)]
#[path = "litellm_http_tests.rs"]
mod route_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwt_user_budget_is_selected_and_details_are_preserved() {
        let details = json!({"state":"ok","subjects":[
            {"scope":"key","state":"unavailable","spend":{"value":999.0}},
            {"scope":"user","state":"ok","spend":{"value":75.0},"max_budget":{"kind":"limited","value":300.0},"remaining":{"value":225.0}}
        ]});
        let snapshot = quota("jwt", details.clone());
        assert!(snapshot.available);
        assert_eq!(snapshot.windows.len(), 1);
        let window = &snapshot.windows[0];
        assert_eq!(window.id, "user_budget");
        assert_eq!(window.label, "User budget");
        assert_eq!(window.limit, Some(300.0));
        assert_eq!(window.used, Some(75.0));
        assert_eq!(window.remaining, Some(225.0));
        assert_eq!(window.used_percent, Some(25.0));
        assert!(snapshot.facts.iter().any(|fact| fact.id == "scope"
            && fact.value
                == ProviderQuotaFactValue::String("Configured LiteLLM user".to_string())));
        assert_eq!(snapshot.litellm, Some(details));
    }

    #[test]
    fn successful_key_takes_precedence_over_user_without_summing() {
        let snapshot = quota(
            "key",
            json!({"state":"ok","subjects":[
                {"scope":"user","state":"ok","spend":{"value":75.0},"max_budget":{"value":300.0}},
                {"scope":"key","state":"permission_denied","spend":{"value":999.0}},
                {"scope":"key","state":"ok","spend":{"value":2.0},"max_budget":{"value":10.0}}
            ]}),
        );
        assert!(snapshot.available);
        assert_eq!(snapshot.windows.len(), 1);
        assert_eq!(snapshot.windows[0].id, "key_budget");
        assert_eq!(snapshot.windows[0].used, Some(2.0));
        assert_eq!(snapshot.windows[0].limit, Some(10.0));
        assert_eq!(snapshot.windows[0].used_percent, Some(20.0));
        assert!(snapshot.facts.iter().any(|fact| fact.id == "scope"
            && fact.value == ProviderQuotaFactValue::String("Configured LiteLLM key".to_string())));
    }

    #[test]
    fn denied_or_missing_subjects_never_publish_stale_amounts_or_zero() {
        for state in [
            "permission_denied",
            "unsupported",
            "malformed_response",
            "unknown",
        ] {
            let snapshot = quota(
                "denied",
                json!({"state":"ok","subjects":[
                    {"scope":"key","state":state,"spend":{"value":99.0},"max_budget":{"value":100.0}},
                    {"scope":"user","state":"ok","spend":{"value":75.0},"max_budget":{"value":300.0}}
                ]}),
            );
            assert!(!snapshot.available, "{state}");
            assert!(snapshot.windows.is_empty());
            assert!(snapshot.facts.is_empty());
        }
        for details in [
            json!({"state":"ok","subjects":[]}),
            json!({"state":"ok","subjects":[{"scope":"key","state":"unavailable","spend":{"value":99.0}}]}),
            json!({"state":"ok","subjects":[{"scope":"user","state":"permission_denied","spend":{"value":99.0}}]}),
            json!({"state":"permission_denied","subjects":[{"scope":"key","state":"ok","spend":{"value":99.0}}]}),
        ] {
            let snapshot = quota("empty", details);
            assert!(!snapshot.available);
            assert!(snapshot.windows.is_empty());
            assert!(snapshot.facts.is_empty());
        }
    }

    #[test]
    fn unlimited_user_without_key_reports_spend_without_inventing_percentage() {
        let snapshot = quota(
            "jwt",
            json!({"state":"ok","subjects":[
                {"scope":"user","state":"ok","spend":{"value":0.00001},"max_budget":{"kind":"unlimited"}}
            ]}),
        );
        assert!(snapshot.available);
        assert!(snapshot.windows.is_empty());
        assert!(snapshot
            .facts
            .iter()
            .any(|fact| fact.id == "spend" && fact.unit.as_deref() == Some("USD")));
        assert!(snapshot.facts.iter().any(|fact| fact.id == "max_budget"
            && fact.value == ProviderQuotaFactValue::String("Unlimited".to_string())));
    }

    #[test]
    fn user_zero_and_overage_keep_existing_budget_semantics() {
        for (spend, budget, remaining, percent) in [
            (0.0, 300.0, 300.0, Some(0.0)),
            (12.5, 10.0, -2.5, Some(100.0)),
            (0.0, 0.0, 0.0, None),
        ] {
            let snapshot = quota(
                "jwt",
                json!({"state":"ok","subjects":[
                    {"scope":"key","state":"unavailable"},
                    {"scope":"user","state":"ok","spend":{"value":spend},"max_budget":{"value":budget},"remaining":{"value":remaining,"overage":remaining < 0.0}}
                ]}),
            );
            assert_eq!(snapshot.windows[0].used, Some(spend));
            assert_eq!(snapshot.windows[0].limit, Some(budget));
            assert_eq!(snapshot.windows[0].remaining, Some(remaining));
            assert_eq!(snapshot.windows[0].used_percent, percent);
            assert_eq!(
                snapshot.windows[0].status.as_deref(),
                (remaining < 0.0).then_some("over_budget")
            );
        }
    }

    #[test]
    fn quota_preserves_overage_and_limits_to_key_budget() {
        let snapshot = quota(
            "work",
            json!({"state":"ok","subjects":[
                {"scope":"key","state":"ok","spend":{"value":12.5},"max_budget":{"kind":"limited","value":10.0},"remaining":{"value":-2.5,"overage":true},"budget_reset_at":"2026-12-01T00:00:00Z"},
                {"scope":"team","spend":{"value":200.0},"max_budget":{"kind":"limited","value":300.0}}
            ]}),
        );
        assert!(snapshot.available);
        assert_eq!(snapshot.windows.len(), 1);
        assert_eq!(snapshot.windows[0].limit, Some(10.0));
        assert_eq!(snapshot.windows[0].used_percent, Some(100.0));
        assert_eq!(snapshot.windows[0].remaining, Some(-2.5));
        assert_eq!(snapshot.windows[0].status.as_deref(), Some("over_budget"));
        assert_eq!(
            snapshot.litellm.unwrap()["subjects"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn unlimited_and_denied_are_not_zero_budget_or_usage() {
        let unlimited = quota(
            "work",
            json!({"state":"ok","subjects":[{"scope":"key","state":"ok","spend":{"value":0.00001},"max_budget":{"kind":"unlimited"},"remaining":{"derived":false}}]}),
        );
        assert!(unlimited.windows.is_empty());
        assert!(unlimited.facts.iter().any(|fact| fact.id == "spend"));
        assert!(unlimited.facts.iter().any(|fact| fact.id == "max_budget"
            && fact.value == ProviderQuotaFactValue::String("Unlimited".to_string())));
        let denied = quota(
            "work",
            json!({"state":"permission_denied","message":"Not permitted","subjects":[]}),
        );
        assert!(!denied.available);
        assert!(denied.facts.is_empty());
        assert_eq!(denied.litellm.unwrap()["state"], "permission_denied");
    }
}
