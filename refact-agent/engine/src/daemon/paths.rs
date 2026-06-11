use std::path::PathBuf;

fn home_dir() -> PathBuf {
    home::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn daemon_cache_root() -> PathBuf {
    #[cfg(test)]
    if let Ok(path) = std::env::var("REFACT_DAEMON_CACHE_DIR") {
        return PathBuf::from(path);
    }
    home_dir().join(".cache").join("refact")
}

fn daemon_config_root() -> PathBuf {
    #[cfg(test)]
    if let Ok(path) = std::env::var("REFACT_DAEMON_CONFIG_DIR") {
        return PathBuf::from(path);
    }
    home_dir().join(".config").join("refact")
}

pub fn daemon_dir() -> PathBuf {
    daemon_cache_root().join("daemon")
}

pub fn lock_path() -> PathBuf {
    daemon_dir().join("daemon.lock")
}

pub fn daemon_json_path() -> PathBuf {
    daemon_dir().join("daemon.json")
}

pub fn events_jsonl_path() -> PathBuf {
    daemon_dir().join("events.jsonl")
}

pub fn rotated_events_jsonl_path() -> PathBuf {
    daemon_dir().join("events.jsonl.1")
}

pub fn logs_dir() -> PathBuf {
    daemon_dir().join("logs")
}

pub fn daemon_log_path() -> PathBuf {
    logs_dir().join("daemon.log")
}

pub fn projects_json_path() -> PathBuf {
    daemon_dir().join("projects.json")
}

pub fn daemon_config_path() -> PathBuf {
    daemon_config_root().join("daemon.yaml")
}
