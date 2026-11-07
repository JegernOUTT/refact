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
    #[serde(default)]
    continue_on_error: bool,
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
        continue_on_error: envelope.continue_on_error,
        steps,
    })
}

fn parse_browser_step(index: usize, raw_step: Value) -> Result<BrowserStep, String> {
    let action = raw_step
        .get("action")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let (Some(action), Some(object)) = (action.as_deref(), raw_step.as_object()) {
        if let Some(stray) = stray_field_on_fieldless_action(action, object) {
            return Err(format!(
                "step[{index}] ({action}): unknown field `{}`; `{action}` takes no fields",
                bounded_error_text(stray)
            ));
        }
    }
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

fn stray_field_on_fieldless_action<'a>(
    action: &str,
    object: &'a serde_json::Map<String, Value>,
) -> Option<&'a str> {
    let (_, required, optional, aliases) = BrowserStep::ACTION_FIELDS
        .iter()
        .find(|(name, ..)| *name == action)?;
    if !(required.is_empty() && optional.is_empty() && aliases.is_empty()) {
        return None;
    }
    object
        .keys()
        .find(|key| key.as_str() != "action")
        .map(String::as_str)
}

fn format_action_suggestions(action: &str) -> String {
    let lowered = action.to_ascii_lowercase().replace('-', "_");
    let max_distance = (lowered.len() / 3).max(2);
    let mut scored = BrowserStep::ACTION_NAMES
        .iter()
        .filter_map(|candidate| {
            let distance = edit_distance(&lowered, candidate);
            let prefix = shared_prefix_len(&lowered, candidate);
            let tier = if candidate.starts_with(&lowered) || lowered.starts_with(candidate) {
                0
            } else if distance <= max_distance {
                1
            } else if prefix >= MIN_ACTION_SUGGESTION_SCORE {
                2
            } else {
                return None;
            };
            Some((tier, distance, prefix, *candidate))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(left.1.cmp(&right.1))
            .then(right.2.cmp(&left.2))
            .then(left.3.cmp(right.3))
    });
    let names = scored
        .into_iter()
        .take(MAX_ACTION_SUGGESTIONS)
        .map(|(_, _, _, candidate)| candidate)
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

fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (i, left_char) in left.iter().enumerate() {
        current[0] = i + 1;
        for (j, right_char) in right.iter().enumerate() {
            let substitution = previous[j] + usize::from(left_char != right_char);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
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

    fn assert_step_rejection(step: Value, expected: &str) {
        let error =
            parse_browser_action_request(serde_json::json!({"steps": [step.clone()]})).unwrap_err();
        assert!(
            error.starts_with("step[0] ("),
            "expected an action-qualified error for {step}: {error}"
        );
        assert!(
            error.contains(expected),
            "unexpected error for {step}: {error}"
        );
    }

    #[test]
    fn fieldless_actions_reject_stray_fields_like_every_other_action() {
        assert_step_rejection(
            serde_json::json!({"action": "reload", "bogus_field_xyz": 1}),
            "unknown field `bogus_field_xyz`; `reload` takes no fields",
        );
        assert_step_rejection(
            serde_json::json!({"action": "list_tabs", "timeout_ms": 5000}),
            "`list_tabs` takes no fields",
        );
        assert_step_rejection(
            serde_json::json!({"action": "tab_log", "locator": {"by": "css", "value": "#x"}}),
            "`tab_log` takes no fields",
        );
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "reload"}, {"action": "hide_highlight"}],
            "continue_on_error": true
        }))
        .unwrap();
        assert_eq!(request.steps.len(), 2);
        assert!(request.continue_on_error);
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
    fn cdp_send_defaults_to_the_page_target_and_omits_absent_params() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [
                {"action": "cdp_send", "method": "Browser.getVersion", "target": "browser"},
                {"action": "cdp_send", "method": "Runtime.evaluate", "params": {"expression": "1+1"}}
            ]
        }))
        .unwrap();

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
    fn http_request_defaults_to_a_bare_get_and_keeps_its_optional_fields() {
        let request = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "http_request", "url": "https://example.com/api/session"}]
        }))
        .unwrap();

        let BrowserStep::HttpRequest {
            options,
        } = &request.steps[0]
        else {
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
        let BrowserStep::HttpRequest {
            options,
        } = &full.steps[0]
        else {
            panic!("expected an http_request step");
        };
        assert_eq!(options.method.as_deref(), Some("post"));
        assert_eq!(
            options.body_json,
            Some(serde_json::json!({"name": "widget"}))
        );
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
    fn step_errors_are_single_line_and_bounded() {
        let error = parse_browser_action_request(serde_json::json!({
            "steps": [{"action": "x".repeat(4000)}]
        }))
        .unwrap_err();

        assert!(!error.contains('\n'), "unexpected error: {error}");
        assert!(error.chars().count() < 600, "unexpected error: {error}");
    }

    #[test]
    fn web_socket_steps_reject_unknown_fields_and_bad_modes() {
        for (step, expected) in [
            (serde_json::json!({"action": "route_web_socket"}), "pattern"),
            (
                serde_json::json!({"action": "route_web_socket", "pattern": "wss://x/**", "mode": "record"}),
                "record",
            ),
            (
                serde_json::json!({"action": "route_web_socket", "pattern": "wss://x/**", "on_page_message": "ignore"}),
                "ignore",
            ),
            (
                serde_json::json!({"action": "route_web_socket", "pattern": "wss://x/**", "on_message": "drop"}),
                "on_message",
            ),
            (serde_json::json!({"action": "close_web_socket"}), "pattern"),
            (
                serde_json::json!({"action": "close_web_socket", "pattern": "wss://x/**", "close_code": 1000}),
                "close_code",
            ),
        ] {
            assert_step_rejection(step, expected);
        }
    }

    #[test]
    fn lifecycle_steps_reject_unknown_and_missing_fields() {
        for (step, expected) in [
            (
                serde_json::json!({"action": "cancel_download", "download_id": "dl-1"}),
                "download_id",
            ),
            (
                serde_json::json!({"action": "grant_permissions", "permissions": ["camera"], "state": "ask"}),
                "ask",
            ),
            (
                serde_json::json!({"action": "grant_permissions", "origin": "https://x.test"}),
                "permissions",
            ),
            (
                serde_json::json!({"action": "start_har_recording", "mode": "minimal", "content": "omit", "updat": "merge"}),
                "updat",
            ),
            (
                serde_json::json!({"action": "storage_state", "indexedDb": true}),
                "indexedDb",
            ),
            (
                serde_json::json!({"action": "set_storage_state", "indexed_db": true}),
                "state",
            ),
        ] {
            assert_step_rejection(step, expected);
        }
    }

    #[test]
    fn block_service_workers_is_an_optional_batch_level_flag() {
        let omitted = parse_browser_action_request(serde_json::json!({"steps": []})).unwrap();
        assert_eq!(omitted.block_service_workers, None);
        assert_eq!(
            serde_json::to_value(&omitted)
                .unwrap()
                .get("block_service_workers"),
            None
        );

        for flag in [true, false] {
            let parsed = parse_browser_action_request(
                serde_json::json!({"block_service_workers": flag, "steps": []}),
            )
            .unwrap();
            assert_eq!(parsed.block_service_workers, Some(flag));
            assert_eq!(
                serde_json::to_value(&parsed).unwrap()["block_service_workers"],
                serde_json::json!(flag)
            );
        }

        for rejected in [
            serde_json::json!({"block_service_workers": "yes", "steps": []}),
            serde_json::json!({"blockServiceWorkers": true, "steps": []}),
        ] {
            assert!(
                parse_browser_action_request(rejected.clone()).is_err(),
                "expected rejection: {rejected}"
            );
        }
    }
}
