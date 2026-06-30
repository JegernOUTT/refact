use tokio::net::TcpStream;

pub async fn is_someone_listening_on_that_tcp_port(
    port: u16,
    timeout: tokio::time::Duration,
) -> bool {
    match tokio::time::timeout(timeout, TcpStream::connect(&format!("127.0.0.1:{}", port))).await {
        Ok(Ok(_)) => true,   // Connection successful
        Ok(Err(_)) => false, // Connection failed, refused
        Err(e) => {
            // Timeout occurred
            tracing::error!("Timeout occurred while checking port {}: {}", port, e);
            false // still no one is listening, as far as we can tell
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::net::TcpListener;

    use super::*;

    #[tokio::test]
    async fn returns_true_when_localhost_port_is_listening() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        assert!(is_someone_listening_on_that_tcp_port(port, Duration::from_secs(1)).await);
    }

    #[tokio::test]
    async fn returns_false_for_unused_localhost_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        assert!(!is_someone_listening_on_that_tcp_port(port, Duration::from_secs(1)).await);
    }
}
