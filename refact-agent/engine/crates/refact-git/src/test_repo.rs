use std::fs;
use std::path::Path;

use git2::{Oid, Repository};

pub(crate) fn init_repo() -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Test User").unwrap();
    config.set_str("user.email", "test@example.com").unwrap();
    (dir, repo)
}

pub(crate) fn write(root: &Path, path: &str, content: impl AsRef<[u8]>) {
    let file = root.join(path);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(file, content).unwrap();
}

pub(crate) fn stage(repo: &Repository, paths: &[&str]) {
    let mut index = repo.index().unwrap();
    for path in paths {
        if repo.workdir().unwrap().join(path).exists() {
            index.add_path(Path::new(path)).unwrap();
        } else {
            index.remove_path(Path::new(path)).unwrap();
        }
    }
    index.write().unwrap();
}

pub(crate) fn commit(repo: &Repository, message: &str) -> Oid {
    let mut index = repo.index().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = git2::Signature::new(
        "Test User",
        "test@example.com",
        &git2::Time::new(1_700_000_000 + commit_count(repo) as i64 * 60, 0),
    )
    .unwrap();
    let parents = repo
        .head()
        .ok()
        .and_then(|head| head.target())
        .and_then(|oid| repo.find_commit(oid).ok())
        .into_iter()
        .collect::<Vec<_>>();
    let parent_refs = parents.iter().collect::<Vec<_>>();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parent_refs,
    )
    .unwrap()
}

fn commit_count(repo: &Repository) -> usize {
    let Ok(mut walk) = repo.revwalk() else {
        return 0;
    };
    if walk.push_head().is_err() {
        return 0;
    }
    walk.count()
}

pub(crate) fn commit_files(repo: &Repository, files: &[(&str, &str)], message: &str) -> Oid {
    let root = repo.workdir().unwrap().to_path_buf();
    for (path, content) in files {
        write(&root, path, content);
    }
    stage(
        repo,
        &files.iter().map(|(path, _)| *path).collect::<Vec<_>>(),
    );
    commit(repo, message)
}
