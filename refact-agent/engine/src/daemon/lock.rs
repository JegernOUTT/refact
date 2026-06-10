use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

use fd_lock::{RwLock, RwLockWriteGuard};

pub fn open_lock(path: &Path) -> io::Result<RwLock<File>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)?;
    Ok(RwLock::new(file))
}

pub fn try_lock(lock: &mut RwLock<File>) -> io::Result<RwLockWriteGuard<'_, File>> {
    lock.try_write()
}

pub fn is_already_locked(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_lock_prevents_double_acquire() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.lock");
        let mut first = open_lock(&path).unwrap();
        let mut second = open_lock(&path).unwrap();
        let guard = try_lock(&mut first).unwrap();
        let error = try_lock(&mut second).unwrap_err();
        assert!(is_already_locked(&error));
        drop(guard);
        let _second_guard = try_lock(&mut second).unwrap();
    }
}
