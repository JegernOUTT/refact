use refact_review_core::*;

fn actor() -> Actor {
    Actor {
        kind: ActorKind::Agent,
        id: "owner".into(),
    }
}

fn mutate(store: &ReviewStore, review: &Review, key: &str, operation: Operation) -> Review {
    store
        .mutate(
            &review.id,
            Mutation {
                expected_revision: review.revision,
                request_key: key.into(),
                actor: actor(),
                at_ms: 1,
                operation,
            },
        )
        .unwrap()
}

#[test]
fn html_pages_update_directly_preserve_exact_bytes_and_legacy_history() {
    let root = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(root.path()).unwrap();
    let tree = repo
        .find_tree(repo.index().unwrap().write_tree().unwrap())
        .unwrap();
    let signature = git2::Signature::new("Test", "test@localhost", &git2::Time::new(1, 0)).unwrap();
    let oid = repo
        .commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
        .unwrap()
        .to_string();
    let store = ReviewStore::open(root.path()).unwrap();
    let candidate = store
        .capture(
            root.path(),
            CaptureRequest {
                base_oid: oid.clone(),
                head_oid: oid,
                policy: CapturePolicy::default(),
                origin: Origin::Local,
            },
        )
        .unwrap();
    let review = store
        .create("owner", "HTML Tour", Origin::Local, "create")
        .unwrap();
    let review = mutate(
        &store,
        &review,
        "stage",
        Operation::StageCandidate {
            candidate: candidate.clone(),
        },
    );
    let review = mutate(
        &store,
        &review,
        "activate",
        Operation::ActivateCandidate {
            candidate_id: candidate.id.clone(),
        },
    );
    let legacy: TourVersion = serde_json::from_value(serde_json::json!({
        "candidate_id": candidate.id, "version": 1, "title": "Old explanation", "body": "Legacy Markdown", "anchors": []
    })).unwrap();
    assert_eq!(legacy.html, None);
    let review = mutate(
        &store,
        &review,
        "legacy",
        Operation::PutTourStep {
            step: TourStep {
                id: "overview.html".into(),
                current: legacy.clone(),
                history: vec![],
            },
        },
    );
    let html = "<!doctype html><html><body><button id=next>Next</button><refact-slot data-block=source></refact-slot><script>const state={value:'<&'};document.getElementById('next').onclick=()=>state.value+='!';</script></body></html>";
    let page = TourVersion {
        html: Some(html.into()),
        blocks: vec![],
        candidate_id: Some(candidate.id.clone()),
        version: 2,
        title: "Visual overview".into(),
        body: "Short static alternative".into(),
        anchors: vec![],
    };
    let request = Mutation {
        expected_revision: review.revision,
        request_key: "visual-update".into(),
        actor: actor(),
        at_ms: 2,
        operation: Operation::PutTourStep {
            step: TourStep {
                id: "overview.html".into(),
                current: page,
                history: vec![],
            },
        },
    };
    let updated = store.mutate(&review.id, request.clone()).unwrap();
    assert_eq!(updated.tour[0].current.html.as_deref(), Some(html));
    assert_eq!(updated.tour[0].history, vec![legacy]);
    assert!(updated.threads.is_empty());
    assert_eq!(store.mutate(&review.id, request).unwrap(), updated);
    assert_eq!(
        ReviewStore::open(root.path())
            .unwrap()
            .load(&review.id)
            .unwrap(),
        updated
    );
    for html in [String::new(), " ".into(), "x".repeat(1024 * 1024 + 1)] {
        let mut step = updated.tour[0].clone();
        step.current.version = 3;
        step.current.html = Some(html);
        let attempt = Mutation {
            expected_revision: updated.revision,
            request_key: "invalid-page".into(),
            actor: actor(),
            at_ms: 3,
            operation: Operation::PutTourStep {
                step,
            },
        };
        assert!(store.mutate(&review.id, attempt).is_err());
    }
    for forge in ["body", "html", "append"] {
        let mut step = updated.tour[0].clone();
        step.current.version = 3;
        match forge {
            "body" => step.history[0].body = "forged".into(),
            "html" => step.history[0].html = Some("<p>forged</p>".into()),
            _ => step.history.push(step.current.clone()),
        }
        assert!(store
            .mutate(
                &review.id,
                Mutation {
                    expected_revision: updated.revision,
                    request_key: format!("tamper-{forge}"),
                    actor: actor(),
                    at_ms: 4,
                    operation: Operation::PutTourStep {
                        step
                    },
                }
            )
            .is_err());
    }
    assert_eq!(store.load(&review.id).unwrap(), updated);
}
