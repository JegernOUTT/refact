use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use refact_integrations::browser_models::{BrowserCookie, BrowserCookieSameSite};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE, COOKIE, LOCATION};
use reqwest::{Method, StatusCode};
use serde_json::Value;
use url::Url;

use crate::network::{mask_headers, mask_text};

pub const HTTP_INLINE_BODY_LIMIT_BYTES: usize = 8 * 1024;
pub const DEFAULT_HTTP_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_HTTP_MAX_REDIRECTS: u32 = 20;

const SUMMARY_RESPONSE_HEADERS: [&str; 2] = ["content-type", "content-length"];
const CROSS_ORIGIN_STRIPPED_HEADERS: [&str; 2] = ["authorization", "cookie"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequestBody {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

#[derive(Clone, Debug)]
pub struct HttpRequestSpec {
    pub url: Url,
    pub method: Method,
    pub headers: BTreeMap<String, String>,
    pub body: Option<HttpRequestBody>,
    pub timeout: Duration,
    pub max_redirects: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub status_text: String,
    pub final_url: String,
    pub method: String,
    pub redirects: u32,
    pub headers: BTreeMap<String, String>,
    pub set_cookies: Vec<BrowserCookie>,
    pub rejected_cookies: Vec<String>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetCookieScan {
    pub accepted: Vec<BrowserCookie>,
    pub rejected: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpResponseBody {
    Empty,
    Inline(String),
    Artifact,
}

pub fn parse_http_url(url: &str) -> Result<Url, String> {
    let parsed = Url::parse(url)
        .map_err(|error| format!("Invalid URL: {}", mask_text(&error.to_string())))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "Unsupported URL scheme '{}': http_request allows http and https only",
            parsed.scheme()
        ));
    }
    if parsed.host_str().is_none() {
        return Err("URL must include a host".to_string());
    }
    Ok(parsed)
}

pub fn parse_http_method(method: Option<&str>) -> Result<Method, String> {
    let Some(method) = method else {
        return Ok(Method::GET);
    };
    Method::from_bytes(method.trim().to_ascii_uppercase().as_bytes())
        .map_err(|_| format!("Invalid HTTP method '{}'", mask_text(method)))
}

pub fn build_request_body(
    body: Option<&str>,
    body_json: Option<&Value>,
    form: Option<&BTreeMap<String, String>>,
) -> Result<Option<HttpRequestBody>, String> {
    let provided = [body.is_some(), body_json.is_some(), form.is_some()]
        .into_iter()
        .filter(|present| *present)
        .count();
    if provided > 1 {
        return Err("http_request accepts only one of body, body_json, or form".to_string());
    }
    if let Some(body) = body {
        return Ok(Some(HttpRequestBody {
            bytes: body.as_bytes().to_vec(),
            content_type: None,
        }));
    }
    if let Some(body_json) = body_json {
        return Ok(Some(HttpRequestBody {
            bytes: serde_json::to_vec(body_json)
                .map_err(|error| format!("Failed to serialize body_json: {error}"))?,
            content_type: Some("application/json".to_string()),
        }));
    }
    if let Some(form) = form {
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form.iter())
            .finish();
        return Ok(Some(HttpRequestBody {
            bytes: encoded.into_bytes(),
            content_type: Some("application/x-www-form-urlencoded".to_string()),
        }));
    }
    Ok(None)
}

pub fn cookie_header(cookies: &[BrowserCookie], url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let secure_transport = url.scheme() == "https" || is_local_host(host);
    let pairs = cookies
        .iter()
        .filter(|cookie| !cookie.secure || secure_transport)
        .filter(|cookie| domain_matches(host, &cookie.domain))
        .filter(|cookie| path_matches(url.path(), &cookie.path))
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>();
    (!pairs.is_empty()).then(|| pairs.join("; "))
}

pub fn parse_set_cookies(url: &Url, headers: &[String]) -> SetCookieScan {
    let Some(host) = url.host_str() else {
        return SetCookieScan::default();
    };
    let mut scan = SetCookieScan::default();
    for header in headers {
        match parse_set_cookie(url, host, header) {
            Some(Ok(cookie)) => scan.accepted.push(cookie),
            Some(Err(reason)) => scan.rejected.push(reason),
            None => {}
        }
    }
    scan
}

