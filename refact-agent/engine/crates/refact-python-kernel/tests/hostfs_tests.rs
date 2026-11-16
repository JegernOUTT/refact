use monty_types::*;
use refact_python_kernel::{hostfs::*, kernel::OsBackend, protocol::*};
use std::{collections::BTreeMap, path::PathBuf};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "hostfs-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn fs(&self) -> HostFs {
        HostFs::new(HostFsConfig {
            project_root: self.0.clone(),
            ..Default::default()
        })
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn write(fs: &mut HostFs, p: &str, s: &str) {
    fs.handle(OsFunctionCall::WriteText(PathStringDataArgs {
        path: p.into(),
        data: s.into(),
    }))
    .unwrap();
}
#[test]
fn read_write_roundtrip_records_paths() {
    let t = Temp::new();
    let mut f = t.fs();
    write(&mut f, "a", "hello");
    assert_eq!(
        f.handle(OsFunctionCall::ReadText("a".into()))
            .unwrap()
            .as_ref()
            .as_str(),
        Some("hello")
    );
    let events = f.take_events();
    assert!(events.iter().any(
        |e| matches!(e,KernelEvent::Fs{path,bytes:5,..} if path==&t.0.join("a").to_string_lossy())
    ));
}
#[test]
fn relative_paths_resolve_against_project_root() {
    let t = Temp::new();
    write(&mut t.fs(), "a", "hi");
    assert_eq!(std::fs::read_to_string(t.0.join("a")).unwrap(), "hi");
}
#[test]
fn windows_path_mapping_round_trip() {
    let p = host_path("/C:/dir/file", Platform::Windows);
    assert_eq!(virtual_path(&p, Platform::Windows), "/C:/dir/file");
    let verbatim = PathBuf::from(r"\\?\C:\dir\file");
    let v = virtual_path(&verbatim, Platform::Windows);
    assert_eq!(v, "/C:/dir/file");
    assert_eq!(
        host_path(&v, Platform::Windows),
        PathBuf::from(r"C:\dir\file")
    );
    assert_eq!(
        virtual_path(&PathBuf::from(r"\\?\UNC\server\share\x"), Platform::Windows),
        "//server/share/x"
    );
    assert_eq!(
        virtual_path(&PathBuf::from(r"\\server\share\x"), Platform::Windows),
        "//server/share/x"
    );
    assert_eq!(
        virtual_path(&PathBuf::from(r"C:\dir\file"), Platform::Windows),
        "/C:/dir/file"
    );
    let volume = PathBuf::from(r"\\?\Volume{1b3b1146-4076-11e1-84aa-806e6f6e6963}\x");
    let v = virtual_path(&volume, Platform::Windows);
    assert_eq!(v, "//?/Volume{1b3b1146-4076-11e1-84aa-806e6f6e6963}/x");
    assert_eq!(host_path(&v, Platform::Windows), volume);
    assert_eq!(host_path("/tmp/a", Platform::Unix), PathBuf::from("/tmp/a"));
}
#[test]
fn mkdir_parents_exist_ok_semantics() {
    let t = Temp::new();
    let mut f = t.fs();
    let mkdir = |parents, exist_ok| {
        OsFunctionCall::Mkdir(MkdirCallArgs {
            path: "a/b".into(),
            parents,
            exist_ok,
        })
    };
    assert_eq!(
        f.handle(mkdir(false, false)).unwrap_err().0,
        "FileNotFoundError"
    );
    f.handle(mkdir(true, false)).unwrap();
    assert_eq!(
        f.handle(mkdir(true, false)).unwrap_err().0,
        "FileExistsError"
    );
    f.handle(mkdir(true, true)).unwrap();
}
#[test]
fn rename_and_unlink_record_operations() {
    let t = Temp::new();
    let mut f = t.fs();
    write(&mut f, "a", "hi");
    f.handle(OsFunctionCall::Rename(RenameCallArgs {
        src: "a".into(),
        dst: "b".into(),
    }))
    .unwrap();
    f.handle(OsFunctionCall::Unlink("b".into())).unwrap();
    let es = f.take_events();
    for op in ["Path.rename", "Path.unlink"] {
        assert!(es
            .iter()
            .any(|e| matches!(e,KernelEvent::Fs{op:o,..} if o==op)));
    }
}
#[test]
fn iterdir_and_scan_list_entries() {
    let t = Temp::new();
    let mut f = t.fs();
    write(&mut f, "a", "hi");
    let v = f.handle(OsFunctionCall::Iterdir(".".into())).unwrap();
    assert!(v.py_repr().contains("a"));
    let v = f
        .handle(OsFunctionCall::Scan(scan::ScanArgs::listing(
            ".".into(),
            Some(1),
            false,
        )))
        .unwrap();
    assert!(v.py_repr().contains("a"));
}
#[test]
fn errors_map_to_python_exception_types() {
    let t = Temp::new();
    let mut f = t.fs();
    assert_eq!(
        f.handle(OsFunctionCall::ReadText("missing".into()))
            .unwrap_err()
            .0,
        "FileNotFoundError"
    );
    assert_eq!(
        f.handle(OsFunctionCall::ReadText(".".into()))
            .unwrap_err()
            .0,
        "IsADirectoryError"
    );
    assert_eq!(
        f.handle(OsFunctionCall::Open(OpenCallArgs {
            path: ".".into(),
            mode: "r".parse().unwrap(),
        }))
        .unwrap_err()
        .0,
        "IsADirectoryError"
    );
    write(&mut f, "a", "hi");
    assert_eq!(
        f.handle(OsFunctionCall::Iterdir("a".into())).unwrap_err().0,
        "NotADirectoryError"
    );
}
#[test]
fn getenv_matches_configured_environment() {
    let mut f = HostFs::new(HostFsConfig {
        env: BTreeMap::from([("ONLY".into(), "yes".into())]),
        ..Default::default()
    });
    let get = |key: &str| {
        OsFunctionCall::Getenv(GetenvArgs {
            key: key.into(),
            default: MontyObject::none(),
        })
    };
    assert_eq!(
        f.handle(get("ONLY")).unwrap().as_ref().as_str(),
        Some("yes")
    );
    assert_eq!(f.handle(get("PATH")).unwrap().py_repr(), "None");
}
#[test]
fn first_write_capture_respects_limits() {
    let t = Temp::new();
    let captures = Temp::new();
    std::fs::write(t.0.join("a"), "original").unwrap();
    std::fs::write(t.0.join("big"), "too long").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        capture_dir: Some(captures.0.clone()),
        max_capture_file_bytes: 8,
        max_capture_run_bytes: 8,
        ..Default::default()
    });
    write(&mut f, "a", "new");
    write(&mut f, "a", "again");
    write(&mut f, "big", "new");
    write(&mut f, "missing", "new");
    let es = f.take_events();
    let captures: Vec<_> = es
        .iter()
        .filter_map(|e| {
            if let KernelEvent::WriteCapture {
                original, ..
            } = e
            {
                Some(original)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(captures.len(), 3);
    assert!(
        matches!(captures[0],WriteOriginal::File{capture_path} if std::fs::read_to_string(capture_path).unwrap()=="original")
    );
    assert!(matches!(captures[1], WriteOriginal::TooLarge));
    assert!(matches!(captures[2], WriteOriginal::Missing));
}
#[cfg(unix)]
#[test]
fn follows_symlinks_outside_project() {
    let t = Temp::new();
    let outside = Temp::new();
    std::fs::write(outside.0.join("a"), "secret").unwrap();
    std::os::unix::fs::symlink(outside.0.join("a"), t.0.join("link")).unwrap();
    let mut f = t.fs();
    assert_eq!(
        f.handle(OsFunctionCall::ReadText("link".into()))
            .unwrap()
            .as_ref()
            .as_str(),
        Some("secret")
    );
    write(&mut f, "link", "changed");
    assert_eq!(
        std::fs::read_to_string(outside.0.join("a")).unwrap(),
        "changed"
    );
}
#[test]
fn open_handle_read_write_seek_close() {
    let t = Temp::new();
    let mut f = t.fs();
    f.handle(OsFunctionCall::Open(OpenCallArgs {
        path: "a".into(),
        mode: "w".parse().unwrap(),
    }))
    .unwrap();
    assert_eq!(std::fs::read(t.0.join("a")).unwrap(), b"");
    write(&mut f, "a", "abc");
    f.handle(OsFunctionCall::Open(OpenCallArgs {
        path: "a".into(),
        mode: "r".parse().unwrap(),
    }))
    .unwrap();
}

#[test]
fn protected_prefix_exact_new_file_rename_and_read_are_denied() {
    let t = Temp::new();
    std::fs::create_dir(t.0.join("protected")).unwrap();
    std::fs::write(t.0.join("exact"), "secret").unwrap();
    std::fs::write(t.0.join("ordinary"), "public").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_write: PathDenyRules {
            prefixes: vec![t.0.join("protected")],
            files: vec![t.0.join("exact")],
            ..Default::default()
        },
        deny_read: PathDenyRules {
            prefixes: vec![],
            files: vec![t.0.join("exact")],
            ..Default::default()
        },
        ..Default::default()
    });
    let mut paths = vec!["protected/new", "protected/new/deep", "exact"];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(t.0.join("protected"), t.0.join("alias")).unwrap();
        paths.push("alias/new");
    }
    for path in paths {
        let err = f
            .handle(OsFunctionCall::WriteText(PathStringDataArgs {
                path: path.into(),
                data: "bad".into(),
            }))
            .unwrap_err();
        assert_eq!(err.0, "PermissionError");
        assert!(err.1.contains("engine configuration or state"));
    }
    for (src, dst) in [("ordinary", "protected/new"), ("protected", "moved")] {
        assert_eq!(
            f.handle(OsFunctionCall::Rename(RenameCallArgs {
                src: src.into(),
                dst: dst.into()
            }))
            .unwrap_err()
            .0,
            "PermissionError"
        );
    }
    assert_eq!(
        f.handle(OsFunctionCall::ReadText("exact".into()))
            .unwrap_err()
            .0,
        "PermissionError"
    );
    assert_eq!(
        std::fs::read_to_string(t.0.join("ordinary")).unwrap(),
        "public"
    );
}

