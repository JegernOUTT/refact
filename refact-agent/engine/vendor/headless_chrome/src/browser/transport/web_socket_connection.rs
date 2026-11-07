use std::net::TcpStream;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use log::debug;
use tungstenite::http::Response;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::stream::MaybeTlsStream;
use url::Url;

use crate::types::{Message, parse_raw_message};

const READ_TIMEOUT_DURATION: std::time::Duration = std::time::Duration::from_millis(10);

#[cfg(feature = "rustls-tls-webpki-roots")]
static RUSTLS_INIT: std::sync::Once = std::sync::Once::new();

#[cfg(feature = "rustls-tls-webpki-roots")]
fn init_rustls_provider() {
    RUSTLS_INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[cfg(feature = "rustls-tls-webpki-roots")]
fn add_root_certificates(
    roots: &mut rustls::RootCertStore,
    root_cert: Option<&[u8]>,
) -> Result<()> {
    use std::io::Cursor;

    let Some(cert_bytes) = root_cert else {
        return Ok(());
    };

    if cert_bytes.starts_with(b"-----BEGIN CERTIFICATE-----") {
        let mut reader = Cursor::new(cert_bytes);

        for cert in rustls_pemfile::certs(&mut reader) {
            roots.add(cert?)?;
        }
    } else {
        roots.add(rustls::pki_types::CertificateDer::from(cert_bytes.to_vec()))?;
    }

    Ok(())
}

fn set_read_timeout(stream: &mut MaybeTlsStream<TcpStream>) -> Result<()> {
    let tcp_stream = match stream {
        MaybeTlsStream::Plain(s) => s,

        #[cfg(any(
            feature = "rustls-tls-native-roots",
            feature = "rustls-tls-webpki-roots"
        ))]
        MaybeTlsStream::Rustls(s) => &mut s.sock,

        #[cfg(feature = "native-tls")]
        MaybeTlsStream::NativeTls(s) => s.get_mut(),

        #[allow(unreachable_patterns)]
        _ => {
            return Err(anyhow::anyhow!("unsupported websocket stream type"));
        }
    };

    tcp_stream.set_read_timeout(Some(READ_TIMEOUT_DURATION))?;
    tcp_stream.set_write_timeout(Some(Duration::from_millis(100)))?;

    Ok(())
}

const QUEUED: u8 = 0;
const WRITING: u8 = 1;
const COMPLETED: u8 = 2;
const CANCELLED: u8 = 3;

// A single socket owner avoids read-mutex starvation. Both the queue and writes are bounded.
struct Outgoing {
    text: String,
    deadline: Instant,
    done: mpsc::Sender<Result<()>>,
    state: Arc<AtomicU8>,
}

pub struct WebSocketConnection {
    outgoing: mpsc::SyncSender<Outgoing>,
    stopped: Arc<AtomicBool>,
    socket: TcpStream,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl std::fmt::Debug for WebSocketConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("WebSocketConnection")
            .finish_non_exhaustive()
    }
}

fn tcp(stream: &mut MaybeTlsStream<TcpStream>) -> Result<&mut TcpStream> {
    match stream {
        MaybeTlsStream::Plain(s) => Ok(s),
        #[cfg(any(
            feature = "rustls-tls-native-roots",
            feature = "rustls-tls-webpki-roots"
        ))]
        MaybeTlsStream::Rustls(s) => Ok(&mut s.sock),
        #[cfg(feature = "native-tls")]
        MaybeTlsStream::NativeTls(s) => Ok(s.get_mut()),
        #[allow(unreachable_patterns)]
        _ => Err(anyhow::anyhow!("unsupported websocket stream")),
    }
}