pub fn remaining_budget(deadline: Instant, timeout: Duration) -> Result<Duration, String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(format!(
            "HTTP request timed out after {}ms while following redirects",
            timeout.as_millis()
        ));
    }
    Ok(remaining)
}

pub fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

pub fn headers_for_hop(
    headers: &BTreeMap<String, String>,
    origin: &Url,
    target: &Url,
) -> BTreeMap<String, String> {
    if same_origin(origin, target) {
        return headers.clone();
    }
    headers
        .iter()
        .filter(|(name, _)| {
            !CROSS_ORIGIN_STRIPPED_HEADERS
                .iter()
                .any(|stripped| name.eq_ignore_ascii_case(stripped))
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

pub fn summarize_response_headers(
    headers: &BTreeMap<String, String>,
    full: bool,
) -> BTreeMap<String, String> {
    let masked = mask_headers(headers.clone());
    if full {
        return masked;
    }
    masked
        .into_iter()
        .filter(|(name, _)| SUMMARY_RESPONSE_HEADERS.contains(&name.to_ascii_lowercase().as_str()))
        .collect()
}

pub fn split_response_body(bytes: &[u8], content_type: Option<&str>) -> HttpResponseBody {
    if bytes.is_empty() {
        return HttpResponseBody::Empty;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return HttpResponseBody::Artifact;
    };
    let rendered = content_type
        .filter(|content_type| content_type.to_ascii_lowercase().contains("json"))
        .and_then(|_| serde_json::from_str::<Value>(text).ok())
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| text.to_string());
    let redacted = mask_text(&rendered);
    if redacted.len() > HTTP_INLINE_BODY_LIMIT_BYTES {
        HttpResponseBody::Artifact
    } else {
        HttpResponseBody::Inline(redacted)
    }
}

pub fn response_body_extension(content_type: Option<&str>) -> &'static str {
    let lowered = content_type.unwrap_or_default().to_ascii_lowercase();
    if lowered.contains("json") {
        "json"
    } else if lowered.starts_with("text/") || lowered.contains("xml") || lowered.contains("html") {
        "txt"
    } else {
        "bin"
    }
}

pub fn save_response_body(
    bytes: &[u8],
    artifacts_dir: &Path,
    file_name: &str,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(artifacts_dir).map_err(|error| {
        format!(
            "Failed to create browser artifacts directory {}: {error}",
            artifacts_dir.display()
        )
    })?;
    let path = artifacts_dir.join(file_name);
    std::fs::write(&path, bytes).map_err(|error| {
        format!(
            "Failed to save HTTP response artifact {}: {error}",
            path.display()
        )
    })?;
    Ok(path)
}

pub async fn send_http_request(
    spec: &HttpRequestSpec,
    jar: &[BrowserCookie],
) -> Result<HttpResponse, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| {
            format!(
                "Failed to build HTTP client: {}",
                mask_text(&error.to_string())
            )
        })?;
    let deadline = Instant::now() + spec.timeout;
    let mut url = spec.url.clone();
    let mut method = spec.method.clone();
    let mut body = spec.body.clone();
    let mut set_cookies: Vec<BrowserCookie> = Vec::new();
    let mut rejected_cookies: Vec<String> = Vec::new();
    let mut redirects = 0u32;
    loop {
        let mut headers = header_map(&headers_for_hop(&spec.headers, &spec.url, &url))?;
        if let Some(content_type) = body.as_ref().and_then(|body| body.content_type.as_deref()) {
            if !headers.contains_key(CONTENT_TYPE) {
                headers.insert(
                    CONTENT_TYPE,
                    HeaderValue::from_str(content_type)
                        .map_err(|_| format!("Invalid content type '{content_type}'"))?,
                );
            }
        }
        if !headers.contains_key(COOKIE) {
            let mut available = jar.to_vec();
            available.extend(set_cookies.iter().cloned());
            if let Some(value) = cookie_header(&available, &url) {
                headers.insert(
                    COOKIE,
                    HeaderValue::from_str(&value)
                        .map_err(|_| "Cookie jar contains a value that is not a valid header")?,
                );
            }
        }
        let mut request = client
            .request(method.clone(), url.clone())
            .timeout(remaining_budget(deadline, spec.timeout)?)
            .headers(headers);
        if let Some(body) = &body {
            request = request.body(body.bytes.clone());
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("HTTP request failed: {}", mask_text(&error.to_string())))?;
        let status = response.status();
        let headers = header_btree(response.headers());
        let scan = parse_set_cookies(&url, &raw_set_cookies(response.headers()));
        set_cookies.extend(scan.accepted);
        rejected_cookies.extend(scan.rejected);
        if let Some(location) = redirect_location(status, response.headers(), &url) {
            if redirects >= spec.max_redirects {
                return Err(format!(
                    "Exceeded max_redirects ({}) while following {}",
                    spec.max_redirects,
                    mask_text(url.as_str())
                ));
            }
            redirects += 1;
            (method, body) = follow_redirect(method, status, body);
            url = location;
            continue;
        }
        let final_url = response.url().to_string();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| {
                format!(
                    "Failed to read HTTP response body: {}",
                    mask_text(&error.to_string())
                )
            })?
            .to_vec();
        return Ok(HttpResponse {
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or_default().to_string(),
            final_url,
            method: method.to_string(),
            redirects,
            headers,
            set_cookies,
            rejected_cookies,
            body: bytes,
        });
    }
}

