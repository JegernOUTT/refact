use super::*;
use serde_json::json;

fn fixture() -> (tempfile::TempDir, PythonWorkspace) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = PythonWorkspace::from_trajectory(
        &temp.path().join("chat.json"),
        "chat",
        temp.path().join("key"),
    );
    (temp, workspace)
}
fn view(workspace: &PythonWorkspace, active: &[String], background: bool) -> WorkspaceView {
    collect(
        workspace,
        "chat",
        1000,
        SessionStatus {
            live: false,
            running: !active.is_empty(),
            background,
        },
        active,
    )
}
#[test]
fn unknown_chat_reports_not_existing() {
    let (_temp, ws) = fixture();
    let v = view(&ws, &[], false);
    assert!(!v.exists);
    assert_eq!(v.session.status, "none");
    assert_eq!(v.disk.quota_bytes, 0);
}
#[test]
fn empty_workspace_reports_idle_and_zero_usage() {
    let (_temp, ws) = fixture();
    std::fs::create_dir_all(ws.dir()).unwrap();
    let v = view(&ws, &[], false);
    assert!(v.exists);
    assert_eq!(v.session.status, "idle");
    assert_eq!(v.disk.workspace_bytes, 0);
    assert!(!v.session.state_persisted);
}
#[test]
fn populated_workspace_lists_state_artifacts_and_runs() {
    let (_temp, ws) = fixture();
    std::fs::create_dir_all(ws.artifacts_dir().join("nested")).unwrap();
    std::fs::create_dir_all(ws.run_dir("r1")).unwrap();
    std::fs::write(ws.artifacts_dir().join("nested/a.txt"), "abc").unwrap();
    std::fs::write(ws.dir().join("state.json"), json!({"bytes":12,"saved_at_ms":42,"last_run_id":"r1","variables":[{"name":"x","type_name":"int","preview":"1"}]}).to_string()).unwrap();
    std::fs::write(ws.run_dir("r1").join("run.json"), json!({"status":"completed","started_at_ms":10,"finished_at_ms":20,"calls_total":2,"background":true}).to_string()).unwrap();
    let v = view(&ws, &[], false);
    assert_eq!(v.session.state_bytes, 12);
    assert_eq!(v.session.saved_at_ms, Some(42));
    assert_eq!(v.session.variables[0].name, "x");
    assert_eq!(v.artifacts[0].rel_path, "nested/a.txt");
    assert_eq!(v.artifacts[0].bytes, 3);
    assert_eq!(v.runs[0].calls_total, 2);
    assert_eq!(v.runs[0].description, None);
    assert_eq!(v.runs[0].code_preview, None);
    assert!(serde_json::to_value(&v.runs[0])
        .unwrap()
        .get("description")
        .is_none());
    std::fs::write(ws.run_dir("r1").join("run.json"), json!({"status":"completed","started_at_ms":10,"description":"Count TODO markers","code_preview":"x = 1"}).to_string()).unwrap();
    let v = view(&ws, &[], false);
    assert_eq!(v.runs[0].description.as_deref(), Some("Count TODO markers"));
    assert_eq!(v.runs[0].code_preview.as_deref(), Some("x = 1"));
    assert!(v.disk.workspace_bytes > 3);
    assert!(serde_json::to_value(&v.runs[0])
        .unwrap()
        .get("llm_calls")
        .is_none());
    std::fs::write(ws.run_dir("r1").join("run.json"), json!({"status":"completed","started_at_ms":10,"calls_total":3,"llm":{"calls":2,"input_tokens":5,"output_tokens":1,"models":["m"]}}).to_string()).unwrap();
    let v = view(&ws, &[], false);
    assert_eq!((v.runs[0].calls_total, v.runs[0].llm_calls), (3, Some(2)));
}
#[test]
fn lists_are_capped_and_flagged_truncated() {
    let (_temp, ws) = fixture();
    std::fs::create_dir_all(ws.artifacts_dir()).unwrap();
    for n in 0..501 {
        std::fs::write(ws.artifacts_dir().join(n.to_string()), "").unwrap();
        std::fs::create_dir_all(ws.run_dir(&n.to_string())).unwrap();
    }
    std::fs::write(
        ws.dir().join("state.json"),
        json!({"variables":vec![json!({"name":"x","type_name":"int","preview":"1"});201]})
            .to_string(),
    )
    .unwrap();
    let v = view(&ws, &[], false);
    assert_eq!(v.artifacts.len(), 500);
    assert_eq!(v.runs.len(), 500);
    assert_eq!(v.session.variables.len(), 200);
    assert!(v.artifacts_truncated && v.runs_truncated && v.session.variables_truncated);
}
#[test]
fn active_run_is_reported_running_or_background() {
    let (_temp, ws) = fixture();
    std::fs::create_dir_all(ws.dir()).unwrap();
    assert_eq!(view(&ws, &["r1".into()], false).session.status, "running");
    assert_eq!(view(&ws, &["r1".into()], true).session.status, "background");
}
#[test]
fn run_without_run_json_is_failed_unless_active() {
    let (_temp, ws) = fixture();
    std::fs::create_dir_all(ws.run_dir("r1")).unwrap();
    assert_eq!(view(&ws, &[], false).runs[0].status, "failed");
    let v = view(&ws, &["r1".into()], false);
    assert_eq!(v.runs[0].status, "running");
    assert_eq!(v.runs[0].finished_at_ms, None);
}
#[cfg(unix)]
#[test]
fn symlink_escaping_workspace_is_not_followed() {
    use std::os::unix::fs::symlink;
    let (temp, ws) = fixture();
    std::fs::create_dir_all(ws.artifacts_dir()).unwrap();
    let outside = temp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("secret"), "secret").unwrap();
    std::fs::write(outside.join("state.json"), "{}").unwrap();
    symlink(&outside, ws.artifacts_dir().join("escape")).unwrap();
    symlink(outside.join("state.json"), ws.dir().join("state.json")).unwrap();
    let v = view(&ws, &[], false);
    assert!(v.artifacts.is_empty());
    assert_eq!(v.disk.workspace_bytes, 0);
    assert!(!v.session.state_persisted);
}

