use super::*;
use crate::test_repo::{commit_files, init_repo, stage, write};
use std::fs;

#[test]
fn diff_for_repo_preserves_content_prefixes_and_cleans_metadata_origins() {
    let (dir, repo) = init_repo();
    let base = (1..=24)
        .map(|line| format!("line {line:02}\n"))
        .collect::<String>();
    commit_files(&repo, &[("file.txt", &base)], "Initial commit");

    let modified = base
        .replace("line 02\n", "changed 02\n")
        .replace("line 22\n", "changed 22\n");
    fs::write(dir.path().join("file.txt"), modified).unwrap();
    fs::write(
        dir.path().join("image.bin"),
        b"\x89PNG\r\n\x1a\n\0binary payload",
    )
    .unwrap();

    let patch = diff_for_repo(dir.path(), None, false).unwrap().patch;
    let lines = patch.lines().collect::<Vec<_>>();

    assert!(lines.iter().any(|line| *line == "-line 02"));
    assert!(lines.iter().any(|line| *line == "+changed 02"));
    assert!(lines.iter().any(|line| *line == " line 03"));
    assert!(lines.iter().filter(|line| line.starts_with("@@ ")).count() >= 2);
    assert!(lines
        .iter()
        .any(|line| *line == "diff --git a/image.bin b/image.bin"));
    assert!(lines
        .iter()
        .any(|line| *line == "Binary files /dev/null and b/image.bin differ"));
    assert!(!patch.contains("Fdiff"));
    assert!(!patch.contains("BBinary"));
    assert!(!patch.contains("H@@"));
}

#[test]
fn rail_diffs_keep_untracked_content_and_staged_changes_apart() {
    let (dir, repo) = init_repo();
    commit_files(&repo, &[("a.txt", "a\n"), ("b.txt", "b\n")], "init");
    write(dir.path(), "a.txt", "a2\n");
    stage(&repo, &["a.txt"]);
    write(dir.path(), "b.txt", "b2\n");
    write(dir.path(), "new.txt", "brand new\n");

    let unstaged = diff_for_repo(dir.path(), None, false).unwrap().patch;
    assert!(unstaged.contains("+b2"), "{unstaged}");
    assert!(unstaged.contains("+brand new"), "{unstaged}");
    assert!(!unstaged.contains("+a2"), "{unstaged}");

    let staged = diff_for_repo(dir.path(), None, true).unwrap().patch;
    assert!(staged.contains("+a2"), "{staged}");
    assert!(!staged.contains("brand new"), "{staged}");

    let against_head = diff_for_repo(dir.path(), None, DiffBase::HeadToWorkdir)
        .unwrap()
        .patch;
    for needle in ["+a2", "+b2", "+brand new"] {
        assert!(against_head.contains(needle), "{needle}: {against_head}");
    }

    let only_b = diff_for_repo(dir.path(), Some("b.txt"), false)
        .unwrap()
        .patch;
    assert!(
        only_b.contains("+b2") && !only_b.contains("brand new"),
        "{only_b}"
    );
    assert_eq!(
        diff_for_repo(dir.path(), Some("../x"), false).unwrap_err(),
        "Git paths must be non-empty paths relative to the repository"
    );
}

