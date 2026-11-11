use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use futures::stream::BoxStream;
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::transport::streamable_http_client::{
    SseError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use rmcp::model::ClientJsonRpcMessage;
use sse_stream::Sse;

use crate::global_context::GlobalContext;

pub trait GatewayCredentialSource: Clone + Send + Sync + 'static {
    fn headers(
        &self,
        config_path: &str,
        url: &str,
    ) -> impl Future<Output = Result<Option<HashMap<String, String>>, String>> + Send;
    fn transport_url(
        &self,
        config_path: &str,
    ) -> impl Future<Output = Result<Option<String>, String>> + Send;
}

#[derive(Clone, Copy, Default)]
pub struct ProviderGatewaySource;

impl GatewayCredentialSource for ProviderGatewaySource {
    async fn headers(
        &self,
        config_path: &str,
        url: &str,
    ) -> Result<Option<HashMap<String, String>>, String> {
        crate::providers::litellm_mcp::gateway_headers(config_path, url).await
    }
    async fn transport_url(&self, config_path: &str) -> Result<Option<String>, String> {
        crate::providers::litellm_mcp::gateway_transport_url(config_path).await
    }
}

pub async fn resolve_request_headers<S: GatewayCredentialSource>(
    source: &S,
    config_path: &str,
    url: &str,
) -> Result<Option<HashMap<HeaderName, HeaderValue>>, String> {
    let Some(raw) = source.headers(config_path, url).await? else {
        return Ok(None);
    };
    match source.transport_url(config_path).await? {
        Some(expected) if expected.trim_end_matches('/') == url.trim().trim_end_matches('/') => {}
        _ => {
            return Err(
                "managed LiteLLM MCP credential refused: url is not the approved gateway transport"
                    .to_string(),
            );
        }
    }
    let mut out = HashMap::new();
    for (k, v) in raw {
        let name = HeaderName::from_bytes(k.as_bytes())
            .map_err(|_| "managed LiteLLM MCP credential has an invalid header name".to_string())?;
        let mut value = HeaderValue::from_str(&v).map_err(|_| {
            "managed LiteLLM MCP credential has an invalid header value".to_string()
        })?;
        value.set_sensitive(true);
        out.insert(name, value);
    }
    if let Some(token) =
        super::mcp_auth::gateway_oauth::current_gateway_access_token(config_path, url).await?
    {
        let mut token = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "Managed OAuth token is invalid".to_string())?;
        token.set_sensitive(true);
        out.insert(HeaderName::from_static("authorization"), token);
    }
    Ok(Some(out))
}

pub async fn check_dispatch(gcx: &Arc<GlobalContext>, config_path: &str) -> Result<(), String> {
    crate::providers::litellm_mcp::check_dispatch(gcx, config_path).await
}

#[derive(Clone)]
pub struct GatewayClient<C, S> {
    inner: C,
    config_path: Arc<str>,
    source: S,
}

impl<C, S> GatewayClient<C, S> {
    pub fn new(inner: C, config_path: &str, source: S) -> Self {
        Self {
            inner,
            config_path: Arc::from(config_path),
            source,
        }
    }
}

fn refused<E: std::error::Error + Send + Sync + 'static>(msg: String) -> StreamableHttpError<E> {
    StreamableHttpError::UnexpectedServerResponse(Cow::Owned(msg))
}

async fn merged<S: GatewayCredentialSource, E: std::error::Error + Send + Sync + 'static>(
    source: &S,
    config_path: &str,
    uri: &str,
    mut custom: HashMap<HeaderName, HeaderValue>,
) -> Result<HashMap<HeaderName, HeaderValue>, StreamableHttpError<E>> {
    if let Some(trusted) = resolve_request_headers(source, config_path, uri)
        .await
        .map_err(refused)?
    {
        custom.extend(trusted);
    }
    Ok(custom)
}

