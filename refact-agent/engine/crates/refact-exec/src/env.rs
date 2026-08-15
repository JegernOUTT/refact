use std::collections::{BTreeMap, HashMap};

use portable_pty::CommandBuilder;

use crate::types::{ExecEnvPolicy, EXEC_ENV_DEFAULTS};

/// Parent environment variables retained by scrubbed child processes.
#[cfg(not(windows))]
pub const BASE_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "SSH_AUTH_SOCK",
    "XDG_RUNTIME_DIR",
    "DISPLAY",
    "WAYLAND_DISPLAY",
];

/// Parent environment variables retained by scrubbed child processes.
#[cfg(windows)]
pub const BASE_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "SSH_AUTH_SOCK",
    "XDG_RUNTIME_DIR",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "SystemRoot",
    "SystemDrive",
    "ComSpec",
    "windir",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramData",
    "PATHEXT",
    "NUMBER_OF_PROCESSORS",
    "OS",
];

#[cfg(not(windows))]
const FALLBACK_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

fn is_secret_name(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    name.ends_with("_API_KEY")
        || name.ends_with("_TOKEN")
        || name.ends_with("_SECRET")
        || name.ends_with("_PASSWORD")
        || name.starts_with("AWS_")
        || name.starts_with("OPENAI_")
        || name.starts_with("ANTHROPIC_")
        || name.starts_with("GOOGLE_APPLICATION_")
}

#[cfg(not(windows))]
fn passthrough_matches(pattern: &str, name: &str) -> bool {
    pattern
        .strip_suffix('*')
        .map_or(pattern == name, |prefix| name.starts_with(prefix))
}

fn utf8_parent_env() -> impl Iterator<Item = (String, String)> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
}

#[cfg(not(windows))]
fn insert_env(env: &mut BTreeMap<String, String>, name: String, value: String) {
    env.insert(name, value);
}

#[cfg(windows)]
fn insert_env(env: &mut BTreeMap<String, String>, name: String, value: String) {
    if let Some(existing) = env
        .keys()
        .find(|existing| existing.eq_ignore_ascii_case(&name))
        .cloned()
    {
        env.remove(&existing);
    }
    env.insert(name, value);
}

#[cfg(windows)]
fn passthrough_matches(pattern: &str, name: &str) -> bool {
    pattern.strip_suffix('*').map_or_else(
        || pattern.eq_ignore_ascii_case(name),
        |prefix| {
            name.get(..prefix.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        },
    )
}

pub fn build_child_env(
    policy: &ExecEnvPolicy,
    request_env: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut child_env = match policy {
        ExecEnvPolicy::Inherit => {
            let mut child_env = BTreeMap::new();
            for (name, value) in utf8_parent_env() {
                insert_env(&mut child_env, name, value);
            }
            child_env
        }
        ExecEnvPolicy::Scrubbed { .. } => {
            let mut child_env = BTreeMap::new();
            for name in BASE_ENV_ALLOWLIST {
                if let Ok(value) = std::env::var(name) {
                    insert_env(&mut child_env, (*name).to_string(), value);
                }
            }
            #[cfg(not(windows))]
            if !child_env.contains_key("PATH") {
                insert_env(
                    &mut child_env,
                    "PATH".to_string(),
                    FALLBACK_PATH.to_string(),
                );
            }
            child_env
        }
    };

    for (name, value) in EXEC_ENV_DEFAULTS {
        insert_env(&mut child_env, (*name).to_string(), (*value).to_string());
    }
    if let ExecEnvPolicy::Scrubbed { passthrough } = policy {
        for (name, value) in utf8_parent_env() {
            if passthrough
                .iter()
                .any(|pattern| passthrough_matches(pattern, &name))
                && !is_secret_name(&name)
            {
                insert_env(&mut child_env, name, value);
            }
        }
    }
    for (name, value) in request_env {
        insert_env(&mut child_env, name.clone(), value.clone());
    }
    child_env.into_iter().collect()
}

pub(crate) fn apply_tokio_child_env(
    command: &mut tokio::process::Command,
    policy: &ExecEnvPolicy,
    request_env: &HashMap<String, String>,
) {
    match policy {
        ExecEnvPolicy::Inherit => {
            for (name, value) in EXEC_ENV_DEFAULTS {
                command.env(name, value);
            }
            for (name, value) in request_env {
                command.env(name, value);
            }
        }
        ExecEnvPolicy::Scrubbed { .. } => {
            command.env_clear();
            for (name, value) in build_child_env(policy, request_env) {
                command.env(name, value);
            }
        }
    }
}

pub(crate) fn apply_pty_child_env(
    command: &mut CommandBuilder,
    policy: &ExecEnvPolicy,
    request_env: &HashMap<String, String>,
) {
    match policy {
        ExecEnvPolicy::Inherit => {
            for (name, value) in EXEC_ENV_DEFAULTS {
                command.env(name, value);
            }
            for (name, value) in request_env {
                command.env(name, value);
            }
        }
        ExecEnvPolicy::Scrubbed { .. } => {
            command.env_clear();
            for (name, value) in build_child_env(policy, request_env) {
                command.env(name, value);
            }
        }
    }
}
