use std::borrow::Cow;

use axum::body::{boxed, Full};
use axum::extract::Path;
use axum::http::{header, HeaderValue, Response, StatusCode};
use axum::Extension;
use axum::response::IntoResponse;
use rust_embed::RustEmbed;

use crate::http::GuiPublicOriginCandidates;

#[derive(RustEmbed)]
#[folder = "assets/chat/"]
struct ChatGuiAsset;

const INDEX_PATH: &str = "index.html";
const ASSET_PREFIX: &str = "dist/chat/";
const CACHE_CONTROL: &str = "no-cache";

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

fn inject_gui_origin_candidates(
    body: Cow<'static, [u8]>,
    candidates: &GuiPublicOriginCandidates,
) -> Cow<'static, [u8]> {
    let Ok(html) = std::str::from_utf8(body.as_ref()) else {
        return body;
    };
    let Ok(json) = serde_json::to_string(&candidates.origins) else {
        return body;
    };
    let marker = "__REFACT_ENGINE_ORIGIN_CANDIDATES__ = []";
    let replacement = format!("__REFACT_ENGINE_ORIGIN_CANDIDATES__ = {json}");
    if html.contains(marker) {
        Cow::Owned(html.replace(marker, &replacement).into_bytes())
    } else {
        body
    }
}

pub async fn handle_gui_asset(Path(path): Path<String>) -> impl IntoResponse {
    if path.is_empty() || path.split('/').any(|part| part == ".." || part.is_empty()) {
        return text_response(
            StatusCode::BAD_REQUEST,
            "invalid GUI asset path".to_string(),
        );
    }

    let embedded_path = format!("{ASSET_PREFIX}{path}");
    match ChatGuiAsset::get(&embedded_path) {
        Some(asset) => asset_response(&embedded_path, asset.data, StatusCode::OK),
        None => text_response(
            StatusCode::NOT_FOUND,
            format!("GUI asset not found: {path}"),
        ),
    }
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

fn asset_response(path: &str, body: Cow<'static, [u8]>, status: StatusCode) -> Response<BoxBody> {
    response_with_body(status, content_type_for_path(path), body)
}

fn html_response(status: StatusCode, body: Cow<'static, [u8]>) -> Response<BoxBody> {
    response_with_body(status, "text/html; charset=utf-8", body)
}

fn text_response(status: StatusCode, body: String) -> Response<BoxBody> {
    response_with_body(
        status,
        "text/plain; charset=utf-8",
        body.into_bytes().into(),
    )
}

type BoxBody = axum::body::BoxBody;

fn response_with_body(
    status: StatusCode,
    content_type: &'static str,
    body: Cow<'static, [u8]>,
) -> Response<BoxBody> {
    let mut response = Response::new(boxed(Full::from(body.into_owned())));
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

fn missing_gui_index_html() -> &'static str {
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
}
