use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use base64::Engine;
use headless_chrome::Tab;
use headless_chrome::browser::tab::{RequestPausedDecision, RequestInterceptor};
use headless_chrome::protocol::cdp::{Fetch, Network};

use refact_integrations::browser_models::{
    HarNotFound, HarRouteInfo, RouteHandler, RouteInfo, RouteInterception, UrlPattern,
};

use crate::har::HarReplay;
use crate::network::{UrlMatcher, mask_headers, mask_text};

const INTERCEPTION_REPORT_CAP: usize = 1_000;
const HAR_ROUTE_PATTERN: &str = "har-replay";
const FETCH_MAX_REDIRECTS: usize = 20;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const FORBIDDEN_REQUEST_HEADERS: [&str; 3] = ["cookie", "host", "content-length"];
const HOP_BY_HOP_RESPONSE_HEADERS: [&str; 4] = [
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
];

#[derive(Clone, Debug)]
struct RegisteredRoute {
    info: RouteInfo,
    matcher: UrlMatcher,
}

#[derive(Debug)]
pub(crate) struct RouteSnapshot {
    routes: Vec<RouteInfo>,
    har_replay: Option<HarReplay>,
}

#[derive(Debug, Default)]
struct RouteState {
    routes: Vec<RegisteredRoute>,
    interceptions: Vec<RouteInterception>,
    har_replay: Option<HarReplay>,
}

#[derive(Debug, Default)]
pub struct RouteRegistry {
    state: Mutex<RouteState>,
}

impl RouteRegistry {
    pub fn add(
        &self,
        pattern: UrlPattern,
        handler: RouteHandler,
        times: Option<u32>,
    ) -> Result<(), String> {
        let matcher = matcher_for_pattern(&pattern)?;
        validate_handler(&handler)?;
        if times == Some(0) {
            return Err("Route times must be at least 1".to_string());
        }
        self.state.lock().unwrap().routes.push(RegisteredRoute {
            info: RouteInfo {
                pattern,
                handler,
                har: None,
                times_remaining: times,
                order: 0,
            },
            matcher,
        });
        Ok(())
    }

    pub fn remove(&self, pattern: Option<&UrlPattern>) -> usize {
        let mut state = self.state.lock().unwrap();
        let previous = state.routes.len() + usize::from(state.har_replay.is_some());
        match pattern {
            Some(pattern) => {
                state.routes.retain(|route| &route.info.pattern != pattern);
                if state
                    .har_replay
                    .as_ref()
                    .is_some_and(|replay| &har_route_pattern(replay) == pattern)
                {
                    state.har_replay = None;
                }
            }
            None => {
                state.routes.clear();
                state.har_replay = None;
            }
        }
        previous - state.routes.len() - usize::from(state.har_replay.is_some())
    }

    pub fn is_empty(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.routes.is_empty() && state.har_replay.is_none()
    }

    pub fn list(&self) -> Vec<RouteInfo> {
        let state = self.state.lock().unwrap();
        state
            .routes
            .iter()
            .rev()
            .map(|route| masked_route_info(&route.info))
            .chain(state.har_replay.as_ref().map(har_route_info))
            .enumerate()
            .map(|(order, info)| RouteInfo { order, ..info })
            .collect()
    }

    pub(crate) fn snapshot(&self) -> RouteSnapshot {
        let state = self.state.lock().unwrap();
        RouteSnapshot {
            routes: state
                .routes
                .iter()
                .map(|route| route.info.clone())
                .collect(),
            har_replay: state.har_replay.clone(),
        }
    }

    pub(crate) fn restore(&self, snapshot: RouteSnapshot) -> Result<(), String> {
        let registered = snapshot
            .routes
            .into_iter()
            .map(|info| {
                let matcher = matcher_for_pattern(&info.pattern)?;
                Ok(RegisteredRoute { info, matcher })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut state = self.state.lock().unwrap();
        state.routes = registered;
        state.har_replay = snapshot.har_replay;
        Ok(())
    }

    pub fn drain_interceptions(&self) -> Vec<RouteInterception> {
        std::mem::take(&mut self.state.lock().unwrap().interceptions)
    }

    pub fn set_har_replay(&self, replay: HarReplay) {
        self.state.lock().unwrap().har_replay = Some(replay);
    }

    pub fn clear_har_replay(&self) {
        self.state.lock().unwrap().har_replay = None;
    }

    pub fn enable_for_tab(
        self: &std::sync::Arc<Self>,
        tab: &Tab,
        handle_auth_requests: bool,
    ) -> Result<(), String> {
        let registry = self.clone();
        let interceptor: std::sync::Arc<dyn RequestInterceptor + Send + Sync> =
            std::sync::Arc::new(move |_, _, event: Fetch::events::RequestPausedEvent| {
                registry.decide(
                    event.params.request_id,
                    event.params.request.url,
                    event.params.request.method,
                    headers_from_cdp(&event.params.request.headers),
                    event.params.request.post_data,
                    event.params.redirected_request_id.is_some(),
                )
            });
        tab.enable_request_interception(interceptor)
            .map_err(|error| format!("Failed to install route interceptor: {error}"))?;
        let patterns = [Fetch::RequestPattern {
            url_pattern: None,
            resource_Type: None,
            request_stage: Some(Fetch::RequestStage::Request),
        }];
        tab.enable_fetch(Some(&patterns), Some(handle_auth_requests))
            .map(|_| ())
            .map_err(|error| format!("Failed to enable request routing: {error}"))
    }

    pub fn disable_for_tab(&self, tab: &Tab) -> Result<(), String> {
        tab.disable_fetch()
            .map(|_| ())
            .map_err(|error| format!("Failed to disable request routing: {error}"))
    }

    fn decide(
        &self,
        request_id: String,
        url: String,
        method: String,
        request_headers: BTreeMap<String, String>,
        post_data: Option<String>,
        redirect_hop: bool,
    ) -> RequestPausedDecision {
        let Some((pattern, handler)) = self.match_chain(&method, &url) else {
            return RequestPausedDecision::Continue(None);
        };
        let outcome = execute_handler(
            request_id,
            &handler,
            &url,
            &method,
            &request_headers,
            post_data.as_deref(),
        );
        let mut state = self.state.lock().unwrap();
        state.interceptions.push(RouteInterception {
            url: mask_text(&url),
            method,
            pattern: mask_pattern(&pattern),
            action: outcome.action,
            request_headers: mask_headers(request_headers),
            request_body_preview: post_data.map(|data| mask_text(&data)),
            response_body_preview: outcome.response_body_preview,
            status: outcome.status,
            reason: outcome.reason,
            redirect_hop,
        });
        if state.interceptions.len() > INTERCEPTION_REPORT_CAP {
            let excess = state.interceptions.len() - INTERCEPTION_REPORT_CAP;
            state.interceptions.drain(..excess);
        }
        outcome.decision
    }

    fn match_chain(&self, method: &str, url: &str) -> Option<(UrlPattern, RouteHandler)> {
        let mut state = self.state.lock().unwrap();
        let mut fallback_pattern = None;
        let mut expired = Vec::new();
        let mut matched = None;
        for index in (0..state.routes.len()).rev() {
            if !state.routes[index].matcher.is_match(url) {
                continue;
            }
            let route = &mut state.routes[index];
            if let Some(remaining) = route.info.times_remaining.as_mut() {
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    expired.push(index);
                }
            }
            if matches!(route.info.handler, RouteHandler::Fallback) {
                fallback_pattern = Some(route.info.pattern.clone());
                continue;
            }
            matched = Some((route.info.pattern.clone(), route.info.handler.clone()));
            break;
        }
        for index in expired {
            state.routes.remove(index);
        }
        matched
            .or_else(|| {
                state
                    .har_replay
                    .as_ref()
                    .and_then(|replay| replay.match_request(method, url))
                    .map(|handler| (UrlPattern::Text(HAR_ROUTE_PATTERN.to_string()), handler))
            })
            .or_else(|| fallback_pattern.map(|pattern| (pattern, RouteHandler::Fallback)))
    }
}

struct HandlerOutcome {
    decision: RequestPausedDecision,
    action: String,
    status: Option<u16>,
    reason: Option<String>,
    response_body_preview: Option<String>,
}

struct UpstreamResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Option<String>,
    body_base64: bool,
}

