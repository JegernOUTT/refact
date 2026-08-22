use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use process_wrap::tokio::{KillOnDrop, TokioCommandWrap};
#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use refact_core::provider_types::CredentialSpec;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;

const OUTPUT_LIMIT: usize = 16 * 1024;
const SAFE_ENV: &[&str] = &[
    "HOME",
    "PATH",
    "TMPDIR",
    "TMP",
    "TEMP",
    "USER",
    "LOGNAME",
    "SystemRoot",
    "ComSpec",
    "PATHEXT",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "USERNAME",
];

#[derive(Default)]
struct CacheEntry {
    value: Option<(String, Instant)>,
}

type CacheKey = (String, CredentialSpec);
type SharedEntry = Arc<Mutex<CacheEntry>>;

fn cache() -> &'static StdMutex<HashMap<CacheKey, SharedEntry>> {
    static CACHE: OnceLock<StdMutex<HashMap<CacheKey, SharedEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn cache_entry(provider_id: &str, spec: &CredentialSpec) -> Result<SharedEntry, String> {
    let key = (provider_id.to_string(), spec.clone());
    let mut entries = cache()
        .lock()
        .map_err(|_| "credential cache unavailable".to_string())?;
    entries.retain(|(id, cached_spec), _| id != provider_id || cached_spec == spec);
    Ok(entries
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(CacheEntry::default())))
        .clone())
}

pub async fn resolve(
    provider_id: &str,
    spec: &CredentialSpec,
    force_refresh: bool,
) -> Result<String, String> {
    spec.validate()?;
    let requested_at = Instant::now();
    let entry = cache_entry(provider_id, spec)?;
    let mut entry = entry.lock().await;
    if let Some((value, created)) = &entry.value {
        if force_refresh {
            if *created >= requested_at {
                return Ok(value.clone());
            }
        } else {
            if created.elapsed() < Duration::from_millis(spec.refresh_interval_ms()) {
                return Ok(value.clone());
            }
        }
    }
    let value = execute(spec).await?;
    entry.value = Some((value.clone(), Instant::now()));
    Ok(value)
}

pub async fn refresh_after_rejection(
    provider_id: &str,
    spec: &CredentialSpec,
    rejected_value: &str,
) -> Result<String, String> {
    spec.validate()?;
    let entry = cache_entry(provider_id, spec)?;
    let mut entry = entry.lock().await;
    if let Some((value, _)) = &entry.value {
        if value != rejected_value {
            return Ok(value.clone());
        }
    }
    let value = execute(spec).await?;
    entry.value = Some((value.clone(), Instant::now()));
    Ok(value)
}

pub fn invalidate(provider_id: &str) {
    if let Ok(mut entries) = cache().lock() {
        entries.retain(|(id, _), _| id != provider_id);
    }
}

pub fn invalidate_all() {
    if let Ok(mut entries) = cache().lock() {
        entries.clear();
    }
}

async fn execute(spec: &CredentialSpec) -> Result<String, String> {
    let CredentialSpec::Command {
        command,
        args,
        cwd,
        env_passthrough,
        ..
    } = spec;

    let mut command_process = tokio::process::Command::new(command);
    command_process
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    if let Some(cwd) = cwd {
        command_process.current_dir(cwd);
    }
    for (name, value) in std::env::vars_os() {
        let name_text = name.to_string_lossy();
        let safe_env = if cfg!(windows) {
            SAFE_ENV
                .iter()
                .any(|safe| name_text.eq_ignore_ascii_case(safe))
        } else {
            SAFE_ENV.iter().any(|safe| name_text == *safe)
        };
        if safe_env
            || env_passthrough
                .iter()
                .any(|pattern| wildcard_match(pattern, &name_text))
        {
            command_process.env(name, value);
        }
    }

    let mut process = TokioCommandWrap::from(command_process);
    process.wrap(KillOnDrop);
    #[cfg(unix)]
    process.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    process.wrap(JobObject);
    let mut child = process
        .spawn()
        .map_err(|_| "credential command could not be started".to_string())?;
    let stdout = child
        .stdout()
        .take()
        .ok_or_else(|| "credential command output unavailable".to_string())?;
    let stderr = child
        .stderr()
        .take()
        .ok_or_else(|| "credential command output unavailable".to_string())?;
    let execution = async {
        let wait = async {
            Box::into_pin(child.wait())
                .await
                .map_err(|_| "credential command failed while waiting".to_string())
        };
        tokio::try_join!(wait, read_capped(stdout), read_capped(stderr))
    };
    let (status, stdout, stderr) =
        match tokio::time::timeout(Duration::from_millis(spec.timeout_ms()), execution).await {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => {
                let _ = Box::into_pin(child.kill()).await;
                return Err(error);
            }
            Err(_) => {
                let _ = Box::into_pin(child.kill()).await;
                return Err("credential command timed out".to_string());
            }
        };
    if stdout.len() > OUTPUT_LIMIT {
        return Err("credential command stdout exceeded the size limit".to_string());
    }
    if stderr.len() > OUTPUT_LIMIT {
        return Err("credential command stderr exceeded the size limit".to_string());
    }
    if !status.success() {
        return Err("credential command exited unsuccessfully".to_string());
    }
    let value = String::from_utf8(stdout)
        .map_err(|_| "credential command stdout was not valid UTF-8".to_string())?;
    let value = value.trim();
    if value.is_empty() {
        return Err("credential command returned an empty credential".to_string());
    }
    Ok(value.to_string())
}

