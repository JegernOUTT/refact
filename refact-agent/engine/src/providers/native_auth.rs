use refact_core::claude_backend::{ClaudeBackend, ClaudeTransportConfig};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

const LIMIT: usize = 16;
const TTL: Duration = Duration::from_secs(600);
const OUTPUT_LIMIT: usize = 4096;

struct Session {
    provider: String,
    config: ClaudeTransportConfig,
    created: Instant,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<State>>,
    worker: JoinHandle<()>,
}

struct State {
    status: &'static str,
    output: String,
}

fn sessions() -> &'static Mutex<HashMap<String, Session>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Session>>> = OnceLock::new();
    SESSIONS.get_or_init(Default::default)
}

fn present(value: &Option<String>) -> bool {
    value.as_ref().is_some_and(|v| !v.is_empty())
}

fn login_spec(
    config: &ClaudeTransportConfig,
    env: impl Fn(&str) -> Option<String>,
) -> Result<(&'static str, Vec<String>, String), String> {
    let blocked: &[&str] = match config.backend {
        ClaudeBackend::Bedrock => &[
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "AWS_WEB_IDENTITY_TOKEN_FILE",
            "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
            "AWS_CONTAINER_CREDENTIALS_FULL_URI",
        ],
        ClaudeBackend::Vertex => &["GOOGLE_APPLICATION_CREDENTIALS"],
        ClaudeBackend::Foundry => &[
            "AZURE_CLIENT_SECRET",
            "AZURE_FEDERATED_TOKEN_FILE",
            "IDENTITY_ENDPOINT",
            "MSI_ENDPOINT",
        ],
        _ => return Err("Native sign-in requires Bedrock, Vertex, or Foundry".into()),
    };
    if present(&config.auth_token)
        || (config.backend == ClaudeBackend::Vertex && config.auth_token.is_some())
        || present(&config.api_key)
        || (config.backend == ClaudeBackend::Bedrock
            && (config.aws_access_key_id.is_some()
                || config.aws_secret_access_key.is_some()
                || config.aws_session_token.is_some()))
        || (config.backend == ClaudeBackend::Foundry
            && (present(&config.foundry_api_key) || present(&config.foundry_auth_token)))
    {
        return Err(
            "Remove explicit credential overrides and save the provider before native sign-in"
                .into(),
        );
    }
    for name in blocked {
        if env(name).is_some_and(|v| !v.is_empty()) {
            return Err(format!("Unset {name} on the engine host and restart the engine before native sign-in; it overrides native credentials"));
        }
    }
    match config.backend {
        ClaudeBackend::Bedrock => {
            let profile = config.aws_profile.clone()
                .or_else(|| env("AWS_PROFILE").filter(|v| !v.is_empty()))
                .or_else(|| env("AWS_DEFAULT_PROFILE").filter(|v| !v.is_empty()))
                .unwrap_or_else(|| "default".into());
            if profile.is_empty() || profile.len() > 128 || profile.starts_with('-')
                || !profile.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.@+".contains(&b))
            {
                return Err("Invalid AWS profile: use 1–128 letters, digits, _, -, ., @, or +, without a leading hyphen".into());
            }
            Ok(("aws", vec!["sso".into(), "login".into(), "--profile".into(), profile.clone(), "--no-browser".into(), "--use-device-code".into()],
                format!("Sign in to AWS SSO profile {profile} using the URL and device code below. Configure this SSO profile on the engine host first (aws configure sso). Existing static credentials in that profile can take precedence; use an SSO-only profile. Credentials are stored on the engine host.")))
        }
        ClaudeBackend::Vertex => Ok(("gcloud", vec!["auth".into(), "application-default".into(), "login".into(), "--quiet".into()],
            "Complete Google Application Default Credentials sign-in in the browser opened on the engine host. For a remote/headless engine, this flow requires access to that host's browser and loopback callback; otherwise cancel and run gcloud auth application-default login on that host yourself. No verification code can be entered in this UI. Credentials are stored on the engine host.".into())),
        ClaudeBackend::Foundry => Ok(("az", vec!["login".into(), "--use-device-code".into(), "--output".into(), "none".into()],
            "Open the Microsoft device-login URL below and enter its device code. Azure CLI credentials are stored on the engine host. Managed identity, when available on the host, takes precedence over Azure CLI credentials.".into())),
        _ => unreachable!(),
    }
}

