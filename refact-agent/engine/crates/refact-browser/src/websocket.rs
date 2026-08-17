use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use headless_chrome::Tab;
use headless_chrome::protocol::cdp::Page;
use serde_json::{Value, json};

use refact_integrations::browser_models::{
    UrlPattern, WebSocketEvent, WebSocketEventKind, WebSocketRouteMode,
};

use crate::network::{UrlMatcher, mask_text};

const WEBSOCKET_EVENT_CAP: usize = 1_000;
const WEBSOCKET_BINDING: &str = "__refact_websocket_event";
const WEBSOCKET_DISPATCH: &str = "__refactWebSocketDispatch";

#[derive(Clone, Debug)]
struct RegisteredWebSocketRoute {
    pattern: UrlPattern,
    matcher: UrlMatcher,
    mode: WebSocketRouteMode,
}

#[derive(Clone, Debug)]
struct RoutedSocket {
    tab_target_id: String,
    route_id: String,
    url: String,
}

#[derive(Debug, Default)]
struct WebSocketState {
    sequence: u64,
    urls: HashMap<String, String>,
    events: VecDeque<WebSocketEvent>,
    report_cursor: u64,
    routes: Vec<RegisteredWebSocketRoute>,
    routed_sockets: HashMap<String, RoutedSocket>,
    commands: Vec<(String, Value)>,
}

#[derive(Debug, Default)]
pub struct WebSocketRegistry {
    state: Mutex<WebSocketState>,
    changed: Condvar,
}

impl WebSocketRegistry {
    pub fn add_route(&self, pattern: UrlPattern, mode: WebSocketRouteMode) -> Result<(), String> {
        let matcher = matcher_for_pattern(&pattern)?;
        self.state
            .lock()
            .unwrap()
            .routes
            .push(RegisteredWebSocketRoute {
                pattern,
                matcher,
                mode,
            });
        Ok(())
    }

    pub fn remove_routes(&self, pattern: Option<&UrlPattern>) -> usize {
        let mut state = self.state.lock().unwrap();
        let previous = state.routes.len();
        match pattern {
            Some(pattern) => state.routes.retain(|route| &route.pattern != pattern),
            None => state.routes.clear(),
        }
        previous - state.routes.len()
    }

    pub fn route_count(&self) -> usize {
        self.state.lock().unwrap().routes.len()
    }

    pub fn cursor(&self) -> u64 {
        self.state.lock().unwrap().sequence
    }

    pub fn record_created(&self, socket_id: String, url: String) {
        let mut state = self.state.lock().unwrap();
        let has_page_route = state
            .routed_sockets
            .values()
            .any(|socket| socket.url == url);
        if has_page_route {
            return;
        }
        state.urls.insert(socket_id.clone(), url.clone());
        push_event(
            &mut state,
            socket_id,
            url,
            WebSocketEventKind::Created,
            None,
            None,
            None,
            None,
            false,
        );
        self.changed.notify_all();
    }

    pub fn record_handshake(&self, socket_id: &str, status: u16) {
        self.record(
            socket_id,
            WebSocketEventKind::HandshakeResponse,
            None,
            None,
            Some(status),
            None,
            false,
        );
    }

    pub fn record_frame(&self, socket_id: &str, sent: bool, data: String, opcode: u8) {
        let data = if opcode == 1 {
            mask_text(&data)
        } else {
            format!("[binary frame: {} bytes]", decoded_len(&data))
        };
        self.record(
            socket_id,
            if sent {
                WebSocketEventKind::FrameSent
            } else {
                WebSocketEventKind::FrameReceived
            },
            Some(data),
            Some(opcode),
            None,
            None,
            false,
        );
    }

    pub fn record_closed(&self, socket_id: &str) {
        self.record(
            socket_id,
            WebSocketEventKind::Closed,
            None,
            None,
            None,
            None,
            false,
        );
        self.state.lock().unwrap().urls.remove(socket_id);
    }

    pub fn record_error(&self, socket_id: &str, error: String) {
        self.record(
            socket_id,
            WebSocketEventKind::Error,
            None,
            None,
            None,
            Some(mask_text(&error)),
            false,
        );
    }

