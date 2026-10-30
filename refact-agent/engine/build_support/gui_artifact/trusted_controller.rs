use super::*;

pub(super) fn load() -> Result<Controller> {
    #[cfg(target_os = "linux")]
    {
        host_boundary()?;
        let mut directory = protected_root()?;
        for name in ["etc", "refact", "worktree-cache"] {
            directory = child(&directory, name, true)?;
        }
        let file = child(&directory, "gui-controller.json", false)?;
        let mut bytes = Vec::new();
        file.take(MAX_JSON + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_JSON {
            return refuse("GUI controller exceeds size bound");
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        refuse("Trusted GUI prebuilt ingress is Linux-only; use an explicit normal local build or API-only skip")
    }
}

#[cfg(target_os = "linux")]
fn host_boundary() -> Result<()> {
    if unsafe { libc::geteuid() } == 0 || unsafe { libc::getuid() } != unsafe { libc::geteuid() } {
        return refuse("GUI provenance requires a non-root unchanged host UID");
    }
    let mapping = fs::read_to_string("/proc/self/uid_map")?;
    if mapping.split_whitespace().collect::<Vec<_>>() != ["0", "0", "4294967295"] {
        return refuse("GUI provenance refuses non-initial UID mappings");
    }
    let status = fs::read_to_string("/proc/self/status")?;
    for field in ["CapEff:", "CapPrm:"] {
        let value = status
            .lines()
            .find_map(|line| line.strip_prefix(field))
            .ok_or("Missing capability state")?;
        if u64::from_str_radix(value.trim(), 16)? != 0 {
            return refuse("GUI provenance refuses capable consumers");
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn protected_root() -> Result<File> {
    use std::os::fd::FromRawFd;
    let fd = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    protection(&file)?;
    refuse_writable(&file, c".")?;
    Ok(file)
}

#[cfg(target_os = "linux")]
fn child(parent: &File, name: &str, directory: bool) -> Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = std::ffi::CString::new(name)?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | if directory { libc::O_DIRECTORY } else { 0 };
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return refuse(
            "Missing or unsafe admin GUI controller; prebuilt reuse refused without fallback",
        );
    }
    let file = unsafe { File::from_raw_fd(fd) };
    protection(&file)?;
    let metadata = file.metadata()?;
    if directory != metadata.is_dir() || (!directory && !metadata.is_file()) {
        return refuse("Unsafe GUI controller entry type");
    }
    refuse_writable(parent, &name)?;
    Ok(file)
}

#[cfg(target_os = "linux")]
fn protection(file: &File) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    if metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        || (metadata.is_file() && metadata.nlink() != 1)
    {
        return refuse("GUI controller and all ancestors must be root-owned and not group/world writable or aliased");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn refuse_writable(parent: &File, name: &std::ffi::CStr) -> Result<()> {
    use std::os::fd::AsRawFd;
    if unsafe {
        libc::faccessat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::W_OK,
            libc::AT_EACCESS | libc::AT_SYMLINK_NOFOLLOW,
        )
    } == 0
    {
        return refuse("GUI controller or ancestor is effectively writable by consumer");
    }
    match io::Error::last_os_error().raw_os_error() {
        Some(libc::EACCES | libc::EROFS) => Ok(()),
        _ => refuse("Cannot establish GUI controller effective write protection"),
    }
}