fn execute_handler(
    request_id: String,
    handler: &RouteHandler,
    url: &str,
    method: &str,
    request_headers: &BTreeMap<String, String>,
    post_data: Option<&str>,
) -> HandlerOutcome {
    match handler {
        RouteHandler::Fulfill {
            status,
            headers,
            body,
            content_type,
            body_base64,
            ..
        } => fulfill_outcome(
            request_id,
            "fulfill",
            *status,
            header_pairs(headers),
            body.clone(),
            content_type.clone(),
            *body_base64,
        ),
        RouteHandler::Abort { reason } => HandlerOutcome {
            decision: RequestPausedDecision::Fail(Fetch::FailRequest {
                request_id,
                error_reason: parse_error_reason(reason).unwrap_or(Network::ErrorReason::Failed),
            }),
            action: "abort".to_string(),
            status: None,
            reason: Some(reason.clone()),
            response_body_preview: None,
        },
        RouteHandler::Continue {
            url,
            method,
            headers,
            post_data,
        } => HandlerOutcome {
            decision: RequestPausedDecision::Continue(Some(Fetch::ContinueRequest {
                request_id,
                url: url.clone(),
                method: method.clone(),
                post_data: post_data
                    .as_ref()
                    .map(|data| base64::engine::general_purpose::STANDARD.encode(data.as_bytes())),
                headers: headers.as_ref().map(|overrides| {
                    headers_to_cdp(&merge_request_headers(request_headers, Some(overrides)))
                }),
                intercept_response: None,
            })),
            action: "continue".to_string(),
            status: None,
            reason: None,
            response_body_preview: None,
        },
        RouteHandler::Fallback => HandlerOutcome {
            decision: RequestPausedDecision::Continue(None),
            action: "fallback".to_string(),
            status: None,
            reason: None,
            response_body_preview: None,
        },
        RouteHandler::FetchAndFulfill {
            url: url_override,
            method: method_override,
            headers: header_overrides,
            post_data: post_override,
            status: status_override,
            response_headers,
            body: body_override,
            body_base64,
        } => {
            let target_url = url_override.clone().unwrap_or_else(|| url.to_string());
            let target_method = method_override
                .clone()
                .unwrap_or_else(|| method.to_string());
            let headers = merge_request_headers(request_headers, header_overrides.as_ref());
            let request_body = post_override
                .clone()
                .or_else(|| post_data.map(str::to_string));
            match fetch_upstream(&target_url, &target_method, &headers, request_body) {
                Ok(response) => {
                    let headers = merge_response_headers(response.headers, response_headers);
                    let (body, encoded) = match body_override {
                        Some(body) => (Some(body.clone()), *body_base64),
                        None => (response.body, response.body_base64),
                    };
                    fulfill_outcome(
                        request_id,
                        "fetch_and_fulfill",
                        status_override.unwrap_or(response.status),
                        headers,
                        body,
                        None,
                        encoded,
                    )
                }
                Err(error) => HandlerOutcome {
                    decision: RequestPausedDecision::Fail(Fetch::FailRequest {
                        request_id,
                        error_reason: Network::ErrorReason::Failed,
                    }),
                    action: "abort".to_string(),
                    status: None,
                    reason: Some(mask_text(&error)),
                    response_body_preview: None,
                },
            }
        }
    }
}

fn fulfill_outcome(
    request_id: String,
    action: &str,
    status: u16,
    mut headers: Vec<(String, String)>,
    body: Option<String>,
    content_type: Option<String>,
    body_base64: bool,
) -> HandlerOutcome {
    if let Some(content_type) = content_type {
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        {
            headers.push(("Content-Type".to_string(), content_type));
        }
    }
    let body_wire = body.as_ref().map(|body| {
        if body_base64 {
            body.clone()
        } else {
            base64::engine::general_purpose::STANDARD.encode(body.as_bytes())
        }
    });
    let response_body_preview = body.as_ref().map(|body| {
        if body_base64 {
            "[base64 body]".to_string()
        } else {
            mask_text(body)
        }
    });
    HandlerOutcome {
        decision: RequestPausedDecision::Fulfill(Fetch::FulfillRequest {
            request_id,
            response_code: status as u32,
            response_headers: Some(header_entries(&headers)),
            binary_response_headers: None,
            body: body_wire,
            response_phrase: None,
        }),
        action: action.to_string(),
        status: Some(status),
        reason: None,
        response_body_preview,
    }
}

fn fetch_client() -> Result<&'static reqwest::blocking::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::blocking::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .redirect(reqwest::redirect::Policy::limited(FETCH_MAX_REDIRECTS))
                .timeout(FETCH_TIMEOUT)
                .build()
                .map_err(|error| format!("Failed to build route fetch client: {error}"))
        })
        .as_ref()
        .map_err(|error| error.clone())
}

fn fetch_upstream(
    url: &str,
    method: &str,
    headers: &BTreeMap<String, String>,
    body: Option<String>,
) -> Result<UpstreamResponse, String> {
    let method = reqwest::Method::from_bytes(method.to_ascii_uppercase().as_bytes())
        .map_err(|error| format!("Invalid route fetch method: {error}"))?;
    let mut request = fetch_client()?.request(method, url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if let Some(body) = body {
        request = request.body(body);
    }
    let response = request
        .send()
        .map_err(|error| format!("Route fetch failed: {error}"))?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter(|(name, _)| !is_hop_by_hop_response_header(name.as_str()))
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.to_string(), value.to_string()))
        })
        .collect::<Vec<_>>();
    let bytes = response
        .bytes()
        .map_err(|error| format!("Route fetch response failed: {error}"))?;
    let (body, body_base64) = match String::from_utf8(bytes.to_vec()) {
        Ok(text) => (Some(text), false),
        Err(error) => (
            Some(base64::engine::general_purpose::STANDARD.encode(error.into_bytes())),
            true,
        ),
    };
    Ok(UpstreamResponse {
        status,
        headers,
        body,
        body_base64,
    })
}