    pub fn handle_page_event(&self, tab_target_id: &str, payload: &Value) {
        let Some(event_type) = payload.get("type").and_then(Value::as_str) else {
            return;
        };
        let Some(route_id) = payload.get("id").and_then(Value::as_str) else {
            return;
        };
        match event_type {
            "created" => {
                let Some(url) = payload.get("url").and_then(Value::as_str) else {
                    return;
                };
                let mut state = self.state.lock().unwrap();
                let Some(route) = state
                    .routes
                    .iter()
                    .find(|route| route.matcher.is_match(url))
                    .cloned()
                else {
                    state.routed_sockets.insert(
                        route_id.to_string(),
                        RoutedSocket {
                            tab_target_id: tab_target_id.to_string(),
                            route_id: route_id.to_string(),
                            url: url.to_string(),
                        },
                    );
                    drop(state);
                    let _ = self.queue_command(route_id, json!({"type": "connect"}));
                    return;
                };
                let routed = !matches!(route.mode, WebSocketRouteMode::ObserveAndModify);
                state.routed_sockets.insert(
                    route_id.to_string(),
                    RoutedSocket {
                        tab_target_id: tab_target_id.to_string(),
                        route_id: route_id.to_string(),
                        url: url.to_string(),
                    },
                );
                push_event(
                    &mut state,
                    route_id.to_string(),
                    url.to_string(),
                    WebSocketEventKind::Created,
                    None,
                    None,
                    None,
                    None,
                    routed,
                );
                let mode = route.mode;
                drop(state);
                let _ = self.queue_command(
                    route_id,
                    match mode {
                        WebSocketRouteMode::Mock => json!({"type": "open"}),
                        WebSocketRouteMode::ObserveAndModify => json!({"type": "connect"}),
                    },
                );
            }
            "page_message" | "server_message" => {
                let Some(data) = payload.get("data").and_then(Value::as_str) else {
                    return;
                };
                let binary = payload
                    .get("is_base64")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let routed = self
                    .state
                    .lock()
                    .unwrap()
                    .routed_sockets
                    .get(route_id)
                    .cloned();
                let Some(socket) = routed else {
                    return;
                };
                self.record(
                    route_id,
                    if event_type == "page_message" {
                        WebSocketEventKind::FrameSent
                    } else {
                        WebSocketEventKind::FrameReceived
                    },
                    Some(if binary {
                        format!("[binary frame: {} bytes]", decoded_len(data))
                    } else {
                        mask_text(data)
                    }),
                    Some(if binary { 2 } else { 1 }),
                    None,
                    None,
                    true,
                );
                let _ = self.queue_command(
                    &socket.route_id,
                    json!({
                        "type": if event_type == "page_message" { "send_to_server" } else { "send_to_page" },
                        "data": data,
                        "is_base64": binary,
                    }),
                );
            }
            "closed" => {
                self.record(
                    route_id,
                    WebSocketEventKind::Closed,
                    None,
                    None,
                    None,
                    None,
                    true,
                );
                self.state.lock().unwrap().routed_sockets.remove(route_id);
            }
            "error" => self.record(
                route_id,
                WebSocketEventKind::Error,
                None,
                None,
                None,
                payload
                    .get("message")
                    .and_then(Value::as_str)
                    .map(mask_text),
                true,
            ),
            _ => {}
        }
        self.changed.notify_all();
    }

    pub fn drain_report(&self) -> Vec<WebSocketEvent> {
        let mut state = self.state.lock().unwrap();
        let cursor = state.report_cursor;
        let events = state
            .events
            .iter()
            .filter(|event| event.sequence > cursor)
            .cloned()
            .collect::<Vec<_>>();
        state.report_cursor = state.sequence;
        events
    }

