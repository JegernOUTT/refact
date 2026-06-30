use std::io::{Read, Write};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

pub struct PtyHandle {
    pub writer: Box<dyn Write + Send>,
    pub reader: Box<dyn Read + Send>,
    pub master: Box<dyn MasterPty + Send>,
}

pub fn default_pty_size() -> PtySize {
    PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }
}

// PTY cleanup limitation: `portable_pty` creates a new session (setsid) on Unix, which also
// creates a new process group for the direct child. Cleanup via `child.kill()` terminates the
// direct child only. Descendants that create their own subprocess groups (e.g., background jobs
// inside the PTY shell) will NOT be cleaned up. This is a known limitation and would require
// a session-level cleanup mechanism (e.g., `kill(-sid, SIGKILL)` using the session id) to
// address fully, which is not currently implemented.
pub fn spawn_pty(
    cmd: CommandBuilder,
    size: PtySize,
) -> Result<(PtyHandle, Box<dyn Child + Send>), String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(size)
        .map_err(|error| format!("failed to open pty: {error}"))?;
    let mut child: Box<dyn Child + Send> = pair
        .slave
        .spawn_command(cmd)
        .map_err(|error| format!("failed to spawn pty command: {error}"))?;
    let reader = match pair.master.try_clone_reader() {
        Ok(reader) => reader,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("failed to clone pty reader: {error}"));
        }
    };
    let writer = match pair.master.take_writer() {
        Ok(writer) => writer,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("failed to take pty writer: {error}"));
        }
    };
    Ok((
        PtyHandle {
            writer,
            reader,
            master: pair.master,
        },
        child,
    ))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::timeout;

    use crate::types::{ExecOutputStream, ExecSpawnRequest, ExecStatus};
    use crate::ExecRegistry;

    #[cfg(unix)]
    #[test]
    fn pty_post_spawn_cleanup_kills_child() {
        use super::{default_pty_size, native_pty_system, CommandBuilder};
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(default_pty_size()).unwrap();
        let mut cmd = CommandBuilder::new("sh");
        cmd.arg("-c");
        cmd.arg("sleep 30");
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        let pid = child.process_id().expect("child has pid");

        assert!(
            unsafe { libc::kill(pid as i32, 0) == 0 },
            "child should be alive before cleanup"
        );

        let _ = child.kill();
        let _ = child.wait();

        assert!(
            unsafe { libc::kill(pid as i32, 0) != 0 },
            "child should be killed after cleanup"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pty_echoes_stdin_on_unix() {
        let registry = ExecRegistry::new();
        let result = registry
            .spawn(ExecSpawnRequest::background("cat").with_tty(true))
            .await
            .unwrap();
        let process_id = result.snapshot.meta.process_id.clone();

        registry.write_stdin(&process_id, "hi\n", 0).await.unwrap();

        for _ in 0..40 {
            let read = registry.read(&process_id, 0, None).await;
            if read.chunks.iter().any(|chunk| chunk.text.contains("hi")) {
                registry.kill(&process_id).await.unwrap();
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let read = registry.read(&process_id, 0, None).await;
        registry.kill(&process_id).await.unwrap();
        panic!("pty output did not echo stdin: {:?}", read.chunks);
    }

    #[tokio::test]
    async fn pty_kills_cleanly() {
        let registry = ExecRegistry::new();
        let command = if cfg!(windows) {
            "Start-Sleep -Seconds 30"
        } else {
            "sleep 30"
        };
        let result = registry
            .spawn(ExecSpawnRequest::background(command).with_tty(true))
            .await
            .unwrap();
        let process_id = result.snapshot.meta.process_id.clone();

        let killed = timeout(Duration::from_secs(5), registry.kill(&process_id))
            .await
            .expect("pty kill should not time out")
            .unwrap();

        assert_eq!(killed.status, ExecStatus::Killed);
        let waited = timeout(Duration::from_secs(5), registry.wait(&process_id))
            .await
            .expect("pty wait should not time out")
            .unwrap();
        assert_eq!(waited.status, ExecStatus::Killed);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pty_output_is_combined() {
        let registry = ExecRegistry::new();
        let result = registry
            .spawn(ExecSpawnRequest::foreground("printf out; printf err >&2").with_tty(true))
            .await
            .unwrap();

        let read = registry
            .read(&result.snapshot.meta.process_id, 0, None)
            .await;
        assert!(read
            .chunks
            .iter()
            .all(|chunk| chunk.stream == ExecOutputStream::Combined));
    }
}