#[cfg(unix)]
#[test]
fn links_above_the_storage_base_are_followed_and_links_below_it_are_refused() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("real/trajectories")).unwrap();
    symlink(temp.path().join("real"), temp.path().join("alias")).unwrap();
    let base = temp.path().join("alias/trajectories");
    let key = temp.path().join("key");
    let workspace = |owner: &str, chat: &str| {
        PythonWorkspace::from_trajectory(
            &base.join(owner).join(format!("{chat}.json")),
            chat,
            key.clone(),
        )
    };
    let ws = workspace("owner", "chat");
    std::fs::create_dir_all(ws.run_dir("r1")).unwrap();
    std::fs::write(ws.dir().join("state.json"), json!({"bytes":12}).to_string()).unwrap();
    std::fs::write(
        ws.run_dir("r1").join("run.json"),
        json!({"status":"completed","started_at_ms":10}).to_string(),
    )
    .unwrap();
    let v = view(&ws, &[], false);
    assert!(v.exists);
    assert_eq!(v.session.status, "idle");
    assert_eq!(v.session.state_bytes, 12);
    assert_eq!(v.runs[0].status, "completed");

    symlink(base.join("owner"), base.join("swapped")).unwrap();
    assert!(!view(&workspace("swapped", "chat"), &[], false).exists);

    std::fs::create_dir(base.join("python-link")).unwrap();
    symlink(base.join("owner/python"), base.join("python-link/python")).unwrap();
    assert!(!view(&workspace("python-link", "chat"), &[], false).exists);

    let linked_chat = workspace("owner", "other");
    symlink(ws.dir(), linked_chat.dir()).unwrap();
    assert!(!view(&linked_chat, &[], false).exists);

    let linked_runs = workspace("owner", "runs-link");
    std::fs::create_dir_all(linked_runs.dir()).unwrap();
    symlink(ws.runs_dir(), linked_runs.runs_dir()).unwrap();
    let v = view(&linked_runs, &[], false);
    assert!(v.exists);
    assert!(v.runs.is_empty(), "{:?}", v.runs);
}

