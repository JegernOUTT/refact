use std::collections::HashMap;

use mdns_sd::{ServiceDaemon, ServiceInfo};

pub(crate) struct MdnsAdvertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl MdnsAdvertisement {
    pub(crate) fn start(port: u16) -> Option<Self> {
        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("daemon mDNS service did not start: {e}");
                return None;
            }
        };
        let host_name = crate::http::local_mdns_host_name();
        let display_name = host_name
            .trim_end_matches(".local.")
            .trim_end_matches('.')
            .to_string();
        let instance_name = format!("Refact on {display_name}");
        let mut properties = HashMap::new();
        properties.insert("app".to_string(), "refact".to_string());
        properties.insert("path".to_string(), "/".to_string());
        let service_info = match ServiceInfo::new(
            crate::http::MDNS_SERVICE_TYPE,
            &instance_name,
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

#[cfg(test)]
mod tests {
    #[test]
    fn mdns_uses_shared_host_name_helper() {
        let name = crate::http::local_mdns_host_name();
        assert!(!name.is_empty());
        assert!(name.ends_with(".local."));
    }
}
