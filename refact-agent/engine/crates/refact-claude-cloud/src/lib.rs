mod eventstream;
pub use eventstream::BedrockEventStreamDecoder;

use aws_credential_types::{provider::ProvideCredentials, Credentials};
use aws_sigv4::{
    http_request::{sign, SignableBody, SignableRequest, SigningSettings},
    sign::v4,
};
use azure_core::credentials::TokenCredential;
use refact_core::claude_backend::{ClaudeAuthType, ClaudeBackend, ClaudeTransportConfig};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::{Mutex, OnceCell};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseFraming {
    Sse,
    AwsEventStream,
}

pub struct PreparedRequest {
    pub url: String,
    pub headers: HeaderMap,
    pub body_bytes: Vec<u8>,
    pub framing: ResponseFraming,
}

fn header(headers: &mut HeaderMap, name: &str, value: &str) -> Result<(), String> {
    let mut value = HeaderValue::from_str(value).map_err(|_| "Invalid credential header value")?;
    if matches!(
        name,
        "authorization" | "x-api-key" | "api-key" | "x-amz-security-token"
    ) {
        value.set_sensitive(true);
    }
    headers.insert(
        HeaderName::from_bytes(name.as_bytes()).map_err(|_| "Invalid header name")?,
        value,
    );
    Ok(())
}

fn required<'a>(value: &'a Option<String>, name: &str) -> Result<&'a str, String> {
    value
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("Claude cloud configuration requires {name}"))
}

fn endpoint(base: &str, segments: &[&str]) -> Result<String, String> {
    let mut url = url::Url::parse(base).map_err(|_| "Invalid Claude cloud endpoint")?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Invalid Claude cloud endpoint".into());
    }
    const PATH_ENCODE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~')
        .remove(b':')
        .remove(b'@');
    let mut path = url.path().trim_end_matches('/').to_string();
    for segment in segments {
        if *segment == "." || *segment == ".." {
            return Err("Invalid endpoint path segment".into());
        }
        path.push('/');
        path.push_str(&percent_encoding::utf8_percent_encode(segment, PATH_ENCODE).to_string());
    }
    url.set_path(&path);
    Ok(url.into())
}

fn messages_endpoint(base: &str) -> Result<String, String> {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1/messages") {
        endpoint(base, &[])
    } else if base.ends_with("/v1") {
        endpoint(base, &["messages"])
    } else {
        endpoint(base, &["v1", "messages"])
    }
}

