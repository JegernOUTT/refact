use reqwest::{Client, RequestBuilder, Response};
use serde_json::Value;
use std::sync::OnceLock;

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

pub(super) fn client() -> Result<&'static Client, String> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .user_agent("refact-agent")
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    let same_origin = attempt.previous().first().is_some_and(|original| {
                        original.scheme() == attempt.url().scheme()
                            && original.host_str() == attempt.url().host_str()
                            && original.port_or_known_default()
                                == attempt.url().port_or_known_default()
                    });
                    if same_origin && attempt.previous().len() < 5 {
                        attempt.follow()
                    } else {
                        attempt.stop()
                    }
                }))
                .build()
                .map_err(|_| "Failed to create search HTTP client".into())
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub(super) fn request_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "Request timed out".into()
    } else {
        "Provider request failed".into()
    }
}

pub(super) async fn body(mut response: Response) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err("Provider response exceeded size limit".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if chunk.len() > MAX_RESPONSE_BYTES - bytes.len() {
            return Err("Provider response exceeded size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(super) async fn text(response: Response) -> Result<String, String> {
    Ok(String::from_utf8_lossy(&body(response).await?).into_owned())
}

pub(super) async fn json(request: RequestBuilder) -> Result<Value, String> {
    let response = request.send().await.map_err(request_error)?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Provider returned HTTP {}", status.as_u16()));
    }
    serde_json::from_slice(&body(response).await?)
        .map_err(|_| "Provider returned invalid JSON".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn serve(headers: String, bytes: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/?key=never-echo-this",
            listener.local_addr().unwrap()
        );
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            let _ = socket.write_all(headers.as_bytes()).await;
            let _ = socket.write_all(&bytes).await;
        });
        url
    }

    #[tokio::test]
    async fn rejects_oversized_fixed_and_chunked_bodies() {
        for chunked in [false, true] {
            let bytes = vec![b' '; MAX_RESPONSE_BYTES + 1];
            let (headers, bytes) = if chunked {
                let mut framed = format!("{:x}\r\n", bytes.len()).into_bytes();
                framed.extend(bytes);
                framed.extend(b"\r\n0\r\n\r\n");
                (
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".into(),
                    framed,
                )
            } else {
                (
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", bytes.len()),
                    bytes,
                )
            };
            let url = serve(headers, bytes).await;
            assert_eq!(
                json(client().unwrap().get(url)).await.unwrap_err(),
                "Provider response exceeded size limit"
            );
        }
    }

    #[tokio::test]
    async fn transport_and_body_errors_never_echo_urls() {
        let error = client()
            .unwrap()
            .get("http://127.0.0.1:0/?key=never-echo-this")
            .send()
            .await
            .unwrap_err();
        assert!(!request_error(error).contains("never-echo-this"));
        let url = serve(
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n".into(),
            b"short".to_vec(),
        )
        .await;
        let error = json(client().unwrap().get(url)).await.unwrap_err();
        assert!(!error.contains("never-echo-this"));
    }

    #[tokio::test]
    async fn redirects_never_forward_custom_secrets_to_another_origin() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = serve(format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: http://localhost:{}/stolen\r\nContent-Length: 0\r\n\r\n", target.local_addr().unwrap().port()), vec![]).await;
        let response = client()
            .unwrap()
            .post(url)
            .header("x-api-key", "secret")
            .header("Mcp-Session-Id", "session")
            .body("private query")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 307);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), target.accept())
                .await
                .is_err()
        );
    }
}
