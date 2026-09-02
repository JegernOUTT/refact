use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::{OnceLock, RwLock};

use axum::body::{boxed, Bytes, Full};
use axum::extract::Path;
use axum::http::{header, HeaderMap, HeaderValue, Response, StatusCode};
use axum::Extension;
use axum::response::IntoResponse;
use flate2::write::GzEncoder;
use flate2::Compression;
use rust_embed::{EmbeddedFile, RustEmbed};

use crate::http::GuiPublicOriginCandidates;

#[derive(RustEmbed)]
#[folder = "assets/chat/"]
#[exclude = "*.map"]
pub(crate) struct ChatGuiAsset;

pub(crate) const INDEX_PATH: &str = "index.html";
pub(crate) const ASSET_PREFIX: &str = "dist/chat/";
const CACHE_CONTROL: &str = "no-cache";
const MIN_GZIP_BYTES: usize = 1024;
const ORIGIN_CANDIDATES_PLACEHOLDER: &str =
    "/*__REFACT_ORIGIN_CANDIDATES__*/ window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ || []";

static GZIP_CACHE: OnceLock<RwLock<HashMap<String, Option<Bytes>>>> = OnceLock::new();

pub async fn handle_gui_index(
    Extension(candidates): Extension<GuiPublicOriginCandidates>,
) -> impl IntoResponse {
    match ChatGuiAsset::get(INDEX_PATH) {
        Some(asset) => {
            let body = inject_gui_origin_candidates(asset.data, &candidates);
            asset_response(INDEX_PATH, body, StatusCode::OK)
        }
        None => html_response(
            StatusCode::SERVICE_UNAVAILABLE,
            missing_gui_index_html().as_bytes().to_vec().into(),
        ),
    }
}

pub(crate) fn inject_gui_origin_candidates(
    body: Cow<'static, [u8]>,
    candidates: &GuiPublicOriginCandidates,
) -> Cow<'static, [u8]> {
    let Ok(html) = std::str::from_utf8(body.as_ref()) else {
        return body;
    };
    let Ok(json) = serde_json::to_string(&candidates.origins) else {
        return body;
    };
    if html.contains(ORIGIN_CANDIDATES_PLACEHOLDER) {
        Cow::Owned(
            html.replace(ORIGIN_CANDIDATES_PLACEHOLDER, &json)
                .into_bytes(),
        )
    } else {
        tracing::warn!("GUI origin candidates placeholder missing; serving index.html unchanged");
        body
    }
}

pub async fn handle_gui_asset(
    Path(path): Path<String>,
    request_headers: HeaderMap,
) -> impl IntoResponse {
    if path.is_empty() || path.split('/').any(|part| part == ".." || part.is_empty()) {
        return text_response(
            StatusCode::BAD_REQUEST,
            "invalid GUI asset path".to_string(),
        );
    }

    let embedded_path = format!("{ASSET_PREFIX}{path}");
    match ChatGuiAsset::get(&embedded_path) {
        Some(asset) => embedded_asset_response(&embedded_path, asset, &request_headers),
        None => text_response(
            StatusCode::NOT_FOUND,
            format!("GUI asset not found: {path}"),
        ),
    }
}

pub(crate) fn embedded_asset_response(
    embedded_path: &str,
    asset: EmbeddedFile,
    request_headers: &HeaderMap,
) -> Response<BoxBody> {
    let etag = asset_etag(&asset);

    if etag_matches(request_headers, &etag) {
        let mut response = Response::new(boxed(Full::from(Bytes::new())));
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        apply_validation_headers(response.headers_mut(), &etag);
        return response;
    }

    let compressed = if accepts_gzip(request_headers) {
        gzip_asset(embedded_path, asset.data.as_ref())
    } else {
        None
    };

    let content_type = content_type_for_path(embedded_path);
    let is_gzipped = compressed.is_some();
    let body = compressed.unwrap_or_else(|| Bytes::from(asset.data.into_owned()));

    let mut response = response_with_bytes(StatusCode::OK, content_type, body);
    if is_gzipped {
        response
            .headers_mut()
            .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    }
    apply_validation_headers(response.headers_mut(), &etag);
    response
}

fn apply_validation_headers(headers: &mut HeaderMap, etag: &str) {
    if let Ok(value) = HeaderValue::from_str(etag) {
        headers.insert(header::ETAG, value);
    }
    headers.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
}

fn asset_etag(asset: &EmbeddedFile) -> String {
    let mut etag = String::with_capacity(66);
    etag.push('"');
    for byte in asset.metadata.sha256_hash() {
        let _ = write!(etag, "{byte:02x}");
    }
    etag.push('"');
    etag
}

fn etag_matches(request_headers: &HeaderMap, etag: &str) -> bool {
    request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(',').any(|candidate| {
                let candidate = candidate.trim();
                candidate == "*"
                    || candidate == etag
                    || candidate
                        .strip_prefix("W/")
                        .is_some_and(|weak| weak == etag)
            })
        })
}

