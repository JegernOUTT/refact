use crate::{
    kernel::{Kernel, KernelIo, OsBackend},
    protocol::*,
};
use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
#[derive(Debug, Clone)]
pub struct WorkerCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<PathBuf>,
}
#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub memory_limit_bytes: u64,
    pub memory_poll_ms: u64,
}
impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            memory_limit_bytes: 128 * 1024 * 1024,
            memory_poll_ms: 25,
        }
    }
}
#[derive(Debug, Clone)]
pub enum ClientEvent {
    Kernel(KernelEvent),
    ProtocolError {
        message: String,
    },
    MemoryExceeded {
        rss_bytes: u64,
    },
    Exited {
        code: Option<i32>,
        stderr_tail: String,
    },
}
pub struct KernelClient {
    commands: Option<mpsc::Sender<KernelCommand>>,
    events: mpsc::UnboundedReceiver<ClientEvent>,
    kill: Option<mpsc::UnboundedSender<()>>,
    pid: Option<u32>,
}
impl KernelClient {
    pub fn spawn(command: WorkerCommand, options: ClientOptions) -> io::Result<Self> {
        let mut cmd = tokio::process::Command::new(command.program);
        cmd.args(command.args)
            .env_clear()
            .envs(command.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = command.cwd {
            cmd.current_dir(cwd);
        }
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (tx, mut rx) = mpsc::channel::<KernelCommand>(16);
        let (events, event_rx) = mpsc::unbounded_channel();
        let (kill, mut kill_rx) = mpsc::unbounded_channel();
        let output_events = events.clone();
        let output_kill = kill.clone();
        let output = tokio::spawn(async move {
            loop {
                let read = async {
                    let mut header = [0; 4];
                    if stdout.read(&mut header[..1]).await? == 0 {
                        return Ok(None);
                    }
                    stdout.read_exact(&mut header[1..]).await?;
                    let len = u32::from_le_bytes(header) as usize;
                    if len > MAX_FRAME_BYTES {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "frame too large",
                        ));
                    }
                    let mut b = vec![0; len];
                    stdout.read_exact(&mut b).await?;
                    serde_json::from_slice::<KernelEvent>(&b)
                        .map(Some)
                        .map_err(io::Error::other)
                }
                .await;
                let event = match read {
                    Ok(Some(e)) => e,
                    Ok(None) => break,
                    Err(e) => {
                        let _ = output_events.send(ClientEvent::ProtocolError {
                            message: e.to_string(),
                        });
                        let _ = output_kill.send(());
                        break;
                    }
                };
                if output_events.send(ClientEvent::Kernel(event)).is_err() {
                    break;
                }
            }
        });
        let tail = Arc::new(Mutex::new(Vec::new()));
        let stderr_tail = tail.clone();
        let error_reader = tokio::spawn(async move {
            let mut b = [0; 4096];
            while let Ok(n) = stderr.read(&mut b).await {
                if n == 0 {
                    break;
                }
                let mut t = stderr_tail.lock().unwrap();
                t.extend_from_slice(&b[..n]);
                let excess = t.len().saturating_sub(16384);
                t.drain(..excess);
            }
        });
        let writer = tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                let mut bytes = vec![];
                if write_frame(&mut bytes, &cmd).is_err() || stdin.write_all(&bytes).await.is_err()
                {
                    break;
                }
                if matches!(cmd, KernelCommand::Shutdown) {
                    break;
                }
            }
        });
        let mut child = ReapGuard(child);
        tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval(Duration::from_millis(options.memory_poll_ms.max(1)));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut exceeded = None;
            let status = loop {
                tokio::select! {status=child.wait()=>break status,_=kill_rx.recv()=>{let _=child.start_kill();break child.wait().await},_=ticker.tick()=>{if let Some(pid)=pid {let pid=sysinfo::Pid::from_u32(pid);let rss=tokio::task::spawn_blocking(move||{let mut system=sysinfo::System::new();system.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[pid]),true,sysinfo::ProcessRefreshKind::nothing().with_memory());system.process(pid).map(|p|p.memory()).unwrap_or(0)}).await.unwrap_or(0);if options.memory_limit_bytes>0&&rss>options.memory_limit_bytes{exceeded=Some(rss);let _=child.start_kill();break child.wait().await}}}}
            };
            writer.abort();
            let _ = output.await;
            let _ = error_reader.await;
            if let Some(rss_bytes) = exceeded {
                let _ = events.send(ClientEvent::MemoryExceeded {
                    rss_bytes,
                });
            }
            let stderr_tail = String::from_utf8_lossy(&tail.lock().unwrap()).into_owned();
            let _ = events.send(ClientEvent::Exited {
                code: status.ok().and_then(|s| s.code()),
                stderr_tail,
            });
        });
        Ok(Self {
            commands: Some(tx),
            events: event_rx,
            kill: Some(kill),
            pid,
        })
    }
    pub fn spawn_in_thread(os: Box<dyn OsBackend>, _options: ClientOptions) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel(16);
        let (events, event_rx) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("python-kernel".into())
            .stack_size(crate::worker::INTERPRETER_STACK_SIZE)
            .spawn(move || {
                struct ChannelIo {
                    rx: mpsc::Receiver<KernelCommand>,
                    tx: mpsc::UnboundedSender<ClientEvent>,
                }
                impl KernelIo for ChannelIo {
                    fn emit(&mut self, e: KernelEvent) -> io::Result<()> {
                        self.tx
                            .send(ClientEvent::Kernel(e))
                            .map_err(|_| io::ErrorKind::BrokenPipe.into())
                    }
                    fn recv(&mut self) -> io::Result<KernelCommand> {
                        self.rx
                            .blocking_recv()
                            .ok_or_else(|| io::ErrorKind::UnexpectedEof.into())
                    }
                }
                let mut io = ChannelIo {
                    rx,
                    tx: events,
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut kernel = Kernel::new(os);
                    io.emit(KernelEvent::Ready {
                        monty_version: "1.6.0".into(),
                    })?;
                    while let Ok(cmd) = io.recv() {
                        if !kernel.handle(cmd, &mut io)? {
                            break;
                        }
                    }
                    Ok::<_, std::io::Error>(())
                }));
                let _ = io.tx.send(ClientEvent::Exited {
                    code: Some(if matches!(result, Ok(Ok(()))) { 0 } else { 1 }),
                    stderr_tail: String::new(),
                });
            })?;
        let (kill, mut kill_rx) = mpsc::unbounded_channel();
        let shutdown = tx.clone();
        tokio::spawn(async move {
            let _ = kill_rx.recv().await;
            let _ = shutdown.send(KernelCommand::Shutdown).await;
        });
        Ok(Self {
            commands: Some(tx),
            events: event_rx,
            kill: Some(kill),
            pid: None,
        })
    }
    pub async fn send(&self, mut cmd: KernelCommand) -> io::Result<()> {
        // Values a frame cannot carry are refused here with an explicit error, never cut.
        match &mut cmd {
            KernelCommand::Feed {
                globals, ..
            } => {
                for g in globals.iter() {
                    if let GlobalKind::Json(v) = &g.kind {
                        if let Some(message) = frame_refusal(v) {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("global {}: {message}", g.name),
                            ));
                        }
                    }
                }
            }
            KernelCommand::Results {
                results, ..
            } => {
                for (_, r) in results.iter_mut() {
                    refuse_unframeable(r);
                }
                refuse_over_frame(results);
            }
            KernelCommand::SyncResult {
                result, ..
            } => refuse_unframeable(result),
            _ => {}
        }
        self.commands
            .as_ref()
            .ok_or(io::ErrorKind::BrokenPipe)?
            .send(cmd)
            .await
            .map_err(|_| io::ErrorKind::BrokenPipe.into())
    }
    pub async fn next_event(&mut self) -> ClientEvent {
        self.events.recv().await.unwrap_or(ClientEvent::Exited {
            code: None,
            stderr_tail: String::new(),
        })
    }
    pub async fn kill(&mut self) -> io::Result<()> {
        if let Some(kill) = &self.kill {
            let _ = kill.send(());
        }
        self.commands.take();
        Ok(())
    }
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }
    pub fn kill_handle(&self) -> Option<mpsc::UnboundedSender<()>> {
        self.kill.clone()
    }
}
impl Drop for KernelClient {
    fn drop(&mut self) {
        if let Some(kill) = &self.kill {
            let _ = kill.send(());
        }
        self.commands.take();
    }
}