#[test]
fn protected_descendants_do_not_block_mkdir_or_root_files() {
    let t = Temp::new();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_write: PathDenyRules {
            prefixes: vec![t.0.join(".refact")],
            files: vec![],
            ..Default::default()
        },
        ..Default::default()
    });
    f.handle(OsFunctionCall::Mkdir(MkdirCallArgs {
        path: ".".into(),
        parents: true,
        exist_ok: true,
    }))
    .unwrap();
    write(&mut f, "root.txt", "ok");
    assert_eq!(
        f.handle(OsFunctionCall::Rmdir(".".into())).unwrap_err().0,
        "PermissionError"
    );
}

#[test]
fn protected_file_hardlinks_cannot_be_read_even_under_allow_prefix() {
    let t = Temp::new();
    let workspace = t.0.join("config/workspace");
    std::fs::create_dir_all(workspace.join("artifacts")).unwrap();
    for name in ["secret.key", "secrets.yaml"] {
        let protected = t.0.join("config").join(name);
        std::fs::write(&protected, "protected contents").unwrap();
        std::fs::hard_link(&protected, t.0.join(name)).unwrap();
        std::fs::hard_link(&protected, workspace.join("artifacts").join(name)).unwrap();
    }
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_read: PathDenyRules {
            prefixes: vec![t.0.join("config")],
            files: vec!["config/secret.key".into(), "config/secrets.yaml".into()],
            allow_prefixes: vec![workspace],
        },
        ..Default::default()
    });
    for path in [
        "secret.key",
        "secrets.yaml",
        "config/workspace/artifacts/secret.key",
        "config/workspace/artifacts/secrets.yaml",
    ] {
        for call in [
            OsFunctionCall::ReadText(path.into()),
            OsFunctionCall::ReadBytes(path.into()),
        ] {
            let err = f.handle(call).unwrap_err();
            assert_eq!(err.0, "PermissionError", "{path}");
            assert!(!err.1.contains("protected contents"), "{}", err.1);
        }
    }
    assert!(f.take_events().is_empty());
}

