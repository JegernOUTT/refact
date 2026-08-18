pub use refact_integrations::browser_models::*;

use serde::Deserialize;
use serde_json::Value;

const MAX_STEP_PARSE_ERROR_CHARS: usize = 400;
const MAX_ACTION_SUGGESTIONS: usize = 3;
const MIN_ACTION_SUGGESTION_SCORE: usize = 3;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserActionRequestEnvelope {
    #[serde(default)]
    session: SessionPolicy,
    #[serde(default)]
    target: TabTarget,
    #[serde(default)]
    attach_screenshot: Option<bool>,
    #[serde(default)]
    page_context: Option<PageContextMode>,
    #[serde(default)]
    network: NetworkReportMode,
    #[serde(default)]
    block_service_workers: Option<bool>,
    steps: Vec<Value>,
}

pub fn parse_browser_action_request(value: Value) -> Result<BrowserActionRequest, String> {
    let envelope: BrowserActionRequestEnvelope =
        serde_json::from_value(value).map_err(|error| bounded_error_text(&error.to_string()))?;
    let mut steps = Vec::with_capacity(envelope.steps.len());
    for (index, raw_step) in envelope.steps.into_iter().enumerate() {
        steps.push(parse_browser_step(index, raw_step)?);
    }
    Ok(BrowserActionRequest {
        session: envelope.session,
        target: envelope.target,
        attach_screenshot: envelope.attach_screenshot,
        page_context: envelope.page_context,
        network: envelope.network,
        block_service_workers: envelope.block_service_workers,
        steps,
    })
}

fn parse_browser_step(index: usize, raw_step: Value) -> Result<BrowserStep, String> {
    let action = raw_step
        .get("action")
        .and_then(Value::as_str)
        .map(str::to_string);
    serde_json::from_value::<BrowserStep>(raw_step).map_err(|error| match action {
        Some(action) if BrowserStep::ACTION_NAMES.contains(&action.as_str()) => format!(
            "step[{index}] ({action}): {}",
            bounded_error_text(&error.to_string())
        ),
        Some(action) => format!(
            "step[{index}]: unknown action '{}'{}",
            bounded_error_text(&action),
            format_action_suggestions(&action)
        ),
        None => format!("step[{index}]: {}", bounded_error_text(&error.to_string())),
    })
}

fn format_action_suggestions(action: &str) -> String {
    let lowered = action.to_ascii_lowercase();
    let mut scored = BrowserStep::ACTION_NAMES
        .iter()
        .map(|candidate| (shared_prefix_len(&lowered, candidate), *candidate))
        .filter(|(score, _)| *score >= MIN_ACTION_SUGGESTION_SCORE)
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(right.1)));
    let names = scored
        .into_iter()
        .take(MAX_ACTION_SUGGESTIONS)
        .map(|(_, candidate)| candidate)
        .collect::<Vec<_>>();
    if names.is_empty() {
        String::new()
    } else {
        format!("; did you mean {}", names.join(", "))
    }
}

fn shared_prefix_len(left: &str, right: &str) -> usize {
    left.bytes()
        .zip(right.bytes())
        .take_while(|(left, right)| left == right)
        .count()
}

