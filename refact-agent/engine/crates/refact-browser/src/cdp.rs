use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::WebSocket;
use headless_chrome::browser::transport::Transport;

pub const CDP_INLINE_RESULT_LIMIT_BYTES: usize = 8 * 1024;
pub const CDP_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

const MAX_CDP_ERROR_CHARS: usize = 400;
const RESET_BLIND_DOMAINS: [&str; 2] = ["Emulation", "Network"];
const MUTATING_PREFIXES: [&str; 5] = ["set", "clear", "emulate", "add", "remove"];
const COOKIE_METHODS: [&str; 5] = [
    "Network.getCookies",
    "Network.getAllCookies",
    "Network.setCookie",
    "Network.setCookies",
    "Network.deleteCookies",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CdpGuardrail {
    Denied { reason: String },
    Allowed { warnings: Vec<String> },
}

pub fn classify_cdp_command(
    method: &str,
    params: Option<&Value>,
    own_target_id: Option<&str>,
) -> CdpGuardrail {
    if method == "Browser.close" {
        return CdpGuardrail::Denied {
            reason: "Browser.close would shut down the shared browser and every other tab; \
                     close the session instead"
                .to_string(),
        };
    }
    if method == "Target.closeTarget" {
        let requested = params
            .and_then(|params| params.get("targetId"))
            .and_then(Value::as_str);
        if let (Some(requested), Some(own)) = (requested, own_target_id) {
            if requested == own {
                return CdpGuardrail::Denied {
                    reason: format!(
                        "Target.closeTarget would close the tab this session is driving ({own}); \
                         use close_tab so the session can pick a new active tab"
                    ),
                };
            }
        }
    }
    CdpGuardrail::Allowed {
        warnings: reset_blindness_warning(method).into_iter().collect(),
    }
}

fn reset_blindness_warning(method: &str) -> Option<String> {
    let (domain, command) = method.split_once('.')?;
    if !RESET_BLIND_DOMAINS.contains(&domain) {
        return None;
    }
    if !MUTATING_PREFIXES
        .iter()
        .any(|prefix| command.starts_with(prefix))
    {
        return None;
    }
    Some(format!(
        "{method} changes {domain} state directly; list_routes and reset do not track it, \
         so undo it with another cdp_send"
    ))
}

pub fn is_cookie_or_storage_method(method: &str) -> bool {
    let domain = method.split_once('.').map(|(domain, _)| domain);
    matches!(domain, Some("Storage") | Some("DOMStorage")) || COOKIE_METHODS.contains(&method)
}

pub fn redact_cdp_result(method: &str, value: Value) -> Value {
    let value = redact_strings(value);
    if is_cookie_or_storage_method(method) {
        redact_stored_values(value)
    } else {
        value
    }
}

fn redact_strings(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(refact_core::string_utils::redact_sensitive(&text)),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_strings).collect()),
        Value::Object(entries) => Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key, redact_strings(value)))
                .collect(),
        ),
        other => other,
    }
}

fn redact_stored_values(value: Value) -> Value {
    match value {
        Value::Object(entries) => Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| match (key.as_str(), &value) {
                    ("value", Value::String(_)) => (key, Value::String("[REDACTED]".to_string())),
                    ("entries", Value::Array(_)) => (key, redact_storage_entries(value)),
                    _ => (key, redact_stored_values(value)),
                })
                .collect(),
        ),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(redact_stored_values).collect())
        }
        other => other,
    }
}

fn redact_storage_entries(value: Value) -> Value {
    let Value::Array(entries) = value else {
        return value;
    };
    Value::Array(
        entries
            .into_iter()
            .map(|entry| match entry {
                Value::Array(mut pair) if pair.len() > 1 => {
                    pair[1] = Value::String("[REDACTED]".to_string());
                    Value::Array(pair)
                }
                other => redact_stored_values(other),
            })
            .collect(),
    )
}

