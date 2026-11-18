use axum::Json;
use serde_json::{json, Value};

pub async fn handle_legacy_code_completion(request: hyper::Request<hyper::Body>) -> Json<Value> {
    use hyper::body::HttpBody;
    let mut body = request.into_body();
    while let Some(Ok(_)) = body.data().await {}
    Json(json!({
        "choices": [{"index": 0, "code_completion": "", "finish_reason": "stop"}],
        "created": 0.0,
        "model": "",
        "cached": false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn legacy_code_completion_is_empty_for_both_client_parsers() {
        let value = handle_legacy_code_completion(hyper::Request::new(hyper::Body::empty()))
            .await
            .0;
        assert_eq!(value["choices"].as_array().unwrap().len(), 1);
        assert_eq!(value["choices"][0]["code_completion"].as_str(), Some(""));
        assert_eq!(value["choices"][0]["index"].as_u64(), Some(0));
        assert_eq!(value["choices"][0]["finish_reason"].as_str(), Some("stop"));
        assert_eq!(value["created"].as_f64(), Some(0.0));
        assert_eq!(value["model"].as_str(), Some(""));
        assert_eq!(value["cached"].as_bool(), Some(false));
    }

    async fn read_response<R: tokio::io::AsyncBufRead + Unpin>(
        reader: &mut R,
    ) -> (String, Vec<(String, String)>, Value) {
        let mut status = String::new();
        reader.read_line(&mut status).await.unwrap();
        let mut headers = Vec::new();
        loop {
            let mut line = String::new();
            assert_ne!(
                reader.read_line(&mut line).await.unwrap(),
                0,
                "closed in headers"
            );
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
        let length: usize = headers
            .iter()
            .find(|(name, _)| name == "content-length")
            .map(|(_, value)| value.parse().unwrap())
            .expect("inert completion must have a content length");
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await.unwrap();
        (
            status.trim_end().to_string(),
            headers,
            serde_json::from_slice(&body).unwrap(),
        )
    }

    fn inert() -> Value {
        json!({
            "choices": [{"index": 0, "code_completion": "", "finish_reason": "stop"}],
            "created": 0.0, "model": "", "cached": false,
        })
    }

    async fn serve() -> (
        BufReader<tokio::net::tcp::OwnedReadHalf>,
        tokio::net::tcp::OwnedWriteHalf,
        String,
        tokio::task::JoinHandle<Result<(), hyper::Error>>,
    ) {
        let app = crate::app_state::AppState::from_gcx(
            crate::global_context::tests::make_test_gcx().await,
        )
        .await;
        let router = crate::http::routers::make_refact_http_server(app);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(router.into_make_service()),
        );
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (read_half, write_half) = stream.into_split();
        (
            BufReader::new(read_half),
            write_half,
            addr.to_string(),
            server,
        )
    }

    async fn expect_inert_keep_alive<R: tokio::io::AsyncBufRead + Unpin>(
        reader: &mut R,
        attempt: &str,
    ) {
        let (status, headers, value) =
            tokio::time::timeout(std::time::Duration::from_secs(60), read_response(reader))
                .await
                .unwrap_or_else(|_| panic!("{attempt}: no response"));
        assert_eq!(status, "HTTP/1.1 200 OK", "{attempt}");
        assert!(
            !headers
                .iter()
                .any(|(name, value)| name == "connection" && value.eq_ignore_ascii_case("close")),
            "{attempt}: keep-alive was disabled: {headers:?}"
        );
        assert_eq!(value, inert(), "{attempt}");
    }

    #[tokio::test]
    async fn legacy_code_completion_drains_large_bodies_over_keep_alive() {
        let (mut reader, mut write_half, addr, server) = serve().await;
        let document = format!(
            r#"{{"inputs":{{"sources":{{"a.py":"{}"}},"cursor":{{"file":"a.py","line":0,"character":0}}}},"stream":false}}"#,
            "x".repeat(512 * 1024)
        );
        for attempt in 0..2 {
            let request = format!(
                "POST /v1/code-completion HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{document}",
                document.len()
            );
            let writer = tokio::spawn(async move {
                write_half.write_all(request.as_bytes()).await.unwrap();
                write_half.flush().await.unwrap();
                write_half
            });
            expect_inert_keep_alive(&mut reader, &format!("attempt {attempt}")).await;
            write_half = tokio::time::timeout(std::time::Duration::from_secs(20), writer)
                .await
                .unwrap_or_else(|_| panic!("attempt {attempt}: body was not consumed"))
                .unwrap();
        }
        server.abort();
    }

    #[tokio::test]
    async fn legacy_code_completion_streams_chunked_bodies_past_the_body_limit() {
        let (mut reader, mut write_half, addr, server) = serve().await;
        const CHUNK: usize = 64 * 1024;
        const CHUNKS: usize = 16 * 1024 * 1024 / CHUNK + 1;
        let writer = tokio::spawn(async move {
            write_half
                .write_all(
                    format!(
                        "POST /v1/code-completion HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let chunk = format!("{CHUNK:x}\r\n{}\r\n", "x".repeat(CHUNK));
            for _ in 0..CHUNKS {
                write_half.write_all(chunk.as_bytes()).await.unwrap();
            }
            write_half.write_all(b"0\r\n\r\n").await.unwrap();
            write_half
                .write_all(
                    format!(
                        "POST /v1/code-completion HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 2\r\n\r\n{{}}"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            write_half.flush().await.unwrap();
            write_half
        });
        expect_inert_keep_alive(&mut reader, "over-limit chunked request").await;
        expect_inert_keep_alive(&mut reader, "follow-up request").await;
        tokio::time::timeout(std::time::Duration::from_secs(60), writer)
            .await
            .expect("chunked body was not consumed")
            .unwrap();
        server.abort();
    }
}
