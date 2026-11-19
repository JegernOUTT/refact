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
            ..Default::default()
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
            ..Default::default()
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

const READ_ONLY: &str = "this chat is read-only: scripts may write only under ARTIFACTS_DIR";

struct Chat {
    t: Temp,
    project: PathBuf,
    workspace: PathBuf,
    artifacts: PathBuf,
    run: PathBuf,
    outside: PathBuf,
}
impl Chat {
    fn new() -> Self {
        let t = Temp::new();
        let root = std::fs::canonicalize(&t.0).unwrap();
        let workspace = root.join("workspace");
        let chat = Self {
            project: root.join("project"),
            artifacts: workspace.join("artifacts"),
            run: workspace.join("runs/r1"),
            outside: root.join("outside"),
            workspace,
            t,
        };
        for dir in [&chat.project, &chat.artifacts, &chat.run, &chat.outside] {
            std::fs::create_dir_all(dir).unwrap();
        }
        chat
    }
    fn config(&self, read_only: bool) -> HostFsConfig {
        HostFsConfig {
            project_root: self.project.clone(),
            capture_dir: Some(self.run.join("captures")),
            deny_write: PathDenyRules {
                prefixes: vec![self.workspace.clone(), self.project.join(".refact")],
                files: vec![self.project.join("settings.yaml")],
                allow_prefixes: vec![self.artifacts.clone()],
                read_only: read_only.then(|| READ_ONLY.to_string()),
            },
            ..Default::default()
        }
    }
    fn fs(&self, read_only: bool) -> HostFs {
        HostFs::new(self.config(read_only))
    }
}
fn text_of(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}
fn refused(p: &std::path::Path) -> (String, String) {
    (
        "PermissionError".into(),
        format!("{READ_ONLY}; refused: {}", p.display()),
    )
}
fn snapshot(root: &std::path::Path) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            let meta = std::fs::symlink_metadata(&p).unwrap();
            let value = if meta.is_symlink() {
                format!("-> {}", std::fs::read_link(&p).unwrap().display())
            } else if meta.is_dir() {
                dirs.push(p.clone());
                "dir".into()
            } else {
                String::from_utf8_lossy(&std::fs::read(&p).unwrap()).into_owned()
            };
            out.insert(p, value);
        }
    }
    out
}
fn write_call(p: &std::path::Path, data: &str) -> OsFunctionCall {
    OsFunctionCall::WriteText(PathStringDataArgs {
        path: text_of(p).as_str().into(),
        data: data.into(),
    })
}
fn append_call(p: &std::path::Path, data: &str) -> OsFunctionCall {
    OsFunctionCall::AppendText(PathStringDataArgs {
        path: text_of(p).as_str().into(),
        data: data.into(),
    })
}
fn open_call(p: &std::path::Path, mode: &str) -> OsFunctionCall {
    OsFunctionCall::Open(OpenCallArgs {
        path: text_of(p).as_str().into(),
        mode: mode.parse().unwrap(),
    })
}
fn mkdir_call(p: &std::path::Path, parents: bool, exist_ok: bool) -> OsFunctionCall {
    OsFunctionCall::Mkdir(MkdirCallArgs {
        path: text_of(p).as_str().into(),
        parents,
        exist_ok,
    })
}
fn rename_call(src: &std::path::Path, dst: &std::path::Path) -> OsFunctionCall {
    OsFunctionCall::Rename(RenameCallArgs {
        src: text_of(src).as_str().into(),
        dst: text_of(dst).as_str().into(),
    })
}

#[test]
fn read_only_refuses_every_write_outside_artifacts() {
    let c = Chat::new();
    let a = c.project.join("a.txt");
    let new = c.project.join("new.txt");
    let artifact = c.artifacts.join("out.txt");
    std::fs::write(&a, "original").unwrap();
    std::fs::create_dir(c.project.join("empty")).unwrap();
    std::fs::write(&artifact, "artifact").unwrap();
    let mut f = c.fs(true);
    let cases = vec![
        (a.clone(), write_call(&a, "x")),
        (a.clone(), append_call(&a, "x")),
        (
            a.clone(),
            OsFunctionCall::WriteBytes(PathBytesDataArgs {
                path: text_of(&a).as_str().into(),
                data: b"x".to_vec(),
            }),
        ),
        (
            a.clone(),
            OsFunctionCall::AppendBytes(PathBytesDataArgs {
                path: text_of(&a).as_str().into(),
                data: b"x".to_vec(),
            }),
        ),
        (new.clone(), write_call(&new, "x")),
        (
            c.outside.join("x.txt"),
            write_call(&c.outside.join("x.txt"), "x"),
        ),
        (a.clone(), open_call(&a, "w")),
        (a.clone(), open_call(&a, "a")),
        (new.clone(), open_call(&new, "wb")),
        (new.clone(), open_call(&new, "ab")),
        (
            new.clone(),
            OsFunctionCall::CreateExclusive(CreateExclusiveCallArgs {
                path: text_of(&new).as_str().into(),
                mode: "w".parse().unwrap(),
                permissions: 0o644,
            }),
        ),
        (
            c.project.join("dir"),
            mkdir_call(&c.project.join("dir"), false, false),
        ),
        (
            c.project.join("dir/deep"),
            mkdir_call(&c.project.join("dir/deep"), true, true),
        ),
        (
            a.clone(),
            OsFunctionCall::Unlink(text_of(&a).as_str().into()),
        ),
        (
            c.project.join("empty"),
            OsFunctionCall::Rmdir(text_of(&c.project.join("empty")).as_str().into()),
        ),
        (a.clone(), rename_call(&a, &c.artifacts.join("moved.txt"))),
        (
            c.project.join("b.txt"),
            rename_call(&artifact, &c.project.join("b.txt")),
        ),
        (a.clone(), rename_call(&a, &c.project.join("b.txt"))),
    ];
    let before = snapshot(&c.t.0);
    for (path, call) in cases {
        let what = format!("{call:?}");
        assert_eq!(f.handle(call).unwrap_err(), refused(&path), "{what}");
    }
    assert_eq!(snapshot(&c.t.0), before);
    assert!(f.take_events().is_empty());
}

#[test]
fn read_only_keeps_reads_and_artifact_writes() {
    let c = Chat::new();
    std::fs::write(c.project.join("a.txt"), "code").unwrap();
    let mut f = c.fs(true);
    let a = c.project.join("a.txt");
    assert_eq!(f.handle(write_call(&a, "x")).unwrap_err(), refused(&a));
    for call in [
        OsFunctionCall::ReadText(text_of(&a).as_str().into()),
        OsFunctionCall::ReadBytes(text_of(&a).as_str().into()),
        OsFunctionCall::Stat(text_of(&a).as_str().into()),
        OsFunctionCall::Exists(text_of(&a).as_str().into()),
        OsFunctionCall::Iterdir(text_of(&c.project).as_str().into()),
        open_call(&a, "r"),
    ] {
        f.handle(call).unwrap();
    }
    let out = c.artifacts.join("out.txt");
    let handle = c.artifacts.join("handle.txt");
    for call in [
        write_call(&out, "1"),
        append_call(&out, "2"),
        open_call(&handle, "w"),
        mkdir_call(&c.artifacts.join("sub/deep"), true, false),
        mkdir_call(&c.artifacts, true, true),
        rename_call(&out, &c.artifacts.join("sub/out.txt")),
        OsFunctionCall::Unlink(text_of(&handle).as_str().into()),
        OsFunctionCall::Rmdir(text_of(&c.artifacts.join("sub/deep")).as_str().into()),
    ] {
        let what = format!("{call:?}");
        f.handle(call).unwrap_or_else(|e| panic!("{what}: {e:?}"));
    }
    assert_eq!(
        std::fs::read_to_string(c.artifacts.join("sub/out.txt")).unwrap(),
        "12"
    );
    assert!(!handle.exists() && !c.artifacts.join("sub/deep").exists());
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "code");
}

#[test]
fn read_only_text_comes_before_every_other_rule() {
    let c = Chat::new();
    std::fs::write(c.project.join("settings.yaml"), "trusted").unwrap();
    std::fs::write(c.run.join("run.json"), "{}").unwrap();
    let mut paths = vec![
        c.workspace.join("state.json"),
        c.run.join("run.json"),
        c.project.join(".refact/hooks.yaml"),
        c.project.join("settings.yaml"),
    ];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(c.outside.join("missing"), c.project.join("dangling")).unwrap();
        paths.push(c.project.join("dangling"));
    }
    let mut f = c.fs(true);
    let before = snapshot(&c.t.0);
    for p in paths {
        assert_eq!(f.handle(write_call(&p, "x")).unwrap_err(), refused(&p));
    }
    for call in [
        rename_call(&c.artifacts, &c.project.join("moved")),
        OsFunctionCall::Rmdir(text_of(&c.artifacts).as_str().into()),
    ] {
        assert_eq!(f.handle(call).unwrap_err(), refused(&c.artifacts));
    }
    assert_eq!(snapshot(&c.t.0), before);
}

#[cfg(unix)]
#[test]
fn read_only_judges_an_entry_by_its_own_name_not_a_link_target() {
    use std::os::unix::fs::symlink;
    let c = Chat::new();
    let kept = c.artifacts.join("kept.txt");
    let other = c.artifacts.join("other.txt");
    let target = c.project.join("target.txt");
    std::fs::write(&kept, "kept").unwrap();
    std::fs::write(&other, "other").unwrap();
    std::fs::write(&target, "target").unwrap();
    let alias = c.project.join("alias");
    let link = c.artifacts.join("link");
    symlink(&kept, &alias).unwrap();
    symlink(&target, &link).unwrap();
    let mut f = c.fs(true);
    let before = snapshot(&c.t.0);
    for call in [
        OsFunctionCall::Unlink(text_of(&alias).as_str().into()),
        rename_call(&alias, &c.artifacts.join("moved")),
        rename_call(&other, &alias),
        write_call(&alias, "x"),
        append_call(&alias, "x"),
        open_call(&alias, "w"),
    ] {
        let what = format!("{call:?}");
        assert_eq!(f.handle(call).unwrap_err(), refused(&alias), "{what}");
    }
    let through_link = (
        "PermissionError".to_string(),
        format!("{READ_ONLY}; refused: {} is a symlink", link.display()),
    );
    for call in [
        write_call(&link, "x"),
        append_call(&link, "x"),
        open_call(&link, "w"),
        open_call(&link, "a"),
    ] {
        let what = format!("{call:?}");
        assert_eq!(f.handle(call).unwrap_err(), through_link, "{what}");
    }
    assert_eq!(snapshot(&c.t.0), before);
    assert!(f.take_events().is_empty());
    let renamed = c.artifacts.join("link2");
    f.handle(rename_call(&link, &renamed)).unwrap();
    assert_eq!(std::fs::read_link(&renamed).unwrap(), target);
    f.handle(OsFunctionCall::Unlink(text_of(&renamed).as_str().into()))
        .unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "target");
    assert!(!f.take_events().iter().any(|e| matches!(
        e,
        KernelEvent::WriteCapture {
            original: WriteOriginal::File { .. },
            ..
        }
    )));
}

#[test]
fn read_only_refuses_content_writes_to_a_hard_linked_artifact() {
    let c = Chat::new();
    let shared = c.project.join("shared.txt");
    let artifact = c.artifacts.join("shared.txt");
    std::fs::write(&shared, "shared").unwrap();
    std::fs::hard_link(&shared, &artifact).unwrap();
    let mut f = c.fs(true);
    let linked = (
        "PermissionError".to_string(),
        format!("{READ_ONLY}; refused: {} has 2 links", artifact.display()),
    );
    for call in [
        write_call(&artifact, "x"),
        append_call(&artifact, "x"),
        open_call(&artifact, "w"),
        open_call(&artifact, "a"),
    ] {
        let what = format!("{call:?}");
        assert_eq!(f.handle(call).unwrap_err(), linked, "{what}");
    }
    assert_eq!(std::fs::read_to_string(&shared).unwrap(), "shared");
    assert!(f.take_events().is_empty());
    f.handle(OsFunctionCall::Unlink(text_of(&artifact).as_str().into()))
        .unwrap();
    assert_eq!(std::fs::read_to_string(&shared).unwrap(), "shared");
}

