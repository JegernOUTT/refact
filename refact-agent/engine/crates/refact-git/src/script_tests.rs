use super::*;
use crate::test_repo::{commit, commit_files, init_repo, stage, write};
use serde_json::json;
use std::fs;

fn parse(kwargs: Value) -> Result<GitRequest, String> {
    GitRequest::parse(&kwargs)
}

fn root(dir: &tempfile::TempDir) -> PathBuf {
    dunce::canonicalize(dir.path()).unwrap()
}

fn call(
    dir: &tempfile::TempDir,
    folders: &[PathBuf],
    blocked: &[&str],
    request: GitRequest,
) -> Result<GitOutput, GitError> {
    let base = root(dir);
    let blocked: Vec<PathBuf> = blocked.iter().map(|p| base.join(p)).collect();
    let may_read = move |path: &Path| !blocked.iter().any(|b| b == path);
    let cancel = AtomicBool::new(false);
    run(
        &request,
        &GitContext {
            root: dir.path(),
            folders,
            may_read: &may_read,
            cancel: &cancel,
        },
    )
}

fn whole(dir: &tempfile::TempDir, request: GitRequest) -> GitOutput {
    call(dir, &[root(dir)], &[], request).unwrap()
}

fn read_at(path: &Path, request: GitRequest) -> Result<GitOutput, GitError> {
    let folder = dunce::canonicalize(path).unwrap();
    let may_read = |_: &Path| true;
    let cancel = AtomicBool::new(false);
    run(
        &request,
        &GitContext {
            root: path,
            folders: std::slice::from_ref(&folder),
            may_read: &may_read,
            cancel: &cancel,
        },
    )
}

