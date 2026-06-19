use std::collections::HashMap;
use std::net::IpAddr;

use mdns_sd::{ServiceDaemon, ServiceInfo};

use crate::daemon::config::DaemonConfig;

pub(crate) struct MdnsAdvertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl MdnsAdvertisement {
    pub(crate) fn start(config: &DaemonConfig, bind_ip: IpAddr, port: u16) -> Option<Self> {
        if !should_advertise(config, bind_ip) {
            tracing::debug!("daemon mDNS advertisement disabled for bind {bind_ip}");
            return None;
        }
        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("daemon mDNS service did not start: {e}");
                return None;
            }
        };
        let host_name = crate::http::local_mdns_host_name();
        let properties = service_properties(config.auth.enabled);
        let service_info = match ServiceInfo::new(
            crate::http::MDNS_SERVICE_TYPE,
            instance_name(),
            &host_name,
            "",
            port,
            Some(properties),
        ) {
            Ok(info) => info.enable_addr_auto(),
            Err(e) => {
                tracing::warn!("daemon mDNS service info invalid: {e}");
                let _ = daemon.shutdown();
                return None;
            }
        };
        let fullname = service_info.get_fullname().to_string();
        if let Err(e) = daemon.register(service_info) {
            tracing::warn!("daemon mDNS register failed {fullname}: {e}");
            let _ = daemon.shutdown();
            return None;
        }
        tracing::info!(
            "daemon mDNS advertising as http://{}:{port}/ ({fullname})",
            crate::http::local_mdns_browser_host(),
        );
        Some(MdnsAdvertisement { daemon, fullname })
    }

    pub(crate) fn stop(self) {
        if let Err(e) = self.daemon.unregister(&self.fullname) {
            tracing::warn!("failed to unregister daemon mDNS {}: {e}", self.fullname);
        }
        if let Err(e) = self.daemon.shutdown() {
            tracing::warn!("failed to stop daemon mDNS: {e}");
        }
    }
}

pub(crate) fn should_advertise(config: &DaemonConfig, bind_ip: IpAddr) -> bool {
    config
        .mdns
        .enabled
        .unwrap_or_else(|| !bind_ip.is_loopback())
}

pub(crate) fn service_properties(auth_enabled: bool) -> HashMap<String, String> {
    let mut properties = HashMap::new();
    properties.insert("app".to_string(), "refact".to_string());
    properties.insert("path".to_string(), "/".to_string());
    properties.insert(
        "auth".to_string(),
        if auth_enabled { "required" } else { "none" }.to_string(),
    );
    properties
}

fn instance_name() -> &'static str {
    "Refact Daemon"
}

#[cfg(test)]
mod tests {
    use crate::daemon::config::{AuthConfig, DaemonConfig, MdnsConfig};

    use super::*;

    #[test]
    fn mdns_uses_shared_host_name_helper() {
        let name = crate::http::local_mdns_host_name();
        assert!(!name.is_empty());
        assert!(name.ends_with(".local."));
    }

    #[test]
    fn mdns_default_advertises_only_non_loopback_binds() {
        let config = DaemonConfig::default();

        assert!(!should_advertise(&config, "127.0.0.1".parse().unwrap()));
        assert!(!should_advertise(&config, "::1".parse().unwrap()));
        assert!(should_advertise(&config, "0.0.0.0".parse().unwrap()));
        assert!(should_advertise(&config, "192.168.1.10".parse().unwrap()));
    }

    #[test]
    fn mdns_explicit_enable_and_disable_are_honored() {
        let enabled = DaemonConfig {
            mdns: MdnsConfig {
                enabled: Some(true),
            },
            ..DaemonConfig::default()
        };
        let disabled = DaemonConfig {
            mdns: MdnsConfig {
                enabled: Some(false),
            },
            ..DaemonConfig::default()
        };

        assert!(should_advertise(&enabled, "127.0.0.1".parse().unwrap()));
        assert!(!should_advertise(&disabled, "0.0.0.0".parse().unwrap()));
    }

    #[test]
    fn mdns_txt_auth_flag_reflects_auth_config() {
        assert_eq!(service_properties(false).get("auth").unwrap(), "none");
        assert_eq!(service_properties(true).get("auth").unwrap(), "required");

        let required = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
            },
            ..DaemonConfig::default()
        };
        assert_eq!(
            service_properties(required.auth.enabled)
                .get("auth")
                .unwrap(),
            "required"
        );
    }

    #[test]
    fn mdns_instance_name_is_generic() {
        assert_eq!(instance_name(), "Refact Daemon");
    }
}