pub fn normalize_route_handler(
    handler: RouteHandler,
    artifacts_dir: &Path,
    allowed_roots: &[PathBuf],
) -> Result<RouteHandler, String> {
    let RouteHandler::Fulfill {
        status,
        headers,
        body,
        path,
        json,
        content_type,
        body_base64,
    } = handler
    else {
        return Ok(handler);
    };
    if usize::from(body.is_some()) + usize::from(path.is_some()) + usize::from(json.is_some()) > 1 {
        return Err("Route fulfill accepts only one of body, path, or json".to_string());
    }
    let (body, content_type, body_base64) = match (path, json) {
        (Some(path), _) => {
            let resolved =
                resolve_contained_path(&path, "Route fulfill", artifacts_dir, allowed_roots)?;
            let bytes = std::fs::read(&resolved)
                .map_err(|error| format!("Failed to read route fulfill path: {error}"))?;
            let content_type =
                content_type.or_else(|| Some(content_type_for_path(&resolved).to_string()));
            match String::from_utf8(bytes) {
                Ok(text) => (Some(text), content_type, false),
                Err(error) => (
                    Some(base64::engine::general_purpose::STANDARD.encode(error.into_bytes())),
                    content_type,
                    true,
                ),
            }
        }
        (_, Some(json)) => (
            Some(
                serde_json::to_string(&json)
                    .map_err(|error| format!("Invalid route fulfill json: {error}"))?,
            ),
            Some(content_type.unwrap_or_else(|| "application/json".to_string())),
            false,
        ),
        _ => (body, content_type, body_base64),
    };
    Ok(RouteHandler::Fulfill {
        status,
        headers,
        body,
        path: None,
        json: None,
        content_type,
        body_base64,
    })
}

pub fn resolve_contained_path(
    path: &str,
    label: &str,
    artifacts_dir: &Path,
    allowed_roots: &[PathBuf],
) -> Result<PathBuf, String> {
    let candidate = Path::new(path);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        artifacts_dir.join(candidate)
    };
    let canonical = resolved
        .canonicalize()
        .map_err(|_| format!("{label} path does not exist: {path}"))?;
    let contained = std::iter::once(artifacts_dir)
        .chain(allowed_roots.iter().map(PathBuf::as_path))
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| canonical.starts_with(&root));
    if !contained {
        return Err(format!(
            "{label} path escapes the allowed directories: {path}"
        ));
    }
    if !canonical.is_file() {
        return Err(format!("{label} path is not a file: {path}"));
    }
    Ok(canonical)
}

