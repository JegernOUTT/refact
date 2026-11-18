use super::*;
use serde_json::json;

use crate::tools::tool_trajectory_context::ToolTrajectoryContext;

async fn project_ctx() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Arc<AMutex<AtCommandsContext>>,
) {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let trajectories = project.join(".refact").join("trajectories");
    std::fs::create_dir_all(&trajectories).unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        root.path().join("cache"),
        root.path().join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project];
    let ctx = AtCommandsContext::new(
        gcx,
        4096,
        10,
        false,
        vec![],
        "current-chat".into(),
        None,
        String::new(),
        None,
        None,
    )
    .await;
    (root, trajectories, Arc::new(AMutex::new(ctx)))
}

fn write_traj(dir: &std::path::Path, id: &str, title: &str, messages: Vec<Value>) {
    let body = json!({"id": id, "title": title, "messages": messages});
    std::fs::write(
        dir.join(format!("{id}.json")),
        serde_json::to_vec(&body).unwrap(),
    )
    .unwrap();
}

fn text_of(result: Vec<ContextEnum>) -> String {
    match result.into_iter().next() {
        Some(ContextEnum::ChatMessage(ChatMessage {
            content: ChatContent::SimpleText(text),
            ..
        })) => text,
        other => panic!("unexpected tool result: {other:?}"),
    }
}

async fn run_search(ctx: &Arc<AMutex<AtCommandsContext>>, args: Value) -> Result<String, String> {
    let args: HashMap<String, Value> = serde_json::from_value(args).unwrap();
    ToolSearchTrajectories {
        config_path: String::new(),
    }
    .tool_execute(ctx.clone(), &"call".to_string(), &args)
    .await
    .map(|(_, result)| text_of(result))
}

fn parse_hits(output: &str) -> Vec<(String, String, String)> {
    let mut hits = Vec::new();
    let mut id = None;
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("📁 ") {
            id = Some(rest.to_string());
        }
        if let Some(rest) = line.strip_prefix("📍 Messages: ") {
            let (start, end) = rest.split_once('-').unwrap();
            hits.push((id.take().unwrap(), start.to_string(), end.to_string()));
        }
    }
    hits
}

#[tokio::test]
async fn search_output_ranges_expand_through_get_trajectory_context() {
    let (_root, dir, ctx) = project_ctx().await;
    let mut messages: Vec<Value> = (0..8)
        .map(|i| json!({"role": "user", "content": format!("filler {i}")}))
        .collect();
    messages.push(json!({"role": "user", "content": "how do we rotate the zebra credentials"}));
    messages.push(json!({"role": "assistant", "content": "zebra credentials rotate via vault"}));
    write_traj(&dir, "past-chat", "Credential rotation", messages);
    write_traj(
        &dir,
        "current-chat",
        "Current",
        vec![json!({"role": "user", "content": "unrelated"})],
    );

    let output = run_search(&ctx, json!({"query": "zebra credentials", "top_n": "3"}))
        .await
        .unwrap();
    let hits = parse_hits(&output);
    assert_eq!(hits.len(), 1, "{output}");
    let (id, start, end) = &hits[0];
    assert_eq!(id, "past-chat");
    assert!(output.contains("📌 Credential rotation"));
    assert!(output.contains("[msg 8] user: how do we rotate the zebra credentials"));
    assert!(start.parse::<usize>().unwrap() <= 8 && end.parse::<usize>().unwrap() >= 9);

    let args: HashMap<String, Value> = serde_json::from_value(json!({
        "trajectory_id": id, "message_start": start, "message_end": end, "expand_by": "0"
    }))
    .unwrap();
    let expanded = ToolTrajectoryContext {
        config_path: String::new(),
    }
    .tool_execute(ctx.clone(), &"call".to_string(), &args)
    .await
    .map(|(_, result)| text_of(result))
    .unwrap();
    assert!(
        expanded.contains("zebra credentials rotate via vault"),
        "{expanded}"
    );
    assert!(expanded.contains(&format!("(requested {start}-{end})")));
}

#[tokio::test]
async fn search_reports_no_matches_and_rejects_invalid_query() {
    let (_root, dir, ctx) = project_ctx().await;
    write_traj(
        &dir,
        "a",
        "A",
        vec![json!({"role": "user", "content": "giraffe"})],
    );

    let output = run_search(&ctx, json!({"query": "okapi"})).await.unwrap();
    assert_eq!(output, NO_MATCHING_TRAJECTORIES);

    let output = run_search(&ctx, json!({"query": "the and"})).await.unwrap();
    assert_eq!(output, NO_MATCHING_TRAJECTORIES);

    assert_eq!(
        run_search(&ctx, json!({})).await.unwrap_err(),
        "argument `query` is missing"
    );
    assert!(run_search(&ctx, json!({"query": 5}))
        .await
        .unwrap_err()
        .starts_with("argument `query` is not a string"));
}

#[tokio::test]
async fn search_respects_top_n_and_reflects_updates_and_deletions() {
    let (_root, dir, ctx) = project_ctx().await;
    for i in 0..4 {
        write_traj(
            &dir,
            &format!("chat-{i}"),
            "T",
            vec![json!({"role": "user", "content": "quokka"})],
        );
    }
    let output = run_search(&ctx, json!({"query": "quokka", "top_n": 2}))
        .await
        .unwrap();
    assert_eq!(parse_hits(&output).len(), 2);

    write_traj(
        &dir,
        "chat-0",
        "T",
        vec![json!({"role": "user", "content": "quokka wombat"})],
    );
    let output = run_search(&ctx, json!({"query": "wombat"})).await.unwrap();
    let ids: Vec<String> = parse_hits(&output).into_iter().map(|h| h.0).collect();
    assert_eq!(ids, vec!["chat-0"]);

    std::fs::remove_file(dir.join("chat-0.json")).unwrap();
    let output = run_search(&ctx, json!({"query": "wombat"})).await.unwrap();
    assert_eq!(output, NO_MATCHING_TRAJECTORIES);
}

#[test]
fn format_marks_truncated_previews_and_coverage_gaps() {
    let hit = TrajectoryHit {
        trajectory_id: "t".into(),
        title: "Title".into(),
        message_start: 2,
        message_end: 5,
        score: 3.0,
        snippet: "abcdefghij".into(),
    };
    let stats = RefreshStats {
        read: 0,
        pending: 4,
        over_cap: 7,
    };
    let output = format_search_output(&[hit], &stats, 4);
    assert!(output.contains(
        "📁 t\n📌 Title\n⭐ Relevance: 100%\n📍 Messages: 2-5\n\nabcd\n⚠️ showing 4 of 10"
    ));
    assert!(output.contains("4 changed trajectories were not indexed yet"));
    assert!(output.contains("7 older trajectories are beyond the search cap"));

    let empty = format_search_output(&[], &RefreshStats::default(), 4);
    assert_eq!(empty, NO_MATCHING_TRAJECTORIES);
}