fn accepts_gzip(request_headers: &HeaderMap) -> bool {
    request_headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(',').any(|encoding| {
                encoding
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .eq_ignore_ascii_case("gzip")
            })
        })
}

fn gzip_cache() -> &'static RwLock<HashMap<String, Option<Bytes>>> {
    GZIP_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn gzip_asset(embedded_path: &str, data: &[u8]) -> Option<Bytes> {
    if data.len() < MIN_GZIP_BYTES {
        return None;
    }

    if let Ok(cache) = gzip_cache().read() {
        if let Some(cached) = cache.get(embedded_path) {
            return cached.clone();
        }
    }

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    let compressed = encoder
        .write_all(data)
        .and_then(|()| encoder.finish())
        .ok()
        .filter(|compressed| compressed.len() < data.len())
        .map(Bytes::from);

    if let Ok(mut cache) = gzip_cache().write() {
        cache.insert(embedded_path.to_string(), compressed.clone());
    }
    compressed
}

pub async fn handle_favicon() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

pub(crate) fn content_type_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

pub(crate) fn asset_response(
    path: &str,
    body: Cow<'static, [u8]>,
    status: StatusCode,
) -> Response<BoxBody> {
    response_with_body(status, content_type_for_path(path), body)
}

pub(crate) fn html_response(status: StatusCode, body: Cow<'static, [u8]>) -> Response<BoxBody> {
    response_with_body(status, "text/html; charset=utf-8", body)
}

pub(crate) fn text_response(status: StatusCode, body: String) -> Response<BoxBody> {
    response_with_body(
        status,
        "text/plain; charset=utf-8",
        body.into_bytes().into(),
    )
}

pub(crate) type BoxBody = axum::body::BoxBody;

fn response_with_body(
    status: StatusCode,
    content_type: &'static str,
    body: Cow<'static, [u8]>,
) -> Response<BoxBody> {
    response_with_bytes(status, content_type, Bytes::from(body.into_owned()))
}

fn response_with_bytes(
    status: StatusCode,
    content_type: &'static str,
    body: Bytes,
) -> Response<BoxBody> {
    let mut response = Response::new(boxed(Full::from(body)));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CACHE_CONTROL),
    );
    response
}

pub(crate) fn missing_gui_index_html() -> &'static str {
    r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Refact GUI assets missing</title>
  </head>
  <body>
    <h1>Refact GUI assets are not bundled in this build.</h1>
    <p>Run <code>cargo build</code> from <code>refact-agent/engine</code> with Node.js and npm available, or set <code>REFACT_SKIP_GUI_BUILD=1</code> only for API-only builds.</p>
  </body>
