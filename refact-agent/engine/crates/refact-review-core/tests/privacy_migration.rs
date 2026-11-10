use refact_review_core::*;
use refact_privacy::{Attribution, FileRecord};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

fn record(path: &str) -> FileRecord {
    FileRecord {
        path: path.into(),
        zone: "private".into(),
        attribution: Attribution::Declared,
    }
}
fn receipt(path: &str) -> Value {
    json!({"privacy": [record(path)], "summary": "retired"})
}
fn mutation(revision: u64) -> Mutation {
    Mutation {
        expected_revision: revision,
        request_key: "rename".into(),
        actor: Actor {
            kind: ActorKind::User,
            id: "owner".into(),
        },
        at_ms: 1,
        operation: Operation::Rename {
            title: "Renamed".into(),
        },
    }
}

#[test]
fn privacy_is_monotone_on_create_mutation_and_growing_retries() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let review = store
        .create_with_privacy("owner", "Title", Origin::Local, "create", vec![record("a")])
        .unwrap();
    let renamed = store
        .mutate_with_privacy(&review.id, mutation(0), vec![record("b")])
        .unwrap();
    let retried = store
        .mutate_with_privacy(&review.id, mutation(0), vec![record("c")])
        .unwrap();
    assert_eq!(retried.revision, renamed.revision);
    assert_eq!(retried.activity, renamed.activity);
    assert_eq!(retried.privacy, vec![record("a"), record("b"), record("c")]);
    let create_retry = store
        .create_with_privacy("owner", "Title", Origin::Local, "create", vec![record("d")])
        .unwrap();
    assert_eq!(create_retry.revision, 0);
    assert_eq!(create_retry.title, "Title");
    assert_eq!(store.load(&review.id).unwrap().title, "Renamed");
    assert_eq!(
        store.load(&review.id).unwrap().privacy,
        vec![record("a"), record("b"), record("c"), record("d")]
    );
    let mut bad = record("x");
    bad.zone.clear();
    assert!(store
        .mutate_with_privacy(&review.id, mutation(0), vec![bad])
        .is_err());
    assert_eq!(
        ReviewStore::open(root.path())
            .unwrap()
            .load(&review.id)
            .unwrap()
            .privacy
            .len(),
        4
    );
}

fn legacy(root: &std::path::Path) -> (String, Value, BlobRef) {
    let store = ReviewStore::open(root).unwrap();
    let review = store
        .create("owner", "Title", Origin::Local, "create")
        .unwrap();
    let blob = store.put_blob(b"immutable source\n<&>").unwrap();
    drop(store);
    let c = Connection::open(root.join(".refact/reviews/reviews.sqlite")).unwrap();
    c.execute_batch("DELETE FROM review_data_migrations; CREATE TABLE request_intakes(scope TEXT,key TEXT,document TEXT); CREATE TABLE proposal_turn_completions(document TEXT);").unwrap();
    let mut value = serde_json::to_value(review).unwrap();
    value["context_intake"] = receipt("review");
    value["tour_revision_proposals"] = json!([{"id":"retired"}]);
    let version = json!({"version":1,"title":"Old","body":"saved","html":"<p>Exact &lt; bytes</p>","anchors":[],"context_intake":receipt("history")});
    let mut current = version.clone();
    current["version"] = json!(2);
    current["context_intake"] = receipt("current");
    value["tour"] = json!([{"id":"page","current":current,"history":[version]}]);
    value["threads"] = json!([{"id":"thread","anchors":[],"annotation_id":null,"resolved":false,"contributions":[{"id":"proposal-content","thread_id":"thread","actor":{"kind":"agent","id":"owner"},"body":"Saved proposal remains ordinary discussion","created_at_ms":7,"chat_message_id":null}]}]);
    value["materials"] = json!([{"id":"material","kind":"imported","candidate_id":"c","agent_id":null,"description":"saved","content_digest":"digest","input_digests":{"context_intake":"literal","tour_revision_proposals":"literal2"},"environment":null,"worktree_id":null,"worktree_root":null,"artifacts":[blob]}]);
    let id = value["id"].as_str().unwrap().to_owned();
    c.execute(
        "UPDATE reviews SET document=?1 WHERE id=?2",
        params![value.to_string(), id],
    )
    .unwrap();
    let mut object_step = value["tour"][0].clone();
    object_step["current"]["context_intake"] = receipt("object");
    c.execute("INSERT INTO object_revisions VALUES(?1,0,?2)", params![id, json!({"review_id":id,"revision":0,"actor":{"kind":"user","id":"owner"},"at_ms":1,"object":{"kind":"tour_step","value":object_step}}).to_string()]).unwrap();
    let mut response = value.clone();
    response["context_intake"] = receipt("response");
    c.execute("UPDATE requests SET response=?1", [response.to_string()])
        .unwrap();
    c.execute(
        "INSERT INTO request_intakes VALUES('create:owner','create',?1)",
        [receipt("request").to_string()],
    )
    .unwrap();
    (id, value, blob)
}