struct ReapGuard(tokio::process::Child);
impl std::ops::Deref for ReapGuard {
    type Target = tokio::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ReapGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for ReapGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.0.id() {
            let _ = self.0.start_kill();
            #[cfg(unix)]
            let _ = std::thread::Builder::new()
                .name("python-reaper".into())
                .spawn(move || {
                    let mut status = 0;
                    loop {
                        // The supervisor exclusively owns this direct child PID.
                        let result = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) };
                        if result >= 0
                            || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted
                        {
                            break;
                        }
                    }
                });
        }
    }
}
fn frame_refusal(v: &serde_json::Value) -> Option<String> {
    use crate::convert::{json_size, MAX_DEPTH, MAX_VALUE_BYTES};
    let size = json_size(v);
    (size.bytes as u64 > MAX_VALUE_BYTES || size.depth > MAX_DEPTH).then(|| {
        format!(
            "a result of {} bytes nested {} levels deep cannot be passed to a script (limit {MAX_VALUE_BYTES} bytes, {MAX_DEPTH} levels); narrow the call",
            size.bytes, size.depth
        )
    })
}
fn refuse_unframeable(r: &mut HostResult) {
    if let HostResult::Ok(v) = r {
        if let Some(message) = frame_refusal(v) {
            *r = HostResult::Err {
                exc_type: "RuntimeError".into(),
                message,
            };
        }
    }
}
/// Results answered together share one frame; the largest become errors until it fits.
fn refuse_over_frame(results: &mut [(u32, HostResult)]) {
    use crate::convert::json_size;
    const HEADROOM: usize = 64 * 1024;
    let mut sizes: Vec<(usize, usize)> = results
        .iter()
        .enumerate()
        .map(|(i, (_, r))| match r {
            HostResult::Ok(v) => (json_size(v).bytes, i),
            HostResult::Err {
                message, ..
            } => (message.len() * 6 + 64, i),
        })
        .collect();
    let mut total: usize = sizes.iter().map(|(b, _)| b + 16).sum();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    for (bytes, i) in sizes {
        if total + HEADROOM <= MAX_FRAME_BYTES {
            break;
        }
        if let HostResult::Ok(_) = results[i].1 {
            let message = format!(
                "a result of {bytes} bytes did not fit: results awaited together must stay under {MAX_FRAME_BYTES} bytes in total; narrow the call or await it alone"
            );
            total = total - bytes + message.len() + 64;
            results[i].1 = HostResult::Err {
                exc_type: "RuntimeError".into(),
                message,
            };
        }
    }
}
