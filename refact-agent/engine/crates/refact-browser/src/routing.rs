use std::collections::BTreeMap;
use std::sync::Mutex;

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
    pub fn add(&self, pattern: UrlPattern, handler: RouteHandler) -> Result<(), String> {
        let matcher = matcher_for_pattern(&pattern)?;
        validate_handler(&handler)?;
        self.state.lock().unwrap().routes.push(RegisteredRoute {
            info: RouteInfo {
                pattern,
                handler,
                har: None,
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
            .map(|route| masked_route_info(&route.info))
            .chain(state.har_replay.as_ref().map(har_route_info))
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
        let mut state = self.state.lock().unwrap();
        let route = state
            .routes
            .iter()
            .find(|route| route.matcher.is_match(&url))
            .cloned();
        let (pattern, handler) = if let Some(route) = route {
            (route.info.pattern, route.info.handler)
        } else if let Some(handler) = state
            .har_replay
            .as_ref()
            .and_then(|replay| replay.match_request(&method, &url))
        {
            (UrlPattern::Text(HAR_ROUTE_PATTERN.to_string()), handler)
        } else {
            return RequestPausedDecision::Continue(None);
        };

        let (decision, action, status, reason, response_body_preview) = match &handler {
            RouteHandler::Fulfill {
                status,
                headers,
                body,
                content_type,
                body_base64,
            } => {
                let mut headers = headers.clone();
                if let Some(content_type) = content_type {
                    if !headers
                        .keys()
                        .any(|name| name.eq_ignore_ascii_case("content-type"))
                    {
                        headers.insert("Content-Type".to_string(), content_type.clone());
                    }
                }
                let body_wire = body.as_ref().map(|body| {
                    if *body_base64 {
                        body.clone()
                    } else {
                        base64::engine::general_purpose::STANDARD.encode(body.as_bytes())
                    }
                });
                let response_body_preview = body.as_ref().map(|body| {
                    if *body_base64 {
                        "[base64 body]".to_string()
                    } else {
                        mask_text(body)
                    }
                });
                (
                    RequestPausedDecision::Fulfill(Fetch::FulfillRequest {
                        request_id,
                        response_code: *status as u32,
                        response_headers: Some(headers_to_cdp(&headers)),
                        binary_response_headers: None,
                        body: body_wire,
                        response_phrase: None,
                    }),
                    "fulfill".to_string(),
                    Some(*status),
                    None,
                    response_body_preview,
                )
            }
            RouteHandler::Abort { reason } => (
                RequestPausedDecision::Fail(Fetch::FailRequest {
                    request_id,
                    error_reason: parse_error_reason(reason)
                        .unwrap_or(Network::ErrorReason::Failed),
                }),
                "abort".to_string(),
                None,
                Some(reason.clone()),
                None,
            ),
            RouteHandler::Continue {
                url,
                method,
                headers,
                post_data,
            } => (
                RequestPausedDecision::Continue(Some(Fetch::ContinueRequest {
                    request_id,
                    url: url.clone(),
                    method: method.clone(),
                    post_data: post_data.as_ref().map(|data| {
                        base64::engine::general_purpose::STANDARD.encode(data.as_bytes())
                    }),
                    headers: headers.as_ref().map(|overrides| {
                        headers_to_cdp(&merge_headers(&request_headers, overrides))
                    }),
                    intercept_response: None,
                })),
                "continue".to_string(),
                None,
                None,
                None,
            ),
        };

        state.interceptions.push(RouteInterception {
            url: mask_text(&url),
            method,
            pattern: mask_pattern(&pattern),
            action,
            request_headers: mask_headers(request_headers),
            request_body_preview: post_data.map(|data| mask_text(&data)),
            response_body_preview,
            status,
            reason,
            redirect_hop,
        });
        if state.interceptions.len() > INTERCEPTION_REPORT_CAP {
            let excess = state.interceptions.len() - INTERCEPTION_REPORT_CAP;
            state.interceptions.drain(..excess);
        }
        decision
    }
}

fn matcher_for_pattern(pattern: &UrlPattern) -> Result<UrlMatcher, String> {
    match pattern {
        UrlPattern::Text(value) => UrlMatcher::text(value),
        UrlPattern::Regex { source, flags } => UrlMatcher::regex(source, flags),
    }
}

fn validate_handler(handler: &RouteHandler) -> Result<(), String> {
    match handler {
        RouteHandler::Fulfill { status, .. } if !(100..=599).contains(status) => Err(format!(
            "Route fulfill status must be between 100 and 599, got {status}"
        )),
        RouteHandler::Fulfill {
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

fn masked_route_info(info: &RouteInfo) -> RouteInfo {
    let handler = match &info.handler {
        RouteHandler::Fulfill {
            status,
            headers,
            body,
            content_type,
            body_base64,
        } => RouteHandler::Fulfill {
            status: *status,
            headers: mask_headers(headers.clone()),
            body: body.as_ref().map(|body| {
                if *body_base64 {
                    "[base64 body]".to_string()
                } else {
                    mask_text(body)
                }
            }),
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
    };
    RouteInfo {
        pattern: mask_pattern(&info.pattern),
        handler,
        har: info.har.clone(),
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

    fn har_replay(dir: &tempfile::TempDir, name: &str, not_found: HarNotFound) -> HarReplay {
        let path = dir.path().join(name);
        std::fs::write(&path, HAR_FIXTURE).unwrap();
        HarReplay::load(&path, None, not_found).unwrap()
    }

    #[test]
    fn registry_add_remove_and_clear_reuse_url_matcher() {
        let registry = RouteRegistry::default();
        let (pattern, handler) = route(
            "https://example.com/**",
            RouteHandler::Abort {
                reason: "blockedbyclient".to_string(),
            },
        );
        registry.add(pattern.clone(), handler).unwrap();
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
                    content_type: None,
                    body_base64: false,
                },
            )
            .is_err());
        assert!(registry
            .add(
                UrlPattern::Text("https://example.com/**".to_string()),
                RouteHandler::Fulfill {
                    status: 200,
                    headers: BTreeMap::new(),
                    body: Some("not base64".to_string()),
                    content_type: None,
                    body_base64: true,
                },
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
        let (pattern, handler) = route(
            "https://example.com/**",
            RouteHandler::Abort {
                reason: "blockedbyclient".to_string(),
            },
        );
        registry.add(pattern, handler).unwrap();
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
        let (pattern, handler) = route(
            "https://example.com/**",
            RouteHandler::Abort {
                reason: "blockedbyclient".to_string(),
            },
        );
        registry.add(pattern.clone(), handler).unwrap();
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

        let decision = registry.decide(
            "one".to_string(),
            "https://example.com/missing".to_string(),
            "GET".to_string(),
            BTreeMap::new(),
            None,
            false,
        );
        assert!(matches!(decision, RequestPausedDecision::Fail(_)));

        let reports = registry.drain_interceptions();
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].pattern,
            UrlPattern::Text("har-replay".to_string())
        );
        assert_eq!(reports[0].action, "abort");

        registry.remove(None);
        let after = registry.decide(
            "two".to_string(),
            "https://example.com/missing".to_string(),
            "GET".to_string(),
            BTreeMap::new(),
            None,
            false,
        );
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
                content_type: Some("text/plain".to_string()),
                body_base64: false,
            },
            RouteHandler::Abort {
                reason: "blockedbyclient".to_string(),
            },
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
                )
                .unwrap();
            let decision = registry.decide(
                index.to_string(),
                "https://example.com/api".to_string(),
                "GET".to_string(),
                BTreeMap::new(),
                None,
                false,
            );
            match index {
                0 => assert!(matches!(decision, RequestPausedDecision::Fulfill(_))),
                1 => assert!(matches!(decision, RequestPausedDecision::Fail(_))),
                _ => assert!(matches!(decision, RequestPausedDecision::Continue(Some(_)))),
            }
            assert_eq!(registry.drain_interceptions().len(), 1);
        }
    }
}