#[test]
fn legacy_migration_unions_all_stored_sources_and_preserves_content_and_direct_identity() {
    let root = tempfile::tempdir().unwrap();
    let (id, before, blob) = legacy(root.path());
    let store = ReviewStore::open(root.path()).unwrap();
    let review = store.load(&id).unwrap();
    let paths: std::collections::HashSet<_> =
        review.privacy.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        ["review", "history", "current", "object", "response", "request"]
            .into_iter()
            .collect()
    );
    assert_eq!(
        serde_json::to_value(&review.threads).unwrap(),
        before["threads"]
    );
    assert_eq!(
        serde_json::to_value(&review.materials).unwrap(),
        before["materials"]
    );
    assert_eq!(store.read_blob(&blob).unwrap(), b"immutable source\n<&>");
    assert_eq!(
        review.tour[0].current.html.as_deref(),
        Some("<p>Exact &lt; bytes</p>")
    );
    assert_eq!(review.tour[0].history[0].body, "saved");
    assert_eq!(store.object_history(&id).unwrap().len(), 1);
    let retry = store
        .create("owner", "Title", Origin::Local, "create")
        .unwrap();
    assert_eq!(retry.id, id);
    assert_eq!(retry.privacy, review.privacy);
    let c = Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
    for table in [
        "request_intakes",
        "proposal_turn_completions",
        "stack_request_intakes",
    ] {
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name=?1",
                [table],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    for sql in [
        "SELECT document FROM reviews",
        "SELECT document FROM object_revisions",
        "SELECT response FROM requests",
    ] {
        let text: String = c.query_row(sql, [], |r| r.get(0)).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert!(value.get("context_intake").is_none());
        assert!(value.get("tour_revision_proposals").is_none());
        assert!(value.pointer("/tour/0/current/context_intake").is_none());
        assert!(value
            .pointer("/object/value/current/context_intake")
            .is_none());
    }
    assert_eq!(
        ReviewStore::open(root.path()).unwrap().load(&id).unwrap(),
        review
    );
}

#[test]
fn malformed_legacy_provenance_rolls_back_documents_and_table_removal() {
    for malformed in [
        json!(null),
        json!([{"path":"x","zone":"private","attribution":"invented"}]),
        json!([{"path":"x","zone":"","attribution":"declared"}]),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (id, _, _) = legacy(root.path());
        let c = Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
        c.execute(
            "UPDATE request_intakes SET document=?1",
            [json!({"privacy":malformed}).to_string()],
        )
        .unwrap();
        let before: String = c
            .query_row("SELECT document FROM reviews WHERE id=?1", [&id], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(ReviewStore::open(root.path()).is_err());
        assert_eq!(
            c.query_row("SELECT document FROM reviews WHERE id=?1", [&id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            before
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM request_intakes", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM review_data_migrations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn stack_cached_review_migration_preserves_manifest_bytes_and_extracts_history_privacy() {
    for malformed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (id, before, _) = legacy(root.path());
        let db = Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
        let mut cached = before.clone();
        cached["context_intake"] = receipt("stack-cache-only");
        cached["tour"][0]["history"][0]["context_intake"] = receipt("stack-history-only");
        if malformed {
            cached["privacy"] = json!([{"path":"bad","zone":"","attribution":"declared"}]);
        }
        db.execute(
            "INSERT INTO stack_requests VALUES('member','cached','identity',?1)",
            [cached.to_string()],
        )
        .unwrap();
        let manifest = " { \"id\": \"stack\", \"owner_chat_id\": \"owner\", \"members\": [] } ";
        db.execute(
            "INSERT INTO stack_requests VALUES('manifest','cached','manifest-identity',?1)",
            [manifest],
        )
        .unwrap();
        let result = ReviewStore::open(root.path());
        if malformed {
            assert!(result.is_err());
            assert_eq!(
                db.query_row(
                    "SELECT response FROM stack_requests WHERE scope='member'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                cached.to_string()
            );
            assert_eq!(
                db.query_row("SELECT count(*) FROM request_intakes", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                db.query_row("SELECT document FROM reviews WHERE id=?1", [&id], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
                before.to_string()
            );
        } else {
            let store = result.unwrap();
            let current = store.load(&id).unwrap();
            assert!(current.privacy.contains(&record("stack-cache-only")));
            assert!(current.privacy.contains(&record("stack-history-only")));
            let text: String = db
                .query_row(
                    "SELECT response FROM stack_requests WHERE scope='member'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let value: Value = serde_json::from_str(&text).unwrap();
            assert!(value.get("context_intake").is_none());
            assert!(value.get("tour_revision_proposals").is_none());
            assert!(value.pointer("/tour/0/history/0/context_intake").is_none());
            let response: Review = serde_json::from_value(value).unwrap();
            assert_eq!(response.privacy, current.privacy);
            assert_eq!(
                response.tour[0].history[0].html,
                current.tour[0].history[0].html
            );
        }
        assert_eq!(
            db.query_row(
                "SELECT response FROM stack_requests WHERE scope='manifest'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            manifest
        );
    }
}
