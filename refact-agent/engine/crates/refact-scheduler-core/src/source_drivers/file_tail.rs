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
pub(crate) fn read(
    root: &cap_std::fs::Dir,
    grant: &FileGrant,
    cursor: Option<&str>,
) -> Result<Batch, String> {
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
        let mut bytes = Vec::new();
        file.take(262_145)
            .read_to_end(&mut bytes)
            .map_err(|_| "file read failed")?;
        let mut lines = Vec::new();
        let mut consumed = 0;
        for line in bytes.split_inclusive(|b| *b == b'\n') {
            if line.last() != Some(&b'\n') {
                break;
            }
            if lines.len() >= 128 {
                break;
            }
            if line.len() > 262_144 {
                return Err("file line limit".into());
            }
            consumed += line.len();
            position.offset += line.len() as u64;
            let text = std::str::from_utf8(line)
                .map_err(|_| "file line is not UTF-8")?
                .trim_end_matches(['\r', '\n'])
                .to_owned();
            lines.push((
                text,
                serde_json::to_string(&position).map_err(|_| "checkpoint serialization failed")?,
            ));
        }
        if lines.is_empty() && bytes.len() > 262_144 && consumed == 0 {
            return Err("file partial line limit".into());
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
    #[test]
    fn hardlinks_are_not_file_capabilities() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("log"), b"secret\n").unwrap();
        std::fs::hard_link(temp.path().join("log"), temp.path().join("alias")).unwrap();
        let root =
            cap_std::fs::Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        let grant = FileGrant {
            path: "alias".into(),
            rotation: RotationPolicy::Follow,
        };
        assert!(read(&root, &grant, None).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn windows_devices_streams_and_foreign_checkpoints_are_denied() {
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
        assert!(read(&root, &grant, Some(r#"{"dev":1,"ino":1,"offset":0}"#)).is_err());
        assert!(read(
            &root,
            &grant,
            Some(r#"{"identity_kind":"unix","dev":1,"ino":1,"offset":0}"#)
        )
        .is_err());
        let batch = read(&root, &grant, None).unwrap();
        assert!(batch.lines[0].1.contains("windows_volume_file_index_v1"));
    }
    #[test]
    fn partial_restart_rotation_truncation_and_escape() {
        let temp = tempfile::tempdir().unwrap();
        let root =
            cap_std::fs::Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        let grant = FileGrant {
            path: "log".into(),
            rotation: RotationPolicy::Follow,
        };
        std::fs::write(temp.path().join("log"), b"one\npart").unwrap();
        let first = read(&root, &grant, None).unwrap();
        assert_eq!(first.lines.len(), 1);
        let cursor = &first.lines[0].1;
        assert!(read(&root, &grant, Some(cursor)).unwrap().lines.is_empty());
        std::fs::write(temp.path().join("log"), b"one\npartial\n").unwrap();
        assert_eq!(
            read(&root, &grant, Some(cursor)).unwrap().lines[0].0,
            "partial"
        );
        std::fs::rename(temp.path().join("log"), temp.path().join("old")).unwrap();
        std::fs::write(temp.path().join("log"), b"new\n").unwrap();
        let rotated = read(&root, &grant, Some(cursor)).unwrap();
        assert!(rotated.gap);
        std::fs::write(temp.path().join("log"), b"x\n").unwrap();
        assert!(read(&root, &grant, Some(&rotated.lines[0].1)).unwrap().gap);
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
            .is_err());
        }
    }
}