#[test]
fn protected_file_hardlinks_cannot_be_written_even_under_allow_prefix() {
    let t = Temp::new();
    let artifacts = t.0.join("config/workspace/artifacts");
    std::fs::create_dir_all(&artifacts).unwrap();
    let protected = t.0.join("config/settings.yaml");
    std::fs::write(&protected, "trusted configuration").unwrap();
    std::fs::hard_link(&protected, t.0.join("alias")).unwrap();
    std::fs::hard_link(&protected, artifacts.join("alias")).unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_write: PathDenyRules {
            prefixes: vec![t.0.join("config")],
            files: vec![protected.clone()],
            allow_prefixes: vec![artifacts],
        },
        ..Default::default()
    });
    for path in ["alias", "config/workspace/artifacts/alias"] {
        for call in [
            OsFunctionCall::WriteText(PathStringDataArgs {
                path: path.into(),
                data: "bad".into(),
            }),
            OsFunctionCall::AppendText(PathStringDataArgs {
                path: path.into(),
                data: "bad".into(),
            }),
            OsFunctionCall::Open(OpenCallArgs {
                path: path.into(),
                mode: "w".parse().unwrap(),
            }),
        ] {
            assert_eq!(f.handle(call).unwrap_err().0, "PermissionError", "{path}");
            assert_eq!(std::fs::read(&protected).unwrap(), b"trusted configuration");
            assert_eq!(
                std::fs::read(t.0.join(path)).unwrap(),
                b"trusted configuration"
            );
        }
    }
    assert!(f.take_events().is_empty());
}