fn safe_path(path: &std::ffi::OsStr, cwd: &Path) -> Result<std::ffi::OsString, String> {
    let cwd = cwd
        .canonicalize()
        .map_err(|_| "Cannot resolve engine working directory")?;
    let project = cwd
        .ancestors()
        .find(|p| p.join(".git").exists())
        .unwrap_or(&cwd);
    let dirs: Vec<PathBuf> = std::env::split_paths(path)
        .filter(|p| p.is_absolute())
        .filter_map(|p| p.canonicalize().ok())
        .filter(|p| !p.starts_with(project) && !p.starts_with(&cwd))
        .collect();
    std::env::join_paths(dirs).map_err(|_| "Invalid engine-host PATH".into())
}

fn resolve_cli(name: &str, path: &std::ffi::OsStr) -> Result<PathBuf, String> {
    let resolved = which::which_in(name, Some(path), std::env::temp_dir())
        .map_err(|_| format!("Install {name} CLI on the engine host and add its absolute installation directory to PATH, then restart the engine"))?;
    if resolved
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("cmd") || s.eq_ignore_ascii_case("bat"))
    {
        return Err(format!("Native sign-in requires a {name} executable, not a shell script wrapper; run the login command manually on this engine host"));
    }
    Ok(resolved)
}

pub async fn start(
    provider_name: &str,
    transport: &ClaudeTransportConfig,
    shutdown: Arc<AtomicBool>,
) -> Result<Value, String> {
    if shutdown.load(Ordering::Relaxed) {
        return Err("Engine is shutting down".into());
    }
    let (cli, argv, instructions) = login_spec(transport, |name| std::env::var(name).ok())?;
    let cwd = std::env::current_dir().map_err(|_| "Cannot resolve engine working directory")?;
    let path = safe_path(&std::env::var_os("PATH").unwrap_or_default(), &cwd)?;
    let executable = resolve_cli(cli, &path)?;
    let canonical_executable = executable
        .canonicalize()
        .map_err(|_| format!("Cannot resolve {cli} CLI installation"))?;
    let canonical_cwd = cwd
        .canonicalize()
        .map_err(|_| "Cannot resolve engine working directory")?;
    let project = canonical_cwd
        .ancestors()
        .find(|p| p.join(".git").exists())
        .unwrap_or(&canonical_cwd);
    if canonical_executable.starts_with(project) || canonical_executable.starts_with(&canonical_cwd)
    {
        return Err("Native sign-in CLI must be installed outside the project directory".into());
    }
    let mut command = Command::new(executable);
    command
        .args(argv)
        .env("PATH", path)
        .env("NO_COLOR", "1")
        .env("LC_ALL", "C.UTF-8")
        .env("AWS_PAGER", "");
    start_command(
        sessions(),
        provider_name,
        transport,
        shutdown,
        command,
        instructions,
        TTL,
    )
    .await
}

