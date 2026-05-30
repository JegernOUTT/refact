use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use crate::exec::types::ExecProcessId;

#[derive(Debug, Clone)]
pub struct SpillTarget {
    root: PathBuf,
    chat_component: String,
    process_component: String,
}

impl SpillTarget {
    pub fn new(chat_id: &str, process_id: &ExecProcessId) -> Result<Self, String> {
        let root = default_spill_root()?;
        Ok(Self::with_root(root, chat_id, process_id))
    }

    pub fn with_root(root: PathBuf, chat_id: &str, process_id: &ExecProcessId) -> Self {
        Self {
            root,
            chat_component: safe_spill_component("chat", chat_id),
            process_component: safe_spill_component("process", process_id.as_str()),
        }
    }

    pub fn dir(&self) -> PathBuf {
        self.root.join(&self.chat_component)
    }

    pub fn path(&self) -> PathBuf {
        self.dir().join(format!("{}.log", self.process_component))
    }
}

pub struct SpillWriter {
    path: PathBuf,
    file: tokio::fs::File,
}

impl SpillWriter {
    pub async fn create(target: &SpillTarget) -> Result<Self, String> {
        create_private_dir_all(&target.root).await?;
        let dir = target.dir();
        create_private_dir_all(&dir).await?;
        let path = target.path();
        ensure_safe_spill_file_path(&path).await?;
        let mut options = tokio::fs::OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        let file = options
            .open(&path)
            .await
            .map_err(|error| format!("failed to open exec spill log: {error}"))?;
        Ok(Self { path, file })
    }

    pub async fn write_line(&mut self, line: &str) -> Result<(), String> {
        self.file
            .write_all(line.as_bytes())
            .await
            .map_err(|error| format!("failed to write exec spill log: {error}"))?;
        self.file
            .flush()
            .await
            .map_err(|error| format!("failed to flush exec spill log: {error}"))
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
async fn validate_private_dir(path: &Path) -> Result<(), String> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|error| format!("failed to inspect exec spill directory: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "exec spill directory is not a real directory: {}",
            path.display()
        ));
    }
    if metadata.uid() != current_uid() {
        return Err(format!(
            "exec spill directory is owned by another user: {}",
            path.display()
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(|error| {
                format!(
                    "failed to secure exec spill directory {}: {error}",
                    path.display()
                )
            })?;
        let metadata = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|error| format!("failed to inspect exec spill directory: {error}"))?;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "exec spill directory has unsafe permissions: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn create_private_dir_all(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if current.as_os_str().is_empty() || current.exists() {
            continue;
        }
        let result = tokio::fs::DirBuilder::new()
            .mode(0o700)
            .create(&current)
            .await;
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(format!(
                    "failed to create exec spill directory {}: {error}",
                    current.display()
                ));
            }
        }
    }
    validate_private_dir(path).await
}

#[cfg(not(unix))]
async fn create_private_dir_all(path: &Path) -> Result<(), String> {
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|error| format!("failed to create exec spill directory: {error}"))
}

#[cfg(unix)]
async fn ensure_safe_spill_file_path(path: &Path) -> Result<(), String> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing to open symlink exec spill log: {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "refusing to open non-file exec spill log: {}",
            path.display()
        )),
        Ok(metadata) if metadata.uid() != current_uid() => Err(format!(
            "refusing to open exec spill log owned by another user: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to inspect exec spill log {}: {error}",
            path.display()
        )),
    }
}

