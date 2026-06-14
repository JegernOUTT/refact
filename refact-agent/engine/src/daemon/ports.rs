use std::net::TcpListener;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortPair {
    pub http_port: u16,
    pub lsp_port: u16,
}

pub fn allocate_port_pair() -> Result<PortPair, String> {
    for _ in 0..16 {
        let http_port = allocate_ephemeral_port()?;
        let lsp_port = allocate_ephemeral_port()?;
        if http_port != lsp_port {
            return Ok(PortPair {
                http_port,
                lsp_port,
            });
        }
    }
    Err("failed to allocate distinct worker ports".to_string())
}

fn allocate_ephemeral_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("failed to bind ephemeral port: {error}"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|error| format!("failed to read ephemeral port: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_pair_allocates_distinct_ports() {
        let pair = allocate_port_pair().unwrap();
        assert_ne!(pair.http_port, pair.lsp_port);
        assert!(pair.http_port > 0);
        assert!(pair.lsp_port > 0);
    }
}
