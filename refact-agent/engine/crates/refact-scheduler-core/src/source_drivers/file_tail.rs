use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationPolicy {
    Stop,
    Follow,
}

/// Host-approved capability metadata, bound by the journal to source digest.
/// Root is the service's canonical project capability, never app supplied.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileGrant {
    pub path: String,
    pub rotation: RotationPolicy,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cursor {
    #[cfg(windows)]
    identity_kind: String,
    dev: u64,
    ino: u64,
    offset: u64,
}
pub(crate) struct Batch {
    pub lines: Vec<(String, String)>,
    pub gap: bool,
}
pub(crate) fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 4096
        || path.contains('\\')
        || (cfg!(windows) && path.contains(':'))
        || !Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("file tail requires a strict project-relative path".into());
    }
    #[cfg(windows)]
    for component in path.split('/') {
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if component.ends_with(['.', ' '])
            || matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            )
            || stem
                .strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT"))
                .is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
        {
            return Err("file tail Windows device or ambiguous path denied".into());
        }
    }
    Ok(())
}
/// Reopen through a confined directory on every read (symlink escapes denied by
/// cap-std). Only complete newline-terminated records advance the cursor. The
/// partial line stays in the file, not an unbounded in-memory restart buffer.
/// Best effort only: same-inode copytruncate followed by regrowth beyond the
/// saved offset between reads cannot be detected; continuity is not guaranteed.
#[cfg(test)]
pub(crate) async fn read(
    root: &cap_std::fs::Dir,
    grant: &FileGrant,
    cursor: Option<&str>,
) -> Result<Batch, String> {
    read_cooperative(root, grant, cursor, tokio::task::yield_now).await
}
pub(crate) async fn read_cooperative<F, Fut>(
    root: &cap_std::fs::Dir,
    grant: &FileGrant,
    cursor: Option<&str>,
    mut checkpoint: F,
) -> Result<Batch, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    validate_path(&grant.path)?;
    use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
    let path = Path::new(&grant.path);
    let mut directory = root.try_clone().map_err(|_| "file root unavailable")?;
    if let Some(parent) = path.parent() {
        for part in parent.components() {
            directory = directory
                .open_dir_nofollow(part.as_os_str())
                .map_err(|_| "file parent capability denied")?;
            #[cfg(windows)]
            {
                use cap_std::fs::MetadataExt;
                if directory
                    .dir_metadata()
                    .map_err(|_| "file parent metadata unavailable")?
                    .file_attributes()
                    & 0x400
                    != 0
                {
                    return Err("file reparse parent capability denied".into());
                }
            }
        }
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = directory
        .open_with(path.file_name().ok_or("file name missing")?, &options)
        .map_err(|_| "file capability open denied")?;
    let metadata = file.metadata().map_err(|_| "file metadata unavailable")?;
    if !metadata.is_file() {
        return Err("tail target is not a regular file".into());
    }
    #[cfg(unix)]
    let (dev, ino) = {
        use cap_std::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("file hardlink capability denied".into());
        }
        (metadata.dev(), metadata.ino())
    };
    #[cfg(windows)]
    let (dev, ino) = {
        use cap_primitives::fs::_WindowsByHandle;
        if metadata.number_of_links() != Some(1) || metadata.file_attributes() & 0x400 != 0 {
            return Err("file reparse or hardlink capability denied".into());
        }
        (
            u64::from(
                metadata
                    .volume_serial_number()
                    .ok_or("file volume identity unavailable")?,
            ),
            metadata
                .file_index()
                .ok_or("file index identity unavailable")?,
        )
    };
    #[cfg(not(any(unix, windows)))]
    return Err("file identity checkpoint unsupported on this platform".into());
    #[cfg(any(unix, windows))]
    {
        let mut position = match cursor {
            Some(raw) => {
                serde_json::from_str::<Cursor>(raw).map_err(|_| "invalid file checkpoint")?
            }
            None => Cursor {
                #[cfg(windows)]
                identity_kind: "windows_volume_file_index_v1".into(),
                dev,
                ino,
                offset: 0,
            },
        };
        // Unix dev/inode checkpoints must never be interpreted as Windows identities.
        #[cfg(windows)]
        if position.identity_kind != "windows_volume_file_index_v1" {
            return Err("incompatible file checkpoint identity".into());
        }
        let gap = position.dev != dev || position.ino != ino || position.offset > metadata.len();
        if gap {
            if matches!(grant.rotation, RotationPolicy::Stop) {
                return Err("file rotation forbidden".into());
            }
            position = Cursor {
                #[cfg(windows)]
                identity_kind: "windows_volume_file_index_v1".into(),
                dev,
                ino,
                offset: 0,
            };
        }
        file.seek(SeekFrom::Start(position.offset))
            .map_err(|_| "file seek failed")?;
        let remaining = metadata.len().saturating_sub(position.offset);
        let mut reader = file.take(remaining);
        let mut lines = Vec::new();
        let mut line = Vec::new();
        let mut chunk = [0u8; 16384];
        'read: loop {
            checkpoint().await;
            let count = reader.read(&mut chunk).map_err(|_| "file read failed")?;
            if count == 0 {
                break;
            }
            for byte in &chunk[..count] {
                line.push(*byte);
                if *byte != b'\n' {
                    continue;
                }
                position.offset += line.len() as u64;
                let text = std::str::from_utf8(&line)
                    .map_err(|_| "file line is not UTF-8")?
                    .trim_end_matches(['\r', '\n'])
                    .to_owned();
                lines.push((
                    text,
                    serde_json::to_string(&position)
                        .map_err(|_| "checkpoint serialization failed")?,
                ));
                line.clear();
                if lines.len() >= 128 {
                    break 'read;
                }
            }
        }
        Ok(Batch {
            lines,
            gap,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn read_uses_finite_length_snapshot_when_writer_appends() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("log");
        std::fs::write(&path, b"first\n").unwrap();
        let root =
            cap_std::fs::Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        let grant = FileGrant {
            path: "log".into(),
            rotation: RotationPolicy::Follow,
        };
        let mut appended = false;
        let batch = read_cooperative(&root, &grant, None, || {
            if !appended {
                use std::io::Write;
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap()
                    .write_all(b"second\n")
                    .unwrap();
                appended = true;
            }
            std::future::ready(())
        })
        .await
        .unwrap();
        assert_eq!(batch.lines.len(), 1);
        assert_eq!(batch.lines[0].0, "first");
        let next = read(&root, &grant, Some(&batch.lines[0].1)).await.unwrap();
        assert_eq!(next.lines.len(), 1);
        assert_eq!(next.lines[0].0, "second");
    }
    #[tokio::test]
    async fn large_lines_partial_restart_and_record_backpressure() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("log");
        let text = "history".repeat(50000);
        std::fs::write(&path, &text).unwrap();
        let root =
            cap_std::fs::Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        let grant = FileGrant {
            path: "log".into(),
            rotation: RotationPolicy::Follow,
        };
        assert!(read(&root, &grant, None).await.unwrap().lines.is_empty());
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"\n").unwrap();
        for _ in 0..129 {
            file.write_all(b"next\n").unwrap();
        }
        let batch = read(&root, &grant, None).await.unwrap();
        assert_eq!(batch.lines.len(), 128);
        assert_eq!(batch.lines[0].0, text);
        let cursor = batch.lines.last().unwrap().1.clone();
        let remaining = read(&root, &grant, Some(&cursor)).await.unwrap();
        assert_eq!(
            remaining
                .lines
                .iter()
                .map(|line| line.0.as_str())
                .collect::<Vec<_>>(),
            vec!["next", "next"]
        );
        assert!(
            read(&root, &grant, Some(&remaining.lines.last().unwrap().1))
                .await
                .unwrap()
                .lines
                .is_empty()
        );
    }
    #[tokio::test]
    async fn hardlinks_are_not_file_capabilities() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("log"), b"secret\n").unwrap();
        std::fs::hard_link(temp.path().join("log"), temp.path().join("alias")).unwrap();
        let root =
            cap_std::fs::Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        let grant = FileGrant {
            path: "alias".into(),
            rotation: RotationPolicy::Follow,
        };
        assert!(read(&root, &grant, None).await.is_err());
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_devices_streams_and_foreign_checkpoints_are_denied() {
        for path in [
            "log:secret",
            "CON",
            "dir/NUL.txt",
            "COM1",
            "lpt².txt",
            "log.",
            "log ",
        ] {
            assert!(validate_path(path).is_err(), "{path}");
        }
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("log"), b"line\n").unwrap();
        let root =
            cap_std::fs::Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        let grant = FileGrant {
            path: "log".into(),
            rotation: RotationPolicy::Follow,
        };
        assert!(read(&root, &grant, Some(r#"{"dev":1,"ino":1,"offset":0}"#))
            .await
            .is_err());
        assert!(read(
            &root,
            &grant,
            Some(r#"{"identity_kind":"unix","dev":1,"ino":1,"offset":0}"#)
        )
        .await
        .is_err());
        let batch = read(&root, &grant, None).await.unwrap();
        assert!(batch.lines[0].1.contains("windows_volume_file_index_v1"));
    }
    #[tokio::test]
    async fn partial_restart_rotation_truncation_and_escape() {
        let temp = tempfile::tempdir().unwrap();
        let root =
            cap_std::fs::Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        let grant = FileGrant {
            path: "log".into(),
            rotation: RotationPolicy::Follow,
        };
        std::fs::write(temp.path().join("log"), b"one\npart").unwrap();
        let first = read(&root, &grant, None).await.unwrap();
        assert_eq!(first.lines.len(), 1);
        let cursor = &first.lines[0].1;
        assert!(read(&root, &grant, Some(cursor))
            .await
            .unwrap()
            .lines
            .is_empty());
        std::fs::write(temp.path().join("log"), b"one\npartial\n").unwrap();
        assert_eq!(
            read(&root, &grant, Some(cursor)).await.unwrap().lines[0].0,
            "partial"
        );
        std::fs::rename(temp.path().join("log"), temp.path().join("old")).unwrap();
        std::fs::write(temp.path().join("log"), b"new\n").unwrap();
        let rotated = read(&root, &grant, Some(cursor)).await.unwrap();
        assert!(rotated.gap);
        std::fs::write(temp.path().join("log"), b"x\n").unwrap();
        assert!(
            read(&root, &grant, Some(&rotated.lines[0].1))
                .await
                .unwrap()
                .gap
        );
        assert!(validate_path("../secret").is_err());
        assert!(validate_path("/absolute").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", temp.path().join("escape")).unwrap();
            assert!(read(
                &root,
                &FileGrant {
                    path: "escape".into(),
                    rotation: RotationPolicy::Follow
                },
                None
            )
            .await
            .is_err());
        }
    }
}