#[test]
fn ordinary_hardlinks_and_new_files_remain_allowed_with_explicit_denies() {
    let t = Temp::new();
    std::fs::write(t.0.join("protected"), "secret").unwrap();
    std::fs::write(t.0.join("ordinary"), "public").unwrap();
    std::fs::hard_link(t.0.join("ordinary"), t.0.join("alias")).unwrap();
    let rules = PathDenyRules {
        files: vec!["missing-protected".into(), "protected".into()],
        ..Default::default()
    };
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_write: rules.clone(),
        deny_read: rules,
        ..Default::default()
    });
    assert_eq!(
        f.handle(OsFunctionCall::ReadText("alias".into()))
            .unwrap()
            .as_ref()
            .as_str(),
        Some("public")
    );
    write(&mut f, "alias", "changed");
    f.handle(OsFunctionCall::AppendText(PathStringDataArgs {
        path: "alias".into(),
        data: " again".into(),
    }))
    .unwrap();
    assert_eq!(
        std::fs::read(t.0.join("ordinary")).unwrap(),
        b"changed again"
    );
    write(&mut f, "new", "created");
    assert_eq!(std::fs::read(t.0.join("new")).unwrap(), b"created");
    assert_eq!(std::fs::read(t.0.join("protected")).unwrap(), b"secret");
}

#[cfg(unix)]
#[test]
fn explicit_denies_need_no_read_access_to_unrelated_files() {
    use std::os::unix::fs::PermissionsExt;
    let t = Temp::new();
    std::fs::write(t.0.join("protected"), "secret").unwrap();
    std::fs::write(t.0.join("write-only"), "old").unwrap();
    let set_mode = |mode| {
        std::fs::set_permissions(
            t.0.join("write-only"),
            std::fs::Permissions::from_mode(mode),
        )
        .unwrap()
    };
    set_mode(0o200);
    let rules = PathDenyRules {
        files: vec!["protected".into()],
        ..Default::default()
    };
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_write: rules.clone(),
        deny_read: rules,
        ..Default::default()
    });
    f.handle(OsFunctionCall::Stat("write-only".into())).unwrap();
    let listing = f
        .handle(OsFunctionCall::Scan(scan::ScanArgs::listing(
            ".".into(),
            Some(1),
            false,
        )))
        .unwrap()
        .py_repr();
    assert!(listing.contains("write-only"), "{listing}");
    assert!(!listing.contains("protected"), "{listing}");
    write(&mut f, "write-only", "new");
    set_mode(0o600);
    assert_eq!(std::fs::read(t.0.join("write-only")).unwrap(), b"new");
}

