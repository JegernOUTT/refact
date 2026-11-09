use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedMutexGuard, Semaphore};
use tokenizers::Tokenizer;
use reqwest::header::AUTHORIZATION;
use crate::files_correction::canonical_path;
use crate::global_context::GlobalContext;
use crate::caps::{default_hf_tokenizer_template, strip_model_from_finetune, BaseModelRecord};
use refact_core::model_caps::is_predefined_cloud_tokenizer;

const TOKENIZER_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const TOKENIZER_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const TOKENIZER_DOWNLOAD_ATTEMPTS: usize = 3;
const TOKENIZER_RETRY_DELAY: Duration = Duration::from_millis(200);
const TOKENIZER_MAX_BYTES: usize = 32 * 1024 * 1024;
static TOKENIZER_PARSE_PERMITS: Semaphore = Semaphore::const_new(1);

struct TokenizerLoadLease {
    _model: Arc<OwnedMutexGuard<()>>,
    _cache: OwnedMutexGuard<()>,
}

async fn tokenizer_blocking<F>(work: F) -> Result<Tokenizer, String>
where
    F: FnOnce() -> Result<Tokenizer, String> + Send + 'static,
{
    let permit = TOKENIZER_PARSE_PERMITS
        .acquire()
        .await
        .map_err(|_| "tokenizer loader unavailable".to_string())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await
    .map_err(|_| "tokenizer loader failed".to_string())?
}

fn parse_tokenizer(bytes: &[u8]) -> Result<Tokenizer, String> {
    if bytes.len() > TOKENIZER_MAX_BYTES {
        return Err("tokenizer exceeds size limit".to_string());
    }
    Tokenizer::from_bytes(bytes).map_err(|_| "invalid tokenizer file".to_string())
}