fn content_type_for_path(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        Some("md") => "text/markdown; charset=utf-8",
        Some("csv") => "text/csv; charset=utf-8",
        Some("xml") => "application/xml",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("pdf") => "application/pdf",
        Some("wasm") => "application/wasm",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn is_forbidden_request_header(name: &str) -> bool {
    FORBIDDEN_REQUEST_HEADERS
        .iter()
        .any(|forbidden| name.eq_ignore_ascii_case(forbidden))
}

fn is_hop_by_hop_response_header(name: &str) -> bool {
    HOP_BY_HOP_RESPONSE_HEADERS
        .iter()
        .any(|hop| name.eq_ignore_ascii_case(hop))
}

fn merge_request_headers(
    existing: &BTreeMap<String, String>,
    overrides: Option<&BTreeMap<String, String>>,
) -> BTreeMap<String, String> {
    let Some(overrides) = overrides else {
        return existing.clone();
    };
    let allowed = overrides
        .iter()
        .filter(|(name, _)| !is_forbidden_request_header(name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    merge_headers(existing, &allowed)
}

fn matcher_for_pattern(pattern: &UrlPattern) -> Result<UrlMatcher, String> {
    match pattern {
        UrlPattern::Text(value) => UrlMatcher::text(value),
        UrlPattern::Regex { source, flags } => UrlMatcher::regex(source, flags),
    }
}

fn validate_handler(handler: &RouteHandler) -> Result<(), String> {
    match handler {
        RouteHandler::Fulfill { status, .. }
        | RouteHandler::FetchAndFulfill {
            status: Some(status),
            ..
        } if !(100..=599).contains(status) => Err(format!(
            "Route fulfill status must be between 100 and 599, got {status}"
        )),
        RouteHandler::Fulfill {
            body: Some(body),
            body_base64: true,
            ..
        }
        | RouteHandler::FetchAndFulfill {
            body: Some(body),
            body_base64: true,
            ..
        } => base64::engine::general_purpose::STANDARD
            .decode(body)
            .map(|_| ())
            .map_err(|error| format!("Invalid base64 route body: {error}")),
        RouteHandler::Abort { reason } => parse_error_reason(reason)
            .map(|_| ())
            .ok_or_else(|| format!("Unsupported route abort reason: {reason}")),
        _ => Ok(()),
    }
}

fn parse_error_reason(reason: &str) -> Option<Network::ErrorReason> {
    let normalized = reason
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    Some(match normalized.as_str() {
        "failed" => Network::ErrorReason::Failed,
        "aborted" => Network::ErrorReason::Aborted,
        "timedout" => Network::ErrorReason::TimedOut,
        "accessdenied" => Network::ErrorReason::AccessDenied,
        "connectionclosed" => Network::ErrorReason::ConnectionClosed,
        "connectionreset" => Network::ErrorReason::ConnectionReset,
        "connectionrefused" => Network::ErrorReason::ConnectionRefused,
        "connectionaborted" => Network::ErrorReason::ConnectionAborted,
        "connectionfailed" => Network::ErrorReason::ConnectionFailed,
        "namenotresolved" => Network::ErrorReason::NameNotResolved,
        "internetdisconnected" => Network::ErrorReason::InternetDisconnected,
        "addressunreachable" => Network::ErrorReason::AddressUnreachable,
        "blockedbyclient" => Network::ErrorReason::BlockedByClient,
        "blockedbyresponse" => Network::ErrorReason::BlockedByResponse,
        _ => return None,
    })
}

fn headers_from_cdp(headers: &Network::Headers) -> BTreeMap<String, String> {
    headers
        .0
        .as_ref()
        .and_then(|value| value.as_object())
        .map(|headers| {
            headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| value.to_string()),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn headers_to_cdp(headers: &BTreeMap<String, String>) -> Vec<Fetch::HeaderEntry> {
    headers
        .iter()
        .map(|(name, value)| Fetch::HeaderEntry {
            name: name.clone(),
            value: value.clone(),
        })
        .collect()
}

fn header_entries(headers: &[(String, String)]) -> Vec<Fetch::HeaderEntry> {
    headers
        .iter()
        .map(|(name, value)| Fetch::HeaderEntry {
            name: name.clone(),
            value: value.clone(),
        })
        .collect()
}

fn header_pairs(headers: &BTreeMap<String, String>) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn merge_response_headers(
    upstream: Vec<(String, String)>,
    overrides: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let mut merged = upstream
        .into_iter()
        .filter(|(name, _)| {
            !overrides
                .keys()
                .any(|override_name| override_name.eq_ignore_ascii_case(name))
        })
        .collect::<Vec<_>>();
    merged.extend(header_pairs(overrides));
    merged
}

fn merge_headers(
    existing: &BTreeMap<String, String>,
    overrides: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut merged = existing.clone();
    for (name, value) in overrides {
        if let Some(existing_name) = merged
            .keys()
            .find(|existing_name| existing_name.eq_ignore_ascii_case(name))
            .cloned()
        {
            merged.remove(&existing_name);
        }
        merged.insert(name.clone(), value.clone());
    }
    merged
}

fn masked_body(body: Option<&String>, body_base64: bool) -> Option<String> {
    body.map(|body| {
        if body_base64 {
            "[base64 body]".to_string()
        } else {
            mask_text(body)
        }
    })
}

fn masked_route_info(info: &RouteInfo) -> RouteInfo {
    let handler = match &info.handler {
        RouteHandler::Fulfill {
            status,
            headers,
            body,
            path,
            json,
            content_type,
            body_base64,
        } => RouteHandler::Fulfill {
            status: *status,
            headers: mask_headers(headers.clone()),
            body: masked_body(body.as_ref(), *body_base64),
            path: path.as_ref().map(|path| mask_text(path)),
            json: json.clone(),
            content_type: content_type.clone(),
            body_base64: *body_base64,
        },
        RouteHandler::Abort { reason } => RouteHandler::Abort {
            reason: reason.clone(),
        },
        RouteHandler::Continue {
            url,
            method,
            headers,
            post_data,
        } => RouteHandler::Continue {
            url: url.as_ref().map(|url| mask_text(url)),
            method: method.clone(),
            headers: headers.clone().map(mask_headers),
            post_data: post_data.as_ref().map(|data| mask_text(data)),
        },
        RouteHandler::Fallback => RouteHandler::Fallback,
        RouteHandler::FetchAndFulfill {
            url,
            method,
            headers,
            post_data,
            status,
            response_headers,
            body,
            body_base64,
        } => RouteHandler::FetchAndFulfill {
            url: url.as_ref().map(|url| mask_text(url)),
            method: method.clone(),
            headers: headers.clone().map(mask_headers),
            post_data: post_data.as_ref().map(|data| mask_text(data)),
            status: *status,
            response_headers: mask_headers(response_headers.clone()),
            body: masked_body(body.as_ref(), *body_base64),
            body_base64: *body_base64,
        },
    };
    RouteInfo {
        pattern: mask_pattern(&info.pattern),
        handler,
        har: info.har.clone(),
        times_remaining: info.times_remaining,
        order: info.order,
    }
}

fn har_route_pattern(replay: &HarReplay) -> UrlPattern {
    UrlPattern::Text(format!("{HAR_ROUTE_PATTERN}:{}", replay.label()))
}

fn har_route_info(replay: &HarReplay) -> RouteInfo {
    RouteInfo {
        pattern: har_route_pattern(replay),
        handler: match replay.not_found() {
            HarNotFound::Abort => RouteHandler::Abort {
                reason: "blockedbyclient".to_string(),
            },
            HarNotFound::Fallback => RouteHandler::Continue {
                url: None,
                method: None,
                headers: None,
                post_data: None,
            },
        },
        har: Some(HarRouteInfo {
            entry_count: replay.entry_count(),
            not_found: replay.not_found(),
        }),
        times_remaining: None,
        order: 0,
    }
}

fn mask_pattern(pattern: &UrlPattern) -> UrlPattern {
    match pattern {
        UrlPattern::Text(value) => UrlPattern::Text(mask_text(value)),
        UrlPattern::Regex { source, flags } => UrlPattern::Regex {
            source: mask_text(source),
            flags: flags.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HAR_FIXTURE: &str = r#"{"log":{"version":"1.2","creator":{"name":"test","version":"1"},"entries":[{"startedDateTime":"1970-01-01T00:00:00.000Z","time":1.0,"request":{"method":"GET","url":"https://example.com/api","httpVersion":"HTTP/1.1","headers":[],"queryString":[],"cookies":[],"headersSize":-1,"bodySize":0},"response":{"status":200,"statusText":"OK","httpVersion":"HTTP/1.1","headers":[],"cookies":[],"content":{"size":2,"mimeType":"text/plain","text":"ok"},"redirectURL":"","headersSize":-1,"bodySize":2},"cache":{},"timings":{"send":0.0,"wait":0.0,"receive":0.0}}]}}"#;

    fn route(pattern: &str, handler: RouteHandler) -> (UrlPattern, RouteHandler) {
        (UrlPattern::Text(pattern.to_string()), handler)
    }
    fn fetch_and_fulfill(response_headers: BTreeMap<String, String>) -> RouteHandler {
        RouteHandler::FetchAndFulfill {
            url: None,
            method: None,
            headers: None,
            post_data: None,
            status: None,
            response_headers,
            body: None,
            body_base64: false,
        }
    }

    fn fulfilled_headers(outcome: &HandlerOutcome) -> Vec<(String, String)> {
        match &outcome.decision {
            RequestPausedDecision::Fulfill(request) => request
                .response_headers
                .as_ref()
                .map(|headers| {
                    headers
                        .iter()
                        .map(|entry| (entry.name.to_ascii_lowercase(), entry.value.clone()))
                        .collect()
                })
                .unwrap_or_default(),
            other => panic!("expected a fulfill decision, got {other:?}"),
        }
    }

    fn upstream_serving(response: &'static str) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/resource", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let _ = stream.read(&mut [0u8; 1024]);
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        });
        (url, handle)
    }

    #[test]
    fn duplicate_response_headers_survive_fetch_and_fulfill() {
        let (url, server) = upstream_serving(
            "HTTP/1.1 200 OK\r\nSet-Cookie: a=1; Path=/\r\nSet-Cookie: b=2; Path=/\r\nContent-Length: 2\r\n\r\nok",
        );
        let outcome = execute_handler(
            "request-1".to_string(),
            &fetch_and_fulfill(BTreeMap::new()),
            &url,
            "GET",
            &BTreeMap::new(),
            None,
        );
        server.join().unwrap();
        let cookies = fulfilled_headers(&outcome)
            .into_iter()
            .filter(|(name, _)| name == "set-cookie")
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        assert_eq!(
            cookies,
            vec!["a=1; Path=/".to_string(), "b=2; Path=/".to_string()],
            "every Set-Cookie must reach fulfillRequest, not just the last one"
        );
    }

    #[test]
    fn response_header_overrides_replace_every_upstream_copy_case_insensitively() {
        let upstream = vec![
            ("Set-Cookie".to_string(), "a=1".to_string()),
            ("set-cookie".to_string(), "b=2".to_string()),
            ("X-Keep".to_string(), "yes".to_string()),
        ];
        let overrides = BTreeMap::from([("SET-COOKIE".to_string(), "override=1".to_string())]);
        assert_eq!(
            merge_response_headers(upstream, &overrides),
            vec![
                ("X-Keep".to_string(), "yes".to_string()),
                ("SET-COOKIE".to_string(), "override=1".to_string()),
            ]
        );
    }

    #[test]
    fn route_fetch_decides_inline_and_stays_bounded() {
        assert!(
            FETCH_TIMEOUT <= Duration::from_secs(10),
            "the interceptor callback blocks all CDP traffic while fetching"
        );
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/gone", dead.local_addr().unwrap());
        drop(dead);
        let started = std::time::Instant::now();
        let outcome = execute_handler(
            "request-1".to_string(),
            &fetch_and_fulfill(BTreeMap::new()),
            &url,
            "GET",
            &BTreeMap::new(),
            None,
        );
        assert!(
            matches!(outcome.decision, RequestPausedDecision::Fail(_)),
            "a failed upstream fetch must resolve the interception inline"
        );
        assert_eq!(outcome.action, "abort");
        assert!(outcome.reason.is_some());
        assert!(started.elapsed() < FETCH_TIMEOUT);
    }

    fn fulfill(status: u16, body: &str) -> RouteHandler {
        RouteHandler::Fulfill {
            status,
            headers: BTreeMap::new(),
            body: Some(body.to_string()),
            path: None,
            json: None,
            content_type: None,
            body_base64: false,
        }
    }

    fn abort(reason: &str) -> RouteHandler {
        RouteHandler::Abort {
            reason: reason.to_string(),
        }
    }

    fn har_replay(dir: &tempfile::TempDir, name: &str, not_found: HarNotFound) -> HarReplay {
        let path = dir.path().join(name);
        std::fs::write(&path, HAR_FIXTURE).unwrap();
        HarReplay::load(&path, None, not_found).unwrap()
    }

    fn decide_get(registry: &RouteRegistry, request_id: &str, url: &str) -> RequestPausedDecision {
        registry.decide(
            request_id.to_string(),
            url.to_string(),
            "GET".to_string(),
            BTreeMap::new(),
            None,
            false,
        )
    }

    #[test]
    fn registry_add_remove_and_clear_reuse_url_matcher() {
        let registry = RouteRegistry::default();
        let (pattern, handler) = route("https://example.com/**", abort("blockedbyclient"));
        registry.add(pattern.clone(), handler, None).unwrap();
        registry
            .add(
                UrlPattern::Regex {
                    source: "/assets/.*".to_string(),
                    flags: "i".to_string(),
                },
                RouteHandler::Continue {
                    url: None,
                    method: None,
                    headers: None,
                    post_data: None,
                },
                None,
            )
            .unwrap();

        assert!(!registry.is_empty());
        assert_eq!(registry.list().len(), 2);
        assert_eq!(registry.remove(Some(&pattern)), 1);
        assert_eq!(registry.remove(None), 1);
        assert!(registry.is_empty());
    }

    #[test]
    fn registry_rejects_invalid_fulfill_status_and_base64_body() {
        let registry = RouteRegistry::default();
        assert!(registry
            .add(
                UrlPattern::Text("https://example.com/**".to_string()),
                RouteHandler::Fulfill {
                    status: 99,
                    headers: BTreeMap::new(),
                    body: None,
                    path: None,
                    json: None,
                    content_type: None,
                    body_base64: false,
                },
                None,
            )
            .is_err());
        assert!(registry
            .add(
                UrlPattern::Text("https://example.com/**".to_string()),
                RouteHandler::Fulfill {
                    status: 200,
                    headers: BTreeMap::new(),
                    body: Some("not base64".to_string()),
                    path: None,
                    json: None,
                    content_type: None,
                    body_base64: true,
                },
                None,
            )
            .is_err());
        assert!(registry.is_empty());
    }

    #[test]
    fn har_replay_is_listed_with_not_found_mode_and_entry_count() {
        let dir = tempfile::tempdir().unwrap();
        let registry = RouteRegistry::default();
        registry.set_har_replay(har_replay(&dir, "page.har", HarNotFound::Abort));

        let listed = registry.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].pattern,
            UrlPattern::Text("har-replay:page.har".to_string())
        );
        let har = listed[0].har.clone().expect("HAR route metadata");
        assert_eq!(har.entry_count, 1);
        assert_eq!(har.not_found, HarNotFound::Abort);
        assert!(matches!(listed[0].handler, RouteHandler::Abort { .. }));
    }

    #[test]
    fn unroute_all_clears_har_replay_and_counts_it() {
        let dir = tempfile::tempdir().unwrap();
        let registry = RouteRegistry::default();
        let (pattern, handler) = route("https://example.com/**", abort("blockedbyclient"));
        registry.add(pattern, handler, None).unwrap();
        registry.set_har_replay(har_replay(&dir, "page.har", HarNotFound::Abort));

        assert_eq!(registry.list().len(), 2);
        assert_eq!(registry.remove(None), 2);
        assert!(registry.is_empty());
        assert!(registry.list().is_empty());
    }

    #[test]
    fn targeted_unroute_removes_only_the_har_replay() {
        let dir = tempfile::tempdir().unwrap();
        let registry = RouteRegistry::default();
        let (pattern, handler) = route("https://example.com/**", abort("blockedbyclient"));
        registry.add(pattern.clone(), handler, None).unwrap();
        registry.set_har_replay(har_replay(&dir, "page.har", HarNotFound::Fallback));

        assert_eq!(registry.remove(Some(&pattern)), 1);
        assert!(!registry.is_empty());
        assert_eq!(registry.list().len(), 1);

        let har_pattern = UrlPattern::Text("har-replay:page.har".to_string());
        assert_eq!(registry.remove(Some(&har_pattern)), 1);
        assert!(registry.is_empty());
        assert_eq!(registry.remove(Some(&har_pattern)), 0);
    }

    #[test]
    fn re_registering_har_replay_replaces_the_listed_entry() {
        let dir = tempfile::tempdir().unwrap();
        let registry = RouteRegistry::default();
        registry.set_har_replay(har_replay(&dir, "page.har", HarNotFound::Abort));
        registry.set_har_replay(har_replay(&dir, "other.har", HarNotFound::Fallback));

        let listed = registry.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].pattern,
            UrlPattern::Text("har-replay:other.har".to_string())
        );
        assert_eq!(
            listed[0].har.as_ref().unwrap().not_found,
            HarNotFound::Fallback
        );
    }

    #[test]
    fn har_interception_attribution_stays_har_replay() {
        let dir = tempfile::tempdir().unwrap();
        let registry = RouteRegistry::default();
        registry.set_har_replay(har_replay(&dir, "page.har", HarNotFound::Abort));

        let decision = decide_get(&registry, "one", "https://example.com/missing");
        assert!(matches!(decision, RequestPausedDecision::Fail(_)));

        let reports = registry.drain_interceptions();
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].pattern,
            UrlPattern::Text("har-replay".to_string())
        );
        assert_eq!(reports[0].action, "abort");

        registry.remove(None);
        let after = decide_get(&registry, "two", "https://example.com/missing");
        assert!(matches!(after, RequestPausedDecision::Continue(None)));
        assert!(registry.drain_interceptions().is_empty());
    }

    #[test]
    fn snapshot_and_restore_round_trip_the_har_replay() {
        let dir = tempfile::tempdir().unwrap();
        let registry = RouteRegistry::default();
        registry.set_har_replay(har_replay(&dir, "page.har", HarNotFound::Abort));

        let snapshot = registry.snapshot();
        assert_eq!(registry.remove(None), 1);
        assert!(registry.is_empty());

        registry.restore(snapshot).unwrap();
        assert!(!registry.is_empty());
        assert_eq!(
            registry.list()[0].pattern,
            UrlPattern::Text("har-replay:page.har".to_string())
        );
    }

    #[test]
    fn redirect_hops_are_matched_and_reported_each_time() {
        let registry = RouteRegistry::default();
        registry
            .add(
                UrlPattern::Text("https://example.com/**".to_string()),
                RouteHandler::Continue {
                    url: None,
                    method: None,
                    headers: Some(BTreeMap::from([(
                        "Authorization".to_string(),
                        "Bearer secret".to_string(),
                    )])),
                    post_data: Some("token=hidden".to_string()),
                },
                None,
            )
            .unwrap();

        for (request_id, redirect_hop) in [("one", false), ("two", true)] {
            let decision = registry.decide(
                request_id.to_string(),
                "https://example.com/api/data".to_string(),
                "POST".to_string(),
                BTreeMap::from([("Cookie".to_string(), "session=secret".to_string())]),
                Some("password=hunter2".to_string()),
                redirect_hop,
            );
            assert!(matches!(decision, RequestPausedDecision::Continue(Some(_))));
        }

        let reports = registry.drain_interceptions();
        assert_eq!(reports.len(), 2);
        assert!(!reports[0].redirect_hop);
        assert!(reports[1].redirect_hop);
        assert_eq!(reports[0].request_headers["Cookie"], "[REDACTED]");
        assert!(!reports[0]
            .request_body_preview
            .as_deref()
            .unwrap()
            .contains("hunter2"));
    }

    #[test]
    fn fulfill_abort_and_continue_create_expected_decisions() {
        let handlers = [
            RouteHandler::Fulfill {
                status: 201,
                headers: BTreeMap::new(),
                body: Some("ok".to_string()),
                path: None,
                json: None,
                content_type: Some("text/plain".to_string()),
                body_base64: false,
            },
            abort("blockedbyclient"),
            RouteHandler::Continue {
                url: Some("https://example.com/other".to_string()),
                method: Some("PATCH".to_string()),
                headers: None,
                post_data: Some("updated".to_string()),
            },
        ];
        for (index, handler) in handlers.into_iter().enumerate() {
            let registry = RouteRegistry::default();
            registry
                .add(
                    UrlPattern::Text("https://example.com/**".to_string()),
                    handler,
                    None,
                )
                .unwrap();
            let decision = decide_get(&registry, &index.to_string(), "https://example.com/api");
            match index {
                0 => assert!(matches!(decision, RequestPausedDecision::Fulfill(_))),
                1 => assert!(matches!(decision, RequestPausedDecision::Fail(_))),
                _ => assert!(matches!(decision, RequestPausedDecision::Continue(Some(_)))),
            }
            assert_eq!(registry.drain_interceptions().len(), 1);
        }
    }

    #[test]
    fn newest_matching_route_wins_and_list_reports_chain_order() {
        let registry = RouteRegistry::default();
        let pattern = UrlPattern::Text("https://example.com/**".to_string());
        registry
            .add(pattern.clone(), fulfill(200, "oldest"), None)
            .unwrap();
        registry
            .add(pattern.clone(), fulfill(201, "newest"), None)
            .unwrap();

        decide_get(&registry, "one", "https://example.com/api");
        let reports = registry.drain_interceptions();
        assert_eq!(reports[0].status, Some(201));
        assert_eq!(reports[0].response_body_preview.as_deref(), Some("newest"));

        let listed = registry.list();
        assert_eq!(listed[0].order, 0);
        assert_eq!(listed[1].order, 1);
        assert!(matches!(
            &listed[0].handler,
            RouteHandler::Fulfill { body, .. } if body.as_deref() == Some("newest")
        ));
    }

    #[test]
    fn fallback_handler_passes_to_the_next_older_matching_route() {
        let registry = RouteRegistry::default();
        let pattern = UrlPattern::Text("https://example.com/**".to_string());
        registry
            .add(pattern.clone(), fulfill(200, "oldest"), None)
            .unwrap();
        registry
            .add(pattern.clone(), RouteHandler::Fallback, None)
            .unwrap();

        let decision = decide_get(&registry, "one", "https://example.com/api");
        assert!(matches!(decision, RequestPausedDecision::Fulfill(_)));
        let reports = registry.drain_interceptions();
        assert_eq!(reports[0].action, "fulfill");
        assert_eq!(reports[0].status, Some(200));
    }

    #[test]
    fn fallback_chain_traverses_into_the_har_tail() {
        let dir = tempfile::tempdir().unwrap();
        let registry = RouteRegistry::default();
        registry
            .add(
                UrlPattern::Text("https://example.com/**".to_string()),
                RouteHandler::Fallback,
                None,
            )
            .unwrap();
        registry.set_har_replay(har_replay(&dir, "page.har", HarNotFound::Abort));

        let decision = decide_get(&registry, "one", "https://example.com/api");
        assert!(matches!(decision, RequestPausedDecision::Fulfill(_)));
        let reports = registry.drain_interceptions();
        assert_eq!(
            reports[0].pattern,
            UrlPattern::Text("har-replay".to_string())
        );
        assert_eq!(reports[0].response_body_preview.as_deref(), Some("ok"));
    }

    #[test]
    fn fallback_without_a_tail_reaches_the_network_and_is_still_reported() {
        let registry = RouteRegistry::default();
        registry
            .add(
                UrlPattern::Text("https://example.com/**".to_string()),
                RouteHandler::Fallback,
                None,
            )
            .unwrap();

        let decision = decide_get(&registry, "one", "https://example.com/api");
        assert!(matches!(decision, RequestPausedDecision::Continue(None)));
        let reports = registry.drain_interceptions();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].action, "fallback");
    }

    #[test]
    fn times_counts_down_and_expires_the_route_from_the_registry_and_listing() {
        let registry = RouteRegistry::default();
        let pattern = UrlPattern::Text("https://example.com/**".to_string());
        registry
            .add(pattern.clone(), fulfill(200, "mocked"), Some(2))
            .unwrap();

        assert_eq!(registry.list()[0].times_remaining, Some(2));
        assert!(matches!(
            decide_get(&registry, "one", "https://example.com/api"),
            RequestPausedDecision::Fulfill(_)
        ));
        assert_eq!(registry.list()[0].times_remaining, Some(1));

        assert!(matches!(
            decide_get(&registry, "two", "https://example.com/api"),
            RequestPausedDecision::Fulfill(_)
        ));
        assert!(registry.is_empty());
        assert!(registry.list().is_empty());

        assert!(matches!(
            decide_get(&registry, "three", "https://example.com/api"),
            RequestPausedDecision::Continue(None)
        ));
    }

    #[test]
    fn traversed_fallback_handlers_also_consume_their_times_budget() {
        let registry = RouteRegistry::default();
        let pattern = UrlPattern::Text("https://example.com/**".to_string());
        registry
            .add(pattern.clone(), fulfill(200, "oldest"), None)
            .unwrap();
        registry
            .add(pattern.clone(), RouteHandler::Fallback, Some(1))
            .unwrap();

        decide_get(&registry, "one", "https://example.com/api");
        let listed = registry.list();
        assert_eq!(listed.len(), 1);
        assert!(matches!(
            &listed[0].handler,
            RouteHandler::Fulfill { body, .. } if body.as_deref() == Some("oldest")
        ));
    }

    #[test]
    fn unmatched_routes_keep_their_times_budget() {
        let registry = RouteRegistry::default();
        registry
            .add(
                UrlPattern::Text("https://example.com/api/**".to_string()),
                fulfill(200, "mocked"),
                Some(1),
            )
            .unwrap();

        decide_get(&registry, "one", "https://example.com/assets/app.js");
        assert_eq!(registry.list()[0].times_remaining, Some(1));
    }

    #[test]
    fn zero_times_is_rejected_at_registration() {
        let registry = RouteRegistry::default();
        let error = registry
            .add(
                UrlPattern::Text("https://example.com/**".to_string()),
                fulfill(200, "mocked"),
                Some(0),
            )
            .unwrap_err();

        assert!(error.contains("times"), "unexpected error: {error}");
        assert!(registry.is_empty());
    }

    #[test]
    fn targeted_unroute_removes_every_handler_of_that_pattern() {
        let registry = RouteRegistry::default();
        let pattern = UrlPattern::Text("https://example.com/**".to_string());
        registry
            .add(pattern.clone(), fulfill(200, "oldest"), None)
            .unwrap();
        registry
            .add(pattern.clone(), RouteHandler::Fallback, None)
            .unwrap();
        registry
            .add(
                UrlPattern::Text("https://other.dev/**".to_string()),
                abort("failed"),
                None,
            )
            .unwrap();

        assert_eq!(registry.remove(Some(&pattern)), 2);
        assert_eq!(registry.list().len(), 1);
    }

    #[test]
    fn fulfill_from_path_infers_the_content_type_from_the_extension() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mock.json"), r#"{"ok":true}"#).unwrap();

        let normalized = normalize_route_handler(
            RouteHandler::Fulfill {
                status: 200,
                headers: BTreeMap::new(),
                body: None,
                path: Some("mock.json".to_string()),
                json: None,
                content_type: None,
                body_base64: false,
            },
            dir.path(),
            &[],
        )
        .unwrap();

        let RouteHandler::Fulfill {
            body,
            path,
            content_type,
            body_base64,
            ..
        } = normalized
        else {
            panic!("expected a fulfill handler");
        };
        assert_eq!(body.as_deref(), Some(r#"{"ok":true}"#));
        assert_eq!(path, None);
        assert_eq!(content_type.as_deref(), Some("application/json"));
        assert!(!body_base64);
    }

    #[test]
    fn fulfill_from_path_encodes_binary_files_as_base64() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pixel.png"), [0xffu8, 0xd8, 0x00, 0x01]).unwrap();

        let normalized = normalize_route_handler(
            RouteHandler::Fulfill {
                status: 200,
                headers: BTreeMap::new(),
                body: None,
                path: Some("pixel.png".to_string()),
                json: None,
                content_type: None,
                body_base64: false,
            },
            dir.path(),
            &[],
        )
        .unwrap();

        let RouteHandler::Fulfill {
            body,
            content_type,
            body_base64,
            ..
        } = normalized
        else {
            panic!("expected a fulfill handler");
        };
        assert!(body_base64);
        assert_eq!(content_type.as_deref(), Some("image/png"));
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(body.unwrap())
                .unwrap(),
            vec![0xffu8, 0xd8, 0x00, 0x01]
        );
    }

    #[test]
    fn fulfill_path_rejects_traversal_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("outside-route.txt");
        std::fs::write(&outside, "secret").unwrap();
        let artifacts = dir.path().join("artifacts");
        std::fs::create_dir(&artifacts).unwrap();

        let traversal = normalize_route_handler(
            RouteHandler::Fulfill {
                status: 200,
                headers: BTreeMap::new(),
                body: None,
                path: Some("../../outside-route.txt".to_string()),
                json: None,
                content_type: None,
                body_base64: false,
            },
            &artifacts,
            &[],
        )
        .unwrap_err();
        assert!(traversal.contains("escapes"), "unexpected: {traversal}");

        let missing = normalize_route_handler(
            RouteHandler::Fulfill {
                status: 200,
                headers: BTreeMap::new(),
                body: None,
                path: Some("nope.txt".to_string()),
                json: None,
                content_type: None,
                body_base64: false,
            },
            &artifacts,
            &[],
        )
        .unwrap_err();
        assert!(missing.contains("does not exist"), "unexpected: {missing}");

        std::fs::remove_file(&outside).unwrap();
    }

    fn fulfill_from_path(
        path: &Path,
        artifacts: &Path,
        allowed_roots: &[PathBuf],
    ) -> Result<RouteHandler, String> {
        normalize_route_handler(
            RouteHandler::Fulfill {
                status: 200,
                headers: BTreeMap::new(),
                body: None,
                path: Some(path.to_string_lossy().into_owned()),
                json: None,
                content_type: None,
                body_base64: false,
            },
            artifacts,
            allowed_roots,
        )
    }

    #[test]
    fn absolute_fulfill_paths_are_contained_by_artifacts_and_allowed_roots() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = dir.path().join("artifacts");
        let workspace = dir.path().join("workspace");
        let secrets = dir.path().join("secrets");
        for directory in [&artifacts, &workspace, &secrets] {
            std::fs::create_dir(directory).unwrap();
        }
        let inside = artifacts.join("mock.json");
        let in_workspace = workspace.join("fixture.json");
        let outside = secrets.join("id_rsa");
        for (path, body) in [
            (&inside, r#"{"ok":true}"#),
            (&in_workspace, r#"{"ws":true}"#),
            (&outside, "PRIVATE KEY"),
        ] {
            std::fs::write(path, body).unwrap();
        }

        assert!(fulfill_from_path(&inside, &artifacts, &[]).is_ok());

        let escaped = fulfill_from_path(&outside, &artifacts, &[]).unwrap_err();
        assert!(
            escaped.contains("escapes the allowed directories"),
            "unexpected: {escaped}"
        );

        let denied = fulfill_from_path(&in_workspace, &artifacts, &[]).unwrap_err();
        assert!(
            denied.contains("escapes the allowed directories"),
            "unexpected: {denied}"
        );

        let allowed = fulfill_from_path(&in_workspace, &artifacts, &[workspace.clone()]).unwrap();
        let RouteHandler::Fulfill { body, .. } = allowed else {
            panic!("expected a fulfill handler");
        };
        assert_eq!(body.as_deref(), Some(r#"{"ws":true}"#));

        let still_denied = fulfill_from_path(&outside, &artifacts, &[workspace]).unwrap_err();
        assert!(
            still_denied.contains("escapes the allowed directories"),
            "unexpected: {still_denied}"
        );
    }

    #[test]
    fn relative_traversal_out_of_an_allowed_root_is_still_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = dir.path().join("artifacts");
        let secrets = dir.path().join("secrets");
        for directory in [&artifacts, &secrets] {
            std::fs::create_dir(directory).unwrap();
        }
        std::fs::write(secrets.join("id_rsa"), "PRIVATE KEY").unwrap();

        let error = resolve_contained_path(
            "../secrets/id_rsa",
            "Route fulfill",
            &artifacts,
            &[artifacts.clone()],
        )
        .unwrap_err();
        assert!(
            error.contains("escapes the allowed directories"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn contained_paths_must_resolve_to_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = dir.path().join("artifacts");
        std::fs::create_dir_all(artifacts.join("nested")).unwrap();

        let error = resolve_contained_path("nested", "HAR replay", &artifacts, &[]).unwrap_err();
        assert!(error.contains("is not a file"), "unexpected: {error}");
    }

    #[test]
    fn fulfill_json_shorthand_sets_the_body_and_content_type() {
        let dir = tempfile::tempdir().unwrap();
        let normalized = normalize_route_handler(
            RouteHandler::Fulfill {
                status: 200,
                headers: BTreeMap::new(),
                body: None,
                path: None,
                json: Some(serde_json::json!({"source": "mocked"})),
                content_type: None,
                body_base64: false,
            },
            dir.path(),
            &[],
        )
        .unwrap();

        let RouteHandler::Fulfill {
            body,
            json,
            content_type,
            ..
        } = normalized
        else {
            panic!("expected a fulfill handler");
        };
        assert_eq!(body.as_deref(), Some(r#"{"source":"mocked"}"#));
        assert_eq!(json, None);
        assert_eq!(content_type.as_deref(), Some("application/json"));
    }

    #[test]
    fn fulfill_rejects_more_than_one_body_source() {
        let dir = tempfile::tempdir().unwrap();
        let error = normalize_route_handler(
            RouteHandler::Fulfill {
                status: 200,
                headers: BTreeMap::new(),
                body: Some("inline".to_string()),
                path: None,
                json: Some(serde_json::json!({})),
                content_type: None,
                body_base64: false,
            },
            dir.path(),
            &[],
        )
        .unwrap_err();

        assert!(error.contains("only one of"), "unexpected: {error}");
    }

    #[test]
    fn non_fulfill_handlers_pass_normalization_through_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let handler = RouteHandler::FetchAndFulfill {
            url: None,
            method: None,
            headers: None,
            post_data: None,
            status: Some(500),
            response_headers: BTreeMap::new(),
            body: None,
            body_base64: false,
        };

        assert_eq!(
            normalize_route_handler(handler.clone(), dir.path(), &[]).unwrap(),
            handler
        );
    }

    #[test]
    fn forbidden_request_headers_keep_their_original_values() {
        let original = BTreeMap::from([
            ("Cookie".to_string(), "session=real".to_string()),
            ("Host".to_string(), "example.com".to_string()),
            ("Content-Length".to_string(), "4".to_string()),
            ("Accept".to_string(), "text/html".to_string()),
        ]);
        let overrides = BTreeMap::from([
            ("cookie".to_string(), "session=forged".to_string()),
            ("host".to_string(), "evil.dev".to_string()),
            ("content-length".to_string(), "9999".to_string()),
            ("accept".to_string(), "application/json".to_string()),
        ]);

        let merged = merge_request_headers(&original, Some(&overrides));

        assert_eq!(merged["Cookie"], "session=real");
        assert_eq!(merged["Host"], "example.com");
        assert_eq!(merged["Content-Length"], "4");
        assert_eq!(merged["accept"], "application/json");
        assert!(!merged.contains_key("Accept"));
    }

    #[test]
    fn fetch_and_fulfill_reports_an_abort_when_the_upstream_request_fails() {
        let registry = RouteRegistry::default();
        registry
            .add(
                UrlPattern::Text("https://example.invalid/**".to_string()),
                RouteHandler::FetchAndFulfill {
                    url: None,
                    method: None,
                    headers: None,
                    post_data: None,
                    status: None,
                    response_headers: BTreeMap::new(),
                    body: None,
                    body_base64: false,
                },
                None,
            )
            .unwrap();

        let decision = decide_get(&registry, "one", "https://example.invalid/api");
        assert!(matches!(decision, RequestPausedDecision::Fail(_)));
        let reports = registry.drain_interceptions();
        assert_eq!(reports[0].action, "abort");
        assert!(reports[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("Route fetch failed")));
    }

    #[test]
    fn fetch_and_fulfill_validates_its_status_override_and_base64_body() {
        let registry = RouteRegistry::default();
        assert!(registry
            .add(
                UrlPattern::Text("https://example.com/**".to_string()),
                RouteHandler::FetchAndFulfill {
                    url: None,
                    method: None,
                    headers: None,
                    post_data: None,
                    status: Some(700),
                    response_headers: BTreeMap::new(),
                    body: None,
                    body_base64: false,
                },
                None,
            )
            .is_err());
        assert!(registry
            .add(
                UrlPattern::Text("https://example.com/**".to_string()),
                RouteHandler::FetchAndFulfill {
                    url: None,
                    method: None,
                    headers: None,
                    post_data: None,
                    status: None,
                    response_headers: BTreeMap::new(),
                    body: Some("not base64".to_string()),
                    body_base64: true,
                },
                None,
            )
            .is_err());
        assert!(registry.is_empty());
    }
}
