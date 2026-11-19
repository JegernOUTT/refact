use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::tools::tool_python::tests::{call, execute, fixture, with_tools};
use crate::tools::tools_description::Tool;

async fn run(ccx: Arc<AMutex<AtCommandsContext>>, code: &str) -> ChatMessage {
    let (_, messages) = crate::tools::tool_python::ToolPython::default()
        .tool_execute(
            ccx,
            &uuid::Uuid::new_v4().to_string(),
            &HashMap::from([("code".to_string(), json!(code))]),
        )
        .await
        .unwrap();
    match messages.into_iter().next().unwrap() {
        ContextEnum::ChatMessage(message) => message,
        _ => panic!("expected a message"),
    }
}

fn repo(root: &Path) -> git2::Repository {
    let repo = git2::Repository::init(root).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Test User").unwrap();
    config.set_str("user.email", "test@example.com").unwrap();
    repo
}

fn commit(repo: &git2::Repository, files: &[(&str, &str)], message: &str) -> git2::Oid {
    let root = repo.workdir().unwrap().to_path_buf();
    let mut index = repo.index().unwrap();
    for (path, content) in files {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, content).unwrap();
        index.add_path(Path::new(path)).unwrap();
    }
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = git2::Signature::now("Test User", "test@example.com").unwrap();
    let parents = repo
        .head()
        .ok()
        .and_then(|head| head.target())
        .and_then(|oid| repo.find_commit(oid).ok())
        .into_iter()
        .collect::<Vec<_>>();
    let parents = parents.iter().collect::<Vec<_>>();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parents,
    )
    .unwrap()
}

fn text(message: &ChatMessage) -> String {
    message.content.content_text_only()
}

fn calls(message: &ChatMessage) -> Vec<Value> {
    message.extra["python"]["calls"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[tokio::test]
async fn git_status_diff_and_show_through_the_interpreter() {
    let (dir, _app, ccx) = fixture().await;
    let project = dir.path().join("project");
    let repo = repo(&project);
    commit(&repo, &[("a.txt", "1\n"), ("b.txt", "b\n")], "first");
    commit(&repo, &[("a.txt", "2\n")], "second");
    std::fs::write(project.join("a.txt"), "3\n").unwrap();
    std::fs::write(project.join("new.txt"), "untracked\n").unwrap();
    let branch = repo.head().unwrap().shorthand().unwrap().to_string();
    let message = run(
        ccx,
        "s = await git(\"status\")\nd = await git(\"diff\", path=\"a.txt\")\nf = await git(\"show\", \"HEAD~1\", \"a.txt\")\nl = await git(\"log\", n=1)\nb = await git(\"branches\")\n[s[\"ok\"], [c[\"path\"] for c in s[\"data\"][\"changes\"]], s[\"data\"][\"untracked\"], d[\"data\"][\"files\"][0][\"patch\"], f[\"data\"][\"text\"], l[\"data\"][\"commits\"][0][\"subject\"], b[\"ok\"], b[\"data\"][\"current\"], [x[\"name\"] + \"@\" + x[\"tip_oid\"][:7] for x in b[\"data\"][\"branches\"]], b[\"data\"][\"truncated\"]]",
    )
    .await;
    assert_eq!(message.tool_failed, Some(false), "{}", text(&message));
    let result = text(&message);
    let tip = repo.head().unwrap().target().unwrap().to_string();
    let listed = format!("{branch}@{}", &tip[..7]);
    for needle in [
        "a.txt",
        "new.txt",
        "-2\\n+3\\n",
        "\"1\\n\"",
        "second",
        branch.as_str(),
        listed.as_str(),
    ] {
        assert!(result.contains(needle), "{needle}: {result}");
    }
    assert!(!result.contains("untracked\\n"), "{result}");
    let records = calls(&message);
    assert_eq!(records.len(), 5);
    assert!(records
        .iter()
        .all(|r| r["tool"] == "git" && r["status"] == "ok"));
    assert!(records[0]["args_preview"]
        .as_str()
        .unwrap()
        .contains("\"command\":\"status\""));
    assert!(records[1]["args_preview"]
        .as_str()
        .unwrap()
        .contains("\"command\":\"diff -- a.txt\""));
    assert!(records.iter().all(|r| r["diffs"] == json!([])));
    let run_dir = std::path::PathBuf::from(message.extra["python"]["run_dir"].as_str().unwrap());
    assert!(run_dir.join("calls/1_git.json").exists());
    let logged: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("calls/2_git.json")).unwrap()).unwrap();
    assert_eq!(logged["tool"], "git");
    assert_eq!(logged["kwargs"]["subcommand"], "diff");
}

