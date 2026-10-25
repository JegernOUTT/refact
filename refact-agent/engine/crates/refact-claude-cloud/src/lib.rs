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
            header(&mut headers, "anthropic-version", "2023-06-01")?;
        }
        ClaudeBackend::Custom => {
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
    static CACHE: OnceCell<AwsKeyCache<Mutex<Option<CachedAwsCredentials>>>> =
        OnceCell::const_new();
    let cache = CACHE.get_or_init(|| async { AwsKeyCache::default() }).await;
    let entry = cache.entry(aws_key(config)).await?;
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
    static CACHE: OnceCell<AwsKeyCache<OnceCell<aws_types::SdkConfig>>> = OnceCell::const_new();
    let cache = CACHE.get_or_init(|| async { AwsKeyCache::default() }).await;
    let entry = cache.entry(aws_key(config)).await?;
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
        static ADC: OnceCell<google_cloud_auth::credentials::AccessTokenCredentials> =
            OnceCell::const_new();
        let creds = ADC
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

async fn azure_token() -> Result<String, String> {
    static CREDENTIAL: tokio::sync::OnceCell<std::sync::Arc<dyn TokenCredential>> =
        tokio::sync::OnceCell::const_new();
    let credential = CREDENTIAL
        .get_or_try_init(|| async {
            let credential: std::sync::Arc<dyn TokenCredential> = if let (
                Ok(tenant),
                Ok(client),
                Ok(secret),
            ) = (
                std::env::var("AZURE_TENANT_ID"),
                std::env::var("AZURE_CLIENT_ID"),
                std::env::var("AZURE_CLIENT_SECRET"),
            ) {
                azure_identity::ClientSecretCredential::new(&tenant, client, secret.into(), None)
                    .map_err(|_| "Azure client secret configuration failed")?
            } else if std::env::var_os("AZURE_FEDERATED_TOKEN_FILE").is_some() {
                azure_identity::WorkloadIdentityCredential::new(None)
                    .map_err(|_| "Azure workload identity configuration failed")?
            } else {
                azure_identity::ManagedIdentityCredential::new(None)
                    .map_err(|_| "Azure managed identity configuration failed")?
            };
            Ok::<_, String>(credential)
        })
        .await?;
    credential
        .get_token(&["https://cognitiveservices.azure.com/.default"], None)
        .await
        .map(|token| token.token.secret().to_string())
        .map_err(|_| "Azure token acquisition failed".into())
}

#[cfg(test)]
mod tests;