#[test]
fn read_only_resolves_parent_references_before_judging() {
    let c = Chat::new();
    std::fs::create_dir(c.artifacts.join("sub")).unwrap();
    let mut f = c.fs(true);
    let escape = c.artifacts.join("../escape.txt");
    assert_eq!(
        f.handle(write_call(&escape, "x")).unwrap_err(),
        refused(&escape)
    );
    let runs = c.artifacts.join("../runs");
    assert_eq!(
        f.handle(rename_call(&runs, &c.artifacts.join("runs")))
            .unwrap_err(),
        refused(&runs)
    );
    assert!(!c.workspace.join("escape.txt").exists());
    f.handle(write_call(&c.artifacts.join("sub/../ok.txt"), "ok"))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(c.artifacts.join("ok.txt")).unwrap(),
        "ok"
    );
}

#[test]
fn read_only_mkdir_of_an_existing_directory_changes_nothing() {
    let c = Chat::new();
    let mut f = c.fs(true);
    f.handle(mkdir_call(&c.project, true, true)).unwrap();
    f.handle(mkdir_call(&c.project, false, true)).unwrap();
    assert_eq!(
        f.handle(mkdir_call(&c.project, false, false))
            .unwrap_err()
            .0,
        "FileExistsError"
    );
    let new = c.project.join("new");
    assert_eq!(
        f.handle(mkdir_call(&new, true, true)).unwrap_err(),
        refused(&new)
    );
    assert!(!new.exists());
    assert!(f.take_events().is_empty());
}

#[cfg(unix)]
#[test]
fn writes_beneath_a_directory_link_inside_artifacts_are_judged_where_it_leads() {
    let c = Chat::new();
    std::os::unix::fs::symlink(&c.project, c.artifacts.join("out")).unwrap();
    std::os::unix::fs::symlink(&c.run, c.artifacts.join("run")).unwrap();
    std::fs::write(c.artifacts.join("a.txt"), "a").unwrap();
    let out = c.artifacts.join("out");
    let mut f = c.fs(true);
    let before = snapshot(&c.t.0);
    for (p, call) in [
        (out.join("new"), mkdir_call(&out.join("new"), false, false)),
        (
            out.join("deep/er"),
            mkdir_call(&out.join("deep/er"), true, true),
        ),
        (out.join("x.txt"), write_call(&out.join("x.txt"), "x")),
        (out.join("y.txt"), open_call(&out.join("y.txt"), "w")),
        (
            out.join("a.txt"),
            rename_call(&c.artifacts.join("a.txt"), &out.join("a.txt")),
        ),
    ] {
        let what = format!("{call:?}");
        assert_eq!(f.handle(call).unwrap_err(), refused(&p), "{what}");
    }
    assert_eq!(snapshot(&c.t.0), before);
    assert!(f.take_events().is_empty());
    let mut f = c.fs(false);
    let run = c.artifacts.join("run");
    for call in [
        write_call(&run.join("run.json"), "x"),
        mkdir_call(&run.join("new"), false, false),
        rename_call(&c.artifacts.join("a.txt"), &run.join("a.txt")),
    ] {
        let what = format!("{call:?}");
        assert_eq!(f.handle(call).unwrap_err().0, "PermissionError", "{what}");
    }
    assert_eq!(snapshot(&c.t.0), before);
    f.handle(write_call(&out.join("free.txt"), "x")).unwrap();
    assert_eq!(
        std::fs::read_to_string(c.project.join("free.txt")).unwrap(),
        "x"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn allow_containment_never_folds_case_while_deny_rules_do() {
    let c = Chat::new();
    let variant = c.workspace.join("ARTIFACTS");
    std::fs::create_dir(&variant).unwrap();
    std::fs::create_dir(c.project.join(".REFACT")).unwrap();
    std::fs::write(c.artifacts.join("a.txt"), "a").unwrap();
    let folding = |read_only| {
        let mut config = c.config(read_only);
        config.fold_case = true;
        HostFs::new(config)
    };
    let mut f = folding(true);
    let before = snapshot(&c.t.0);
    for (p, call) in [
        (
            variant.join("x.txt"),
            write_call(&variant.join("x.txt"), "x"),
        ),
        (
            variant.join("y.txt"),
            open_call(&variant.join("y.txt"), "w"),
        ),
        (
            variant.join("sub"),
            mkdir_call(&variant.join("sub"), false, false),
        ),
        (
            variant.join("a.txt"),
            rename_call(&c.artifacts.join("a.txt"), &variant.join("a.txt")),
        ),
    ] {
        let what = format!("{call:?}");
        assert_eq!(f.handle(call).unwrap_err(), refused(&p), "{what}");
    }
    assert_eq!(snapshot(&c.t.0), before);
    f.handle(write_call(&c.artifacts.join("ok.txt"), "ok"))
        .unwrap();
    let mut f = folding(false);
    let hooks = c.project.join(".REFACT/hooks.yaml");
    let err = f.handle(write_call(&hooks, "x")).unwrap_err();
    assert_eq!(err.0, "PermissionError");
    assert!(err.1.contains("engine configuration or state"), "{}", err.1);
    assert!(!hooks.exists());
    f.handle(write_call(&c.artifacts.join("ok.txt"), "again"))
        .unwrap();
}

#[test]
fn windows_path_forms_map_to_host_paths_and_stay_outside_artifacts() {
    for (virtual_form, host) in [
        ("/C:/ws/artifacts/../x", r"C:\ws\artifacts\..\x"),
        ("C:/ws/x", r"C:\ws\x"),
        ("//?/C:/ws/x", r"\\?\C:\ws\x"),
        ("//./pipe/x", r"\\.\pipe\x"),
        ("//server/share/x", r"\\server\share\x"),
        ("C:x", "C:x"),
        ("/C:/ws/x:stream", r"C:\ws\x:stream"),
    ] {
        assert_eq!(
            host_path(virtual_form, Platform::Windows),
            PathBuf::from(host),
            "{virtual_form}"
        );
    }
    let c = Chat::new();
    let mut f = c.fs(true);
    let before = snapshot(&c.t.0);
    for spelled in [
        r"C:\ws\artifacts\x.txt",
        "C:/ws/artifacts/x.txt",
        r"\\?\C:\x.txt",
        "//./C:/x.txt",
        "artifacts:stream",
    ] {
        let err = f
            .handle(OsFunctionCall::WriteText(PathStringDataArgs {
                path: spelled.into(),
                data: "x".into(),
            }))
            .unwrap_err();
        assert_eq!(err, refused(&f.resolve(spelled)), "{spelled}");
    }
    assert_eq!(snapshot(&c.t.0), before);
    #[cfg(windows)]
    {
        let plain = c
            .artifacts
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string();
        for (i, spelled) in [
            format!(r"{plain}\a1.txt"),
            format!("{}/a2.txt", plain.replace('\\', "/")),
            virtual_path(&PathBuf::from(&plain).join("a3.txt"), Platform::Windows),
            format!(r"\\?\{plain}\a4.txt"),
            format!(r"{}\a5.txt", plain.to_uppercase()),
        ]
        .iter()
        .enumerate()
        {
            f.handle(OsFunctionCall::WriteText(PathStringDataArgs {
                path: spelled.as_str().into(),
                data: "ok".into(),
            }))
            .unwrap_or_else(|e| panic!("{spelled}: {e:?}"));
            assert_eq!(
                std::fs::read_to_string(c.artifacts.join(format!("a{}.txt", i + 1))).unwrap(),
                "ok",
                "{spelled}"
            );
        }
        let stream = format!("{plain}:stream");
        assert_eq!(
            f.handle(OsFunctionCall::WriteText(PathStringDataArgs {
                path: stream.as_str().into(),
                data: "x".into(),
            }))
            .unwrap_err(),
            refused(&f.resolve(&stream))
        );
    }
}

#[test]
fn writable_chat_keeps_workspace_and_run_dir_unwritable() {
    let c = Chat::new();
    let run_json = c.run.join("run.json");
    std::fs::write(&run_json, "trusted").unwrap();
    std::fs::write(c.workspace.join("state.json"), "trusted").unwrap();
    std::fs::write(c.artifacts.join("a.txt"), "a").unwrap();
    std::fs::hard_link(&run_json, c.artifacts.join("run-alias.json")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        c.workspace.join("state.json"),
        c.artifacts.join("state-link"),
    )
    .unwrap();
    let mut f = c.fs(false);
    let alias = c.artifacts.join("run-alias.json");
    for call in [
        write_call(&c.workspace.join("state.json"), "x"),
        write_call(&run_json, "x"),
        write_call(&c.run.join("new.json"), "x"),
        write_call(&alias, "x"),
        append_call(&alias, "x"),
        open_call(&alias, "w"),
        open_call(&alias, "a"),
        #[cfg(unix)]
        write_call(&c.artifacts.join("state-link"), "x"),
        rename_call(&c.artifacts.join("a.txt"), &c.run.join("moved.txt")),
        rename_call(&c.artifacts.join("a.txt"), &run_json),
        OsFunctionCall::Unlink(text_of(&run_json).as_str().into()),
    ] {
        let what = format!("{call:?}");
        assert_eq!(f.handle(call).unwrap_err().0, "PermissionError", "{what}");
    }
    assert_eq!(std::fs::read_to_string(&run_json).unwrap(), "trusted");
    assert_eq!(
        std::fs::read_to_string(c.workspace.join("state.json")).unwrap(),
        "trusted"
    );
    assert!(!c.run.join("new.json").exists());
    f.handle(write_call(&c.project.join("free.txt"), "x"))
        .unwrap();
    f.handle(write_call(&c.artifacts.join("a.txt"), "b"))
        .unwrap();
}

#[cfg(unix)]
#[test]
fn contained_writes_never_follow_a_link_swapped_in_after_the_check() {
    let c = Chat::new();
    let run_json = c.run.join("run.json");
    std::fs::write(&run_json, "trusted").unwrap();
    let artifact = c.artifacts.join("a.txt");
    for read_only in [false, true] {
        let mut config = c.config(read_only);
        config.write_hook = Some(|p: &std::path::Path| {
            let workspace = p.parent().unwrap().parent().unwrap();
            std::fs::remove_file(p).unwrap();
            std::os::unix::fs::symlink(workspace.join("runs/r1/run.json"), p).unwrap();
        });
        let mut f = HostFs::new(config);
        for call in [
            write_call(&artifact, "x"),
            append_call(&artifact, "x"),
            open_call(&artifact, "w"),
            open_call(&artifact, "a"),
        ] {
            let _ = std::fs::remove_file(&artifact);
            std::fs::write(&artifact, "a").unwrap();
            let what = format!("{read_only} {call:?}");
            let err = f.handle(call).unwrap_err();
            assert_eq!(err.0, "PermissionError", "{what}");
            assert!(err.1.contains("is a symlink"), "{what}: {}", err.1);
            assert_eq!(std::fs::read_to_string(&run_json).unwrap(), "trusted");
        }
    }
}

#[cfg(unix)]
#[test]
fn writable_entry_operations_keep_symlinked_protected_paths_and_their_targets() {
    use std::os::unix::fs::symlink;
    let c = Chat::new();
    let real_refact = c.outside.join("refact");
    let dotfiles = c.outside.join("dotfiles");
    let real_settings = c.outside.join("settings.yaml");
    std::fs::create_dir_all(&real_refact).unwrap();
    std::fs::create_dir_all(dotfiles.join("tool")).unwrap();
    std::fs::write(real_refact.join("hooks.yaml"), "trusted").unwrap();
    std::fs::write(dotfiles.join("tool/config.yaml"), "trusted").unwrap();
    std::fs::write(&real_settings, "trusted").unwrap();
    symlink(&real_refact, c.project.join(".refact")).unwrap();
    symlink(&dotfiles, c.project.join("cfg")).unwrap();
    symlink(&real_settings, c.project.join("settings.yaml")).unwrap();
    symlink(&real_refact, c.artifacts.join("to-refact")).unwrap();
    std::fs::write(c.project.join("free.txt"), "free").unwrap();
    let mut config = c.config(false);
    config.deny_write.prefixes.push(c.project.join("cfg/tool"));
    let mut f = HostFs::new(config);
    let before = snapshot(&c.t.0);
    for protected in [
        c.project.join(".refact"),
        c.project.join("cfg"),
        c.project.join("settings.yaml"),
    ] {
        for call in [
            OsFunctionCall::Unlink(text_of(&protected).as_str().into()),
            rename_call(&protected, &c.project.join("moved")),
            rename_call(&c.project.join("free.txt"), &protected),
        ] {
            let what = format!("{call:?}");
            let err = f.handle(call).unwrap_err();
            assert_eq!(err.0, "PermissionError", "{what}");
            assert!(
                err.1.contains("engine configuration or state"),
                "{what}: {}",
                err.1
            );
        }
    }
    for real in [
        real_refact.join("hooks.yaml"),
        dotfiles.join("tool/config.yaml"),
        real_settings.clone(),
    ] {
        assert_eq!(
            f.handle(write_call(&real, "x")).unwrap_err().0,
            "PermissionError",
            "{}",
            real.display()
        );
    }
    assert_eq!(snapshot(&c.t.0), before);
    assert!(f.take_events().is_empty());
    let link = c.artifacts.join("to-refact");
    f.handle(rename_call(&link, &c.artifacts.join("moved-link")))
        .unwrap();
    f.handle(OsFunctionCall::Unlink(
        text_of(&c.artifacts.join("moved-link")).as_str().into(),
    ))
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(real_refact.join("hooks.yaml")).unwrap(),
        "trusted"
    );
}

#[test]
fn deny_rules_without_read_only_still_parse() {
    let rules: PathDenyRules =
        serde_json::from_str(r#"{"prefixes":[],"files":[],"allow_prefixes":[]}"#).unwrap();
    assert!(rules.read_only.is_none());
    let command: KernelCommand = serde_json::from_str(
        r#"{"Configure":{"project_root":"/p","env":{},"capture_dir":null,"deny_write":{"prefixes":[],"files":[]}}}"#,
    )
    .unwrap();
    assert!(matches!(
        command,
        KernelCommand::Configure {
            deny_write: PathDenyRules {
                read_only: None,
                ..
            },
            ..
        }
    ));
}

#[tokio::test]
async fn read_only_scripts_follow_chdir_and_refuse_writes_of_imported_code() {
    use refact_python_kernel::client::{ClientEvent, ClientOptions, KernelClient};
    let c = Chat::new();
    std::fs::write(
        c.project.join("writer.py"),
        "open('module-wrote.txt', 'w').write('x')\n",
    )
    .unwrap();
    let mut client =
        KernelClient::spawn_in_thread(Box::new(c.fs(true)), ClientOptions::default()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), client.next_event())
        .await
        .unwrap();
    let globals = [("ARTIFACTS", &c.artifacts), ("PROJECT", &c.project)]
        .into_iter()
        .map(|(name, p)| GlobalBinding {
            name: name.into(),
            kind: GlobalKind::Json(serde_json::json!(text_of(p))),
        })
        .collect();
    client
        .send(KernelCommand::Feed {
            run_id: "read-only".into(),
            code: "import os\nimport io\nos.chdir(ARTIFACTS)\nopen('out.txt', 'w').write('ok')\nout = []\nfor attempt in ['../escape.txt']:\n    try:\n        open(attempt, 'w')\n        out.append('wrote')\n    except PermissionError as e:\n        out.append(str(e))\nos.chdir(PROJECT)\ntry:\n    io.open('io.txt', 'w')\n    out.append('wrote')\nexcept PermissionError as e:\n    out.append(str(e))\ntry:\n    import writer\n    out.append('imported')\nexcept PermissionError as e:\n    out.append(str(e))\nout".into(),
            globals,
            limits: RunLimits::default(),
        })
        .await
        .unwrap();
    let value = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(15), client.next_event())
            .await
            .unwrap()
        {
            ClientEvent::Kernel(KernelEvent::Completed {
                value, ..
            }) => break value,
            ClientEvent::Kernel(KernelEvent::Raised {
                message, ..
            }) => panic!("{message}"),
            _ => {}
        }
    };
    let expected: Vec<String> = [
        c.artifacts.join("../escape.txt"),
        c.project.join("io.txt"),
        c.project.join("module-wrote.txt"),
    ]
    .iter()
    .map(|p| refused(p).1)
    .collect();
    assert_eq!(value, serde_json::json!(expected));
    assert_eq!(
        std::fs::read_to_string(c.artifacts.join("out.txt")).unwrap(),
        "ok"
    );
    assert!(!c.workspace.join("escape.txt").exists());
    assert!(!c.project.join("module-wrote.txt").exists());
    assert!(!c.project.join("io.txt").exists());
    client.send(KernelCommand::Shutdown).await.unwrap();
}

