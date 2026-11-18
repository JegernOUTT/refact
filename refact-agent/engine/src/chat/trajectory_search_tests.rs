use super::*;
use serde_json::json;

fn msg(role: &str, content: &str) -> Value {
    json!({"role": role, "content": content})
}

fn write_traj(dir: &Path, id: &str, title: &str, messages: Vec<Value>) -> PathBuf {
    let path = dir.join(format!("{id}.json"));
    let body = json!({"id": id, "title": title, "messages": messages, "root_chat_id": id});
    std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();
    path
}

fn filler(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| {
            msg(
                if i % 2 == 0 { "user" } else { "assistant" },
                "lorem ipsum dolor",
            )
        })
        .collect()
}

fn search(dir: &Path, query: &str, top_n: usize) -> (Vec<TrajectoryHit>, RefreshStats) {
    let mut index = TrajectorySearchIndex::default();
    let stats = index.refresh(&[dir.to_path_buf()], &SearchLimits::default());
    (index.search(&query_terms(query), top_n), stats)
}

#[test]
fn finds_trajectory_and_window_containing_the_terms() {
    let dir = tempfile::tempdir().unwrap();
    let mut messages = vec![msg("system", "flamingo flamingo flamingo")];
    messages.extend(filler(9));
    messages.push(msg("user", "the flamingo parser crashes on unicode"));
    messages.push(msg("assistant", "fixed the flamingo parser"));
    write_traj(dir.path(), "chat-a", "Untitled work", messages);
    write_traj(dir.path(), "chat-b", "Other", filler(6));

    let (hits, _) = search(dir.path(), "flamingo parser", 5);

    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit.trajectory_id, "chat-a");
    assert_eq!(hit.title, "Untitled work");
    assert!(hit.message_start <= 10 && hit.message_end >= 11, "{hit:?}");
    assert!(
        hit.message_start >= 1,
        "system messages are not indexed: {hit:?}"
    );
    assert!(hit
        .snippet
        .contains("[msg 10] user: the flamingo parser crashes"));
    assert!(hit
        .snippet
        .contains("[msg 11] assistant: fixed the flamingo parser"));
}

#[test]
fn ranking_prefers_denser_window_and_title_match() {
    let dir = tempfile::tempdir().unwrap();
    write_traj(
        dir.path(),
        "dense",
        "Misc",
        vec![
            msg("user", "kraken deploy kraken rollback kraken"),
            msg("assistant", "kraken done"),
        ],
    );
    write_traj(
        dir.path(),
        "sparse",
        "Misc",
        vec![msg(
            "user",
            "kraken mentioned once among many other unrelated words here",
        )],
    );
    write_traj(
        dir.path(),
        "titled",
        "Kraken migration",
        vec![msg(
            "user",
            "kraken mentioned once among many other unrelated words here",
        )],
    );
    for i in 0..5 {
        write_traj(dir.path(), &format!("noise-{i}"), "Noise", filler(4));
    }

    let (hits, _) = search(dir.path(), "kraken", 5);
    let ids: Vec<&str> = hits.iter().map(|h| h.trajectory_id.as_str()).collect();

    let rank = |id: &str| ids.iter().position(|x| *x == id).unwrap();
    assert_eq!(ids.len(), 3, "{ids:?}");
    assert!(rank("dense") < rank("sparse"), "{ids:?}");
    assert!(rank("titled") < rank("sparse"), "{ids:?}");
    assert!(hits[rank("titled")].score > hits[rank("sparse")].score);
}

#[test]
fn top_n_is_respected_and_zero_returns_nothing() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..6 {
        write_traj(
            dir.path(),
            &format!("t{i}"),
            "T",
            vec![msg("user", "walrus")],
        );
    }
    assert_eq!(search(dir.path(), "walrus", 3).0.len(), 3);
    assert_eq!(search(dir.path(), "walrus", 0).0.len(), 0);
}

#[test]
fn no_match_and_stopword_only_query_return_empty() {
    let dir = tempfile::tempdir().unwrap();
    write_traj(dir.path(), "t", "T", vec![msg("user", "walrus")]);
    assert!(search(dir.path(), "narwhal", 5).0.is_empty());
    assert!(query_terms("the and of").is_empty());
    assert!(search(dir.path(), "the and of", 5).0.is_empty());
}

