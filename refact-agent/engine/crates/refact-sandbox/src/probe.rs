use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn run_with_timeout(
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<ExitStatus, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) => return Ok(status),
            None if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10));
            }
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "probe timed out after {:.3}s",
                    timeout.as_secs_f64()
                ));
            }
        }
    }
}