fn header_map(headers: &BTreeMap<String, String>) -> Result<HeaderMap, String> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("Invalid header name '{}'", mask_text(name)))?;
        let value = HeaderValue::from_str(value)
            .map_err(|_| format!("Invalid value for header '{name}'"))?;
        map.insert(name, value);
    }
    Ok(map)
}

fn header_btree(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), value.to_string()))
        })
        .collect()
}

fn raw_set_cookies(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_string))
        .collect()
}

fn redirect_location(status: StatusCode, headers: &HeaderMap, base: &Url) -> Option<Url> {
    if !matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let location = headers.get(LOCATION)?.to_str().ok()?;
    base.join(location).ok()
}

fn follow_redirect(
    method: Method,
    status: StatusCode,
    body: Option<HttpRequestBody>,
) -> (Method, Option<HttpRequestBody>) {
    let downgrade = status.as_u16() == 303
        || (matches!(status.as_u16(), 301 | 302) && !matches!(method, Method::GET | Method::HEAD));
    if downgrade {
        (Method::GET, None)
    } else {
        (method, body)
    }
}

fn is_local_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost") || host == "127.0.0.1" || host == "[::1]"
}

fn domain_matches(host: &str, domain: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let domain = domain.to_ascii_lowercase();
    if host == domain {
        return true;
    }
    if !domain.starts_with('.') {
        return false;
    }
    format!(".{host}").ends_with(&domain)
}

fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    if request_path == cookie_path {
        return true;
    }
    let request_path = if request_path.ends_with('/') {
        request_path.to_string()
    } else {
        format!("{request_path}/")
    };
    let cookie_path = if cookie_path.ends_with('/') {
        cookie_path.to_string()
    } else {
        format!("{cookie_path}/")
    };
    request_path.starts_with(&cookie_path)
}

fn default_cookie_path(url: &Url) -> String {
    let segments = url
        .path()
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    format!(
        "/{}",
        segments[..segments.len().saturating_sub(1)].join("/")
    )
}

fn is_public_suffix(domain: &str) -> bool {
    domain
        .trim_start_matches('.')
        .split('.')
        .filter(|label| !label.is_empty())
        .count()
        < 2
}

fn set_cookie_domain_rejection(host: &str, name: &str, domain: &str) -> Option<String> {
    let host = host.to_ascii_lowercase();
    if domain.trim_start_matches('.') == host {
        return None;
    }
    if is_public_suffix(domain) {
        return Some(mask_text(&format!(
            "{name}: Domain={domain} is a public suffix"
        )));
    }
    if !domain_matches(&host, domain) {
        return Some(mask_text(&format!(
            "{name}: Domain={domain} does not domain-match {host}"
        )));
    }
    None
}

