//! Legacy MCP HTTP+SSE client transport (2024-11-05 protocol revision).
//!
//! The pinned rmcp SDK only ships the modern Streamable HTTP client transport,
//! so this module implements the deprecated-but-still-deployed HTTP+SSE flow on
//! top of rmcp's `SinkStreamTransport` primitive:
//!
//! 1. `GET <url>` with `Accept: text/event-stream` opens the event stream.
//! 2. The server's first `event: endpoint` frame carries the POST endpoint
//!    (usually relative, e.g. `/messages?sessionId=...`).
//! 3. Client JSON-RPC messages are POSTed to that endpoint; server JSON-RPC
//!    messages arrive as `event: message` frames on the long-lived stream.
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::channel::mpsc;
use futures::{Sink, StreamExt};
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::sink_stream::SinkStreamTransport;
use sse_stream::SseStream;
use tokio::sync::Mutex as AMutex;
use tokio::time::{timeout, Duration};

use super::mcp_auth::{AuthType, MCPAuthSettings};
use super::session_mcp::add_log_entry;

const POSTER_CHANNEL_CAPACITY: usize = 64;
const ENDPOINT_WAIT_SECS: u64 = 30;
const POST_TIMEOUT_SECS: u64 = 30;
const PKCE_CONNECT_TIMEOUT_SECS: u64 = 15;
const SSE_HEADER_TIMEOUT_SECS: u64 = 30;

pub type LegacySseMessageStream = futures::stream::BoxStream<'static, ServerJsonRpcMessage>;
pub type LegacySseTransport = SinkStreamTransport<LegacySseSink, LegacySseMessageStream>;

/// Sink half of the legacy SSE transport: forwards client messages to a
/// background poster task that POSTs them to the discovered message endpoint.
/// Dropping the sink closes the channel, which ends the poster task; the
/// JoinHandle is aborted on drop as a belt-and-braces cleanup.
pub struct LegacySseSink {
    tx: mpsc::Sender<ClientJsonRpcMessage>,
    poster: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for LegacySseSink {
    fn drop(&mut self) {
        if let Some(handle) = self.poster.take() {
            handle.abort();
        }
    }
}

impl Sink<ClientJsonRpcMessage> for LegacySseSink {
    type Error = mpsc::SendError;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.tx).poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: ClientJsonRpcMessage) -> Result<(), Self::Error> {
        Pin::new(&mut self.tx).start_send(item)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.tx).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.tx).poll_close(cx)
    }
}

/// Resolves the `endpoint` event payload against the SSE base URL.
/// Cross-origin endpoints are rejected: the poster reuses the authenticated
/// HTTP client, so honoring them would let a malicious server redirect
/// bearer/OAuth-authenticated JSON-RPC traffic to an attacker host.
pub fn resolve_message_endpoint(base_url: &str, endpoint_data: &str) -> Result<String, String> {
    let data = endpoint_data.trim();
    if data.is_empty() {
        return Err("endpoint event carried no data".to_string());
    }
    let base = url::Url::parse(base_url).map_err(|e| format!("invalid base url: {}", e))?;
    let resolved = base
        .join(data)
        .map_err(|e| format!("cannot resolve endpoint '{}': {}", data, e))?;
    let same_origin = resolved.scheme() == base.scheme()
        && resolved.host_str() == base.host_str()
        && resolved.port_or_known_default() == base.port_or_known_default();
    if !same_origin {
        return Err(format!(
            "endpoint '{}' is cross-origin relative to '{}'; refusing to send authenticated traffic there",
            resolved, base
        ));
    }
    Ok(resolved.to_string())
}