use refact_python_kernel::kernel::{HelperCall, HelperError};
use serde_json::{json, Value};

fn call_helper(
    f: &mut HostFs,
    name: &str,
    args: Value,
    kwargs: Value,
    list_files: &mut dyn FnMut(&FileListRequest) -> Result<FileList, HelperError>,
) -> Result<Value, (String, String)> {
    let result = f.helper(HelperCall {
        name,
        args: args.as_array().cloned().unwrap_or_default(),
        kwargs: kwargs.as_object().cloned().unwrap_or_default(),
        marks: vec![],
        max_result_bytes: 16 * 1024 * 1024,
        list_files,
    });
    match result {
        Ok(value) => Ok(value),
        Err(HelperError::Raise(kind, message)) => Err((kind, message)),
        Err(_) => panic!("{name}() was interrupted"),
    }
}
fn no_listing(_: &FileListRequest) -> Result<FileList, HelperError> {
    panic!("unexpected file listing")
}
fn helper(
    f: &mut HostFs,
    name: &str,
    args: Value,
    kwargs: Value,
) -> Result<Value, (String, String)> {
    call_helper(f, name, args, kwargs, &mut no_listing)
}
fn lists_itself(request: &FileListRequest) -> Result<FileList, HelperError> {
    Ok(FileList {
        files: vec![request.path.clone()],
        ..Default::default()
    })
}
fn grep(f: &mut HostFs, args: Value, kwargs: Value) -> Result<Value, (String, String)> {
    call_helper(f, "grep", args, kwargs, &mut lists_itself)
}
fn raised(kind: &str, message: impl Into<String>) -> (String, String) {
    (kind.into(), message.into())
}
fn fs_events(f: &mut HostFs) -> Vec<(String, PathBuf, u64)> {
    f.take_events()
        .into_iter()
        .filter_map(|e| match e {
            KernelEvent::Fs {
                op,
                path,
                bytes,
                ..
            } => Some((op, PathBuf::from(path), bytes)),
            _ => None,
        })
        .collect()
}
fn captures(events: &[KernelEvent]) -> Vec<(PathBuf, WriteOriginal)> {
    events
        .iter()
        .filter_map(|e| match e {
            KernelEvent::WriteCapture {
                path,
                original,
                ..
            } => Some((PathBuf::from(path), original.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn read_returns_numbered_lines_of_a_range() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "a\nb\nc\nd\n").unwrap();
    let many: String = (1..=10_000).map(|i| format!("{i}\n")).collect();
    std::fs::write(t.0.join("many.txt"), many).unwrap();
    let mut f = t.fs();
    for (args, kwargs, expected) in [
        (json!(["a.txt", 2, 3]), json!({}), "   2 | b\n   3 | c\n"),
        (
            json!(["a.txt"]),
            json!({"start": 3, "end": 99}),
            "   3 | c\n   4 | d\n",
        ),
        (
            json!(["a.txt"]),
            json!({}),
            "   1 | a\n   2 | b\n   3 | c\n   4 | d\n",
        ),
        (json!(["a.txt", 4, 4]), json!({}), "   4 | d\n"),
        (
            json!(["many.txt", 9998, 9999]),
            json!({}),
            "9998 | 9998\n9999 | 9999\n",
        ),
        (
            json!(["many.txt", 9999]),
            json!({}),
            " 9999 | 9999\n10000 | 10000\n",
        ),
    ] {
        assert_eq!(
            helper(&mut f, "read", args.clone(), kwargs).unwrap(),
            json!(expected),
            "{args}"
        );
    }
}

#[test]
fn read_negative_start_returns_the_last_lines() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "1\n2\n3\n4\n5\n").unwrap();
    let mut f = t.fs();
    assert_eq!(
        helper(&mut f, "read", json!(["a.txt", -2]), json!({})).unwrap(),
        json!("   4 | 4\n   5 | 5\n")
    );
    assert_eq!(
        helper(&mut f, "read", json!(["a.txt", -9]), json!({})).unwrap(),
        json!("   1 | 1\n   2 | 2\n   3 | 3\n   4 | 4\n   5 | 5\n")
    );
    assert_eq!(
        helper(
            &mut f,
            "read",
            json!(["a.txt", -2]),
            json!({"numbered": false})
        )
        .unwrap(),
        json!("4\n5\n")
    );
}

#[test]
fn read_unnumbered_is_the_exact_text_with_its_line_endings() {
    let t = Temp::new();
    let text = "\u{feff}first\r\nsecond\nthird\r\nlast";
    std::fs::write(t.0.join("a.txt"), text).unwrap();
    let mut f = t.fs();
    assert_eq!(
        helper(&mut f, "read", json!(["a.txt"]), json!({"numbered": false})).unwrap(),
        json!(text)
    );
    assert_eq!(
        helper(
            &mut f,
            "read",
            json!(["a.txt", 2, 3]),
            json!({"numbered": false})
        )
        .unwrap(),
        json!("second\nthird\r\n")
    );
    assert_eq!(
        helper(&mut f, "read", json!(["a.txt"]), json!({})).unwrap(),
        json!("   1 | \u{feff}first\n   2 | second\n   3 | third\n   4 | last\n")
    );
}

#[test]
fn read_range_errors_state_the_line_count() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "a\nb\nc\nd\n").unwrap();
    std::fs::write(t.0.join("b.txt"), "x").unwrap();
    let mut f = t.fs();
    for (args, message) in [
        (
            json!(["a.txt", 0]),
            "read(): start must be 1 or more, or negative to count from the end (-40 reads the last 40 lines); a.txt has 4 lines",
        ),
        (
            json!(["a.txt", 3, 2]),
            "read(): end (2) is before start (3); a.txt has 4 lines",
        ),
        (
            json!(["a.txt", -2, 3]),
            "read(): end cannot be combined with a negative start; a.txt has 4 lines",
        ),
        (
            json!(["a.txt", 5]),
            "read(): start 5 is past the end of a.txt (4 lines)",
        ),
        (
            json!(["b.txt", 2]),
            "read(): start 2 is past the end of b.txt (1 line)",
        ),
    ] {
        assert_eq!(
            helper(&mut f, "read", args.clone(), json!({})).unwrap_err(),
            raised("ValueError", message),
            "{args}"
        );
    }
}

#[test]
fn read_of_an_empty_file() {
    let t = Temp::new();
    std::fs::write(t.0.join("empty.txt"), "").unwrap();
    let mut f = t.fs();
    for (args, kwargs) in [
        (json!(["empty.txt"]), json!({})),
        (json!(["empty.txt", 1, 5]), json!({})),
        (json!(["empty.txt", -3]), json!({})),
        (json!(["empty.txt"]), json!({"numbered": false})),
    ] {
        assert_eq!(helper(&mut f, "read", args, kwargs).unwrap(), json!(""));
    }
    assert_eq!(
        helper(&mut f, "read", json!(["empty.txt", 2]), json!({})).unwrap_err(),
        raised(
            "ValueError",
            "read(): start 2 is past the end of empty.txt (0 lines)"
        )
    );
}