impl WebSocketConnection {
    pub fn new(
        ws_url: &Url,
        _process_id: Option<u32>,
        messages_tx: mpsc::Sender<Message>,
        root_cert: Option<Vec<u8>>,
    ) -> Result<Self> {
        let (mut connection, _) =
            Self::websocket_connection_with_root_cert(ws_url, root_cert.as_deref())?;
        let socket = tcp(connection.get_mut())?.try_clone()?;
        let (outgoing, rx) = mpsc::sync_channel::<Outgoing>(64);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopped);
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                // Limit work per iteration so incoming replies cannot starve behind writes.
                for _ in 0..16 {
                    let Ok(request) = rx.try_recv() else { break };
                    // A caller can cancel an enqueued request without touching a healthy socket.
                    if request
                        .state
                        .compare_exchange(QUEUED, WRITING, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                    {
                        continue;
                    }
                    let remaining = request.deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        request.state.store(CANCELLED, Ordering::Release);
                        let _ = request.done.send(Err(super::CommandTimeout {
                            method: "websocket send",
                            effect_unknown: false,
                        }
                        .into()));
                        continue;
                    }
                    let result = tcp(connection.get_mut())
                        .and_then(|s| {
                            s.set_write_timeout(Some(remaining.min(Duration::from_millis(100))))?;
                            Ok(())
                        })
                        .and_then(|()| {
                            connection
                                .send(tungstenite::Message::text(request.text))
                                .map_err(Into::into)
                        });
                    let failed = result.is_err();
                    let _ = request.done.send(result);
                    request.state.store(COMPLETED, Ordering::Release);
                    // A partial write has unknown effect; retire only this connection, never replay.
                    if failed {
                        stop.store(true, Ordering::Release);
                        break;
                    }
                }
                if stop.load(Ordering::Acquire) {
                    break;
                }
                match connection.read() {
                    Ok(tungstenite::Message::Text(text)) => {
                        if let Ok(message) = parse_raw_message(&text) {
                            if messages_tx.send(message).is_err() {
                                break;
                            }
                        }
                    }
                    Ok(tungstenite::Message::Close(_)) => break,
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(e))
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => break,
                }
            }
            stop.store(true, Ordering::Release);
            let _ = tcp(connection.get_mut()).map(|s| s.shutdown(std::net::Shutdown::Both));
            let _ = messages_tx.send(Message::ConnectionShutdown);
        });
        Ok(Self {
            outgoing,
            stopped,
            socket,
            thread: Mutex::new(Some(thread)),
        })
    }

    /// Interrupt I/O immediately without waiting for the socket owner.
    pub fn interrupt(&self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
    }

    pub fn shutdown(&self) {
        self.interrupt();
        if let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }

    pub fn send_message(&self, message_text: &str, deadline: Instant) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(AtomicU8::new(QUEUED));
        let mut request = Outgoing {
            text: message_text.to_owned(),
            deadline,
            done: tx,
            state: Arc::clone(&state),
        };
        loop {
            if self.stopped.load(Ordering::Acquire) {
                return Err(super::ConnectionClosed {}.into());
            }
            if Instant::now() >= deadline {
                return Err(super::CommandTimeout {
                    method: "websocket send",
                    effect_unknown: false,
                }
                .into());
            }
            match self.outgoing.try_send(request) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(r)) => {
                    request = r;
                    std::thread::park_timeout(Duration::from_millis(1));
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err(super::ConnectionClosed {}.into());
                }
            }
        }
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                if e.downcast_ref::<super::CommandTimeout>().is_some() {
                    Err(e)
                } else {
                    Err(super::CommandTimeout {
                        method: "websocket send",
                        effect_unknown: true,
                    }
                    .into())
                }
            }
            Err(_) => {
                let queued = state
                    .compare_exchange(QUEUED, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok();
                // Only an actually writing command can have partially reached Chrome.
                // Completed sends need no interruption even if completion raced our receive.
                if !queued && state.load(Ordering::Acquire) == WRITING {
                    self.interrupt();
                }
                Err(super::CommandTimeout {
                    method: "websocket send",
                    effect_unknown: !queued && state.load(Ordering::Acquire) != CANCELLED,
                }
                .into())
            }
        }
    }

    pub fn websocket_connection_with_root_cert(
        ws_url: &Url,
        root_cert: Option<&[u8]>,
    ) -> Result<(
        tungstenite::WebSocket<MaybeTlsStream<TcpStream>>,
        Response<Option<Vec<u8>>>,
    )> {
        let config = Some(
            WebSocketConfig::default()
                .accept_unmasked_frames(true)
                .max_message_size(None)
                .max_frame_size(None),
        );

        // Never perform an unbounded OS DNS lookup. Localhost is resolved without DNS;
        // other hostname users must supply an already resolved numeric endpoint.
        let host = ws_url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("missing websocket host"))?;
        let ip = if host.eq_ignore_ascii_case("localhost") {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        } else {
            host.trim_matches(['[', ']']).parse::<std::net::IpAddr>()
                .map_err(|_| anyhow::anyhow!("bounded CDP connection requires a numeric IP endpoint or localhost; resolve other hostnames before connecting"))?
        };
        anyhow::ensure!(
            matches!(ws_url.scheme(), "ws" | "wss"),
            "CDP endpoint must use ws or wss"
        );
        let default_deadline = Instant::now() + Duration::from_secs(30);
        let deadline = super::COMMAND_DEADLINE.with(|d| {
            d.get()
                .map_or(default_deadline, |r| r.min(default_deadline))
        });
        let timeout_error = || super::CommandTimeout {
            method: "websocket connect",
            effect_unknown: false,
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(timeout_error().into());
        }
        let address = std::net::SocketAddr::new(
            ip,
            ws_url
                .port_or_known_default()
                .ok_or_else(|| anyhow::anyhow!("missing websocket port"))?,
        );
        let tcp = TcpStream::connect_timeout(&address, remaining).map_err(|error| {
            if Instant::now() >= deadline || error.kind() == std::io::ErrorKind::TimedOut {
                anyhow::Error::from(timeout_error())
            } else {
                error.into()
            }
        })?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(timeout_error().into());
        }
        tcp.set_read_timeout(Some(remaining))?;
        tcp.set_write_timeout(Some(remaining))?;
        // Socket timeouts alone restart after each partial TLS/HTTP read. A joined,
        // cancellable watchdog enforces the same absolute deadline even for slow-drip peers.
        let interrupt = tcp.try_clone()?;
        let (stop, stopped) = mpsc::channel();
        let watchdog = std::thread::spawn(move || {
            if stopped.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                == Err(mpsc::RecvTimeoutError::Timeout)
            {
                let _ = interrupt.shutdown(std::net::Shutdown::Both);
            }
        });
        let result: Result<_> = (|| {
            use tungstenite::client::IntoClientRequest;
            #[cfg(any(
                feature = "native-tls",
                feature = "rustls-tls-native-roots",
                feature = "rustls-tls-webpki-roots"
            ))]
            #[allow(unused_mut)]
            let mut connector = None;
            if root_cert.is_some() {
                #[cfg(feature = "rustls-tls-webpki-roots")]
                {
                    init_rustls_provider();
                    let mut roots = rustls::RootCertStore::empty();
                    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                    add_root_certificates(&mut roots, root_cert)?;
                    let tls_config = rustls::ClientConfig::builder()
                        .with_root_certificates(roots)
                        .with_no_client_auth();
                    connector = Some(tungstenite::Connector::Rustls(Arc::new(tls_config)));
                }
                #[cfg(not(feature = "rustls-tls-webpki-roots"))]
                return Err(anyhow::anyhow!(
                    "root_cert requires feature rustls-tls-webpki-roots"
                ));
            }
            let request = ws_url.as_str().into_client_request()?;
            #[cfg(any(
                feature = "native-tls",
                feature = "rustls-tls-native-roots",
                feature = "rustls-tls-webpki-roots"
            ))]
            let mut client = tungstenite::client_tls_with_config(request, tcp, config, connector)?;
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls-tls-native-roots",
                feature = "rustls-tls-webpki-roots"
            )))]
            let mut client = {
                anyhow::ensure!(
                    ws_url.scheme() == "ws",
                    "wss requires an enabled TLS feature"
                );
                tungstenite::client::client_with_config(
                    request,
                    MaybeTlsStream::Plain(tcp),
                    config,
                )?
            };
            set_read_timeout(client.0.get_mut())?;
            Ok(client)
        })();
        let _ = stop.send(());
        let _ = watchdog.join();
        if Instant::now() >= deadline {
            return Err(timeout_error().into());
        }
        debug!("WebSocket handshake finished");
        result
    }
}