#[tokio::test]
async fn git_refusals_raise_python_errors_and_run_nothing() {
    let (dir, _app, ccx) = fixture().await;
    let project = dir.path().join("project");
    let repo = repo(&project);
    commit(&repo, &[("a.txt", "1\n")], "first");
    let message = run(
        ccx,
        "out = []\ntry:\n    await git(\"push\")\nexcept TypeError as e:\n    out.append(str(e))\ntry:\n    await git(\"status\", \"x\")\nexcept TypeError as e:\n    out.append(str(e))\ntry:\n    await git(\"diff\", rev=\"--output=\" + PROJECT_ROOT + \"/out.txt\")\nexcept TypeError as e:\n    out.append(str(e))\ntry:\n    await git(\"diff\", path=\"../outside\")\nexcept TypeError as e:\n    out.append(str(e))\nmissing = await git(\"show\", \"nope\")\nout.append(missing[\"error\"])\n\" | \".join(out)",
    )
    .await;
    let result = text(&message);
    for needle in [
        "unknown subcommand 'push'; read-only subcommands: status, diff, log, show, branches",
        "git('status') takes no positional argument after the subcommand",
        "Revision must not start with '-'",
        "Git paths must be non-empty paths relative to the repository",
        "revision not found: nope",
    ] {
        assert!(result.contains(needle), "{needle}: {result}");
    }
    assert!(!project.join("out.txt").exists());
    let records = calls(&message);
    assert_eq!(records.len(), 5);
    assert!(
        records.iter().all(|r| r["status"] == "failed"),
        "{records:?}"
    );
}

#[tokio::test]
async fn git_withholds_blocked_files_and_records_restricted_ones() {
    let (dir, app, ccx) = fixture().await;
    let project = dir.path().join("project");
    let repo = repo(&project);
    commit(
        &repo,
        &[
            ("secret.txt", "s1\n"),
            ("guarded.txt", "g1\n"),
            ("plain.txt", "p1\n"),
        ],
        "first",
    );
    for (path, content) in [
        ("secret.txt", "s2 hidden\n"),
        ("guarded.txt", "g2\n"),
        ("plain.txt", "p2\n"),
    ] {
        std::fs::write(project.join(path), content).unwrap();
    }
    let canonical = dunce::canonicalize(&project).unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec![canonical.join("secret.txt").to_string_lossy().into_owned()];
    policy.zones.insert(
        0,
        refact_privacy::Zone {
            name: "guarded".into(),
            patterns: vec![canonical.join("guarded.txt").to_string_lossy().into_owned()],
            send_to: vec!["trusted".into()],
            ..Default::default()
        },
    );
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let message = run(
        ccx,
        "d = await git(\"diff\")\nstr(d[\"data\"][\"withheld_count\"]) + \":\" + \",\".join(sorted([f[\"path\"] for f in d[\"data\"][\"files\"]]))",
    )
    .await;
    let result = text(&message);
    assert!(result.contains("1:guarded.txt,plain.txt"), "{result}");
    assert!(!result.contains("secret"), "{result}");
    assert!(!result.contains("hidden"), "{result}");
    let files = message.extra["privacy"]["files"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        files
            .iter()
            .any(|f| f["zone"] == "guarded" && f["path"].as_str().unwrap().ends_with("guarded.txt")),
        "{files:?}"
    );
    assert!(!files
        .iter()
        .any(|f| f["path"].as_str().unwrap().ends_with("secret.txt")));
}