#[test]
fn read_refuses_directories_missing_files_non_utf8_text_and_denied_paths() {
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("sub")).unwrap();
    std::fs::create_dir_all(t.0.join("config")).unwrap();
    std::fs::write(t.0.join("bin.dat"), b"ok\nab\xffcd\n").unwrap();
    std::fs::write(t.0.join("config/secret.yaml"), "key: x\n").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_read: PathDenyRules {
            prefixes: vec![t.0.join("config")],
            ..Default::default()
        },
        ..Default::default()
    });
    let mut cases = vec![
        ("sub", raised("IsADirectoryError", "read(): sub is a directory")),
        (
            "missing.txt",
            raised("FileNotFoundError", "read(): no such file: missing.txt"),
        ),
        (
            "bin.dat",
            raised(
                "UnicodeDecodeError",
                "read(): bin.dat is not UTF-8 text (invalid byte at offset 5); use open(path, \"rb\")",
            ),
        ),
        (
            "config/secret.yaml",
            raised(
                "PermissionError",
                format!(
                    "read(): reading {} is not allowed (engine configuration)",
                    t.0.join("config/secret.yaml").display()
                ),
            ),
        ),
    ];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(t.0.join("config"), t.0.join("alias")).unwrap();
        cases.push((
            "alias/secret.yaml",
            raised(
                "PermissionError",
                format!(
                    "read(): reading {} is not allowed (engine configuration)",
                    t.0.join("alias/secret.yaml").display()
                ),
            ),
        ));
    }
    for (path, expected) in cases {
        assert_eq!(
            helper(&mut f, "read", json!([path]), json!({})).unwrap_err(),
            expected,
            "{path}"
        );
    }
    assert!(fs_events(&mut f)
        .iter()
        .all(|(_, p, _)| p == &t.0.join("bin.dat")));
}

#[test]
fn read_records_one_read_event_like_open() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "a\nb\n").unwrap();
    let mut f = t.fs();
    helper(&mut f, "read", json!(["a.txt"]), json!({})).unwrap();
    assert_eq!(
        fs_events(&mut f),
        vec![("read".into(), t.0.join("a.txt"), 4)]
    );
}

#[test]
fn read_refuses_a_result_over_the_limit_before_building_it() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "x".repeat(4096)).unwrap();
    let mut f = t.fs();
    let err = f
        .helper(HelperCall {
            name: "read",
            args: vec![json!("a.txt")],
            kwargs: Default::default(),
            marks: vec![],
            max_result_bytes: 1024,
            list_files: &mut no_listing,
        })
        .err()
        .unwrap();
    let HelperError::Raise(kind, message) = err else {
        panic!("interrupted")
    };
    assert_eq!(kind, "RuntimeError");
    assert!(
        message.starts_with("read() returned more than 1024 bytes"),
        "{message}"
    );
}

#[test]
fn helper_arguments_bind_like_python() {
    let t = Temp::new();
    let mut f = t.fs();
    for (name, args, kwargs, message) in [
        (
            "read",
            json!([]),
            json!({}),
            "read() missing required argument: 'path'",
        ),
        (
            "read",
            json!(["a"]),
            json!({"lines": 1}),
            "read() got an unexpected keyword argument 'lines'",
        ),
        (
            "read",
            json!(["a", 1, 2, 3]),
            json!({}),
            "read() takes at most 3 positional arguments (4 given)",
        ),
        (
            "read",
            json!(["a", 1]),
            json!({"start": 2}),
            "read() got multiple values for argument 'start'",
        ),
        (
            "read",
            json!(["a", "1"]),
            json!({}),
            "read() argument start must be int, not str",
        ),
        (
            "read",
            json!([1]),
            json!({}),
            "read() argument path must be str or Path, not int",
        ),
        (
            "read",
            json!(["a"]),
            json!({"end": "x"}),
            "read() argument end must be int or None, not str",
        ),
        (
            "read",
            json!(["a"]),
            json!({"numbered": 1}),
            "read() argument numbered must be bool, not int",
        ),
        (
            "read",
            json!(["a", 1.5]),
            json!({}),
            "read() argument start must be int, not float",
        ),
        (
            "grep",
            json!([]),
            json!({}),
            "grep() missing required argument: 'pattern'",
        ),
        (
            "grep",
            json!(["x"]),
            json!({"regex": 1}),
            "grep() got an unexpected keyword argument 'regex'",
        ),
        (
            "grep",
            json!(["x", ".", 2]),
            json!({}),
            "grep() takes at most 2 positional arguments (3 given)",
        ),
        (
            "grep",
            json!(["x"]),
            json!({"glob": 1}),
            "grep() argument glob must be str or None, not int",
        ),
        (
            "grep",
            json!([null]),
            json!({}),
            "grep() argument pattern must be str, not NoneType",
        ),
        (
            "replace",
            json!(["a", "b"]),
            json!({}),
            "replace() missing required argument: 'new'",
        ),
        (
            "replace",
            json!(["a", "b", "c"]),
            json!({"count": "1"}),
            "replace() argument count must be int, not str",
        ),
        (
            "replace",
            json!(["a", ["b"], "c"]),
            json!({}),
            "replace() argument old must be str, not list",
        ),
    ] {
        assert_eq!(
            helper(&mut f, name, args.clone(), kwargs).unwrap_err(),
            raised("TypeError", message),
            "{name}{args}"
        );
    }
    assert_eq!(
        helper(&mut f, "glob", json!([]), json!({})).unwrap_err(),
        raised("NameError", "name 'glob' is not available in this session")
    );
    assert!(f.take_events().is_empty());
}

const LINES: &str = "plain one\nmatch two\nmatch three\nplain four\nplain five\nplain six\nplain seven\nmatch eight\nplain nine\nplain ten\n";

#[test]
fn grep_one_file_prints_matches_and_context_groups() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), LINES).unwrap();
    let mut f = t.fs();
    assert_eq!(
        grep(&mut f, json!(["match", "a.txt"]), json!({})).unwrap(),
        json!("a.txt:2: match two\na.txt:3: match three\na.txt:8: match eight")
    );
    assert_eq!(
        grep(&mut f, json!(["match", "a.txt"]), json!({"context": 1})).unwrap(),
        json!("a.txt-1- plain one\na.txt:2: match two\na.txt:3: match three\na.txt-4- plain four\n--\na.txt-7- plain seven\na.txt:8: match eight\na.txt-9- plain nine")
    );
    let wide = grep(&mut f, json!(["match", "a.txt"]), json!({"context": 2}))
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert!(!wide.contains("--"), "{wide}");
    assert_eq!(wide.lines().count(), 10);
    assert_eq!(
        grep(&mut f, json!(["absent", "a.txt"]), json!({})).unwrap(),
        json!("no matches (searched 1 file)")
    );
}

#[test]
fn grep_counts_shown_and_hidden_matches() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "x1\nx2\nx3\nx4\nx5\n").unwrap();
    let many: String = (0..1236).map(|i| format!("y{i}\n")).collect();
    std::fs::write(t.0.join("many.txt"), many).unwrap();
    let mut f = t.fs();
    assert_eq!(
        grep(&mut f, json!(["x", "a.txt"]), json!({"limit": 2})).unwrap(),
        json!("a.txt:1: x1\na.txt:2: x2\n… 3 more matches not shown (limit 2); narrow the pattern or raise limit")
    );
    assert_eq!(
        grep(
            &mut f,
            json!(["x", "a.txt"]),
            json!({"limit": 2, "data": true})
        )
        .unwrap(),
        json!({
            "matches": [
                {"path": "a.txt", "line": 1, "text": "x1"},
                {"path": "a.txt", "line": 2, "text": "x2"}
            ],
            "total": 5,
            "truncated": true,
            "files_searched": 1,
            "files_skipped": 0
        })
    );
    let all = grep(&mut f, json!(["x", "a.txt"]), json!({"data": true})).unwrap();
    assert_eq!(all["total"], json!(5));
    assert_eq!(all["truncated"], json!(false));
    assert_eq!(all["matches"].as_array().unwrap().len(), 5);
    let text = grep(&mut f, json!(["y", "many.txt"]), json!({"limit": 2})).unwrap();
    assert!(
        text.as_str().unwrap().ends_with(
            "\n… 1,234 more matches not shown (limit 2); narrow the pattern or raise limit"
        ),
        "{text}"
    );
}

#[test]
fn grep_uses_rust_regex_and_checks_its_arguments() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "Alpha\nbeta\n").unwrap();
    std::fs::create_dir(t.0.join("d")).unwrap();
    let mut f = t.fs();
    assert_eq!(
        grep(&mut f, json!(["(", "a.txt"]), json!({})).unwrap_err(),
        raised("ValueError", "grep(): invalid pattern: unclosed group")
    );
    let (kind, message) = grep(&mut f, json!(["a(?=l)", "a.txt"]), json!({})).unwrap_err();
    assert_eq!(kind, "ValueError");
    assert!(
        message.starts_with("grep(): invalid pattern: look-around"),
        "{message}"
    );
    assert_eq!(
        grep(
            &mut f,
            json!(["ALPHA", "a.txt"]),
            json!({"ignore_case": true})
        )
        .unwrap(),
        json!("a.txt:1: Alpha")
    );
    assert_eq!(
        grep(&mut f, json!(["ALPHA", "a.txt"]), json!({})).unwrap(),
        json!("no matches (searched 1 file)")
    );
    for (kwargs, message) in [
        (
            json!({"context": 21}),
            "grep(): context must be between 0 and 20",
        ),
        (
            json!({"context": -1}),
            "grep(): context must be between 0 and 20",
        ),
        (
            json!({"limit": 0}),
            "grep(): limit must be between 1 and 10000",
        ),
        (
            json!({"limit": 10001}),
            "grep(): limit must be between 1 and 10000",
        ),
        (
            json!({"glob": "*.rs"}),
            "grep(): glob filters directory searches; path is a file",
        ),
    ] {
        assert_eq!(
            grep(&mut f, json!(["a", "a.txt"]), kwargs.clone()).unwrap_err(),
            raised("ValueError", message),
            "{kwargs}"
        );
    }
    let (kind, message) = grep(&mut f, json!(["a", "d"]), json!({"glob": "["})).unwrap_err();
    assert_eq!(kind, "ValueError");
    assert!(message.starts_with("grep(): invalid glob: "), "{message}");
    assert_eq!(
        grep(&mut f, json!(["a", "missing"]), json!({})).unwrap_err(),
        raised(
            "FileNotFoundError",
            "grep(): no such file or directory: missing"
        )
    );
}

#[test]
fn grep_directory_asks_once_and_guards_every_listed_file() {
    let t = Temp::new();
    for dir in ["src/deep", "src/private", "config", "other"] {
        std::fs::create_dir_all(t.0.join(dir)).unwrap();
    }
    std::fs::write(t.0.join("src/a.rs"), "fn needle() {}\n").unwrap();
    std::fs::write(t.0.join("src/b.txt"), "needle\n").unwrap();
    std::fs::write(t.0.join("src/deep/c.rs"), "no\nneedle here\n").unwrap();
    std::fs::write(t.0.join("src/none.rs"), "nothing\n").unwrap();
    std::fs::write(t.0.join("src/private/key.rs"), "needle\n").unwrap();
    std::fs::write(t.0.join("config/secret.rs"), "needle\n").unwrap();
    std::fs::write(t.0.join("other/outside.rs"), "needle\n").unwrap();
    let mut listed = vec![
        t.0.join("src/deep/c.rs"),
        t.0.join("src/b.txt"),
        t.0.join("other/outside.rs"),
        t.0.join("src/private/key.rs"),
        t.0.join("src/none.rs"),
        t.0.join("src/a.rs"),
    ];
    let mut protected = 1;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(t.0.join("config/secret.rs"), t.0.join("src/link.rs")).unwrap();
        listed.push(t.0.join("src/link.rs"));
        protected += 1;
    }
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_read: PathDenyRules {
            prefixes: vec![t.0.join("config"), t.0.join("src/private")],
            ..Default::default()
        },
        ..Default::default()
    });
    let mut requests = vec![];
    let mut list = |request: &FileListRequest| {
        requests.push(request.clone());
        Ok(FileList {
            files: listed.clone(),
            blocked: 1,
            ..Default::default()
        })
    };
    let text = call_helper(
        &mut f,
        "grep",
        json!(["needle", "src"]),
        json!({}),
        &mut list,
    )
    .unwrap();
    assert_eq!(
        text,
        json!(format!("src/a.rs:1: fn needle() {{}}\nsrc/b.txt:1: needle\nsrc/deep/c.rs:2: needle here\nskipped: {protected} protected files, 1 privacy-blocked file"))
    );
    assert_eq!(
        requests,
        vec![FileListRequest {
            path: t.0.join("src"),
            max_entries: 20_000,
            glob: None,
        }]
    );
    let recorded: Vec<PathBuf> = fs_events(&mut f)
        .into_iter()
        .map(|(op, p, _)| {
            assert_eq!(op, "grep");
            p
        })
        .collect();
    assert_eq!(
        recorded,
        vec![
            t.0.join("src/a.rs"),
            t.0.join("src/b.txt"),
            t.0.join("src/deep/c.rs")
        ]
    );
    for (glob, expected) in [
        (
            "*.rs",
            "src/a.rs:1: fn needle() {}\nsrc/deep/c.rs:2: needle here",
        ),
        ("deep/*.rs", "src/deep/c.rs:2: needle here"),
        (
            "**/*.rs",
            "src/a.rs:1: fn needle() {}\nsrc/deep/c.rs:2: needle here",
        ),
    ] {
        let mut list = |_: &FileListRequest| {
            Ok(FileList {
                files: vec![
                    t.0.join("src/a.rs"),
                    t.0.join("src/b.txt"),
                    t.0.join("src/deep/c.rs"),
                ],
                ..Default::default()
            })
        };
        assert_eq!(
            call_helper(
                &mut f,
                "grep",
                json!(["needle", "src"]),
                json!({"glob": glob}),
                &mut list
            )
            .unwrap(),
            json!(expected),
            "{glob}"
        );
    }
}

