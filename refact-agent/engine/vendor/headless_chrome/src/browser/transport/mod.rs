use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};
use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

use anyhow::Result;

use thiserror::Error;

use log::{error, info, trace, warn};
use url::Url;
use waiting_call_registry::WaitingCallRegistry;
use web_socket_connection::WebSocketConnection;

use crate::protocol::cdp::{Target, types::Event, types::Method};

use crate::types::{CallId, Message, parse_raw_message, parse_response};

mod waiting_call_registry;
mod web_socket_connection;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

#[derive(Debug)]
pub enum MethodDestination {
    Target(SessionId),
    Browser,
}

impl SessionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for SessionId {
    fn from(session_id: String) -> Self {
        Self(session_id)
    }
}

#[derive(Debug, Eq, PartialEq, Hash)]
enum ListenerId {
    SessionId(SessionId),
    Browser,
}

type Listeners = Arc<Mutex<HashMap<ListenerId, Sender<Event>>>>;

#[derive(Debug)]
pub struct Transport {
    web_socket_connection: Arc<WebSocketConnection>,
    waiting_call_registry: Arc<WaitingCallRegistry>,
    listeners: Listeners,
    open: Arc<AtomicBool>,
    call_id_counter: Arc<AtomicU32>,
    loop_shutdown_tx: Mutex<mpsc::SyncSender<()>>,
    command_timeout: Mutex<Duration>,
    incoming_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    ws_url: Url,
}

#[derive(Debug, Error)]
#[error("Unable to make method calls because underlying connection is closed")]
pub struct ConnectionClosed {}

/// A command may have reached Chrome before its response deadline expired. Never replay it.
#[derive(Debug, Error)]
#[error("CDP command deadline exceeded: {method} (effect_unknown={effect_unknown})")]
pub struct CommandTimeout {
    pub method: &'static str,
    pub effect_unknown: bool,
}

thread_local! { static COMMAND_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) }; }

/// Request-scoped deadline for synchronous CDP work on this thread. Nested guards only shorten it.
pub struct CommandDeadlineGuard {
    previous: Option<Instant>,
    _not_send: PhantomData<Rc<()>>,
}
impl CommandDeadlineGuard {
    /// Absolute request deadline on this thread, if installed. Does not start a new budget.
    pub fn current_deadline() -> Option<Instant> {
        COMMAND_DEADLINE.with(Cell::get)
    }

    pub fn new(deadline: Instant) -> Self {
        let previous = COMMAND_DEADLINE
            .with(|d| d.replace(Some(d.get().map_or(deadline, |p| p.min(deadline)))));
        Self {
            previous,
            _not_send: PhantomData,
        }
    }
}
impl Drop for CommandDeadlineGuard {
    fn drop(&mut self) {
        COMMAND_DEADLINE.with(|d| d.set(self.previous));
    }
}

/// Cloneable connection-generation cancellation. Closes only the CDP socket, not Chrome.
#[derive(Clone)]
pub struct CancellationHandle {
    open: Arc<AtomicBool>,
    registry: Arc<WaitingCallRegistry>,
    connection: Arc<WebSocketConnection>,
}
impl CancellationHandle {
    pub fn cancel(&self) {
        self.open.store(false, Ordering::SeqCst);
        self.registry.cancel_outstanding_method_calls();
        self.connection.interrupt();
    }
}