async fn start_command(
    registry: &Mutex<HashMap<String, Session>>,
    provider: &str,
    config: &ClaudeTransportConfig,
    shutdown: Arc<AtomicBool>,
    mut command: Command,
    instructions: String,
    ttl: Duration,
) -> Result<Value, String> {
    let mut entries = registry.lock().await;
    let stale: Vec<String> = entries
        .iter()
        .filter(|(_, s)| s.created.elapsed() > TTL || s.provider == provider)
        .map(|(id, _)| id.clone())
        .collect();
    for id in stale {
        if let Some(session) = entries.get_mut(&id) {
            session.stop.store(true, Ordering::Relaxed);
            let _ = (&mut session.worker).await;
        }
        entries.remove(&id);
    }
    if entries.len() >= LIMIT {
        return Err(
            "Too many native sign-in sessions; cancel an existing session and retry".into(),
        );
    }
    let host_dir = tempfile::Builder::new()
        .prefix("refact-native-auth-")
        .tempdir()
        .map_err(|_| "Cannot create a private engine-host login directory")?;
    command
        .current_dir(host_dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn().map_err(|e| {
        format!(
            "Cannot start native sign-in CLI ({:?}); check the CLI installation on the engine host",
            e.kind()
        )
    })?;
    let state = Arc::new(Mutex::new(State {
        status: "pending",
        output: instructions.clone(),
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let worker_state = state.clone();
    let worker_stop = stop.clone();
    let backend = config.backend;
    let worker = tokio::spawn(async move {
        let _host_dir = host_dir;
        run(child, backend, worker_state, worker_stop, shutdown, ttl).await;
    });
    let id = uuid::Uuid::new_v4().to_string();
    entries.insert(
        id.clone(),
        Session {
            provider: provider.into(),
            config: config.clone(),
            created: Instant::now(),
            stop,
            state,
            worker,
        },
    );
    Ok(
        json!({"session_id": id, "authorize_url": "", "mode": "native", "instructions": instructions, "poll_interval": 2}),
    )
}

async fn read_pipe(
    pipe: &mut Option<impl AsyncRead + Unpin>,
    buf: &mut [u8],
) -> std::io::Result<usize> {
    match pipe {
        Some(pipe) => pipe.read(buf).await,
        None => std::future::pending().await,
    }
}

async fn terminate(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn run(
    mut child: Child,
    backend: ClaudeBackend,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    ttl: Duration,
) {
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let mut out = [0u8; 1024];
    let mut err = [0u8; 1024];
    let mut out_line = Vec::new();
    let mut err_line = Vec::new();
    let deadline = tokio::time::Instant::now() + ttl;
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let result = loop {
        tokio::select! {
            biased;
            _ = tick.tick() => {
                if stop.load(Ordering::Relaxed) || shutdown.load(Ordering::Relaxed) {
                    terminate(&mut child).await;
                    break Err("Native sign-in cancelled");
                }
                if tokio::time::Instant::now() >= deadline {
                    terminate(&mut child).await;
                    break Err("Native sign-in timed out; retry sign-in");
                }
            }
            read = read_pipe(&mut stdout, &mut out) => match read {
                Ok(0) | Err(_) => { stdout = None; flush_output(&state, backend, &mut out_line).await; }
                Ok(n) => capture(&state, backend, &mut out_line, &out[..n]).await,
            },
            read = read_pipe(&mut stderr, &mut err) => match read {
                Ok(0) | Err(_) => { stderr = None; flush_output(&state, backend, &mut err_line).await; }
                Ok(n) => capture(&state, backend, &mut err_line, &err[..n]).await,
            },
            exit = child.wait() => break match exit {
                Ok(status) if status.success() => Ok(()),
                _ => Err("Native sign-in CLI failed; check its installation and cloud account/profile configuration on the engine host, then retry"),
            },
        }
    };
    let _ = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            if stdout.is_none() && stderr.is_none() {
                break;
            }
            tokio::select! {
                read = read_pipe(&mut stdout, &mut out) => match read {
                    Ok(0) | Err(_) => stdout = None,
                    Ok(n) => capture(&state, backend, &mut out_line, &out[..n]).await,
                },
                read = read_pipe(&mut stderr, &mut err) => match read {
                    Ok(0) | Err(_) => stderr = None,
                    Ok(n) => capture(&state, backend, &mut err_line, &err[..n]).await,
                },
            }
        }
    })
    .await;
    flush_output(&state, backend, &mut out_line).await;
    flush_output(&state, backend, &mut err_line).await;
    if result.is_ok() {
        refact_claude_cloud::reset_cached_credentials(backend).await;
    }
    let mut state = state.lock().await;
    state.status = if result.is_ok() {
        "completed"
    } else {
        "failed"
    };
    state.output.push('\n');
    state.output.push_str(result.err().unwrap_or("Native sign-in completed on the engine host. Cloud permissions and model access are checked on the next request."));
}

async fn capture(state: &Mutex<State>, backend: ClaudeBackend, line: &mut Vec<u8>, bytes: &[u8]) {
    for b in bytes {
        if *b == b'\n' || *b == b'\r' {
            flush_output(state, backend, line).await;
        } else if line.len() < 2048 {
            line.push(*b);
        }
    }
}

async fn flush_output(state: &Mutex<State>, backend: ClaudeBackend, line: &mut Vec<u8>) {
    let text = String::from_utf8_lossy(line);
    let safe = safe_output(backend, &text);
    if !safe.is_empty() {
        let mut state = state.lock().await;
        if state.output.len() + safe.len() < OUTPUT_LIMIT - 256 && !state.output.contains(&safe) {
            state.output.push('\n');
            state.output.push_str(&safe);
        }
    }
    line.clear();
}

fn safe_output(backend: ClaudeBackend, text: &str) -> String {
    let mut result = Vec::new();
    for word in text.split_whitespace() {
        let word = word.trim_matches(|c: char| matches!(c, '\'' | '"' | '(' | ')' | ',' | '.'));
        if let Ok(url) = url::Url::parse(word) {
            let host = url.host_str().unwrap_or_default();
            let allowed = match backend {
                ClaudeBackend::Bedrock => {
                    host.starts_with("device.sso.")
                        && host.ends_with(".amazonaws.com")
                        && url.path() == "/"
                }
                ClaudeBackend::Foundry => matches!(
                    (host, url.path()),
                    ("microsoft.com" | "www.microsoft.com", "/devicelogin")
                        | ("login.microsoftonline.com", "/common/oauth2/deviceauth")
                ),
                _ => false,
            };
            if allowed
                && url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
            {
                result.push(format!("Login URL: {word}"));
            }
        }
    }
    let lower = text.to_ascii_lowercase();
    let code_text = lower
        .find("enter the code")
        .map(|i| &text[i + "enter the code".len()..])
        .or_else(|| {
            lower
                .find("following code")
                .map(|i| &text[i + "following code".len()..])
        })
        .or_else(|| (backend == ClaudeBackend::Bedrock && text.trim().len() == 9).then_some(text));
    if let Some(code_text) = code_text {
        for word in code_text.split_whitespace().take(2) {
            let code = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
            let aws = code.len() == 9
                && code.as_bytes()[4] == b'-'
                && code
                    .bytes()
                    .enumerate()
                    .all(|(i, b)| i == 4 || b.is_ascii_uppercase() || b.is_ascii_digit());
            let azure = (8..=12).contains(&code.len())
                && code
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
            if (backend == ClaudeBackend::Bedrock && aws)
                || (backend == ClaudeBackend::Foundry && azure)
            {
                result.push(format!("Device code: {code}"));
            }
        }
    }
    result.join("\n")
}

pub async fn poll(
    provider_name: &str,
    transport: &ClaudeTransportConfig,
    session_id: &str,
) -> Result<Value, String> {
    poll_in(sessions(), provider_name, transport, session_id).await
}

async fn poll_in(
    registry: &Mutex<HashMap<String, Session>>,
    provider: &str,
    config: &ClaudeTransportConfig,
    id: &str,
) -> Result<Value, String> {
    let entries = registry.lock().await;
    let session = entries
        .get(id)
        .filter(|s| s.provider == provider && &s.config == config && s.created.elapsed() <= TTL)
        .ok_or(
            "Native sign-in session not found for this provider configuration; start sign-in again",
        )?;
    let state = session.state.lock().await;
    let mut value = json!({"success": state.status == "completed", "status": state.status,
        "auth_status": if state.status == "completed" { "authenticated" } else if state.status == "pending" { "pending" } else { "error" }, "output": state.output});
    if state.status == "pending" {
        value["poll_interval"] = json!(2);
    }
    Ok(value)
}

pub async fn cancel(provider_name: &str, session_id: Option<&str>) -> Result<(), String> {
    cancel_in(sessions(), provider_name, session_id).await
}

async fn cancel_in(
    registry: &Mutex<HashMap<String, Session>>,
    provider: &str,
    id: Option<&str>,
) -> Result<(), String> {
    let mut entries = registry.lock().await;
    if let Some(id) = id {
        if entries.get(id).is_some_and(|s| s.provider != provider) {
            return Err("Native sign-in session belongs to another provider".into());
        }
    }
    let ids: Vec<String> = entries
        .iter()
        .filter(|(key, s)| s.provider == provider && id.is_none_or(|id| key.as_str() == id))
        .map(|(key, _)| key.clone())
        .collect();
    for id in ids {
        if let Some(session) = entries.get_mut(&id) {
            session.stop.store(true, Ordering::Relaxed);
            (&mut session.worker)
                .await
                .map_err(|_| "Native sign-in worker failed during cleanup")?;
        }
        entries.remove(&id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(backend: ClaudeBackend) -> ClaudeTransportConfig {
        ClaudeTransportConfig {
            backend,
            ..Default::default()
        }
    }

    #[test]
    fn argv_profiles_and_override_validation() {
        let mut aws = config(ClaudeBackend::Bedrock);
        let (cli, args, _) = login_spec(&aws, |_| None).unwrap();
        assert_eq!(cli, "aws");
        assert_eq!(
            args,
            [
                "sso",
                "login",
                "--profile",
                "default",
                "--no-browser",
                "--use-device-code"
            ]
        );
        assert_eq!(
            login_spec(&aws, |k| (k == "AWS_PROFILE").then(|| "sso-work".into()))
                .unwrap()
                .1[3],
            "sso-work"
        );
        aws.aws_profile = Some("team.prod".into());
        assert_eq!(login_spec(&aws, |_| None).unwrap().1[3], "team.prod");
        for invalid in ["--debug", "a\nb", "foo;bar", "../profile", "", "a b"] {
            aws.aws_profile = Some(invalid.into());
            assert!(login_spec(&aws, |_| None).is_err());
        }
        aws.aws_profile = None;
        aws.aws_access_key_id = Some("secret".into());
        assert!(login_spec(&aws, |_| None)
            .unwrap_err()
            .contains("Remove explicit"));
        for (backend, name) in [
            (ClaudeBackend::Bedrock, "AWS_ACCESS_KEY_ID"),
            (ClaudeBackend::Vertex, "GOOGLE_APPLICATION_CREDENTIALS"),
            (ClaudeBackend::Foundry, "AZURE_CLIENT_SECRET"),
        ] {
            let error = login_spec(&config(backend), |k| {
                (k == name).then(|| "private-secret".into())
            })
            .unwrap_err();
            assert!(error.contains(name));
            assert!(!error.contains("private-secret"));
        }
        assert_eq!(
            login_spec(&config(ClaudeBackend::Vertex), |_| None)
                .unwrap()
                .1,
            ["auth", "application-default", "login", "--quiet"]
        );
        assert_eq!(
            login_spec(&config(ClaudeBackend::Foundry), |_| None)
                .unwrap()
                .1,
            ["login", "--use-device-code", "--output", "none"]
        );
        assert!(login_spec(&config(ClaudeBackend::Oauth), |_| None).is_err());
    }

    #[test]
    fn output_is_allowlisted_not_raw_credentials() {
        let aws = [
            "https://device.sso.us-east-1.amazonaws.com/",
            "ABCD-EFGH",
            "{\"accessToken\":\"super-secret\"}",
        ]
        .map(|line| safe_output(ClaudeBackend::Bedrock, line))
        .join("\n");
        assert!(aws.contains("Login URL:"));
        assert!(aws.contains("Device code: ABCD-EFGH"));
        assert!(!aws.contains("secret"));
        let azure = safe_output(ClaudeBackend::Foundry, "To sign in, open https://microsoft.com/devicelogin and enter the code A1B2C3D4E to authenticate.");
        assert!(azure.contains("https://microsoft.com/devicelogin"));
        assert!(azure.contains("Device code: A1B2C3D4E"));
        for dangerous in [
            "https://microsoft.com.evil/devicelogin",
            "https://microsoft.com/devicelogin?token=secret",
            "https://evil@microsoft.com/devicelogin",
            "http://microsoft.com/devicelogin",
            "{\"refresh_token\":\"secret\"}",
        ] {
            assert!(safe_output(ClaudeBackend::Foundry, dangerous).is_empty());
        }
        assert!(safe_output(
            ClaudeBackend::Vertex,
            "https://accounts.google.com/?token=secret"
        )
        .is_empty());
    }

    #[test]
    fn executable_path_ignores_relative_empty_and_project_entries() {
        let project = tempfile::tempdir().unwrap();
        let trusted = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join(".git")).unwrap();
        let path = std::env::join_paths([
            Path::new(""),
            Path::new("relative"),
            project.path(),
            trusted.path(),
        ])
        .unwrap();
        let filtered = safe_path(&path, project.path()).unwrap();
        assert_eq!(
            std::env::split_paths(&filtered).collect::<Vec<_>>(),
            vec![trusted.path().canonicalize().unwrap()]
        );
        let error = resolve_cli("refact-test-cli-does-not-exist", &filtered).unwrap_err();
        assert!(error.contains("Install"));
        assert!(error.contains("engine host"));
    }

    #[cfg(unix)]
    #[test]
    fn executable_resolution_preserves_multicall_symlink_name() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("multicall");
        std::fs::write(&target, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = dir.path().join("gcloud");
        symlink(&target, &link).unwrap();
        let resolved = resolve_cli("gcloud", dir.path().as_os_str()).unwrap();
        assert_eq!(resolved, link);
        assert_eq!(resolved.canonicalize().unwrap(), target);
    }

    #[cfg(unix)]
    fn fake(dir: &Path, script: &str) -> Command {
        let executable = dir.join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&executable, script).unwrap();
        let mut command = Command::new("/bin/sh");
        command.arg(executable);
        command
    }

    #[cfg(unix)]
    async fn terminal(
        registry: &Mutex<HashMap<String, Session>>,
        provider: &str,
        cfg: &ClaudeTransportConfig,
        id: &str,
    ) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let result = poll_in(registry, provider, cfg, id).await.unwrap();
                if result["status"] != "pending" {
                    break result;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_process_poll_isolation_and_completion() {
        let registry = Mutex::new(HashMap::new());
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(ClaudeBackend::Foundry);
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut command = fake(dir.path(), "printf '%s\\n' 'https://microsoft.com/devicelogin and enter the code A1B2C3D4E' '{\"accessToken\":\"secret\"}'; read value; exit 0");
        command.arg("separate argument");
        let result = start_command(
            &registry,
            "azure-one",
            &cfg,
            shutdown,
            command,
            "instructions".into(),
            TTL,
        )
        .await
        .unwrap();
        assert_eq!(result["mode"], "native");
        assert_eq!(result["authorize_url"], "");
        let id = result["session_id"].as_str().unwrap();
        assert!(poll_in(&registry, "azure-two", &cfg, id).await.is_err());
        assert!(
            poll_in(&registry, "azure-one", &config(ClaudeBackend::Bedrock), id)
                .await
                .is_err()
        );
        assert!(cancel_in(&registry, "azure-two", Some(id)).await.is_err());
        let result = terminal(&registry, "azure-one", &cfg, id).await;
        assert_eq!(result["success"], true);
        assert_eq!(result["auth_status"], "authenticated");
        assert!(!result["output"].as_str().unwrap().contains("secret"));
        assert_eq!(
            poll_in(&registry, "azure-one", &cfg, id).await.unwrap(),
            result
        );
        cancel_in(&registry, "azure-one", Some(id)).await.unwrap();
        assert!(registry.lock().await.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn duplicate_cancel_timeout_and_shutdown_reap_workers() {
        let registry = Mutex::new(HashMap::new());
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(ClaudeBackend::Bedrock);
        let shutdown = Arc::new(AtomicBool::new(false));
        let make = || fake(dir.path(), "exec /bin/sleep 60");
        let result = start_command(
            &registry,
            "aws-one",
            &cfg,
            shutdown.clone(),
            make(),
            "instructions".into(),
            TTL,
        )
        .await
        .unwrap();
        let id = result["session_id"].as_str().unwrap();
        assert_eq!(
            poll_in(&registry, "aws-one", &cfg, id).await.unwrap()["poll_interval"],
            2
        );
        let previous_state = registry.lock().await.get(id).unwrap().state.clone();
        let replacement = start_command(
            &registry,
            "aws-one",
            &cfg,
            shutdown.clone(),
            make(),
            "instructions".into(),
            TTL,
        )
        .await
        .unwrap();
        assert_eq!(previous_state.lock().await.status, "failed");
        assert!(poll_in(&registry, "aws-one", &cfg, id).await.is_err());
        let replacement_id = replacement["session_id"].as_str().unwrap();
        assert_ne!(id, replacement_id);
        cancel_in(&registry, "aws-one", Some(id)).await.unwrap();
        assert_eq!(
            poll_in(&registry, "aws-one", &cfg, replacement_id)
                .await
                .unwrap()["status"],
            "pending"
        );
        let state = registry
            .lock()
            .await
            .get(replacement_id)
            .unwrap()
            .state
            .clone();
        cancel_in(&registry, "aws-one", Some(replacement_id))
            .await
            .unwrap();
        assert_eq!(state.lock().await.status, "failed");
        assert!(registry.lock().await.is_empty());
        let result = start_command(
            &registry,
            "timeout",
            &cfg,
            shutdown.clone(),
            make(),
            "instructions".into(),
            Duration::from_millis(1),
        )
        .await
        .unwrap();
        let id = result["session_id"].as_str().unwrap();
        assert!(terminal(&registry, "timeout", &cfg, id).await["output"]
            .as_str()
            .unwrap()
            .contains("timed out"));
        let result = start_command(
            &registry,
            "shutdown",
            &cfg,
            shutdown.clone(),
            make(),
            "instructions".into(),
            TTL,
        )
        .await
        .unwrap();
        let id = result["session_id"].as_str().unwrap();
        shutdown.store(true, Ordering::Relaxed);
        assert_eq!(
            terminal(&registry, "shutdown", &cfg, id).await["status"],
            "failed"
        );
        cancel_in(&registry, "timeout", None).await.unwrap();
        cancel_in(&registry, "shutdown", None).await.unwrap();
        assert!(registry.lock().await.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_retention_capacity_and_partial_device_output() {
        let registry = Mutex::new(HashMap::new());
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(ClaudeBackend::Foundry);
        let shutdown = Arc::new(AtomicBool::new(false));
        let first = start_command(
            &registry,
            "first",
            &cfg,
            shutdown.clone(),
            fake(dir.path(), "exit 0"),
            "instructions".into(),
            TTL,
        )
        .await
        .unwrap();
        let id = first["session_id"].as_str().unwrap();
        terminal(&registry, "first", &cfg, id).await;
        for index in 1..LIMIT {
            start_command(
                &registry,
                &format!("provider-{index}"),
                &cfg,
                shutdown.clone(),
                fake(dir.path(), "exit 0"),
                "instructions".into(),
                TTL,
            )
            .await
            .unwrap();
        }
        assert_eq!(
            poll_in(&registry, "first", &cfg, id).await.unwrap()["status"],
            "completed"
        );
        assert!(start_command(
            &registry,
            "over-limit",
            &cfg,
            shutdown,
            fake(dir.path(), "exit 0"),
            "instructions".into(),
            TTL
        )
        .await
        .unwrap_err()
        .contains("Too many"));
        let state = Mutex::new(State {
            status: "pending",
            output: String::new(),
        });
        let mut line = Vec::new();
        for part in [
            b"To sign in, open https://micro".as_slice(),
            b"soft.com/devicelogin and enter the co",
            b"de ABCDEFGHI to authenticate.\n",
        ] {
            capture(&state, ClaudeBackend::Foundry, &mut line, part).await;
        }
        let output = state.lock().await.output.clone();
        assert!(output.contains("Device code: ABCDEFGHI"));
        assert!(output.contains("https://microsoft.com/devicelogin"));
        cancel_in(&registry, "first", None).await.unwrap();
        for index in 1..LIMIT {
            cancel_in(&registry, &format!("provider-{index}"), None)
                .await
                .unwrap();
        }
        assert!(registry.lock().await.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_process_is_reaped_without_global_logout() {
        let registry = Mutex::new(HashMap::new());
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("pid");
        let mut command = fake(dir.path(), "printf '%s' \"$$\" > \"$RECORD\"; printf '%s\\n' 'https://microsoft.com/devicelogin and enter the code ABCDEFGHI'; exec /bin/sleep 60");
        command.env("RECORD", &record);
        let cfg = config(ClaudeBackend::Foundry);
        let result = start_command(
            &registry,
            "cancel",
            &cfg,
            Arc::new(AtomicBool::new(false)),
            command,
            "instructions".into(),
            TTL,
        )
        .await
        .unwrap();
        let id = result["session_id"].as_str().unwrap();
        let pid: i32 = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(&record) {
                    if let Ok(pid) = pid.parse() {
                        break pid;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if poll_in(&registry, "cancel", &cfg, id).await.unwrap()["output"]
                    .as_str()
                    .unwrap()
                    .contains("Device code: ABCDEFGHI")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        cancel_in(&registry, "cancel", None).await.unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        assert!(registry.lock().await.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn argv_cwd_output_bounds_and_cli_failure() {
        let registry = Mutex::new(HashMap::new());
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(ClaudeBackend::Bedrock);
        let shutdown = Arc::new(AtomicBool::new(false));
        let record = dir.path().join("argv");
        let mut command = fake(
            dir.path(),
            "printf '%s\\n' \"$PWD\" \"$@\" > \"$RECORD\"; exit 7",
        );
        command
            .env("RECORD", &record)
            .args(["--profile", "team.prod", "literal;not-shell"]);
        let result = start_command(
            &registry,
            "failure",
            &cfg,
            shutdown.clone(),
            command,
            "instructions".into(),
            TTL,
        )
        .await
        .unwrap();
        let id = result["session_id"].as_str().unwrap();
        let result = terminal(&registry, "failure", &cfg, id).await;
        assert_eq!(result["status"], "failed");
        assert_eq!(result["success"], false);
        let record = std::fs::read_to_string(record).unwrap();
        let lines: Vec<_> = record.lines().collect();
        assert!(lines[0].contains("refact-native-auth-"));
        assert_eq!(
            &lines[1..],
            &["--profile", "team.prod", "literal;not-shell"]
        );
        let state = Mutex::new(State {
            status: "pending",
            output: String::new(),
        });
        let mut line = Vec::new();
        capture(
            &state,
            ClaudeBackend::Bedrock,
            &mut line,
            &vec![b'x'; 100_000],
        )
        .await;
        assert!(line.len() <= 2048);
        for _ in 0..1000 {
            capture(&state, ClaudeBackend::Bedrock, &mut line, b"\nABCD-EFGH\n").await;
        }
        assert!(state.lock().await.output.len() < OUTPUT_LIMIT);
        let result = start_command(
            &registry,
            "missing",
            &cfg,
            shutdown,
            Command::new(dir.path().join("missing")),
            "instructions".into(),
            TTL,
        )
        .await;
        assert!(result.unwrap_err().contains("Cannot start"));
        cancel_in(&registry, "failure", None).await.unwrap();
    }
}