#[test]
fn grep_sends_its_glob_and_counts_the_files_it_could_not_read() {
    let t = Temp::new();
    std::fs::create_dir(t.0.join("src")).unwrap();
    std::fs::write(t.0.join("src/a.rs"), "needle\n").unwrap();
    let locked = t.0.join("src/locked.rs");
    std::fs::write(&locked, "locked\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    let unreadable = 1 + usize::from(std::fs::File::open(&locked).is_err());
    let listed = vec![t.0.join("src/a.rs"), t.0.join("src/gone.rs"), locked];
    let mut f = t.fs();
    let mut requests = vec![];
    let mut list = |request: &FileListRequest| {
        requests.push(serde_json::to_value(request).unwrap());
        Ok(serde_json::from_value(json!({"files": listed, "unlisted": 2})).unwrap())
    };
    let text = call_helper(
        &mut f,
        "grep",
        json!(["needle", "src"]),
        json!({"glob": "*.rs"}),
        &mut list,
    )
    .unwrap();
    let files = |n: usize| if n == 1 { "file" } else { "files" };
    assert_eq!(
        text,
        json!(format!(
            "src/a.rs:1: needle\nskipped: {unreadable} unreadable {}, 2 files whose names are not UTF-8",
            files(unreadable)
        ))
    );
    let data = call_helper(
        &mut f,
        "grep",
        json!(["needle", "src"]),
        json!({"glob": "*.rs", "data": true}),
        &mut list,
    )
    .unwrap();
    assert_eq!(data["files_skipped"], json!(unreadable + 2));
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert_eq!(
            request,
            json!({"path": t.0.join("src"), "max_entries": 20_000, "glob": "*.rs"})
        );
    }
}

#[test]
fn file_list_messages_without_glob_or_unlisted_still_parse() {
    let request: FileListRequest =
        serde_json::from_value(json!({"path": "/p", "max_entries": 5})).unwrap();
    assert_eq!(request.glob, None);
    let list: FileList = serde_json::from_value(json!({"files": ["/p/a"]})).unwrap();
    assert_eq!((list.unlisted, list.blocked, list.truncated), (0, 0, false));
}

#[cfg(unix)]
#[tokio::test]
async fn grep_under_a_root_that_is_not_utf8_raises_instead_of_panicking() {
    use refact_python_kernel::client::ClientEvent;
    use std::os::unix::ffi::OsStrExt;
    let t = Temp::new();
    let root = t.0.join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.txt"), "needle\n").unwrap();
    let mut client = helper_client(Box::new(HostFs::new(HostFsConfig {
        project_root: root,
        ..Default::default()
    })))
    .await;
    feed_helpers(
        &client,
        "root",
        "try:\n    grep('needle')\n    r = 'searched'\nexcept OSError as e:\n    r = str(e)\nr",
    )
    .await;
    loop {
        match next_event(&mut client).await {
            ClientEvent::Kernel(KernelEvent::Completed {
                value, ..
            }) => {
                let text = value.as_str().unwrap();
                assert!(text.starts_with("grep(): "), "{text}");
                assert!(text.contains("not UTF-8"), "{text}");
                break;
            }
            ClientEvent::Kernel(KernelEvent::Fs {
                ..
            }) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    client.send(KernelCommand::Shutdown).await.unwrap();
}

#[test]
fn grep_records_a_named_file_even_without_matches() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "abc\n").unwrap();
    let mut f = t.fs();
    grep(&mut f, json!(["zzz", "a.txt"]), json!({})).unwrap();
    assert_eq!(
        fs_events(&mut f),
        vec![("grep".into(), t.0.join("a.txt"), 4)]
    );
}

#[test]
fn grep_never_scans_a_privacy_blocked_file() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "needle\n").unwrap();
    let mut f = t.fs();
    let mut blocked = |_: &FileListRequest| {
        Ok(FileList {
            files: vec![],
            blocked: 1,
            ..Default::default()
        })
    };
    assert_eq!(
        call_helper(
            &mut f,
            "grep",
            json!(["needle", "a.txt"]),
            json!({}),
            &mut blocked
        )
        .unwrap(),
        json!("no matches (searched 0 files)\nskipped: 1 privacy-blocked file")
    );
    let data = call_helper(
        &mut f,
        "grep",
        json!(["needle", "a.txt"]),
        json!({"data": true}),
        &mut blocked,
    )
    .unwrap();
    assert_eq!(data["files_skipped"], json!(1));
    assert_eq!(data["files_searched"], json!(0));
    assert!(f.take_events().is_empty());
}

#[test]
fn grep_skips_large_binary_and_non_utf8_files_and_counts_them() {
    let t = Temp::new();
    std::fs::create_dir(t.0.join("d")).unwrap();
    std::fs::write(t.0.join("d/big.txt"), "needle".repeat(700_000)).unwrap();
    std::fs::write(t.0.join("d/nul.bin"), "needle\0\n").unwrap();
    std::fs::write(t.0.join("d/latin1.txt"), b"needle \xe9\n").unwrap();
    std::fs::write(t.0.join("d/ok.txt"), "needle\n").unwrap();
    let mut f = t.fs();
    let mut list = |_: &FileListRequest| {
        Ok(FileList {
            files: ["big.txt", "nul.bin", "latin1.txt", "ok.txt"]
                .iter()
                .map(|n| t.0.join("d").join(n))
                .collect(),
            ..Default::default()
        })
    };
    assert_eq!(
        call_helper(&mut f, "grep", json!(["needle", "d"]), json!({}), &mut list).unwrap(),
        json!("d/ok.txt:1: needle\nskipped: 1 file over 4 MiB, 2 binary or non-UTF-8 files")
    );
    let data = call_helper(
        &mut f,
        "grep",
        json!(["needle", "d"]),
        json!({"data": true}),
        &mut list,
    )
    .unwrap();
    assert_eq!(
        (
            &data["files_searched"],
            &data["files_skipped"],
            &data["truncated"]
        ),
        (&json!(1), &json!(3), &json!(false))
    );
}

#[test]
fn grep_says_why_it_stopped_early() {
    let t = Temp::new();
    std::fs::create_dir(t.0.join("d")).unwrap();
    std::fs::write(t.0.join("d/a.txt"), "needle\n").unwrap();
    std::fs::write(t.0.join("d/b.txt"), "needle\n").unwrap();
    let mut f = t.fs();
    let mut cut = |_: &FileListRequest| {
        Ok(FileList {
            files: vec![t.0.join("d/a.txt")],
            truncated: true,
            ..Default::default()
        })
    };
    assert_eq!(
        call_helper(&mut f, "grep", json!(["needle", "d"]), json!({}), &mut cut).unwrap(),
        json!("d/a.txt:1: needle\nstopped early: the file listing of d stopped at 20,000 entries; counts cover only the files searched; narrow path or glob")
    );
    assert_eq!(
        call_helper(
            &mut f,
            "grep",
            json!(["needle", "d"]),
            json!({"data": true}),
            &mut cut
        )
        .unwrap()["truncated"],
        json!(true)
    );
    let mut small = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        max_scan_bytes: 10,
        ..Default::default()
    });
    let mut both = |_: &FileListRequest| {
        Ok(FileList {
            files: vec![t.0.join("d/a.txt"), t.0.join("d/b.txt")],
            ..Default::default()
        })
    };
    assert_eq!(
        call_helper(&mut small, "grep", json!(["needle", "d"]), json!({}), &mut both).unwrap(),
        json!("d/a.txt:1: needle\nstopped early: the files to search hold more than 10 bytes; counts cover only the files searched; narrow path or glob")
    );
    let data = call_helper(
        &mut small,
        "grep",
        json!(["needle", "d"]),
        json!({"data": true}),
        &mut both,
    )
    .unwrap();
    assert_eq!(
        (&data["total"], &data["truncated"]),
        (&json!(1), &json!(true))
    );
}

#[test]
fn grep_matches_lines_without_their_endings_and_after_a_bom() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "\u{feff}start\r\nmiddle end\r\nlast").unwrap();
    let mut f = t.fs();
    for (pattern, expected) in [
        ("^start$", "a.txt:1: start"),
        ("end$", "a.txt:2: middle end"),
        ("^last$", "a.txt:3: last"),
    ] {
        assert_eq!(
            grep(&mut f, json!([pattern, "a.txt"]), json!({})).unwrap(),
            json!(expected),
            "{pattern}"
        );
    }
}

#[test]
fn grep_cuts_long_lines_and_long_output() {
    let t = Temp::new();
    std::fs::write(t.0.join("long.txt"), format!("{}needle\n", "x".repeat(500))).unwrap();
    let many: String = (0..600)
        .map(|i| format!("needle {i:03} {}\n", "y".repeat(380)))
        .collect();
    std::fs::write(t.0.join("many.txt"), many).unwrap();
    let mut f = t.fs();
    assert_eq!(
        grep(&mut f, json!(["needle", "long.txt"]), json!({})).unwrap(),
        json!(format!("long.txt:1: {}…", "x".repeat(400)))
    );
    let text = grep(
        &mut f,
        json!(["needle", "many.txt"]),
        json!({"limit": 10000}),
    )
    .unwrap()
    .as_str()
    .unwrap()
    .to_string();
    let (body, notice) = text.rsplit_once('\n').unwrap();
    assert_eq!(
        notice,
        "… output cut at 200,000 characters; narrow the search"
    );
    assert!(body.chars().count() <= 200_000);
    assert!(body.lines().all(|l| l.starts_with("many.txt:")));
}

#[test]
fn replace_writes_one_occurrence_and_returns_a_unified_diff() {
    let t = Temp::new();
    let captures_dir = Temp::new();
    std::fs::write(t.0.join("a.txt"), "a\nb\nc\nd\ne\n").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        capture_dir: Some(captures_dir.0.clone()),
        ..Default::default()
    });
    assert_eq!(
        helper(&mut f, "replace", json!(["a.txt", "c\n", "C\n"]), json!({})).unwrap(),
        json!("--- a/a.txt\n+++ b/a.txt\n@@ -1,5 +1,5 @@\n a\n b\n-c\n+C\n d\n e\n")
    );
    assert_eq!(
        std::fs::read_to_string(t.0.join("a.txt")).unwrap(),
        "a\nb\nC\nd\ne\n"
    );
    let events = f.take_events();
    let ops: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            KernelEvent::Fs {
                op,
                bytes,
                ..
            } => Some((op.clone(), *bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(
        ops,
        vec![("replace".into(), 10), ("replace.write".into(), 10)]
    );
    let captured = captures(&events);
    assert_eq!(captured.len(), 1);
    assert!(
        matches!(&captured[0].1, WriteOriginal::File { capture_path } if std::fs::read_to_string(capture_path).unwrap() == "a\nb\nc\nd\ne\n")
    );
}

#[test]
fn replace_with_count_zero_replaces_every_occurrence() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "x1 x2 x3\n").unwrap();
    let mut f = t.fs();
    helper(
        &mut f,
        "replace",
        json!(["a.txt", "x", "y"]),
        json!({"count": 0}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(t.0.join("a.txt")).unwrap(),
        "y1 y2 y3\n"
    );
    std::fs::write(t.0.join("aaa.txt"), "aaa").unwrap();
    helper(&mut f, "replace", json!(["aaa.txt", "aa", "b"]), json!({})).unwrap();
    assert_eq!(std::fs::read_to_string(t.0.join("aaa.txt")).unwrap(), "ba");
}