impl<C, S> StreamableHttpClient for GatewayClient<C, S>
where
    C: StreamableHttpClient + Sync,
    S: GatewayCredentialSource,
{
    type Error = C::Error;

    fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> impl Future<Output = Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>>>
           + Send
           + '_ {
        async move {
            let h = merged::<S, C::Error>(&self.source, &self.config_path, &uri, custom_headers)
                .await?;
            let auth_header = if h.contains_key(&HeaderName::from_static("authorization")) {
                None
            } else {
                auth_header
            };
            self.inner
                .post_message(uri, message, session_id, auth_header, h)
                .await
        }
    }

    fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> impl Future<Output = Result<(), StreamableHttpError<Self::Error>>> + Send + '_ {
        async move {
            let h = merged::<S, C::Error>(&self.source, &self.config_path, &uri, custom_headers)
                .await?;
            let auth_header = if h.contains_key(&HeaderName::from_static("authorization")) {
                None
            } else {
                auth_header
            };
            self.inner
                .delete_session(uri, session_id, auth_header, h)
                .await
        }
    }

    fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> impl Future<
        Output = Result<
            BoxStream<'static, Result<Sse, SseError>>,
            StreamableHttpError<Self::Error>,
        >,
    > + Send
           + '_ {
        async move {
            let h = merged::<S, C::Error>(&self.source, &self.config_path, &uri, custom_headers)
                .await?;
            let auth_header = if h.contains_key(&HeaderName::from_static("authorization")) {
                None
            } else {
                auth_header
            };
            self.inner
                .get_stream(uri, session_id, last_event_id, auth_header, h)
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct Src {
        headers: Arc<Mutex<Result<Option<HashMap<String, String>>, String>>>,
        url: Option<String>,
    }

    impl GatewayCredentialSource for Src {
        async fn headers(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<HashMap<String, String>>, String> {
            self.headers.lock().unwrap().clone()
        }
        async fn transport_url(&self, _: &str) -> Result<Option<String>, String> {
            Ok(self.url.clone())
        }
    }

    fn src(h: Result<Option<HashMap<String, String>>, String>, url: Option<&str>) -> Src {
        Src {
            headers: Arc::new(Mutex::new(h)),
            url: url.map(str::to_string),
        }
    }

    fn key(v: &str) -> Result<Option<HashMap<String, String>>, String> {
        Ok(Some(HashMap::from([(
            "x-litellm-api-key".to_string(),
            v.to_string(),
        )])))
    }

    const URL: &str = "https://gw.example/srv/mcp";

    #[tokio::test]
    async fn managed_injects_dynamic_key_for_approved_url() {
        let s = src(key("k1"), Some(URL));
        let h = resolve_request_headers(&s, "/g/http_llm_a.yaml", URL)
            .await
            .unwrap()
            .unwrap();
        let v = &h[&HeaderName::from_static("x-litellm-api-key")];
        assert_eq!(v.to_str().unwrap(), "k1");
        assert!(v.is_sensitive());
        *s.headers.lock().unwrap() = key("k2");
        let h = resolve_request_headers(&s, "/g/http_llm_a.yaml", URL)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            h[&HeaderName::from_static("x-litellm-api-key")]
                .to_str()
                .unwrap(),
            "k2"
        );
    }

    #[tokio::test]
    async fn manual_path_gets_no_injection() {
        let s = src(Ok(None), None);
        assert!(resolve_request_headers(&s, "/p/http_manual.yaml", URL)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn stale_binding_and_foreign_url_are_refused() {
        let s = src(Err("identity changed".into()), Some(URL));
        assert!(resolve_request_headers(&s, "/g/http_llm_a.yaml", URL)
            .await
            .is_err());
        let s = src(key("k"), Some(URL));
        assert!(
            resolve_request_headers(&s, "/g/http_llm_a.yaml", "https://gw.example/oauth/x")
                .await
                .is_err()
        );
        let s = src(key("k"), None);
        assert!(resolve_request_headers(&s, "/g/http_llm_a.yaml", URL)
            .await
            .is_err());
    }

    #[derive(Clone, Default)]
    struct Inner(Arc<Mutex<Vec<HashMap<HeaderName, HeaderValue>>>>);

    impl StreamableHttpClient for Inner {
        type Error = std::io::Error;
        async fn post_message(
            &self,
            _: Arc<str>,
            _: ClientJsonRpcMessage,
            _: Option<Arc<str>>,
            _: Option<String>,
            h: HashMap<HeaderName, HeaderValue>,
        ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
            self.0.lock().unwrap().push(h);
            Ok(StreamableHttpPostResponse::Accepted)
        }
        async fn delete_session(
            &self,
            _: Arc<str>,
            _: Arc<str>,
            _: Option<String>,
            h: HashMap<HeaderName, HeaderValue>,
        ) -> Result<(), StreamableHttpError<Self::Error>> {
            self.0.lock().unwrap().push(h);
            Ok(())
        }
        async fn get_stream(
            &self,
            _: Arc<str>,
            _: Arc<str>,
            _: Option<String>,
            _: Option<String>,
            _: HashMap<HeaderName, HeaderValue>,
        ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>>
        {
            Err(StreamableHttpError::ServerDoesNotSupportSse)
        }
    }

    #[tokio::test]
    async fn wrapper_sends_current_key_and_refuses_after_change() {
        let inner = Inner::default();
        let s = src(key("k1"), Some(URL));
        let c = GatewayClient::new(inner.clone(), "/g/http_llm_a.yaml", s.clone());
        let msg = || {
            serde_json::from_value::<ClientJsonRpcMessage>(serde_json::json!(
                {"jsonrpc":"2.0","method":"notifications/initialized"}
            ))
            .unwrap()
        };
        let manual = HashMap::from([(
            HeaderName::from_static("x-manual"),
            HeaderValue::from_static("m"),
        )]);
        c.post_message(URL.into(), msg(), None, None, manual)
            .await
            .unwrap();
        *s.headers.lock().unwrap() = key("k2");
        c.delete_session(URL.into(), "s".into(), None, HashMap::new())
            .await
            .unwrap();
        *s.headers.lock().unwrap() = Err("identity changed; reconfirm".into());
        assert!(c
            .post_message(URL.into(), msg(), None, None, HashMap::new())
            .await
            .is_err());
        let sent = inner.0.lock().unwrap();
        assert_eq!(sent.len(), 2, "refused request must not reach the network");
        assert_eq!(sent[0][&HeaderName::from_static("x-litellm-api-key")], "k1");
        assert_eq!(sent[0][&HeaderName::from_static("x-manual")], "m");
        assert_eq!(sent[1][&HeaderName::from_static("x-litellm-api-key")], "k2");
    }
}
