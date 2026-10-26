use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;

use crate::app_state::AppState;
use crate::coordination::error::WorkError;
use crate::coordination::{metrics, metrics_view, scope};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsQuery {
    pub chat_id: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

async fn caller(app: &AppState, query: &MetricsQuery) -> Result<scope::WorkCaller, WorkError> {
    match &query.chat_id {
        None => scope::operator_caller(app).await,
        Some(id) => {
            let session = app
                .chat
                .sessions
                .read()
                .await
                .get(id)
                .cloned()
                .ok_or_else(WorkError::not_visible)?;
            let model = session
                .try_lock()
                .map_err(|_| WorkError::unavailable())?
                .thread
                .model
                .clone();
            scope::model_caller(app, id, &model).await
        }
    }
}

type ExtractedQuery = Result<Query<MetricsQuery>, axum::extract::rejection::QueryRejection>;

pub async fn page(
    State(app): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    query: ExtractedQuery,
) -> Result<Json<metrics_view::MetricsPage>, WorkError> {
    super::work::authorize(&app, &headers, peer.map(|peer| peer.0))?;
    let Query(query) = query.map_err(|_| WorkError::invalid("invalid metrics query"))?;
    Ok(Json(metrics_view::page(
        &app,
        &caller(&app, &query).await?,
        metrics_view::MetricsRequest {
            rows: query.limit,
            cursor: query.cursor,
        },
        metrics::recorder().snapshot(),
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::metrics::{self, Decision, Explanation, EVIDENCE_VERDICTS};
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    async fn app(token: Option<&str>) -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        std::sync::Arc::get_mut(&mut gcx)
            .unwrap()
            .cmdline
            .daemon_auth_token = token.map(str::to_string);
        std::sync::Arc::get_mut(&mut gcx).unwrap().cmdline.http_host = Some("127.0.0.1".into());
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        (AppState::from_gcx(gcx).await, dir)
    }

    async fn request(
        app: AppState,
        uri: &str,
        token: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().uri(uri);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let mut request = builder.body(Body::empty()).unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo("127.0.0.1:7".parse::<SocketAddr>().unwrap()));
        let router = crate::http::routers::v1::make_v1_router(app.clone()).with_state(app);
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    fn seed() -> String {
        let secret = "sk-live-ZZQ2ab3Ccd4Eef5Ggh6Iij7Kkl8M".to_string();
        metrics::recorder().reset();
        for verdict in ["failed", "blocked", "unavailable", "not_run"] {
            metrics::recorder().count(EVIDENCE_VERDICTS, vec![("verdict", verdict.to_string())]);
        }
        metrics::recorder().explain(Explanation::new(
            Decision::Denial,
            "denied",
            "policy.no_secret",
            vec![("reason".to_string(), secret.clone())],
        ));
        secret
    }

    #[tokio::test]
    #[serial_test::serial(coordination_metrics)]
    async fn metrics_require_authorization_before_any_snapshot_is_read() {
        let (app, _dir) = app(Some("metrics-token")).await;
        seed();
        for token in [None, Some("wrong")] {
            let (status, body) = request(app.clone(), "/metrics", token).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert!(body.get("entries").is_none());
        }
    }

    #[tokio::test]
    #[serial_test::serial(coordination_metrics)]
    async fn metrics_page_preserves_verdict_series_redaction_and_explanation_detail() {
        let (app, _dir) = app(Some("metrics-token")).await;
        let secret = seed();
        let (status, body) = request(app, "/metrics?limit=128", Some("metrics-token")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let rendered = body.to_string();
        assert!(
            !rendered.contains(&secret),
            "secret reached reader: {rendered}"
        );
        let verdicts = body["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| {
                (entry["kind"] == "series" && entry["series"]["name"] == EVIDENCE_VERDICTS)
                    .then_some(entry["series"]["labels"].as_array())
                    .flatten()
                    .into_iter()
                    .flatten()
                    .find(|label| label["name"] == "verdict")
                    .and_then(|label| label["value"].as_str())
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            verdicts,
            ["blocked", "failed", "not_run", "unavailable"].into()
        );
        let explanation = body["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["kind"] == "explanation")
            .unwrap();
        assert_eq!(explanation["explanation"]["decision"], "denial");
        assert_eq!(explanation["explanation"]["rule"], "policy.no_secret");
        assert_eq!(explanation["explanation"]["inputs"][0][0], "reason");
        assert!(explanation["explanation"]["inputs"][0][1]
            .as_str()
            .unwrap()
            .starts_with("redacted:"));
    }
}