#[test]
fn replace_names_the_real_count_and_leaves_a_mismatch_untouched() {
    let t = Temp::new();
    std::fs::write(t.0.join("two.txt"), "x x").unwrap();
    std::fs::write(t.0.join("three.txt"), "x x x").unwrap();
    std::fs::write(t.0.join("crlf.txt"), "a\r\nb\r\n").unwrap();
    std::fs::write(t.0.join("empty.txt"), "").unwrap();
    let captures_dir = Temp::new();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        capture_dir: Some(captures_dir.0.clone()),
        ..Default::default()
    });
    for (args, kwargs, message) in [
        (
            json!(["two.txt", "x", "y"]),
            json!({"count": 3}),
            "replace(): found 2 occurrences of old in two.txt, expected 3; the file is unchanged.",
        ),
        (
            json!(["three.txt", "x", "y"]),
            json!({}),
            "replace(): found 3 occurrences of old in three.txt, expected 1; the file is unchanged. Pass count=3 (or count=0) to replace all of them, or include more surrounding text to select one.",
        ),
        (
            json!(["three.txt", "x", "y"]),
            json!({"count": 2}),
            "replace(): found 3 occurrences of old in three.txt, expected 2; the file is unchanged. Pass count=3 (or count=0) to replace all of them, or include more surrounding text to select 2.",
        ),
        (
            json!(["crlf.txt", "a\nb", "z"]),
            json!({}),
            "replace(): found 0 occurrences of old in crlf.txt, expected 1; the file is unchanged. Check whitespace and line endings; read(path, numbered=False) shows the exact text. The file uses CRLF line endings and old uses LF.",
        ),
        (
            json!(["empty.txt", "x", "y"]),
            json!({"count": 0}),
            "replace(): found 0 occurrences of old in empty.txt, expected at least 1; the file is unchanged. Check whitespace and line endings; read(path, numbered=False) shows the exact text.",
        ),
    ] {
        assert_eq!(
            helper(&mut f, "replace", args.clone(), kwargs).unwrap_err(),
            raised("ValueError", message),
            "{args}"
        );
    }
    assert_eq!(std::fs::read(t.0.join("two.txt")).unwrap(), b"x x");
    assert_eq!(std::fs::read(t.0.join("three.txt")).unwrap(), b"x x x");
    assert_eq!(std::fs::read(t.0.join("crlf.txt")).unwrap(), b"a\r\nb\r\n");
    let events = f.take_events();
    assert!(captures(&events).is_empty());
    assert!(events
        .iter()
        .all(|e| matches!(e, KernelEvent::Fs { op, .. } if op == "replace")));
    assert_eq!(
        std::fs::read_dir(&captures_dir.0).map_or(0, |d| d.count()),
        0
    );
}

#[test]
fn replace_refuses_bad_arguments_before_reading() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "a").unwrap();
    let mut f = t.fs();
    for (args, kwargs, message) in [
        (
            json!(["a.txt", "", "x"]),
            json!({}),
            "replace(): old must not be empty",
        ),
        (
            json!(["a.txt", "a", "a"]),
            json!({}),
            "replace(): old and new are identical; nothing to change",
        ),
        (
            json!(["a.txt", "a", "b"]),
            json!({"count": -1}),
            "replace(): count must be 0 or more (0 replaces every occurrence)",
        ),
    ] {
        assert_eq!(
            helper(&mut f, "replace", args, kwargs).unwrap_err(),
            raised("ValueError", message)
        );
    }
    assert!(f.take_events().is_empty());
}

#[test]
fn replace_keeps_every_byte_it_does_not_replace() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "\u{feff}head\r\nmid\nlast").unwrap();
    std::fs::write(t.0.join("bin.txt"), b"ok \xff mid").unwrap();
    let mut f = t.fs();
    helper(&mut f, "replace", json!(["a.txt", "mid", "MID"]), json!({})).unwrap();
    assert_eq!(
        std::fs::read_to_string(t.0.join("a.txt")).unwrap(),
        "\u{feff}head\r\nMID\nlast"
    );
    f.take_events();
    assert_eq!(
        helper(&mut f, "replace", json!(["bin.txt", "mid", "MID"]), json!({})).unwrap_err(),
        raised(
            "UnicodeDecodeError",
            "replace(): bin.txt is not UTF-8 text (invalid byte at offset 3); the file is unchanged."
        )
    );
    assert_eq!(std::fs::read(t.0.join("bin.txt")).unwrap(), b"ok \xff mid");
    assert!(captures(&f.take_events()).is_empty());
}

#[test]
fn replace_limits_input_size_and_growth() {
    let t = Temp::new();
    std::fs::File::create(t.0.join("big.txt"))
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    std::fs::write(t.0.join("grow.txt"), "a".repeat(1024 * 1024)).unwrap();
    let mut f = t.fs();
    assert_eq!(
        helper(&mut f, "replace", json!(["big.txt", "a", "b"]), json!({})).unwrap_err(),
        raised(
            "ValueError",
            "replace(): big.txt is 33,554,433 bytes; replace() edits files up to 32 MiB"
        )
    );
    assert_eq!(
        helper(
            &mut f,
            "replace",
            json!(["grow.txt", "a", "b".repeat(40)]),
            json!({"count": 0})
        )
        .unwrap_err(),
        raised(
            "ValueError",
            "replace(): replacing 1,048,576 occurrences would make grow.txt 41,943,040 bytes; replace() writes files up to 32 MiB; the file is unchanged."
        )
    );
    assert_eq!(
        std::fs::read(t.0.join("grow.txt")).unwrap().len(),
        1024 * 1024
    );
}

#[cfg(unix)]
#[test]
fn replace_keeps_the_mode_and_a_symlink() {
    use std::os::unix::fs::PermissionsExt;
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "old\n").unwrap();
    std::fs::set_permissions(t.0.join("a.txt"), std::fs::Permissions::from_mode(0o640)).unwrap();
    std::fs::write(t.0.join("real.txt"), "old\n").unwrap();
    std::os::unix::fs::symlink(t.0.join("real.txt"), t.0.join("link.txt")).unwrap();
    let mut f = t.fs();
    helper(&mut f, "replace", json!(["a.txt", "old", "new"]), json!({})).unwrap();
    assert_eq!(
        std::fs::metadata(t.0.join("a.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o640
    );
    helper(
        &mut f,
        "replace",
        json!(["link.txt", "old", "new"]),
        json!({}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(t.0.join("real.txt")).unwrap(),
        "new\n"
    );
    assert_eq!(
        std::fs::read_link(t.0.join("link.txt")).unwrap(),
        t.0.join("real.txt")
    );
}

#[test]
fn replace_refuses_a_file_with_more_than_one_link() {
    let t = Temp::new();
    std::fs::write(t.0.join("h1.txt"), "old\n").unwrap();
    std::fs::hard_link(t.0.join("h1.txt"), t.0.join("h2.txt")).unwrap();
    let mut f = t.fs();
    assert_eq!(
        helper(&mut f, "replace", json!(["h1.txt", "old", "new"]), json!({})).unwrap_err(),
        raised(
            "PermissionError",
            "replace(): h1.txt has 2 links; replace() edits only files with one link; the file is unchanged."
        )
    );
    assert_eq!(
        std::fs::read_to_string(t.0.join("h2.txt")).unwrap(),
        "old\n"
    );
}

#[test]
fn replace_detects_a_change_made_while_it_ran_and_writes_nothing() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "old").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        replace_hook: Some(|p: &std::path::Path| {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .append(true)
                .open(p)
                .unwrap()
                .write_all(b" changed")
                .unwrap();
        }),
        ..Default::default()
    });
    assert_eq!(
        helper(&mut f, "replace", json!(["a.txt", "old", "new"]), json!({})).unwrap_err(),
        raised(
            "RuntimeError",
            "replace(): a.txt changed on disk while replace() ran; nothing was written. Read it again and retry."
        )
    );
    assert_eq!(
        std::fs::read_to_string(t.0.join("a.txt")).unwrap(),
        "old changed"
    );
    let names: Vec<_> = std::fs::read_dir(&t.0)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("a.txt")]);
    let events = f.take_events();
    assert!(captures(&events).is_empty());
    assert!(!events
        .iter()
        .any(|e| matches!(e, KernelEvent::Fs { op, .. } if op == "replace.write")));
}

fn replace_leftovers(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".replace"))
        .collect()
}

fn replace_wrote_nothing(f: &mut HostFs) {
    let events = f.take_events();
    assert!(captures(&events).is_empty(), "{events:?}");
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, KernelEvent::Fs { op, .. } if op == "replace.write")),
        "{events:?}"
    );
}

#[test]
fn replace_refuses_a_file_it_may_not_open_for_writing() {
    let t = Temp::new();
    let p = t.0.join("ro.txt");
    std::fs::write(&p, "old").unwrap();
    let mut permissions = std::fs::metadata(&p).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&p, permissions.clone()).unwrap();
    let writable = std::fs::OpenOptions::new().write(true).open(&p).is_ok();
    let mut f = t.fs();
    if writable {
        helper(
            &mut f,
            "replace",
            json!(["ro.txt", "old", "new"]),
            json!({}),
        )
        .unwrap();
    } else {
        let opened = f.handle(open_call(&p, "w")).unwrap_err();
        f.take_events();
        assert_eq!(
            helper(
                &mut f,
                "replace",
                json!(["ro.txt", "old", "new"]),
                json!({})
            )
            .unwrap_err(),
            raised(&opened.0, format!("replace(): {}", opened.1))
        );
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "old");
        assert!(replace_leftovers(&t.0).is_empty());
        replace_wrote_nothing(&mut f);
    }
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&p, permissions).unwrap();
}

#[test]
fn replace_leaves_no_capture_record_or_temp_file_when_its_rename_fails() {
    let t = Temp::new();
    let captures_dir = Temp::new();
    std::fs::write(t.0.join("a.txt"), "old").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        capture_dir: Some(captures_dir.0.clone()),
        replace_hook: Some(|p: &std::path::Path| {
            let dir = p.parent().unwrap();
            for name in replace_leftovers(dir) {
                std::fs::remove_file(dir.join(name)).unwrap();
            }
        }),
        ..Default::default()
    });
    let (kind, message) =
        helper(&mut f, "replace", json!(["a.txt", "old", "new"]), json!({})).unwrap_err();
    assert_eq!(kind, "FileNotFoundError");
    assert!(message.starts_with("replace(): "), "{message}");
    assert!(message.ends_with("the file is unchanged."), "{message}");
    assert_eq!(std::fs::read_to_string(t.0.join("a.txt")).unwrap(), "old");
    assert!(replace_leftovers(&t.0).is_empty());
    replace_wrote_nothing(&mut f);
    assert_eq!(
        std::fs::read_dir(&captures_dir.0).map_or(0, |d| d.count()),
        0
    );
}

#[test]
fn replace_detects_a_change_of_identity_mtime_link_count_or_mode_alone() {
    let t = Temp::new();
    let hooks: Vec<(&str, fn(&std::path::Path))> = vec![
        ("identity", |p| {
            let modified = std::fs::metadata(p).unwrap().modified().unwrap();
            let copy = p.with_extension("copy");
            std::fs::write(&copy, std::fs::read(p).unwrap()).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&copy)
                .unwrap()
                .set_modified(modified)
                .unwrap();
            std::fs::rename(&copy, p).unwrap();
        }),
        ("mtime", |p| {
            let modified = std::fs::metadata(p).unwrap().modified().unwrap();
            std::fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(modified + std::time::Duration::from_secs(10))
                .unwrap();
        }),
        ("links", |p| {
            std::fs::hard_link(p, p.with_extension("alias")).unwrap();
        }),
        #[cfg(unix)]
        ("mode", |p| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o640)).unwrap();
        }),
    ];
    for (what, hook) in hooks {
        let dir = t.0.join(what);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("a.txt"), std::fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        let mut f = HostFs::new(HostFsConfig {
            project_root: dir.clone(),
            replace_hook: Some(hook),
            ..Default::default()
        });
        assert_eq!(
            helper(&mut f, "replace", json!(["a.txt", "old", "new"]), json!({})).unwrap_err(),
            raised(
                "RuntimeError",
                "replace(): a.txt changed on disk while replace() ran; nothing was written. Read it again and retry."
            ),
            "{what}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "old",
            "{what}"
        );
        assert!(replace_leftovers(&dir).is_empty(), "{what}");
        replace_wrote_nothing(&mut f);
    }
}

