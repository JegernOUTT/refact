//! Windows owned-profile capabilities. No path-based deletion, including on failure.
//! Sharing denies DELETE on every ancestor for the entire capability lifetime.
//! ACL/token setup deliberately mirrors artifact_store's Windows implementation.
#![cfg(windows)]

use std::ffi::{c_void, OsStr};
use std::fs::{File, OpenOptions};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{LocalFree, HANDLE};
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

fn io(e: std::io::Error) -> String {
    format!("named profile filesystem: {e}")
}
fn last() -> String {
    io(std::io::Error::last_os_error())
}
fn wide(s: &OsStr) -> Result<Vec<u16>, String> {
    let mut w: Vec<_> = s.encode_wide().collect();
    if w.contains(&0) {
        return Err("NUL in profile path".into());
    }
    w.push(0);
    Ok(w)
}
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn current_sid() -> Result<String, String> {
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last());
    }
    let token = unsafe { File::from_raw_handle(token.cast()) };
    let mut needed = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle().cast(),
            TokenUser,
            std::ptr::null_mut(),
            0,
            &mut needed,
        );
    }
    let mut buf = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle().cast(),
            TokenUser,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(last());
    }
    let user = unsafe { &*buf.as_ptr().cast::<TOKEN_USER>() };
    let mut sid = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
        return Err(last());
    }
    let _allocation = Local(sid.cast());
    let mut len = 0;
    while unsafe { *sid.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16(unsafe { std::slice::from_raw_parts(sid, len) })
        .map_err(|_| "invalid SID".into())
}
fn descriptor(sid: &str) -> Result<Local, String> {
    let sddl = wide(OsStr::new(&format!(
        "O:{sid}D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)"
    )))?;
    let mut sd = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut sd,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(last());
    }
    Ok(Local(sd))
}
fn private_directory(path: &Path, sd: &Local) -> Result<(), String> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    };
    if unsafe { CreateDirectoryW(wide(path.as_os_str())?.as_ptr(), &sa) } == 0 {
        return Err(last());
    }
    Ok(())
}
// Verify the *opened object*, never GetNamedSecurityInfo on a potentially replaced path.
fn private_acl(file: &File, expected: &Local) -> Result<(), String> {
    let mut owner = std::ptr::null_mut();
    let mut acl = std::ptr::null_mut();
    let mut sd = std::ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle().cast(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if status != 0 {
        return Err(io(std::io::Error::from_raw_os_error(status as i32)));
    }
    let _sd = Local(sd);
    let mut control = 0;
    let mut revision = 0;
    let mut expected_owner = std::ptr::null_mut();
    let mut defaulted = 0;
    let mut present = 0;
    let mut expected_acl = std::ptr::null_mut();
    unsafe {
        if GetSecurityDescriptorControl(sd, &mut control, &mut revision) == 0
            || GetSecurityDescriptorOwner(expected.0, &mut expected_owner, &mut defaulted) == 0
            || GetSecurityDescriptorDacl(
                expected.0,
                &mut present,
                &mut expected_acl,
                &mut defaulted,
            ) == 0
            || control & SE_DACL_PROTECTED == 0
            || acl.is_null()
            || owner.is_null()
            || EqualSid(owner, expected_owner) == 0
        {
            return Err("profile ACL/owner is not private".into());
        }
        // The only accepted policy is exactly our protected two-ACE policy.
        if (*acl).AceCount != (*expected_acl).AceCount {
            return Err("unexpected profile ACL entries".into());
        }
        for i in 0..u32::from((*acl).AceCount) {
            let mut actual: *mut c_void = std::ptr::null_mut();
            let mut wanted: *mut c_void = std::ptr::null_mut();
            if GetAce(acl, i, &mut actual) == 0 || GetAce(expected_acl, i, &mut wanted) == 0 {
                return Err(last());
            }
            let a = &*actual.cast::<ACE_HEADER>();
            let b = &*wanted.cast::<ACE_HEADER>();
            if a.AceSize != b.AceSize
                || std::slice::from_raw_parts(actual.cast::<u8>(), a.AceSize as usize)
                    != std::slice::from_raw_parts(wanted.cast::<u8>(), b.AceSize as usize)
            {
                return Err("unexpected profile ACL policy".into());
            }
        }
    }
    Ok(())
}
fn info(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION, String> {
    let mut value = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut value) } == 0 {
        return Err(last());
    }
    Ok(value)
}
fn open(path: &Path, delete: bool) -> Result<File, String> {
    OpenOptions::new()
        .access_mode(
            FILE_READ_ATTRIBUTES
                | READ_CONTROL
                | if delete {
                    DELETE | FILE_LIST_DIRECTORY
                } else {
                    0
                },
        )
        .share_mode(if delete {
            FILE_SHARE_READ | FILE_SHARE_WRITE
        } else {
            FILE_SHARE_READ
        })
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(io)
}
fn directory(file: &File) -> Result<(), String> {
    let attr = info(file)?.dwFileAttributes;
    if attr & FILE_ATTRIBUTE_REPARSE_POINT != 0 || attr & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err("profile ancestor is not a plain directory".into());
    }
    Ok(())
}
fn pin(cache: &Path) -> Result<(PathBuf, Vec<File>), String> {
    let absolute = std::path::absolute(cache).map_err(io)?;
    let mut path = PathBuf::new();
    let mut pins = Vec::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(p)
                if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
            {
                path.push(component)
            }
            Component::RootDir | Component::Normal(_) => {
                path.push(component);
                let file = open(&path, false)?;
                directory(&file)?;
                pins.push(file);
            }
            _ => return Err("profile cache requires local disk path without traversal".into()),
        }
    }
    if pins.is_empty() {
        return Err("invalid profile cache".into());
    }
    Ok((path, pins))
}
fn disposition(file: &File) -> Result<(), String> {
    let value = FILE_DISPOSITION_INFO {
        DeleteFile: true,
    };
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle().cast(),
            FileDispositionInfo,
            (&value as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    } == 0
    {
        return Err(last());
    }
    Ok(())
}