pub async fn prepare_attempt(
    url: String,
    mut headers: HeaderMap,
    mut body: Value,
    model: &str,
    stream: bool,
    config: &ClaudeTransportConfig,
) -> Result<PreparedRequest, String> {
    let internal: Vec<_> = headers
        .keys()
        .filter(|k| k.as_str().starts_with("x-refact-internal-"))
        .cloned()
        .collect();
    for key in internal {
        headers.remove(key);
    }
    if config.backend != ClaudeBackend::Oauth {
        for name in [
            "authorization",
            "x-api-key",
            "api-key",
            "x-app",
            "x-stainless-helper-method",
            "x-amz-security-token",
            "x-amz-date",
            "anthropic-workspace-id",
            "host",
            "content-length",
        ] {
            headers.remove(name);
        }
        if let Some(beta) = headers.remove("anthropic-beta") {
            let filtered = beta
                .to_str()
                .map_err(|_| "Invalid Anthropic beta header")?
                .split(',')
                .map(str::trim)
                .filter(|v| !v.contains("oauth") && !v.starts_with("claude-code") && !v.is_empty())
                .collect::<Vec<_>>()
                .join(",");
            if !filtered.is_empty() {
                header(&mut headers, "anthropic-beta", &filtered)?;
            }
        }
    }
    header(&mut headers, "content-type", "application/json")?;
    let mut target = url;
    let mut framing = ResponseFraming::Sse;
    match config.backend {
        ClaudeBackend::Oauth => {}
        ClaudeBackend::Anthropic => {
            if config.auth_type == ClaudeAuthType::Oauth {
                if config.base_url.is_some() {
                    return Err(
                        "Claude Console OAuth supports only the default Anthropic endpoint".into(),
                    );
                }
                target = messages_endpoint("https://api.anthropic.com")?;
                header(
                    &mut headers,
                    "authorization",
                    &format!("Bearer {}", required(&config.auth_token, "auth_token")?),
                )?;
                let betas = match headers.get("anthropic-beta") {
                    Some(beta) => format!(
                        "{},oauth-2025-04-20",
                        beta.to_str().map_err(|_| "Invalid Anthropic beta header")?
                    ),
                    None => "oauth-2025-04-20".to_string(),
                };
                header(&mut headers, "anthropic-beta", &betas)?;
                if let Some(workspace) = config
                    .oauth_workspace_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                {
                    header(&mut headers, "anthropic-workspace-id", workspace)?;
                }
            } else {
                target = messages_endpoint(
                    config
                        .base_url
                        .as_deref()
                        .unwrap_or("https://api.anthropic.com"),
                )?;
                header(
                    &mut headers,
                    "x-api-key",
                    required(&config.api_key, "api_key")?,
                )?;
            }
            header(&mut headers, "anthropic-version", "2023-06-01")?;
        }
        ClaudeBackend::Custom => {
            if config.auth_type == ClaudeAuthType::Oauth {
                return Err("Claude OAuth is not supported for custom endpoints".into());
            }
            target = messages_endpoint(required(&config.base_url, "base_url")?)?;
            match config.auth_type {
                ClaudeAuthType::ApiKey => header(
                    &mut headers,
                    "x-api-key",
                    required(&config.api_key, "api_key")?,
                )?,
                ClaudeAuthType::Bearer => header(
                    &mut headers,
                    "authorization",
                    &format!("Bearer {}", required(&config.auth_token, "auth_token")?),
                )?,
                ClaudeAuthType::None => {}
                ClaudeAuthType::Oauth => {
                    return Err("Claude OAuth is not supported for custom endpoints".into())
                }
            }
            header(&mut headers, "anthropic-version", "2023-06-01")?;
        }
        ClaudeBackend::Bedrock => {
            let region = match config.aws_region.clone().filter(|v| !v.is_empty()) {
                Some(region) => region,
                None => tokio::time::timeout(Duration::from_secs(15), aws_sdk(config))
                    .await
                    .map_err(|_| "AWS region resolution timed out")??
                    .region()
                    .map(|r| r.as_ref().to_string())
                    .ok_or(
                        "AWS region unavailable; configure aws_region or an AWS profile region",
                    )?,
            };
            if !region
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            {
                return Err("Invalid AWS region".into());
            }
            let suffix = if region.starts_with("cn-") {
                "amazonaws.com.cn"
            } else if region.starts_with("us-iso-") {
                "c2s.ic.gov"
            } else if region.starts_with("us-isob-") {
                "sc2s.sgov.gov"
            } else if region.starts_with("eu-isoe-") {
                "cloud.adc-e.uk"
            } else if region.starts_with("us-isof-") {
                "csp.hci.ic.gov"
            } else {
                "amazonaws.com"
            };
            let base = config
                .base_url
                .clone()
                .unwrap_or_else(|| format!("https://bedrock-runtime.{region}.{suffix}"));
            target = endpoint(
                &base,
                &[
                    "model",
                    model,
                    if stream {
                        "invoke-with-response-stream"
                    } else {
                        "invoke"
                    },
                ],
            )?;
            transform(&mut body, "bedrock-2023-05-31")?;
            normalize_bedrock_cache_control(&mut body);
            if let Some(beta) = headers.remove("anthropic-beta") {
                let betas = beta
                    .to_str()
                    .map_err(|_| "Invalid Anthropic beta header")?
                    .split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(|v| Value::String(v.to_string()))
                    .collect::<Vec<_>>();
                body["anthropic_beta"] = Value::Array(betas);
            }
            if stream {
                framing = ResponseFraming::AwsEventStream;
                header(&mut headers, "accept", "application/vnd.amazon.eventstream")?;
                header(&mut headers, "x-amzn-bedrock-accept", "application/json")?;
            }
            if let Some(token) = config.auth_token.as_deref().filter(|v| !v.is_empty()) {
                header(&mut headers, "authorization", &format!("Bearer {token}"))?;
            } else {
                let bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
                let credentials =
                    tokio::time::timeout(Duration::from_secs(15), aws_credentials(config))
                        .await
                        .map_err(|_| "AWS credential resolution timed out")??;
                sign_request(
                    &target,
                    &mut headers,
                    &bytes,
                    &region,
                    credentials,
                    SystemTime::now(),
                )?;
                return Ok(PreparedRequest {
                    url: target,
                    headers,
                    body_bytes: bytes,
                    framing,
                });
            }
        }
        ClaudeBackend::Vertex => {
            let project = required(&config.gcp_project, "gcp_project")?;
            let region = required(&config.gcp_region, "gcp_region")?;
            if !region
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            {
                return Err("Invalid GCP region".into());
            }
            if matches!(region, "us" | "eu") {
                return Err(
                    "Vertex requires a regional location or global, not a multi-region".into(),
                );
            }
            let base = config.base_url.clone().unwrap_or_else(|| {
                if region == "global" {
                    "https://aiplatform.googleapis.com".into()
                } else {
                    format!("https://{region}-aiplatform.googleapis.com")
                }
            });
            target = endpoint(
                &base,
                &[
                    "v1",
                    "projects",
                    project,
                    "locations",
                    region,
                    "publishers",
                    "anthropic",
                    "models",
                    &format!(
                        "{model}:{}",
                        if stream {
                            "streamRawPredict"
                        } else {
                            "rawPredict"
                        }
                    ),
                ],
            )?;
            transform(&mut body, "vertex-2023-10-16")?;
            body["stream"] = Value::Bool(stream);
            let token = vertex_token(config).await?;
            header(&mut headers, "authorization", &format!("Bearer {token}"))?;
        }
        ClaudeBackend::Foundry => {
            let base = match config.base_url.as_deref() {
                Some(base) => base.to_string(),
                None => {
                    let resource = required(&config.azure_resource, "azure_resource")?;
                    if !resource
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    {
                        return Err("Invalid Azure resource name".into());
                    }
                    format!("https://{resource}.services.ai.azure.com")
                }
            };
            let base = base.trim_end_matches('/');
            target = if base.ends_with("/v1/messages") {
                endpoint(base, &[])?
            } else if base.ends_with("/anthropic") {
                endpoint(base, &["v1", "messages"])?
            } else {
                endpoint(base, &["anthropic", "v1", "messages"])?
            };
            header(&mut headers, "anthropic-version", "2023-06-01")?;
            if let Some(token) = config.auth_token.as_deref().filter(|v| !v.is_empty()) {
                header(&mut headers, "authorization", &format!("Bearer {token}"))?;
            } else if let Some(key) = config.api_key.as_deref().filter(|v| !v.is_empty()) {
                header(&mut headers, "api-key", key)?;
            } else {
                let token = tokio::time::timeout(Duration::from_secs(15), azure_token())
                    .await
                    .map_err(|_| "Azure credential resolution timed out")??;
                header(&mut headers, "authorization", &format!("Bearer {token}"))?;
            }
        }
    }
    Ok(PreparedRequest {
        url: target,
        headers,
        body_bytes: serde_json::to_vec(&body).map_err(|e| e.to_string())?,
        framing,
    })
}