#[cfg(unix)]
#[test]
fn replace_keeps_mode_bits_a_write_would_clear() {
    use std::os::unix::fs::PermissionsExt;
    let t = Temp::new();
    let p = t.0.join("tool.sh");
    std::fs::write(&p, "echo old\n").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o4755)).unwrap();
    let mut f = t.fs();
    helper(
        &mut f,
        "replace",
        json!(["tool.sh", "old", "new"]),
        json!({}),
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "echo new\n");
    assert_eq!(
        std::fs::metadata(&p).unwrap().permissions().mode() & 0o7777,
        0o4755
    );
}

#[cfg(unix)]
#[test]
fn replace_captures_and_records_the_path_as_given_like_open() {
    let t = Temp::new();
    let captures_dir = Temp::new();
    for name in ["one", "two"] {
        std::fs::write(t.0.join(format!("{name}.txt")), "orig").unwrap();
        std::os::unix::fs::symlink(
            t.0.join(format!("{name}.txt")),
            t.0.join(format!("{name}-link.txt")),
        )
        .unwrap();
    }
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        capture_dir: Some(captures_dir.0.clone()),
        ..Default::default()
    });
    write(&mut f, "one-link.txt", "x1");
    helper(
        &mut f,
        "replace",
        json!(["one-link.txt", "x1", "y"]),
        json!({}),
    )
    .unwrap();
    helper(
        &mut f,
        "replace",
        json!(["two-link.txt", "orig", "x2"]),
        json!({}),
    )
    .unwrap();
    write(&mut f, "two-link.txt", "z");
    let events = f.take_events();
    let captured = captures(&events);
    assert_eq!(captured.len(), 2, "{captured:?}");
    for name in ["one-link.txt", "two-link.txt"] {
        let (_, capture) = captured
            .iter()
            .find(|(p, _)| p == &t.0.join(name))
            .unwrap_or_else(|| panic!("{name}: {captured:?}"));
        assert!(
            matches!(capture, WriteOriginal::File { capture_path } if std::fs::read_to_string(capture_path).unwrap() == "orig"),
            "{name}"
        );
    }
    let replaced: Vec<PathBuf> = events
        .iter()
        .filter_map(|e| match e {
            KernelEvent::Fs {
                op,
                path,
                ..
            } if op.starts_with("replace") => Some(PathBuf::from(path)),
            _ => None,
        })
        .collect();
    assert_eq!(
        replaced,
        vec![
            t.0.join("one-link.txt"),
            t.0.join("one-link.txt"),
            t.0.join("two-link.txt"),
            t.0.join("two-link.txt")
        ]
    );
    assert_eq!(std::fs::read_to_string(t.0.join("one.txt")).unwrap(), "y");
    assert_eq!(std::fs::read_to_string(t.0.join("two.txt")).unwrap(), "z");
}

#[test]
fn replace_shares_the_first_write_capture_with_open() {
    let t = Temp::new();
    let captures_dir = Temp::new();
    std::fs::write(t.0.join("a.txt"), "1").unwrap();
    std::fs::write(t.0.join("b.txt"), "orig").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        capture_dir: Some(captures_dir.0.clone()),
        ..Default::default()
    });
    helper(&mut f, "replace", json!(["a.txt", "1", "2"]), json!({})).unwrap();
    write(&mut f, "a.txt", "3");
    write(&mut f, "b.txt", "x1");
    helper(&mut f, "replace", json!(["b.txt", "x1", "y"]), json!({})).unwrap();
    let captured = captures(&f.take_events());
    assert_eq!(captured.len(), 2);
    for (path, original) in [("a.txt", "1"), ("b.txt", "orig")] {
        let (_, capture) = captured.iter().find(|(p, _)| p == &t.0.join(path)).unwrap();
        assert!(
            matches!(capture, WriteOriginal::File { capture_path } if std::fs::read_to_string(capture_path).unwrap() == original),
            "{path}"
        );
    }
}

#[test]
fn replace_refuses_protected_paths_and_the_artifacts_root() {
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("state/artifacts")).unwrap();
    std::fs::write(t.0.join("state/state.json"), "old").unwrap();
    std::fs::write(t.0.join("state/artifacts/ok.txt"), "old").unwrap();
    let mut f = artifacts_fs(&t);
    assert_eq!(
        helper(&mut f, "replace", json!(["state/state.json", "old", "new"]), json!({})).unwrap_err(),
        raised(
            "PermissionError",
            format!(
                "replace(): engine configuration or state: {}; use the file-edit tools to change it",
                t.0.join("state/state.json").display()
            )
        )
    );
    assert_eq!(
        helper(
            &mut f,
            "replace",
            json!(["state/artifacts", "old", "new"]),
            json!({})
        )
        .unwrap_err()
        .0,
        "PermissionError"
    );
    helper(
        &mut f,
        "replace",
        json!(["state/artifacts/ok.txt", "old", "new"]),
        json!({}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(t.0.join("state/state.json")).unwrap(),
        "old"
    );
    assert_eq!(
        std::fs::read_to_string(t.0.join("state/artifacts/ok.txt")).unwrap(),
        "new"
    );
}

#[test]
fn replace_in_a_read_only_chat_is_refused_before_it_reads() {
    let c = Chat::new();
    let a = c.project.join("a.txt");
    std::fs::write(&a, "old").unwrap();
    std::fs::write(c.artifacts.join("b.txt"), "old").unwrap();
    let mut f = c.fs(true);
    for (old, new) in [("old", "new"), ("", "x"), ("old", "old")] {
        assert_eq!(
            helper(&mut f, "replace", json!([text_of(&a), old, new]), json!({})).unwrap_err(),
            raised("PermissionError", format!("replace(): {}", refused(&a).1))
        );
    }
    assert!(f.take_events().is_empty());
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "old");
    helper(
        &mut f,
        "replace",
        json!([text_of(&c.artifacts.join("b.txt")), "old", "new"]),
        json!({}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(c.artifacts.join("b.txt")).unwrap(),
        "new"
    );
}

#[test]
fn replace_diff_is_cut_at_60_lines_and_4000_characters() {
    let t = Temp::new();
    let lines: String = (0..100).map(|i| format!("x{i}\n")).collect();
    std::fs::write(t.0.join("a.txt"), lines).unwrap();
    let wide: String = (0..50)
        .map(|i| format!("w{i:03}{}\n", "-".repeat(116)))
        .collect();
    std::fs::write(t.0.join("wide.txt"), wide).unwrap();
    let mut f = t.fs();
    let diff = helper(
        &mut f,
        "replace",
        json!(["a.txt", "x", "y"]),
        json!({"count": 0}),
    )
    .unwrap();
    let diff: Vec<&str> = diff.as_str().unwrap().lines().collect();
    assert_eq!(diff.len(), 61);
    assert_eq!(
        &diff[..3],
        &["--- a/a.txt", "+++ b/a.txt", "@@ -1,100 +1,100 @@"]
    );
    assert_eq!(diff[60], "… 143 more diff lines");
    let diff = helper(
        &mut f,
        "replace",
        json!(["wide.txt", "w", "W"]),
        json!({"count": 0}),
    )
    .unwrap();
    let diff = diff.as_str().unwrap();
    let (kept, note) = diff.trim_end().rsplit_once('\n').unwrap();
    assert!(kept.chars().count() <= 4000, "{}", kept.chars().count());
    let shown = kept.lines().count();
    assert_eq!(note, format!("… {} more diff lines", 103 - shown));
}

fn helper_bindings() -> Vec<GlobalBinding> {
    ["read", "grep", "replace"]
        .into_iter()
        .map(|name| GlobalBinding {
            name: name.into(),
            kind: GlobalKind::HostHelper,
        })
        .collect()
}

async fn next_event(
    client: &mut refact_python_kernel::client::KernelClient,
) -> refact_python_kernel::client::ClientEvent {
    tokio::time::timeout(std::time::Duration::from_secs(15), client.next_event())
        .await
        .unwrap()
}

async fn helper_client(os: Box<dyn OsBackend>) -> refact_python_kernel::client::KernelClient {
    let mut client = refact_python_kernel::client::KernelClient::spawn_in_thread(
        os,
        refact_python_kernel::client::ClientOptions::default(),
    )
    .unwrap();
    next_event(&mut client).await;
    client
}