#[cfg(windows)]
#[test]
fn junction_below_the_storage_base_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("trajectories");
    let key = temp.path().join("key");
    let ws = PythonWorkspace::from_trajectory(&base.join("owner/chat.json"), "chat", key.clone());
    std::fs::create_dir_all(ws.dir()).unwrap();
    assert!(view(&ws, &[], false).exists);
    let junction = base.join("swapped");
    let status = std::process::Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&junction)
        .arg(base.join("owner"))
        .status()
        .unwrap();
    assert!(status.success(), "junction setup must succeed");
    let swapped = PythonWorkspace::from_trajectory(&junction.join("chat.json"), "chat", key);
    assert!(!view(&swapped, &[], false).exists);
    std::fs::remove_dir(junction).unwrap();
}

#[test]
fn active_run_ids_are_chat_scoped_and_removed_with_guard() {
    let runtime = super::super::PythonToolRuntime::default();
    let guard = runtime.register_run("chat", "tool", "r1");
    let _other = runtime.register_run("other", "tool", "r2");
    assert_eq!(runtime.active_run_ids("chat"), vec!["r1"]);
    drop(guard);
    assert!(runtime.active_run_ids("chat").is_empty());
}

#[tokio::test]
async fn registry_status_tracks_reserved_and_detached_turns() {
    let registry = super::super::registry::KernelRegistry::default();
    assert!(!registry.session_status("chat").running);
    let lease = registry
        .reserve_turn("chat", "tool")
        .acquire()
        .await
        .unwrap();
    assert!(registry.session_status("chat").running);
    assert!(!registry.session_status("chat").live);
    lease.mark_background("job");
    assert!(registry.session_status("chat").background);
    drop(lease);
    assert!(!registry.session_status("chat").running);
}

#[tokio::test]
async fn workspace_walk_depth_is_bounded_and_reports_truncation() {
    let (_dir, app, _) = crate::tools::tool_python::tests::fixture().await;
    let workspace = PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    workspace.ensure_dirs().await.unwrap();
    let mut path = workspace.artifacts_dir();
    for _ in 0..20 {
        path = path.join("deep");
    }
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("hidden"), "data").unwrap();
    let view = workspace_view(&app, "python-test").await.unwrap();
    assert!(view.artifacts_truncated);
    assert!(view.artifacts.is_empty());
}

#[test]
fn workspace_view_retains_kernel_variable_truncation() {
    let (_temp, workspace) = fixture();
    std::fs::create_dir_all(workspace.dir()).unwrap();
    std::fs::write(
        workspace.dir().join("state.json"),
        json!({"variables":[],"variables_truncated":true}).to_string(),
    )
    .unwrap();
    assert!(view(&workspace, &[], false).session.variables_truncated);
}

#[test]
fn active_run_calls_total_uses_live_started_calls() {
    let (_temp, ws) = fixture();
    std::fs::create_dir_all(ws.run_dir("r1")).unwrap();
    let mut v = view(&ws, &["r1".into()], false);
    apply_live_counts(
        &mut v,
        &std::collections::HashMap::from([("r1".into(), 231)]),
    );
    assert_eq!(v.runs[0].calls_total, 231);
}