#[test]
fn rail_status_lists_changes_untracked_and_staged_renames() {
    let (dir, repo) = init_repo();
    commit_files(
        &repo,
        &[("keep.txt", "keep\n"), ("old.txt", "same content\n")],
        "init",
    );
    fs::rename(dir.path().join("old.txt"), dir.path().join("new.txt")).unwrap();
    stage(&repo, &["old.txt", "new.txt"]);
    write(dir.path(), "keep.txt", "changed\n");
    write(dir.path(), "dir/untracked.txt", "u\n");

    let status = status_for_repo(dir.path()).unwrap();
    let changes = status
        .changes
        .iter()
        .map(|c| {
            (
                c.relative_path.as_str(),
                c.old_path.as_deref(),
                c.status.as_str(),
                c.staged,
                c.unstaged,
            )
        })
        .collect::<Vec<_>>();
    assert!(
        changes.contains(&("keep.txt", None, "MODIFIED", false, true)),
        "{changes:?}"
    );
    assert!(
        changes.contains(&("new.txt", Some("old.txt"), "RENAMED", true, false)),
        "{changes:?}"
    );
    assert_eq!(
        status
            .untracked
            .iter()
            .map(|u| u.relative_path.as_str())
            .collect::<Vec<_>>(),
        vec!["dir/untracked.txt"]
    );
    let staged = status
        .staged
        .iter()
        .map(|c| (c.relative_path.as_str(), c.status.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(staged, vec![("new.txt", "ADDED"), ("old.txt", "DELETED")]);
    assert!(status.untracked_included);
    assert_eq!(status.repo_state, "clean");
    assert!(status.head_oid.is_some());
}

#[test]
fn rail_log_filters_by_path_and_refuses_unknown_references() {
    let (dir, repo) = init_repo();
    commit_files(&repo, &[("a.txt", "1\n")], "first a");
    commit_files(&repo, &[("b.txt", "1\n")], "only b");
    commit_files(&repo, &[("a.txt", "2\n")], "second a");

    let filters = LogFilters {
        path: Some(PathBuf::from("a.txt")),
        ..Default::default()
    };
    let log = log_for_repo(dir.path(), 10, 0, &filters).unwrap();
    assert_eq!(
        log.commits
            .iter()
            .map(|c| c.message_first_line.as_str())
            .collect::<Vec<_>>(),
        vec!["second a", "first a"]
    );
    assert_eq!(log.commits[0].refs[0].name, "HEAD");

    let missing = LogFilters {
        reference: Some("nope".into()),
        ..Default::default()
    };
    assert_eq!(
        log_for_repo(dir.path(), 10, 0, &missing).unwrap_err(),
        GitReadError::NotFound("Reference not found: nope".into())
    );
}

#[test]
fn rail_commit_details_check_the_oid_and_list_files() {
    let (dir, repo) = init_repo();
    commit_files(&repo, &[("a.txt", "1\n")], "first");
    let second = commit_files(&repo, &[("a.txt", "2\n"), ("b.txt", "b\n")], "second");

    let details = commit_details_for_repo(dir.path(), &second.to_string()[..10]).unwrap();
    assert_eq!(details.oid, second.to_string());
    assert_eq!(details.message, "second");
    assert_eq!(
        details
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.status.as_str()))
            .collect::<Vec<_>>(),
        vec![("a.txt", "MODIFIED"), ("b.txt", "ADDED")]
    );
    assert_eq!(
        commit_details_for_repo(dir.path(), "xyz").unwrap_err(),
        GitReadError::BadRequest("Commit oid must be 4-40 hex characters".into())
    );
    assert_eq!(
        commit_details_for_repo(dir.path(), "0000000").unwrap_err(),
        GitReadError::NotFound("Commit not found".into())
    );
}

#[test]
fn rail_branches_report_upstream_distance_and_tags() {
    let (dir, repo) = init_repo();
    let first = commit_files(&repo, &[("a.txt", "1\n")], "first");
    commit_files(&repo, &[("a.txt", "2\n")], "second");
    let head = repo.head().unwrap();
    let branch = head.shorthand().unwrap().to_string();
    repo.reference("refs/remotes/origin/main", first, true, "test")
        .unwrap();
    let mut config = repo.config().unwrap();
    config
        .set_str(&format!("branch.{branch}.remote"), "origin")
        .unwrap();
    config
        .set_str(&format!("branch.{branch}.merge"), "refs/heads/main")
        .unwrap();
    repo.remote("origin", "https://example.com/repo.git")
        .unwrap();
    repo.tag_lightweight("v1", &repo.find_object(first, None).unwrap(), false)
        .unwrap();

    let branches = branches_for_repo(dir.path()).unwrap();
    assert_eq!(branches.current.as_deref(), Some(branch.as_str()));
    let local = &branches.branches[0];
    assert_eq!(local.upstream.as_deref(), Some("origin/main"));
    assert_eq!((local.ahead, local.behind), (Some(1), Some(0)));
    assert_eq!(branches.remote_branches[0].short_name, "main");
    assert_eq!(branches.tags[0].name, "v1");
    assert_eq!(branches.tags[0].target_oid, first.to_string());

    let head = head_status(&repo);
    assert_eq!(head.upstream.as_deref(), Some("origin/main"));
    assert_eq!((head.ahead, head.behind), (Some(1), Some(0)));
    assert!(!head.head_detached);
}