#[tokio::test]
async fn git_checks_project_relative_globs_before_returning_results() {
    let (dir, app, ccx) = fixture().await;
    let project = dir.path().join("project");
    let repo = repo(&project);
    commit(
        &repo,
        &[
            ("samples/withheld.txt", "fixture plum before\n"),
            ("samples/allowed.txt", "fixture pear before\n"),
        ],
        "fruit fixture",
    );
    std::fs::write(project.join("samples/withheld.txt"), "fixture plum after\n").unwrap();
    std::fs::write(project.join("samples/allowed.txt"), "fixture pear after\n").unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked.push("samples/withheld*.txt".into());
    policy.zones.insert(
        0,
        refact_privacy::Zone {
            name: "fruit-fixture".into(),
            patterns: vec!["samples/allowed.txt".into()],
            send_to: vec!["model".into()],
            ..Default::default()
        },
    );
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();

    let message = run(
        ccx,
        r#"try:
    await git("show", "HEAD", "samples/withheld.txt")
    denied = False
except PermissionError:
    denied = True
shown = await git("show", "HEAD", "samples/allowed.txt")
diff = await git("diff")
[denied, shown["data"]["text"], diff["data"]["withheld_count"], [f["path"] for f in diff["data"]["files"]]]"#,
    )
    .await;

    let python = &message.extra["python"];
    assert_eq!(message.tool_failed, Some(false), "{python}");
    let run_dir = Path::new(python["run_dir"].as_str().unwrap());
    let read_json = |path: &str| -> Value {
        serde_json::from_slice(&std::fs::read(run_dir.join(path)).unwrap()).unwrap()
    };
    assert_eq!(
        read_json("result.json"),
        json!([true, "fixture pear before\n", 1, ["samples/allowed.txt"]])
    );
    let records = calls(&message);
    let statuses: Vec<_> = records.iter().map(|r| r["status"].as_str()).collect();
    assert_eq!(statuses, [Some("denied"), Some("ok"), Some("ok")]);
    let denied = read_json("calls/1_git.json");
    assert_eq!(denied["envelope"]["exception"], "PermissionError");
    assert!(!denied.to_string().contains("fixture plum"), "{denied}");
    let diff = read_json("calls/3_git.json");
    let diff = &diff["envelope"];
    assert!(diff["data"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("+fixture pear after"));
    assert!(!diff.to_string().contains("samples/withheld.txt"), "{diff}");
    assert!(!diff.to_string().contains("fixture plum"), "{diff}");
    let privacy = message.extra["privacy"]["files"].as_array().unwrap();
    assert_eq!(privacy.len(), 1, "{privacy:?}");
    assert_eq!(privacy[0]["zone"], "fruit-fixture");
    assert_eq!(privacy[0]["attribution"], "observed");
    assert!(privacy[0]["path"]
        .as_str()
        .unwrap()
        .ends_with("samples/allowed.txt"));
}

#[tokio::test]
async fn git_discards_output_when_its_privacy_snapshot_changes() {
    let (dir, app, ccx) = fixture().await;
    let repo = repo(&dir.path().join("project"));
    commit(
        &repo,
        &[("fruit.txt", "fixture apricot\n")],
        "fruit fixture",
    );
    *app.gcx.privacy_classification.observer.lock().unwrap() = Some(Box::new(|gcx, event| {
        if event == "git-output" {
            crate::privacy::classification_cache::invalidate(gcx);
        }
    }));

    let message = run(
        ccx,
        "try:\n    r = await git('show', 'HEAD', 'fruit.txt')\nexcept PermissionError as e:\n    r = str(e)\nr",
    )
    .await;
    let python = &message.extra["python"];
    assert_eq!(message.tool_failed, Some(false), "{python}");
    let run_dir = Path::new(python["run_dir"].as_str().unwrap());
    let logged: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("calls/1_git.json")).unwrap()).unwrap();
    assert_eq!(calls(&message)[0]["status"], "denied");
    assert_eq!(logged["envelope"]["exception"], "PermissionError");
    assert_eq!(
        logged["envelope"]["message"],
        refact_privacy::gate::PRIVACY_CONTEXT_CHANGED
    );
    assert!(!logged.to_string().contains("fixture apricot"), "{logged}");
    assert!(!text(&message).contains("fixture apricot"));
}

