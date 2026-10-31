use super::*;
use serde_json::json;

const LIMIT: usize = 1024 * 1024;
const CALL_TIMEOUT: Duration = Duration::from_secs(20);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(60);

fn validate(config: &ClaudeTransportConfig) -> Result<(), String> {
    if config.backend != ClaudeBackend::Bedrock
        || config.base_url.is_some()
        || config.auth_token.is_some()
    {
        return Err("Bedrock setup requires AWS credentials and the default AWS endpoint".into());
    }
    Ok(())
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(CALL_TIMEOUT)
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| "AWS HTTP client unavailable".into())
}

async fn resolve(config: &ClaudeTransportConfig) -> Result<(String, Credentials), String> {
    validate(config)?;
    // Reload profile files rather than letting an earlier wizard/runtime resolution mask changes.
    reset_cached_credentials(ClaudeBackend::Bedrock).await;
    tokio::time::timeout(Duration::from_secs(15), async {
        let region = match config.aws_region.as_deref().filter(|v| !v.is_empty()) {
            Some(region) => region.to_string(),
            None => aws_sdk(config)
                .await?
                .region()
                .map(|v| v.as_ref().to_string())
                .ok_or("AWS region unavailable")?,
        };
        if region.len() > 64
            || region.is_empty()
            || !region
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err("Invalid AWS region".into());
        }
        let credentials = aws_credentials(config)
            .await
            .map_err(|_| "AWS credential acquisition failed")?;
        Ok((region, credentials))
    })
    .await
    .map_err(|_| "AWS credential resolution timed out".to_string())?
}

fn service_url(service: &str, region: &str) -> Result<String, String> {
    let suffix = if region.starts_with("cn-") {
        "amazonaws.com.cn"
    } else if region.starts_with("us-iso") || region.starts_with("eu-isoe-") {
        return Err("Bedrock setup is unsupported in this AWS partition".into());
    } else {
        "amazonaws.com"
    };
    Ok(format!("https://{service}.{region}.{suffix}"))
}

async fn send(
    client: &reqwest::Client,
    method: reqwest::Method,
    service: &str,
    url: &str,
    body: Vec<u8>,
    region: &str,
    credentials: &Credentials,
) -> Result<(u16, Vec<u8>), String> {
    let mut headers = HeaderMap::new();
    header(
        &mut headers,
        "content-type",
        if service == "sts" {
            "application/x-www-form-urlencoded"
        } else {
            "application/json"
        },
    )?;
    sign_service_request(
        method.as_str(),
        service,
        url,
        &mut headers,
        &body,
        region,
        credentials.clone(),
        SystemTime::now(),
    )?;
    let mut response = client
        .request(method, url)
        .headers(headers)
        .body(body)
        .send()
        .await
        .map_err(|_| "AWS request failed or timed out")?;
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "AWS response read failed")?
    {
        if bytes.len().saturating_add(chunk.len()) > LIMIT {
            return Err("AWS response exceeded size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((status, bytes))
}

fn safe_status(status: u16) -> &'static str {
    match status {
        200..=299 => "available: model invocation succeeded",
        401 => "credentials: AWS credentials were rejected",
        403 => "access_denied: check IAM permissions and Anthropic model access",
        404 => "not_found: model or inference profile is unavailable in this region",
        408 | 504 => "timeout: AWS request timed out",
        429 => "throttled: AWS rate or quota limit reached",
        400 | 422 => "invalid_request: check model ID, region, inference profile and model access",
        500..=599 => "service_error: AWS service is temporarily unavailable",
        _ => "request_failed: AWS request did not succeed",
    }
}

fn parse_identity(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > LIMIT {
        return Err("Invalid STS identity response".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "Invalid STS identity response")?;
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            ..Default::default()
        },
    )
    .map_err(|_| "Invalid STS identity response")?;
    let root = doc.root_element();
    let namespace = root.tag_name().namespace();
    if root.tag_name().name() != "GetCallerIdentityResponse"
        || !matches!(
            namespace,
            None | Some("https://sts.amazonaws.com/doc/2011-06-15/")
        )
    {
        return Err("Invalid STS identity response".into());
    }
    fn child<'a, 'input>(
        parent: roxmltree::Node<'a, 'input>,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<roxmltree::Node<'a, 'input>, &'static str> {
        let mut matches = parent.children().filter(|node| {
            node.is_element()
                && node.tag_name().name() == name
                && node.tag_name().namespace() == namespace
        });
        let result = matches.next().ok_or("Invalid STS identity response")?;
        if matches.next().is_some() {
            return Err("Invalid STS identity response");
        }
        Ok(result)
    }
    let result = child(root, "GetCallerIdentityResult", namespace)?;
    let field = |name: &str| -> Result<String, String> {
        let node = child(result, name, namespace)?;
        if node.children().any(|child| child.is_element()) {
            return Err("Invalid STS identity response".into());
        }
        let value: String = node
            .children()
            .filter(roxmltree::Node::is_text)
            .filter_map(|child| child.text())
            .collect();
        if value.is_empty() || value.len() > 2048 || value.chars().any(char::is_control) {
            return Err("Invalid STS identity response".into());
        }
        Ok(value)
    };
    let account = field("Account")?;
    let arn = field("Arn")?;
    let user_id = field("UserId")?;
    if account.len() != 12
        || !account.bytes().all(|b| b.is_ascii_digit())
        || !arn.starts_with("arn:")
    {
        return Err("Invalid STS identity response".into());
    }
    Ok(json!({"account":account,"arn":arn,"user_id":user_id}))
}