#[test]
fn newest_eight_displays_across_runs_skip_unreadable_files() {
    let (_temp, ws) = fixture();
    let mut write = |run: &str, started: u64, count: u32| {
        let dir = ws.run_dir(run).join("displays");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            ws.run_dir(run).join("run.json"),
            json!({"status":"completed","started_at_ms":started,"displays_total":count})
                .to_string(),
        )
        .unwrap();
        for index in 1..=count {
            let rows: Vec<_> = (0..300).map(|i| json!([i, "c".repeat(300)])).collect();
            let file = json!({"schema":1,"index":index,"kind":"table","title":format!("{run}-{index}"),"columns":["n","s"],"rows":rows,"rows_total":300,"truncated":false});
            std::fs::write(dir.join(format!("{index}.json")), file.to_string()).unwrap();
        }
    };
    write("old", 10, 3);
    write("new", 20, 6);
    std::fs::write(ws.run_dir("new").join("displays/6.json"), "{not json").unwrap();
    std::fs::write(ws.run_dir("new").join("displays/notes.txt"), "x").unwrap();
    let v = view(&ws, &[], false);
    let titles: Vec<_> = v
        .displays
        .iter()
        .map(|d| d.display.title.clone().unwrap())
        .collect();
    assert_eq!(
        titles,
        ["new-5", "new-4", "new-3", "new-2", "new-1", "old-3", "old-2", "old-1"]
    );
    assert!(!v.displays_truncated);
    let first = &v.displays[0];
    assert_eq!((first.run_id.as_str(), first.started_at_ms), ("new", 20));
    assert_eq!(first.display.rows.as_ref().unwrap().len(), 200);
    assert_eq!(first.display.rows_total, Some(300));
    assert_eq!(
        first.display.rows.as_ref().unwrap()[0][1]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        200
    );
    assert!(first.display.truncated);
    assert!(first.display.path.ends_with("5.json"));
    let entry = v.runs.iter().find(|r| r.run_id == "new").unwrap();
    assert_eq!(entry.displays_total, Some(6));
    let wire = serde_json::to_value(&v).unwrap();
    assert_eq!(wire["displays"][0]["run_id"], "new");
    assert_eq!(wire["displays"][0]["kind"], "table");
    assert!(wire.get("displays_truncated").is_none());
    write("newest", 30, 1);
    let v = view(&ws, &[], false);
    assert_eq!(v.displays.len(), 8);
    assert!(v.displays_truncated);
    assert_eq!(v.displays[0].run_id, "newest");
    let empty = fixture();
    std::fs::create_dir_all(empty.1.dir()).unwrap();
    let wire = serde_json::to_value(view(&empty.1, &[], false)).unwrap();
    assert!(wire.get("displays").is_none());
}

#[cfg(unix)]
#[test]
fn reads_go_through_the_checked_storage_base_not_an_alias_changed_after_the_check() {
    let temp = tempfile::tempdir().unwrap();
    for (tree, bytes) in [("a", 1), ("b", 2)] {
        let ws = PythonWorkspace::from_trajectory(
            &temp.path().join(tree).join("trajectories/root/chat.json"),
            "chat",
            temp.path().join("key"),
        );
        std::fs::create_dir_all(ws.dir()).unwrap();
        std::fs::write(
            ws.dir().join("state.json"),
            json!({ "bytes": bytes }).to_string(),
        )
        .unwrap();
    }
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(temp.path().join("b"), &alias).unwrap();
    let ws = PythonWorkspace::from_trajectory(
        &alias.join("trajectories/root/chat.json"),
        "chat",
        temp.path().join("key"),
    );
    let checked_while_the_alias_named_a = Storage {
        root: alias.join("trajectories"),
        real: std::fs::canonicalize(temp.path().join("a/trajectories")).unwrap(),
    };
    let meta = read_json(
        &checked_while_the_alias_named_a,
        &ws.dir().join("state.json"),
    )
    .unwrap();
    assert_eq!(meta["bytes"], 1);
}
#[test]
fn read_only_runs_are_flagged_in_the_summary_and_the_view() {
    let (_temp, ws) = fixture();
    for (run, read_only) in [("r1", true), ("r2", false)] {
        std::fs::create_dir_all(ws.run_dir(run)).unwrap();
        let mut meta = json!({"status":"completed","started_at_ms":10});
        if read_only {
            meta["read_only"] = json!(true);
        }
        let summary = crate::python_tool::workspace::run_summary(&meta);
        assert_eq!(summary.get("read_only").is_some(), read_only);
        std::fs::write(
            ws.run_dir(run)
                .join(crate::python_tool::workspace::RUN_SUMMARY_FILE),
            summary.to_string(),
        )
        .unwrap();
    }
    let v = view(&ws, &[], false);
    let flag = |run: &str| {
        let view = v.runs.iter().find(|r| r.run_id == run).unwrap();
        serde_json::to_value(view)
            .unwrap()
            .get("read_only")
            .cloned()
    };
    assert_eq!(flag("r1"), Some(json!(true)));
    assert_eq!(flag("r2"), None);
}