#[test]
fn denied_files_spelled_with_a_trailing_separator_or_under_a_file_match_only_themselves() {
    let t = Temp::new();
    std::fs::write(t.0.join("protected"), "secret").unwrap();
    std::fs::write(t.0.join("ordinary"), "public").unwrap();
    std::fs::hard_link(t.0.join("protected"), t.0.join("alias")).unwrap();
    let rules = PathDenyRules {
        files: vec![
            t.0.join("ordinary").join("missing"),
            t.0.join("protected").join(""),
        ],
        ..Default::default()
    };
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_write: rules.clone(),
        deny_read: rules,
        ..Default::default()
    });
    assert_eq!(
        f.handle(OsFunctionCall::ReadText("ordinary".into()))
            .unwrap()
            .as_ref()
            .as_str(),
        Some("public")
    );
    write(&mut f, "ordinary", "changed");
    f.handle(OsFunctionCall::Stat("ordinary".into())).unwrap();
    for call in [
        OsFunctionCall::ReadText("alias".into()),
        OsFunctionCall::WriteText(PathStringDataArgs {
            path: "alias".into(),
            data: "bad".into(),
        }),
    ] {
        assert_eq!(f.handle(call).unwrap_err().0, "PermissionError");
    }
    assert_eq!(std::fs::read(t.0.join("protected")).unwrap(), b"secret");
    assert_eq!(std::fs::read(t.0.join("ordinary")).unwrap(), b"changed");
}

#[test]
fn artifacts_allow_prefix_uses_resolved_paths() {
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("state/artifacts")).unwrap();
    std::fs::write(t.0.join("state/state.json"), "trusted").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_write: PathDenyRules {
            prefixes: vec![t.0.join("state")],
            allow_prefixes: vec![t.0.join("state/artifacts")],
            ..Default::default()
        },
        ..Default::default()
    });
    write(&mut f, "state/artifacts/out", "ok");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            t.0.join("state/state.json"),
            t.0.join("state/artifacts/link"),
        )
        .unwrap();
        assert_eq!(
            f.handle(OsFunctionCall::WriteText(PathStringDataArgs {
                path: "state/artifacts/link".into(),
                data: "bad".into()
            }))
            .unwrap_err()
            .0,
            "PermissionError"
        );
    }
    assert_eq!(
        f.handle(OsFunctionCall::Rename(RenameCallArgs {
            src: ".".into(),
            dst: "moved".into()
        }))
        .unwrap_err()
        .0,
        "PermissionError"
    );
}

#[test]
fn protected_import_candidate_is_permission_error() {
    let t = Temp::new();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_read: PathDenyRules {
            prefixes: vec![t.0.join("config")],
            ..Default::default()
        },
        ..Default::default()
    });
    let result = f.handle_result(OsFunctionCall::Import(ImportArgs {
        module: "secret".into(),
        search_path: vec![t.0.join("config").to_string_lossy().into_owned()],
    }));
    assert!(
        matches!(result, ExtFunctionResult::Error(error) if error.exc_type().to_string() == "PermissionError")
    );
}

#[test]
fn config_workspace_read_exception_allows_capture_but_not_secrets() {
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("config/trajectories/python/chat/artifacts")).unwrap();
    std::fs::create_dir_all(t.0.join("config/trajectories/python/chat/runs")).unwrap();
    let workspace = t.0.join("config/trajectories/python/chat");
    std::fs::write(workspace.join("artifacts/out"), "before").unwrap();
    std::fs::write(workspace.join("runs/old"), "saved").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_write: PathDenyRules {
            prefixes: vec![t.0.join("config")],
            allow_prefixes: vec![workspace.join("artifacts")],
            ..Default::default()
        },
        deny_read: PathDenyRules {
            prefixes: vec![t.0.join("config")],
            allow_prefixes: vec![workspace.clone()],
            ..Default::default()
        },
        capture_dir: Some(workspace.join("runs/captures")),
        ..Default::default()
    });
    for p in [
        "config/trajectories/python/chat/artifacts/out",
        "config/trajectories/python/chat/runs/old",
    ] {
        f.handle(OsFunctionCall::ReadText(p.into())).unwrap();
    }
    write(
        &mut f,
        "config/trajectories/python/chat/artifacts/out",
        "after",
    );
    assert!(f.take_events().iter().any(|e| matches!(
        e,
        KernelEvent::WriteCapture {
            original: WriteOriginal::File { .. },
            ..
        }
    )));
    for p in ["config/secrets.yaml", "config/providers.d/x.yaml"] {
        assert_eq!(
            f.handle(OsFunctionCall::ReadText(p.into())).unwrap_err().0,
            "PermissionError"
        );
    }
}