/// Fixed-cardinality, calling-thread telemetry. No method arguments or dynamic names retained.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct ProfileCounters {
    pub commands: u64,
    pub command_us: u64,
    pub send_us: u64,
    pub target_acks: u64,
    pub evaluate: u64,
    pub call_function: u64,
    pub input: u64,
    pub frame_tree: u64,
    pub other: u64,
    /// Inclusive command duration buckets: <=1,5,10,20,50,100,500ms, then overflow.
    pub latency_buckets: [u64; 8],
}
thread_local! {
    static PROFILE_COUNTERS: Cell<ProfileCounters> = const { Cell::new(ProfileCounters {
        commands: 0, command_us: 0, send_us: 0, target_acks: 0,
        evaluate: 0, call_function: 0, input: 0, frame_tree: 0, other: 0, latency_buckets: [0; 8],
    }) };
}
pub fn profiling_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("REFACT_BROWSER_PROFILE").is_some_and(|v| v == "1"))
}
pub fn profile_counters() -> ProfileCounters {
    PROFILE_COUNTERS.with(Cell::get)
}
struct ProfileCommand(Option<Instant>);
impl ProfileCommand {
    fn new(method: &str) -> Self {
        if !profiling_enabled() {
            return Self(None);
        }
        PROFILE_COUNTERS.with(|cell| {
            let mut c = cell.get();
            c.commands = c.commands.saturating_add(1);
            match method {
                "Target.sendMessageToTarget" => c.target_acks = c.target_acks.saturating_add(1),
                "Runtime.evaluate" => c.evaluate = c.evaluate.saturating_add(1),
                "Runtime.callFunctionOn" => c.call_function = c.call_function.saturating_add(1),
                "Page.getFrameTree" => c.frame_tree = c.frame_tree.saturating_add(1),
                "Input.dispatchMouseEvent"
                | "Input.dispatchKeyEvent"
                | "Input.insertText"
                | "Input.dispatchTouchEvent" => c.input = c.input.saturating_add(1),
                _ => c.other = c.other.saturating_add(1),
            }
            cell.set(c);
        });
        Self(Some(Instant::now()))
    }
}
impl Drop for ProfileCommand {
    fn drop(&mut self) {
        if let Some(start) = self.0 {
            PROFILE_COUNTERS.with(|cell| {
                let mut c = cell.get();
                let us = start.elapsed().as_micros();
                let bucket = [1000, 5000, 10000, 20000, 50000, 100000, 500000]
                    .iter()
                    .position(|bound| us <= *bound)
                    .unwrap_or(7);
                c.latency_buckets[bucket] = c.latency_buckets[bucket].saturating_add(1);
                c.command_us = c
                    .command_us
                    .saturating_add(start.elapsed().as_micros().min(u64::MAX as u128) as u64);
                cell.set(c);
            });
        }
    }
}

impl Transport {
    pub fn new(
        ws_url: Url,
        process_id: Option<u32>,
        idle_browser_timeout: Duration,
        root_cert: Option<Vec<u8>>,
    ) -> Result<Self> {
        let (messages_tx, messages_rx) = mpsc::channel();
        let web_socket_connection = Arc::new(WebSocketConnection::new(
            &ws_url,
            process_id,
            messages_tx,
            root_cert,
        )?);

        let waiting_call_registry = Arc::new(WaitingCallRegistry::new());

        let listeners = Arc::new(Mutex::new(HashMap::new()));

        let open = Arc::new(AtomicBool::new(true));

        let (shutdown_tx, shutdown_rx) = mpsc::sync_channel(100);

        let guarded_shutdown_tx = Mutex::new(shutdown_tx);

        let incoming_thread = Self::handle_incoming_messages(
            messages_rx,
            Arc::clone(&waiting_call_registry),
            Arc::clone(&listeners),
            Arc::clone(&open),
            Arc::clone(&web_socket_connection),
            shutdown_rx,
            process_id,
            idle_browser_timeout,
        );

        Ok(Self {
            web_socket_connection,
            waiting_call_registry,
            listeners,
            open,
            call_id_counter: Arc::new(AtomicU32::new(0)),
            loop_shutdown_tx: guarded_shutdown_tx,
            command_timeout: Mutex::new(Duration::from_secs(30)),
            incoming_thread: Mutex::new(Some(incoming_thread)),
            ws_url,
        })
    }

    /// Returns a number based on thread-safe unique counter, incrementing it so that the
    /// next CallId is different.
    pub fn unique_call_id(&self) -> CallId {
        self.call_id_counter.fetch_add(1, Ordering::SeqCst)
    }

    pub fn call_method<C>(
        &self,
        method: C,
        destination: MethodDestination,
    ) -> Result<C::ReturnObject>
    where
        C: Method + serde::Serialize,
    {
        self.call_method_with_deadline(method, destination, self.command_deadline())
    }

    fn command_deadline(&self) -> Instant {
        let default_deadline = Instant::now() + *self.command_timeout.lock().unwrap();
        COMMAND_DEADLINE.with(|d| {
            d.get()
                .map_or(default_deadline, |r| r.min(default_deadline))
        })
    }