    pub fn wait_for_frame(
        &self,
        matcher: Option<&UrlMatcher>,
        after: u64,
        timeout: Duration,
    ) -> Result<WebSocketEvent, String> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(event) = state.events.iter().find(|event| {
                event.sequence > after
                    && matches!(
                        event.kind,
                        WebSocketEventKind::FrameSent | WebSocketEventKind::FrameReceived
                    )
                    && matcher.is_none_or(|matcher| matcher.is_match(&event.url))
            }) {
                return Ok(event.clone());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("Timed out after {}ms", timeout.as_millis()));
            }
            let (next, wait) = self.changed.wait_timeout(state, remaining).unwrap();
            state = next;
            if wait.timed_out() {
                return Err(format!("Timed out after {}ms", timeout.as_millis()));
            }
        }
    }

    pub fn send_to_page(&self, pattern: &UrlPattern, data: &str) -> Result<usize, String> {
        let matcher = matcher_for_pattern(pattern)?;
        let sockets = self
            .state
            .lock()
            .unwrap()
            .routed_sockets
            .values()
            .filter(|socket| matcher.is_match(&socket.url))
            .cloned()
            .collect::<Vec<_>>();
        for socket in &sockets {
            self.queue_command(
                &socket.route_id,
                json!({"type": "send_to_page", "data": data, "is_base64": false}),
            )?;
            self.record(
                &socket.route_id,
                WebSocketEventKind::FrameReceived,
                Some(mask_text(data)),
                Some(1),
                None,
                None,
                true,
            );
        }
        Ok(sockets.len())
    }

    pub fn flush_commands(&self, tabs: &[std::sync::Arc<Tab>]) -> Result<(), String> {
        let commands = std::mem::take(&mut self.state.lock().unwrap().commands);
        for (route_id, command) in commands {
            let socket = self
                .state
                .lock()
                .unwrap()
                .routed_sockets
                .get(&route_id)
                .cloned();
            let Some(socket) = socket else {
                continue;
            };
            let Some(tab) = tabs
                .iter()
                .find(|tab| tab.get_target_id() == &socket.tab_target_id)
            else {
                continue;
            };
            let command = serde_json::to_string(&command)
                .map_err(|error| format!("Failed to serialize WebSocket command: {error}"))?;
            tab.evaluate(
                &format!("globalThis[{WEBSOCKET_DISPATCH:?}]?.({command})"),
                false,
            )
            .map_err(|error| format!("Failed to dispatch WebSocket command: {error}"))?;
        }
        Ok(())
    }

    fn record(
        &self,
        socket_id: &str,
        kind: WebSocketEventKind,
        data: Option<String>,
        opcode: Option<u8>,
        status: Option<u16>,
        error: Option<String>,
        routed: bool,
    ) {
        let mut state = self.state.lock().unwrap();
        let url = state
            .routed_sockets
            .get(socket_id)
            .map(|socket| socket.url.clone())
            .or_else(|| state.urls.get(socket_id).cloned())
            .unwrap_or_default();
        push_event(
            &mut state,
            socket_id.to_string(),
            url,
            kind,
            data,
            opcode,
            status,
            error,
            routed,
        );
        self.changed.notify_all();
    }

    fn queue_command(&self, route_id: &str, mut command: Value) -> Result<(), String> {
        command["id"] = Value::String(route_id.to_string());
        self.state
            .lock()
            .unwrap()
            .commands
            .push((route_id.to_string(), command));
        Ok(())
    }
}

fn push_event(
    state: &mut WebSocketState,
    socket_id: String,
    url: String,
    kind: WebSocketEventKind,
    data: Option<String>,
    opcode: Option<u8>,
    status: Option<u16>,
    error: Option<String>,
    routed: bool,
) {
    state.sequence += 1;
    state.events.push_back(WebSocketEvent {
        sequence: state.sequence,
        socket_id,
        url: mask_text(&url),
        kind,
        data,
        opcode,
        status,
        error,
        routed,
    });
    while state.events.len() > WEBSOCKET_EVENT_CAP {
        state.events.pop_front();
    }
}

fn decoded_len(data: &str) -> usize {
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map(|data| data.len())
        .unwrap_or(data.len())
}

fn matcher_for_pattern(pattern: &UrlPattern) -> Result<UrlMatcher, String> {
    match pattern {
        UrlPattern::Text(value) => UrlMatcher::text(value),
        UrlPattern::Regex { source, flags } => UrlMatcher::regex(source, flags),
    }
}

fn websocket_event_from_binding_payload(payload: &Value) -> Option<Value> {
    let value = match payload {
        Value::String(text) => serde_json::from_str::<Value>(text).ok()?,
        other => other.clone(),
    };
    if let Some(args) = value.get("args").and_then(Value::as_array) {
        return match args.first() {
            Some(Value::String(text)) => serde_json::from_str::<Value>(text)
                .ok()
                .filter(Value::is_object),
            Some(event) if event.is_object() => Some(event.clone()),
            _ => None,
        };
    }
    value.get("type").and_then(Value::as_str)?;
    Some(value)
}