fn artifacts_fs(t: &Temp) -> HostFs {
    HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_write: PathDenyRules {
            prefixes: vec![t.0.join("state")],
            allow_prefixes: vec![t.0.join("state/artifacts")],
            ..Default::default()
        },
        ..Default::default()
    })
}

#[cfg(unix)]
#[test]
fn dangling_symlink_writes_are_denied_at_every_component() {
    use std::os::unix::fs::symlink;
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("state/artifacts")).unwrap();
    symlink(
        t.0.join("state/new-secret"),
        t.0.join("state/artifacts/link"),
    )
    .unwrap();
    symlink(t.0.join("missing-dir"), t.0.join("dangling-dir")).unwrap();
    let mut f = artifacts_fs(&t);
    for path in ["state/artifacts/link", "dangling-dir/child"] {
        for call in [
            OsFunctionCall::WriteText(PathStringDataArgs {
                path: path.into(),
                data: "bad".into(),
            }),
            OsFunctionCall::AppendText(PathStringDataArgs {
                path: path.into(),
                data: "bad".into(),
            }),
            OsFunctionCall::Open(OpenCallArgs {
                path: path.into(),
                mode: "w".parse().unwrap(),
            }),
            OsFunctionCall::Mkdir(MkdirCallArgs {
                path: path.into(),
                parents: true,
                exist_ok: true,
            }),
        ] {
            assert_eq!(f.handle(call).unwrap_err().0, "PermissionError", "{path}");
        }
    }
    assert!(!t.0.join("state/new-secret").exists());
    assert!(!t.0.join("missing-dir").exists());
    assert!(f.take_events().is_empty());
}

#[test]
fn artifacts_root_cannot_be_removed_renamed_or_replaced() {
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("state/artifacts")).unwrap();
    std::fs::write(t.0.join("replacement"), "bad").unwrap();
    let mut f = artifacts_fs(&t);
    for call in [
        OsFunctionCall::Rmdir("state/artifacts".into()),
        OsFunctionCall::Rename(RenameCallArgs {
            src: "state/artifacts".into(),
            dst: "moved".into(),
        }),
        OsFunctionCall::Rename(RenameCallArgs {
            src: "replacement".into(),
            dst: "state/artifacts".into(),
        }),
    ] {
        assert_eq!(f.handle(call).unwrap_err().0, "PermissionError");
    }
    write(&mut f, "state/artifacts/out", "ok");
    assert!(t.0.join("state/artifacts").is_dir());
}

#[cfg(unix)]
#[test]
fn artifacts_allow_root_must_be_a_real_existing_directory() {
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("state/elsewhere")).unwrap();
    std::os::unix::fs::symlink(t.0.join("state/elsewhere"), t.0.join("state/artifacts")).unwrap();
    let mut f = artifacts_fs(&t);
    assert_eq!(
        f.handle(OsFunctionCall::WriteText(PathStringDataArgs {
            path: "state/artifacts/out".into(),
            data: "bad".into(),
        }))
        .unwrap_err()
        .0,
        "PermissionError"
    );
    assert!(!t.0.join("state/elsewhere/out").exists());
    std::fs::remove_file(t.0.join("state/artifacts")).unwrap();
    assert_eq!(
        f.handle(OsFunctionCall::Mkdir(MkdirCallArgs {
            path: "state/artifacts/new".into(),
            parents: true,
            exist_ok: true,
        }))
        .unwrap_err()
        .0,
        "PermissionError"
    );
}

