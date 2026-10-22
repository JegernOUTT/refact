use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static PROCESS_ROOT: OnceLock<PathBuf> = OnceLock::new();
static PROCESS_MARKER: OnceLock<String> = OnceLock::new();
const ROOT_PREFIX: &str = "refact-tests-";
#[cfg(not(target_os = "linux"))]
use std::time::Duration;
#[cfg(not(target_os = "linux"))]
const STALE_ROOT_AGE: Duration = Duration::from_secs(24 * 60 * 60);

pub fn process_root() -> &'static Path {
    PROCESS_ROOT
        .get_or_init(|| {
            let root = std::env::temp_dir().join(format!(
                "{ROOT_PREFIX}{}-{}",
                std::process::id(),
                process_start_marker()
            ));
            sweep_dead_roots_except(Some(&root));
            std::fs::create_dir_all(&root).expect("create test process temp root");
            root
        })
        .as_path()
}

pub fn scratch_dir(prefix: &str) -> PathBuf {
    let path = process_root().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).expect("create test scratch directory");
    path
}

fn sweep_dead_roots() {
    sweep_dead_roots_except(PROCESS_ROOT.get().map(PathBuf::as_path));
}

fn sweep_dead_roots_except(current_root: Option<&Path>) {
    let temp_dir = std::env::temp_dir();
    let entries = match std::fs::read_dir(&temp_dir) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::debug!(
                "cannot scan test temp roots in {}: {error}",
                temp_dir.display()
            );
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!("cannot read test temp root entry: {error}");
                continue;
            }
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(ROOT_PREFIX) || current_root.is_some_and(|path| path == entry.path()) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                tracing::debug!(
                    "cannot inspect test temp root {}: {error}",
                    entry.path().display()
                );
                continue;
            }
        };
        if !file_type.is_dir() && !file_type.is_symlink() {
            continue;
        }
        if !root_is_dead(name, &entry.path()) {
            continue;
        }

        let result = if file_type.is_symlink() {
            std::fs::remove_file(entry.path())
        } else {
            std::fs::remove_dir_all(entry.path())
        };
        if let Err(error) = result {
            tracing::debug!(
                "cannot remove stale test temp root {}: {error}",
                entry.path().display()
            );
        }
    }
}

fn root_is_dead(name: &str, _path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Some((pid, marker)) = parse_root_name(name) else {
            return false;
        };
        let observed_marker = if pid == std::process::id() {
            Some(process_start_marker())
        } else {
            process_start_marker_for_pid(pid)
        };
        observed_marker.as_deref() != Some(marker)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let Ok(metadata) = std::fs::symlink_metadata(_path) else {
            return false;
        };
        let Ok(modified) = metadata.modified() else {
            return false;
        };
        std::time::SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age >= STALE_ROOT_AGE)
    }
}

#[cfg(target_os = "linux")]
fn parse_root_name(name: &str) -> Option<(u32, &str)> {
    let suffix = name.strip_prefix(ROOT_PREFIX)?;
    let (pid, marker) = suffix.split_once('-')?;
    Some((pid.parse().ok()?, marker))
}

fn process_start_marker() -> String {
    PROCESS_MARKER
        .get_or_init(|| {
            #[cfg(target_os = "linux")]
            if let Some(marker) = process_start_marker_for_pid(std::process::id()) {
                return marker;
            }
            uuid::Uuid::new_v4().to_string()
        })
        .clone()
}

#[cfg(target_os = "linux")]
fn process_start_marker_for_pid(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_command = stat.rsplit_once(')')?.1;
    after_command.split_whitespace().nth(19).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_dirs_live_under_the_process_root_and_are_unique() {
        let first = scratch_dir("cache");
        let second = scratch_dir("cache");

        assert!(first.starts_with(process_root()));
        assert!(second.starts_with(process_root()));
        assert_ne!(first, second);
        assert!(std::fs::read_dir(first).unwrap().next().is_none());
        assert!(std::fs::read_dir(second).unwrap().next().is_none());
    }

    #[test]
    fn sweep_removes_a_dead_process_root_and_keeps_live_ones() {
        let current_root = process_root().to_path_buf();
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" });
        if cfg!(windows) {
            child.args(["/C", "exit", "0"]);
        }
        let child = child.spawn().unwrap();
        let pid = child.id();
        child.wait_with_output().unwrap();
        let dead_root = fake_root(pid, "test-marker");
        std::fs::create_dir_all(&dead_root).unwrap();
        let recycled_pid_root = fake_root(std::process::id(), "different-marker");
        std::fs::create_dir_all(&recycled_pid_root).unwrap();
        #[cfg(not(target_os = "linux"))]
        {
            mark_old(&dead_root);
            mark_old(&recycled_pid_root);
        }

        sweep_dead_roots();

        assert!(!dead_root.exists());
        assert!(!recycled_pid_root.exists());
        assert!(current_root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn sweep_never_follows_a_symlink() {
        use std::os::unix::fs::symlink;

        let child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait_with_output().unwrap();
        let victim =
            std::env::temp_dir().join(format!("test-temp-victim-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&victim).unwrap();
        let victim_file = victim.join("survives");
        std::fs::write(&victim_file, "safe").unwrap();
        let link = fake_root(pid, "test-marker");
        symlink(&victim, &link).unwrap();
        #[cfg(not(target_os = "linux"))]
        mark_old(&link);

        sweep_dead_roots();

        assert!(!link.exists());
        assert_eq!(std::fs::read_to_string(&victim_file).unwrap(), "safe");
        std::fs::remove_dir_all(victim).unwrap();
    }

    fn fake_root(pid: u32, marker: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{ROOT_PREFIX}{pid}-{marker}"))
    }

    #[cfg(not(target_os = "linux"))]
    fn mark_old(path: &Path) {
        let old = filetime::FileTime::from_system_time(
            std::time::SystemTime::now() - STALE_ROOT_AGE - Duration::from_secs(1),
        );
        if std::fs::symlink_metadata(path)
            .unwrap()
            .file_type()
            .is_symlink()
        {
            filetime::set_symlink_file_times(path, old, old).unwrap();
        } else {
            filetime::set_file_mtime(path, old).unwrap();
        }
    }
}