pub fn bounded_cdp_text(message: &str) -> String {
    let collapsed = refact_core::string_utils::redact_sensitive(message)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() <= MAX_CDP_ERROR_CHARS {
        return collapsed;
    }
    let kept = collapsed
        .chars()
        .take(MAX_CDP_ERROR_CHARS)
        .collect::<String>();
    format!("{kept}...")
}

type CdpSocket = WebSocket<MaybeTlsStream<TcpStream>>;

pub const RAW_CDP_MAX_MESSAGE_BYTES: usize = 256 * 1024 * 1024;
pub const RAW_CDP_MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;

/// Opens a raw CDP websocket whose limits match what Chrome may legitimately send (response
/// bodies, screenshots, large console payloads) instead of tungstenite's 64/16 MiB defaults.
pub fn connect_raw_cdp(ws_url: &str) -> Result<CdpSocket, tungstenite::Error> {
    let config = WebSocketConfig::default()
        .max_message_size(Some(RAW_CDP_MAX_MESSAGE_BYTES))
        .max_frame_size(Some(RAW_CDP_MAX_FRAME_BYTES));
    tungstenite::client::connect_with_config(ws_url, Some(config), 3).map(|(socket, _)| socket)
}

pub struct CdpSession {
    transport: Transport,
    attached_sessions: Mutex<HashMap<String, String>>,
}

impl CdpSession {
    pub fn connect(ws_url: &str) -> Result<Self, String> {
        let url = url::Url::parse(ws_url).map_err(|error| format!("Invalid CDP URL: {error}"))?;
        let transport =
            Transport::new(url, None, Duration::from_secs(86_400), None).map_err(|error| {
                format!(
                    "Failed to open raw CDP session: {}",
                    bounded_cdp_text(&error.to_string())
                )
            })?;
        Ok(Self {
            transport,
            attached_sessions: Mutex::new(HashMap::new()),
        })
    }

    pub fn send(
        &self,
        method: &str,
        params: Option<&Value>,
        target_id: Option<&str>,
    ) -> Result<Value, String> {
        let session_id = match target_id {
            Some(target_id) => Some(self.attach(target_id)?),
            None => None,
        };
        self.call(method, params, session_id.as_deref())
    }

    fn attach(&self, target_id: &str) -> Result<String, String> {
        if let Some(session_id) = self
            .attached_sessions
            .lock()
            .map_err(|error| format!("Failed to lock CDP sessions: {error}"))?
            .get(target_id)
        {
            return Ok(session_id.clone());
        }
        let attached = self.call(
            "Target.attachToTarget",
            Some(&json!({"targetId": target_id, "flatten": true})),
            None,
        )?;
        let session_id = attached
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Target.attachToTarget returned no sessionId for {target_id}"))?
            .to_string();
        self.attached_sessions
            .lock()
            .map_err(|error| format!("Failed to lock CDP sessions: {error}"))?
            .insert(target_id.to_string(), session_id.clone());
        Ok(session_id)
    }

    fn call(
        &self,
        method: &str,
        params: Option<&Value>,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        self.transport
            .call_method_json(method, params.cloned().unwrap_or(Value::Null), session_id)
            .map_err(|error| bounded_cdp_text(&error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_session(
        script: impl FnOnce(WebSocket<TcpStream>) + Send + 'static,
    ) -> (CdpSession, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "ws://{}/devtools/browser/test",
            listener.local_addr().unwrap()
        );
        let peer = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            script(tungstenite::accept(stream).unwrap());
        });
        (CdpSession::connect(&url).unwrap(), peer)
    }

    fn read_frame(ws: &mut WebSocket<TcpStream>) -> Value {
        serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap()
    }