#[test]
fn mkdir_existing_denied_directory_is_noop_or_file_exists() {
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("state/artifacts")).unwrap();
    let mut f = artifacts_fs(&t);
    for path in ["state", "state/artifacts"] {
        for parents in [false, true] {
            f.handle(OsFunctionCall::Mkdir(MkdirCallArgs {
                path: path.into(),
                parents,
                exist_ok: true,
            }))
            .unwrap();
            assert_eq!(
                f.handle(OsFunctionCall::Mkdir(MkdirCallArgs {
                    path: path.into(),
                    parents,
                    exist_ok: false,
                }))
                .unwrap_err()
                .0,
                "FileExistsError"
            );
        }
    }
    assert_eq!(
        f.handle(OsFunctionCall::Mkdir(MkdirCallArgs {
            path: "state/new".into(),
            parents: true,
            exist_ok: true,
        }))
        .unwrap_err()
        .0,
        "PermissionError"
    );
}

#[test]
fn read_denials_cover_metadata_and_listing_but_not_existence_probes() {
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("secret")).unwrap();
    std::fs::write(t.0.join("secret/hidden"), "secret").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_read: PathDenyRules {
            prefixes: vec![t.0.join("secret")],
            ..Default::default()
        },
        ..Default::default()
    });
    for call in [
        OsFunctionCall::Stat("secret/hidden".into()),
        OsFunctionCall::Iterdir("secret".into()),
    ] {
        assert_eq!(f.handle(call).unwrap_err().0, "PermissionError");
    }
    for call in [
        OsFunctionCall::Exists("secret/hidden".into()),
        OsFunctionCall::IsFile("secret/hidden".into()),
        OsFunctionCall::IsDir("secret".into()),
    ] {
        assert_eq!(f.handle(call).unwrap().py_repr(), "True");
    }
}

#[test]
fn scan_skips_denied_directories_and_keeps_allowed_workspace() {
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("secret")).unwrap();
    std::fs::create_dir_all(t.0.join("public")).unwrap();
    std::fs::write(t.0.join("secret/hidden"), "secret").unwrap();
    std::fs::write(t.0.join("public/visible"), "public").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_read: PathDenyRules {
            prefixes: vec![t.0.join("secret")],
            ..Default::default()
        },
        ..Default::default()
    });
    let result = f
        .handle(OsFunctionCall::Scan(scan::ScanArgs::listing(
            ".".into(),
            Some(4),
            false,
        )))
        .unwrap()
        .py_repr();
    assert!(!result.contains("hidden"), "{result}");
    assert!(result.contains("visible"), "{result}");
}

#[tokio::test]
async fn python_glob_skips_read_denied_directories() {
    use refact_python_kernel::client::{ClientEvent, ClientOptions, KernelClient};
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("secret")).unwrap();
    std::fs::create_dir_all(t.0.join("public")).unwrap();
    std::fs::write(t.0.join("secret/hidden.txt"), "secret").unwrap();
    std::fs::write(t.0.join("public/visible.txt"), "public").unwrap();
    let mut client = KernelClient::spawn_in_thread(
        Box::new(HostFs::new(HostFsConfig {
            project_root: t.0.clone(),
            deny_read: PathDenyRules {
                prefixes: vec![t.0.join("secret")],
                ..Default::default()
            },
            ..Default::default()
        })),
        ClientOptions::default(),
    )
    .unwrap();
    client.next_event().await;
    client
        .send(KernelCommand::Feed {
            run_id: "glob-deny".into(),
            code: "from pathlib import Path\nsorted(p.name for p in Path('.').glob('**/*.txt'))"
                .into(),
            globals: vec![],
            limits: RunLimits::default(),
        })
        .await
        .unwrap();
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(10), client.next_event())
            .await
            .unwrap();
        match event {
            ClientEvent::Kernel(KernelEvent::Fs {
                ..
            }) => {}
            ClientEvent::Kernel(KernelEvent::Completed {
                value, ..
            }) => {
                assert_eq!(value, serde_json::json!(["visible.txt"]));
                break;
            }
            other => panic!("unexpected glob result: {other:?}"),
        }
    }
    client.send(KernelCommand::Shutdown).await.unwrap();
}

