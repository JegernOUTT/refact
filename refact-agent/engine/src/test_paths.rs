use std::path::PathBuf;

pub fn tempdir() -> tempfile::TempDir {
    let base =
        dunce::canonicalize(std::env::temp_dir()).expect("canonicalize temporary directory base");
    tempfile::tempdir_in(base).expect("create temporary directory in canonical base")
}

pub fn abs(relative: &str) -> PathBuf {
    let mut path = if cfg!(windows) {
        PathBuf::from("C:\\")
    } else {
        PathBuf::from("/")
    };
    for part in relative.split('/').filter(|part| !part.is_empty()) {
        path.push(part);
    }
    path
}

pub fn abs_str(relative: &str) -> String {
    abs(relative).to_string_lossy().into_owned()
}

#[test]
fn tempdir_has_canonical_path_and_owned_lifetime() {
    let directory = tempdir();
    let path = directory.path().to_path_buf();
    assert!(path.is_dir());
    assert_eq!(path, dunce::canonicalize(&path).unwrap());
    assert_eq!(
        path.parent().unwrap(),
        dunce::canonicalize(std::env::temp_dir()).unwrap()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(std::fs::metadata(&path).unwrap().uid(), unsafe {
            libc::geteuid()
        });
    }
    std::fs::write(path.join("owned-fixture"), b"fixture").unwrap();
    drop(directory);
    assert!(!path.exists());
}
