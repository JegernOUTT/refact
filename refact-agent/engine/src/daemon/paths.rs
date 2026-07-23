use std::path::PathBuf;

pub const DAEMON_DIR_ENV: &str = "REFACT_DAEMON_DIR";

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

pub fn cache_root() -> PathBuf {
    daemon_cache_root()
}

fn daemon_config_root() -> PathBuf {
    #[cfg(test)]
    if let Ok(path) = std::env::var("REFACT_DAEMON_CONFIG_DIR") {
        return PathBuf::from(path);
    }
    home_dir().join(".config").join("refact")
}

fn daemon_dir_override() -> Option<PathBuf> {
    std::env::var_os(DAEMON_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn daemon_dir() -> PathBuf {
    daemon_dir_override().unwrap_or_else(|| daemon_cache_root().join("daemon"))
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
    daemon_dir_override()
        .map(|path| path.join("daemon.yaml"))
        .unwrap_or_else(|| daemon_config_root().join("daemon.yaml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &std::path::Path) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn set_raw(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn clear(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    #[serial]
    fn daemon_dir_env_override_isolates_daemon_state_paths() {
        let tempdir = tempfile::tempdir().unwrap();
        let _guard = EnvVarGuard::set(DAEMON_DIR_ENV, tempdir.path());
        assert_eq!(daemon_dir(), tempdir.path());
        assert_eq!(daemon_json_path(), tempdir.path().join("daemon.json"));
        assert_eq!(projects_json_path(), tempdir.path().join("projects.json"));
        assert_eq!(lock_path(), tempdir.path().join("daemon.lock"));
        assert_eq!(events_jsonl_path(), tempdir.path().join("events.jsonl"));
        assert_eq!(rotated_events_jsonl_path(), tempdir.path().join("events.jsonl.1"));
        assert_eq!(daemon_log_path(), tempdir.path().join("logs").join("daemon.log"));
        assert_eq!(daemon_config_path(), tempdir.path().join("daemon.yaml"));
    }

    #[test]
    #[serial]
    fn daemon_dir_defaults_under_cache_root_without_override() {
        let cache = tempfile::tempdir().unwrap();
        let _cache_guard = EnvVarGuard::set("REFACT_DAEMON_CACHE_DIR", cache.path());
        let _dir_guard = EnvVarGuard::clear(DAEMON_DIR_ENV);
        let expected = cache.path().join("daemon");
        assert_eq!(daemon_dir(), expected);
        assert_eq!(projects_json_path(), expected.join("projects.json"));
        assert_eq!(daemon_json_path(), expected.join("daemon.json"));
    }

    #[test]
    #[serial]
    fn empty_daemon_dir_env_falls_back_to_cache_root() {
        let cache = tempfile::tempdir().unwrap();
        let _cache_guard = EnvVarGuard::set("REFACT_DAEMON_CACHE_DIR", cache.path());
        let _dir_guard = EnvVarGuard::set_raw(DAEMON_DIR_ENV, "");
        assert_eq!(daemon_dir(), cache.path().join("daemon"));
    }
}