/// Opens the legacy SSE connection: performs the GET, waits for the `endpoint`
/// event, and returns a ready-to-serve transport.
pub async fn connect_legacy_sse(
    client: reqwest::Client,
    url: &str,
    logs: Arc<AMutex<Vec<String>>>,
    debug_name: &str,
) -> Result<LegacySseTransport, String> {
    let response = timeout(
        Duration::from_secs(SSE_HEADER_TIMEOUT_SECS),
        client
            .get(url)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send(),
    )
    .await
    .map_err(|_| {
        format!(
            "SSE GET {} timed out after {}s waiting for response headers",
            url, SSE_HEADER_TIMEOUT_SECS
        )
    })?
    .map_err(|e| format!("SSE GET {} failed: {}", url, e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("SSE GET {} returned HTTP {}", url, status));
    }

    let mut events = SseStream::from_byte_stream(response.bytes_stream()).boxed();

    let endpoint_data = timeout(Duration::from_secs(ENDPOINT_WAIT_SECS), async {
        while let Some(event) = events.next().await {
            match event {
                Ok(sse) => {
                    if sse.event.as_deref() == Some("endpoint") {
                        return match sse.data {
                            Some(data) => Ok(data),
                            None => Err("SSE endpoint event carried no data".to_string()),
                        };
                    }
                    // Ignore keepalive comments and any pre-endpoint frames.
                }
                Err(e) => {
                    return Err(format!("SSE stream error before endpoint event: {}", e));
                }
            }
        }
        Err("SSE stream ended before the endpoint event".to_string())
    })
    .await
    .map_err(|_| {
        format!(
            "server did not send the SSE endpoint event within {}s",
            ENDPOINT_WAIT_SECS
        )
    })??;

    let endpoint = resolve_message_endpoint(url, &endpoint_data)?;
    tracing::info!("legacy SSE endpoint for {}: {}", debug_name, endpoint);

    let (tx, mut rx) = mpsc::channel::<ClientJsonRpcMessage>(POSTER_CHANNEL_CAPACITY);
    let poster_logs = logs.clone();
    let poster_endpoint = endpoint.clone();
    let poster_client = client.clone();
    // The poster ends deterministically when the sink (tx) is dropped together
    // with the transport; LegacySseSink also aborts it on drop.
    let poster = tokio::spawn(async move {
        while let Some(message) = rx.next().await {
            let body = match serde_json::to_vec(&message) {
                Ok(body) => body,
                Err(e) => {
                    add_log_entry(
                        poster_logs.clone(),
                        format!("legacy SSE: failed to serialize outgoing message: {}", e),
                    )
                    .await;
                    continue;
                }
            };
            match poster_client
                .post(&poster_endpoint)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .timeout(Duration::from_secs(POST_TIMEOUT_SECS))
                .body(body)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {}
                Ok(resp) => {
                    add_log_entry(
                        poster_logs.clone(),
                        format!(
                            "legacy SSE: POST {} returned HTTP {}",
                            poster_endpoint,
                            resp.status()
                        ),
                    )
                    .await;
                }
                Err(e) => {
                    add_log_entry(
                        poster_logs.clone(),
                        format!("legacy SSE: POST {} failed: {}", poster_endpoint, e),
                    )
                    .await;
                }
            }
        }
    });

    let stream_debug_name = debug_name.to_string();
    let messages: LegacySseMessageStream = events
        .filter_map(move |event| {
            let debug_name = stream_debug_name.clone();
            async move {
                match event {
                    Ok(sse) => {
                        let is_message = sse.event.as_deref().map_or(true, |e| e == "message");
                        if !is_message {
                            return None;
                        }
                        let data = sse.data?;
                        match serde_json::from_str::<ServerJsonRpcMessage>(&data) {
                            Ok(message) => Some(message),
                            Err(e) => {
                                tracing::warn!(
                                    "legacy SSE ({}): cannot parse server message: {}",
                                    debug_name,
                                    e
                                );
                                None
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("legacy SSE ({}): stream error: {}", debug_name, e);
                        None
                    }
                }
            }
        })
        .boxed();

    Ok(SinkStreamTransport::new(
        LegacySseSink {
            tx,
            poster: Some(poster),
        },
        messages,
    ))
}

/// Builds the reqwest client used for the legacy SSE connection. OAuth2 PKCE
/// tokens are injected as a static bearer header for the connection lifetime;
/// the health monitor reconnects (rebuilding the client with a fresh token)
/// when the connection drops after token expiry.
pub async fn build_legacy_sse_client(
    url: &str,
    headers: &HashMap<String, String>,
    auth: &MCPAuthSettings,
    config_path: &str,
    logs: Arc<AMutex<Vec<String>>>,
    debug_name: &str,
    session: Arc<AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
) -> Option<reqwest::Client> {
    if auth.auth_type != AuthType::Oauth2Pkce {
        return super::integr_mcp_common::build_reqwest_client_for_mcp(
            url,
            headers,
            auth,
            "legacy SSE",
            logs,
            debug_name,
        )
        .await;
    }

    let auth_client = super::integr_mcp_common::build_auth_client_for_mcp(
        url,
        headers,
        config_path,
        "legacy SSE",
        logs.clone(),
        debug_name,
        session,
    )
    .await?;
    let token = match auth_client
        .auth_manager
        .lock()
        .await
        .get_access_token()
        .await
    {
        Ok(token) => token,
        Err(e) => {
            let msg = format!("legacy SSE: cannot obtain OAuth access token: {}", e);
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            return None;
        }
    };

    let mut header_map = reqwest::header::HeaderMap::new();
    for (k, v) in headers {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            header_map.insert(name, value);
        }
    }
    match reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)) {
        Ok(mut value) => {
            value.set_sensitive(true);
            header_map.insert(reqwest::header::AUTHORIZATION, value);
        }
        Err(e) => {
            let msg = format!("legacy SSE: invalid bearer token: {}", e);
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            return None;
        }
    }

    match reqwest::Client::builder()
        .default_headers(header_map)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(PKCE_CONNECT_TIMEOUT_SECS))
        .build()
    {
        Ok(client) => Some(client),
        Err(e) => {
            let msg = format!("legacy SSE: failed to build HTTP client: {}", e);
            tracing::error!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::transport::Transport;

    #[test]
    fn test_resolve_message_endpoint_relative() {
        assert_eq!(
            resolve_message_endpoint("http://127.0.0.1:3000/sse", "/messages?sessionId=abc")
                .unwrap(),
            "http://127.0.0.1:3000/messages?sessionId=abc"
        );
        assert_eq!(
            resolve_message_endpoint("https://mcp.example.com/api/sse", "messages").unwrap(),
            "https://mcp.example.com/api/messages"
        );
    }

    #[test]
    fn test_resolve_message_endpoint_absolute() {
        assert_eq!(
            resolve_message_endpoint(
                "http://127.0.0.1:3000/sse",
                "http://127.0.0.1:3000/rpc?sid=1"
            )
            .unwrap(),
            "http://127.0.0.1:3000/rpc?sid=1"
        );
    }

    #[test]
    fn test_resolve_message_endpoint_rejects_bad_input() {
        assert!(resolve_message_endpoint("http://127.0.0.1:3000/sse", "").is_err());
        assert!(resolve_message_endpoint("not a url", "/messages").is_err());
    }

    #[test]
    fn test_resolve_message_endpoint_rejects_cross_origin() {
        // A malicious server must not be able to redirect authenticated
        // JSON-RPC traffic to another origin via the endpoint event.
        assert!(resolve_message_endpoint(
            "http://127.0.0.1:3000/sse",
            "https://attacker.example/collect"
        )
        .is_err());
        assert!(resolve_message_endpoint(
            "http://127.0.0.1:3000/sse",
            "http://127.0.0.1:4000/messages"
        )
        .is_err());
        assert!(resolve_message_endpoint(
            "https://mcp.example.com/sse",
            "http://mcp.example.com/messages"
        )
        .is_err());
    }

    async fn spawn_legacy_sse_server() -> (
        String,
        Arc<AMutex<Vec<String>>>,
        tokio::task::JoinHandle<()>,
    ) {
        use axum::routing::{get, post};

        let posted: Arc<AMutex<Vec<String>>> = Arc::new(AMutex::new(Vec::new()));
        let posted_for_route = posted.clone();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{}", addr);

        let sse_route = get(|| async {
            let frames: Vec<Result<axum::body::Bytes, std::io::Error>> = vec![
                Ok(axum::body::Bytes::from_static(
                    b"event: endpoint\ndata: /messages?sessionId=test-1\n\n",
                )),
                Ok(axum::body::Bytes::from_static(
                    b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n\n",
                )),
            ];
            let body_stream = futures::stream::iter(frames).chain(futures::stream::pending());
            (
                [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                axum::body::StreamBody::new(body_stream),
            )
        });
        let messages_route = post(move |body: String| {
            let posted = posted_for_route.clone();
            async move {
                posted.lock().await.push(body);
                axum::http::StatusCode::ACCEPTED
            }
        });

        let app = axum::Router::new()
            .route("/sse", sse_route)
            .route("/messages", messages_route);

        listener.set_nonblocking(true).unwrap();
        let handle = tokio::spawn(async move {
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(app.into_make_service())
                .await
                .unwrap();
        });

        (base, posted, handle)
    }

    #[tokio::test]
    async fn test_legacy_sse_roundtrip() {
        let (base, posted, server_handle) = spawn_legacy_sse_server().await;
        let logs = Arc::new(AMutex::new(Vec::new()));

        let mut transport = connect_legacy_sse(
            reqwest::Client::new(),
            &format!("{}/sse", base),
            logs.clone(),
            "test_server",
        )
        .await
        .expect("legacy SSE connect must succeed");

        let received = timeout(
            Duration::from_secs(5),
            <LegacySseTransport as rmcp::transport::Transport<rmcp::RoleClient>>::receive(
                &mut transport,
            ),
        )
        .await
        .expect("must receive a server message in time")
        .expect("stream must yield a message");
        let received_json = serde_json::to_string(&received).unwrap();
        assert!(
            received_json.contains("ping"),
            "expected server ping request, got: {}",
            received_json
        );

        let outgoing: ClientJsonRpcMessage = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .unwrap();
        <LegacySseTransport as rmcp::transport::Transport<rmcp::RoleClient>>::send(
            &mut transport,
            outgoing,
        )
        .await
        .expect("send must succeed");

        let mut delivered = None;
        for _ in 0..50 {
            {
                let bodies = posted.lock().await;
                if let Some(first) = bodies.first() {
                    delivered = Some(first.clone());
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let delivered = delivered.expect("client message must be POSTed to the endpoint");
        assert!(
            delivered.contains("notifications/initialized"),
            "unexpected POST body: {}",
            delivered
        );

        drop(transport);
        server_handle.abort();
    }

    #[tokio::test]
    async fn test_legacy_sse_connect_fails_without_endpoint_event() {
        use axum::routing::get;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let sse_route = get(|| async {
            let frames: Vec<Result<axum::body::Bytes, std::io::Error>> =
                vec![Ok(axum::body::Bytes::from_static(b": keepalive\n\n"))];
            let body_stream = futures::stream::iter(frames);
            (
                [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                axum::body::StreamBody::new(body_stream),
            )
        });
        let app = axum::Router::new().route("/sse", sse_route);
        listener.set_nonblocking(true).unwrap();
        let handle = tokio::spawn(async move {
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(app.into_make_service())
                .await
                .unwrap();
        });

        let logs = Arc::new(AMutex::new(Vec::new()));
        let result = connect_legacy_sse(
            reqwest::Client::new(),
            &format!("http://{}/sse", addr),
            logs,
            "test_server",
        )
        .await;
        assert!(
            result.is_err(),
            "connect must fail when stream ends without endpoint event"
        );
        handle.abort();
    }

    #[tokio::test]
    async fn test_legacy_sse_connect_does_not_follow_redirects() {
        use axum::routing::get;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let sse_route = get(|| async {
            (
                axum::http::StatusCode::TEMPORARY_REDIRECT,
                [(
                    axum::http::header::LOCATION,
                    "http://attacker.example/collect",
                )],
            )
        });
        let app = axum::Router::new().route("/sse", sse_route);
        listener.set_nonblocking(true).unwrap();
        let handle = tokio::spawn(async move {
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(app.into_make_service())
                .await
                .unwrap();
        });

        // Client hardened exactly like the production MCP clients: redirects
        // disabled so a hostile server cannot bounce authenticated traffic to
        // another origin.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let logs = Arc::new(AMutex::new(Vec::new()));
        let result =
            connect_legacy_sse(client, &format!("http://{}/sse", addr), logs, "redirect_test")
                .await;
        assert!(
            result.is_err(),
            "connect must refuse to follow a cross-origin redirect"
        );
        handle.abort();
    }
}