    #[test]
    fn dynamic_session_sends_flattened_unknown_method_and_maps_remote_error() {
        let (session, peer) = mock_session(|mut ws| {
            let attach = read_frame(&mut ws);
            assert_eq!(attach["method"], "Target.attachToTarget");
            assert_eq!(attach["params"], json!({"targetId":"tab", "flatten":true}));
            assert!(attach.get("sessionId").is_none());
            ws.send(tungstenite::Message::text(
                json!({"id":attach["id"],
                "result":{"sessionId":"flat-session"}})
                .to_string(),
            ))
            .unwrap();
            let raw = read_frame(&mut ws);
            assert_eq!(raw["method"], "FutureDomain.notInGeneratedProtocol");
            assert_eq!(raw["params"], json!({"nested":{"numbers":[3,5]}}));
            assert_eq!(raw["sessionId"], "flat-session");
            assert!(raw["id"] != attach["id"]);
            ws.send(tungstenite::Message::text(
                json!({"id":raw["id"], "sessionId":"flat-session",
                "result":{"nested":[false,{"value":17}]}})
                .to_string(),
            ))
            .unwrap();
            let failed = read_frame(&mut ws);
            // A second targeted command must reuse the attachment, not issue attach again.
            assert_eq!(failed["method"], "FutureDomain.fail");
            assert_eq!(failed["sessionId"], "flat-session");
            ws.send(tungstenite::Message::text(
                json!({"id":failed["id"],
                "error":{"code":-32000,"message":"mock refusal"}})
                .to_string(),
            ))
            .unwrap();
            let _ = ws.read();
        });
        assert_eq!(
            session
                .send(
                    "FutureDomain.notInGeneratedProtocol",
                    Some(&json!({"nested":{"numbers":[3,5]}})),
                    Some("tab")
                )
                .unwrap(),
            json!({"nested":[false,{"value":17}]})
        );
        assert_eq!(
            session
                .send("FutureDomain.fail", None, Some("tab"))
                .unwrap_err(),
            "Method call error -32000: mock refusal"
        );
        drop(session);
        peer.join().unwrap();
    }