#[test]
fn updated_file_is_reindexed_and_deleted_file_disappears() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_traj(dir.path(), "live", "T", vec![msg("user", "otter")]);
    write_traj(dir.path(), "other", "T", vec![msg("user", "otter")]);
    let dirs = [dir.path().to_path_buf()];
    let limits = SearchLimits::default();
    let mut index = TrajectorySearchIndex::default();

    assert_eq!(index.refresh(&dirs, &limits).read, 2);
    assert!(index.search(&query_terms("badger"), 5).is_empty());
    assert_eq!(index.refresh(&dirs, &limits).read, 0);

    write_traj(
        dir.path(),
        "live",
        "T",
        vec![
            msg("user", "otter"),
            msg("assistant", "badger badger and more"),
        ],
    );
    assert_eq!(index.refresh(&dirs, &limits).read, 1);
    let hits = index.search(&query_terms("badger"), 5);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].trajectory_id, "live");
    assert_eq!((hits[0].message_start, hits[0].message_end), (0, 1));

    std::fs::remove_file(&path).unwrap();
    index.refresh(&dirs, &limits);
    assert!(index.search(&query_terms("badger"), 5).is_empty());
    let ids: Vec<_> = index
        .search(&query_terms("otter"), 5)
        .into_iter()
        .map(|h| h.trajectory_id)
        .collect();
    assert_eq!(ids, vec!["other"]);
    assert_eq!(index.indexed_paths(), vec![dir.path().join("other.json")]);
}

#[test]
fn nested_chat_folders_are_searched_and_ids_come_from_file_stem() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("root-chat");
    std::fs::create_dir(&nested).unwrap();
    write_traj(&nested, "sub-chat", "Sub", vec![msg("user", "pelican")]);
    std::fs::create_dir(nested.join("python")).unwrap();
    write_traj(
        &nested.join("python"),
        "deep",
        "Deep",
        vec![msg("user", "pelican")],
    );

    let (hits, _) = search(dir.path(), "pelican", 5);
    let ids: Vec<_> = hits.iter().map(|h| h.trajectory_id.as_str()).collect();
    assert_eq!(ids, vec!["sub-chat"]);
}

#[test]
fn hidden_buddy_internal_mismatched_and_invalid_files_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, body: Value| {
        std::fs::write(dir.path().join(name), serde_json::to_vec(&body).unwrap()).unwrap();
    };
    let messages = vec![msg("user", "heron")];
    write("ok.json", json!({"id": "ok", "messages": messages}));
    write(
        "buddy.json",
        json!({"id": "buddy", "buddy_meta": {"x": 1}, "messages": messages}),
    );
    write(
        "trace.json",
        json!({"id": "trace", "link_type": "internal:title_generation", "messages": messages}),
    );
    write(
        "mismatch.json",
        json!({"id": "other", "messages": messages}),
    );
    write(
        ".hidden.json",
        json!({"id": ".hidden", "messages": messages}),
    );
    write("index.json", json!({"id": "index", "messages": messages}));
    std::fs::write(dir.path().join("broken.json"), b"{not json").unwrap();
    std::fs::write(dir.path().join("ok.json.tmp.abc"), b"heron").unwrap();

    let (hits, _) = search(dir.path(), "heron", 10);
    let ids: Vec<_> = hits.iter().map(|h| h.trajectory_id.as_str()).collect();
    assert_eq!(ids, vec!["ok"]);
    assert_eq!(hits[0].title, "Untitled");
}

#[test]
fn compression_reports_and_segment_summaries_are_not_indexed() {
    let dir = tempfile::tempdir().unwrap();
    write_traj(
        dir.path(),
        "c",
        "T",
        vec![
            msg(COMPRESSION_REPORT_ROLE, "ibis"),
            json!({"role": "assistant", "content": "ibis", "extra": {"compression": {"kind": SEGMENT_SUMMARY_KIND}}}),
            msg("context_file", "ibis"),
        ],
    );
    assert!(search(dir.path(), "ibis", 5).0.is_empty());
}