async fn feed_helpers(
    client: &refact_python_kernel::client::KernelClient,
    run_id: &str,
    code: &str,
) {
    client
        .send(KernelCommand::Feed {
            run_id: run_id.into(),
            code: code.into(),
            globals: helper_bindings(),
            limits: RunLimits::default(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn helpers_run_in_the_worker_and_ignore_chdir() {
    use refact_python_kernel::client::ClientEvent;
    let t = Temp::new();
    std::fs::create_dir(t.0.join("sub")).unwrap();
    std::fs::write(t.0.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(t.0.join("sub/a.txt"), "other\n").unwrap();
    std::fs::write(t.0.join("bin.dat"), b"\xff").unwrap();
    let mut client = helper_client(Box::new(t.fs())).await;
    feed_helpers(
        &client,
        "helpers",
        "from pathlib import Path\nimport os\nos.chdir('sub')\nr = read('a.txt', 2)\np = read(Path('a.txt'), numbered=False)\nd = replace('a.txt', 'two', 'TWO')\ng = grep('TWO', 'a.txt')\ntry:\n    read('bin.dat')\n    u = 'read'\nexcept UnicodeDecodeError as e:\n    u = str(e)\ntry:\n    read(1)\n    w = 'read'\nexcept TypeError as e:\n    w = str(e)\n[r, p, d.split('\\n')[0], read('a.txt', numbered=False), g, u, w]",
    )
    .await;
    let mut ops = vec![];
    let mut pending_grep = None;
    let value = loop {
        match next_event(&mut client).await {
            ClientEvent::Kernel(KernelEvent::Fs {
                op,
                run_id,
                ..
            }) => {
                assert_eq!(run_id, "helpers");
                if op == "grep" {
                    assert!(
                        pending_grep.is_none(),
                        "grep audit preceded its confirmation"
                    );
                }
                ops.push(op);
            }
            ClientEvent::Kernel(KernelEvent::SyncCall {
                function,
                call_id,
                args,
                ..
            }) => {
                assert_eq!(function, GREP_FILES);
                assert!(pending_grep.replace(call_id).is_none());
                let request: FileListRequest = serde_json::from_value(args[0].clone()).unwrap();
                assert_eq!(request.path, t.0.join("a.txt"));
                client
                    .send(KernelCommand::SyncResult {
                        call_id,
                        result: HostResult::Ok(json!(FileList {
                            files: vec![request.path],
                            ..Default::default()
                        })),
                    })
                    .await
                    .unwrap();
            }
            ClientEvent::Kernel(KernelEvent::GrepRevalidate {
                run_id,
                call_id,
            }) => {
                assert_eq!(run_id, "helpers");
                assert_eq!(pending_grep.take(), Some(call_id));
                client
                    .send(KernelCommand::GrepRevalidated {
                        run_id,
                        call_id,
                        allowed: true,
                    })
                    .await
                    .unwrap();
            }
            ClientEvent::Kernel(KernelEvent::WriteCapture {
                run_id, ..
            }) => {
                assert_eq!(run_id, "helpers")
            }
            ClientEvent::Kernel(KernelEvent::Completed {
                value, ..
            }) => break value,
            other => panic!("unexpected {other:?}"),
        }
    };
    assert_eq!(
        value,
        json!([
            "   2 | two\n",
            "one\ntwo\n",
            "--- a/a.txt",
            "one\nTWO\n",
            "a.txt:2: TWO",
            "read(): bin.dat is not UTF-8 text (invalid byte at offset 0); use open(path, \"rb\")",
            "read() argument path must be str or Path, not int"
        ])
    );
    assert_eq!(
        std::fs::read_to_string(t.0.join("sub/a.txt")).unwrap(),
        "other\n"
    );
    assert!(pending_grep.is_none());
    assert_eq!(ops.iter().filter(|op| *op == "grep").count(), 1, "{ops:?}");
    assert!(ops.contains(&"replace.write".to_string()), "{ops:?}");
    client.send(KernelCommand::Shutdown).await.unwrap();
}

#[tokio::test]
async fn grep_completion_without_confirmation_never_releases_scan_results_or_audits() {
    use refact_python_kernel::client::ClientEvent;
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "apple sample\n").unwrap();
    let mut client = helper_client(Box::new(t.fs())).await;
    for run in [
        "denied",
        "wrong-call",
        "wrong-run",
        "wrong-response",
        "aborted",
    ] {
        feed_helpers(&client, run, "x = 41\ngrep('apple', 'a.txt', data=True)").await;
        let ClientEvent::Kernel(KernelEvent::SyncCall {
            run_id,
            call_id,
            function,
            ..
        }) = next_event(&mut client).await
        else {
            panic!("expected listing request")
        };
        assert_eq!(run_id, run);
        assert_eq!(function, GREP_FILES);
        client
            .send(KernelCommand::SyncResult {
                call_id,
                result: HostResult::Ok(json!(FileList {
                    files: vec![t.0.join("a.txt")],
                    ..Default::default()
                })),
            })
            .await
            .unwrap();
        let event = next_event(&mut client).await;
        assert!(
            matches!(event, ClientEvent::Kernel(KernelEvent::GrepRevalidate {
            run_id: ref echoed, call_id: id,
        }) if echoed == run && id == call_id),
            "{event:?}"
        );
        let reply = match run {
            "aborted" => KernelCommand::Abort {
                run_id,
            },
            "wrong-response" => KernelCommand::SyncResult {
                call_id,
                result: HostResult::Ok(Value::Null),
            },
            _ => KernelCommand::GrepRevalidated {
                run_id: if run == "wrong-run" {
                    "elsewhere".into()
                } else {
                    run_id
                },
                call_id: if run == "wrong-call" {
                    call_id + 1
                } else {
                    call_id
                },
                allowed: run != "denied",
            },
        };
        client.send(reply).await.unwrap();
        let event = next_event(&mut client).await;
        match (run, event) {
            (
                "aborted",
                ClientEvent::Kernel(KernelEvent::Cancelled {
                    run_id,
                }),
            ) => assert_eq!(run_id, run),
            (
                _,
                ClientEvent::Kernel(KernelEvent::Raised {
                    run_id,
                    exc_type,
                    message,
                    ..
                }),
            ) if run != "aborted" => {
                assert_eq!(run_id, run);
                assert_eq!(exc_type, "RuntimeError");
                assert_eq!(message, "grep(): results unavailable");
            }
            (_, other) => panic!("unexpected {other:?}"),
        }
    }
    feed_helpers(&client, "after", "x + 1").await;
    let event = next_event(&mut client).await;
    assert!(
        matches!(event, ClientEvent::Kernel(KernelEvent::Completed { ref run_id, ref value, .. })
        if run_id == "after" && *value == json!(42)),
        "{event:?}"
    );
    client.send(KernelCommand::Shutdown).await.unwrap();
}

#[tokio::test]
async fn a_path_binds_only_to_the_path_of_a_helper_and_ints_past_64_bits_are_value_errors() {
    use refact_python_kernel::client::ClientEvent;
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "old\n").unwrap();
    let mut client = helper_client(Box::new(t.fs())).await;
    feed_helpers(
        &client,
        "marks",
        "from pathlib import Path\nout = []\nfor call in [lambda: grep(Path('old')), lambda: grep('old', glob=Path('*.txt')), lambda: replace('a.txt', Path('old'), 'new'), lambda: replace('a.txt', 'old', new=Path('new')), lambda: read('a.txt', Path('1')), lambda: read('a.txt', numbered=Path('x')), lambda: read('a.txt', 2**63), lambda: read('a.txt', end=-2**63 - 1), lambda: replace('a.txt', 'old', 'new', count=2**64), lambda: grep('old', limit=10**30), lambda: read(2**70)]:\n    try:\n        call()\n        out.append('ran')\n    except (TypeError, ValueError) as e:\n        out.append(type(e).__name__ + ': ' + str(e))\nout.append(read(Path('a.txt'), numbered=False))\nout",
    )
    .await;
    let value = loop {
        match next_event(&mut client).await {
            ClientEvent::Kernel(KernelEvent::SyncCall {
                call_id, ..
            }) => client
                .send(KernelCommand::SyncResult {
                    call_id,
                    result: HostResult::Ok(json!(FileList {
                        files: vec![t.0.join("a.txt")],
                        ..Default::default()
                    })),
                })
                .await
                .unwrap(),
            ClientEvent::Kernel(KernelEvent::Completed {
                value, ..
            }) => break value,
            ClientEvent::Kernel(KernelEvent::Fs {
                ..
            }) => {}
            other => panic!("unexpected {other:?}"),
        }
    };
    assert_eq!(
        value,
        json!([
            "TypeError: grep() argument pattern must be str, not PosixPath",
            "TypeError: grep() argument glob must be str or None, not PosixPath",
            "TypeError: replace() argument old must be str, not PosixPath",
            "TypeError: replace() argument new must be str, not PosixPath",
            "TypeError: read() argument start must be int, not PosixPath",
            "TypeError: read() argument numbered must be bool, not PosixPath",
            "ValueError: read() argument start is out of range",
            "ValueError: read() argument end is out of range",
            "ValueError: replace() argument count is out of range",
            "ValueError: grep() argument limit is out of range",
            "TypeError: read() argument path must be str or Path, not int",
            "old\n"
        ])
    );
    assert_eq!(std::fs::read_to_string(t.0.join("a.txt")).unwrap(), "old\n");
    client.send(KernelCommand::Shutdown).await.unwrap();
}

#[test]
fn helper_ints_past_64_bits_are_value_errors_not_clamped() {
    let t = Temp::new();
    std::fs::write(t.0.join("a.txt"), "a\n").unwrap();
    let mut f = t.fs();
    for (name, args, kwargs, message) in [
        (
            "read",
            json!(["a.txt", u64::MAX]),
            json!({}),
            "read() argument start is out of range",
        ),
        (
            "read",
            json!(["a.txt"]),
            json!({"end": u64::MAX}),
            "read() argument end is out of range",
        ),
        (
            "replace",
            json!(["a.txt", "a", "b"]),
            json!({"count": u64::MAX}),
            "replace() argument count is out of range",
        ),
    ] {
        assert_eq!(
            helper(&mut f, name, args.clone(), kwargs).unwrap_err(),
            raised("ValueError", message),
            "{name}{args}"
        );
    }
    assert_eq!(std::fs::read_to_string(t.0.join("a.txt")).unwrap(), "a\n");
}

#[tokio::test]
async fn a_script_cannot_call_grep_files_or_answer_it() {
    use refact_python_kernel::client::ClientEvent;
    let t = Temp::new();
    std::fs::create_dir(t.0.join("d")).unwrap();
    std::fs::write(t.0.join("d/a.txt"), "needle\n").unwrap();
    let mut client = helper_client(Box::new(t.fs())).await;
    feed_helpers(&client, "dotted", "x = 5\ngrep.files('d')").await;
    loop {
        match next_event(&mut client).await {
            ClientEvent::Kernel(KernelEvent::Raised {
                exc_type, ..
            }) => {
                assert_eq!(exc_type, "AttributeError");
                break;
            }
            ClientEvent::Kernel(KernelEvent::SyncCall {
                ..
            }) => panic!("grep.files was reached"),
            _ => {}
        }
    }
    for (run, answer, expected) in [
        (
            "malformed",
            Some(HostResult::Ok(json!("not a file list"))),
            "RuntimeError",
        ),
        (
            "failed",
            Some(HostResult::Err {
                exc_type: "RuntimeError".into(),
                message: "listing failed".into(),
            }),
            "RuntimeError",
        ),
        ("wrong-id", None, "Cancelled"),
        ("aborted", None, "Cancelled"),
    ] {
        feed_helpers(&client, run, "grep('needle', 'd')").await;
        let outcome = loop {
            match next_event(&mut client).await {
                ClientEvent::Kernel(KernelEvent::SyncCall {
                    call_id, ..
                }) => {
                    let command = match (&answer, run) {
                        (Some(result), _) => KernelCommand::SyncResult {
                            call_id,
                            result: result.clone(),
                        },
                        (None, "wrong-id") => KernelCommand::SyncResult {
                            call_id: call_id + 1,
                            result: HostResult::Ok(json!(FileList::default())),
                        },
                        (None, _) => KernelCommand::Abort {
                            run_id: run.into(),
                        },
                    };
                    client.send(command).await.unwrap();
                }
                ClientEvent::Kernel(KernelEvent::Raised {
                    exc_type,
                    message,
                    ..
                }) => {
                    if run == "failed" {
                        assert_eq!(message, "listing failed");
                    }
                    break exc_type;
                }
                ClientEvent::Kernel(KernelEvent::Cancelled {
                    ..
                }) => break "Cancelled".into(),
                ClientEvent::Kernel(KernelEvent::Completed {
                    value, ..
                }) => {
                    panic!("{run} completed with {value}")
                }
                _ => {}
            }
        };
        assert_eq!(outcome, expected, "{run}");
    }
    feed_helpers(&client, "after", "x").await;
    loop {
        if let ClientEvent::Kernel(KernelEvent::Completed {
            value, ..
        }) = next_event(&mut client).await
        {
            assert_eq!(value, json!(5));
            break;
        }
    }
    client.send(KernelCommand::Shutdown).await.unwrap();
}

#[tokio::test]
async fn helpers_are_unavailable_without_a_host_file_system() {
    use refact_python_kernel::client::ClientEvent;
    let mut client = helper_client(Box::new(refact_python_kernel::kernel::FakeOs::default())).await;
    feed_helpers(&client, "fake", "read('a.txt')").await;
    loop {
        match next_event(&mut client).await {
            ClientEvent::Kernel(KernelEvent::Raised {
                exc_type,
                message,
                ..
            }) => {
                assert_eq!(
                    (exc_type.as_str(), message.as_str()),
                    ("NameError", "name 'read' is not available in this session")
                );
                break;
            }
            ClientEvent::Kernel(KernelEvent::SyncCall {
                ..
            }) => panic!("sent to the engine"),
            _ => {}
        }
    }
    client.send(KernelCommand::Shutdown).await.unwrap();
}
#[test]
fn may_read_answers_what_a_read_of_the_path_would() {
    let t = Temp::new();
    std::fs::create_dir_all(t.0.join("config/inner")).unwrap();
    std::fs::create_dir_all(t.0.join("config/allowed")).unwrap();
    std::fs::write(t.0.join("secret.yaml"), "s").unwrap();
    std::fs::write(t.0.join("public.txt"), "p").unwrap();
    let mut f = HostFs::new(HostFsConfig {
        project_root: t.0.clone(),
        deny_read: PathDenyRules {
            prefixes: vec![t.0.join("config")],
            allow_prefixes: vec![t.0.join("config/allowed")],
            files: vec![t.0.join("secret.yaml")],
            ..Default::default()
        },
        ..Default::default()
    });
    for (path, expected) in [
        ("public.txt", true),
        ("missing/new.txt", true),
        ("secret.yaml", false),
        ("config/inner/x.txt", false),
        ("config/gone.txt", false),
        ("config/allowed/ok.txt", true),
    ] {
        assert_eq!(f.may_read(&t.0.join(path)), expected, "{path}");
        let read = f.handle(OsFunctionCall::ReadText(path.into()));
        if !expected {
            assert_eq!(read.unwrap_err().0, "PermissionError", "{path}");
        }
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(t.0.join("secret.yaml"), t.0.join("alias.yaml")).unwrap();
        assert!(!f.may_read(&t.0.join("alias.yaml")));
    }
}