pub fn install_websocket_router(
    tab: &Tab,
    registry: std::sync::Arc<WebSocketRegistry>,
) -> Result<(), String> {
    let target_id = tab.get_target_id().to_string();
    tab.expose_function(
        WEBSOCKET_BINDING,
        std::sync::Arc::new(move |payload: Value| {
            let Some(event) = websocket_event_from_binding_payload(&payload) else {
                return;
            };
            registry.handle_page_event(&target_id, &event);
        }),
    )
    .map_err(|error| format!("Failed to expose WebSocket route binding: {error}"))?;
    let script = websocket_mock_script();
    tab.call_method(Page::AddScriptToEvaluateOnNewDocument {
        source: script.clone(),
        world_name: None,
        include_command_line_api: None,
        run_immediately: Some(true),
    })
    .map_err(|error| format!("Failed to install WebSocket route script: {error}"))?;
    tab.evaluate(&script, false)
        .map(|_| ())
        .map_err(|error| format!("Failed to activate WebSocket route script: {error}"))
}

fn websocket_mock_script() -> String {
    format!(
        r#"(() => {{
  if (globalThis.{dispatch}) return;
  const NativeWebSocket = globalThis.WebSocket;
  const sockets = new Map();
  let nextId = 0;
  const emit = event => globalThis.{binding}(JSON.stringify(event));
  const toWire = data => {{
    if (typeof data === 'string') return Promise.resolve({{ data, is_base64: false }});
    const blob = data instanceof Blob ? data : new Blob([data]);
    return blob.arrayBuffer().then(buffer => {{
      const bytes = new Uint8Array(buffer);
      let binary = '';
      for (const byte of bytes) binary += String.fromCharCode(byte);
      return {{ data: btoa(binary), is_base64: true }};
    }});
  }};
  const fromWire = (data, binaryType) => {{
    if (!data.is_base64) return data.data;
    const binary = atob(data.data);
    const bytes = Uint8Array.from(binary, ch => ch.charCodeAt(0));
    return binaryType === 'arraybuffer' ? bytes.buffer : new Blob([bytes]);
  }};
  class RoutedWebSocket extends EventTarget {{
    static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
    CONNECTING = 0; OPEN = 1; CLOSING = 2; CLOSED = 3;
    constructor(url, protocols) {{
      super();
      this.url = new URL(url, document.baseURI).href.replace(/^http/, 'ws');
      this.protocols = protocols;
      this.readyState = 0;
      this.binaryType = 'blob';
      this.bufferedAmount = 0;
      this.extensions = '';
      this.protocol = '';
      this.onopen = null; this.onmessage = null; this.onerror = null; this.onclose = null;
      this.id = `refact-ws-${{++nextId}}`;
      sockets.set(this.id, this);
      emit({{ type: 'created', id: this.id, url: this.url }});
    }}
    send(data) {{
      if (this.readyState !== 1) throw new DOMException('WebSocket is not open');
      toWire(data).then(wire => emit({{ type: 'page_message', id: this.id, ...wire }}));
    }}
    close(code = 1000, reason = '') {{
      this.readyState = 3;
      this.server?.close(code, reason);
      this.dispatchEvent(new CloseEvent('close', {{ code, reason, wasClean: true }}));
      emit({{ type: 'closed', id: this.id }});
      sockets.delete(this.id);
    }}
    open() {{
      if (this.readyState !== 0) return;
      this.readyState = 1;
      const event = new Event('open'); this.dispatchEvent(event); this.onopen?.(event);
    }}
    connect() {{
      this.server = new NativeWebSocket(this.url, this.protocols);
      this.server.binaryType = this.binaryType;
      this.server.onopen = () => this.open();
      this.server.onmessage = event => toWire(event.data).then(wire => emit({{ type: 'server_message', id: this.id, ...wire }}));
      this.server.onerror = () => emit({{ type: 'error', id: this.id, message: 'WebSocket server error' }});
      this.server.onclose = event => {{
        this.readyState = 3;
        const close = new CloseEvent('close', {{ code: event.code, reason: event.reason, wasClean: event.wasClean }});
        this.dispatchEvent(close); this.onclose?.(close);
        emit({{ type: 'closed', id: this.id }});
        sockets.delete(this.id);
      }};
    }}
    sendToPage(data) {{
      this.open();
      const event = new MessageEvent('message', {{ data: fromWire(data, this.binaryType), origin: new URL(this.url).origin }});
      this.dispatchEvent(event); this.onmessage?.(event);
    }}
    sendToServer(data) {{ this.server?.send(fromWire(data, this.binaryType)); }}
  }}
  globalThis.{dispatch} = request => {{
    const socket = sockets.get(request.id);
    if (!socket) return;
    if (request.type === 'open') socket.open();
    if (request.type === 'connect') socket.connect();
    if (request.type === 'send_to_page') socket.sendToPage(request);
    if (request.type === 'send_to_server') socket.sendToServer(request);
  }};
  globalThis.WebSocket = RoutedWebSocket;
}})();"#,
        dispatch = WEBSOCKET_DISPATCH,
        binding = WEBSOCKET_BINDING,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_mapping_masks_frames_and_bounds_buffer() {
        let registry = WebSocketRegistry::default();
        registry.record_created(
            "ws-1".to_string(),
            "wss://example.test/socket?token=secret".to_string(),
        );
        registry.record_frame("ws-1", false, "password=hunter2".to_string(), 1);
        for index in 0..WEBSOCKET_EVENT_CAP + 10 {
            registry.record_frame("ws-1", true, format!("message-{index}"), 1);
        }
        let events = registry.drain_report();
        assert_eq!(events.len(), WEBSOCKET_EVENT_CAP);
        assert!(events.iter().all(|event| !event.url.contains("secret")));
        assert!(events.iter().all(|event| {
            event
                .data
                .as_deref()
                .is_none_or(|data| !data.contains("hunter2"))
        }));
    }

    #[test]
    fn shim_constructs_from_one_string_and_emits_a_single_json_string() {
        let script = websocket_mock_script();
        assert!(script.contains("constructor(url, protocols)"));
        assert!(script.contains(&format!(
            "globalThis.{WEBSOCKET_BINDING}(JSON.stringify(event))"
        )));
        assert!(!script.contains(&format!("globalThis.{WEBSOCKET_BINDING}(event)")));
    }

    #[test]
    fn binding_payload_shapes_unwrap_to_page_events() {
        let direct = Value::String(
            json!({"type": "created", "id": "refact-ws-1", "url": "ws://host/ws-echo"}).to_string(),
        );
        assert_eq!(
            websocket_event_from_binding_payload(&direct).unwrap()["type"],
            json!("created")
        );

        let wrapped = Value::String(
            json!({
                "name": WEBSOCKET_BINDING,
                "seq": 1,
                "args": [json!({"type": "closed", "id": "refact-ws-1"}).to_string()],
            })
            .to_string(),
        );
        assert_eq!(
            websocket_event_from_binding_payload(&wrapped).unwrap()["type"],
            json!("closed")
        );

        assert!(websocket_event_from_binding_payload(&Value::String("not json".to_string())).is_none());
        assert!(
            websocket_event_from_binding_payload(&Value::String(json!({"seq": 1}).to_string()))
                .is_none()
        );
    }

    #[test]
    fn created_binding_payload_registers_socket_reachable_by_send() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = WebSocketRegistry::default();
        registry
            .add_route(pattern.clone(), WebSocketRouteMode::Mock)
            .unwrap();
        let payload = Value::String(
            json!({"type": "created", "id": "refact-ws-1", "url": "ws://127.0.0.1:8123/ws-echo"})
                .to_string(),
        );
        let event = websocket_event_from_binding_payload(&payload).unwrap();
        registry.handle_page_event("tab-1", &event);
        assert_eq!(registry.send_to_page(&pattern, "mocked-frame").unwrap(), 1);
        let events = registry.drain_report();
        assert!(events.iter().any(|event| {
            matches!(event.kind, WebSocketEventKind::Created) && event.routed
        }));
        assert!(events.iter().any(|event| {
            matches!(event.kind, WebSocketEventKind::FrameReceived)
                && event.data.as_deref() == Some("mocked-frame")
        }));
    }

    #[test]
    fn routed_websocket_events_forward_by_default_and_agent_messages_are_masked() {
        let registry = WebSocketRegistry::default();
        registry
            .add_route(
                UrlPattern::Text("wss://example.test/**".to_string()),
                WebSocketRouteMode::ObserveAndModify,
            )
            .unwrap();
        registry.handle_page_event(
            "tab-1",
            &json!({"type": "created", "id": "route-1", "url": "wss://example.test/ws"}),
        );
        registry.handle_page_event(
            "tab-1",
            &json!({"type": "page_message", "id": "route-1", "data": "token=secret", "is_base64": false}),
        );
        let events = registry.drain_report();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].data.as_deref(), Some("token=[REDACTED]"));
    }
}