impl Drop for WebSocketConnection {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    #[test]
    fn enqueued_timeout_cancels_only_request_not_healthy_socket() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        let (outgoing, receiver) = mpsc::sync_channel(1);
        let connection = WebSocketConnection {
            outgoing,
            stopped: Arc::new(AtomicBool::new(false)),
            socket,
            thread: Mutex::new(None),
        };
        let error = connection
            .send_message(
                "enqueued but owner deliberately paused",
                Instant::now() + Duration::from_millis(30),
            )
            .unwrap_err();
        assert!(
            !error
                .downcast_ref::<super::super::CommandTimeout>()
                .unwrap()
                .effect_unknown
        );
        assert!(
            !connection.stopped.load(Ordering::Acquire),
            "queued deadline retired healthy connection"
        );
        let queued = receiver
            .try_recv()
            .expect("request was never actually enqueued");
        assert_eq!(queued.state.load(Ordering::Acquire), CANCELLED);
        assert!(
            queued
                .state
                .compare_exchange(QUEUED, WRITING, Ordering::AcqRel, Ordering::Acquire)
                .is_err(),
            "cancelled request can still dispatch"
        );
        use std::io::{Read, Write};
        (&connection.socket).write_all(b"healthy").unwrap();
        let mut bytes = [0; 7];
        peer.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"healthy");
    }

    #[test]
    fn stalled_upgrade_obeys_absolute_request_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!(
            "ws://localhost:{}/test",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let (ready, seen) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            use std::io::Read;
            let mut byte = [0; 1];
            socket.read_exact(&mut byte).unwrap();
            ready.send(()).unwrap();
            wait.recv().unwrap(); // No HTTP upgrade response, regardless of idle timeout.
        });
        let deadline = Instant::now() + Duration::from_millis(100);
        let _guard = super::super::CommandDeadlineGuard::new(deadline);
        let error =
            WebSocketConnection::websocket_connection_with_root_cert(&url, None).unwrap_err();
        seen.recv_timeout(Duration::from_secs(2)).unwrap();
        release.send(()).unwrap();
        peer.join().unwrap();
        assert!(
            error
                .downcast_ref::<super::super::CommandTimeout>()
                .is_some(),
            "wrong handshake error: {error:#}"
        );
        assert!(Instant::now() < deadline + Duration::from_secs(2));
    }

    #[test]
    fn full_socket_queue_respects_deadline_without_dispatch() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        let (outgoing, _receiver) = mpsc::sync_channel(1);
        let (done, _result) = mpsc::channel();
        outgoing
            .send(Outgoing {
                text: "occupied".into(),
                deadline: Instant::now() + Duration::from_secs(5),
                done,
                state: Arc::new(AtomicU8::new(QUEUED)),
            })
            .unwrap();
        let connection = WebSocketConnection {
            outgoing,
            stopped: Arc::new(AtomicBool::new(false)),
            socket,
            thread: Mutex::new(None),
        };
        let deadline = Instant::now() + Duration::from_millis(50);
        let error = connection
            .send_message("must not enqueue", deadline)
            .unwrap_err();
        assert!(
            !error
                .downcast_ref::<super::super::CommandTimeout>()
                .unwrap()
                .effect_unknown
        );
        assert!(Instant::now() < deadline + Duration::from_secs(1));
    }
}
