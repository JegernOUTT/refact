use std::collections::HashMap;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::{
    http::{StatusCode, Uri},
    response::IntoResponse,
};
use hyper::Server;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::global_context::GlobalContext;
use crate::http::routers::make_refact_http_server;

pub mod routers;
mod utils;

async fn handler_404(path: Uri) -> impl IntoResponse {
    info!("404 {}", path);
    (StatusCode::NOT_FOUND, format!("no handler for {}", path))
}

pub(crate) fn resolve_http_bind_addr(
    http_host: Option<&str>,
    inside_container: bool,
    port: u16,
) -> Result<SocketAddr, String> {
    let host = http_host
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .unwrap_or(if inside_container { "0.0.0.0" } else { "127.0.0.1" });
    let ip = host
        .parse::<IpAddr>()
        .map_err(|error| format!("invalid --http-host '{host}': {error}"))?;
    Ok(SocketAddr::new(ip, port))
}

const MDNS_SERVICE_TYPE: &str = "_refact-lsp._tcp.local.";

pub(crate) fn local_mdns_host_name() -> String {
    let hostname = hostname::get()
        .ok()
        .and_then(|value| value.into_string().ok())
        .map(|value| sanitize_mdns_label(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "refact".to_string());
    format!("{hostname}.local.")
}

pub(crate) fn local_mdns_browser_host() -> String {
    local_mdns_host_name().trim_end_matches('.').to_string()
}

fn sanitize_mdns_label(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

struct MdnsRegistration {
    daemon: ServiceDaemon,
    fullname: String,
}

impl MdnsRegistration {
    fn shutdown(self) {
        if let Err(error) = self.daemon.unregister(&self.fullname) {
            tracing::warn!("failed to unregister mDNS service {}: {}", self.fullname, error);
        }
        if let Err(error) = self.daemon.shutdown() {
            tracing::warn!("failed to shutdown mDNS daemon: {}", error);
        }
    }
}

fn start_mdns_advertisement(port: u16, app_searchable_id: String) -> Option<MdnsRegistration> {
    let daemon = match ServiceDaemon::new() {
        Ok(daemon) => daemon,
        Err(error) => {
            tracing::warn!("mDNS service daemon did not start: {}", error);
            return None;
        }
    };

    let host_name = local_mdns_host_name();
    let instance_name = format!(
        "Refact on {}",
        host_name.trim_end_matches(".local.").trim_end_matches('.')
    );
    let mut properties = HashMap::new();
    properties.insert("app".to_string(), "refact".to_string());
    properties.insert("path".to_string(), "/".to_string());
    properties.insert("id".to_string(), app_searchable_id);

    let service_info = match ServiceInfo::new(
        MDNS_SERVICE_TYPE,
        &instance_name,
        &host_name,
        "",
        port,
        Some(properties),
    ) {
        Ok(service_info) => service_info.enable_addr_auto(),
        Err(error) => {
            tracing::warn!("mDNS service info was invalid: {}", error);
            let _ = daemon.shutdown();
            return None;
        }
    };
    let fullname = service_info.get_fullname().to_string();

    if let Err(error) = daemon.register(service_info) {
        tracing::warn!("failed to register mDNS service {}: {}", fullname, error);
        let _ = daemon.shutdown();
        return None;
    }

    info!(
        "mDNS advertising Refact GUI as http://{}:{}/ ({})",
        local_mdns_browser_host(),
        port,
        fullname
    );
    Some(MdnsRegistration { daemon, fullname })
}

pub async fn start_server(
    gcx: Arc<GlobalContext>,
    ask_shutdown_receiver: std::sync::mpsc::Receiver<String>,
) -> Option<JoinHandle<()>> {
    let (port, is_inside_container, http_host) = {
        (
            gcx.cmdline.http_port,
            gcx.cmdline.inside_container,
            gcx.cmdline.http_host.clone(),
        )
    };
    if port == 0 {
        return None;
    }
    let shutdown_flag: Arc<AtomicBool> = gcx.shutdown_flag.clone();
    let app_searchable_id = gcx.app_searchable_id.lock().unwrap().clone();
    Some(tokio::spawn(async move {
        let addr = match resolve_http_bind_addr(http_host.as_deref(), is_inside_container, port) {
            Ok(addr) => addr,
            Err(error) => {
                error!("server error: {}", error);
                return;
            }
        };
        if !addr.ip().is_loopback() {
            tracing::warn!(
                "HTTP server is listening on non-loopback address {}; local Refact APIs are reachable from the network if firewall rules allow it",
                addr
            );
        }
        let builder = Server::try_bind(&addr).map_err(|e| {
            let _ = write!(std::io::stderr(), "PORT_BUSY {}\n", e);
            format!("port busy, address {}: {}", addr, e)
        });
        match builder {
            Ok(builder) => {
                info!("HTTP server listening on {}", addr);
                let mdns = start_mdns_advertisement(port, app_searchable_id.clone());
                let app_state = crate::app_state::AppState::from_gcx(gcx.clone()).await;
                let router = make_refact_http_server(app_state);
                let gcx_for_shutdown = gcx.clone();
                let shutdown = async move {
                    crate::global_context::block_until_signal(ask_shutdown_receiver, shutdown_flag)
                        .await;
                    crate::chat::close_all_chat_sessions(
                        crate::app_state::AppState::from_gcx(gcx_for_shutdown).await,
                    )
                    .await;
                };
                let server = builder
                    .serve(router.into_make_service())
                    .with_graceful_shutdown(shutdown);
                let resp = server
                    .await
                    .map_err(|e| format!("HTTP server error: {}", e));
                if let Err(e) = resp {
                    error!("server error: {}", e);
                } else {
                    info!("clean shutdown");
                }
                if let Some(mdns) = mdns {
                    mdns.shutdown();
                }
            }
            Err(e) => {
                error!("server error: {}", e);
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_bind_defaults_to_loopback() {
        let addr = resolve_http_bind_addr(None, false, 8001).unwrap();
        assert_eq!(addr.to_string(), "127.0.0.1:8001");
    }

    #[test]
    fn http_bind_preserves_inside_container_default() {
        let addr = resolve_http_bind_addr(None, true, 8001).unwrap();
        assert_eq!(addr.to_string(), "0.0.0.0:8001");
    }

    #[test]
    fn http_bind_uses_explicit_host() {
        let addr = resolve_http_bind_addr(Some("127.0.0.1"), true, 8001).unwrap();
        assert_eq!(addr.to_string(), "127.0.0.1:8001");
    }

    #[test]
    fn http_bind_rejects_invalid_host() {
        assert!(resolve_http_bind_addr(Some("localhost"), false, 8001).is_err());
    }
}