async fn identity(
    client: &reqwest::Client,
    region: &str,
    creds: &Credentials,
) -> Result<Value, String> {
    let (status, body) = send(
        client,
        reqwest::Method::POST,
        "sts",
        &service_url("sts", region)?,
        b"Action=GetCallerIdentity&Version=2011-06-15".to_vec(),
        region,
        creds,
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(safe_status(status).into());
    }
    parse_identity(&body)
}

pub async fn verify_identity(config: &ClaudeTransportConfig) -> Result<Value, String> {
    tokio::time::timeout(TOTAL_TIMEOUT, async {
        let (region, creds) = resolve(config).await?;
        identity(&client()?, &region, &creds).await
    })
    .await
    .map_err(|_| "AWS identity verification timed out".to_string())?
}

fn profile_page(page: &Value, profiles: &mut Vec<Value>) -> Result<Option<String>, String> {
    let entries = page
        .get("inferenceProfileSummaries")
        .and_then(Value::as_array)
        .ok_or("Invalid Bedrock profile discovery response")?;
    for entry in entries {
        let models = entry
            .get("models")
            .and_then(Value::as_array)
            .ok_or("Invalid Bedrock profile discovery response")?;
        let model_ids: Vec<_> = models
            .iter()
            .filter_map(|m| m.get("modelArn").and_then(Value::as_str))
            .filter_map(|arn| {
                arn.split_once(":foundation-model/")
                    .map(|(_, id)| id.to_string())
            })
            .collect();
        if !model_ids.iter().any(|id| id.starts_with("anthropic.")) {
            continue;
        }
        let field = |key| {
            entry
                .get(key)
                .and_then(Value::as_str)
                .filter(|v| v.len() <= 2048)
                .ok_or("Invalid Bedrock profile discovery response")
        };
        profiles.push(
            json!({"id":field("inferenceProfileId")?,"arn":field("inferenceProfileArn")?,
            "name":field("inferenceProfileName")?,"status":field("status")?,"model_ids":model_ids}),
        );
        if profiles.len() > 2000 {
            return Err("Bedrock discovery exceeded profile limit".into());
        }
    }
    match page.get("nextToken") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(token)) if token.is_empty() => Ok(None),
        Some(Value::String(token)) if token.len() <= 4096 => Ok(Some(token.clone())),
        _ => Err("Invalid Bedrock pagination token".into()),
    }
}

pub async fn verify(config: &ClaudeTransportConfig) -> Result<Value, String> {
    tokio::time::timeout(TOTAL_TIMEOUT, async {
        let (region, creds) = resolve(config).await?;
        let client = client()?;
        let identity = identity(&client, &region, &creds).await?;
        let mut profiles = Vec::new();
        let mut warnings = Vec::new();
        let mut token: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        for index in 0..20 {
            let mut url = url::Url::parse(&format!("{}/inference-profiles", service_url("bedrock", &region)?))
                .map_err(|_| "Invalid AWS endpoint")?;
            url.query_pairs_mut().append_pair("maxResults", "100");
            if let Some(token) = &token { url.query_pairs_mut().append_pair("nextToken", token); }
            let (status, bytes) = send(&client, reqwest::Method::GET, "bedrock", url.as_str(), Vec::new(), &region, &creds).await?;
            if status == 403 {
                warnings.push("Inference profile discovery denied; availability is unknown. Enter a custom model ID or ARN and explicitly probe it.");
                break;
            }
            if !(200..300).contains(&status) { return Err(safe_status(status).into()); }
            let page: Value = serde_json::from_slice(&bytes).map_err(|_| "Invalid Bedrock profile discovery response")?;
            token = profile_page(&page, &mut profiles)?;
            match &token {
                None => break,
                Some(token) if !seen.insert(token.clone()) || index == 19 => {
                    warnings.push("Inference profile discovery incomplete; pagination limit reached. Custom IDs can still be probed.");
                    break;
                }
                _ => {}
            }
        }
        Ok(json!({"identity":identity,"inference_profiles":profiles,"warnings":warnings}))
    }).await.map_err(|_| "AWS setup verification timed out".to_string())?
}