async fn open_tokenizer(
    path: &Path,
    lease: Option<Arc<TokenizerLoadLease>>,
) -> Result<Tokenizer, String> {
    let path = path.to_path_buf();
    tokenizer_blocking(move || {
        let _lease = lease;
        let metadata =
            std::fs::metadata(&path).map_err(|_| "failed to inspect tokenizer file".to_string())?;
        if !metadata.is_file() || metadata.len() > TOKENIZER_MAX_BYTES as u64 {
            return Err("tokenizer file is not regular or exceeds size limit".to_string());
        }
        let file =
            std::fs::File::open(path).map_err(|_| "failed to open tokenizer file".to_string())?;
        let metadata = file
            .metadata()
            .map_err(|_| "failed to inspect tokenizer file".to_string())?;
        if !metadata.is_file() || metadata.len() > TOKENIZER_MAX_BYTES as u64 {
            return Err("tokenizer file is not regular or exceeds size limit".to_string());
        }
        let mut bytes = Vec::new();
        file.take((TOKENIZER_MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "failed to read tokenizer file".to_string())?;
        parse_tokenizer(&bytes)
    })
    .await
}

async fn download_tokenizer_file(
    http_client: &reqwest::Client,
    http_path: &str,
    tokenizer_api_token: &str,
) -> Result<Vec<u8>, String> {
    let mut req = http_client
        .get(http_path)
        .timeout(TOKENIZER_REQUEST_TIMEOUT);
    if !tokenizer_api_token.is_empty() {
        req = req.header(AUTHORIZATION, format!("Bearer {tokenizer_api_token}"));
    }
    let mut res = req
        .send()
        .await
        .map_err(|_| "tokenizer request failed".to_string())?
        .error_for_status()
        .map_err(|_| "tokenizer request returned an unsuccessful status".to_string())?;
    if res
        .content_length()
        .is_some_and(|len| len > TOKENIZER_MAX_BYTES as u64)
    {
        return Err("tokenizer exceeds size limit".to_string());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|_| "failed to read tokenizer response".to_string())?
    {
        if chunk.len() > TOKENIZER_MAX_BYTES - bytes.len() {
            return Err("tokenizer exceeds size limit".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn tokenizer_load_is_disabled(tokenizer: &str) -> bool {
    tokenizer.starts_with("fake") || is_predefined_cloud_tokenizer(tokenizer)
}

#[cfg(test)]
async fn try_download_tokenizer_file_and_open(
    http_client: &reqwest::Client,
    http_path: &str,
    tokenizer_api_token: &str,
    path: &Path,
) -> Result<Tokenizer, String> {
    try_download_tokenizer_file_and_open_with_lease(
        http_client,
        http_path,
        tokenizer_api_token,
        path,
        None,
    )
    .await
}

async fn try_download_tokenizer_file_and_open_with_lease(
    http_client: &reqwest::Client,
    http_path: &str,
    tokenizer_api_token: &str,
    path: &Path,
    lease: Option<Arc<TokenizerLoadLease>>,
) -> Result<Tokenizer, String> {
    if let Ok(tokenizer) = open_tokenizer(path, lease.clone()).await {
        return Ok(tokenizer);
    }
    let deadline = tokio::time::Instant::now() + TOKENIZER_DOWNLOAD_TIMEOUT;
    let mut last_error = "failed to download tokenizer".to_string();
    for attempt in 0..TOKENIZER_DOWNLOAD_ATTEMPTS {
        let download = async {
            if attempt != 0 {
                tokio::time::sleep(TOKENIZER_RETRY_DELAY).await;
            }
            download_tokenizer_file(http_client, http_path, tokenizer_api_token).await
        };
        let bytes = match tokio::time::timeout_at(deadline, download).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(err)) => {
                last_error = err;
                continue;
            }
            Err(_) => return Err("tokenizer download deadline exceeded".to_string()),
        };
        let path = path.to_path_buf();
        let worker_lease = lease.clone();
        match tokenizer_blocking(move || {
            let _lease = worker_lease;
            let tokenizer = parse_tokenizer(&bytes)?;
            let parent = path
                .parent()
                .ok_or_else(|| "tokenizer path has no parent".to_string())?;
            std::fs::create_dir_all(parent)
                .map_err(|_| "failed to create tokenizer cache directory".to_string())?;
            let mut temp = tempfile::NamedTempFile::new_in(parent)
                .map_err(|_| "failed to create tokenizer staging file".to_string())?;
            temp.write_all(&bytes)
                .and_then(|_| temp.flush())
                .map_err(|_| "failed to write tokenizer staging file".to_string())?;
            temp.persist(&path)
                .map_err(|_| "failed to publish tokenizer cache file".to_string())?;
            Ok(tokenizer)
        })
        .await
        {
            Ok(tokenizer) => return Ok(tokenizer),
            Err(err) => last_error = err,
        }
    }
    Err(last_error)
}

pub async fn cached_tokenizer(
    global_context: Arc<GlobalContext>,
    model_rec: &BaseModelRecord,
) -> Result<Option<Arc<Tokenizer>>, String> {
    let model_id = strip_model_from_finetune(&model_rec.id);
    let tokenizer_state = global_context.tokenizer_state.clone();
    if let Some(tokenizer) = tokenizer_state.read().unwrap().map.get(&model_id).cloned() {
        return Ok(tokenizer);
    }
    if tokenizer_load_is_disabled(&model_rec.tokenizer) {
        return Ok(None);
    }
    let tokenizer_download_lock = tokenizer_state.read().unwrap().download_lock(&model_id);
    let model_lease = Arc::new(tokenizer_download_lock.lock_owned().await);
    if let Some(tokenizer) = tokenizer_state.read().unwrap().map.get(&model_id).cloned() {
        return Ok(tokenizer);
    }

    let (client2, cache_dir, hf_tokenizer_template) = {
        let (client, cache_dir, caps_state) = {
            (
                global_context.http_client.clone(),
                global_context.cache_dir.clone(),
                global_context.caps_state.clone(),
            )
        };
        let template = caps_state
            .read()
            .await
            .caps
            .clone()
            .map(|caps| caps.hf_tokenizer_template.clone())
            .unwrap_or_else(default_hf_tokenizer_template);
        (client, cache_dir, template)
    };

    let (mut tok_file_path, tok_url) = match &model_rec.tokenizer {
        empty_tok if empty_tok.is_empty() => {
            return Err(format!(
                "failed to load tokenizer: empty tokenizer for {model_id}"
            ))
        }
        disabled_tok if tokenizer_load_is_disabled(disabled_tok) => return Ok(None),
        hf_tok if hf_tok.starts_with("hf://") => {
            let hf_model = hf_tok.strip_prefix("hf://").unwrap();
            let url = hf_tokenizer_template.replace("$HF_MODEL", hf_model);
            (PathBuf::new(), url)
        }
        http_tok if http_tok.starts_with("http://") || http_tok.starts_with("https://") => {
            (PathBuf::new(), http_tok.to_string())
        }
        file_tok => {
            let file = if file_tok.starts_with("file://") {
                url::Url::parse(file_tok)
                    .and_then(|url| url.to_file_path().map_err(|_| url::ParseError::EmptyHost))
                    .map_err(|_| "invalid tokenizer file URL".to_string())?
            } else {
                canonical_path(file_tok)
            };
            (canonical_path(file.to_string_lossy()), "".to_string())
        }
    };

    let remote = tok_file_path.as_os_str().is_empty();
    if remote {
        let tokenizer_cache_dir = std::path::PathBuf::from(cache_dir).join("tokenizers");
        let sanitized_tok_key = tok_url
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect::<String>();

        tok_file_path = tokenizer_cache_dir
            .join(&sanitized_tok_key)
            .join("tokenizer.json");
    }
    #[cfg(target_os = "linux")]
    let cache_key = crate::files_correction::canonicalize_normalized_path(tok_file_path.clone());
    #[cfg(not(target_os = "linux"))]
    let cache_key = PathBuf::new();
    let cache_lock = tokenizer_state.read().unwrap().cache_lock(&cache_key);
    let lease = Arc::new(TokenizerLoadLease {
        _model: model_lease,
        _cache: cache_lock.lock_owned().await,
    });

    let mut tokenizer = if remote {
        try_download_tokenizer_file_and_open_with_lease(
            &client2,
            &tok_url,
            &model_rec.tokenizer_api_key,
            &tok_file_path,
            Some(lease.clone()),
        )
        .await?
    } else {
        open_tokenizer(&tok_file_path, Some(lease.clone())).await?
    };
    let _ = tokenizer.with_truncation(None);
    tokenizer.with_padding(None);
    let arc = Some(Arc::new(tokenizer));

    tokenizer_state
        .write()
        .unwrap()
        .map
        .insert(model_id, arc.clone());
    Ok(arc)
}

/// Estimate as length / 3.5, since 3 is reasonable estimate for code, and 4 for natural language
fn estimate_tokens(text: &str) -> usize {
    1 + text.len() * 2 / 7
}

pub fn count_text_tokens(tokenizer: Option<Arc<Tokenizer>>, text: &str) -> Result<usize, String> {
    match tokenizer {
        Some(tokenizer) => match tokenizer.encode_fast(text, false) {
            Ok(tokens) => Ok(tokens.len()),
            Err(e) => Err(format!("Encoding error: {e}")),
        },
        None => Ok(estimate_tokens(text)),
    }
}

pub fn count_text_tokens_with_fallback(tokenizer: Option<Arc<Tokenizer>>, text: &str) -> usize {
    count_text_tokens(tokenizer, text).unwrap_or_else(|e| {
        tracing::error!("{e}");
        estimate_tokens(text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_tokenizer() -> Tokenizer {
        use tokenizers::models::wordlevel::WordLevel;
        use std::collections::HashMap;
        let vocab = HashMap::from([("[UNK]".to_string(), 0), ("hello".to_string(), 1)]);
        Tokenizer::new(
            WordLevel::builder()
                .vocab(vocab.into_iter().collect())
                .unk_token("[UNK]".into())
                .build()
                .unwrap(),
        )
    }

    #[test]
    fn tokenizer_load_is_disabled_for_fake_and_predefined_cloud_tokenizers() {
        assert!(tokenizer_load_is_disabled("fake"));
        assert!(tokenizer_load_is_disabled("fake-gemini"));
        assert!(tokenizer_load_is_disabled("openai"));
        assert!(tokenizer_load_is_disabled("anthropic"));
        assert!(tokenizer_load_is_disabled("claude"));
    }

    #[test]
    fn tokenizer_load_remains_enabled_for_external_tokenizer_sources() {
        assert!(!tokenizer_load_is_disabled("hf://Xenova/claude-tokenizer"));
        assert!(!tokenizer_load_is_disabled(
            "https://example.com/tokenizer.json"
        ));
        assert!(!tokenizer_load_is_disabled("file:///tmp/tokenizer.json"));
        assert!(!tokenizer_load_is_disabled("/tmp/tokenizer.json"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cached_present_and_absent_entries_bypass_download_lock() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let model = BaseModelRecord {
            id: "cached-test/model".into(),
            tokenizer: "https://127.0.0.1:1/tokenizer.json".into(),
            ..Default::default()
        };
        let tokenizer = Arc::new(small_tokenizer());
        gcx.tokenizer_state
            .write()
            .unwrap()
            .map
            .insert(model.id.clone(), Some(tokenizer.clone()));
        let lock = gcx.tokenizer_state.read().unwrap().download_lock(&model.id);
        let guard = lock.lock().await;
        let mut future = Box::pin(cached_tokenizer(gcx.clone(), &model));
        let ready = match futures::poll!(future.as_mut()) {
            std::task::Poll::Ready(result) => result.unwrap().unwrap(),
            std::task::Poll::Pending => panic!("cached tokenizer waited for download lock"),
        };
        assert!(Arc::ptr_eq(&tokenizer, &ready));

        let absent = BaseModelRecord {
            id: "cached-test/none".into(),
            ..model.clone()
        };
        gcx.tokenizer_state
            .write()
            .unwrap()
            .map
            .insert(absent.id.clone(), None);
        let absent_lock = gcx
            .tokenizer_state
            .read()
            .unwrap()
            .download_lock(&absent.id);
        let absent_guard = absent_lock.lock().await;
        let mut absent_future = Box::pin(cached_tokenizer(gcx.clone(), &absent));
        match futures::poll!(absent_future.as_mut()) {
            std::task::Poll::Ready(result) => assert!(result.unwrap().is_none()),
            std::task::Poll::Pending => panic!("cached None waited for download lock"),
        }
        let disabled = BaseModelRecord {
            id: "fake-model".into(),
            tokenizer: "fake".into(),
            ..Default::default()
        };
        let disabled_lock = gcx
            .tokenizer_state
            .read()
            .unwrap()
            .download_lock(&disabled.id);
        let disabled_guard = disabled_lock.lock().await;
        let mut disabled_future = Box::pin(cached_tokenizer(gcx, &disabled));
        match futures::poll!(disabled_future.as_mut()) {
            std::task::Poll::Ready(result) => assert!(result.unwrap().is_none()),
            std::task::Poll::Pending => panic!("disabled tokenizer waited for download lock"),
        }
        drop(disabled_guard);
        drop(absent_guard);
        drop(guard);
    }

    fn serve_responses(responses: Vec<(u16, Vec<u8>)>) -> (String, std::thread::JoinHandle<usize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let mut count = 0;
            for (status, body) in responses {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if std::time::Instant::now() >= deadline {
                                return count;
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => return count,
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request).unwrap();
                count += 1;
                let reason = if status == 200 {
                    "OK"
                } else {
                    "Internal Server Error"
                };
                write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
            count
        });
        (format!("http://{address}/tokenizer.json"), thread)
    }

    #[tokio::test]
    #[serial_test::serial(tokenizer_blocking)]
    async fn tokenizer_download_retries_invalid_body_and_preserves_existing_file_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokenizer.json");
        std::fs::write(&path, b"initially malformed").unwrap();
        let expected = small_tokenizer();
        let expected_json = expected.to_string(false).unwrap();
        let body = expected_json.as_bytes().to_vec();
        let (url, server) = serve_responses(vec![(200, b"invalid json".to_vec()), (200, body)]);
        let loaded = try_download_tokenizer_file_and_open(&reqwest::Client::new(), &url, "", &path)
            .await
            .unwrap();
        assert_eq!(
            loaded.encode("hello", false).unwrap().get_ids(),
            expected.encode("hello", false).unwrap().get_ids()
        );
        assert_eq!(server.join().unwrap(), 2);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "staging file remained"
        );
        assert_eq!(std::fs::read(&path).unwrap(), expected_json.as_bytes());

        std::fs::write(&path, b"preserve these invalid bytes").unwrap();
        let before = std::fs::read(&path).unwrap();
        let (url, server) = serve_responses(vec![(500, Vec::new()); TOKENIZER_DOWNLOAD_ATTEMPTS]);
        let error = try_download_tokenizer_file_and_open(&reqwest::Client::new(), &url, "", &path)
            .await
            .unwrap_err();
        assert!(error.contains("unsuccessful status"), "{error}");
        assert_eq!(server.join().unwrap(), TOKENIZER_DOWNLOAD_ATTEMPTS);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "staging file remained"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[serial_test::serial(tokenizer_blocking)]
    async fn stalled_download_for_one_model_does_not_block_another_model() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let gcx = crate::global_context::tests::make_test_gcx().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/tokenizer.json", listener.local_addr().unwrap());
        let (headers_tx, headers_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let body = small_tokenizer().to_string(false).unwrap().into_bytes();
        let body_len = body.len();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
            let _ = headers_tx.send(());
            let _ = release_rx.await;
            let _ = stream.write_all(&body).await;
        });

        let model_a = BaseModelRecord {
            id: "stalled-model-a".into(),
            tokenizer: url,
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let tokenizer_path = dir.path().join("tokenizer.json");
        std::fs::write(&tokenizer_path, small_tokenizer().to_string(false).unwrap()).unwrap();
        let model_b = BaseModelRecord {
            id: "local-model-b".into(),
            tokenizer: tokenizer_path.to_string_lossy().into_owned(),
            ..Default::default()
        };

        let download_gcx = gcx.clone();
        let download = tokio::spawn(async move { cached_tokenizer(download_gcx, &model_a).await });
        tokio::time::timeout(Duration::from_secs(2), headers_rx)
            .await
            .expect("server did not send response headers")
            .expect("header notification dropped");

        let local_gcx = gcx;
        let mut local = tokio::spawn(async move { cached_tokenizer(local_gcx, &model_b).await });
        let local_result = tokio::time::timeout(Duration::from_secs(2), &mut local).await;
        let local_completed_before_release = local_result.is_ok();

        let _ = release_tx.send(());
        server.await.expect("server task failed");
        let download_result = tokio::time::timeout(Duration::from_secs(2), download)
            .await
            .expect("download task did not finish after body release")
            .expect("download task panicked");
        let local_result = match local_result {
            Ok(joined) => joined,
            Err(_) => tokio::time::timeout(Duration::from_secs(2), &mut local)
                .await
                .expect("local tokenizer task did not finish after cleanup"),
        };

        let loaded = local_result
            .expect("local tokenizer task panicked")
            .expect("local tokenizer failed")
            .expect("local tokenizer unexpectedly absent");
        assert!(
            download_result.is_ok(),
            "valid tokenizer body failed to load"
        );
        assert!(loaded.encode("hello", false).is_ok());
        assert!(
            local_completed_before_release,
            "local tokenizer load was blocked by another model's stalled download"
        );
    }

    #[tokio::test]
    #[serial_test::serial(tokenizer_blocking)]
    async fn concurrent_loads_of_same_model_share_one_download_and_arc() {
        assert_concurrent_loads_share_one_download(true).await;
    }

    #[tokio::test]
    #[serial_test::serial(tokenizer_blocking)]
    async fn different_models_sharing_a_cache_path_download_once() {
        assert_concurrent_loads_share_one_download(false).await;
    }

    async fn assert_concurrent_loads_share_one_download(same_model: bool) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let gcx = crate::global_context::tests::make_test_gcx().await;
        let body = small_tokenizer().to_string(false).unwrap().into_bytes();
        let body_len = body.len();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/tokenizer.json", listener.local_addr().unwrap());
        let (headers_tx, headers_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
            let _ = headers_tx.send(());
            let mut release_rx = release_rx;
            let mut duplicate = tokio::select! {
                _ = &mut release_rx => None,
                result = listener.accept() => {
                    let duplicate = result.unwrap().0;
                    let _ = release_rx.await;
                    Some(duplicate)
                }
            };
            let _ = stream.write_all(&body).await;
            if duplicate.is_none() {
                duplicate = tokio::time::timeout(Duration::from_millis(200), listener.accept())
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .map(|(stream, _)| stream);
            }
            if let Some(mut duplicate) = duplicate {
                let _ = tokio::time::timeout(Duration::from_secs(2), duplicate.read(&mut request))
                    .await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n"
                );
                let _ = duplicate.write_all(response.as_bytes()).await;
                let _ = duplicate.write_all(&body).await;
                true
            } else {
                false
            }
        });

        let model = BaseModelRecord {
            id: "same-model".into(),
            tokenizer: url,
            ..Default::default()
        };
        let first_gcx = gcx.clone();
        let first_model = model.clone();
        let mut first =
            tokio::spawn(async move { cached_tokenizer(first_gcx, &first_model).await });
        tokio::time::timeout(Duration::from_secs(2), headers_rx)
            .await
            .expect("server did not receive the first request")
            .expect("header notification dropped");
        let mut second_model = model.clone();
        if !same_model {
            second_model.id = "another-model".into();
        }
        let mut second = Box::pin(cached_tokenizer(gcx, &second_model));
        let second_pending = futures::poll!(second.as_mut()).is_pending();
        let before_release = tokio::time::timeout(Duration::from_millis(200), &mut second).await;
        let waited_for_release = before_release.is_err();
        let _ = release_tx.send(());
        let extra_request = server.await.expect("server task failed");
        let first_result = tokio::time::timeout(Duration::from_secs(2), &mut first)
            .await
            .expect("first caller timed out")
            .expect("first caller panicked");
        let second_result = match before_release {
            Ok(result) => result,
            Err(_) => tokio::time::timeout(Duration::from_secs(2), &mut second)
                .await
                .expect("second caller timed out"),
        }
        .expect("second load failed");

        let first = first_result
            .expect("first load failed")
            .expect("first tokenizer absent");
        let second = second_result.expect("second tokenizer absent");
        assert!(
            second_pending,
            "second caller did not wait for the gated load"
        );
        assert!(
            waited_for_release,
            "second load completed before the gated response was released"
        );
        assert!(
            !extra_request,
            "more than one HTTP request was received for the same model"
        );
        if same_model {
            assert!(
                Arc::ptr_eq(&first, &second),
                "same model produced distinct tokenizer Arcs"
            );
        }
        assert_eq!(
            first.encode("hello", false).unwrap().get_ids(),
            second.encode("hello", false).unwrap().get_ids()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tokenizer_request_timeout_covers_stalled_response_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/tokenizer.json", listener.local_addr().unwrap());
        let (headers_tx, headers_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nx")
                .await
                .unwrap();
            let _ = headers_tx.send(());
            let _ = release_rx.await;
        });
        let client = tokio::spawn(async move {
            download_tokenizer_file(&reqwest::Client::new(), &url, "").await
        });
        headers_rx
            .await
            .expect("server did not send response headers");
        tokio::time::advance(TOKENIZER_REQUEST_TIMEOUT + Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        let timed_out = client.is_finished();
        let _ = release_tx.send(());
        server.await.expect("server task failed");
        let result = client.await.expect("client task failed");
        assert!(timed_out, "client did not finish at the request timeout");
        assert!(
            result.is_err(),
            "stalled response body unexpectedly succeeded"
        );
    }

    #[tokio::test]
    #[serial_test::serial(tokenizer_blocking)]
    async fn cancelled_tokenizer_blocking_keeps_permit_until_worker_exits() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let mut waiter = tokio::spawn(async move {
            tokenizer_blocking(move || {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                finished_tx.send(()).unwrap();
                Ok(small_tokenizer())
            })
            .await
        });
        tokio::time::timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(move || started_rx.recv().unwrap()),
        )
        .await
        .expect("worker did not start")
        .unwrap();
        waiter.abort();
        let permit_retained = TOKENIZER_PARSE_PERMITS.try_acquire().is_err();
        release_tx.send(()).unwrap();
        tokio::time::timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(move || finished_rx.recv().unwrap()),
        )
        .await
        .expect("worker did not finish")
        .unwrap();
        tokio::time::timeout(Duration::from_secs(2), TOKENIZER_PARSE_PERMITS.acquire())
            .await
            .expect("worker did not release parse permit")
            .unwrap();
        assert!(
            permit_retained,
            "cancelling the waiter released the worker's permit"
        );
    }

    #[tokio::test]
    #[serial_test::serial(tokenizer_blocking)]
    async fn cancelled_tokenizer_worker_keeps_model_lease_until_worker_exits() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let model_id = "cancelled-open/model";
        let state = crate::global_context::tests::make_test_gcx()
            .await
            .tokenizer_state
            .clone();
        let lock = state.read().unwrap().download_lock(model_id);
        let lease = Arc::new(lock.clone().lock_owned().await);
        let worker_lease = lease;
        let waiter = tokio::spawn(async move {
            tokenizer_blocking(move || {
                let _lease = worker_lease;
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                finished_tx.send(()).unwrap();
                Ok(small_tokenizer())
            })
            .await
        });
        tokio::time::timeout(
            Duration::from_secs(2),
            tokio::task::spawn_blocking(move || started_rx.recv().unwrap()),
        )
        .await
        .expect("blocking worker did not start")
        .unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        let same_lock = state.read().unwrap().download_lock(model_id);
        assert!(Arc::ptr_eq(&lock, &same_lock));
        assert!(
            lock.try_lock().is_err(),
            "worker released model lease too soon"
        );
        release_tx.send(()).unwrap();
        tokio::time::timeout(
            Duration::from_secs(2),
            tokio::task::spawn_blocking(move || finished_rx.recv().unwrap()),
        )
        .await
        .expect("blocking worker did not exit")
        .unwrap();
        tokio::time::timeout(Duration::from_secs(2), lock.lock())
            .await
            .expect("model lease remained locked after worker exit");
        drop(lock);
        drop(same_lock);
    }

    #[test]
    fn invalid_and_oversized_tokenizer_bodies_are_rejected_without_parse_leaks() {
        assert!(parse_tokenizer(b"not json").is_err());
        assert_eq!(
            parse_tokenizer(&vec![0; TOKENIZER_MAX_BYTES + 1]).unwrap_err(),
            "tokenizer exceeds size limit"
        );
    }
}