</html>
"#
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;

    #[test]
    fn content_type_maps_common_gui_assets() {
        assert_eq!(
            content_type_for_path("index.html"),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type_for_path("index.umd.cjs"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type_for_path("style.css"),
            "text/css; charset=utf-8"
        );
        assert_eq!(
            content_type_for_path("manifest.json"),
            "application/json; charset=utf-8"
        );
        assert_eq!(content_type_for_path("font.woff2"), "font/woff2");
    }

    fn headers_with(name: header::HeaderName, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(name, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn accepts_gzip_reads_quality_values_and_ignores_other_encodings() {
        assert!(accepts_gzip(&headers_with(
            header::ACCEPT_ENCODING,
            "gzip, deflate, br"
        )));
        assert!(accepts_gzip(&headers_with(
            header::ACCEPT_ENCODING,
            "br;q=1.0, GZIP;q=0.8"
        )));
        assert!(!accepts_gzip(&headers_with(
            header::ACCEPT_ENCODING,
            "br, deflate"
        )));
        assert!(!accepts_gzip(&HeaderMap::new()));
    }

    #[test]
    fn etag_matches_handles_wildcard_weak_and_lists() {
        let etag = "\"abc123\"";
        assert!(etag_matches(
            &headers_with(header::IF_NONE_MATCH, "*"),
            etag
        ));
        assert!(etag_matches(
            &headers_with(header::IF_NONE_MATCH, "\"abc123\""),
            etag
        ));
        assert!(etag_matches(
            &headers_with(header::IF_NONE_MATCH, "W/\"abc123\""),
            etag
        ));
        assert!(etag_matches(
            &headers_with(header::IF_NONE_MATCH, "\"other\", \"abc123\""),
            etag
        ));
        assert!(!etag_matches(
            &headers_with(header::IF_NONE_MATCH, "\"other\""),
            etag
        ));
        assert!(!etag_matches(&HeaderMap::new(), etag));
    }

    #[test]
    fn gzip_asset_skips_payloads_below_threshold() {
        let small = vec![b'a'; MIN_GZIP_BYTES - 1];
        assert!(gzip_asset("test/small.js", &small).is_none());
    }

    #[test]
    fn gzip_asset_compresses_and_serves_cached_bytes() {
        let data = "console.log('refact');".repeat(500).into_bytes();
        assert!(data.len() >= MIN_GZIP_BYTES);

        let first = gzip_asset("test/compressible.js", &data).expect("compressible payload");
        assert!(first.len() < data.len());

        let second = gzip_asset("test/compressible.js", &data).expect("cached payload");
        assert_eq!(first, second);
    }

    #[test]
    fn gzip_asset_rejects_incompressible_payloads() {
        let data: Vec<u8> = (0..8192u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
            .collect();
        let compressed = gzip_asset("test/random.bin", &data);
        if let Some(compressed) = compressed {
            assert!(compressed.len() < data.len());
        }
    }

    fn largest_embedded_js_asset() -> String {
        ChatGuiAsset::iter()
            .filter(|path| {
                path.ends_with(".js") || path.ends_with(".cjs") || path.ends_with(".css")
            })
            .filter_map(|path| {
                let size = ChatGuiAsset::get(path.as_ref())?.data.len();
                (size >= MIN_GZIP_BYTES).then(|| (size, path.to_string()))
            })
            .max()
            .map(|(_, path)| path)
            .expect("embedded GUI assets are required to verify asset negotiation")
    }

    #[test]
    fn embedded_asset_response_negotiates_gzip_and_revalidation() {
        let path = largest_embedded_js_asset();
        let asset = ChatGuiAsset::get(&path).expect("asset exists");
        let raw_len = asset.data.len();
        let etag = asset_etag(&asset);
        assert!(raw_len >= MIN_GZIP_BYTES);

        let plain = embedded_asset_response(&path, asset, &HeaderMap::new());
        assert_eq!(plain.status(), StatusCode::OK);
        assert!(!plain.headers().contains_key(header::CONTENT_ENCODING));
        assert_eq!(plain.headers().get(header::ETAG).unwrap(), etag.as_str());
        assert_eq!(
            plain.headers().get(header::VARY).unwrap(),
            "Accept-Encoding"
        );

        let asset = ChatGuiAsset::get(&path).expect("asset exists");
        let gzipped =
            embedded_asset_response(&path, asset, &headers_with(header::ACCEPT_ENCODING, "gzip"));
        assert_eq!(gzipped.status(), StatusCode::OK);
        assert_eq!(
            gzipped.headers().get(header::CONTENT_ENCODING).unwrap(),
            "gzip"
        );

        let compressed = ChatGuiAsset::get(&path)
            .and_then(|asset| gzip_asset(&path, asset.data.as_ref()))
            .expect("text asset compresses");
        assert!(compressed.len() < raw_len);

        let asset = ChatGuiAsset::get(&path).expect("asset exists");
        let not_modified =
            embedded_asset_response(&path, asset, &headers_with(header::IF_NONE_MATCH, &etag));
        assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            not_modified.headers().get(header::ETAG).unwrap(),
            etag.as_str()
        );
        assert_ne!(etag, "\"\"");
    }

    #[test]
    fn origin_candidates_inject_into_real_embedded_index() {
        let asset = ChatGuiAsset::get(INDEX_PATH).expect("embedded index.html");
        let candidates = GuiPublicOriginCandidates {
            origins: vec![
                "http://127.0.0.1:8001".to_string(),
                "http://workstation.local:8001".to_string(),
            ],
        };

        let injected = inject_gui_origin_candidates(asset.data, &candidates);
        let html = std::str::from_utf8(injected.as_ref()).unwrap();

        assert!(html.contains("http://127.0.0.1:8001"));
        assert!(html.contains("http://workstation.local:8001"));
        assert!(!html.contains(ORIGIN_CANDIDATES_PLACEHOLDER));
        assert!(!html.contains("window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ || []"));

        let reinjected = inject_gui_origin_candidates(injected.clone(), &candidates);
        assert_eq!(reinjected.as_ref(), injected.as_ref());
    }

    #[test]
    fn origin_candidates_missing_marker_warns_and_returns_unchanged() {
        let html = r#"<html><head></head><body></body></html>"#;
        let candidates = GuiPublicOriginCandidates {
            origins: vec!["http://127.0.0.1:8001".to_string()],
        };
        let logs = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_writer(SharedWriter(logs.clone()))
            .with_ansi(false)
            .finish();

        let injected = tracing::subscriber::with_default(subscriber, || {
            inject_gui_origin_candidates(Cow::Borrowed(html.as_bytes()), &candidates)
        });

        assert_eq!(injected.as_ref(), html.as_bytes());
        let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
        assert!(logs.contains("GUI origin candidates placeholder missing"));
    }

    #[derive(Clone)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    struct SharedWriterGuard(Arc<Mutex<Vec<u8>>>);

    impl<'a> MakeWriter<'a> for SharedWriter {
        type Writer = SharedWriterGuard;

        fn make_writer(&'a self) -> Self::Writer {
            SharedWriterGuard(self.0.clone())
        }
    }

    impl Write for SharedWriterGuard {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
