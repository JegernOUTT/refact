use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesystemKind {
    Local,
    NonLocal {
        fs_type: String,
        mount_point: String,
    },
    Unknown,
}

pub const NON_LOCAL_FS_TYPES: &[&str] = &[
    "9p",
    "afs",
    "beegfs",
    "ceph",
    "cifs",
    "davfs",
    "fuse.davfs",
    "fuse.rclone",
    "fuse.s3fs",
    "fuse.sshfs",
    "gfs2",
    "glusterfs",
    "lustre",
    "nfs",
    "nfs3",
    "nfs4",
    "ocfs2",
    "smb2",
    "smb3",
    "smbfs",
    "vboxsf",
    "virtiofs",
];

pub fn is_non_local_fs_type(fs_type: &str) -> bool {
    NON_LOCAL_FS_TYPES.contains(&fs_type)
}

fn unescape_mount_field(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let digits: String = chars.clone().take(3).collect();
        let octal = digits.len() == 3 && digits.chars().all(|d| ('0'..='7').contains(&d));
        match octal.then(|| u8::from_str_radix(&digits, 8).ok()).flatten() {
            Some(value) => {
                out.push(value as char);
                for _ in 0..3 {
                    chars.next();
                }
            }
            None => out.push(c),
        }
    }
    out
}

fn path_has_prefix(path: &Path, prefix: &Path) -> bool {
    let mut actual = path.components();
    for expected in prefix.components() {
        match actual.next() {
            Some(component) if component == expected => {}
            _ => return false,
        }
    }
    true
}

pub fn classify_mounts(mounts: &str, path: &Path) -> FilesystemKind {
    let mut best: Option<(usize, String, String)> = None;
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        if fields.next().is_none() {
            continue;
        }
        let mount_point = match fields.next() {
            Some(value) => unescape_mount_field(value),
            None => continue,
        };
        let fs_type = match fields.next() {
            Some(value) => unescape_mount_field(value),
            None => continue,
        };
        if !path_has_prefix(path, Path::new(&mount_point)) {
            continue;
        }
        let depth = Path::new(&mount_point).components().count();
        let better = best.as_ref().map(|(best, _, _)| depth >= *best);
        if better.unwrap_or(true) {
            best = Some((depth, fs_type, mount_point));
        }
    }
    match best {
        Some((_, fs_type, mount_point)) if is_non_local_fs_type(&fs_type) => {
            FilesystemKind::NonLocal {
                fs_type,
                mount_point,
            }
        }
        Some(_) => FilesystemKind::Local,
        None => FilesystemKind::Unknown,
    }
}

#[cfg(target_os = "linux")]
pub fn classify_path(path: &Path) -> FilesystemKind {
    let probe = nearest_existing(path);
    let probe = std::fs::canonicalize(&probe).unwrap_or(probe);
    match std::fs::read_to_string("/proc/self/mounts") {
        Ok(mounts) => classify_mounts(&mounts, &probe),
        Err(_) => FilesystemKind::Unknown,
    }
}

#[cfg(target_os = "linux")]
fn nearest_existing(path: &Path) -> std::path::PathBuf {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return current;
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => return current,
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn classify_path(_path: &Path) -> FilesystemKind {
    FilesystemKind::Unknown
}