fn parse_set_cookie(url: &Url, host: &str, header: &str) -> Option<Result<BrowserCookie, String>> {
    let mut attributes = header.split(';').filter(|part| !part.trim().is_empty());
    let pair = attributes.next()?;
    if !pair.contains('=') {
        return None;
    }
    let (name, value) = split_cookie_pair(pair);
    if name.is_empty() {
        return None;
    }
    let mut cookie = BrowserCookie {
        name,
        value,
        domain: String::new(),
        path: String::new(),
        expires: None,
        http_only: false,
        secure: false,
        same_site: None,
        url: None,
    };
    for attribute in attributes {
        let (key, value) = split_cookie_pair(attribute);
        match key.to_ascii_lowercase().as_str() {
            "domain" => {
                let domain = value.to_ascii_lowercase();
                cookie.domain = if !domain.starts_with('.') && domain.contains('.') {
                    format!(".{domain}")
                } else {
                    domain
                };
            }
            "path" => cookie.path = value,
            "secure" => cookie.secure = true,
            "httponly" => cookie.http_only = true,
            "max-age" => {
                if let Ok(seconds) = value.parse::<i64>() {
                    cookie.expires = Some(if seconds <= 0 {
                        0.0
                    } else {
                        Utc::now().timestamp() as f64 + seconds as f64
                    });
                }
            }
            "expires" => {
                if cookie.expires.is_none() {
                    cookie.expires = parse_cookie_expires(&value);
                }
            }
            "samesite" => {
                cookie.same_site = match value.to_ascii_lowercase().as_str() {
                    "strict" => Some(BrowserCookieSameSite::Strict),
                    "lax" => Some(BrowserCookieSameSite::Lax),
                    "none" => Some(BrowserCookieSameSite::None),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    if cookie.domain.is_empty() {
        cookie.domain = host.to_ascii_lowercase();
    } else if let Some(reason) = set_cookie_domain_rejection(host, &cookie.name, &cookie.domain) {
        return Some(Err(reason));
    }
    if !cookie.path.starts_with('/') {
        cookie.path = default_cookie_path(url);
    }
    Some(Ok(cookie))
}

fn split_cookie_pair(part: &str) -> (String, String) {
    match part.find('=') {
        Some(position) => (
            part[..position].trim().to_string(),
            part[position + 1..].trim().to_string(),
        ),
        None => (part.trim().to_string(), String::new()),
    }
}

fn parse_cookie_expires(value: &str) -> Option<f64> {
    DateTime::parse_from_rfc2822(value)
        .map(|parsed| parsed.timestamp() as f64)
        .or_else(|_| {
            DateTime::parse_from_str(value, "%a, %d-%b-%Y %H:%M:%S %Z")
                .map(|parsed| parsed.timestamp() as f64)
        })
        .or_else(|_| DateTime::parse_from_rfc3339(value).map(|parsed| parsed.timestamp() as f64))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_redirect_chain_shares_one_timeout_budget() {
        let timeout = Duration::from_millis(300);
        let deadline = Instant::now() + timeout;
        let first = remaining_budget(deadline, timeout).unwrap();
        assert!(
            first <= timeout && first > Duration::from_millis(250),
            "{first:?}"
        );

        let spent = Instant::now() - Duration::from_millis(200) + timeout;
        let second = remaining_budget(spent, timeout).unwrap();
        assert!(second < first, "later hops must get a smaller budget");

        let error =
            remaining_budget(Instant::now() - Duration::from_millis(1), timeout).unwrap_err();
        assert_eq!(
            error,
            "HTTP request timed out after 300ms while following redirects"
        );
    }

    fn cookie(name: &str, value: &str, domain: &str, path: &str, secure: bool) -> BrowserCookie {
        BrowserCookie {
            name: name.to_string(),
            value: value.to_string(),
            domain: domain.to_string(),
            path: path.to_string(),
            expires: None,
            http_only: false,
            secure,
            same_site: None,
            url: None,
        }
    }

    fn jar() -> Vec<BrowserCookie> {
        vec![
            cookie("session", "abc123", "api.example.test", "/", false),
            cookie("wide", "shared", ".example.test", "/", false),
            cookie("scoped", "deep", "api.example.test", "/admin", false),
            cookie("tls_only", "https", "api.example.test", "/", true),
            cookie("other", "nope", "other.test", "/", false),
        ]
    }

    #[test]
    fn cookie_header_only_carries_cookies_matching_the_request_domain_path_and_scheme() {
        let header = cookie_header(
            &jar(),
            &Url::parse("https://api.example.test/v1/me").unwrap(),
        );
        assert_eq!(
            header.as_deref(),
            Some("session=abc123; wide=shared; tls_only=https")
        );

        let insecure = cookie_header(
            &jar(),
            &Url::parse("http://api.example.test/v1/me").unwrap(),
        )
        .unwrap();
        assert!(!insecure.contains("tls_only"));

        let deep = cookie_header(
            &jar(),
            &Url::parse("https://api.example.test/admin/x").unwrap(),
        )
        .unwrap();
        assert!(deep.contains("scoped=deep"));

        let foreign = cookie_header(&jar(), &Url::parse("https://elsewhere.test/").unwrap());
        assert_eq!(foreign, None);
    }

    #[test]
    fn cookie_header_never_dumps_the_whole_jar() {
        let header = cookie_header(&jar(), &Url::parse("https://api.example.test/v1").unwrap())
            .unwrap_or_default();
        assert!(!header.contains("other=nope"));
        assert!(!header.contains("scoped=deep"));
    }

    #[test]
    fn set_cookie_writeback_parses_attributes_and_defaults_domain_and_path() {
        let url = Url::parse("https://api.example.test/v1/login").unwrap();
        let parsed = parse_set_cookies(
            &url,
            &[
                "sid=xyz; Path=/; HttpOnly; Secure; SameSite=Lax".to_string(),
                "pref=dark".to_string(),
                "wide=1; Domain=example.test".to_string(),
                "evil=1; Domain=attacker.test".to_string(),
                "; broken".to_string(),
            ],
        );

        let rejected = parsed.rejected;
        let parsed = parsed.accepted;
        assert_eq!(parsed.len(), 3);
        assert_eq!(rejected.len(), 1);
        assert!(rejected[0].contains("evil"), "{rejected:?}");
        assert_eq!(parsed[0].name, "sid");
        assert_eq!(parsed[0].value, "xyz");
        assert_eq!(parsed[0].domain, "api.example.test");
        assert_eq!(parsed[0].path, "/");
        assert!(parsed[0].http_only && parsed[0].secure);
        assert_eq!(parsed[0].same_site, Some(BrowserCookieSameSite::Lax));
        assert_eq!(parsed[1].path, "/v1");
        assert_eq!(parsed[2].domain, ".example.test");
    }

    #[test]
    fn set_cookie_max_age_zero_expires_immediately_and_positive_max_age_is_absolute() {
        let url = Url::parse("https://api.example.test/").unwrap();
        let parsed = parse_set_cookies(
            &url,
            &[
                "gone=1; Max-Age=0".to_string(),
                "stay=1; Max-Age=600".to_string(),
                "dated=1; Expires=Wed, 21 Oct 2015 07:28:00 GMT".to_string(),
            ],
        );

        let parsed = parsed.accepted;
        assert_eq!(parsed[0].expires, Some(0.0));
        assert!(parsed[1].expires.unwrap() > Utc::now().timestamp() as f64);
        assert_eq!(parsed[2].expires, Some(1_445_412_480.0));
    }

    #[test]
    fn round_trip_cookies_flow_from_the_jar_into_the_header_and_back_from_set_cookie() {
        let url = Url::parse("https://api.example.test/v1/session").unwrap();
        let header = cookie_header(&jar(), &url).unwrap();
        assert!(header.contains("session=abc123"));

        let refreshed = parse_set_cookies(&url, &["session=rotated; Path=/".to_string()]).accepted;
        let merged = [refreshed.clone(), jar()].concat();
        let next = cookie_header(&merged, &url).unwrap();
        assert!(next.starts_with("session=rotated"));
    }

    #[test]
    fn set_cookie_domain_must_domain_match_and_may_not_be_a_public_suffix() {
        let url = Url::parse("https://api.example.test/v1").unwrap();
        let scan = parse_set_cookies(
            &url,
            &[
                "host_only=1".to_string(),
                "parent=1; Domain=.example.test".to_string(),
                "exact=1; Domain=api.example.test".to_string(),
                "sibling=1; Domain=other.example.test".to_string(),
                "foreign=1; Domain=evil.test".to_string(),
                "tld=1; Domain=.test".to_string(),
            ],
        );

        let accepted = scan
            .accepted
            .iter()
            .map(|cookie| (cookie.name.as_str(), cookie.domain.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            accepted,
            vec![
                ("host_only", "api.example.test"),
                ("parent", ".example.test"),
                ("exact", ".api.example.test"),
            ]
        );

        assert_eq!(scan.rejected.len(), 3);
        assert!(scan.rejected[0].contains("sibling"), "{:?}", scan.rejected);
        assert!(scan.rejected[1].contains("foreign"), "{:?}", scan.rejected);
        assert!(
            scan.rejected[2].contains("public suffix"),
            "{:?}",
            scan.rejected
        );
    }

    #[test]
    fn a_public_suffix_domain_identical_to_the_request_host_stays_host_only() {
        let scan = parse_set_cookies(
            &Url::parse("http://localhost:8080/app").unwrap(),
            &["dev=1; Domain=localhost".to_string()],
        );

        assert_eq!(scan.rejected, Vec::<String>::new());
        assert_eq!(scan.accepted[0].domain, "localhost");
    }

    #[test]
    fn cross_origin_redirects_drop_authorization_and_cookie_headers() {
        let headers = BTreeMap::from([
            ("Authorization".to_string(), "Bearer secret".to_string()),
            ("Cookie".to_string(), "session=secret".to_string()),
            ("Accept".to_string(), "application/json".to_string()),
        ]);
        let origin = Url::parse("https://api.example.test/v1").unwrap();

        let same = headers_for_hop(
            &headers,
            &origin,
            &Url::parse("https://api.example.test/v2").unwrap(),
        );
        assert_eq!(same, headers);

        for target in [
            "https://evil.test/v1",
            "http://api.example.test/v1",
            "https://api.example.test:8443/v1",
            "https://other.example.test/v1",
        ] {
            let stripped = headers_for_hop(&headers, &origin, &Url::parse(target).unwrap());
            assert_eq!(
                stripped,
                BTreeMap::from([("Accept".to_string(), "application/json".to_string())]),
                "{target}"
            );
        }
    }

    #[test]
    fn header_stripping_is_case_insensitive_and_default_ports_stay_same_origin() {
        let headers = BTreeMap::from([
            ("authorization".to_string(), "Bearer secret".to_string()),
            ("cOOkie".to_string(), "session=secret".to_string()),
        ]);
        let origin = Url::parse("https://api.example.test/v1").unwrap();

        assert!(headers_for_hop(
            &headers,
            &origin,
            &Url::parse("https://evil.test/").unwrap()
        )
        .is_empty());
        assert_eq!(
            headers_for_hop(
                &headers,
                &origin,
                &Url::parse("https://api.example.test:443/other").unwrap()
            ),
            headers
        );
    }

    #[test]
    fn jar_cookies_are_rescoped_to_the_redirect_target() {
        let target = Url::parse("https://evil.test/").unwrap();
        assert_eq!(cookie_header(&jar(), &target), None);

        let mut hijacked = jar();
        hijacked.push(cookie("planted", "yes", "evil.test", "/", false));
        assert_eq!(
            cookie_header(&hijacked, &target).as_deref(),
            Some("planted=yes")
        );
    }

    #[test]
    fn response_headers_are_summarized_and_redacted_unless_full_headers_is_set() {
        let headers = BTreeMap::from([
            ("content-type".to_string(), "application/json".to_string()),
            ("content-length".to_string(), "42".to_string()),
            ("set-cookie".to_string(), "sid=supersecret".to_string()),
            ("x-api-key".to_string(), "leaky".to_string()),
            ("server".to_string(), "fixture".to_string()),
        ]);

        let summary = summarize_response_headers(&headers, false);
        assert_eq!(summary.len(), 2);
        assert_eq!(summary["content-type"], "application/json");
        assert!(!summary.contains_key("server"));

        let full = summarize_response_headers(&headers, true);
        assert_eq!(full["set-cookie"], "[REDACTED]");
        assert_eq!(full["x-api-key"], "[REDACTED]");
        assert_eq!(full["server"], "fixture");
    }

    #[test]
    fn body_budget_inlines_small_text_pretty_prints_json_and_spills_large_or_binary_bodies() {
        assert_eq!(split_response_body(b"", None), HttpResponseBody::Empty);
        assert_eq!(
            split_response_body(b"plain", Some("text/plain")),
            HttpResponseBody::Inline("plain".to_string())
        );
        assert_eq!(
            split_response_body(br#"{"a":1}"#, Some("application/json; charset=utf-8")),
            HttpResponseBody::Inline("{\n  \"a\": 1\n}".to_string())
        );
        assert_eq!(
            split_response_body(&[0xff, 0xfe, 0x00], Some("application/octet-stream")),
            HttpResponseBody::Artifact
        );
        let large = vec![b'x'; HTTP_INLINE_BODY_LIMIT_BYTES + 1];
        assert_eq!(
            split_response_body(&large, Some("text/plain")),
            HttpResponseBody::Artifact
        );
        assert!(matches!(
            split_response_body(
                &vec![b'x'; HTTP_INLINE_BODY_LIMIT_BYTES],
                Some("text/plain")
            ),
            HttpResponseBody::Inline(_)
        ));
    }

    #[test]
    fn inline_bodies_are_redacted_before_display() {
        let HttpResponseBody::Inline(text) = split_response_body(
            b"token=hunter2secret and Bearer abcdefghijklmnop",
            Some("text/plain"),
        ) else {
            panic!("expected an inline body");
        };
        assert!(!text.contains("hunter2secret"), "{text}");
        assert!(!text.contains("abcdefghijklmnop"), "{text}");
        assert_eq!(text, "token=[REDACTED] and Bearer [REDACTED]");

        let HttpResponseBody::Inline(json) = split_response_body(
            br#"{"callback":"https://api.example.test/cb?session=abc123"}"#,
            Some("application/json"),
        ) else {
            panic!("expected an inline body");
        };
        assert!(!json.contains("abc123"), "{json}");
        assert!(json.contains("[REDACTED]"), "{json}");
    }

    #[test]
    fn request_bodies_are_mutually_exclusive_and_carry_their_content_type() {
        let form = BTreeMap::from([
            ("name".to_string(), "a b".to_string()),
            ("kind".to_string(), "x&y".to_string()),
        ]);
        let encoded = build_request_body(None, None, Some(&form))
            .unwrap()
            .unwrap();
        assert_eq!(
            encoded.content_type.as_deref(),
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(
            String::from_utf8(encoded.bytes).unwrap(),
            "kind=x%26y&name=a+b"
        );

        let json = build_request_body(None, Some(&serde_json::json!({"a": 1})), None)
            .unwrap()
            .unwrap();
        assert_eq!(json.content_type.as_deref(), Some("application/json"));
        assert_eq!(String::from_utf8(json.bytes).unwrap(), r#"{"a":1}"#);

        let raw = build_request_body(Some("hello"), None, None)
            .unwrap()
            .unwrap();
        assert_eq!(raw.content_type, None);

        assert!(build_request_body(None, None, None).unwrap().is_none());
        assert!(build_request_body(Some("hello"), Some(&Value::Null), None).is_err());
    }

    #[test]
    fn urls_are_restricted_to_http_and_https() {
        assert!(parse_http_url("https://api.example.test/v1").is_ok());
        assert!(parse_http_url("http://api.example.test/v1").is_ok());
        for rejected in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:,x",
            "ftp://x.test/",
        ] {
            let error = parse_http_url(rejected).unwrap_err();
            assert!(error.contains("Unsupported URL scheme"), "{error}");
        }
        assert!(parse_http_url("not a url").is_err());
    }

    #[test]
    fn redirects_downgrade_unsafe_methods_and_resolve_relative_locations() {
        let base = Url::parse("https://api.example.test/v1/login").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(LOCATION, HeaderValue::from_static("/v1/home"));
        assert_eq!(
            redirect_location(StatusCode::FOUND, &headers, &base)
                .unwrap()
                .as_str(),
            "https://api.example.test/v1/home"
        );
        assert!(redirect_location(StatusCode::OK, &headers, &base).is_none());

        let body = Some(HttpRequestBody {
            bytes: b"a=1".to_vec(),
            content_type: None,
        });
        assert_eq!(
            follow_redirect(Method::POST, StatusCode::SEE_OTHER, body.clone()),
            (Method::GET, None)
        );
        assert_eq!(
            follow_redirect(Method::POST, StatusCode::FOUND, body.clone()),
            (Method::GET, None)
        );
        assert_eq!(
            follow_redirect(Method::POST, StatusCode::PERMANENT_REDIRECT, body.clone()),
            (Method::POST, body)
        );
    }

    #[test]
    fn response_artifacts_pick_an_extension_and_land_inside_the_artifact_directory() {
        assert_eq!(response_body_extension(Some("application/json")), "json");
        assert_eq!(response_body_extension(Some("text/csv")), "txt");
        assert_eq!(response_body_extension(Some("image/png")), "bin");
        assert_eq!(response_body_extension(None), "bin");

        let dir = tempfile::tempdir().unwrap();
        let artifacts_dir = dir.path().join("artifacts");
        let path = save_response_body(b"payload", &artifacts_dir, "http-1-0.json").unwrap();
        assert_eq!(path, artifacts_dir.join("http-1-0.json"));
        assert_eq!(std::fs::read(&path).unwrap(), b"payload");
    }
}