    pub fn set_default_command_timeout(&self, timeout: Duration) -> Result<()> {
        anyhow::ensure!(
            !timeout.is_zero() && timeout <= Duration::from_secs(86_400),
            "command timeout must be in (0, 86400s]"
        );
        *self.command_timeout.lock().unwrap() = timeout;
        Ok(())
    }

    pub fn cancellation_handle(&self) -> CancellationHandle {
        CancellationHandle {
            open: self.open.clone(),
            registry: self.waiting_call_registry.clone(),
            connection: self.web_socket_connection.clone(),
        }
    }

    pub fn ws_url(&self) -> &Url {
        &self.ws_url
    }

    /// One absolute deadline includes queueing, sending, outer target ACK and inner response.
    pub fn call_method_with_deadline<C>(
        &self,
        method: C,
        destination: MethodDestination,
        deadline: Instant,
    ) -> Result<C::ReturnObject>
    where
        C: Method + serde::Serialize,
    {
        let _profile = ProfileCommand::new(C::NAME);
        let call_id = self.unique_call_id();
        let message_text = serde_json::to_string(&method.to_method_call(call_id))?;
        let response =
            self.call_serialized(call_id, message_text, destination, deadline, C::NAME)?;
        parse_response::<C::ReturnObject>(response)
    }

    /// Calls a runtime-selected CDP method, using the flattened session wire format.
    /// Shares typed calls' deadline, cancellation, write bound and waiter cleanup.
    pub fn call_method_json(
        &self,
        method: &str,
        params: serde_json::Value,
        session: Option<&str>,
    ) -> Result<serde_json::Value> {
        let _profile = ProfileCommand::new(method);
        let deadline = self.command_deadline();
        let call_id = self.unique_call_id();
        let mut frame = serde_json::json!({"id": call_id, "method": method});
        if !params.is_null() {
            frame["params"] = params;
        }
        if let Some(session) = session {
            frame["sessionId"] = serde_json::Value::String(session.to_owned());
        }
        let response = self.call_serialized(
            call_id,
            serde_json::to_string(&frame)?,
            MethodDestination::Browser,
            deadline,
            "raw CDP",
        )?;
        if let Some(error) = response.error {
            return Err(error.into());
        }
        Ok(response.result.unwrap_or(serde_json::Value::Null))
    }