#[cfg(not(unix))]
async fn ensure_safe_spill_file_path(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn default_spill_root() -> Result<PathBuf, String> {
    Ok(spill_root_from_parts(
        std::env::var_os("XDG_CACHE_HOME"),
        home::home_dir(),
        std::env::temp_dir(),
    ))
}

fn spill_root_from_parts(
    cache_home: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
    temp_dir: PathBuf,
) -> PathBuf {
    if let Some(cache_home) = cache_home.filter(|value| !value.is_empty()) {
        let path = PathBuf::from(cache_home);
        if path.is_absolute() {
            return path.join("refact").join("exec");
        }
    }
    if let Some(home) = home {
        return home.join(".cache").join("refact").join("exec");
    }
    fallback_temp_spill_root(temp_dir)
}

#[cfg(unix)]
fn fallback_temp_spill_root(temp_dir: PathBuf) -> PathBuf {
    temp_dir.join(format!("refact-{}-exec", current_uid()))
}

#[cfg(not(unix))]
fn fallback_temp_spill_root(temp_dir: PathBuf) -> PathBuf {
    temp_dir.join("refact").join("exec")
}

fn safe_spill_component(prefix: &str, raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    format!("{prefix}_{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use std::path::Component;

    use super::*;

    #[tokio::test]
    async fn spill_target_writes_with_hashed_path_components() {
        let temp = tempfile::tempdir().unwrap();
        let target = SpillTarget::with_root(
            temp.path().to_path_buf(),
            "../escape/chat",
            &ExecProcessId("exec_../../evil".to_string()),
        );
        let mut writer = SpillWriter::create(&target).await.unwrap();
        writer.write_line("safe\n").await.unwrap();
        drop(writer);

        let relative = target
            .path()
            .strip_prefix(temp.path())
            .expect("path should stay under root")
            .to_path_buf();
        assert!(relative
            .components()
            .all(|component| matches!(component, Component::Normal(_))));
        assert!(!relative.to_string_lossy().contains(".."));
        assert!(!relative.to_string_lossy().contains("escape"));
        assert!(!relative.to_string_lossy().contains("evil"));
        assert_eq!(
            tokio::fs::read_to_string(target.path()).await.unwrap(),
            "safe\n"
        );
    }

    #[test]
    fn spill_root_falls_back_to_temp_without_cache_or_home() {
        let temp = PathBuf::from("/tmp/refact-test-root");

        let root = spill_root_from_parts(None, None, temp.clone());

        assert_eq!(root, fallback_temp_spill_root(temp));
    }

    #[test]
    fn spill_root_prefers_xdg_cache_home() {
        let root = spill_root_from_parts(
            Some(std::ffi::OsString::from("/cache-root")),
            Some(PathBuf::from("/home/root")),
            PathBuf::from("/tmp/refact-test-root"),
        );

        assert_eq!(
            root,
            PathBuf::from("/cache-root").join("refact").join("exec")
        );
    }

    #[test]
    fn spill_root_ignores_relative_xdg_cache_home() {
        let root = spill_root_from_parts(
            Some(std::ffi::OsString::from("relative-cache")),
            Some(PathBuf::from("/home/root")),
            PathBuf::from("/tmp/refact-test-root"),
        );

        assert_eq!(
            root,
            PathBuf::from("/home/root")
                .join(".cache")
                .join("refact")
                .join("exec")
        );
    }

    #[tokio::test]
    async fn spill_writer_truncates_reused_service_log() {
        let temp = tempfile::tempdir().unwrap();
        let process_id = ExecProcessId("exec_service_api_deadbeef".to_string());
        let target = SpillTarget::with_root(temp.path().to_path_buf(), "chat-a", &process_id);

        let mut first = SpillWriter::create(&target).await.unwrap();
        first.write_line("old\n").await.unwrap();
        drop(first);

        let mut second = SpillWriter::create(&target).await.unwrap();
        second.write_line("new\n").await.unwrap();
        drop(second);

        assert_eq!(
            tokio::fs::read_to_string(target.path()).await.unwrap(),
            "new\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spill_writer_rejects_symlink_log_file() {
        let temp = tempfile::tempdir().unwrap();
        let process_id = ExecProcessId("exec_symlink_log".to_string());
        let target = SpillTarget::with_root(temp.path().to_path_buf(), "chat-a", &process_id);
        create_private_dir_all(&target.dir()).await.unwrap();
        let outside = temp.path().join("outside.log");
        std::fs::write(&outside, "outside").unwrap();
        std::os::unix::fs::symlink(&outside, target.path()).unwrap();

        let error = match SpillWriter::create(&target).await {
            Ok(_) => panic!("symlink spill log should be rejected"),
            Err(error) => error,
        };

        assert!(
            error.contains("symlink exec spill log")
                || error.contains("failed to open exec spill log"),
            "unexpected error: {error}"
        );
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "outside");
    }
}