    #[test]
    fn dynamic_session_deadline_is_not_replayed_and_late_reply_is_discarded() {
        use headless_chrome::browser::transport::CommandDeadlineGuard;
        use std::sync::mpsc;
        use std::time::Instant;
        let (release, wait) = mpsc::channel();
        let (seen, received) = mpsc::channel();
        let (session, peer) = mock_session(move |mut ws| {
            let first = read_frame(&mut ws);
            assert_eq!(first["method"], "FutureDomain.pending");
            seen.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            ws.send(tungstenite::Message::text(
                json!({"id":first["id"],
                "result":{"late":true}})
                .to_string(),
            ))
            .unwrap();
            let next = read_frame(&mut ws);
            assert_eq!(
                next["method"], "FutureDomain.next",
                "uncertain command was replayed"
            );
            assert_ne!(next["id"], first["id"]);
            // Missing result is a valid null result, never a panic.
            ws.send(tungstenite::Message::text(
                json!({"id":next["id"]}).to_string(),
            ))
            .unwrap();
            let _ = ws.read();
        });
        let guard = CommandDeadlineGuard::new(Instant::now() + Duration::from_millis(100));
        let error = session
            .send("FutureDomain.pending", None, None)
            .unwrap_err();
        drop(guard);
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            error,
            "CDP command deadline exceeded: raw CDP (effect_unknown=true)"
        );
        release.send(()).unwrap();
        assert_eq!(
            session.send("FutureDomain.next", None, None).unwrap(),
            Value::Null
        );
        drop(session);
        peer.join().unwrap();
    }

    fn denial(method: &str, params: Option<&Value>, own: Option<&str>) -> String {
        match classify_cdp_command(method, params, own) {
            CdpGuardrail::Denied {
                reason,
            } => reason,
            CdpGuardrail::Allowed {
                ..
            } => panic!("expected {method} to be denied"),
        }
    }

    #[test]
    fn browser_close_is_denied_whatever_the_params_are() {
        assert!(denial("Browser.close", None, Some("TAB-1")).contains("shut down"));
        assert!(
            denial("Browser.close", Some(&json!({"targetId": "OTHER"})), None)
                .contains("Browser.close")
        );
    }

    #[test]
    fn close_target_is_denied_only_for_the_session_owned_tab() {
        let own = json!({"targetId": "TAB-1"});
        assert!(
            denial("Target.closeTarget", Some(&own), Some("TAB-1")).contains("TAB-1"),
            "closing our own tab must be denied"
        );

        for allowed in [
            classify_cdp_command(
                "Target.closeTarget",
                Some(&json!({"targetId": "TAB-2"})),
                Some("TAB-1"),
            ),
            classify_cdp_command("Target.closeTarget", Some(&own), None),
            classify_cdp_command("Target.closeTarget", None, Some("TAB-1")),
        ] {
            assert_eq!(
                allowed,
                CdpGuardrail::Allowed {
                    warnings: vec![]
                }
            );
        }
    }

    #[test]
    fn everything_outside_the_denylist_is_allowed() {
        for method in [
            "Runtime.evaluate",
            "Page.navigate",
            "Target.createTarget",
            "Browser.getVersion",
            "Network.setCookie",
        ] {
            assert!(matches!(
                classify_cdp_command(method, None, Some("TAB-1")),
                CdpGuardrail::Allowed { .. }
            ));
        }
    }

    #[test]
    fn cookie_and_storage_results_have_their_values_redacted() {
        let cookies = redact_cdp_result(
            "Network.getCookies",
            json!({"cookies": [{"name": "session", "value": "super-secret", "domain": "x.dev"}]}),
        );
        assert_eq!(cookies["cookies"][0]["value"], "[REDACTED]");
        assert_eq!(cookies["cookies"][0]["name"], "session");
        assert_eq!(cookies["cookies"][0]["domain"], "x.dev");

        let storage = redact_cdp_result(
            "DOMStorage.getDOMStorageItems",
            json!({"entries": [["token", "abcdef"], ["theme", "dark"]]}),
        );
        assert_eq!(storage["entries"][0][0], "token");
        assert_eq!(storage["entries"][0][1], "[REDACTED]");
        assert_eq!(storage["entries"][1][1], "[REDACTED]");
    }

    #[test]
    fn non_storage_results_keep_their_values_but_still_lose_secrets() {
        let result = redact_cdp_result(
            "Runtime.evaluate",
            json!({"result": {"type": "string", "value": "plain result"}}),
        );
        assert_eq!(result["result"]["value"], "plain result");

        let leaked = redact_cdp_result(
            "Runtime.evaluate",
            json!({"result": {"value": "Authorization: Bearer abcdef123456"}}),
        );
        assert!(
            leaked["result"]["value"]
                .as_str()
                .is_some_and(|value| value.contains("[REDACTED]")),
            "secrets must be redacted everywhere, got {leaked}"
        );
    }

    #[test]
    fn cookie_and_storage_detection_covers_the_documented_methods() {
        for method in [
            "Storage.getCookies",
            "DOMStorage.getDOMStorageItems",
            "Network.setCookie",
            "Network.getAllCookies",
        ] {
            assert!(is_cookie_or_storage_method(method), "{method}");
        }
        for method in ["Runtime.evaluate", "Network.getResponseBody", "Page.reload"] {
            assert!(!is_cookie_or_storage_method(method), "{method}");
        }
    }

    #[test]
    fn cdp_errors_are_single_line_and_bounded() {
        let bounded = bounded_cdp_text(&format!("line\n{}", "x".repeat(4_000)));
        assert!(!bounded.contains('\n'));
        assert!(bounded.chars().count() <= MAX_CDP_ERROR_CHARS + 3);
        assert!(bounded.ends_with("..."));

        assert_eq!(
            bounded_cdp_text("{\"code\": -32000,\n \"message\": \"nope\"}"),
            "{\"code\": -32000, \"message\": \"nope\"}"
        );
    }
}