#[tokio::test]
async fn git_works_in_a_read_only_chat_and_obeys_the_mode_deny_rule() {
    let (dir, app, _) = fixture().await;
    let project = dir.path().join("project");
    let repo = repo(&project);
    commit(&repo, &[("a.txt", "1\n")], "first");
    let app = with_tools(app);
    let message = execute(
        app.clone(),
        &[call(
            "read-only",
            "r = await git(\"log\")\n[r[\"ok\"], r[\"data\"][\"commits\"][0][\"subject\"], 'run' in tools()]",
            false,
        )],
    )
    .await;
    let result = text(&message[0]);
    assert!(
        result.contains("first") && result.contains("false"),
        "{result}"
    );
    assert_eq!(message[0].extra["python"]["read_only"], true);

    let mut registry =
        crate::yaml_configs::customization_registry::get_project_registry(app.gcx.clone())
            .await
            .unwrap();
    registry
        .modes
        .get_mut("agent")
        .unwrap()
        .tool_confirm
        .rules
        .insert(
            0,
            crate::yaml_configs::customization_types::ToolConfirmRule {
                match_pattern: "git".into(),
                action: "deny".into(),
            },
        );
    let root = crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .remove(0);
    app.gcx
        .project_registry_cache
        .write()
        .unwrap()
        .insert(root, registry);
    app.gcx.tool_catalog_generations.advance_customization();
    let message = execute(
        app,
        &[call(
            "denied",
            "try:\n    await git(\"status\")\n    r = 'ran'\nexcept PermissionError as e:\n    r = str(e)\nr",
            false,
        )],
    )
    .await;
    assert!(
        text(&message[0]).contains("mode policy: deny (git)"),
        "{}",
        text(&message[0])
    );
    assert_eq!(calls(&message[0])[0]["status"], "denied");
}

#[tokio::test]
async fn git_charges_the_nested_call_budget() {
    let (dir, _app, ccx) = fixture().await;
    let project = dir.path().join("project");
    let repo = repo(&project);
    commit(&repo, &[("a.txt", "1\n")], "first");
    std::fs::write(
        project.join(".refact/coordination.yaml"),
        "python_max_nested_calls: 1\n",
    )
    .unwrap();
    let message = run(
        ccx,
        "await git(\"status\")\ntry:\n    await git(\"status\")\n    r = 'ran'\nexcept RuntimeError as e:\n    r = str(e)\nr",
    )
    .await;
    assert!(
        text(&message).contains("nested call budget exhausted"),
        "{}",
        text(&message)
    );
}