pub struct WindowsOwnedProfile {
    path: PathBuf,
    root: Mutex<Option<File>>,
    // Pins outlive root deletion, and are deliberately retained after failed cleanup.
    _pins: Vec<File>,
}
impl WindowsOwnedProfile {
    pub fn create(cache: &Path) -> Result<Self, String> {
        let (cache, mut pins) = pin(cache)?;
        let sd = descriptor(&current_sid()?)?;
        let parent = cache.join("named_browser_profiles");
        // A failed create is acceptable only if the opened existing directory passes
        // the exact protected owner/DACL policy below (including AlreadyExists races).
        let _ = private_directory(&parent, &sd);
        let parent_handle = open(&parent, false)?;
        directory(&parent_handle)?;
        private_acl(&parent_handle, &sd)?;
        // Kernel sharing is the inter-process lock: no waiting, stale PID, or unlink race.
        let _allocation = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(parent.join(".allocation.lock"))
            .map_err(io)?;
        let lock_info = info(&_allocation)?;
        if lock_info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY)
            != 0
            || lock_info.nNumberOfLinks != 1
        {
            return Err("invalid allocation lock".into());
        }
        let mut count = 0;
        for entry in std::fs::read_dir(&parent).map_err(io)? {
            let entry = entry.map_err(io)?;
            if entry.file_name() == ".allocation.lock" {
                continue;
            }
            count += 1;
            if count >= 32 {
                return Err("named profile disk quota exceeded".into());
            }
        }
        let name = format!("profile-{}", uuid::Uuid::new_v4());
        let path = parent.join(&name);
        let root = create_root(&parent_handle, &name, &sd)?;
        directory(&root)?;
        private_acl(&root, &sd)?;
        pins.push(parent_handle);
        Ok(Self {
            path,
            root: Mutex::new(Some(root)),
            _pins: pins,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// The caller must supply the verified OwnedProcess exit receipt, not an observed PID.
    /// Any SingletonLock representation on Windows is conservatively retained.
    pub fn cleanup(&self, exited: bool, lock_safe: bool) -> Result<(), String> {
        if !exited || !lock_safe {
            return Err("process exit or profile lock not verified".into());
        }
        let mut root = self.root.lock().map_err(|_| "profile guard poisoned")?;
        let Some(file) = root.as_ref() else {
            return Ok(());
        };
        // Freeze reparse/attribute mutation during cleanup. The existing root DELETE
        // handle requires this second handle to SHARE_DELETE; the original still denies
        // third-party DELETE opens. No handle is dropped during the transition.
        let shield = unsafe {
            ReOpenFile(
                file.as_raw_handle().cast(),
                FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            )
        };
        if shield == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return Err(last());
        }
        let shield = unsafe { File::from_raw_handle(shield.cast()) };
        directory(file)?;
        let sd = descriptor(&current_sid()?)?;
        private_acl(file, &sd)?;
        if !singleton_absent(file)? {
            return Err("profile SingletonLock is present or unreadable".into());
        }
        let mut budget = Budget {
            nodes: 100_000,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        remove_contents(&self.path, file, &mut budget, 0)?;
        disposition(file)?;
        drop(root.take()); // Commit delete-on-close while every ancestor remains pinned.
        drop(shield);
        Ok(())
    }
}
struct Budget {
    nodes: usize,
    deadline: Instant,
}
impl Budget {
    fn consume(&mut self) -> Result<(), String> {
        if self.nodes == 0 || Instant::now() >= self.deadline {
            return Err("profile cleanup budget exhausted; guard retained".into());
        }
        self.nodes -= 1;
        Ok(())
    }
}
// Atomic creation returns the capability for the object actually created. There
// is no CreateDirectory/open gap in which a replacement can acquire ownership.
fn create_root(parent: &File, name: &str, sd: &Local) -> Result<File, String> {
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        NtCreateFile, FILE_CREATE, FILE_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT,
        FILE_SYNCHRONOUS_IO_NONALERT,
    };
    use windows_sys::Win32::Foundation::UNICODE_STRING;
    let mut name: Vec<u16> = name.encode_utf16().collect();
    let mut unicode = UNICODE_STRING {
        Length: (name.len() * 2) as u16,
        MaximumLength: (name.len() * 2) as u16,
        Buffer: name.as_mut_ptr(),
    };
    let object = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle().cast(),
        ObjectName: &mut unicode,
        Attributes: 0x40,
        SecurityDescriptor: sd.0.cast(),
        SecurityQualityOfService: std::ptr::null_mut(),
    };
    let mut ios = unsafe { std::mem::zeroed() };
    let mut handle = std::ptr::null_mut();
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            DELETE | READ_CONTROL | FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | SYNCHRONIZE,
            &object,
            &mut ios,
            std::ptr::null(),
            FILE_ATTRIBUTE_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_CREATE,
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 {
        return Err(nt_error(status));
    }
    Ok(unsafe { File::from_raw_handle(handle.cast()) })
}
fn relative(dir: &File, name: &[u16], delete: bool) -> Result<File, i32> {
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        NtCreateFile, FILE_OPEN, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
    };
    use windows_sys::Win32::Foundation::UNICODE_STRING;
    if name.is_empty() || name.len() > 32766 || name.iter().any(|c| matches!(*c, 0 | 47 | 58 | 92))
    {
        return Err(-1);
    }
    let mut name = name.to_vec();
    let mut unicode = UNICODE_STRING {
        Length: (name.len() * 2) as u16,
        MaximumLength: (name.len() * 2) as u16,
        Buffer: name.as_mut_ptr(),
    };
    let object = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: dir.as_raw_handle().cast(),
        ObjectName: &mut unicode,
        Attributes: 0x40,
        SecurityDescriptor: std::ptr::null_mut(),
        SecurityQualityOfService: std::ptr::null_mut(),
    };
    let mut status = unsafe { std::mem::zeroed() };
    let mut handle = std::ptr::null_mut();
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_READ_ATTRIBUTES
                | FILE_LIST_DIRECTORY
                | SYNCHRONIZE
                | if delete { DELETE } else { 0 },
            &object,
            &mut status,
            std::ptr::null(),
            0,
            FILE_SHARE_READ,
            FILE_OPEN,
            FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null(),
            0,
        )
    };
    if result < 0 {
        return Err(result);
    }
    Ok(unsafe { File::from_raw_handle(handle.cast()) })
}
fn nt_error(status: i32) -> String {
    io(std::io::Error::from_raw_os_error(unsafe {
        windows_sys::Win32::Foundation::RtlNtStatusToDosError(status)
    } as i32))
}
fn singleton_absent(dir: &File) -> Result<bool, String> {
    match relative(
        dir,
        &"SingletonLock".encode_utf16().collect::<Vec<_>>(),
        false,
    ) {
        Ok(_) => Ok(false),
        Err(status) if status == 0xC0000034u32 as i32 => Ok(true), // STATUS_OBJECT_NAME_NOT_FOUND only
        Err(status) => Err(nt_error(status)),
    }
}
// Bounded native handle enumeration. Names are never resolved through a path.
fn enumerate(
    dir: &File,
    mut visit: impl FnMut(&[u16]) -> Result<(), String>,
) -> Result<(), String> {
    let mut buffer = vec![0u64; 8192]; // aligned, fixed 64 KiB
    let bytes = buffer.len() * 8;
    let name_offset = std::mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    let mut class = FileIdBothDirectoryRestartInfo;
    loop {
        if unsafe {
            GetFileInformationByHandleEx(
                dir.as_raw_handle().cast(),
                class,
                buffer.as_mut_ptr().cast(),
                bytes as u32,
            )
        } == 0
        {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(18) {
                return Ok(());
            } // ERROR_NO_MORE_FILES
            return Err(io(error));
        }
        class = FileIdBothDirectoryInfo;
        let mut offset = 0usize;
        loop {
            if offset + std::mem::size_of::<FILE_ID_BOTH_DIR_INFO>() > bytes {
                return Err("invalid directory record".into());
            }
            let record = unsafe {
                &*(buffer
                    .as_ptr()
                    .cast::<u8>()
                    .add(offset)
                    .cast::<FILE_ID_BOTH_DIR_INFO>())
            };
            let len = record.FileNameLength as usize;
            if len % 2 != 0 || offset + name_offset + len > bytes {
                return Err("invalid directory name".into());
            }
            let name = unsafe { std::slice::from_raw_parts(record.FileName.as_ptr(), len / 2) };
            if name != [46] && name != [46, 46] {
                visit(name)?;
            }
            if record.NextEntryOffset == 0 {
                break;
            }
            let step = record.NextEntryOffset as usize;
            if step < name_offset + len || step % 8 != 0 {
                return Err("invalid directory offset".into());
            }
            offset = offset
                .checked_add(step)
                .ok_or("directory offset overflow")?;
        }
    }
}
fn remove_contents(
    _path: &Path,
    dir: &File,
    budget: &mut Budget,
    depth: usize,
) -> Result<(), String> {
    budget.consume()?;
    if depth >= 128 {
        return Err("profile cleanup depth exceeded; guard retained".into());
    }
    directory(dir)?;
    enumerate(dir, |name| {
        budget.consume()?;
        let child = relative(dir, name, true).map_err(nt_error)?;
        let attr = info(&child)?.dwFileAttributes;
        if attr & FILE_ATTRIBUTE_DIRECTORY != 0 && attr & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            remove_contents(Path::new(""), &child, budget, depth + 1)?;
        }
        // No attribute clearing/content writes: a hard link can refer outside.
        disposition(&child)?;
        drop(child);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn junction(link: &Path, target: &Path) {
        let output = std::process::Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction setup required, not skipped: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn windows_cleanup_proof_lock_and_neighbor_preservation() {
        let cache = tempfile::tempdir().unwrap();
        let profile = WindowsOwnedProfile::create(cache.path()).unwrap();
        let neighbor = WindowsOwnedProfile::create(cache.path()).unwrap();
        std::fs::create_dir(profile.path().join("nested")).unwrap();
        std::fs::write(profile.path().join("nested/data"), b"delete").unwrap();
        std::fs::write(neighbor.path().join("keep"), b"keep").unwrap();
        assert!(profile.cleanup(false, true).is_err());
        assert!(profile.cleanup(true, false).is_err());
        std::fs::write(
            profile.path().join("SingletonLock"),
            b"unrecognized Windows lock",
        )
        .unwrap();
        assert!(profile.cleanup(true, true).is_err());
        assert!(profile.root.lock().unwrap().is_some());
        std::fs::remove_file(profile.path().join("SingletonLock")).unwrap();
        profile.cleanup(true, true).unwrap();
        assert!(!profile.path().exists());
        assert_eq!(
            std::fs::read(neighbor.path().join("keep")).unwrap(),
            b"keep"
        );
        neighbor.cleanup(true, true).unwrap();
    }
    #[test]
    fn windows_root_and_ancestor_replacement_is_denied() {
        let cache = tempfile::tempdir().unwrap();
        let profile = WindowsOwnedProfile::create(cache.path()).unwrap();
        assert!(std::fs::rename(profile.path(), profile.path().with_extension("moved")).is_err());
        assert!(std::fs::remove_dir(profile.path()).is_err());
        assert!(std::fs::rename(cache.path(), cache.path().with_extension("moved")).is_err());
        profile.cleanup(true, true).unwrap();
    }
    #[test]
    fn windows_reparse_ancestor_rejected_and_child_deleted_not_followed() {
        let cache = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"keep").unwrap();
        let alias = cache.path().join("alias");
        junction(&alias, outside.path());
        assert!(WindowsOwnedProfile::create(&alias).is_err());
        assert!(WindowsOwnedProfile::create(&alias.join("subdir")).is_err());
        let profile = WindowsOwnedProfile::create(cache.path()).unwrap();
        junction(&profile.path().join("junction"), outside.path());
        std::fs::hard_link(
            outside.path().join("secret"),
            profile.path().join("hardlink"),
        )
        .unwrap();
        profile.cleanup(true, true).unwrap();
        assert_eq!(
            std::fs::read(outside.path().join("secret")).unwrap(),
            b"keep"
        );
        std::fs::remove_dir(alias).unwrap();
    }
    #[test]
    fn windows_readonly_and_budget_failures_retain_original_guard() {
        let cache = tempfile::tempdir().unwrap();
        let profile = WindowsOwnedProfile::create(cache.path()).unwrap();
        let readonly = profile.path().join("readonly");
        std::fs::write(&readonly, b"keep").unwrap();
        let mut permissions = std::fs::metadata(&readonly).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&readonly, permissions).unwrap();
        assert!(profile.cleanup(true, true).is_err());
        assert!(profile.root.lock().unwrap().is_some());
        let root = profile.root.lock().unwrap();
        let mut budget = Budget {
            nodes: 0,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        assert!(remove_contents(profile.path(), root.as_ref().unwrap(), &mut budget, 0).is_err());
        drop(root);
        let mut permissions = std::fs::metadata(&readonly).unwrap().permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(&readonly, permissions).unwrap();
        profile.cleanup(true, true).unwrap();
    }
    #[test]
    fn windows_persistent_quota_survives_guard_drop_and_concurrency() {
        let cache = tempfile::tempdir().unwrap();
        for _ in 0..32 {
            drop(WindowsOwnedProfile::create(cache.path()).unwrap());
        }
        assert!(WindowsOwnedProfile::create(cache.path()).is_err());
        let cache2 = tempfile::tempdir().unwrap();
        let workers: Vec<_> = (0..64)
            .map(|_| {
                let path = cache2.path().to_path_buf();
                std::thread::spawn(move || WindowsOwnedProfile::create(&path))
            })
            .collect();
        for worker in workers {
            drop(worker.join().unwrap());
        }
        let count = std::fs::read_dir(cache2.path().join("named_browser_profiles"))
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name() != ".allocation.lock")
            .count();
        assert!(count > 0 && count <= 32);
    }
    #[test]
    fn windows_protected_acl_is_exact_and_tampering_retains_profile() {
        let cache = tempfile::tempdir().unwrap();
        let profile = WindowsOwnedProfile::create(cache.path()).unwrap();
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; $a=Get-Acl -LiteralPath $env:PROFILE; if (!$a.AreAccessRulesProtected) {throw 'unprotected'}; $sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value; $r=@($a.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier])); if ($r.Count -ne 2) {throw 'unexpected ACE count'}; foreach ($x in $r) {if (($x.IdentityReference.Value -ne $sid -and $x.IdentityReference.Value -ne 'S-1-5-18') -or $x.AccessControlType -ne 'Allow' -or $x.FileSystemRights -ne 'FullControl') {throw 'unexpected ACE'}}; $a.SetAccessRuleProtection($false,$true); Set-Acl -LiteralPath $env:PROFILE -AclObject $a"])
            .env("PROFILE", profile.path()).output().unwrap();
        assert!(
            output.status.success(),
            "ACL inspection/tamper setup: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(profile.cleanup(true, true).is_err());
        assert!(profile.root.lock().unwrap().is_some());
        assert!(profile.path().exists());
    }
}
