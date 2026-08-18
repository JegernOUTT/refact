use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use headless_chrome::Tab;
use headless_chrome::protocol::cdp::Page;
use serde_json::{Value, json};

use refact_integrations::browser_models::{
    UrlPattern, WebSocketEvent, WebSocketEventKind, WebSocketFrameDisposition,
    WebSocketMessageAction, WebSocketRouteMode,
};

use crate::network::{UrlMatcher, mask_text};

const WEBSOCKET_EVENT_CAP: usize = 1_000;
const WEBSOCKET_BINDING: &str = "__refact_websocket_event";
const WEBSOCKET_DISPATCH: &str = "__refactWebSocketDispatch";
const FLUSH_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug)]
struct RegisteredWebSocketRoute {
    pattern: UrlPattern,
    matcher: UrlMatcher,
    mode: WebSocketRouteMode,
    on_page_message: WebSocketMessageAction,
    on_server_message: WebSocketMessageAction,
}

#[derive(Clone, Debug)]
struct RoutedSocket {
    tab_target_id: String,
    route_id: String,
    url: String,
    protocols: Vec<String>,
    routed: bool,
    on_page_message: WebSocketMessageAction,
    on_server_message: WebSocketMessageAction,
}

struct EventDraft {
    kind: WebSocketEventKind,
    data: Option<String>,
    opcode: Option<u8>,
    status: Option<u16>,
    error: Option<String>,
    routed: bool,
    disposition: Option<WebSocketFrameDisposition>,
    close_code: Option<u16>,
    close_reason: Option<String>,
}

impl EventDraft {
    fn new(kind: WebSocketEventKind) -> Self {
        Self {
            kind,
            data: None,
            opcode: None,
            status: None,
            error: None,
            routed: false,
            disposition: None,
            close_code: None,
            close_reason: None,
        }
    }

    fn routed(mut self) -> Self {
        self.routed = true;
        self
    }

    fn frame(mut self, data: String, opcode: u8) -> Self {
        self.data = Some(data);
        self.opcode = Some(opcode);
        self
    }

    fn disposition(mut self, disposition: WebSocketFrameDisposition) -> Self {
        self.disposition = Some(disposition);
        self
    }