fn normalize_bedrock_cache_control(body: &mut Value) {
    let normalize = |block: &mut Value| {
        if matches!(
            block.get("type").and_then(Value::as_str),
            Some("thinking" | "redacted_thinking")
        ) || block.get("signature").is_some()
        {
            return;
        }
        if let Some(cache) = block
            .get_mut("cache_control")
            .and_then(Value::as_object_mut)
        {
            cache.remove("ttl");
        }
    };
    for field in ["tools", "system"] {
        if let Some(blocks) = body.get_mut(field).and_then(Value::as_array_mut) {
            blocks.iter_mut().for_each(normalize);
        }
    }
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            if let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) {
                blocks.iter_mut().for_each(normalize);
            }
        }
    }
}

fn transform(body: &mut Value, version: &str) -> Result<(), String> {
    let object = body
        .as_object_mut()
        .ok_or("Claude request body must be an object")?;
    object.remove("model");
    object.remove("stream");
    object.insert("anthropic_version".into(), Value::String(version.into()));
    Ok(())
}

async fn aws_credentials(config: &ClaudeTransportConfig) -> Result<Credentials, String> {
    match (&config.aws_access_key_id, &config.aws_secret_access_key) {
        (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => {
            return Ok(Credentials::new(
                id,
                secret,
                config.aws_session_token.clone(),
                None,
                "refact-explicit",
            ))
        }
        (None, None) => {}
        _ => {
            return Err(
                "AWS static credentials require both access key ID and secret access key".into(),
            )
        }
    }
    let entry = AWS_CREDENTIALS.get().entry(aws_key(config)).await?;
    resolve_aws_credentials(&entry, SystemTime::now, || async {
        let sdk = aws_sdk(config).await?;
        sdk.credentials_provider()
            .ok_or("AWS credential provider unavailable")?
            .provide_credentials()
            .await
            .map_err(|_| "AWS credential acquisition failed".into())
    })
    .await
}

struct Resettable<T>(std::sync::Mutex<Option<Arc<T>>>);

impl<T: Default> Resettable<T> {
    const fn new() -> Self {
        Self(std::sync::Mutex::new(None))
    }

    fn get(&self) -> Arc<T> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(Default::default)
            .clone()
    }

    fn reset(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

static AWS_CREDENTIALS: Resettable<AwsKeyCache<Mutex<Option<CachedAwsCredentials>>>> =
    Resettable::new();
static AWS_SDK: Resettable<AwsKeyCache<OnceCell<aws_types::SdkConfig>>> = Resettable::new();
static GOOGLE_ADC: Resettable<OnceCell<google_cloud_auth::credentials::AccessTokenCredentials>> =
    Resettable::new();
static AZURE_CHAIN: Resettable<OnceCell<AzureChain>> = Resettable::new();

pub async fn reset_cached_credentials(backend: ClaudeBackend) {
    match backend {
        ClaudeBackend::Bedrock => {
            AWS_CREDENTIALS.reset();
            AWS_SDK.reset();
        }
        ClaudeBackend::Vertex => GOOGLE_ADC.reset(),
        ClaudeBackend::Foundry => AZURE_CHAIN.reset(),
        ClaudeBackend::Oauth | ClaudeBackend::Anthropic | ClaudeBackend::Custom => {}
    }
}

type AwsKey = (Option<String>, Option<String>);

fn aws_key(config: &ClaudeTransportConfig) -> AwsKey {
    (config.aws_profile.clone(), config.aws_region.clone())
}

struct AwsKeyCache<T> {
    entries: Mutex<HashMap<AwsKey, Arc<T>>>,
}

impl<T> Default for AwsKeyCache<T> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

impl<T: Default> AwsKeyCache<T> {
    async fn entry(&self, key: AwsKey) -> Result<Arc<T>, String> {
        let mut entries = self.entries.lock().await;
        if let Some(entry) = entries.get(&key) {
            return Ok(entry.clone());
        }
        if entries.len() >= 32 {
            let idle = entries
                .iter()
                .find(|(_, entry)| Arc::strong_count(entry) == 1)
                .map(|(key, _)| key.clone());
            match idle {
                Some(key) => {
                    entries.remove(&key);
                }
                None => return Err("AWS configuration cache is busy; retry later".into()),
            }
        }
        let entry = Arc::new(T::default());
        entries.insert(key, entry.clone());
        Ok(entry)
    }
}

struct CachedAwsCredentials {
    credentials: Credentials,
    refresh_at: SystemTime,
}

async fn resolve_aws_credentials<F, Fut>(
    entry: &Mutex<Option<CachedAwsCredentials>>,
    now: impl Fn() -> SystemTime,
    resolve: F,
) -> Result<Credentials, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Credentials, String>>,
{
    let mut cached = entry.lock().await;
    if let Some(value) = cached.as_ref().filter(|value| now() < value.refresh_at) {
        return Ok(value.credentials.clone());
    }
    let credentials = resolve().await?;
    let resolved_at = now();
    let fallback = resolved_at + Duration::from_secs(3600);
    let refresh_at = credentials
        .expiry()
        .map(|expiry| {
            expiry
                .checked_sub(Duration::from_secs(300))
                .unwrap_or(resolved_at)
        })
        .unwrap_or(fallback)
        .min(fallback);
    *cached = Some(CachedAwsCredentials {
        credentials: credentials.clone(),
        refresh_at,
    });
    Ok(credentials)
}

async fn aws_sdk(config: &ClaudeTransportConfig) -> Result<aws_types::SdkConfig, String> {
    let entry = AWS_SDK.get().entry(aws_key(config)).await?;
    let sdk = entry
        .get_or_init(|| async {
            let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
            if let Some(region) = &config.aws_region {
                loader = loader.region(aws_types::region::Region::new(region.clone()));
            }
            if let Some(profile) = &config.aws_profile {
                loader = loader.profile_name(profile);
            }
            loader.load().await
        })
        .await;
    Ok(sdk.clone())
}

fn sign_request(
    url: &str,
    headers: &mut HeaderMap,
    body: &[u8],
    region: &str,
    credentials: Credentials,
    time: SystemTime,
) -> Result<(), String> {
    let identity = credentials.into();
    let params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name("bedrock")
        .time(time)
        .settings(SigningSettings::default())
        .build()
        .map_err(|_| "AWS signing parameters invalid")?
        .into();
    let request = SignableRequest::new(
        "POST",
        url,
        headers
            .iter()
            .map(|(k, v)| {
                Ok((
                    k.as_str(),
                    v.to_str().map_err(|_| "Invalid signing header")?,
                ))
            })
            .collect::<Result<Vec<_>, &str>>()?
            .into_iter(),
        SignableBody::Bytes(body),
    )
    .map_err(|_| "AWS request could not be signed")?;
    let (instructions, _) = sign(request, &params)
        .map_err(|_| "AWS signing failed")?
        .into_parts();
    for (name, value) in instructions.headers() {
        header(headers, name, value)?;
    }
    Ok(())
}

async fn vertex_token(config: &ClaudeTransportConfig) -> Result<String, String> {
    if let Some(token) = config.auth_token.clone() {
        return Ok(token);
    }
    tokio::time::timeout(Duration::from_secs(15), async {
        let adc = GOOGLE_ADC.get();
        let creds = adc
            .get_or_try_init(|| async {
                google_cloud_auth::credentials::Builder::default()
                    .with_scopes(["https://www.googleapis.com/auth/cloud-platform"])
                    .build_access_token_credentials()
                    .map_err(|_| "Google ADC configuration failed")
            })
            .await?;
        creds
            .access_token()
            .await
            .map(|t| t.token)
            .map_err(|_| "Google ADC token acquisition failed")
    })
    .await
    .map_err(|_| "Google ADC resolution timed out")?
    .map_err(str::to_string)
}

const AZURE_SCOPE: &str = "https://cognitiveservices.azure.com/.default";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AzurePlan {
    ClientSecret,
    WorkloadIdentity,
    ManagedIdentity,
    ImdsThenDeveloperTools,
}

fn azure_plan(present: impl Fn(&str) -> bool) -> AzurePlan {
    if present("AZURE_TENANT_ID") && present("AZURE_CLIENT_ID") && present("AZURE_CLIENT_SECRET") {
        AzurePlan::ClientSecret
    } else if present("AZURE_FEDERATED_TOKEN_FILE") {
        AzurePlan::WorkloadIdentity
    } else if present("IDENTITY_ENDPOINT") || present("MSI_ENDPOINT") {
        AzurePlan::ManagedIdentity
    } else {
        AzurePlan::ImdsThenDeveloperTools
    }
}

type AzureFetch = Arc<
    dyn Fn() -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<azure_core::credentials::AccessToken, String>,
                    > + Send,
            >,
        > + Send
        + Sync,
>;

struct AzureSource {
    fetch: AzureFetch,
    timeout: Option<Duration>,
}

#[derive(Default)]
struct AzureChainState {
    selected: Option<usize>,
    token: Option<azure_core::credentials::AccessToken>,
}

struct AzureChain {
    sources: Vec<AzureSource>,
    state: Mutex<AzureChainState>,
}

impl AzureChain {
    fn new(sources: Vec<AzureSource>) -> Self {
        Self {
            sources,
            state: Mutex::new(AzureChainState::default()),
        }
    }

    async fn token(&self, now: azure_core::time::OffsetDateTime) -> Result<String, String> {
        let mut state = self.state.lock().await;
        if let Some(token) = state
            .token
            .as_ref()
            .filter(|t| now + azure_core::time::Duration::minutes(5) < t.expires_on)
        {
            return Ok(token.token.secret().to_string());
        }
        let indices: Vec<usize> = match state.selected {
            Some(index) => vec![index],
            None => (0..self.sources.len()).collect(),
        };
        for index in indices {
            let source = &self.sources[index];
            let result = match source.timeout {
                Some(limit) => tokio::time::timeout(limit, (source.fetch)())
                    .await
                    .unwrap_or_else(|_| Err("Azure credential source timed out".into())),
                None => (source.fetch)().await,
            };
            if let Ok(token) = result {
                let secret = token.token.secret().to_string();
                state.selected = Some(index);
                state.token = Some(token);
                return Ok(secret);
            }
        }
        state.token = None;
        Err("Azure token acquisition failed".into())
    }
}

fn azure_source(credential: Arc<dyn TokenCredential>, timeout: Option<Duration>) -> AzureSource {
    AzureSource {
        fetch: Arc::new(move || {
            let credential = credential.clone();
            Box::pin(async move {
                credential
                    .get_token(&[AZURE_SCOPE], None)
                    .await
                    .map_err(|_| "Azure token acquisition failed".to_string())
            })
        }),
        timeout,
    }
}

fn azure_chain(plan: AzurePlan) -> Result<AzureChain, String> {
    let sources = match plan {
        AzurePlan::ClientSecret => {
            let var =
                |name| std::env::var(name).map_err(|_| "Azure client secret configuration failed");
            vec![azure_source(
                azure_identity::ClientSecretCredential::new(
                    &var("AZURE_TENANT_ID")?,
                    var("AZURE_CLIENT_ID")?,
                    var("AZURE_CLIENT_SECRET")?.into(),
                    None,
                )
                .map_err(|_| "Azure client secret configuration failed")?,
                None,
            )]
        }
        AzurePlan::WorkloadIdentity => vec![azure_source(
            azure_identity::WorkloadIdentityCredential::new(None)
                .map_err(|_| "Azure workload identity configuration failed")?,
            None,
        )],
        AzurePlan::ManagedIdentity => vec![azure_source(
            azure_identity::ManagedIdentityCredential::new(None)
                .map_err(|_| "Azure managed identity configuration failed")?,
            None,
        )],
        AzurePlan::ImdsThenDeveloperTools => vec![
            azure_source(
                azure_identity::ManagedIdentityCredential::new(None)
                    .map_err(|_| "Azure managed identity configuration failed")?,
                Some(Duration::from_secs(3)),
            ),
            azure_source(
                azure_identity::DeveloperToolsCredential::new(None)
                    .map_err(|_| "Azure CLI credential configuration failed")?,
                None,
            ),
        ],
    };
    Ok(AzureChain::new(sources))
}

async fn azure_token() -> Result<String, String> {
    let slot = AZURE_CHAIN.get();
    let chain = slot
        .get_or_try_init(|| async {
            azure_chain(azure_plan(|name| {
                std::env::var_os(name).is_some_and(|v| !v.is_empty())
            }))
        })
        .await?;
    chain
        .token(azure_core::time::OffsetDateTime::now_utc())
        .await
}

#[cfg(test)]
mod tests;