fn bounded_error_text(message: &str) -> String {
    let collapsed = refact_core::string_utils::redact_sensitive(message)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() <= MAX_STEP_PARSE_ERROR_CHARS {
        return collapsed;
    }
    let kept = collapsed
        .chars()
        .take(MAX_STEP_PARSE_ERROR_CHARS)
        .collect::<String>();
    format!("{kept}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_request() -> Value {
        serde_json::json!({
            "attach_screenshot": true,
            "network": "full",
            "steps": [
                {"action": "accessibility_snapshot"},
                {"action": "click", "locator": {"by": "ref", "value": "e5"}},
                {"action": "send_web_socket_message", "pattern": "wss://example.com/**", "text": "hi"}
            ]
        })
    }

    #[test]
    fn missing_required_field_names_the_step_index_and_action() {
        let error = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "accessibility_snapshot"},
                {"action": "click", "locator": {"by": "ref", "value": "e5"}},
                {"action": "send_web_socket_message", "text": "hi"}
            ]
        }))
        .unwrap_err();

        assert!(
            error.starts_with("step[2] (send_web_socket_message): "),
            "unexpected error: {error}"
        );
        assert!(error.contains("pattern"), "unexpected error: {error}");
        assert!(!error.contains('\n'), "unexpected error: {error}");
    }

    #[test]
    fn unknown_action_names_the_step_index_and_suggests_valid_actions() {
        let error = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "navigate", "url": "https://example.com"},
                {"action": "clic", "locator": {"by": "ref", "value": "e5"}}
            ]
        }))
        .unwrap_err();

        assert_eq!(
            error,
            "step[1]: unknown action 'clic'; did you mean click, click_if_exists"
        );
    }

    #[test]
    fn step_without_an_action_is_still_indexed() {
        let error =
            parse_browser_action_request(serde_json::json!({"steps": [{"url": "https://x.dev"}]}))
                .unwrap_err();

        assert!(error.starts_with("step[0]: "), "unexpected error: {error}");
        assert!(error.contains("action"), "unexpected error: {error}");
    }

    #[test]
    fn valid_batches_parse_identically_to_the_single_shot_parse() {
        let request = valid_request();
        let two_phase = parse_browser_action_request(request.clone()).unwrap();
        let single_shot: BrowserActionRequest = serde_json::from_value(request).unwrap();

        assert_eq!(
            serde_json::to_value(&two_phase).unwrap(),
            serde_json::to_value(&single_shot).unwrap()
        );
    }

    #[test]
    fn envelope_defaults_and_rejections_match_the_single_shot_parse() {
        let minimal = serde_json::json!({"steps": []});
        assert_eq!(
            serde_json::to_value(parse_browser_action_request(minimal.clone()).unwrap()).unwrap(),
            serde_json::to_value(
                serde_json::from_value::<BrowserActionRequest>(minimal.clone()).unwrap()
            )
            .unwrap()
        );

        for rejected in [
            serde_json::json!({}),
            serde_json::json!({"steps": {}}),
            serde_json::json!({"steps": [], "network": "Full"}),
            serde_json::json!({"steps": [], "target": {"type": "id"}}),
        ] {
            assert!(
                parse_browser_action_request(rejected.clone()).is_err(),
                "expected rejection: {rejected}"
            );
            assert!(
                serde_json::from_value::<BrowserActionRequest>(rejected.clone()).is_err(),
                "expected rejection: {rejected}"
            );
        }
    }

    #[test]
    fn renamed_fields_accept_both_canonical_and_legacy_names() {
        for (canonical, legacy) in [
            (
                serde_json::json!({"action": "send_web_socket_message", "pattern": "wss://example.com/**", "text": "hi"}),
                serde_json::json!({"action": "send_web_socket_message", "url_pattern": "wss://example.com/**", "data": "hi"}),
            ),
            (
                serde_json::json!({"action": "wait_for_url", "pattern": "/done"}),
                serde_json::json!({"action": "wait_for_url", "contains": "/done"}),
            ),
            (
                serde_json::json!({"action": "wait_for_request", "pattern": "/api"}),
                serde_json::json!({"action": "wait_for_request", "url_or_pattern": "/api"}),
            ),
            (
                serde_json::json!({"action": "wait_for_response", "pattern": {"source": "/api", "flags": "i"}}),
                serde_json::json!({"action": "wait_for_response", "url_or_pattern": {"source": "/api", "flags": "i"}}),
            ),
            (
                serde_json::json!({"action": "accessibility_snapshot", "locator": {"by": "css", "value": "main"}}),
                serde_json::json!({"action": "accessibility_snapshot", "root": {"by": "css", "value": "main"}}),
            ),
        ] {
            let from_canonical =
                parse_browser_action_request(serde_json::json!({"steps": [canonical.clone()]}))
                    .unwrap();
            let from_legacy =
                parse_browser_action_request(serde_json::json!({"steps": [legacy.clone()]}))
                    .unwrap();
            let canonical_value = serde_json::to_value(&from_canonical).unwrap();
            assert_eq!(
                canonical_value,
                serde_json::to_value(&from_legacy).unwrap(),
                "alias mismatch for {legacy}"
            );

            let serialized = serde_json::to_string(&canonical_value).unwrap();
            for legacy_name in [
                "url_pattern",
                "url_or_pattern",
                "contains",
                "\"data\"",
                "\"root\"",
            ] {
                assert!(
                    !serialized.contains(legacy_name),
                    "serialization must emit canonical names only, got {serialized}"
                );
            }
        }
    }

    #[test]
    fn canonical_and_legacy_names_together_are_rejected() {
        let error = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "wait_for_request", "pattern": "/api", "url_or_pattern": "/api"}]
        }))
        .unwrap_err();

        assert!(
            error.starts_with("step[0] (wait_for_request): "),
            "unexpected error: {error}"
        );
        assert!(
            error.contains("duplicate field"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn accessibility_snapshot_scoping_options_compose_in_a_batch() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [{
                "action": "accessibility_snapshot",
                "locator": {"by": "css", "value": "#dropdown"},
                "depth": 2,
                "boxes": true
            }]
        }))
        .unwrap();

        let BrowserStep::AccessibilitySnapshot { options } = &request.steps[0] else {
            panic!("Expected AccessibilitySnapshot");
        };
        assert_eq!(options.locator, Some(BrowserLocator::css("#dropdown")));
        assert_eq!(options.depth, Some(2));
        assert!(options.boxes);
    }

    #[test]
    fn accessibility_snapshot_rejects_root_and_locator_together() {
        let error = parse_browser_action_request(serde_json::json!({
            "steps": [{
                "action": "accessibility_snapshot",
                "locator": {"by": "css", "value": "main"},
                "root": {"by": "css", "value": "main"}
            }]
        }))
        .unwrap_err();

        assert!(
            error.starts_with("step[0] (accessibility_snapshot): "),
            "unexpected error: {error}"
        );
        assert!(
            error.contains("duplicate field"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn unknown_step_fields_are_rejected_with_the_step_index_and_action() {
        let error = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "navigate", "url": "https://example.com"},
                {"action": "extract_table", "locator": {"by": "css", "value": "table"}, "limitt": 5}
            ]
        }))
        .unwrap_err();

        assert!(
            error.starts_with("step[1] (extract_table): "),
            "unexpected error: {error}"
        );
        assert!(error.contains("limitt"), "unexpected error: {error}");
    }

    #[test]
    fn unknown_fields_are_rejected_on_flattened_option_steps() {
        let error = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "screenshot", "full_page": true, "save_ass": "/tmp/x.png"}]
        }))
        .unwrap_err();

        assert!(
            error.starts_with("step[0] (screenshot): "),
            "unexpected error: {error}"
        );
        assert!(error.contains("save_ass"), "unexpected error: {error}");

        parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "screenshot", "full_page": true, "type": "png"}]
        }))
        .unwrap();
    }

    #[test]
    fn reset_takes_no_parameters_and_batches_with_other_steps() {
        let request =
            parse_browser_action_request(serde_json::json!({"steps": [{"action": "reset"}]}))
                .unwrap();

        assert!(matches!(request.steps.as_slice(), [BrowserStep::Reset]));
        assert_eq!(
            serde_json::to_value(&request.steps[0]).unwrap(),
            serde_json::json!({"action": "reset"})
        );

        let batched = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "route", "pattern": "**/api/**", "handler": {"type": "abort", "reason": "failed"}},
                {"action": "reset"}
            ]
        }))
        .unwrap();

        assert!(matches!(
            batched.steps.as_slice(),
            [BrowserStep::Route { .. }, BrowserStep::Reset]
        ));
    }

    #[test]
    fn cancel_download_takes_an_optional_id_and_rejects_unknown_fields() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "cancel_download"}, {"action": "cancel_download", "id": "guid-1"}]
        }))
        .unwrap();

        assert!(matches!(
            request.steps.as_slice(),
            [
                BrowserStep::CancelDownload { id: None },
                BrowserStep::CancelDownload { id: Some(_) }
            ]
        ));
        assert_eq!(
            serde_json::to_value(&request.steps[0]).unwrap(),
            serde_json::json!({"action": "cancel_download"})
        );

        let error = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "cancel_download", "guid": "guid-1"}]
        }))
        .unwrap_err();
        assert!(
            error.starts_with("step[0] (cancel_download): "),
            "unexpected error: {error}"
        );
        assert!(error.contains("guid"), "unexpected error: {error}");
    }

    #[test]
    fn grant_permissions_defaults_to_granted_and_accepts_denied_or_prompt() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "grant_permissions", "permissions": ["geolocation"]},
                {"action": "grant_permissions", "permissions": ["notifications"], "state": "denied"},
                {"action": "grant_permissions", "permissions": ["midi"], "state": "prompt", "origin": "https://x.test"}
    fn cdp_send_defaults_to_the_page_target_and_omits_absent_params() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "cdp_send", "method": "Browser.getVersion", "target": "browser"},
                {"action": "cdp_send", "method": "Runtime.evaluate", "params": {"expression": "1+1"}}
            ]
        }))
        .unwrap();

        assert!(matches!(
            request.steps.as_slice(),
            [
                BrowserStep::GrantPermissions {
                    state: BrowserPermissionState::Granted,
                    ..
                },
                BrowserStep::GrantPermissions {
                    state: BrowserPermissionState::Denied,
                    ..
                },
                BrowserStep::GrantPermissions {
                    state: BrowserPermissionState::Prompt,
                    origin: Some(_),
                    ..
                }
            ]
        ));
        assert_eq!(
            serde_json::to_value(&request.steps[0]).unwrap(),
            serde_json::json!({"action": "grant_permissions", "permissions": ["geolocation"]})
        );
        assert_eq!(
            serde_json::to_value(&request.steps[1]).unwrap()["state"],
            "denied"
        );

        let error = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "grant_permissions", "permissions": [], "state": "allow"}]
        }))
        .unwrap_err();
        assert!(error.contains("allow"), "unexpected error: {error}");
    }

    #[test]
    fn har_update_and_storage_indexed_db_are_optional_additive_fields() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "start_har_recording", "mode": "full", "content": "embed", "update": "login.har"},
                {"action": "storage_state", "indexed_db": true},
                {"action": "set_storage_state", "state": {}, "indexed_db": false}
            ]
        }))
        .unwrap();

        assert!(matches!(
            request.steps.as_slice(),
            [
                BrowserStep::StartHarRecording {
                    path: None,
                    update: Some(_),
                    ..
                },
                BrowserStep::StorageState {
                    save_as: None,
                    indexed_db: Some(true)
                },
                BrowserStep::SetStorageState {
                    indexed_db: Some(false),
                    ..
                }
            ]
        ));

        let defaults = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "start_har_recording", "mode": "full", "content": "embed"},
                {"action": "storage_state"}
            ]
        }))
        .unwrap();
        assert!(matches!(
            defaults.steps.as_slice(),
            [
                BrowserStep::StartHarRecording { update: None, .. },
                BrowserStep::StorageState {
                    indexed_db: None,
                    ..
                }
            ]
        ));
        assert_eq!(
            serde_json::to_value(&defaults.steps[1]).unwrap(),
            serde_json::json!({"action": "storage_state"})
        let BrowserStep::CdpSend {
            method,
            params,
            target,
        } = &request.steps[0]
        else {
            panic!("expected a cdp_send step");
        };
        assert_eq!(method, "Browser.getVersion");
        assert_eq!(*params, None);
        assert_eq!(*target, CdpTarget::Browser);

        assert!(matches!(
            &request.steps[1],
            BrowserStep::CdpSend {
                target: CdpTarget::Page,
                params: Some(_),
                ..
            }
        ));
        assert_eq!(
            serde_json::to_value(&request.steps[1]).unwrap(),
            serde_json::json!({
                "action": "cdp_send",
                "method": "Runtime.evaluate",
                "params": {"expression": "1+1"},
                "target": "page"
            })
        );
    }

    #[test]
    fn block_service_workers_is_a_batch_level_option() {
        let request = parse_browser_action_request(serde_json::json!({
            "block_service_workers": true,
            "steps": [{"action": "reload"}]
        }))
        .unwrap();
        assert_eq!(request.block_service_workers, Some(true));

        let absent =
            parse_browser_action_request(serde_json::json!({"steps": [{"action": "reload"}]}))
                .unwrap();
        assert_eq!(absent.block_service_workers, None);
        assert!(
            serde_json::to_value(&absent).unwrap()["block_service_workers"].is_null(),
            "absent batch option must not serialize"
        );

        let error = parse_browser_action_request(serde_json::json!({
            "block_service_worker": true,
            "steps": []
        }))
        .unwrap_err();
        assert!(
            error.contains("block_service_worker"),
            "unexpected error: {error}"
        );
    fn cdp_send_requires_a_method_and_rejects_unknown_fields_and_targets() {
        for (step, expected) in [
            (
                serde_json::json!({"action": "cdp_send", "params": {}}),
                "method",
            ),
            (
                serde_json::json!({"action": "cdp_send", "method": "Page.reload", "sessionId": "x"}),
                "sessionId",
            ),
            (
                serde_json::json!({"action": "cdp_send", "method": "Page.reload", "target": "tab"}),
                "tab",
            ),
        ] {
            let error = parse_browser_action_request(serde_json::json!({"steps": [step.clone()]}))
                .unwrap_err();
            assert!(
                error.starts_with("step[0] (cdp_send): "),
                "unexpected error for {step}: {error}"
            );
            assert!(error.contains(expected), "unexpected error: {error}");
        }
    }

    #[test]
    fn tap_parses_both_the_locator_and_the_coordinate_shape() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "tap", "locator": {"by": "ref", "value": "e5"}},
                {"action": "tap", "x": 10.0, "y": 20.0}
            ]
        }))
        .unwrap();

        assert!(matches!(
            request.steps.as_slice(),
            [
                BrowserStep::Tap {
                    locator: Some(_),
                    x: None,
                    y: None
                },
                BrowserStep::Tap {
                    locator: None,
                    x: Some(_),
                    y: Some(_)
                }
            ]
        ));
        assert_eq!(
            serde_json::to_value(&request.steps[1]).unwrap(),
            serde_json::json!({"action": "tap", "x": 10.0, "y": 20.0})
        );
    }

    #[test]
    fn insert_text_locator_is_optional_and_press_sequentially_requires_one() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "insert_text", "text": "hi"},
                {"action": "press_sequentially", "locator": {"by": "css", "value": "#q"}, "text": "hi"}
            ]
        }))
        .unwrap();

        assert!(matches!(
            request.steps.as_slice(),
            [
                BrowserStep::InsertText { locator: None, .. },
                BrowserStep::PressSequentially { delay_ms: None, .. }
            ]
        ));

        let error = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "press_sequentially", "text": "hi"}]
        }))
        .unwrap_err();

        assert!(
            error.starts_with("step[0] (press_sequentially): "),
            "unexpected error: {error}"
        );
        assert!(error.contains("locator"), "unexpected error: {error}");
    }

    #[test]
    fn http_request_defaults_to_a_bare_get_and_keeps_its_optional_fields() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "http_request", "url": "https://example.com/api/session"}]
        }))
        .unwrap();

        let BrowserStep::HttpRequest { options } = &request.steps[0] else {
            panic!("expected an http_request step");
        };
        assert_eq!(options.url, "https://example.com/api/session");
        assert_eq!(options.method, None);
        assert!(options.headers.is_empty());
        assert_eq!(options.body, None);
        assert_eq!(options.form, None);
        assert_eq!(options.fail_on_status, None);
        assert_eq!(
            serde_json::to_value(&request.steps[0]).unwrap(),
            serde_json::json!({"action": "http_request", "url": "https://example.com/api/session"})
        );

        let full = parse_browser_action_request(serde_json::json!({
            "steps": [{
                "action": "http_request",
                "url": "https://example.com/api/items",
                "method": "post",
                "headers": {"accept": "application/json"},
                "body_json": {"name": "widget"},
                "timeout_ms": 1_000,
                "max_redirects": 0,
                "fail_on_status": true,
                "full_headers": true
            }]
        }))
        .unwrap();
        let BrowserStep::HttpRequest { options } = &full.steps[0] else {
            panic!("expected an http_request step");
        };
        assert_eq!(options.method.as_deref(), Some("post"));
        assert_eq!(options.body_json, Some(serde_json::json!({"name": "widget"})));
        assert_eq!(options.max_redirects, Some(0));
        assert_eq!(options.fail_on_status, Some(true));
        assert_eq!(options.full_headers, Some(true));
    }

    #[test]
    fn http_request_requires_a_url_and_rejects_unknown_fields() {
        let missing = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "http_request", "method": "GET"}]
        }))
        .unwrap_err();
        assert!(
            missing.starts_with("step[0] (http_request): "),
            "unexpected error: {missing}"
        );
        assert!(missing.contains("url"), "unexpected error: {missing}");

        let unknown = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "http_request", "url": "https://x.test/", "bodyy": "oops"}]
        }))
        .unwrap_err();
        assert!(
            unknown.starts_with("step[0] (http_request): "),
            "unexpected error: {unknown}"
        );
        assert!(unknown.contains("bodyy"), "unexpected error: {unknown}");
    }

    #[test]
    fn route_accepts_times_and_the_new_chain_handlers() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "route", "pattern": "**/api/**", "handler": {"type": "fulfill", "json": {"ok": true}}, "times": 2},
                {"action": "route", "pattern": "**/api/**", "handler": {"type": "fallback"}},
                {"action": "route", "pattern": "**/api/**", "handler": {"type": "fetch_and_fulfill", "status": 503}}
            ]
        }))
        .unwrap();

        assert!(matches!(
            request.steps[0],
            BrowserStep::Route {
                times: Some(2),
                handler: RouteHandler::Fulfill { status: 200, .. },
                ..
            }
        ));
        assert!(matches!(
            request.steps[1],
            BrowserStep::Route {
                times: None,
                handler: RouteHandler::Fallback,
                ..
            }
        ));
        assert!(matches!(
            request.steps[2],
            BrowserStep::Route {
                handler: RouteHandler::FetchAndFulfill {
                    status: Some(503),
                    ..
                },
                ..
            }
        ));

        assert_eq!(
            serde_json::to_value(&request.steps[1]).unwrap(),
            serde_json::json!({
                "action": "route",
                "pattern": "**/api/**",
                "handler": {"type": "fallback"}
            })
        );
    }

    #[test]
    fn fulfill_path_shorthand_round_trips_without_a_status() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "route", "pattern": "**/app.css", "handler": {"type": "fulfill", "path": "mock.css"}}]
        }))
        .unwrap();

        let BrowserStep::Route {
            handler: RouteHandler::Fulfill { status, path, .. },
            ..
        } = &request.steps[0]
        else {
            panic!("expected a fulfill route");
        };
        assert_eq!(*status, 200);
        assert_eq!(path.as_deref(), Some("mock.css"));
    }

    #[test]
    fn screencast_steps_parse_with_defaults_and_reject_unknown_fields() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "capture_frames"},
                {"action": "capture_frames", "duration_ms": 2000, "interval_ms": 500, "full_page": true},
                {"action": "screencast_start"},
                {"action": "screencast_stop", "compose": false}
            ]
        }))
        .unwrap();

        assert!(matches!(
            request.steps.as_slice(),
            [
                BrowserStep::CaptureFrames {
                    duration_ms: None,
                    frame_count: None,
                    interval_ms: None,
                    locator: None,
                    full_page: None,
                },
                BrowserStep::CaptureFrames {
                    duration_ms: Some(2000),
                    interval_ms: Some(500),
                    full_page: Some(true),
                    ..
                },
                BrowserStep::ScreencastStart {
                    quality: None,
                    max_width: None,
                    max_height: None,
                },
                BrowserStep::ScreencastStop {
                    compose: Some(false)
                },
            ]
        ));
        assert_eq!(
            serde_json::to_value(&request.steps[0]).unwrap(),
            serde_json::json!({"action": "capture_frames"})
        );

        let error = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "capture_frames", "frames": 4}]
        }))
        .unwrap_err();

        assert!(
            error.starts_with("step[0] (capture_frames): unknown field `frames`"),
            "unexpected error: {error}"
        );
    }

    fn expect_matcher(step: Value) -> BrowserExpectation {
        let request = parse_browser_action_request(serde_json::json!({"steps": [step]})).unwrap();
        let BrowserStep::Expect { matcher, .. } = request.steps.into_iter().next().unwrap() else {
            panic!("expected an expect step");
        };
        matcher
    }

    #[test]
    fn expect_not_is_optional_and_omitted_from_serialization_when_absent() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "expect", "locator": {"by": "css", "value": "#a"}, "matcher": {"type": "to_be_visible"}},
                {"action": "expect", "locator": {"by": "css", "value": "#a"}, "matcher": {"type": "to_be_visible"}, "not": true}
            ]
        }))
        .unwrap();

        assert!(matches!(
            request.steps.as_slice(),
            [
                BrowserStep::Expect { not: None, .. },
                BrowserStep::Expect {
                    not: Some(true),
                    ..
                }
            ]
        ));
        assert_eq!(
            serde_json::to_value(&request.steps[0]).unwrap(),
            serde_json::json!({
                "action": "expect",
                "locator": {"by": "css", "value": "#a"},
                "matcher": {"type": "to_be_visible"},
                "soft": false
            })
        );
    }

    #[test]
    fn to_be_checked_keeps_its_bare_form_and_gains_checked_and_indeterminate() {
        assert_eq!(
            expect_matcher(serde_json::json!({
                "action": "expect",
                "locator": {"by": "css", "value": "#a"},
                "matcher": {"type": "to_be_checked"}
            })),
            BrowserExpectation::ToBeChecked {
                checked: None,
                indeterminate: None
            }
        );
        assert_eq!(
            expect_matcher(serde_json::json!({
                "action": "expect",
                "locator": {"by": "css", "value": "#a"},
                "matcher": {"type": "to_be_checked", "indeterminate": true}
            })),
            BrowserExpectation::ToBeChecked {
                checked: None,
                indeterminate: Some(true)
            }
        );
        assert_eq!(
            serde_json::to_value(BrowserExpectation::ToBeChecked {
                checked: None,
                indeterminate: None
            })
            .unwrap(),
            serde_json::json!({"type": "to_be_checked"})
        );
    }

    #[test]
    fn to_be_checked_rejects_indeterminate_combined_with_a_checked_expectation() {
        let error = BrowserExpectation::ToBeChecked {
            checked: Some(true),
            indeterminate: Some(true),
        }
        .validate()
        .unwrap_err();
        assert!(error.contains("indeterminate"), "unexpected error: {error}");

        assert!(BrowserExpectation::ToBeChecked {
            checked: Some(false),
            indeterminate: Some(false),
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn to_have_css_pseudo_parses_and_maps_to_a_selector() {
        assert_eq!(
            expect_matcher(serde_json::json!({
                "action": "expect",
                "locator": {"by": "css", "value": "#a"},
                "matcher": {"type": "to_have_css", "name": "content", "expected": "\"done\"", "pseudo": "before"}
            })),
            BrowserExpectation::ToHaveCss {
                name: "content".to_string(),
                expected: BrowserExpectedText::Text("\"done\"".to_string()),
                ignore_case: false,
                pseudo: Some(BrowserPseudoElement::Before)
            }
        );
        assert_eq!(BrowserPseudoElement::Before.selector(), "::before");
        assert_eq!(BrowserPseudoElement::After.selector(), "::after");

        let error = parse_browser_action_request(serde_json::json!({
            "steps": [{
                "action": "expect",
                "locator": {"by": "css", "value": "#a"},
                "matcher": {"type": "to_have_css", "name": "content", "expected": "x", "pseudo": "marker"}
            }]
        }))
        .unwrap_err();
        assert!(error.contains("marker"), "unexpected error: {error}");
    }

    #[test]
    fn to_have_attribute_expected_is_optional_for_presence_only_checks() {
        assert_eq!(
            expect_matcher(serde_json::json!({
                "action": "expect",
                "locator": {"by": "css", "value": "#a"},
                "matcher": {"type": "to_have_attribute", "name": "disabled"}
            })),
            BrowserExpectation::ToHaveAttribute {
                name: "disabled".to_string(),
                expected: None,
                ignore_case: false
            }
        );
        assert_eq!(
            serde_json::to_value(BrowserExpectation::ToHaveAttribute {
                name: "disabled".to_string(),
                expected: None,
                ignore_case: false
            })
            .unwrap(),
            serde_json::json!({"type": "to_have_attribute", "name": "disabled", "ignore_case": false})
        );
    }

    #[test]
    fn to_be_in_viewport_ratio_parses_and_is_bounded() {
        assert_eq!(
            expect_matcher(serde_json::json!({
                "action": "expect",
                "locator": {"by": "css", "value": "#a"},
                "matcher": {"type": "to_be_in_viewport", "ratio": 0.5}
            })),
            BrowserExpectation::ToBeInViewport { ratio: Some(0.5) }
        );
        assert_eq!(
            expect_matcher(serde_json::json!({
                "action": "expect",
                "locator": {"by": "css", "value": "#a"},
                "matcher": {"type": "to_be_in_viewport"}
            })),
            BrowserExpectation::ToBeInViewport { ratio: None }
        );
        assert!(BrowserExpectation::ToBeInViewport { ratio: Some(1.0) }
            .validate()
            .is_ok());
        assert!(BrowserExpectation::ToBeInViewport { ratio: Some(1.5) }
            .validate()
            .is_err());
        assert!(BrowserExpectation::ToBeInViewport { ratio: Some(-0.1) }
            .validate()
            .is_err());
    }

    #[test]
    fn text_matchers_accept_a_string_a_regex_or_a_list() {
        assert_eq!(
            expect_matcher(serde_json::json!({
                "action": "expect",
                "locator": {"by": "css", "value": "li"},
                "matcher": {"type": "to_have_text", "expected": ["One", {"source": "Tw.", "flags": "i"}]}
            })),
            BrowserExpectation::ToHaveText {
                expected: BrowserExpectedTextOrList::Many(vec![
                    BrowserExpectedText::Text("One".to_string()),
                    BrowserExpectedText::Regex(LocatorRegex {
                        source: "Tw.".to_string(),
                        flags: "i".to_string()
                    })
                ]),
                ignore_case: false
            }
        );
        assert_eq!(
            expect_matcher(serde_json::json!({
                "action": "expect",
                "locator": {"by": "css", "value": "li"},
                "matcher": {"type": "to_contain_text", "expected": "One"}
            })),
            BrowserExpectation::ToContainText {
                expected: BrowserExpectedTextOrList::One(BrowserExpectedText::Text(
                    "One".to_string()
                )),
                ignore_case: false
            }
        );
    }

    #[test]
    fn page_context_defaults_to_snapshot_and_accepts_every_mode() {
        let omitted = parse_browser_action_request(serde_json::json!({"steps": []})).unwrap();
        assert_eq!(omitted.page_context, None);
        assert_eq!(omitted.page_context_mode(), PageContextMode::Snapshot);

        for (raw, expected) in [
            ("snapshot", PageContextMode::Snapshot),
            ("screenshot", PageContextMode::Screenshot),
            ("both", PageContextMode::Both),
            ("none", PageContextMode::None),
        ] {
            let parsed =
                parse_browser_action_request(serde_json::json!({"page_context": raw, "steps": []}))
                    .unwrap();
            assert_eq!(parsed.page_context, Some(expected));
            assert_eq!(parsed.page_context_mode(), expected);
            assert_eq!(serde_json::to_value(expected).unwrap(), raw);
        }
    }

    #[test]
    fn page_context_is_rejected_when_it_is_not_a_known_mode() {
        for rejected in [
            serde_json::json!({"page_context": "Snapshot", "steps": []}),
            serde_json::json!({"page_context": "aria", "steps": []}),
            serde_json::json!({"page_context": true, "steps": []}),
        ] {
            assert!(
                parse_browser_action_request(rejected.clone()).is_err(),
                "expected rejection: {rejected}"
            );
        }
    }

    #[test]
    fn page_context_composes_with_the_attach_screenshot_override() {
        let request = parse_browser_action_request(serde_json::json!({
            "page_context": "none",
            "attach_screenshot": true,
            "steps": [{"action": "navigate", "url": "https://example.com"}]
        }))
        .unwrap();

        assert_eq!(request.page_context_mode(), PageContextMode::None);
        assert_eq!(request.attach_screenshot, Some(true));
        assert_eq!(
            serde_json::to_value(&request).unwrap()["page_context"],
            serde_json::json!("none")
        );
    }

    #[test]
    fn unknown_batch_envelope_fields_are_rejected() {
        let error = parse_browser_action_request(serde_json::json!({
            "steps": [],
            "netwrok": "full"
        }))
        .unwrap_err();

        assert!(error.contains("netwrok"), "unexpected error: {error}");
    }

    #[test]
    fn step_errors_are_single_line_and_bounded() {
        let error = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "x".repeat(4000)}]
        }))
        .unwrap_err();

        assert!(!error.contains('\n'), "unexpected error: {error}");
        assert!(error.chars().count() < 600, "unexpected error: {error}");
    }

    #[test]
    fn gallery_and_state_steps_parse_with_flattened_screenshot_options() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "screenshot_elements", "locators": [{"by": "css", "value": ".card"}], "compose": "separate", "mask": [{"by": "css", "value": ".secret"}]},
                {"action": "capture_element_states", "locator": {"by": "css", "value": "button"}, "states": ["hover", "active"], "labels": true}
            ]
        }))
        .unwrap();

        match &request.steps[0] {
            BrowserStep::ScreenshotElements {
                locators,
                compose,
                options,
                ..
            } => {
                assert_eq!(locators.len(), 1);
                assert_eq!(*compose, BrowserComposeMode::Separate);
                assert_eq!(options.mask.len(), 1);
            }
            other => panic!("unexpected step: {other:?}"),
        }
        match &request.steps[1] {
            BrowserStep::CaptureElementStates { states, labels, .. } => {
                assert_eq!(
                    states,
                    &[BrowserElementState::Hover, BrowserElementState::Active]
                );
                assert_eq!(*labels, Some(true));
            }
            other => panic!("unexpected step: {other:?}"),
        }
    }

    #[test]
    fn throttling_and_device_steps_parse_their_parameters() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "set_network_conditions", "preset": "slow-3g", "latency_ms": 120.0, "offline": false},
                {"action": "set_cpu_throttling", "rate": 4.0},
                {"action": "emulate_device", "name": "iPhone 13"},
                {"action": "list_devices"},
            ]
        }))
        .unwrap();

        match &request.steps[0] {
            BrowserStep::SetNetworkConditions {
                offline,
                latency_ms,
                download_kbps,
                upload_kbps,
                preset,
            } => {
                assert_eq!(*offline, Some(false));
                assert_eq!(*latency_ms, Some(120.0));
                assert_eq!(*download_kbps, None);
                assert_eq!(*upload_kbps, None);
                assert_eq!(preset.as_deref(), Some("slow-3g"));
            }
            other => panic!("unexpected step: {other:?}"),
        }
        assert!(matches!(
            request.steps[1],
            BrowserStep::SetCpuThrottling { rate } if rate == 4.0
        ));
        assert!(matches!(
            &request.steps[2],
            BrowserStep::EmulateDevice { name } if name == "iPhone 13"
        ));
        assert!(matches!(
            &request.steps[3],
            BrowserStep::ListDevices { filter: None }
        ));
    }

    #[test]
    fn throttling_and_device_steps_reject_unknown_and_missing_fields() {
        for (step, expected) in [
            (
                serde_json::json!({"action": "set_network_conditions", "latencyMs": 100}),
                "latencyMs",
            ),
            (serde_json::json!({"action": "set_cpu_throttling"}), "rate"),
            (serde_json::json!({"action": "emulate_device"}), "name"),
            (
                serde_json::json!({"action": "list_devices", "contains": "pixel"}),
                "contains",
            ),
        ] {
            let error = parse_browser_action_request(serde_json::json!({"steps": [step.clone()]}))
                .unwrap_err();
            assert!(error.starts_with("step[0] ("), "unexpected error: {error}");
            assert!(error.contains(expected), "unexpected error: {error}");
        }
    }

    #[test]
    fn gallery_and_state_steps_reject_unknown_fields_and_bad_enum_values() {
        for (step, expected) in [
            (
                serde_json::json!({"action": "screenshot_elements", "locators": [], "composee": "grid"}),
                "composee",
            ),
            (
                serde_json::json!({"action": "screenshot_elements", "locators": [], "compose": "mosaic"}),
                "mosaic",
            ),
            (
                serde_json::json!({"action": "capture_element_states", "locator": {"by": "css", "value": "button"}, "states": ["visited"]}),
                "visited",
            ),
        ] {
            let error = parse_browser_action_request(serde_json::json!({"steps": [step.clone()]}))
                .unwrap_err();
            assert!(error.starts_with("step[0] ("), "unexpected error: {error}");
            assert!(error.contains(expected), "unexpected error: {error}");
        }
    }
}