#[test]
fn file_payload_marks_binary_and_cuts_text_at_a_character_boundary() {
    assert_eq!(
        file_payload(b"text\n".to_vec(), 5),
        (false, false, "text\n".into())
    );
    assert_eq!(
        file_payload(b"a\0b".to_vec(), 3),
        (true, false, String::new())
    );
    let mut long = "é".repeat(MAX_FILE_BYTES / 2).into_bytes();
    long.insert(0, b'x');
    let size = long.len() as u64;
    let (binary, truncated, text) = file_payload(long, size);
    assert!(!binary && truncated);
    assert!(text.len() <= MAX_FILE_BYTES && text.starts_with('x'));
}

#[test]
fn rev_and_path_validators_keep_their_texts() {
    assert_eq!(
        validate_rev_input("-p").unwrap_err(),
        "Revision must not start with '-'"
    );
    assert_eq!(
        validate_rev_input("a b").unwrap_err(),
        "Revision must not contain whitespace or control characters"
    );
    assert!(validate_rev_input("HEAD~1").is_ok());
    for bad in ["", "../x", "/etc/passwd"] {
        assert!(repo_relative_paths(vec![bad.into()]).is_err(), "{bad}");
    }
    assert_eq!(
        repo_relative_paths(vec!["b".into(), "a".into(), "b".into()]).unwrap(),
        vec![PathBuf::from("a"), PathBuf::from("b")]
    );
}

#[test]
fn first_by_name_builds_in_name_order_and_stops_after_the_cap() {
    let candidates = ["e", "a", "d", "b", "c"]
        .iter()
        .map(|name| (name.to_string(), ()))
        .collect::<Vec<_>>();
    let mut built = Vec::new();
    let (kept, cut) = first_by_name(candidates.clone(), 2, |name, _| {
        built.push(name.clone());
        (name != "b").then_some(name)
    });
    assert_eq!(kept, vec!["a", "c"]);
    assert!(cut);
    assert_eq!(built, vec!["a", "b", "c", "d"]);
    let (kept, cut) = first_by_name(candidates, usize::MAX, |name, _| Some(name));
    assert_eq!(kept, vec!["a", "b", "c", "d", "e"]);
    assert!(!cut);
}

#[test]
fn branches_of_keeps_the_first_entries_of_each_kind_and_the_rail_keeps_all() {
    let (dir, repo) = init_repo();
    let first = commit_files(&repo, &[("a.txt", "1\n")], "first");
    let commit = repo.find_commit(first).unwrap();
    let head = repo.head().unwrap().shorthand().unwrap().to_string();
    for name in ["a1", "a2", "a3"] {
        repo.branch(name, &commit, false).unwrap();
        repo.tag_lightweight(&format!("t-{name}"), commit.as_object(), false)
            .unwrap();
        repo.reference(&format!("refs/remotes/origin/{name}"), first, true, "test")
            .unwrap();
    }
    let (capped, cut) = branches_of(&repo, dir.path(), 2).unwrap();
    assert!(cut);
    assert_eq!(
        capped
            .branches
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>(),
        vec!["a1", "a2"]
    );
    assert_eq!(capped.current.as_deref(), Some(head.as_str()));
    assert_eq!(
        capped
            .tags
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        vec!["t-a1", "t-a2"]
    );
    assert_eq!(
        capped
            .remote_branches
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>(),
        vec!["origin/a1", "origin/a2"]
    );
    let (whole, cut) = branches_of(&repo, dir.path(), usize::MAX).unwrap();
    assert!(!cut);
    assert_eq!(whole, branches_for_repo(dir.path()).unwrap());
    assert_eq!(whole.branches.len(), 4);
    assert_eq!(whole.tags.len(), 3);
    assert_eq!(whole.remote_branches.len(), 3);
}

#[test]
fn ref_labels_are_built_only_for_the_wanted_commits() {
    let (_dir, repo) = init_repo();
    let first = commit_files(&repo, &[("a.txt", "1\n")], "first");
    let second = commit_files(&repo, &[("a.txt", "2\n")], "second");
    repo.tag_lightweight("v1", &repo.find_object(first, None).unwrap(), false)
        .unwrap();
    let all = ref_labels_by_oid(&repo).unwrap();
    assert_eq!(all.len(), 2);
    let only_first = ref_labels_where(&repo, |oid| oid == first).unwrap();
    assert_eq!(only_first.len(), 1);
    assert_eq!(only_first[&first], all[&first]);
    let only_head = ref_labels_where(&repo, |oid| oid == second).unwrap();
    assert_eq!(only_head[&second], all[&second]);
    assert_eq!(only_head[&second][0].name, "HEAD");
}