#[test]
fn denial_messages_distinguish_read_write_and_artifacts_root() {
    let t = Temp::new();
    let root = t.0.clone();
    std::fs::create_dir_all(root.join("ws/artifacts")).unwrap();
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::write(root.join("config/secret.yaml"), "x").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: root.clone(),
        deny_write: PathDenyRules {
            prefixes: vec![root.join("ws")],
            allow_prefixes: vec![root.join("ws/artifacts")],
            ..Default::default()
        },
        deny_read: PathDenyRules {
            prefixes: vec![root.join("config")],
            ..Default::default()
        },
        ..Default::default()
    });
    let read = f
        .handle(OsFunctionCall::ReadText("config/secret.yaml".into()))
        .unwrap_err();
    assert_eq!(read.0, "PermissionError");
    assert!(
        read.1.contains("is not allowed (engine configuration)"),
        "{}",
        read.1
    );
    assert!(!read.1.contains("file-edit"), "{}", read.1);
    let write = f
        .handle(OsFunctionCall::WriteText(PathStringDataArgs {
            path: "ws/state.json".into(),
            data: "x".into(),
        }))
        .unwrap_err();
    assert!(write.1.contains("file-edit tools"), "{}", write.1);
    let rename = f
        .handle(OsFunctionCall::Rename(RenameCallArgs {
            src: "ws/artifacts".into(),
            dst: "moved".into(),
        }))
        .unwrap_err();
    assert!(
        rename.1.contains("itself cannot be renamed or removed")
            && rename.1.contains("files inside it"),
        "{}",
        rename.1
    );
    let rmdir = f
        .handle(OsFunctionCall::Rmdir("ws/artifacts".into()))
        .unwrap_err();
    assert!(
        rmdir.1.contains("itself cannot be renamed or removed"),
        "{}",
        rmdir.1
    );
    f.handle(OsFunctionCall::WriteText(PathStringDataArgs {
        path: "ws/artifacts/out.txt".into(),
        data: "ok".into(),
    }))
    .unwrap();
    f.take_events();
    f.handle(OsFunctionCall::Mkdir(MkdirCallArgs {
        path: "ws/artifacts".into(),
        parents: true,
        exist_ok: true,
    }))
    .unwrap();
    assert!(
        f.take_events().is_empty(),
        "no-op mkdir must not be a write"
    );
}

#[tokio::test]
async fn chdir_into_a_read_denied_directory_does_not_open_imports() {
    use refact_python_kernel::client::{ClientEvent, ClientOptions, KernelClient};
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("guarded/pkg_x")).unwrap();
    std::fs::write(t.0.join("guarded/mod_x.py"), "VALUE = 1\n").unwrap();
    std::fs::write(t.0.join("guarded/pkg_x/__init__.py"), "VALUE = 2\n").unwrap();
    let mut client = KernelClient::spawn_in_thread(
        Box::new(HostFs::new(HostFsConfig {
            project_root: t.0.clone(),
            deny_read: PathDenyRules {
                prefixes: vec![t.0.join("guarded")],
                ..Default::default()
            },
            ..Default::default()
        })),
        ClientOptions::default(),
    )
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), client.next_event())
        .await
        .unwrap();
    for (run, module) in [("chdir-module", "mod_x"), ("chdir-package", "pkg_x")] {
        client
            .send(KernelCommand::Feed {
                run_id: run.into(),
                code: format!(
                    "import os\nos.chdir('guarded')\ntry:\n    import {module}\n    out = 'imported'\nexcept PermissionError as e:\n    out = 'PermissionError'\nos.chdir('..')\nout"
                ),
                globals: vec![],
                limits: RunLimits::default(),
            })
            .await
            .unwrap();
        loop {
            let event =
                tokio::time::timeout(std::time::Duration::from_secs(15), client.next_event())
                    .await
                    .unwrap();
            match event {
                ClientEvent::Kernel(KernelEvent::Completed {
                    value, ..
                }) => {
                    assert_eq!(value, serde_json::json!("PermissionError"), "{module}");
                    break;
                }
                ClientEvent::Kernel(KernelEvent::Raised {
                    exc_type,
                    message,
                    ..
                }) => {
                    assert_eq!(exc_type, "PermissionError", "{module}: {message}");
                    break;
                }
                _ => {}
            }
        }
    }
}
