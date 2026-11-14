use super::*;
use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Instant};
use tokio_rustls::TlsAcceptor;

const DEADLINE: Duration = Duration::from_secs(4);
const URL: &str = "https://api.github.com/test";

struct Server {
    addr: SocketAddr,
    requests: Arc<AtomicUsize>,
    request: Option<oneshot::Receiver<String>>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl Server {
    async fn start(parts: Vec<Vec<u8>>, close: bool) -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(
                    include_bytes!("../tests/fixtures/public-release-http/test-server.der")
                        .to_vec(),
                )],
                rustls::pki_types::PrivatePkcs8KeyDer::from(
                    include_bytes!("../tests/fixtures/public-release-http/test-server-key.der")
                        .to_vec(),
                )
                .into(),
            )
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let (request_tx, request) = oneshot::channel();
        let (stop, mut stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let serve = async move {
                let (socket, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                let mut tls = acceptor.accept(socket).await.unwrap();
                let mut raw = Vec::new();
                while !raw.ends_with(b"\r\n\r\n") {
                    assert!(raw.len() < 16 * 1024, "request header bound");
                    raw.push(tls.read_u8().await.unwrap());
                }
                request_tx.send(String::from_utf8(raw).unwrap()).unwrap();
                for part in parts {
                    tls.write_all(&part).await.unwrap();
                    tls.flush().await.unwrap();
                    tokio::task::yield_now().await;
                }
                if close {
                    tls.shutdown().await.unwrap();
                }
                // Retain the first stream and count any attempted automatic follow-up connection.
                tokio::select! {
                    _ = &mut stopped => {},
                    extra = listener.accept() => {
                        extra.unwrap();
                        count.fetch_add(1, Ordering::SeqCst);
                        panic!("unexpected follow-up connection");
                    }
                }
                drop(tls);
            };
            timeout(DEADLINE, serve).await.expect("fixture deadline");
        });
        Self {
            addr,
            requests,
            request: Some(request),
            stop: Some(stop),
            task: Some(task),
        }
    }

    fn client(&self) -> ReqwestPublicTransport {
        ReqwestPublicTransport::loopback_test(self.addr, Duration::from_secs(2))
    }

    async fn request(&mut self) -> String {
        timeout(DEADLINE, self.request.take().unwrap())
            .await
            .unwrap()
            .unwrap()
    }

    async fn finish(mut self) {
        self.stop.take().unwrap().send(()).unwrap();
        timeout(DEADLINE, self.task.take().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(self.requests.load(Ordering::SeqCst), 1);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

fn bytes(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

fn assert_error(result: Result<PublicResponse>, expected: &str) {
    assert_eq!(result.unwrap_err(), reject(expected));
}

#[tokio::test]
async fn real_tls_body_and_public_headers() {
    for asset in [false, true] {
        let mut server = Server::start(
            vec![bytes("HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")],
            true,
        )
        .await;
        let response = timeout(DEADLINE, server.client().get(URL, asset, 5))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello");
        assert!(response.location.is_none());
        let raw = server.request().await;
        let headers = raw.to_ascii_lowercase();
        assert!(raw.starts_with("GET /test HTTP/1.1\r\n"));
        assert!(headers.contains("\r\nhost: api.github.com\r\n"));
        assert!(headers.contains("\r\nuser-agent: refact-public-runtime-resolver\r\n"));
        assert!(headers.contains(if asset {
            "\r\naccept: application/octet-stream\r\n"
        } else {
            "\r\naccept: application/vnd.github+json\r\n"
        }));
        for line in headers.lines() {
            assert!(!line.starts_with("authorization:"));
            assert!(!line.starts_with("proxy-authorization:"));
            assert!(!line.starts_with("cookie:"));
        }
        server.finish().await;
    }
}

#[tokio::test]
async fn real_tls_content_length_rejected_without_waiting_for_body() {
    let mut server = Server::start(
        vec![bytes("HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n")],
        false,
    )
    .await;
    assert_error(
        timeout(DEADLINE, server.client().get(URL, false, 8))
            .await
            .unwrap(),
        "HTTP body limit exceeded",
    );
    server.request().await;
    server.finish().await;
}

#[tokio::test]
async fn real_tls_raw_inventory_header_is_limited_to_exact_source_url() {
    for (path, expected) in [
        (
            format!(
                "contents/tools/dev/ci_test_inventory.json?ref={}",
                "a".repeat(40)
            ),
            "application/vnd.github.raw+json",
        ),
        (
            "contents/tools/dev/ci_test_inventory.json?ref=main".to_string(),
            "application/vnd.github+json",
        ),
    ] {
        let mut server = Server::start(
            vec![bytes("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")],
            true,
        )
        .await;
        let response = server
            .client()
            .get(&format!("{BASE}/{path}"), false, 2)
            .await
            .unwrap();
        assert_eq!(response.body, b"{}");
        let request = server.request().await.to_ascii_lowercase();
        assert!(request.contains(&format!("\r\naccept: {expected}\r\n")));
        server.finish().await;
    }
}

#[tokio::test]
async fn real_tls_chunked_incremental_overflow() {
    let mut server = Server::start(
        vec![
            bytes("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n1234\r\n"),
            bytes("4\r\n5678\r\n"),
        ],
        false,
    )
    .await;
    assert_error(
        timeout(DEADLINE, server.client().get(URL, true, 6))
            .await
            .unwrap(),
        "HTTP body limit exceeded",
    );
    server.request().await;
    server.finish().await;
}

#[tokio::test]
async fn real_tls_empty_api_and_truncated_body_errors() {
    let mut empty = Server::start(
        vec![bytes("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")],
        true,
    )
    .await;
    assert!(
        timeout(DEADLINE, api::<serde_json::Value>(&empty.client(), ""))
            .await
            .unwrap()
            .is_err()
    );
    empty.request().await;
    empty.finish().await;
    let mut truncated = Server::start(
        vec![bytes("HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort")],
        true,
    )
    .await;
    assert!(timeout(DEADLINE, truncated.client().get(URL, false, 10))
        .await
        .unwrap()
        .is_err());
    truncated.request().await;
    truncated.finish().await;
}

#[tokio::test]
async fn real_tls_redirect_is_returned_not_followed() {
    let mut server = Server::start(vec![bytes("HTTP/1.1 302 Found\r\nLocation: https://api.github.com/follow\r\nContent-Length: 0\r\n\r\n")], true).await;
    let response = timeout(DEADLINE, server.client().get(URL, false, 8))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status, 302);
    assert_eq!(
        response.location.as_deref(),
        Some("https://api.github.com/follow")
    );
    server.request().await;
    server.finish().await;
}

#[tokio::test]
async fn real_tls_timeout_and_signed_url_error_redaction() {
    let mut server = Server::start(
        vec![bytes("HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n")],
        false,
    )
    .await;
    let client = ReqwestPublicTransport::loopback_test(server.addr, Duration::from_millis(700));
    let signed =
        "https://release-assets.githubusercontent.com/private-file?sig=TEST_SIGNATURE_SECRET";
    let start = Instant::now();
    let request = client.get(signed, true, 8);
    let (result, raw) = tokio::join!(timeout(DEADLINE, request), server.request());
    assert!(raw.starts_with("GET /private-file?sig=TEST_SIGNATURE_SECRET HTTP/1.1\r\n"));
    let error = format!("{:?}", result.unwrap().unwrap_err());
    assert!(!error.contains("TEST_SIGNATURE_SECRET"));
    assert!(!error.contains("private-file"));
    assert!(!error.contains("https://"));
    assert!(start.elapsed() >= Duration::from_millis(500));
    assert!(start.elapsed() < DEADLINE);
    server.finish().await;
}

#[tokio::test]
async fn real_adapter_forbidden_origins_rejected_before_connect() {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let client = ReqwestPublicTransport::loopback_test(addr, Duration::from_millis(200));
    for (url, asset) in [
        ("http://api.github.com/test", false),
        ("https://api.github.com:443/test", false),
        ("https://user@api.github.com/test", false),
        ("https://api.github.com/test#fragment", false),
        ("https://api.github.com.evil.invalid/test", false),
        ("https://release-assets.githubusercontent.com/test", false),
        ("https://127.0.0.1/test", true),
        ("https://localhost/test", true),
    ] {
        assert_error(
            timeout(DEADLINE, client.get(url, asset, 8)).await.unwrap(),
            "HTTP origin rejected",
        );
    }
    assert!(timeout(Duration::from_millis(50), listener.accept())
        .await
        .is_err());
}