    fn close(mut self, code: Option<u16>, reason: Option<String>) -> Self {
        self.close_code = code;
        self.close_reason = reason;
        self
    }
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
    pub fn add_route(
        &self,
        pattern: UrlPattern,
        mode: WebSocketRouteMode,
        on_page_message: WebSocketMessageAction,
        on_server_message: WebSocketMessageAction,
    ) -> Result<(), String> {
        let matcher = matcher_for_pattern(&pattern)?;
        self.state
            .lock()
            .unwrap()
            .routes
            .push(RegisteredWebSocketRoute {
                pattern,
                matcher,
                mode,
                on_page_message,
                on_server_message,
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

    pub fn top_level_navigation(&self, tab_target_id: &str) -> usize {
        let mut state = self.state.lock().unwrap();
        let previous = state.routed_sockets.len();
        state
            .routed_sockets
            .retain(|_, socket| socket.tab_target_id != tab_target_id);
        previous - state.routed_sockets.len()
    }

    #[cfg(test)]
    fn routed_socket_count(&self) -> usize {
        self.state.lock().unwrap().routed_sockets.len()
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
            Vec::new(),
            EventDraft::new(WebSocketEventKind::Created),
        );
        self.changed.notify_all();
    }

    pub fn record_handshake(&self, socket_id: &str, status: u16) {
        let mut draft = EventDraft::new(WebSocketEventKind::HandshakeResponse);
        draft.status = Some(status);
        self.record(socket_id, draft);
    }

    pub fn record_frame(&self, socket_id: &str, sent: bool, data: String, opcode: u8) {
        let kind = if sent {
            WebSocketEventKind::FrameSent
        } else {
            WebSocketEventKind::FrameReceived
        };
        self.record(
            socket_id,
            EventDraft::new(kind).frame(frame_text(&data, opcode), opcode),
        );
    }

    pub fn record_closed(&self, socket_id: &str) {
        self.record(socket_id, EventDraft::new(WebSocketEventKind::Closed));
        self.state.lock().unwrap().urls.remove(socket_id);
    }

    pub fn record_error(&self, socket_id: &str, error: String) {
        let mut draft = EventDraft::new(WebSocketEventKind::Error);
        draft.error = Some(mask_text(&error));
        self.record(socket_id, draft);
    }

    pub fn handle_page_event(&self, tab_target_id: &str, payload: &Value) {
        let Some(event_type) = payload.get("type").and_then(Value::as_str) else {
            return;
        };
        let Some(route_id) = payload.get("id").and_then(Value::as_str) else {
            return;
        };
        match event_type {
            "created" => self.handle_created(tab_target_id, route_id, payload),
            "page_message" | "server_message" => {
                self.handle_message(route_id, event_type == "page_message", payload)
            }
            "closed" => {
                self.record(
                    route_id,
                    EventDraft::new(WebSocketEventKind::Closed).routed().close(
                        payload
                            .get("code")
                            .and_then(Value::as_u64)
                            .map(|code| code as u16),
                        payload
                            .get("reason")
                            .and_then(Value::as_str)
                            .filter(|reason| !reason.is_empty())
                            .map(mask_text),
                    ),
                );
                self.state.lock().unwrap().routed_sockets.remove(route_id);
            }
            "error" => {
                let mut draft = EventDraft::new(WebSocketEventKind::Error).routed();
                draft.error = payload
                    .get("message")
                    .and_then(Value::as_str)
                    .map(mask_text);
                self.record(route_id, draft);
            }
            _ => {}
        }
        self.changed.notify_all();
    }

    fn handle_created(&self, tab_target_id: &str, route_id: &str, payload: &Value) {
        let Some(url) = payload.get("url").and_then(Value::as_str) else {
            return;
        };
        let protocols = payload
            .get("protocols")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(mask_text)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut state = self.state.lock().unwrap();
        let route = state
            .routes
            .iter()
            .find(|route| route.matcher.is_match(url))
            .cloned();
        let socket = RoutedSocket {
            tab_target_id: tab_target_id.to_string(),
            route_id: route_id.to_string(),
            url: url.to_string(),
            protocols: protocols.clone(),
            routed: route.is_some(),
            on_page_message: route
                .as_ref()
                .map(|route| route.on_page_message)
                .unwrap_or_default(),
            on_server_message: route
                .as_ref()
                .map(|route| route.on_server_message)
                .unwrap_or_default(),
        };
        state
            .routed_sockets
            .insert(route_id.to_string(), socket.clone());
        let command = match route.as_ref().map(|route| route.mode) {
            Some(WebSocketRouteMode::Mock) => json!({"type": "open"}),
            Some(WebSocketRouteMode::Intercept) | None => json!({"type": "connect"}),
        };
        if route.is_some() {
            push_event(
                &mut state,
                route_id.to_string(),
                url.to_string(),
                protocols,
                EventDraft::new(WebSocketEventKind::Created).routed(),
            );
        }
        drop(state);
        self.queue_command(&socket, command);
    }

    fn handle_message(&self, route_id: &str, from_page: bool, payload: &Value) {
        let Some(data) = payload.get("data").and_then(Value::as_str) else {
            return;
        };
        let binary = payload
            .get("is_base64")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let socket = self
            .state
            .lock()
            .unwrap()
            .routed_sockets
            .get(route_id)
            .cloned();
        let Some(socket) = socket else {
            return;
        };
        let action = if from_page {
            socket.on_page_message
        } else {
            socket.on_server_message
        };
        let kind = if from_page {
            WebSocketEventKind::FrameSent
        } else {
            WebSocketEventKind::FrameReceived
        };
        let opcode = if binary { 2 } else { 1 };
        let mut draft = EventDraft::new(kind)
            .frame(frame_text(data, opcode), opcode)
            .disposition(disposition_for(action));
        draft.routed = socket.routed;
        self.record(route_id, draft);
        if matches!(action, WebSocketMessageAction::Forward) {
            self.queue_command(
                &socket,
                json!({
                    "type": if from_page { "send_to_server" } else { "send_to_page" },
                    "data": data,
                    "is_base64": binary,
                }),
            );
        }
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
                    && !matches!(event.disposition, Some(WebSocketFrameDisposition::Dropped))
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
        let sockets = self.matching_sockets(&matcher);
        for socket in &sockets {
            self.queue_command(
                socket,
                json!({"type": "send_to_page", "data": data, "is_base64": false}),
            );
            self.record(
                &socket.route_id,
                EventDraft::new(WebSocketEventKind::FrameReceived)
                    .routed()
                    .frame(mask_text(data), 1)
                    .disposition(WebSocketFrameDisposition::Forwarded),
            );
        }
        Ok(sockets.len())
    }

    pub fn close_sockets(
        &self,
        pattern: &UrlPattern,
        code: Option<u16>,
        reason: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let matcher = matcher_for_pattern(pattern)?;
        let sockets = self.matching_sockets(&matcher);
        for socket in &sockets {
            self.queue_command(
                socket,
                json!({"type": "close", "code": code, "reason": reason}),
            );
            self.record(
                &socket.route_id,
                EventDraft::new(WebSocketEventKind::Closed)
                    .routed()
                    .close(code, reason.map(mask_text)),
            );
        }
        Ok(sockets.into_iter().map(|socket| socket.route_id).collect())
    }

    pub fn forget_sockets(&self, route_ids: &[String]) {
        let mut state = self.state.lock().unwrap();
        for route_id in route_ids {
            state.routed_sockets.remove(route_id);
        }
    }

    pub fn flush_commands(&self, tabs: &[std::sync::Arc<Tab>]) -> Result<(), String> {
        for tab in tabs {
            self.flush_tab_commands(tab)?;
        }
        Ok(())
    }

    pub fn flush_tab_commands(&self, tab: &Tab) -> Result<(), String> {
        self.dispatch_commands(tab.get_target_id(), |payload| {
            tab.evaluate(
                &format!("globalThis[{WEBSOCKET_DISPATCH:?}]?.({payload})"),
                false,
            )
            .map(|_| ())
            .map_err(|error| format!("Failed to dispatch WebSocket command: {error}"))
        })
    }

    fn dispatch_commands(
        &self,
        tab_target_id: &str,
        mut dispatch: impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let commands = self.take_commands_for(tab_target_id);
        for (index, command) in commands.iter().enumerate() {
            let payload = match serde_json::to_string(command) {
                Ok(payload) => payload,
                Err(error) => {
                    self.requeue_commands(tab_target_id, &commands[index + 1..]);
                    return Err(format!("Failed to serialize WebSocket command: {error}"));
                }
            };
            if let Err(error) = dispatch(&payload) {
                self.requeue_commands(tab_target_id, &commands[index..]);
                return Err(error);
            }
        }
        Ok(())
    }

    fn requeue_commands(&self, tab_target_id: &str, commands: &[Value]) {
        if commands.is_empty() {
            return;
        }
        let mut state = self.state.lock().unwrap();
        let mut pending = commands
            .iter()
            .map(|command| (tab_target_id.to_string(), command.clone()))
            .collect::<Vec<_>>();
        pending.append(&mut state.commands);
        state.commands = pending;
    }

    pub fn has_pending_commands(&self, tab_target_id: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .commands
            .iter()
            .any(|(target_id, _)| target_id == tab_target_id)
    }

    pub fn wait_for_pending_commands(&self, tab_target_id: &str, timeout: Duration) -> bool {
        let state = self.state.lock().unwrap();
        if state
            .commands
            .iter()
            .any(|(target_id, _)| target_id == tab_target_id)
        {
            return true;
        }
        let (state, _) = self.changed.wait_timeout(state, timeout).unwrap();
        state
            .commands
            .iter()
            .any(|(target_id, _)| target_id == tab_target_id)
    }

    fn take_commands_for(&self, tab_target_id: &str) -> Vec<Value> {
        let mut state = self.state.lock().unwrap();
        let (mine, rest) = std::mem::take(&mut state.commands)
            .into_iter()
            .partition::<Vec<_>, _>(|(target_id, _)| target_id == tab_target_id);
        state.commands = rest;
        mine.into_iter().map(|(_, command)| command).collect()
    }

    fn matching_sockets(&self, matcher: &UrlMatcher) -> Vec<RoutedSocket> {
        self.state
            .lock()
            .unwrap()
            .routed_sockets
            .values()
            .filter(|socket| matcher.is_match(&socket.url))
            .cloned()
            .collect()
    }

    fn record(&self, socket_id: &str, draft: EventDraft) {
        let mut state = self.state.lock().unwrap();
        let (url, protocols) = match state.routed_sockets.get(socket_id) {
            Some(socket) => (socket.url.clone(), socket.protocols.clone()),
            None => (
                state.urls.get(socket_id).cloned().unwrap_or_default(),
                Vec::new(),
            ),
        };
        push_event(&mut state, socket_id.to_string(), url, protocols, draft);
        self.changed.notify_all();
    }

    fn queue_command(&self, socket: &RoutedSocket, mut command: Value) {
        command["id"] = Value::String(socket.route_id.clone());
        self.state
            .lock()
            .unwrap()
            .commands
            .push((socket.tab_target_id.clone(), command));
        self.changed.notify_all();
    }

    #[cfg(test)]
    fn take_commands(&self) -> Vec<Value> {
        std::mem::take(&mut self.state.lock().unwrap().commands)
            .into_iter()
            .map(|(_, command)| command)
            .collect()
    }
}

fn disposition_for(action: WebSocketMessageAction) -> WebSocketFrameDisposition {
    match action {
        WebSocketMessageAction::Forward => WebSocketFrameDisposition::Forwarded,
        WebSocketMessageAction::Capture => WebSocketFrameDisposition::Captured,
        WebSocketMessageAction::Drop => WebSocketFrameDisposition::Dropped,
    }
}

fn frame_text(data: &str, opcode: u8) -> String {
    if opcode == 1 {
        mask_text(data)
    } else {
        format!("[binary frame: {} bytes]", decoded_len(data))
    }
}

fn push_event(
    state: &mut WebSocketState,
    socket_id: String,
    url: String,
    protocols: Vec<String>,
    draft: EventDraft,
) {
    state.sequence += 1;
    state.events.push_back(WebSocketEvent {
        sequence: state.sequence,
        socket_id,
        url: mask_text(&url),
        kind: draft.kind,
        data: draft.data,
        opcode: draft.opcode,
        status: draft.status,
        error: draft.error,
        routed: draft.routed,
        protocols,
        disposition: draft.disposition,
        close_code: draft.close_code,
        close_reason: draft.close_reason,
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

fn spawn_command_flusher(tab: &std::sync::Arc<Tab>, registry: std::sync::Arc<WebSocketRegistry>) {
    let target_id = tab.get_target_id().to_string();
    let flusher_tab = std::sync::Arc::downgrade(tab);
    std::thread::spawn(move || loop {
        let pending = registry.wait_for_pending_commands(&target_id, FLUSH_POLL_INTERVAL);
        let Some(tab) = flusher_tab.upgrade() else {
            return;
        };
        if pending {
            if let Err(error) = registry.flush_tab_commands(&tab) {
                tracing::warn!("WebSocket command dispatch failed: {error}");
            }
        }
    });
}

pub fn install_websocket_router(
    tab: &std::sync::Arc<Tab>,
    registry: std::sync::Arc<WebSocketRegistry>,
) -> Result<(), String> {
    let target_id = tab.get_target_id().to_string();
    spawn_command_flusher(tab, registry.clone());
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
      this.protocols = protocols === undefined ? [] : (Array.isArray(protocols) ? protocols : [protocols]);
      this.readyState = 0;
      this.binaryType = 'blob';
      this.bufferedAmount = 0;
      this.extensions = '';
      this.protocol = '';
      this.onopen = null; this.onmessage = null; this.onerror = null; this.onclose = null;
      this.id = `refact-ws-${{++nextId}}`;
      sockets.set(this.id, this);
      emit({{ type: 'created', id: this.id, url: this.url, protocols: this.protocols }});
    }}
    send(data) {{
      if (this.readyState !== 1) throw new DOMException('WebSocket is not open');
      toWire(data).then(wire => emit({{ type: 'page_message', id: this.id, ...wire }}));
    }}
    close(code = 1000, reason = '') {{
      this.readyState = 3;
      this.server?.close(code, reason);
      this.dispatchEvent(new CloseEvent('close', {{ code, reason, wasClean: true }}));
      emit({{ type: 'closed', id: this.id, code, reason }});
      sockets.delete(this.id);
    }}
    serverClose(code, reason) {{
      const closeCode = code ?? 1000;
      const closeReason = reason ?? '';
      this.readyState = 3;
      try {{ this.server?.close(); }} catch (error) {{ void error; }}
      this.server = undefined;
      const close = new CloseEvent('close', {{ code: closeCode, reason: closeReason, wasClean: closeCode === 1000 }});
      this.dispatchEvent(close); this.onclose?.(close);
      sockets.delete(this.id);
    }}
    open() {{
      if (this.readyState !== 0) return;
      this.readyState = 1;
      const event = new Event('open'); this.dispatchEvent(event); this.onopen?.(event);
    }}
    connect() {{
      this.server = this.protocols.length ? new NativeWebSocket(this.url, this.protocols) : new NativeWebSocket(this.url);
      this.server.binaryType = this.binaryType;
      this.server.onopen = () => {{ this.protocol = this.server.protocol; this.open(); }};
      this.server.onmessage = event => toWire(event.data).then(wire => emit({{ type: 'server_message', id: this.id, ...wire }}));
      this.server.onerror = () => emit({{ type: 'error', id: this.id, message: 'WebSocket server error' }});
      this.server.onclose = event => {{
        if (this.readyState === 3) return;
        this.readyState = 3;
        const close = new CloseEvent('close', {{ code: event.code, reason: event.reason, wasClean: event.wasClean }});
        this.dispatchEvent(close); this.onclose?.(close);
        emit({{ type: 'closed', id: this.id, code: event.code, reason: event.reason }});
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
    if (request.type === 'close') socket.serverClose(request.code, request.reason);
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

        assert!(
            websocket_event_from_binding_payload(&Value::String("not json".to_string())).is_none()
        );
        assert!(websocket_event_from_binding_payload(&Value::String(
            json!({"seq": 1}).to_string()
        ))
        .is_none());
    }

    fn routed_registry(
        pattern: &UrlPattern,
        mode: WebSocketRouteMode,
        on_page_message: WebSocketMessageAction,
        on_server_message: WebSocketMessageAction,
    ) -> WebSocketRegistry {
        let registry = WebSocketRegistry::default();
        registry
            .add_route(pattern.clone(), mode, on_page_message, on_server_message)
            .unwrap();
        registry
    }

    fn created(registry: &WebSocketRegistry, url: &str, protocols: Value) {
        registry.handle_page_event(
            "tab-1",
            &json!({"type": "created", "id": "route-1", "url": url, "protocols": protocols}),
        );
        registry.take_commands();
    }

    #[test]
    fn commands_that_fail_to_dispatch_are_requeued_in_order() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = routed_registry(
            &pattern,
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        );
        created(&registry, "ws://127.0.0.1:9/ws-echo", json!([]));

        registry.send_to_page(&pattern, "first").unwrap();
        registry.send_to_page(&pattern, "second").unwrap();
        registry.send_to_page(&pattern, "third").unwrap();

        let mut seen = Vec::new();
        let error = registry
            .dispatch_commands("tab-1", |payload| {
                if seen.len() == 1 {
                    return Err("tab is gone".to_string());
                }
                seen.push(payload.to_string());
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error, "tab is gone");
        assert_eq!(seen.len(), 1);

        let remaining = registry.take_commands_for("tab-1");
        assert_eq!(remaining.len(), 2, "undispatched commands must not be lost");
        assert_eq!(remaining[0]["data"], json!("second"));
        assert_eq!(remaining[1]["data"], json!("third"));
    }

    #[test]
    fn requeued_commands_stay_ahead_of_newly_queued_ones() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = routed_registry(
            &pattern,
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        );
        created(&registry, "ws://127.0.0.1:9/ws-echo", json!([]));

        registry.send_to_page(&pattern, "first").unwrap();
        assert!(registry
            .dispatch_commands("tab-1", |_| Err("offline".to_string()))
            .is_err());
        registry.send_to_page(&pattern, "second").unwrap();

        let remaining = registry.take_commands_for("tab-1");
        assert_eq!(remaining[0]["data"], json!("first"));
        assert_eq!(remaining[1]["data"], json!("second"));
    }

    #[test]
    fn close_keeps_the_socket_registered_until_dispatch_succeeds() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = routed_registry(
            &pattern,
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        );
        created(&registry, "ws://127.0.0.1:9/ws-echo", json!([]));

        let closing = registry.close_sockets(&pattern, Some(1001), None).unwrap();
        assert_eq!(closing, vec!["route-1".to_string()]);
        assert!(registry
            .dispatch_commands("tab-1", |_| Err("tab is gone".to_string()))
            .is_err());
        assert_eq!(
            registry.send_to_page(&pattern, "still-here").unwrap(),
            1,
            "a socket whose close never reached the page stays addressable"
        );

        registry.forget_sockets(&closing);
        assert_eq!(registry.send_to_page(&pattern, "after-close").unwrap(), 0);
    }

    #[test]
    fn queued_commands_wake_the_flusher_instead_of_the_binding_callback() {
        let registry = std::sync::Arc::new(routed_registry(
            &UrlPattern::Text("ws://**/ws-echo".to_string()),
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        ));
        let waiter = registry.clone();
        let observer = std::thread::spawn(move || {
            waiter.wait_for_pending_commands("tab-1", Duration::from_secs(2))
        });
        std::thread::sleep(Duration::from_millis(20));
        registry.handle_page_event(
            "tab-1",
            &json!({"type": "created", "id": "route-1", "url": "ws://127.0.0.1:9/ws-echo"}),
        );
        assert!(observer.join().unwrap());
        assert!(registry.has_pending_commands("tab-1"));
        assert!(!registry.has_pending_commands("tab-2"));
    }

    #[test]
    fn waiting_for_commands_gives_up_when_nothing_is_queued() {
        let registry = WebSocketRegistry::default();
        let started = Instant::now();
        assert!(!registry.wait_for_pending_commands("tab-1", Duration::from_millis(50)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn top_level_navigation_drops_only_the_navigating_tabs_routed_sockets() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = routed_registry(
            &pattern,
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        );
        for (tab, route) in [
            ("tab-1", "route-1"),
            ("tab-1", "route-2"),
            ("tab-2", "route-3"),
        ] {
            registry.handle_page_event(
                tab,
                &json!({"type": "created", "id": route, "url": "ws://127.0.0.1:9/ws-echo", "protocols": []}),
            );
        }
        registry.take_commands();
        assert_eq!(registry.routed_socket_count(), 3);

        assert_eq!(registry.top_level_navigation("tab-1"), 2);
        assert_eq!(registry.routed_socket_count(), 1);
        assert_eq!(
            registry.send_to_page(&pattern, "still routed").unwrap(),
            1,
            "the untouched tab keeps its routed socket"
        );

        assert_eq!(
            registry.top_level_navigation("tab-1"),
            0,
            "navigating again cannot remove what is already gone"
        );
    }

    #[test]
    fn repeated_navigation_keeps_the_routed_socket_map_bounded() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = routed_registry(
            &pattern,
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        );
        for document in 0..100 {
            registry.handle_page_event(
                "tab-1",
                &json!({"type": "created", "id": format!("route-{document}"), "url": "ws://127.0.0.1:9/ws-echo", "protocols": []}),
            );
            registry.take_commands();
            registry.top_level_navigation("tab-1");
            assert_eq!(registry.routed_socket_count(), 0);
        }
    }

    #[test]
    fn created_binding_payload_registers_socket_reachable_by_send() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = WebSocketRegistry::default();
        registry
            .add_route(
                pattern.clone(),
                WebSocketRouteMode::Mock,
                WebSocketMessageAction::Forward,
                WebSocketMessageAction::Forward,
            )
            .unwrap();
        let payload = Value::String(
            json!({"type": "created", "id": "refact-ws-1", "url": "ws://127.0.0.1:8123/ws-echo"})
                .to_string(),
        );
        let event = websocket_event_from_binding_payload(&payload).unwrap();
        registry.handle_page_event("tab-1", &event);
        assert_eq!(registry.send_to_page(&pattern, "mocked-frame").unwrap(), 1);
        let events = registry.drain_report();
        assert!(events
            .iter()
            .any(|event| { matches!(event.kind, WebSocketEventKind::Created) && event.routed }));
        assert!(events.iter().any(|event| {
            matches!(event.kind, WebSocketEventKind::FrameReceived)
                && event.data.as_deref() == Some("mocked-frame")
        }));
    }

    #[test]
    fn routed_websocket_events_forward_by_default_and_agent_messages_are_masked() {
        let registry = routed_registry(
            &UrlPattern::Text("wss://example.test/**".to_string()),
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        );
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

    #[test]
    fn intercept_mode_connects_to_the_real_server_and_forwards_both_directions() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = routed_registry(
            &pattern,
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        );

        registry.handle_page_event(
            "tab-1",
            &json!({"type": "created", "id": "route-1", "url": "ws://127.0.0.1:9/ws-echo"}),
        );
        assert_eq!(registry.take_commands()[0]["type"], json!("connect"));

        registry.handle_page_event(
            "tab-1",
            &json!({"type": "page_message", "id": "route-1", "data": "up", "is_base64": false}),
        );
        registry.handle_page_event(
            "tab-1",
            &json!({"type": "server_message", "id": "route-1", "data": "down", "is_base64": false}),
        );

        let commands = registry.take_commands();
        assert_eq!(commands[0]["type"], json!("send_to_server"));
        assert_eq!(commands[0]["data"], json!("up"));
        assert_eq!(commands[1]["type"], json!("send_to_page"));
        assert_eq!(commands[1]["data"], json!("down"));

        let events = registry.drain_report();
        assert!(events.iter().all(|event| event.routed));
        assert_eq!(
            events
                .iter()
                .filter_map(|event| event.disposition)
                .collect::<Vec<_>>(),
            vec![
                WebSocketFrameDisposition::Forwarded,
                WebSocketFrameDisposition::Forwarded
            ]
        );
    }

    #[test]
    fn mock_mode_opens_the_socket_without_connecting_to_the_real_server() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = routed_registry(
            &pattern,
            WebSocketRouteMode::Mock,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        );

        registry.handle_page_event(
            "tab-1",
            &json!({"type": "created", "id": "route-1", "url": "ws://127.0.0.1:9/ws-echo"}),
        );

        assert_eq!(registry.take_commands()[0]["type"], json!("open"));
    }

    #[test]
    fn unmatched_sockets_pass_through_to_the_real_server_and_are_not_reported_as_routed() {
        let registry = routed_registry(
            &UrlPattern::Text("ws://**/routed".to_string()),
            WebSocketRouteMode::Mock,
            WebSocketMessageAction::Drop,
            WebSocketMessageAction::Drop,
        );

        registry.handle_page_event(
            "tab-1",
            &json!({"type": "created", "id": "route-1", "url": "ws://127.0.0.1:9/other"}),
        );
        assert_eq!(registry.take_commands()[0]["type"], json!("connect"));

        registry.handle_page_event(
            "tab-1",
            &json!({"type": "page_message", "id": "route-1", "data": "up", "is_base64": false}),
        );

        assert_eq!(registry.take_commands()[0]["type"], json!("send_to_server"));
        let events = registry.drain_report();
        assert!(events.iter().all(|event| !event.routed));
    }

    #[test]
    fn drop_blocks_the_frame_and_hides_it_from_wait_for_frame() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = routed_registry(
            &pattern,
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Drop,
            WebSocketMessageAction::Forward,
        );
        created(&registry, "ws://127.0.0.1:9/ws-echo", json!([]));

        registry.handle_page_event(
            "tab-1",
            &json!({"type": "page_message", "id": "route-1", "data": "blocked", "is_base64": false}),
        );

        assert!(registry.take_commands().is_empty());
        let events = registry.drain_report();
        let frame = events
            .iter()
            .find(|event| matches!(event.kind, WebSocketEventKind::FrameSent))
            .unwrap();
        assert_eq!(
            frame.disposition,
            Some(WebSocketFrameDisposition::Dropped),
            "dropped frames stay visible in the report"
        );
        assert!(
            registry
                .wait_for_frame(None, 0, Duration::from_millis(50))
                .is_err(),
            "a dropped frame must not satisfy wait_for_web_socket_frame"
        );
    }

    #[test]
    fn capture_surfaces_the_frame_without_forwarding_and_still_redacts_it() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = routed_registry(
            &pattern,
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Capture,
        );
        created(&registry, "ws://127.0.0.1:9/ws-echo", json!([]));

        registry.handle_page_event(
            "tab-1",
            &json!({"type": "server_message", "id": "route-1", "data": "token=secret", "is_base64": false}),
        );

        assert!(
            registry.take_commands().is_empty(),
            "captured frames are not forwarded"
        );
        let frame = registry
            .wait_for_frame(None, 0, Duration::from_millis(50))
            .unwrap();
        assert_eq!(frame.disposition, Some(WebSocketFrameDisposition::Captured));
        assert_eq!(frame.data.as_deref(), Some("token=[REDACTED]"));
    }

    #[test]
    fn close_web_socket_simulates_a_server_close_with_code_and_reason() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = routed_registry(
            &pattern,
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        );
        created(&registry, "ws://127.0.0.1:9/ws-echo", json!([]));

        let closed = registry
            .close_sockets(&pattern, Some(4002), Some("server restarting"))
            .unwrap();
        assert_eq!(closed.len(), 1);
        registry.forget_sockets(&closed);

        let command = registry.take_commands().remove(0);
        assert_eq!(command["type"], json!("close"));
        assert_eq!(command["code"], json!(4002));
        assert_eq!(command["reason"], json!("server restarting"));
        assert_eq!(command["id"], json!("route-1"));

        let closed = registry
            .drain_report()
            .into_iter()
            .find(|event| matches!(event.kind, WebSocketEventKind::Closed))
            .unwrap();
        assert_eq!(closed.close_code, Some(4002));
        assert_eq!(closed.close_reason.as_deref(), Some("server restarting"));

        assert_eq!(
            registry.send_to_page(&pattern, "after-close").unwrap(),
            0,
            "a closed socket is no longer addressable"
        );
    }