// Only directly identifiable documented models; opaque application ARNs stay unknown.
pub fn supports_1m(model_id: &str) -> bool {
    native_1m(model_id) || requires_1m_beta(model_id)
}

fn native_1m(model_id: &str) -> bool {
    let id = model_id
        .rsplit(":foundation-model/")
        .next()
        .unwrap_or(model_id);
    let id = ["us.", "eu.", "apac.", "global.", "jp.", "au.", "us-gov."]
        .iter()
        .find_map(|prefix| id.strip_prefix(prefix))
        .unwrap_or(id);
    matches!(
        id,
        "anthropic.claude-opus-4-7"
            | "anthropic.claude-opus-4-8"
            | "anthropic.claude-opus-5"
            | "anthropic.claude-opus-5-5"
            | "anthropic.claude-sonnet-5"
    )
}

pub(crate) fn requires_1m_beta(model_id: &str) -> bool {
    let id = model_id
        .rsplit(":foundation-model/")
        .next()
        .unwrap_or(model_id);
    let id = ["us.", "eu.", "apac.", "global.", "jp.", "au.", "us-gov."]
        .iter()
        .find_map(|prefix| id.strip_prefix(prefix))
        .unwrap_or(id);
    matches!(
        id,
        "anthropic.claude-opus-4-6" | "anthropic.claude-sonnet-4-6"
    )
}

pub async fn probe(
    config: &ClaudeTransportConfig,
    model_id: &str,
    context_1m: bool,
) -> Result<(u16, String), String> {
    probe_inner(config, model_id, context_1m, None).await
}

pub async fn probe_verified(
    config: &ClaudeTransportConfig,
    model_id: &str,
    context_1m: bool,
    expected_identity: &Value,
) -> Result<(u16, String), String> {
    probe_inner(config, model_id, context_1m, Some(expected_identity)).await
}

async fn probe_inner(
    config: &ClaudeTransportConfig,
    model_id: &str,
    context_1m: bool,
    expected: Option<&Value>,
) -> Result<(u16, String), String> {
    validate(config)?;
    if model_id.is_empty() || model_id.len() > 2048 || model_id.contains(['[', ']']) {
        return Err("Invalid Bedrock model ID; do not include context suffixes".into());
    }
    if context_1m && !supports_1m(model_id) {
        return Err("unsupported_context: 1M context is not established for this model ID".into());
    }
    tokio::time::timeout(TOTAL_TIMEOUT, async {
        let (region, creds) = resolve(config).await?;
        let client = client()?;
        if let Some(expected) = expected {
            let current = identity(&client, &region, &creds).await?;
            require_identity(&current, expected)?;
        }
        // The same resolved credential snapshot is used for identity and the billable request.
        let mut held = config.clone();
        held.aws_region = Some(region);
        held.bedrock_context_1m_models.retain(|id| id != model_id);
        if context_1m {
            held.bedrock_context_1m_models.push(model_id.to_string());
        }
        held.aws_access_key_id = Some(creds.access_key_id().into());
        held.aws_secret_access_key = Some(creds.secret_access_key().into());
        held.aws_session_token = creds.session_token().map(str::to_string);
        let prepared = prepare_attempt(
            String::new(),
            HeaderMap::new(),
            json!({"max_tokens":1,"messages":[{"role":"user","content":"Hi"}]}),
            model_id,
            false,
            &held,
        )
        .await
        .map_err(|_| "Bedrock probe request preparation failed")?;
        execute_probe(&client, prepared).await
    })
    .await
    .map_err(|_| "Bedrock probe timed out".to_string())?
}

fn require_identity(current: &Value, expected: &Value) -> Result<(), String> {
    if current != expected {
        return Err("AWS identity changed; verify setup again".into());
    }
    Ok(())
}

async fn execute_probe(
    client: &reqwest::Client,
    prepared: PreparedRequest,
) -> Result<(u16, String), String> {
    let mut response = client
        .post(prepared.url)
        .headers(prepared.headers)
        .body(prepared.body_bytes)
        .send()
        .await
        .map_err(|_| "Bedrock probe failed or timed out")?;
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Bedrock probe response read failed")?
    {
        if bytes.len().saturating_add(chunk.len()) > LIMIT {
            return Err("Bedrock probe response exceeded size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if (200..300).contains(&status) {
        let valid = status == 200
            && serde_json::from_slice::<Value>(&bytes).is_ok_and(|body| {
                body.get("type").and_then(Value::as_str) == Some("message")
                    && body.get("content").is_some_and(Value::is_array)
                    && body.get("usage").is_some_and(Value::is_object)
            });
        if !valid {
            return Err("Invalid Bedrock probe response".into());
        }
    }
    Ok((status, safe_status(status).to_string()))
}

#[cfg(test)]
mod tests;