#[test]
fn variables_that_could_not_be_listed_are_reported_and_left_out_otherwise() {
    let (_temp, ws) = fixture();
    std::fs::create_dir_all(ws.dir()).unwrap();
    for unknown in [true, false] {
        let mut meta = json!({"variables":[]});
        if unknown {
            meta["variables_unknown"] = json!(true);
        }
        std::fs::write(ws.dir().join("state.json"), meta.to_string()).unwrap();
        let v = view(&ws, &[], false);
        assert_eq!(v.session.variables_unknown, unknown);
        let wire = serde_json::to_value(&v.session).unwrap();
        assert_eq!(
            wire.get("variables_unknown").cloned(),
            unknown.then(|| json!(true))
        );
    }
}

#[cfg(unix)]
#[test]
fn the_artifact_walk_runs_and_displays_go_through_the_checked_storage_base() {
    let temp = tempfile::tempdir().unwrap();
    for tree in ["a", "b"] {
        let ws = PythonWorkspace::from_trajectory(
            &temp.path().join(tree).join("trajectories/root/chat.json"),
            "chat",
            temp.path().join("key"),
        );
        std::fs::create_dir_all(ws.artifacts_dir()).unwrap();
        std::fs::write(ws.artifacts_dir().join(format!("{tree}.txt")), tree).unwrap();
        let run = ws.run_dir(&format!("run-{tree}"));
        std::fs::create_dir_all(run.join("displays")).unwrap();
        std::fs::write(
            run.join("run.json"),
            json!({"status":"completed","started_at_ms":10,"displays_total":1}).to_string(),
        )
        .unwrap();
        std::fs::write(
            run.join("displays/1.json"),
            json!({"schema":1,"index":1,"kind":"table","title":format!("display-{tree}"),"columns":["n"],"rows":[[1]],"rows_total":1,"truncated":false}).to_string(),
        )
        .unwrap();
    }
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(temp.path().join("b"), &alias).unwrap();
    let ws = PythonWorkspace::from_trajectory(
        &alias.join("trajectories/root/chat.json"),
        "chat",
        temp.path().join("key"),
    );
    let checked_while_the_alias_named_a = Storage {
        root: alias.join("trajectories"),
        real: std::fs::canonicalize(temp.path().join("a/trajectories")).unwrap(),
    };
    let v = collect_in(
        Some(checked_while_the_alias_named_a),
        &ws,
        "chat",
        1000,
        SessionStatus::default(),
        &[],
    );
    let artifacts: Vec<_> = v.artifacts.iter().map(|a| a.rel_path.as_str()).collect();
    assert_eq!(artifacts, ["a.txt"]);
    let runs: Vec<_> = v.runs.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(runs, ["run-a"]);
    let displays: Vec<_> = v
        .displays
        .iter()
        .map(|d| d.display.title.clone().unwrap())
        .collect();
    assert_eq!(displays, ["display-a"]);
}
