use super::*;
use crate::providers::instance::ProviderInstance;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn fixture(gcx: &Arc<GlobalContext>, name: &str, endpoint: &str, key: &str) {
    gcx.providers
        .write()
        .await
        .add(Box::new(ProviderInstance::new(
            name,
            "litellm",
            "Work",
            Box::new(LiteLLMProvider {
                endpoint: endpoint.into(),
                api_key: key.into(),
                enabled: true,
                ..Default::default()
            }),
        )));
}

async fn proxy(
    replies: Vec<(String, u16, Value)>,
) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut seen = Vec::new();
        for (expected_key, status, body) in replies {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let mut chunk = [0; 1024];
                let count = socket.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&chunk[..count]);
                assert!(request.len() < 16384);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("GET /key/info HTTP/1.1\r\n"));
            assert!(request
                .to_ascii_lowercase()
                .contains(&format!("authorization: bearer {expected_key}\r\n")));
            let body = body.to_string();
            socket.write_all(format!("HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            seen.push(request);
        }
        seen
    });
    (endpoint, task)
}

#[tokio::test]
async fn account_cache_tracks_actual_key_identity_and_denial_drops_prior_figures() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let (endpoint, server) = proxy(vec![
        (
            "first".into(),
            200,
            json!({"info":{"spend":1,"max_budget":10}}),
        ),
        (
            "second".into(),
            200,
            json!({"info":{"spend":7,"max_budget":20}}),
        ),
        ("second".into(), 403, json!({"error":"denied"})),
    ])
    .await;
    fixture(&gcx, "work", &endpoint, "first").await;
    let first = account(&gcx, "work", false).await.unwrap();
    assert_eq!(first["subjects"][0]["spend"]["value"].as_f64(), Some(1.0));
    assert_eq!(
        account(&gcx, "work", false).await.unwrap()["subjects"][0]["spend"]["value"].as_f64(),
        Some(1.0)
    );
    fixture(&gcx, "work", &endpoint, "second").await;
    let second = account(&gcx, "work", false).await.unwrap();
    assert_eq!(second["subjects"][0]["spend"]["value"].as_f64(), Some(7.0));
    let denied = account(&gcx, "work", true).await.unwrap();
    assert_eq!(denied["state"], "permission_denied");
    assert_eq!(denied["subjects"], json!([]));
    assert_eq!(denied["stale"], false);
    assert_eq!(server.await.unwrap().len(), 3);
}

#[tokio::test]
async fn invalid_usage_range_is_refused_before_any_proxy_request() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    fixture(&gcx, "work", "http://127.0.0.1:1", "unreachable").await;
    let result = usage(
        &gcx,
        "work",
        UsageQuery {
            start: Some(NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()),
            end: Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(result.unwrap_err().status_code, StatusCode::BAD_REQUEST);
}
