use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex as AMutex;
use tokio::time::Duration;
pub async fn blocking_read_until_token_or_timeout<
    StdoutReader: AsyncRead + Unpin,
    StderrReader: AsyncRead + Unpin,
>(
    stdout: &mut StdoutReader,
    stderr: &mut StderrReader,
    timeout_ms: u64,
    output_token: &str,
) -> Result<(String, String, bool), String> {
    assert!(
        timeout_ms > 0,
        "Timeout in ms must be positive to prevent indefinite reading if the stream lacks an EOF"
    );
    let start_time = Instant::now();
    let timeout_duration = Duration::from_millis(timeout_ms);
    let mut output = Vec::new();
    let mut error = Vec::new();
    let mut output_buf = [0u8; 1024];
    let mut error_buf = [0u8; 1024];
    let mut have_the_token = false;

    while start_time.elapsed() < timeout_duration {
        let mut output_bytes_read = 0;
        let mut error_bytes_read = 0;
        tokio::select! {
            stdout_result = stdout.read(&mut output_buf) => {
                match stdout_result {
                    Ok(0) => {},
                    Ok(bytes_read) => {
                        output.extend_from_slice(&output_buf[..bytes_read]);
                        if !output_token.is_empty() && output.trim_ascii_end().ends_with(output_token.as_bytes()) {
                            have_the_token = true;
                        }
                        output_bytes_read = bytes_read;
                    },
                    Err(e) => return Err(format!("Error reading from stdout: {}", e)),
                }
            },
            stderr_result = stderr.read(&mut error_buf) => {
                match stderr_result {
                    Ok(0) => {},
                    Ok(bytes_read) => {
                        error.extend_from_slice(&error_buf[..bytes_read]);
                        error_bytes_read = bytes_read;
                    },
                    Err(e) => return Err(format!("Error reading from stderr: {}", e)),
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {},
        }
        if have_the_token && output_bytes_read == 0 && error_bytes_read == 0 {
            break;
        }
    }

    Ok((
        output.to_string_lossy_and_strip_ansi(),
        error.to_string_lossy_and_strip_ansi(),
        have_the_token,
    ))
}

pub async fn read_file_with_cursor(
    file_path: &Path,
    cursor: Arc<AMutex<u64>>,
) -> Result<(String, usize), String> {
    let file = tokio::fs::OpenOptions::new()
        .read(true)
        .open(file_path)
        .await
        .map_err(|e| format!("Failed to read file: {}", e))?;
    let mut cursor_locked = cursor.lock().await;
    let mut file = tokio::io::BufReader::new(file);
    file.seek(tokio::io::SeekFrom::Start(*cursor_locked))
        .await
        .map_err(|e| format!("Failed to seek: {}", e))?;
    let mut buffer = String::new();
    let bytes_read = file
        .read_to_string(&mut buffer)
        .await
        .map_err(|e| format!("Failed to read to buffer: {}", e))?;
    if bytes_read > 0 {
        *cursor_locked += bytes_read as u64;
    }
    Ok((buffer, bytes_read))
}

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
pub trait AnsiStrippable {
    fn to_string_lossy_and_strip_ansi(&self) -> String;
}

impl AnsiStrippable for [u8] {
    fn to_string_lossy_and_strip_ansi(&self) -> String {
        String::from_utf8_lossy(&strip_ansi_escapes::strip(self)).to_string()
    }
}