#[test]
fn errors_and_ui_only_messages_are_not_indexed_but_tool_failures_are() {
    let dir = tempfile::tempdir().unwrap();
    write_traj(
        dir.path(),
        "errors",
        "T",
        vec![
            msg("user", "visible question"),
            msg("error", "error_secret"),
            json!({"role": "assistant", "content": "ui_secret", "_ui_only": true}),
            msg("tool", "ordinary tool failure"),
        ],
    );
    assert!(search(dir.path(), "error_secret", 5).0.is_empty());
    assert!(search(dir.path(), "ui_secret", 5).0.is_empty());
    let (hits, _) = search(dir.path(), "failure", 5);
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].message_start, hits[0].message_end), (0, 3));
    assert!(hits[0]
        .snippet
        .contains("[msg 3] tool: ordinary tool failure"));
    assert_eq!(search(dir.path(), "visible", 5).0.len(), 1);
}

#[test]
fn per_trajectory_char_cap_limits_indexed_text() {
    let dir = tempfile::tempdir().unwrap();
    let long = format!("{} tapir", "x ".repeat(100));
    write_traj(
        dir.path(),
        "big",
        "T",
        vec![msg("user", &long), msg("assistant", "condor")],
    );
    let limits = SearchLimits {
        max_chars_per_trajectory: 50,
        ..SearchLimits::default()
    };
    let mut index = TrajectorySearchIndex::default();
    index.refresh(&[dir.path().to_path_buf()], &limits);

    assert_eq!(index.indexed_chars(), 50);
    assert!(index.search(&query_terms("tapir"), 5).is_empty());
    assert!(index.search(&query_terms("condor"), 5).is_empty());
}

#[test]
fn file_size_and_trajectory_count_caps_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..4 {
        write_traj(
            dir.path(),
            &format!("t{i}"),
            "T",
            vec![msg("user", "gecko")],
        );
    }
    let dirs = [dir.path().to_path_buf()];
    let mut index = TrajectorySearchIndex::default();
    let stats = index.refresh(
        &dirs,
        &SearchLimits {
            max_trajectories: 2,
            ..SearchLimits::default()
        },
    );
    assert_eq!(stats.over_cap, 2);
    assert_eq!(index.search(&query_terms("gecko"), 10).len(), 2);

    let mut index = TrajectorySearchIndex::default();
    index.refresh(
        &dirs,
        &SearchLimits {
            max_file_bytes: 10,
            ..SearchLimits::default()
        },
    );
    assert!(index.search(&query_terms("gecko"), 10).is_empty());
}

#[test]
fn per_query_read_budget_defers_remaining_files_to_later_searches() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..5 {
        write_traj(dir.path(), &format!("t{i}"), "T", vec![msg("user", "lynx")]);
    }
    let dirs = [dir.path().to_path_buf()];
    let limits = SearchLimits {
        max_reads_per_query: 2,
        ..SearchLimits::default()
    };
    let mut index = TrajectorySearchIndex::default();

    let first = index.refresh(&dirs, &limits);
    assert_eq!((first.read, first.pending), (2, 3));
    assert_eq!(index.search(&query_terms("lynx"), 10).len(), 2);
    let second = index.refresh(&dirs, &limits);
    assert_eq!((second.read, second.pending), (2, 1));
    let third = index.refresh(&dirs, &limits);
    assert_eq!((third.read, third.pending), (1, 0));
    assert_eq!(index.search(&query_terms("lynx"), 10).len(), 5);
}

#[test]
fn snippet_is_bounded_and_centered_on_the_match() {
    let dir = tempfile::tempdir().unwrap();
    let text = format!("{} jaguar {}", "a".repeat(1000), "b".repeat(1000));
    write_traj(dir.path(), "s", "T", vec![msg("user", &text)]);
    let (hits, _) = search(dir.path(), "jaguar", 5);
    let snippet = &hits[0].snippet;
    assert!(snippet.contains("jaguar"));
    assert!(snippet.starts_with("[msg 0] user: …"));
    assert!(snippet.ends_with('…'));
    assert!(snippet.chars().count() < SNIPPET_CHARS + 30);
}