async fn read_capped<R: tokio::io::AsyncRead + Unpin>(mut reader: R) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(OUTPUT_LIMIT + 1);
    let mut chunk = [0u8; 4096];
    loop {
        let count = reader
            .read(&mut chunk)
            .await
            .map_err(|_| "credential command output could not be read".to_string())?;
        if count == 0 {
            break;
        }
        if bytes.len() <= OUTPUT_LIMIT {
            let remaining = OUTPUT_LIMIT + 1 - bytes.len();
            bytes.extend_from_slice(&chunk[..count.min(remaining)]);
        }
    }
    Ok(bytes)
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == value;
    }
    let mut remainder = value;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if index == 0 && !pattern.starts_with('*') {
            let Some(next) = remainder.strip_prefix(part) else {
                return false;
            };
            remainder = next;
        } else if let Some(position) = remainder.find(part) {
            remainder = &remainder[position + part.len()..];
        } else {
            return false;
        }
    }
    pattern.ends_with('*') || remainder.is_empty()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use refact_core::provider_types::{
        DEFAULT_CREDENTIAL_REFRESH_INTERVAL_MS, DEFAULT_CREDENTIAL_TIMEOUT_MS,
    };

    fn shell_spec(script: &str) -> CredentialSpec {
        CredentialSpec::Command {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
            timeout_ms: DEFAULT_CREDENTIAL_TIMEOUT_MS,
            refresh_interval_ms: DEFAULT_CREDENTIAL_REFRESH_INTERVAL_MS,
            cwd: None,
            env_passthrough: Vec::new(),
        }
    }

    #[tokio::test]
    async fn resolves_and_trims_output() {
        assert_eq!(
            resolve("trim", &shell_spec("printf ' token \\n'"), false)
                .await
                .unwrap(),
            "token"
        );
    }

    #[tokio::test]
    async fn caches_and_force_refreshes() {
        let dir = tempfile::tempdir().unwrap();
        let count = dir.path().join("count");
        let script = format!(
            "n=$(cat '{}' 2>/dev/null || echo 0); n=$((n+1)); echo $n > '{}'; echo $n",
            count.display(),
            count.display()
        );
        let spec = shell_spec(&script);
        assert_eq!(resolve("cache", &spec, false).await.unwrap(), "1");
        assert_eq!(resolve("cache", &spec, false).await.unwrap(), "1");
        assert_eq!(resolve("cache", &spec, true).await.unwrap(), "2");
    }

    #[tokio::test]
    async fn concurrent_force_refreshes_are_singleflight() {
        let dir = tempfile::tempdir().unwrap();
        let count = dir.path().join("force-count");
        let script = format!(
            "n=$(cat '{}' 2>/dev/null || echo 0); n=$((n+1)); sleep 0.1; echo $n > '{}'; echo $n",
            count.display(),
            count.display()
        );
        let spec = Arc::new(shell_spec(&script));
        let first = tokio::spawn({
            let spec = spec.clone();
            async move { resolve("force", &spec, true).await }
        });
        let second = tokio::spawn({
            let spec = spec.clone();
            async move { resolve("force", &spec, true).await }
        });
        assert_eq!(first.await.unwrap().unwrap(), "1");
        assert_eq!(second.await.unwrap().unwrap(), "1");
    }

    #[tokio::test]
    async fn concurrent_calls_are_singleflight() {
        let spec = Arc::new(shell_spec("sleep 0.1; printf token"));
        let a = tokio::spawn({
            let spec = spec.clone();
            async move { resolve("same", &spec, false).await }
        });
        let b = tokio::spawn({
            let spec = spec.clone();
            async move { resolve("same", &spec, false).await }
        });
        assert_eq!(a.await.unwrap().unwrap(), "token");
        assert_eq!(b.await.unwrap().unwrap(), "token");
    }

    #[tokio::test]
    async fn concurrent_rejection_refreshes_are_singleflight() {
        let dir = tempfile::tempdir().unwrap();
        let count = dir.path().join("refresh-count");
        let script = format!(
            "n=$(cat '{}' 2>/dev/null || echo 0); n=$((n+1)); echo $n > '{}'; echo $n",
            count.display(),
            count.display()
        );
        let spec = Arc::new(shell_spec(&script));
        assert_eq!(resolve("refresh", &spec, false).await.unwrap(), "1");
        let first = tokio::spawn({
            let spec = spec.clone();
            async move { refresh_after_rejection("refresh", &spec, "1").await }
        });
        let second = tokio::spawn({
            let spec = spec.clone();
            async move { refresh_after_rejection("refresh", &spec, "1").await }
        });
        assert_eq!(first.await.unwrap().unwrap(), "2");
        assert_eq!(second.await.unwrap().unwrap(), "2");
    }

    #[tokio::test]
    async fn refreshes_after_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let count = dir.path().join("expiry-count");
        let script = format!(
            "n=$(cat '{}' 2>/dev/null || echo 0); n=$((n+1)); echo $n > '{}'; echo $n",
            count.display(),
            count.display()
        );
        let mut spec = shell_spec(&script);
        let CredentialSpec::Command {
            refresh_interval_ms,
            ..
        } = &mut spec;
        *refresh_interval_ms = 1_000;
        assert_eq!(resolve("expiry", &spec, false).await.unwrap(), "1");
        tokio::time::sleep(Duration::from_millis(1_050)).await;
        assert_eq!(resolve("expiry", &spec, false).await.unwrap(), "2");
    }

    #[tokio::test]
    async fn reports_timeout_nonzero_empty_and_oversized_without_output() {
        let mut timeout = shell_spec("sleep 1; echo secret");
        let CredentialSpec::Command { timeout_ms, .. } = &mut timeout;
        *timeout_ms = 100;
        assert!(resolve("timeout", &timeout, false)
            .await
            .unwrap_err()
            .contains("timed out"));
        assert!(
            resolve("nonzero", &shell_spec("echo secret >&2; exit 2"), false)
                .await
                .unwrap_err()
                .contains("unsuccessfully")
        );
        assert!(resolve("empty", &shell_spec("printf '  '"), false)
            .await
            .unwrap_err()
            .contains("empty"));
        assert!(resolve("non-utf8", &shell_spec("printf '\\377'"), false)
            .await
            .unwrap_err()
            .contains("UTF-8"));
        let error = resolve("large", &shell_spec("head -c 20000 /dev/zero"), false)
            .await
            .unwrap_err();
        assert!(error.contains("size limit"));
        assert!(!error.contains("secret"));
        assert!(resolve(
            "large-err",
            &shell_spec("head -c 20000 /dev/zero >&2"),
            false
        )
        .await
        .unwrap_err()
        .contains("stderr"));
    }

    #[tokio::test]
    async fn scrubs_environment_and_allows_explicit_passthrough() {
        std::env::set_var("REFACT_CREDENTIAL_TEST_SECRET", "visible");
        let scrubbed = shell_spec("printf ${REFACT_CREDENTIAL_TEST_SECRET:-scrubbed}");
        assert_eq!(
            resolve("env-a", &scrubbed, false).await.unwrap(),
            "scrubbed"
        );
        let mut passed = scrubbed;
        let CredentialSpec::Command {
            env_passthrough, ..
        } = &mut passed;
        env_passthrough.push("REFACT_CREDENTIAL_TEST_*".to_string());
        assert_eq!(resolve("env-b", &passed, false).await.unwrap(), "visible");
        std::env::remove_var("REFACT_CREDENTIAL_TEST_SECRET");
    }

    #[test]
    fn wildcard_patterns_are_anchored() {
        assert!(wildcard_match("AWS_*", "AWS_PROFILE"));
        assert!(wildcard_match("*_TOKEN", "MY_TOKEN"));
        assert!(!wildcard_match("AWS_*", "X_AWS_PROFILE"));
    }
}
