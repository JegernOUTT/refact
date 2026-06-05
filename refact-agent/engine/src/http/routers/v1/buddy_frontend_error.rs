use axum::Extension;
use axum::http::HeaderMap;
use axum::response::Result;
use axum::extract::State;
use hyper::StatusCode;
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::app_state::AppState;
use crate::buddy::actor::redact_sensitive;
use crate::buddy::diagnostics::{classify_error, DiagnosticContext, DiagnosticSeverity};
use crate::custom_error::ScratchError;

const RATE_LIMIT_PER_MIN: usize = 60;

pub struct FrontendErrorRateLimiter {
    buckets: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl FrontendErrorRateLimiter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            buckets: Mutex::new(HashMap::new()),
        })
    }

    pub fn check_and_record(&self, key: &str) -> bool {
        let mut map = self.buckets.lock().unwrap();
        let now = Instant::now();
        let bucket = map.entry(key.to_string()).or_default();
        bucket.retain(|t| now.duration_since(*t).as_secs() < 60);
        if bucket.len() >= RATE_LIMIT_PER_MIN {
            return false;
        }
        bucket.push_back(now);
        true
    }
}

#[derive(Debug, Deserialize)]
pub struct FrontendErrorRequest {
    pub message: String,
    pub stack: String,
    pub url: String,
    pub kind: String,
}

pub async fn handle_v1_buddy_frontend_error(
    State(app): State<AppState>,
    Extension(rl): Extension<Arc<FrontendErrorRateLimiter>>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<FrontendErrorRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let gcx = app.gcx.clone();
    let ip_key = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string());

    if !rl.check_and_record(&ip_key) {
        return Err(ScratchError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded; try again in 60 seconds".into(),
        ));
    }

    let redacted_msg = redact_sensitive(&req.message);
    let _ = redact_sensitive(&req.stack);
    let error_type = classify_fe_error(&redacted_msg, &req.kind);

    let severity = if req.kind == "react_boundary" {
        DiagnosticSeverity::High
    } else {
        DiagnosticSeverity::Medium
    };

    let ctx = DiagnosticContext {
        error_type,
        error_message: redacted_msg,
        source_file: Some(req.url.clone()),
        tool_name: Some("frontend".to_string()),
        chat_id: None,
        collected_at: chrono::Utc::now().to_rfc3339(),
        severity,
        occurrence_count: 1,
    };

    let buddy_arc = gcx.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.add_diagnostic(ctx);
    }

    Ok(axum::Json(serde_json::json!({ "ok": true })))
}

fn classify_fe_error(message: &str, kind: &str) -> String {
    if kind == "react_boundary" {
        return "react_boundary".to_string();
    }
    let lower = message.to_lowercase();
    if lower.contains("timeout") {
        "timeout".to_string()
    } else if lower.contains("network") || lower.contains("fetch") || lower.contains("connect") {
        "network".to_string()
    } else if lower.contains("render") {
        "render".to_string()
    } else {
        classify_error(message)
    }
}