#[tokio::test]
async fn outside_a_repository_git_returns_ok_false() {
    let (_dir, _app, ccx) = fixture().await;
    let message = run(ccx, "r = await git(\"status\")\n[r[\"ok\"], r[\"error\"]]").await;
    let result = text(&message);
    assert!(
        result.contains("false") && result.contains("not a git repository"),
        "{result}"
    );
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn git_pre_tool_use_hook_sees_git_and_a_rewrite_is_checked_again() {
    let (dir, _app, ccx) = fixture().await;
    let project = dir.path().join("project");
    let repo = repo(&project);
    commit(&repo, &[("a.txt", "1\n")], "first");
    let hooks = dir.path().join("config/hooks.yaml");
    std::fs::create_dir_all(hooks.parent().unwrap()).unwrap();
    let use_hook = |command: String| {
        std::fs::write(
            &hooks,
            format!("hooks:\n  PreToolUse:\n    - hooks:\n        - type: command\n          command: {command:?}\n"),
        )
        .unwrap();
        crate::ext::hooks_runner::clear_hook_cache_for_tests();
    };
    let payload = dir.path().join("payload.json");
    use_hook(format!(
        "cat > '{}'; echo git-denied >&2; exit 2",
        payload.display()
    ));
    let denied = run(
        ccx.clone(),
        "try:\n    await git(\"diff\", staged=True)\n    r = 'ran'\nexcept PermissionError as e:\n    r = str(e)\nr",
    )
    .await;
    assert!(text(&denied).contains("git-denied"), "{}", text(&denied));
    assert_eq!(calls(&denied)[0]["status"], "denied");
    let value: Value = serde_json::from_slice(&std::fs::read(&payload).unwrap()).unwrap();
    assert_eq!(value["tool_name"], "git");
    assert_eq!(value["tool_input"]["subcommand"], "diff");
    assert_eq!(value["tool_input"]["staged"], true);
    use_hook(
        "echo '{\"decision\":\"rewrite\",\"tool_input\":{\"subcommand\":\"push\"}}'".to_string(),
    );
    let rewritten = run(
        ccx,
        "try:\n    await git(\"status\")\n    r = 'ran'\nexcept PermissionError as e:\n    r = str(e)\nr",
    )
    .await;
    assert!(
        text(&rewritten).contains("unknown subcommand 'push'"),
        "{}",
        text(&rewritten)
    );
    std::fs::remove_file(&hooks).unwrap();
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
}

#[tokio::test]
async fn git_withholds_tracked_files_under_a_protected_folder() {
    let (dir, _app, ccx) = fixture().await;
    let project = dir.path().join("project");
    let repo = repo(&project);
    commit(
        &repo,
        &[
            (".refact/integrations.d/x.yaml", "token: first\n"),
            ("plain.txt", "p1\n"),
        ],
        "first",
    );
    std::fs::write(
        project.join(".refact/integrations.d/x.yaml"),
        "token: hunter2\n",
    )
    .unwrap();
    std::fs::write(project.join("plain.txt"), "p2\n").unwrap();
    let message = run(
        ccx,
        "d = await git(\"diff\")\ntry:\n    await git(\"show\", \"HEAD\", \".refact/integrations.d/x.yaml\")\n    shown = 'shown'\nexcept PermissionError as e:\n    shown = 'PermissionError'\n\"|\".join([\",\".join([f[\"path\"] for f in d[\"data\"][\"files\"]]), str(d[\"data\"][\"withheld_count\"]), shown, str('token' in str(d))])",
    )
    .await;
    let result = text(&message);
    assert!(
        result.contains("plain.txt|1|PermissionError|False"),
        "{result}"
    );
}

#[tokio::test]
async fn git_reads_only_the_workspace_folder_of_a_repository_above_it() {
    let (dir, _app, ccx) = fixture().await;
    let repo = repo(dir.path());
    commit(
        &repo,
        &[("project/a.txt", "1\n"), ("outside.txt", "o1\n")],
        "both",
    );
    commit(&repo, &[("outside.txt", "o2\n")], "outside only");
    std::fs::write(dir.path().join("project/a.txt"), "2\n").unwrap();
    std::fs::write(dir.path().join("outside.txt"), "o3\n").unwrap();
    let message = run(
        ccx,
        "s = await git(\"status\")\nd = await git(\"diff\")\nl = await git(\"log\")\n\"|\".join([\",\".join([c[\"path\"] for c in s[\"data\"][\"changes\"]]), \",\".join([f[\"path\"] for f in d[\"data\"][\"files\"]]), \",\".join([c[\"subject\"] for c in l[\"data\"][\"commits\"]]), str('outside' in str(s) + str(d) + str(l))])",
    )
    .await;
    let result = text(&message);
    assert!(
        result.contains("project/a.txt|project/a.txt|both|False"),
        "{result}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn git_judges_a_linked_path_by_its_own_name_too() {
    let (dir, app, ccx) = fixture().await;
    let project = dir.path().join("project");
    let repo = repo(&project);
    commit(
        &repo,
        &[
            (".refact/integrations.d/x.yaml", "token: hunter2\n"),
            ("secret.txt", "old secret\n"),
            ("plain.txt", "p\n"),
        ],
        "first",
    );
    for name in [".refact/integrations.d/x.yaml", "secret.txt"] {
        std::fs::remove_file(project.join(name)).unwrap();
        std::os::unix::fs::symlink(project.join("plain.txt"), project.join(name)).unwrap();
    }
    let canonical = dunce::canonicalize(&project).unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec![canonical.join("secret.txt").to_string_lossy().into_owned()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let message = run(
        ccx,
        "out = []\nfor p in [\".refact/integrations.d/x.yaml\", \"secret.txt\"]:\n    try:\n        await git(\"show\", \"HEAD\", p)\n        out.append('shown')\n    except PermissionError:\n        out.append('PermissionError')\nd = await git(\"diff\", rev=\"HEAD\")\n\"|\".join(out + [\",\".join([f[\"path\"] for f in d[\"data\"][\"files\"]]), str(d[\"data\"][\"withheld_count\"]), str('hunter2' in str(d) or 'old secret' in str(d))])",
    )
    .await;
    let result = text(&message);
    assert!(
        result.contains("PermissionError|PermissionError||2|False"),
        "{result}"
    );
}