    fn call_serialized(
        &self,
        call_id: CallId,
        message_text: String,
        destination: MethodDestination,
        deadline: Instant,
        method_name: &'static str,
    ) -> Result<crate::types::Response> {
        if !self.open.load(Ordering::SeqCst) {
            return Err(ConnectionClosed {}.into());
        }
        if Instant::now() >= deadline {
            return Err(CommandTimeout {
                method: method_name,
                effect_unknown: false,
            }
            .into());
        }
        let response_rx = self.waiting_call_registry.register_call(call_id);
        // Covers shutdown racing registration; every exit removes this waiter exactly once.
        let result: Result<crate::types::Response> = (|| {
            if !self.open.load(Ordering::SeqCst) {
                return Err(ConnectionClosed {}.into());
            }
            match destination {
                MethodDestination::Target(session_id) => {
                    self.call_method_with_deadline(
                        Target::SendMessageToTarget {
                            target_id: None,
                            session_id: Some(session_id.0),
                            message: message_text,
                        },
                        MethodDestination::Browser,
                        deadline,
                    )?;
                }
                MethodDestination::Browser => self
                    .web_socket_connection
                    .send_message(&message_text, deadline)?,
            }
            let response = match response_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(response) => response?,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(CommandTimeout {
                        method: method_name,
                        effect_unknown: true,
                    }
                    .into());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(ConnectionClosed {}.into()),
            };
            Ok(response)
        })();
        self.waiting_call_registry.unregister_call(call_id);
        result.map_err(|error| {
            if let Some(timeout) = error.downcast_ref::<CommandTimeout>() {
                CommandTimeout {
                    method: method_name,
                    effect_unknown: timeout.effect_unknown,
                }
                .into()
            } else {
                error
            }
        })
    }

    pub fn call_method_on_target<C>(
        &self,
        session_id: SessionId,
        method: C,
    ) -> Result<C::ReturnObject>
    where
        C: Method + serde::Serialize,
    {
        // TODO: remove clone
        self.call_method(method, MethodDestination::Target(session_id))
    }

    pub fn call_method_on_browser<C>(&self, method: C) -> Result<C::ReturnObject>
    where
        C: Method + serde::Serialize,
    {
        self.call_method(method, MethodDestination::Browser)
    }

    pub fn listen_to_browser_events(&self) -> Receiver<Event> {
        let (events_tx, events_rx) = mpsc::channel();

        let mut listeners = self.listeners.lock().unwrap();
        listeners.insert(ListenerId::Browser, events_tx);

        events_rx
    }

    pub fn listen_to_target_events(&self, session_id: SessionId) -> Receiver<Event> {
        let (events_tx, events_rx) = mpsc::channel();

        let mut listeners = self.listeners.lock().unwrap();
        listeners.insert(ListenerId::SessionId(session_id), events_tx);

        events_rx
    }

    pub fn shutdown(&self) {
        self.cancellation_handle().cancel();
        let _ = self.loop_shutdown_tx.lock().unwrap().try_send(());
        self.web_socket_connection.shutdown();
        if let Some(thread) = self.incoming_thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_incoming_messages(
        messages_rx: Receiver<Message>,
        waiting_call_registry: Arc<WaitingCallRegistry>,
        listeners: Listeners,
        open: Arc<AtomicBool>,
        conn: Arc<WebSocketConnection>,
        shutdown_rx: Receiver<()>,
        process_id: Option<u32>,
        idle_browser_timeout: Duration,
    ) -> std::thread::JoinHandle<()> {
        trace!("Starting handle_incoming_messages");
        std::thread::spawn(move || {
            trace!("Inside handle_incoming_messages thread");
            // this iterator calls .recv() under the hood, so can block thread forever
            // hence need for Connection Shutdown
            loop {
                match shutdown_rx.try_recv() {
                    Ok(()) | Err(TryRecvError::Disconnected) => {
                        info!("Transport incoming message loop loop received shutdown message");
                        break;
                    }
                    Err(TryRecvError::Empty) => {}
                }
                match messages_rx.recv_timeout(idle_browser_timeout) {
                    Err(recv_timeout_error) => {
                        match recv_timeout_error {
                            RecvTimeoutError::Timeout => {
                                error!(
                                    "Transport loop got a timeout while listening for messages (Chrome #{process_id:?})",
                                );
                            }
                            RecvTimeoutError::Disconnected => {
                                error!(
                                    "Transport loop got disconnected from WS's sender (Chrome #{process_id:?})",
                                );
                            }
                        }
                        break;
                    }
                    Ok(message) => match message {
                        Message::ConnectionShutdown => {
                            info!("Received shutdown message");
                            break;
                        }
                        Message::Response(response_to_browser_method_call) => {
                            if waiting_call_registry
                                .resolve_call(response_to_browser_method_call)
                                .is_err()
                            {
                                warn!(
                                    "The browser registered a call but then closed its receiving channel"
                                );
                                break;
                            }
                        }

                        Message::Event(browser_event) => match browser_event {
                            Event::ReceivedMessageFromTarget(target_message_event) => {
                                let session_id = target_message_event.params.session_id.into();
                                let raw_message = target_message_event.params.message;

                                let msg_res = parse_raw_message(&raw_message);
                                match msg_res {
                                    Ok(target_message) => match target_message {
                                        Message::Event(target_event) => {
                                            if let Some(tx) = listeners
                                                .lock()
                                                .unwrap()
                                                .get(&ListenerId::SessionId(session_id))
                                            {
                                                let _ = tx.send(target_event);
                                            }
                                        }

                                        Message::Response(resp) => {
                                            if waiting_call_registry.resolve_call(resp).is_err() {
                                                warn!(
                                                    "The browser registered a call but then closed its receiving channel"
                                                );
                                                break;
                                            }
                                        }
                                        Message::ConnectionShutdown => {}
                                    },
                                    Err(e) => {
                                        trace!(
                                            "Message from target isn't recognised: {:?} - {}",
                                            &raw_message, e,
                                        );
                                    }
                                }
                            }

                            _ => {
                                let mut listeners = listeners.lock().unwrap();
                                if let Some(tx) = listeners.get(&ListenerId::Browser) {
                                    if tx.send(browser_event).is_err() {
                                        warn!(
                                            "Browser event listener disconnected; retiring listener, not shared CDP transport"
                                        );
                                        listeners.remove(&ListenerId::Browser);
                                    }
                                }
                            }
                        },
                    },
                }
            }

            info!("Shutting down message handling loop");

            // Need to do this because otherwise WS thread might block forever
            conn.shutdown();

            open.store(false, Ordering::SeqCst);
            waiting_call_registry.cancel_outstanding_method_calls();
            let mut listeners = listeners.lock().unwrap();
            *listeners = HashMap::new();
            info!("cleared listeners, I think");
        })
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;
    use crate::protocol::cdp::Browser::GetVersion;
    use serde_json::{Value, json};
    use std::net::{TcpListener, TcpStream};
    use tungstenite::{WebSocket, accept};

    fn fixture(
        script: impl FnOnce(WebSocket<TcpStream>) + Send + 'static,
    ) -> (Transport, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!(
            "ws://{}/devtools/browser/test",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let thread = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            script(accept(stream).unwrap());
        });
        (
            Transport::new(url, None, Duration::from_secs(86_400), None).unwrap(),
            thread,
        )
    }

    fn read_call(ws: &mut WebSocket<TcpStream>) -> Value {
        serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap()
    }
    fn reply(ws: &mut WebSocket<TcpStream>, id: &Value) {
        ws.send(tungstenite::Message::text(json!({"id":id,"result": {
            "protocolVersion":"1.3", "product":"fake", "revision":"test", "userAgent":"fake", "jsVersion":"1"
        }}).to_string())).unwrap();
    }
    fn assert_timeout(error: anyhow::Error) {
        assert!(
            error.downcast_ref::<CommandTimeout>().is_some(),
            "wrong error: {error:#}"
        );
    }
    fn empty(transport: &Transport) {
        assert_eq!(
            transport.waiting_call_registry.calls_len(),
            0,
            "deadline leaked a waiter"
        );
    }

    fn send_json(ws: &mut WebSocket<TcpStream>, value: Value) {
        ws.send(tungstenite::Message::text(value.to_string()))
            .unwrap();
    }

    fn target_created(id: &str) -> Value {
        json!({"method":"Target.targetCreated", "params":{"targetInfo":{
            "targetId":id, "type":"page", "title":"fixture", "url":"about:blank",
            "attached":false, "canAccessOpener":false
        }}})
    }

    #[test]
    fn dropped_browser_listener_preserves_response_routing() {
        let (transport, server) = fixture(move |mut ws| {
            let call = read_call(&mut ws);
            send_json(&mut ws, target_created("orphan"));
            reply(&mut ws, &call["id"]);
            let _ = ws.read();
        });
        drop(transport.listen_to_browser_events());
        transport
            .set_default_command_timeout(Duration::from_secs(2))
            .unwrap();
        let result = transport.call_method_on_browser(GetVersion(None));
        transport.shutdown();
        server.join().unwrap();
        empty(&transport);
        assert!(transport.incoming_thread.lock().unwrap().is_none());
        assert_eq!(
            result
                .expect("dropped listener killed unrelated RPC")
                .product,
            "fake"
        );
    }

    fn failed_tab_initialization_preserves_browser(fail_page_enable: bool) {
        use crate::browser::Browser;

        let (failed_tx, failed_rx) = mpsc::channel();
        let (transport, server) = fixture(move |mut ws| {
            let discover = read_call(&mut ws);
            assert_eq!(discover["method"], "Target.setDiscoverTargets");
            send_json(&mut ws, json!({"id":discover["id"], "result":{}}));
            send_json(&mut ws, target_created("ephemeral"));
            while let Ok(message) = ws.read() {
                if !message.is_text() {
                    break;
                }
                let text = message.to_text().unwrap();
                let call: Value = serde_json::from_str(text).unwrap();
                match call["method"].as_str().unwrap() {
                    "Target.attachToTarget" => {
                        if call["params"]["targetId"] == "ephemeral" && !fail_page_enable {
                            send_json(
                                &mut ws,
                                json!({"id":call["id"], "error":{
                                    "code":-32000,"message":"No target with given id found"
                                }}),
                            );
                            send_json(
                                &mut ws,
                                json!({"method":"Target.targetDestroyed",
                                "params":{"targetId":"ephemeral"}}),
                            );
                            failed_tx.send(()).unwrap();
                        } else {
                            let session = if call["params"]["targetId"] == "ephemeral" {
                                "ephemeral-session"
                            } else {
                                "valid-session"
                            };
                            send_json(
                                &mut ws,
                                json!({"id":call["id"], "result":{"sessionId":session}}),
                            );
                        }
                    }
                    "Target.sendMessageToTarget" => {
                        let inner: Value =
                            serde_json::from_str(call["params"]["message"].as_str().unwrap())
                                .unwrap();
                        send_json(&mut ws, json!({"id":call["id"], "result":{}}));
                        let failed = call["params"]["sessionId"] == "ephemeral-session";
                        if failed {
                            assert_eq!(inner["method"], "Page.enable");
                        }
                        let response = if failed {
                            json!({"id":inner["id"],"error":{"code":-32000,"message":"Session closed"}})
                        } else {
                            json!({"id":inner["id"],"result":{}})
                        };
                        send_json(
                            &mut ws,
                            json!({"method":"Target.receivedMessageFromTarget", "params":{
                                "sessionId":call["params"]["sessionId"], "message":response.to_string()
                            }}),
                        );
                        if failed {
                            send_json(
                                &mut ws,
                                json!({"method":"Target.targetDestroyed",
                                "params":{"targetId":"ephemeral"}}),
                            );
                            failed_tx.send(()).unwrap();
                        }
                    }
                    "Browser.getVersion" => {
                        reply(&mut ws, &call["id"]);
                        send_json(&mut ws, target_created("valid"));
                    }
                    other => panic!("unexpected method {other}"),
                }
            }
        });
        transport
            .set_default_command_timeout(Duration::from_secs(2))
            .unwrap();
        let transport = Arc::new(transport);
        let browser =
            Browser::create_browser(None, transport.clone(), Duration::from_secs(10), false)
                .unwrap();
        let observed_failure = failed_rx.recv_timeout(Duration::from_secs(3));
        let health = browser.get_version();
        let until = Instant::now() + Duration::from_secs(3);
        let valid_tab_seen = loop {
            if browser
                .get_tabs()
                .lock()
                .unwrap()
                .iter()
                .any(|tab| tab.get_target_id() == "valid")
            {
                break true;
            }
            if health.is_err() || Instant::now() >= until {
                break false;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let targets: Vec<_> = browser
            .get_tabs()
            .lock()
            .unwrap()
            .iter()
            .map(|tab| tab.get_target_id().clone())
            .collect();
        drop(browser);
        transport.shutdown();
        server.join().unwrap();
        // The browser and tab event workers own transport clones. Shutdown disconnects
        // their event channels; wait for them to retire before asserting test outcomes.
        let until = Instant::now() + Duration::from_secs(2);
        while Arc::strong_count(&transport) != 1 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        empty(&transport);
        assert!(transport.incoming_thread.lock().unwrap().is_none());
        assert_eq!(Arc::strong_count(&transport), 1, "event worker leaked");
        observed_failure.expect("fixture did not exercise initialization failure");
        assert_eq!(
            health
                .expect("failed target killed browser health RPC")
                .product,
            "fake"
        );
        assert!(
            valid_tab_seen,
            "browser stopped accepting later valid targets"
        );
        assert_eq!(targets, vec!["valid"]);
    }

    #[test]
    fn failed_attach_preserves_browser_continuity() {
        failed_tab_initialization_preserves_browser(false);
    }

    #[test]
    fn failed_page_enable_preserves_browser_continuity() {
        failed_tab_initialization_preserves_browser(true);
    }

    #[test]
    fn missing_response_cleanup_and_late_reply_keep_socket_healthy() {
        let (release, wait) = mpsc::channel();
        let (transport, server) = fixture(move |mut ws| {
            let first = read_call(&mut ws);
            wait.recv().unwrap();
            reply(&mut ws, &first["id"]); // removed waiter: must not panic or retire socket
            let second = read_call(&mut ws);
            reply(&mut ws, &second["id"]);
            let _ = ws.read();
        });
        transport
            .set_default_command_timeout(Duration::from_millis(80))
            .unwrap();
        let start = Instant::now();
        assert_timeout(
            transport
                .call_method_on_browser(GetVersion(None))
                .unwrap_err(),
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        empty(&transport);
        release.send(()).unwrap();
        transport
            .set_default_command_timeout(Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            transport
                .call_method_on_browser(GetVersion(None))
                .unwrap()
                .product,
            "fake"
        );
        transport.shutdown();
        server.join().unwrap();
    }

    #[test]
    fn missing_target_ack_cleans_both_waiters() {
        let (seen, received) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let (transport, server) = fixture(move |mut ws| {
            let call = read_call(&mut ws);
            assert_eq!(call["method"], "Target.sendMessageToTarget");
            seen.send(()).unwrap();
            wait.recv().unwrap();
        });
        let transport = Arc::new(transport);
        let t = transport.clone();
        let caller = std::thread::spawn(move || {
            t.call_method_with_deadline(
                GetVersion(None),
                MethodDestination::Target("test".to_string().into()),
                Instant::now() + Duration::from_millis(100),
            )
        });
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_timeout(caller.join().unwrap().unwrap_err());
        empty(&transport);
        transport.shutdown();
        release.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn target_ack_and_inner_response_share_absolute_deadline() {
        let (transport, server) = fixture(move |mut ws| {
            let call = read_call(&mut ws);
            // Controlled pending outer ACK consumes most of the command's budget.
            let (_keep, timer) = mpsc::channel::<()>();
            assert!(timer.recv_timeout(Duration::from_millis(160)).is_err());
            ws.send(tungstenite::Message::text(
                json!({"id":call["id"],"result":{}}).to_string(),
            ))
            .unwrap();
            let _ = ws.read();
        });
        let deadline = Instant::now() + Duration::from_millis(250);
        assert_timeout(
            transport
                .call_method_with_deadline(
                    GetVersion(None),
                    MethodDestination::Target("test".to_string().into()),
                    deadline,
                )
                .unwrap_err(),
        );
        assert!(
            Instant::now() < deadline + Duration::from_millis(120),
            "nested ACK restarted deadline"
        );
        empty(&transport);
        transport.shutdown();
        server.join().unwrap();
    }

    #[test]
    fn cancellation_drains_waiters_and_joins_workers() {
        let (seen, received) = mpsc::channel();
        let (transport, server) = fixture(move |mut ws| {
            read_call(&mut ws);
            seen.send(()).unwrap();
            let _ = ws.read();
        });
        let transport = Arc::new(transport);
        let t = transport.clone();
        let (done, result) = mpsc::channel();
        let caller = std::thread::spawn(move || {
            let _ = done.send(t.call_method_on_browser(GetVersion(None)));
        });
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        let start = Instant::now();
        transport.cancellation_handle().cancel();
        let outcome = result.recv_timeout(Duration::from_secs(2));
        // Always unblock and retire even under a cancellation mutant.
        transport.shutdown();
        caller.join().unwrap();
        let error = outcome
            .expect("cancellation did not interrupt pending command")
            .unwrap_err();
        assert!(error.downcast_ref::<ConnectionClosed>().is_some());
        empty(&transport);
        transport.shutdown();
        assert!(transport.incoming_thread.lock().unwrap().is_none());
        assert!(start.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }

    #[test]
    fn stalled_peer_write_is_bounded() {
        let (release, wait) = mpsc::channel();
        let (transport, server) = fixture(move |_ws| {
            wait.recv().unwrap();
        });
        let connection = transport.web_socket_connection.clone();
        let deadline = Instant::now() + Duration::from_millis(150);
        let payload = "x".repeat(32 * 1024 * 1024);
        assert_timeout(connection.send_message(&payload, deadline).unwrap_err());
        assert!(Instant::now() < deadline + Duration::from_secs(2));
        transport.shutdown();
        empty(&transport);
        release.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn expired_request_guard_prevents_hidden_default_call_send() {
        let (transport, server) = fixture(move |mut ws| {
            assert!(
                ws.read().is_err(),
                "expired request unexpectedly dispatched"
            );
        });
        let _guard = CommandDeadlineGuard::new(Instant::now());
        let error = transport
            .call_method_on_browser(GetVersion(None))
            .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("CDP command deadline exceeded:")
        );
        assert!(
            !error
                .downcast_ref::<CommandTimeout>()
                .unwrap()
                .effect_unknown
        );
        empty(&transport);
        transport.shutdown();
        server.join().unwrap();
    }
}