fn files(output: &GitOutput) -> Vec<(String, String)> {
    output.data["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap().to_string(),
                f["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn diff_request(rev: Option<&str>, staged: bool) -> GitRequest {
    GitRequest::Diff {
        rev: rev.map(str::to_string),
        paths: vec![],
        staged,
    }
}

#[test]
fn parse_accepts_the_documented_forms() {
    assert_eq!(
        parse(json!({"subcommand":"status"})),
        Ok(GitRequest::Status)
    );
    assert_eq!(
        parse(json!({"subcommand":"branches"})),
        Ok(GitRequest::Branches)
    );
    assert_eq!(
        parse(
            json!({"subcommand":"diff","rev":"main...topic","path":["src","./b.rs"],"staged":null})
        ),
        Ok(GitRequest::Diff {
            rev: Some("main...topic".into()),
            paths: vec!["src".into(), "b.rs".into()],
            staged: false
        })
    );
    assert_eq!(
        parse(json!({"subcommand":"diff","staged":true,"rev":"HEAD~1"})),
        Ok(GitRequest::Diff {
            rev: Some("HEAD~1".into()),
            paths: vec![],
            staged: true
        })
    );
    assert_eq!(
        parse(json!({"subcommand":"log","n":10,"rev":"v1..HEAD","path":"a.txt"})),
        Ok(GitRequest::Log {
            n: 10,
            rev: Some("v1..HEAD".into()),
            paths: vec!["a.txt".into()]
        })
    );
    assert_eq!(
        parse(json!({"subcommand":"log"})),
        Ok(GitRequest::Log {
            n: DEFAULT_LOG_COMMITS,
            rev: None,
            paths: vec![]
        })
    );
    assert_eq!(
        parse(json!({"subcommand":"show","#1":"HEAD~2","#2":"src/a.rs"})),
        Ok(GitRequest::Show {
            rev: "HEAD~2".into(),
            path: Some("src/a.rs".into())
        })
    );
    assert_eq!(
        parse(json!({"subcommand":"show","rev":"abc123"})),
        Ok(GitRequest::Show {
            rev: "abc123".into(),
            path: None
        })
    );
    assert_eq!(
        parse(json!({"subcommand":"diff","path":"."})),
        Ok(diff_request(None, false))
    );
    assert_eq!(
        parse(json!({"subcommand":"diff","path":":(top)x"})),
        Ok(GitRequest::Diff {
            rev: None,
            paths: vec![":(top)x".into()],
            staged: false
        })
    );
}

#[test]
fn parse_refuses_writes_options_and_non_commit_forms() {
    for subcommand in ["push", "commit", "config", "checkout", "fetch", "blame"] {
        assert_eq!(
            parse(json!({"subcommand": subcommand})).unwrap_err(),
            format!("git(): unknown subcommand '{subcommand}'; read-only subcommands: status, diff, log, show, branches")
        );
    }
    let refused = [
        (json!({}), "git() missing required argument: 'subcommand'"),
        (json!({"subcommand":1}), "git() subcommand must be a str"),
        (json!({"subcommand":"diff","rev":"--output=/tmp/x"}), "git(): Revision must not start with '-'"),
        (json!({"subcommand":"show","rev":"-p"}), "git(): Revision must not start with '-'"),
        (json!({"subcommand":"diff","path":"../x"}), "git(): Git paths must be non-empty paths relative to the repository"),
        (json!({"subcommand":"diff","path":"/etc/passwd"}), "git(): Git paths must be non-empty paths relative to the repository"),
        (json!({"subcommand":"diff","path":""}), "git(): Git paths must be non-empty paths relative to the repository"),
        (json!({"subcommand":"diff","path":[]}), "git(): path must be a str or a list of str"),
        (json!({"subcommand":"diff","path":[1]}), "git(): path must be a str or a list of str"),
        (json!({"subcommand":"diff","args":["--exec=x"]}), "git('diff') got an unexpected keyword argument 'args'"),
        (json!({"subcommand":"log","output":"x"}), "git('log') got an unexpected keyword argument 'output'"),
        (json!({"subcommand":"status","path":"a"}), "git('status') got an unexpected keyword argument 'path'"),
        (json!({"subcommand":"status","#1":"x"}), "git('status') takes no positional argument after the subcommand"),
        (json!({"subcommand":"diff","#1":"HEAD"}), "git('diff') takes no positional argument after the subcommand; pass rev=, path=, staged= as keywords"),
        (json!({"subcommand":"show"}), "git('show') missing required argument: 'rev'"),
        (json!({"subcommand":"show","#1":"HEAD","rev":"HEAD"}), "git('show') got multiple values for argument 'rev'"),
        (json!({"subcommand":"show","rev":"a..b"}), "git('show'): rev must name one commit, not a range"),
        (json!({"subcommand":"log","rev":"a...b"}), "git(): '...' ranges are accepted only by diff"),
        (json!({"subcommand":"diff","rev":"a..b","staged":true}), "git('diff'): staged cannot be combined with a range"),
        (json!({"subcommand":"diff","staged":"yes"}), "git('diff'): staged must be a bool"),
        (json!({"subcommand":"log","n":0}), "git('log'): n must be an integer between 1 and 200"),
        (json!({"subcommand":"log","n":201}), "git('log'): n must be an integer between 1 and 200"),
        (json!({"subcommand":"log","n":"5"}), "git('log'): n must be an integer between 1 and 200"),
        (json!({"subcommand":"log","rev":5}), "git(): rev must be a str"),
    ];
    for (kwargs, text) in refused {
        assert_eq!(parse(kwargs.clone()).unwrap_err(), text, "{kwargs}");
    }
    for rev in [
        "HEAD:a.txt",
        ":0:a.txt",
        ":/fix",
        "HEAD@{1}",
        "@{u}",
        "v1^{tree}",
        "a..b..c",
    ] {
        let error = parse(json!({"subcommand":"diff","rev":rev})).unwrap_err();
        assert!(
            error.contains(
                "must name a commit; blob, index, path, reflog and peel forms are not accepted"
            ),
            "{rev}: {error}"
        );
    }
}

#[test]
fn checked_requests_render_as_hook_arguments_and_command_lines() {
    let request = GitRequest::Diff {
        rev: Some("HEAD~1".into()),
        paths: vec!["src/a.rs".into()],
        staged: true,
    };
    assert_eq!(
        request.to_kwargs(),
        json!({"subcommand":"diff","rev":"HEAD~1","path":["src/a.rs"],"staged":true})
    );
    assert_eq!(request.command_line(), "diff --staged HEAD~1 -- src/a.rs");
    assert_eq!(GitRequest::parse(&request.to_kwargs()), Ok(request));
    let show = GitRequest::Show {
        rev: "HEAD".into(),
        path: Some("a.txt".into()),
    };
    assert_eq!(show.command_line(), "show HEAD:a.txt");
    assert_eq!(GitRequest::parse(&show.to_kwargs()), Ok(show));
    let log = parse(json!({"subcommand":"log","n":5})).unwrap();
    assert_eq!(log.command_line(), "log -n 5");
    assert_eq!(GitRequest::Status.command_line(), "status");
}

#[test]
fn status_lists_changes_and_withholds_blocked_files_without_names() {
    let (dir, repo) = init_repo();
    commit_files(
        &repo,
        &[
            ("a.txt", "a\n"),
            ("old.txt", "renamed content\n"),
            ("secret.txt", "s\n"),
        ],
        "init",
    );
    write(dir.path(), "a.txt", "a2\n");
    fs::rename(dir.path().join("old.txt"), dir.path().join("new.txt")).unwrap();
    stage(&repo, &["old.txt", "new.txt"]);
    write(dir.path(), "secret.txt", "s2\n");
    write(dir.path(), "notes/todo.txt", "t\n");
    write(dir.path(), "notes/hidden.txt", "h\n");

    let output = call(
        &dir,
        &[root(&dir)],
        &["secret.txt", "notes/hidden.txt"],
        GitRequest::Status,
    )
    .unwrap();
    let data = &output.data;
    assert_eq!(
        data["changes"],
        json!([
            {"path":"a.txt","status":"MODIFIED","staged":false,"unstaged":true},
            {"path":"new.txt","old_path":"old.txt","status":"RENAMED","staged":true,"unstaged":false},
        ])
    );
    assert_eq!(data["untracked"], json!(["notes/todo.txt"]));
    assert_eq!(data["withheld_count"], 2);
    assert_eq!(data["unborn"], false);
    assert_eq!(data["root"], json!(root(&dir).to_string_lossy()));
    assert!(!output.data.to_string().contains("secret"));
    assert!(!output.data.to_string().contains("hidden"));
    assert!(output.paths.is_empty(), "names alone carry no records");
    assert!(
        output
            .summary
            .contains("2 changed, 1 untracked, 2 withheld"),
        "{}",
        output.summary
    );
}

#[test]
fn diff_bases_follow_git() {
    let (dir, repo) = init_repo();
    let first = commit_files(&repo, &[("a.txt", "1\n"), ("b.txt", "b\n")], "first");
    let main = repo.head().unwrap().shorthand().unwrap().to_string();
    let base = repo.find_commit(first).unwrap();
    repo.branch("topic", &base, false).unwrap();
    commit_files(&repo, &[("a.txt", "2\n")], "second on main");
    repo.set_head("refs/heads/topic").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();
    commit_files(&repo, &[("t.txt", "topic\n")], "topic work");
    repo.set_head(&format!("refs/heads/{main}")).unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();

    write(dir.path(), "b.txt", "b staged\n");
    stage(&repo, &["b.txt"]);
    write(dir.path(), "a.txt", "3\n");
    write(dir.path(), "untracked.txt", "u\n");

    let worktree = whole(&dir, diff_request(None, false));
    assert_eq!(files(&worktree), vec![("a.txt".into(), "MODIFIED".into())]);
    let patch = worktree.data["files"][0]["patch"].as_str().unwrap();
    assert!(
        patch.starts_with("@@ -1 +1 @@") && patch.contains("-2\n+3\n"),
        "{patch}"
    );
    assert_eq!(worktree.data["files"][0]["additions"], 1);
    assert_eq!(worktree.data["base"], "index..worktree");

    let staged = whole(&dir, diff_request(None, true));
    assert_eq!(files(&staged), vec![("b.txt".into(), "MODIFIED".into())]);
    assert_eq!(staged.data["base"], "HEAD..index");

    let since = whole(&dir, diff_request(Some("HEAD~1"), false));
    assert_eq!(
        files(&since),
        vec![
            ("a.txt".into(), "MODIFIED".into()),
            ("b.txt".into(), "MODIFIED".into())
        ]
    );
    assert!(since.data["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("-1\n+3\n"));

    let range = whole(&dir, diff_request(Some(&format!("{main}..topic")), false));
    assert_eq!(
        files(&range),
        vec![
            ("a.txt".into(), "MODIFIED".into()),
            ("t.txt".into(), "ADDED".into())
        ]
    );
    let symmetric = whole(&dir, diff_request(Some(&format!("{main}...topic")), false));
    assert_eq!(files(&symmetric), vec![("t.txt".into(), "ADDED".into())]);

    for output in [&worktree, &staged, &since, &range, &symmetric] {
        assert!(!output.data.to_string().contains("untracked.txt"));
        assert_eq!(output.data["truncated"], false);
    }
    assert_eq!(
        call(&dir, &[root(&dir)], &[], diff_request(Some("nope"), false)).err(),
        Some(GitError::Failed("revision not found: nope".into()))
    );
}

#[test]
fn diff_withholds_both_sides_of_a_delta_and_never_shows_their_lines() {
    let (dir, repo) = init_repo();
    let secret = "top secret line one\ntop secret line two\ntop secret line three\n";
    commit_files(
        &repo,
        &[("secret.txt", secret), ("public.txt", "p\n")],
        "init",
    );
    fs::rename(dir.path().join("secret.txt"), dir.path().join("moved.txt")).unwrap();
    stage(&repo, &["secret.txt", "moved.txt"]);
    write(dir.path(), "public.txt", "p2\n");
    stage(&repo, &["public.txt"]);

    let open = call(&dir, &[root(&dir)], &[], diff_request(None, true)).unwrap();
    assert!(
        files(&open).contains(&("moved.txt".into(), "RENAMED".into())),
        "{:?}",
        files(&open)
    );
    let base = root(&dir);
    let mut shown = open.paths.clone();
    shown.sort();
    assert_eq!(
        shown,
        vec![
            base.join("moved.txt"),
            base.join("public.txt"),
            base.join("secret.txt")
        ]
    );

    let guarded = call(
        &dir,
        &[root(&dir)],
        &["secret.txt"],
        diff_request(None, true),
    )
    .unwrap();
    assert_eq!(
        files(&guarded),
        vec![("public.txt".into(), "MODIFIED".into())]
    );
    assert_eq!(guarded.data["withheld_count"], 1);
    assert!(!guarded.data.to_string().contains("top secret"));
    assert_eq!(guarded.paths, vec![base.join("public.txt")]);
}

#[test]
fn diff_budgets_apply_before_patches_are_built() {
    let (dir, repo) = init_repo();
    let big_line = |n: usize| (0..n).map(|i| format!("line {i:06}\n")).collect::<String>();
    let mut initial: Vec<(String, String)> = vec![
        (".gitattributes".into(), "*.txt diff\n".into()),
        ("a_huge.txt".into(), "x\n".into()),
        ("b_long.txt".into(), "old\n".into()),
    ];
    for i in 0..12 {
        initial.push((format!("f{i:02}.txt"), "old\n".into()));
    }
    let refs: Vec<(&str, &str)> = initial
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    commit_files(&repo, &refs, "init");
    write(
        dir.path(),
        "a_huge.txt",
        "y\n".repeat(MAX_DIFF_BLOB_BYTES as usize),
    );
    write(dir.path(), "b_long.txt", big_line(8_000));
    for i in 0..12 {
        write(dir.path(), &format!("f{i:02}.txt"), big_line(5_000));
    }

    let output = whole(&dir, diff_request(None, false));
    let entries = output.data["files"].as_array().unwrap();
    let by_path = |path: &str| entries.iter().find(|f| f["path"] == path).unwrap();
    let huge = by_path("a_huge.txt");
    assert_eq!(huge["patch"], Value::Null);
    assert_eq!(huge["patch_truncated"], true);
    let long = by_path("b_long.txt");
    assert_eq!(long["patch_truncated"], true);
    assert!(long["patch"].as_str().unwrap().len() <= MAX_FILE_PATCH_BYTES);
    let total: usize = entries
        .iter()
        .filter_map(|f| f["patch"].as_str())
        .map(str::len)
        .sum();
    assert!(total <= MAX_TOTAL_PATCH_BYTES, "{total}");
    assert!(entries
        .iter()
        .any(|f| f["patch"].is_null() && f["patch_truncated"] == true && f["binary"] == false));
    assert_eq!(output.data["truncated"], true);
    assert!(output.truncated);
    assert_eq!(entries.len(), 14);
}

#[test]
fn diff_lists_at_most_the_file_budget() {
    let (dir, repo) = init_repo();
    let names: Vec<String> = (0..MAX_DIFF_FILES + 5)
        .map(|i| format!("d/{i:04}.txt"))
        .collect();
    let files_in: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "1\n")).collect();
    commit_files(&repo, &files_in, "init");
    for name in &names {
        write(dir.path(), name, "2\n");
    }
    let output = whole(&dir, diff_request(None, false));
    assert_eq!(
        output.data["files"].as_array().unwrap().len(),
        MAX_DIFF_FILES
    );
    assert_eq!(output.data["truncated"], true);
}

#[test]
fn log_walks_ranges_and_paths_and_caps_files_per_commit() {
    let (dir, repo) = init_repo();
    let first = commit_files(&repo, &[("a.txt", "1\n")], "first a");
    commit_files(&repo, &[("b.txt", "1\n")], "only b\n\nwith a body");
    let names: Vec<String> = (0..MAX_COMMIT_FILES + 3)
        .map(|i| format!("m/{i:04}.txt"))
        .collect();
    let many: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "x\n")).collect();
    commit_files(&repo, &many, "many files");
    let last = commit_files(&repo, &[("a.txt", "2\n")], "second a");

    let all = whole(
        &dir,
        GitRequest::Log {
            n: 10,
            rev: None,
            paths: vec![],
        },
    );
    let commits = all.data["commits"].as_array().unwrap();
    assert_eq!(
        commits
            .iter()
            .map(|c| c["subject"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["second a", "many files", "only b", "first a"]
    );
    assert_eq!(commits[0]["hash"], last.to_string());
    assert_eq!(
        commits[0]["refs"],
        json!(["HEAD", repo.head().unwrap().shorthand().unwrap()])
    );
    assert_eq!(
        commits[0]["files"],
        json!([{"path":"a.txt","status":"MODIFIED"}])
    );
    assert_eq!(
        commits[1]["files"].as_array().unwrap().len(),
        MAX_COMMIT_FILES
    );
    assert_eq!(commits[1]["files_truncated"], true);
    assert_eq!(commits[2]["message"], "only b\n\nwith a body");
    assert!(
        commits[0]["date"]
            .as_str()
            .unwrap()
            .starts_with("2023-11-14T"),
        "{}",
        commits[0]["date"]
    );
    assert_eq!(all.data["shallow"], false);
    assert_eq!(all.summary, "4 commits");

    let path = whole(
        &dir,
        GitRequest::Log {
            n: 10,
            rev: None,
            paths: vec!["a.txt".into()],
        },
    );
    let subjects = |output: &GitOutput| {
        output.data["commits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["subject"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(subjects(&path), vec!["second a", "first a"]);

    let range = whole(
        &dir,
        GitRequest::Log {
            n: 10,
            rev: Some(format!("{first}..HEAD")),
            paths: vec![],
        },
    );
    assert_eq!(subjects(&range), vec!["second a", "many files", "only b"]);
    assert_eq!(range.data["rev"], format!("{first}..{last}"));

    let one = whole(
        &dir,
        GitRequest::Log {
            n: 1,
            rev: None,
            paths: vec![],
        },
    );
    assert_eq!(subjects(&one), vec!["second a"]);
}

#[test]
fn show_gives_a_commit_with_patches_or_one_file_at_a_revision() {
    let (dir, repo) = init_repo();
    commit_files(
        &repo,
        &[("a.txt", "1\n"), ("img.bin", "\0\x01\x02")],
        "first",
    );
    let second = commit_files(&repo, &[("a.txt", "1\n2\n")], "second\n\nbody");
    fs::create_dir_all(dir.path().join("dir")).unwrap();
    commit_files(&repo, &[("dir/x.txt", "x\n")], "third");

    let commit_view = whole(
        &dir,
        GitRequest::Show {
            rev: second.to_string()[..8].to_string(),
            path: None,
        },
    );
    assert_eq!(commit_view.data["rev"], second.to_string());
    assert_eq!(commit_view.data["commit"]["subject"], "second");
    assert_eq!(commit_view.data["commit"]["message"], "second\n\nbody");
    assert_eq!(
        files(&commit_view),
        vec![("a.txt".into(), "MODIFIED".into())]
    );
    assert!(commit_view.data["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("+2\n"));
    assert!(
        commit_view.summary.starts_with(&format!(
            "{} second · 1 files, +1 −0",
            &second.to_string()[..12]
        )),
        "{}",
        commit_view.summary
    );

    let file = |rev: &str, path: &str| {
        call(
            &dir,
            &[root(&dir)],
            &["blocked.txt"],
            GitRequest::Show {
                rev: rev.into(),
                path: Some(path.into()),
            },
        )
    };
    let text = file("HEAD~1", "a.txt").unwrap();
    assert_eq!(text.data["text"], "1\n2\n");
    assert_eq!(text.data["exists"], true);
    assert_eq!(text.data["size"], 4);
    assert_eq!(text.paths, vec![root(&dir).join("a.txt")]);
    let older = file("HEAD~2", "a.txt").unwrap();
    assert_eq!(older.data["text"], "1\n");
    let binary = file("HEAD", "img.bin").unwrap();
    assert_eq!(
        (binary.data["binary"].clone(), binary.data["text"].clone()),
        (json!(true), json!(""))
    );
    assert!(binary.paths.is_empty());
    let missing = file("HEAD~2", "dir/x.txt").unwrap();
    assert_eq!(missing.data["exists"], false);
    assert!(missing.paths.is_empty());
    assert!(
        matches!(file("HEAD", "dir"), Err(GitError::Failed(text)) if text.contains("is a directory"))
    );
    assert!(matches!(
        file("HEAD", "blocked.txt"),
        Err(GitError::Denied(_))
    ));
}

#[test]
fn show_does_not_load_a_file_over_the_byte_limit() {
    let (dir, repo) = init_repo();
    let big = "z".repeat(MAX_FILE_BYTES + 10);
    commit_files(&repo, &[("big.txt", &big)], "big");
    let output = whole(
        &dir,
        GitRequest::Show {
            rev: "HEAD".into(),
            path: Some("big.txt".into()),
        },
    );
    assert_eq!(output.data["truncated"], true);
    assert_eq!(output.data["text"], "");
    assert_eq!(output.data["size"], MAX_FILE_BYTES + 10);
}

#[test]
fn branches_are_listed_from_the_rail_data() {
    let (dir, repo) = init_repo();
    let first = commit_files(&repo, &[("a.txt", "1\n")], "first");
    repo.branch("feature", &repo.find_commit(first).unwrap(), false)
        .unwrap();
    let output = whole(&dir, GitRequest::Branches);
    let names = output.data["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(names.contains(&"feature".to_string()));
    assert_eq!(names.len(), 2);
    assert_eq!(output.data["truncated"], false);
    assert!(output.paths.is_empty());
}

#[test]
fn a_partial_scope_limits_listings_history_and_paths() {
    let (dir, repo) = init_repo();
    commit_files(
        &repo,
        &[("sub/in.txt", "1\n"), ("outside.txt", "1\n")],
        "both",
    );
    commit_files(&repo, &[("outside.txt", "2\n")], "outside only");
    let inside = commit_files(&repo, &[("sub/in.txt", "2\n")], "inside only");
    write(dir.path(), "sub/in.txt", "3\n");
    write(dir.path(), "outside.txt", "3\n");
    write(dir.path(), "new_outside.txt", "n\n");
    write(dir.path(), "sub/new_inside.txt", "n\n");
    let folders = [root(&dir).join("sub")];

    let status = call(&dir, &folders, &[], GitRequest::Status).unwrap();
    assert_eq!(status.data["changes"].as_array().unwrap().len(), 1);
    assert_eq!(status.data["changes"][0]["path"], "sub/in.txt");
    assert_eq!(status.data["untracked"], json!(["sub/new_inside.txt"]));
    assert_eq!(status.data["withheld_count"], 0);

    let diff = call(&dir, &folders, &[], diff_request(Some("HEAD~2"), false)).unwrap();
    assert_eq!(files(&diff), vec![("sub/in.txt".into(), "MODIFIED".into())]);

    let log = call(
        &dir,
        &folders,
        &[],
        GitRequest::Log {
            n: 10,
            rev: None,
            paths: vec![],
        },
    )
    .unwrap();
    let subjects = log.data["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["subject"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(subjects, vec!["inside only", "both"]);
    assert_eq!(
        log.data["commits"][1]["files"],
        json!([{"path":"sub/in.txt","status":"ADDED"}])
    );

    assert!(matches!(
        call(&dir, &folders, &[], GitRequest::Show { rev: "HEAD~1".into(), path: None }),
        Err(GitError::Failed(text)) if text.contains("changes no path inside the chat's workspace folders")
    ));
    let shown = call(
        &dir,
        &folders,
        &[],
        GitRequest::Show {
            rev: inside.to_string(),
            path: None,
        },
    )
    .unwrap();
    assert_eq!(
        files(&shown),
        vec![("sub/in.txt".into(), "MODIFIED".into())]
    );
    assert_eq!(
        call(
            &dir,
            &folders,
            &[],
            GitRequest::Show {
                rev: "HEAD".into(),
                path: Some("outside.txt".into())
            }
        )
        .err(),
        Some(GitError::Type(
            "git(): path 'outside.txt' is outside the chat's workspace folders".into()
        ))
    );
    let elsewhere = tempfile::tempdir().unwrap();
    assert!(matches!(
        call(&dir, &[elsewhere.path().to_path_buf()], &[], GitRequest::Status),
        Err(GitError::Failed(text)) if text.contains("is outside the chat's workspace folders")
    ));
}

#[test]
fn the_repository_holding_project_root_is_read_within_the_workspace_folders() {
    let (dir, repo) = init_repo();
    commit_files(
        &repo,
        &[("project/src/a.txt", "1\n"), ("private/notes.txt", "n\n")],
        "init",
    );
    write(dir.path(), "project/src/a.txt", "2\n");
    write(dir.path(), "private/notes.txt", "n2\n");
    let project = root(&dir).join("project");
    let may_read = |_: &Path| true;
    let cancel = AtomicBool::new(false);
    let context = GitContext {
        root: &project.join("src"),
        folders: std::slice::from_ref(&project),
        may_read: &may_read,
        cancel: &cancel,
    };
    let output = run(&diff_request(None, false), &context).unwrap();
    assert_eq!(output.data["root"], json!(root(&dir).to_string_lossy()));
    assert_eq!(
        files(&output),
        vec![("project/src/a.txt".into(), "MODIFIED".into())]
    );
    let status = run(&GitRequest::Status, &context).unwrap();
    assert!(!status.data.to_string().contains("private"));
}

#[test]
fn bare_unborn_shallow_and_missing_repositories_give_stable_answers() {
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(
        call(&plain, &[root(&plain)], &[], GitRequest::Status).err(),
        Some(GitError::Failed(format!(
            "not a git repository: {}",
            plain.path().to_string_lossy()
        )))
    );

    let bare = tempfile::tempdir().unwrap();
    git2::Repository::init_bare(bare.path()).unwrap();
    assert!(matches!(
        call(&bare, &[root(&bare)], &[], GitRequest::Status),
        Err(GitError::Failed(text)) if text.starts_with("bare repository without a working tree")
    ));

    let (dir, _repo) = init_repo();
    write(dir.path(), "new.txt", "n\n");
    let status = whole(&dir, GitRequest::Status);
    assert_eq!(status.data["unborn"], true);
    assert_eq!(status.data["untracked"], json!(["new.txt"]));
    assert!(status.data["branch"].is_string());
    assert_eq!(
        call(
            &dir,
            &[root(&dir)],
            &[],
            GitRequest::Log {
                n: 5,
                rev: None,
                paths: vec![]
            }
        )
        .err(),
        Some(GitError::Failed("the repository has no commits yet".into()))
    );
    assert_eq!(
        files(&whole(&dir, diff_request(None, true))),
        Vec::<(String, String)>::new()
    );

    let (dir, repo) = init_repo();
    let root_commit = commit_files(&repo, &[("a.txt", "1\n")], "root");
    let main = repo.head().unwrap().shorthand().unwrap().to_string();
    let middle = commit_files(&repo, &[("a.txt", "2\n")], "middle");
    commit_files(&repo, &[("a.txt", "3\n")], "tip");
    repo.branch("topic", &repo.find_commit(root_commit).unwrap(), false)
        .unwrap();
    fs::write(dir.path().join(".git/shallow"), format!("{middle}\n")).unwrap();
    let repo = git2::Repository::open(dir.path()).unwrap();
    assert!(repo.is_shallow());
    let log = whole(
        &dir,
        GitRequest::Log {
            n: 10,
            rev: None,
            paths: vec![],
        },
    );
    assert_eq!(log.data["shallow"], true);
    assert_eq!(log.data["commits"].as_array().unwrap().len(), 2);
    assert!(matches!(
        call(&dir, &[root(&dir)], &[], diff_request(Some(&format!("{main}...topic")), false)),
        Err(GitError::Failed(text)) if text.contains("no merge base") && text.contains("shallow repository")
    ));
}

#[cfg(unix)]
#[test]
fn reads_run_no_configured_program_and_leave_the_index_untouched() {
    let (dir, repo) = init_repo();
    let marker_dir = tempfile::tempdir().unwrap();
    let marker = |name: &str| marker_dir.path().join(name);
    commit_files(&repo, &[("a.txt", "1\n"), ("f.x", "filtered\n")], "init");
    let mut config = repo.config().unwrap();
    let touch = |name: &str| format!("touch {}", marker(name).display());
    config.set_str("diff.external", &touch("external")).unwrap();
    config
        .set_str("core.fsmonitor", &touch("fsmonitor"))
        .unwrap();
    config.set_str("core.pager", &touch("pager")).unwrap();
    config
        .set_str("diff.x.textconv", &touch("textconv"))
        .unwrap();
    config.set_str("filter.x.clean", &touch("clean")).unwrap();
    config.set_str("filter.x.smudge", &touch("smudge")).unwrap();
    config
        .set_str("alias.status", &format!("!{}", touch("alias")))
        .unwrap();
    write(dir.path(), ".gitattributes", "*.x diff=x filter=x\n");
    write(dir.path(), "a.txt", "2\n");
    write(dir.path(), "f.x", "changed\n");
    let index_path = dir.path().join(".git/index");
    let index_before = fs::read(&index_path).unwrap();
    let modified_before = fs::metadata(&index_path).unwrap().modified().unwrap();

    for request in [
        GitRequest::Status,
        diff_request(None, false),
        diff_request(None, true),
        diff_request(Some("HEAD"), false),
        GitRequest::Log {
            n: 5,
            rev: None,
            paths: vec![],
        },
        GitRequest::Show {
            rev: "HEAD".into(),
            path: None,
        },
        GitRequest::Show {
            rev: "HEAD".into(),
            path: Some("f.x".into()),
        },
        GitRequest::Branches,
    ] {
        whole(&dir, request);
        assert!(!dir.path().join(".git/index.lock").exists());
    }
    for name in [
        "external",
        "fsmonitor",
        "pager",
        "textconv",
        "clean",
        "smudge",
        "alias",
    ] {
        assert!(!marker(name).exists(), "{name} ran");
    }
    assert_eq!(fs::read(&index_path).unwrap(), index_before);
    assert_eq!(
        fs::metadata(&index_path).unwrap().modified().unwrap(),
        modified_before
    );
}

#[test]
fn submodules_are_skipped() {
    let (dir, repo) = init_repo();
    let first = commit_files(&repo, &[("a.txt", "1\n")], "init");
    let inner = git2::Repository::init(dir.path().join("vendor/lib")).unwrap();
    let mut inner_config = inner.config().unwrap();
    inner_config.set_str("user.name", "Inner").unwrap();
    inner_config
        .set_str("user.email", "inner@example.com")
        .unwrap();
    commit_files(&inner, &[("inner.txt", "inside the submodule\n")], "inner");
    let mut index = repo.index().unwrap();
    index
        .add(&git2::IndexEntry {
            ctime: git2::IndexTime::new(0, 0),
            mtime: git2::IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: 0o160000,
            uid: 0,
            gid: 0,
            file_size: 0,
            id: first,
            flags: 0,
            flags_extended: 0,
            path: b"vendor/lib".to_vec(),
        })
        .unwrap();
    index.write().unwrap();
    commit(&repo, "add submodule");
    write(
        dir.path(),
        "vendor/lib/inner.txt",
        "changed inside the submodule\n",
    );

    let status = whole(&dir, GitRequest::Status);
    assert!(
        !status.data.to_string().contains("vendor/lib"),
        "{}",
        status.data
    );
    let diff = whole(&dir, diff_request(Some("HEAD~1"), false));
    assert!(
        !diff.data.to_string().contains("vendor/lib"),
        "{}",
        diff.data
    );
    assert!(matches!(
        call(&dir, &[root(&dir)], &[], GitRequest::Show { rev: "HEAD".into(), path: Some("vendor/lib".into()) }),
        Err(GitError::Failed(text)) if text.contains("submodule")
    ));
}

#[test]
fn a_set_cancel_flag_stops_the_call() {
    let (dir, repo) = init_repo();
    commit_files(&repo, &[("a.txt", "1\n")], "first");
    write(dir.path(), "a.txt", "2\n");
    let base = root(&dir);
    let may_read = |_: &Path| true;
    let cancel = AtomicBool::new(true);
    let context = GitContext {
        root: dir.path(),
        folders: std::slice::from_ref(&base),
        may_read: &may_read,
        cancel: &cancel,
    };
    for request in [
        GitRequest::Log {
            n: 5,
            rev: None,
            paths: vec![],
        },
        diff_request(None, false),
    ] {
        assert_eq!(run(&request, &context).err(), Some(GitError::Cancelled));
    }
}

#[test]
fn renames_are_not_detected_when_a_candidate_is_over_the_blob_budget() {
    let (dir, repo) = init_repo();
    let big = "b\n".repeat(MAX_DIFF_BLOB_BYTES as usize / 2 + 1);
    commit_files(
        &repo,
        &[
            ("big.txt", big.as_str()),
            ("small.txt", "one\ntwo\nthree\n"),
        ],
        "init",
    );
    fs::rename(dir.path().join("small.txt"), dir.path().join("moved.txt")).unwrap();
    stage(&repo, &["small.txt", "moved.txt"]);
    assert_eq!(
        files(&whole(&dir, diff_request(None, true))),
        vec![("moved.txt".to_string(), "RENAMED".to_string())]
    );
    fs::rename(dir.path().join("big.txt"), dir.path().join("big_moved.txt")).unwrap();
    stage(&repo, &["big.txt", "big_moved.txt"]);
    let mut listed = files(&whole(&dir, diff_request(None, true)));
    listed.sort();
    assert_eq!(
        listed,
        vec![
            ("big.txt".to_string(), "DELETED".to_string()),
            ("big_moved.txt".to_string(), "ADDED".to_string()),
            ("moved.txt".to_string(), "ADDED".to_string()),
            ("small.txt".to_string(), "DELETED".to_string()),
        ]
    );
}

#[test]
fn status_reads_only_the_scope_so_outside_entries_do_not_fill_its_cap() {
    let (dir, repo) = init_repo();
    commit_files(
        &repo,
        &[("inside/a.txt", "a\n"), ("outside/b.txt", "b\n")],
        "init",
    );
    for i in 0..crate::read::MAX_STATUS_ENTRIES + 1 {
        write(dir.path(), &format!("outside/new/{i:05}.txt"), "x");
    }
    write(dir.path(), "inside/a.txt", "a2\n");
    let output = call(&dir, &[root(&dir).join("inside")], &[], GitRequest::Status).unwrap();
    assert_eq!(output.data["truncated"], false);
    assert_eq!(
        output.data["changes"],
        json!([{"path":"inside/a.txt","status":"MODIFIED","staged":false,"unstaged":true}])
    );
    assert_eq!(output.data["untracked"], json!([]));
}

#[cfg(unix)]
#[test]
fn names_that_cannot_be_checked_as_they_are_are_withheld() {
    use std::os::unix::ffi::OsStrExt;
    let (dir, repo) = init_repo();
    let odd = [
        std::ffi::OsStr::from_bytes(b"a\\secret.txt"),
        std::ffi::OsStr::from_bytes(b"b\xff.txt"),
    ];
    for name in odd {
        fs::write(dir.path().join(name), "secret\n").unwrap();
    }
    write(dir.path(), "plain.txt", "plain\n");
    let mut index = repo.index().unwrap();
    for name in odd {
        index.add_path(Path::new(name)).unwrap();
    }
    index.add_path(Path::new("plain.txt")).unwrap();
    index.write().unwrap();
    commit(&repo, "init");
    for name in odd {
        fs::write(dir.path().join(name), "secret changed\n").unwrap();
    }
    write(dir.path(), "plain.txt", "plain changed\n");
    let blocked = [root(&dir).join(odd[0]), root(&dir).join(odd[1])];
    let may_read = move |path: &Path| !blocked.iter().any(|b| b == path);
    let cancel = AtomicBool::new(false);
    let folders = [root(&dir)];
    let context = GitContext {
        root: dir.path(),
        folders: &folders,
        may_read: &may_read,
        cancel: &cancel,
    };
    let diff = run(&diff_request(None, false), &context).unwrap();
    assert_eq!(
        files(&diff),
        vec![("plain.txt".to_string(), "MODIFIED".to_string())]
    );
    assert_eq!(diff.data["withheld_count"], 2);
    assert!(!diff.data.to_string().contains("secret"), "{}", diff.data);
    let log = run(
        &GitRequest::Log {
            n: 1,
            rev: None,
            paths: vec![],
        },
        &context,
    )
    .unwrap();
    assert_eq!(
        log.data["commits"][0]["files"],
        json!([{"path":"plain.txt","status":"ADDED"}])
    );
    let status = run(&GitRequest::Status, &context).unwrap();
    assert_eq!(
        status.data["changes"],
        json!([{"path":"plain.txt","status":"MODIFIED","staged":false,"unstaged":true}])
    );
}

#[test]
fn a_repository_with_alternates_is_refused_and_a_linked_worktree_is_read() {
    let (other_dir, other) = init_repo();
    let foreign = commit_files(&other, &[("a.txt", "foreign text\n")], "foreign");
    let (dir, repo) = init_repo();
    commit_files(&repo, &[("a.txt", "local\n")], "local");
    let alternates = dir.path().join(".git/objects/info/alternates");
    fs::create_dir_all(alternates.parent().unwrap()).unwrap();
    fs::write(&alternates, " \n").unwrap();
    assert_eq!(whole(&dir, GitRequest::Status).data["changes"], json!([]));
    fs::write(
        &alternates,
        format!("{}\n", other_dir.path().join(".git/objects").display()),
    )
    .unwrap();
    let refused = |path: &Path| {
        GitError::Failed(format!(
            "the repository at {} uses an alternate object store (objects/info/alternates); git() does not read such repositories",
            path.to_string_lossy()
        ))
    };
    for request in [
        GitRequest::Status,
        diff_request(None, false),
        GitRequest::Log {
            n: 5,
            rev: Some(foreign.to_string()),
            paths: vec![],
        },
        GitRequest::Show {
            rev: foreign.to_string(),
            path: Some("a.txt".into()),
        },
        GitRequest::Branches,
    ] {
        assert_eq!(
            call(&dir, &[root(&dir)], &[], request.clone()).err(),
            Some(refused(dir.path())),
            "{request:?}"
        );
    }

    let (main_dir, main) = init_repo();
    commit_files(&main, &[("a.txt", "1\n")], "init");
    let holder = tempfile::tempdir().unwrap();
    let linked = holder.path().join("linked");
    main.worktree("linked", &linked, None).unwrap();
    write(&linked, "a.txt", "2\n");
    let status = read_at(&linked, GitRequest::Status).unwrap();
    assert_eq!(
        status.data["changes"],
        json!([{"path":"a.txt","status":"MODIFIED","staged":false,"unstaged":true}])
    );
    assert_eq!(status.data["branch"], "linked");
    let diff = read_at(&linked, diff_request(None, false)).unwrap();
    assert!(diff.data["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("-1\n+2\n"));
    let log = read_at(
        &linked,
        GitRequest::Log {
            n: 5,
            rev: None,
            paths: vec![],
        },
    )
    .unwrap();
    assert_eq!(log.data["commits"][0]["subject"], "init");
    let shown = read_at(
        &linked,
        GitRequest::Show {
            rev: "HEAD".into(),
            path: Some("a.txt".into()),
        },
    )
    .unwrap();
    assert_eq!(shown.data["text"], "1\n");
    assert!(read_at(&linked, GitRequest::Branches).is_ok());
    let shared = main_dir.path().join(".git/objects/info/alternates");
    fs::create_dir_all(shared.parent().unwrap()).unwrap();
    fs::write(
        &shared,
        format!("{}\n", other_dir.path().join(".git/objects").display()),
    )
    .unwrap();
    assert_eq!(
        read_at(&linked, GitRequest::Status).err(),
        Some(refused(&linked))
    );
}

#[test]
fn each_side_of_a_range_is_checked_on_its_own() {
    for (subcommand, rev) in [
        ("diff", "HEAD..-x"),
        ("diff", "HEAD...--output=/tmp/x"),
        ("diff", "..-x"),
        ("log", "HEAD..-n"),
        ("log", "--all..HEAD"),
    ] {
        assert_eq!(
            parse(json!({"subcommand": subcommand, "rev": rev})).unwrap_err(),
            "git(): Revision must not start with '-'",
            "{rev}"
        );
    }
    for rev in ["HEAD..v1^{tree}", "HEAD@{1}..HEAD", "HEAD...a:b"] {
        assert!(parse(json!({"subcommand":"diff","rev":rev}))
            .unwrap_err()
            .contains("must name a commit"));
    }
    for rev in ["..HEAD", "HEAD..", "a-b..c-d", "HEAD~1...HEAD"] {
        assert!(
            parse(json!({"subcommand":"diff","rev":rev})).is_ok(),
            "{rev}"
        );
    }
}

#[test]
fn a_revision_missing_from_a_shallow_repository_says_that_it_is_shallow() {
    let (dir, repo) = init_repo();
    commit_files(&repo, &[("a.txt", "1\n")], "root");
    let middle = commit_files(&repo, &[("a.txt", "2\n")], "middle");
    commit_files(&repo, &[("a.txt", "3\n")], "tip");
    assert_eq!(
        call(&dir, &[root(&dir)], &[], diff_request(Some("nope"), false)).err(),
        Some(GitError::Failed("revision not found: nope".into()))
    );
    fs::write(dir.path().join(".git/shallow"), format!("{middle}\n")).unwrap();
    let shallow = " (shallow repository: older history is missing)";
    for (request, rev) in [
        (diff_request(Some("nope"), false), "nope"),
        (diff_request(Some("HEAD~2"), false), "HEAD~2"),
        (
            GitRequest::Show {
                rev: "HEAD~2".into(),
                path: None,
            },
            "HEAD~2",
        ),
        (
            GitRequest::Log {
                n: 5,
                rev: Some("v0..HEAD".into()),
                paths: vec![],
            },
            "v0",
        ),
    ] {
        assert_eq!(
            call(&dir, &[root(&dir)], &[], request).err(),
            Some(GitError::Failed(format!(
                "revision not found: {rev}{shallow}"
            )))
        );
    }
}

#[test]
fn a_blocked_path_argument_is_refused_and_hidden_entries_never_select_a_commit() {
    let (dir, repo) = init_repo();
    commit_files(
        &repo,
        &[
            ("public.txt", "p\n"),
            ("secret.txt", "s\n"),
            ("docs/a.txt", "a\n"),
            ("docs/secret.txt", "d\n"),
        ],
        "init",
    );
    commit_files(&repo, &[("secret.txt", "s2\n")], "neutral subject");
    commit_files(&repo, &[("docs/secret.txt", "d2\n")], "docs hidden only");
    commit_files(&repo, &[("docs/a.txt", "a2\n")], "docs visible");
    commit_files(&repo, &[("public.txt", "p2\n")], "public");
    write(dir.path(), "secret.txt", "s3\n");
    write(dir.path(), "docs/secret.txt", "d3\n");
    let blocked = ["secret.txt", "docs/secret.txt", "never/existed.txt"];
    let ask = |folders: &[PathBuf], request: GitRequest| call(&dir, folders, &blocked, request);
    let everything = [root(&dir)];

    for path in blocked {
        let refused = GitError::Denied(format!(
            "git(): reading {path} is not allowed by the privacy policy or the engine's rules"
        ));
        for request in [
            GitRequest::Log {
                n: 10,
                rev: None,
                paths: vec![path.into()],
            },
            GitRequest::Diff {
                rev: None,
                paths: vec![path.into()],
                staged: false,
            },
            GitRequest::Diff {
                rev: Some("HEAD~4".into()),
                paths: vec!["public.txt".into(), path.into()],
                staged: false,
            },
            GitRequest::Show {
                rev: "HEAD".into(),
                path: Some(path.into()),
            },
        ] {
            assert_eq!(
                ask(&everything, request.clone()).err(),
                Some(refused.clone()),
                "{request:?}"
            );
        }
    }

    let subjects = |output: &GitOutput| {
        output.data["commits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["subject"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let in_docs = ask(
        &everything,
        GitRequest::Log {
            n: 10,
            rev: None,
            paths: vec!["docs".into()],
        },
    )
    .unwrap();
    assert_eq!(subjects(&in_docs), vec!["docs visible", "init"]);
    assert_eq!(in_docs.data["withheld_count"], 1);
    assert_eq!(in_docs.data["commits"][1]["withheld_count"], 1);

    let scoped = ask(
        &[root(&dir).join("docs")],
        GitRequest::Log {
            n: 10,
            rev: None,
            paths: vec![],
        },
    )
    .unwrap();
    assert_eq!(subjects(&scoped), vec!["docs visible", "init"]);

    let all = ask(
        &everything,
        GitRequest::Log {
            n: 10,
            rev: None,
            paths: vec![],
        },
    )
    .unwrap();
    assert_eq!(
        subjects(&all),
        vec![
            "public",
            "docs visible",
            "docs hidden only",
            "neutral subject",
            "init"
        ]
    );
    assert_eq!(all.data["commits"][3]["files"], json!([]));
    assert_eq!(all.data["commits"][3]["withheld_count"], 1);
}

#[test]
fn links_are_listed_without_their_target() {
    let (dir, repo) = init_repo();
    commit_files(&repo, &[("a.txt", "a\n")], "init");
    let mut index = repo.index().unwrap();
    let link = git2::IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: 0o120000,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: git2::Oid::zero(),
        flags: 0,
        flags_extended: 0,
        path: b"ln".to_vec(),
    };
    index
        .add_frombuffer(&link, b"../target-secret-name")
        .unwrap();
    index.write().unwrap();
    commit(&repo, "add a link");
    write(dir.path(), "a.txt", "a2\n");

    for output in [
        whole(
            &dir,
            GitRequest::Show {
                rev: "HEAD".into(),
                path: None,
            },
        ),
        whole(&dir, diff_request(Some("HEAD~1"), true)),
        whole(&dir, diff_request(None, false)),
    ] {
        let entries = output.data["files"].as_array().unwrap();
        let ln = entries.iter().find(|f| f["path"] == "ln").unwrap();
        assert_eq!(ln["link"], true);
        assert_eq!(ln["patch"], Value::Null);
        assert_eq!(ln["patch_truncated"], false);
        assert!(
            !output.data.to_string().contains("target-secret-name"),
            "{}",
            output.data
        );
        assert!(!output.paths.contains(&root(&dir).join("ln")));
        if let Some(a) = entries.iter().find(|f| f["path"] == "a.txt") {
            assert_eq!(a["link"], false);
            assert!(a["patch"].as_str().unwrap().contains("+a2"));
        }
    }
    let file = whole(
        &dir,
        GitRequest::Show {
            rev: "HEAD".into(),
            path: Some("ln".into()),
        },
    );
    assert_eq!(
        (
            file.data["exists"].clone(),
            file.data["link"].clone(),
            file.data["text"].clone()
        ),
        (json!(true), json!(true), json!(""))
    );
    assert!(!file.data.to_string().contains("target-secret-name"));
    assert!(file.paths.is_empty());
    let plain = whole(
        &dir,
        GitRequest::Show {
            rev: "HEAD".into(),
            path: Some("a.txt".into()),
        },
    );
    assert_eq!(plain.data["link"], false);
    assert_eq!(plain.data["text"], "a\n");
}

#[cfg(unix)]
#[test]
fn the_gate_checks_the_path_and_the_file_it_resolves_to() {
    use std::os::unix::fs::symlink;
    let (dir, repo) = init_repo();
    commit_files(
        &repo,
        &[
            ("alias.txt", "was a plain file\n"),
            ("open_alias.txt", "readable history\n"),
            ("secret.txt", "s\n"),
            ("public.txt", "p\n"),
            ("old/x.txt", "x\n"),
        ],
        "init",
    );
    for (name, target) in [
        ("alias.txt", "secret.txt"),
        ("open_alias.txt", "public.txt"),
    ] {
        fs::remove_file(dir.path().join(name)).unwrap();
        symlink(target, dir.path().join(name)).unwrap();
    }
    fs::remove_dir_all(dir.path().join("old")).unwrap();
    fs::create_dir_all(dir.path().join("private")).unwrap();
    symlink("private", dir.path().join("old")).unwrap();
    let blocked = ["secret.txt", "private/x.txt"];
    let everything = [root(&dir)];

    for path in ["alias.txt", "old/x.txt"] {
        let refused = GitError::Denied(format!(
            "git(): reading {path} is not allowed by the privacy policy or the engine's rules"
        ));
        for request in [
            GitRequest::Show {
                rev: "HEAD".into(),
                path: Some(path.into()),
            },
            GitRequest::Log {
                n: 5,
                rev: None,
                paths: vec![path.into()],
            },
        ] {
            assert_eq!(
                call(&dir, &everything, &blocked, request).err(),
                Some(refused.clone())
            );
        }
    }
    let readable = call(
        &dir,
        &everything,
        &blocked,
        GitRequest::Show {
            rev: "HEAD".into(),
            path: Some("open_alias.txt".into()),
        },
    )
    .unwrap();
    assert_eq!(readable.data["text"], "readable history\n");

    let status = call(&dir, &everything, &blocked, GitRequest::Status).unwrap();
    let listed = status.data["changes"].to_string();
    assert!(listed.contains("\"open_alias.txt\""), "{listed}");
    assert!(!listed.contains("\"alias.txt\""), "{listed}");
    assert!(!listed.contains("old/x.txt"), "{listed}");
    assert_eq!(status.data["withheld_count"], 2);
    let diff = call(&dir, &everything, &blocked, diff_request(None, false)).unwrap();
    assert_eq!(
        files(&diff),
        vec![("open_alias.txt".into(), "TYPECHANGE".into())]
    );
    assert_eq!(diff.data["withheld_count"], 2);
}

#[test]
fn branches_list_at_most_the_ref_cap_of_each_kind() {
    let (dir, repo) = init_repo();
    let first = commit_files(&repo, &[("a.txt", "1\n")], "first");
    let target = repo.find_object(first, None).unwrap();
    for i in 0..=MAX_REFS {
        repo.tag_lightweight(&format!("v{i:05}"), &target, false)
            .unwrap();
    }
    let output = whole(&dir, GitRequest::Branches);
    let tags = output.data["tags"].as_array().unwrap();
    assert_eq!(tags.len(), MAX_REFS);
    assert_eq!(tags[0]["name"], "v00000");
    assert_eq!(output.data["truncated"], true);
    assert!(
        output.summary.starts_with(&format!(
            "1 branches, 0 remote, {MAX_REFS} tags (truncated); current "
        )),
        "{}",
        output.summary
    );
}