    #[test]
    fn page_requested_subprotocols_are_reported_on_every_event_for_the_socket() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = routed_registry(
            &pattern,
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        );
        created(
            &registry,
            "ws://127.0.0.1:9/ws-echo",
            json!(["graphql-ws", "soap"]),
        );

        registry.handle_page_event(
            "tab-1",
            &json!({"type": "server_message", "id": "route-1", "data": "frame", "is_base64": false}),
        );

        let events = registry.drain_report();
        assert!(!events.is_empty());
        assert!(
            events
                .iter()
                .all(|event| event.protocols == vec!["graphql-ws".to_string(), "soap".to_string()]),
            "every event for the socket carries the requested subprotocols: {events:?}"
        );
    }

    #[test]
    fn binary_frames_are_summarized_by_length_in_both_directions() {
        let pattern = UrlPattern::Text("ws://**/ws-echo".to_string());
        let registry = routed_registry(
            &pattern,
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Capture,
            WebSocketMessageAction::Capture,
        );
        created(&registry, "ws://127.0.0.1:9/ws-echo", json!([]));

        registry.handle_page_event(
            "tab-1",
            &json!({"type": "page_message", "id": "route-1", "data": "AAECAw==", "is_base64": true}),
        );

        let frame = registry
            .wait_for_frame(None, 0, Duration::from_millis(50))
            .unwrap();
        assert_eq!(frame.opcode, Some(2));
        assert_eq!(frame.data.as_deref(), Some("[binary frame: 4 bytes]"));
    }

    #[test]
    fn each_tab_only_drains_its_own_pending_commands() {
        let registry = routed_registry(
            &UrlPattern::Text("ws://**/ws-echo".to_string()),
            WebSocketRouteMode::Intercept,
            WebSocketMessageAction::Forward,
            WebSocketMessageAction::Forward,
        );

        registry.handle_page_event(
            "tab-1",
            &json!({"type": "created", "id": "route-1", "url": "ws://127.0.0.1:9/ws-echo"}),
        );
        registry.handle_page_event(
            "tab-2",
            &json!({"type": "created", "id": "route-2", "url": "ws://127.0.0.1:9/ws-echo"}),
        );

        let first = registry.take_commands_for("tab-1");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0]["id"], json!("route-1"));

        assert!(registry.take_commands_for("tab-3").is_empty());

        let remaining = registry.take_commands();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0]["id"], json!("route-2"));
    }

    #[test]
    fn shim_reports_protocols_and_handles_a_simulated_server_close() {
        let script = websocket_mock_script();
        assert!(script.contains("protocols: this.protocols"));
        assert!(script.contains("serverClose(code, reason)"));
        assert!(script.contains("if (request.type === 'close') socket.serverClose"));
    }

    #[test]
    fn shim_ignores_the_upstream_close_once_the_page_socket_is_already_closed() {
        let script = websocket_mock_script();
        let handler = script
            .split("this.server.onclose = event => {")
            .nth(1)
            .unwrap();
        let first_statement = handler
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap();
        assert_eq!(
            first_statement, "if (this.readyState === 3) return;",
            "a simulated close must not be overwritten by the upstream close event: {handler}"
        );
    }
}
