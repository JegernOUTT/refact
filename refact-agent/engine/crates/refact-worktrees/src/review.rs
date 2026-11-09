use std::path::{Component, Path};

/// Bytes already captured and integrity-checked by the review store. No source reads occur here.
#[derive(Clone, Debug)]
pub struct SnapshotFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub mode: u32,
}

fn path_cstring(path: &Path) -> Result<std::ffi::CString, git2::Error> {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes()
    };
    #[cfg(not(unix))]
    let bytes = path
        .to_str()
        .ok_or_else(|| git2::Error::from_str("non UTF8 worktree path"))?
        .as_bytes();
    std::ffi::CString::new(bytes).map_err(|_| git2::Error::from_str("worktree path NUL"))
}

pub(crate) fn materialize(
    source: &Path,
    destination: &Path,
    id: &str,
    branch: &str,
    files: Vec<SnapshotFile>,
) -> Result<(String, std::path::PathBuf), String> {
    let run = || -> Result<_, git2::Error> {
        let repo = git2::Repository::open(source)?;
        let mut index = git2::Index::new()?;
        let mut seen = std::collections::HashSet::new();
        let mut total = 0usize;
        if files.len() > 20_000 {
            return Err(git2::Error::from_str("snapshot file limit"));
        }
        for file in files {
            total = total
                .checked_add(file.bytes.len())
                .ok_or_else(|| git2::Error::from_str("snapshot overflow"))?;
            if total > 256 * 1024 * 1024
                || file.bytes.len() > 16 * 1024 * 1024
                || file.path.is_empty()
                || file.path.contains('\\')
                || !Path::new(&file.path)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)))
                || file.path.split('/').any(|p| {
                    p.is_empty()
                        || p.eq_ignore_ascii_case(".git")
                        || p.eq_ignore_ascii_case(".refact")
                })
                || !matches!(file.mode, 0o100644 | 0o100755)
                || !seen.insert(file.path.clone())
            {
                return Err(git2::Error::from_str("unsafe snapshot path, mode or size"));
            }
            let oid = repo.blob(&file.bytes)?;
            index.add(&git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode: file.mode,
                uid: 0,
                gid: 0,
                file_size: file.bytes.len() as u32,
                id: oid,
                flags: 0,
                flags_extended: 0,
                path: file.path.into_bytes(),
            })?;
        }
        let tree_oid = index.write_tree_to(&repo)?;
        let tree = repo.find_tree(tree_oid)?;
        let signature =
            git2::Signature::new("Refact Review", "review@localhost", &git2::Time::new(0, 0))?;
        let commit = repo.commit(
            None,
            &signature,
            &signature,
            "Immutable review execution snapshot",
            &tree,
            &[],
        )?;
        repo.reference(
            &format!("refs/refact/reviews/{id}"),
            commit,
            false,
            "pin review snapshot",
        )?;
        let commit_object = repo.find_commit(commit)?;
        let branch_ref = repo.branch(branch, &commit_object, false)?.into_reference();
        let mut options = git2::WorktreeAddOptions::new();
        options.reference(Some(&branch_ref));
        // git2 exposes only &self -> *const options. Copy the POD options into our
        // own stack allocation rather than mutating through a shared-reference pointer.
        // Its borrowed reference points to branch_ref, alive through the synchronous call.
        let source_c = path_cstring(source)?;
        let destination_c = path_cstring(destination)?;
        let name_c =
            std::ffi::CString::new(id).map_err(|_| git2::Error::from_str("worktree name NUL"))?;
        struct RawRepository(*mut libgit2_sys::git_repository);
        impl Drop for RawRepository {
            fn drop(&mut self) {
                unsafe { libgit2_sys::git_repository_free(self.0) };
            }
        }
        struct RawWorktree(*mut libgit2_sys::git_worktree);
        impl Drop for RawWorktree {
            fn drop(&mut self) {
                unsafe { libgit2_sys::git_worktree_free(self.0) };
            }
        }
        fn checked(code: i32) -> Result<(), git2::Error> {
            if code >= 0 {
                return Ok(());
            }
            // libgit2 error storage is thread-local and read immediately after the call.
            let message = unsafe {
                let error = libgit2_sys::git_error_last();
                if error.is_null() || (*error).message.is_null() {
                    "libgit2 worktree creation failed".into()
                } else {
                    std::ffi::CStr::from_ptr((*error).message)
                        .to_string_lossy()
                        .into_owned()
                }
            };
            Err(git2::Error::from_str(&message))
        }
        unsafe {
            let mut raw_options = std::ptr::read(options.raw());
            raw_options.checkout_options.disable_filters = 1;
            let mut repository = std::ptr::null_mut();
            checked(libgit2_sys::git_repository_open(
                &mut repository,
                source_c.as_ptr(),
            ))?;
            let repository = RawRepository(repository);
            let mut worktree = std::ptr::null_mut();
            checked(libgit2_sys::git_worktree_add(
                &mut worktree,
                repository.0,
                name_c.as_ptr(),
                destination_c.as_ptr(),
                &raw_options,
            ))?;
            let _worktree = RawWorktree(worktree);
        }
        Ok((
            commit.to_string(),
            repo.workdir().unwrap_or(source).to_path_buf(),
        ))
    };
    run().map_err(|e| {
        format!(
            "Review snapshot materialization failed (retain any pinned objects for recovery): {e}"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_materialization_rejects_escape_and_preserves_exact_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        git2::Repository::init(&source).unwrap();
        let bad = SnapshotFile {
            path: "../escape".into(),
            bytes: b"bad".to_vec(),
            mode: 0o100644,
        };
        assert!(materialize(
            &source,
            &temp.path().join("bad"),
            "bad",
            "refact/review/bad",
            vec![bad]
        )
        .is_err());
        assert!(!temp.path().join("escape").exists());
        let bytes = b"original\r\n\0binary".to_vec();
        let destination = temp.path().join("checkout");
        let (commit, _) = materialize(
            &source,
            &destination,
            "safe",
            "refact/review/safe",
            vec![SnapshotFile {
                path: "dir/file".into(),
                bytes: bytes.clone(),
                mode: 0o100755,
            }],
        )
        .unwrap();
        assert_eq!(std::fs::read(destination.join("dir/file")).unwrap(), bytes);
        let repo = git2::Repository::open(destination).unwrap();
        assert_eq!(repo.head().unwrap().target().unwrap().to_string(), commit);
        assert_eq!(
            repo.find_commit(repo.head().unwrap().target().unwrap())
                .unwrap()
                .parent_count(),
            0
        );
    }
}
