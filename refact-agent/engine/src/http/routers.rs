use axum::extract::DefaultBodyLimit;
use axum::Router;
use axum::routing::get;
use tower_http::cors::CorsLayer;

use crate::app_state::AppState;
use crate::http::GuiPublicOriginCandidates;
use crate::http::handler_404;
use crate::providers::http::handle_openai_codex_auth_callback;

pub(crate) mod gui;
pub mod info;
pub mod v1;

pub fn make_refact_http_server(app_state: AppState) -> Router {
    Router::new()
        .nest("/v1", v1::make_v1_router(app_state.clone()))
        .route("/build_info", get(info::handle_info))
        .route("/auth/callback", get(handle_openai_codex_auth_callback))
        .route("/", get(gui::handle_gui_index))
        .route("/index.html", get(gui::handle_gui_index))
        .route("/favicon.ico", get(gui::handle_favicon))
        .route("/dist/chat/*path", get(gui::handle_gui_asset))
        .fallback(handler_404)
        .layer(axum::Extension(GuiPublicOriginCandidates::default()))
        .layer(DefaultBodyLimit::max(2usize.pow(20) * 15)) // new limit of payload 15MB(default: 2MB)
        .layer(CorsLayer::very_permissive())
        .with_state(app_state)
}
