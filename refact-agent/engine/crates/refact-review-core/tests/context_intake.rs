use refact_review_core::*;

#[test]
fn candidate_sized_privacy_is_preserved_without_unbounded_receipts() {
    let (root, store, candidate) = fixture();
    let mut intake = receipt(&candidate, false);
    intake.privacy = (0..3045)
        .map(|index| refact_privacy::FileRecord {
            path: format!("/review/project/retained-candidate/source-{index}.rs"),
            zone: "private".into(),
            attribution: refact_privacy::Attribution::Declared,
        })
        .collect();
    assert!(serde_json::to_vec(&intake).unwrap().len() > 256 * 1024);
    validate_context_intake(&intake).unwrap();
    let review = store
        .create_with_intake(
            "owner",
            "Review",
            Origin::Local,
            "large-privacy",
            intake.clone(),
        )
        .unwrap();
    assert_eq!(review.context_intake.as_ref(), Some(&intake));
    let reopened = ReviewStore::open(root.path())
        .unwrap()
        .load(&review.id)
        .unwrap();
    assert_eq!(reopened.context_intake.as_ref(), Some(&intake));
    let mut oversized = intake.clone();
    oversized
        .privacy
        .resize(MAX_FILES + 1, intake.privacy[0].clone());
    assert!(validate_context_intake(&oversized).is_err());
    let mut oversized = intake;
    oversized.privacy[0].path = "x".repeat(MAX_BLOB_BYTES);
    assert!(validate_context_intake(&oversized).is_err());
}

#[test]
fn trusted_history_survives_three_versions_and_old_prefix_replay_without_accepting_forgery() {
    let (_root, store, candidate) = fixture();
    let r = store
        .create("owner", "Review", Origin::Local, "create")
        .unwrap();
    let r = store
        .mutate(
            &r.id,
            mutation(
                &r,
                "stage",
                Operation::StageCandidate {
                    candidate: candidate.clone(),
                },
            ),
        )
        .unwrap();
    let mut r = store
        .mutate(
            &r.id,
            mutation(
                &r,
                "activate",
                Operation::ActivateCandidate {
                    candidate_id: candidate.id.clone(),
                },
            ),
        )
        .unwrap();
    let mut requests = Vec::new();
    for version in 1..=3 {
        let mut step = tour(&candidate, version, None);
        if let Some(old) = r.tour.first() {
            step.history = old.history.clone();
        }
        step.current.body = format!("Explanation {version}");
        let request = mutation(
            &r,
            &format!("version-{version}"),
            Operation::PutTourStep {
                step,
            },
        );
        let mut intake = receipt(&candidate, true);
        intake.id = format!("receipt-{version}");
        intake.privacy = vec![refact_privacy::FileRecord {
            path: "redacted".into(),
            zone: "private".into(),
            attribution: refact_privacy::Attribution::Observed,
        }];
        r = store
            .mutate_with_intake(&r.id, request.clone(), intake.clone())
            .unwrap();
        assert_eq!(r.tour[0].current.context_intake, Some(intake.clone()));
        assert_eq!(
            store
                .mutate_with_intake(&r.id, request.clone(), intake.clone())
                .unwrap(),
            r
        );
        requests.push((request, intake));
    }
    assert_eq!(r.tour[0].history.len(), 2);
    for (index, version) in r.tour[0].history.iter().enumerate() {
        assert_eq!(version.context_intake.as_ref(), Some(&requests[index].1));
    }
    let journal = store.object_history(&r.id).unwrap();
    for (request, intake) in &requests {
        assert_eq!(
            store
                .mutate_with_intake(&r.id, request.clone(), intake.clone())
                .unwrap(),
            r
        );
    }
    let (request, intake) = &requests[2];
    let mut changed = request.clone();
    if let Operation::PutTourStep {
        step,
    } = &mut changed.operation
    {
        step.current.body = "Changed authored content".into();
    }
    assert!(matches!(
        store.mutate_with_intake(&r.id, changed, intake.clone()),
        Err(Error::IdempotencyConflict)
    ));
    let mut changed = intake.clone();
    changed.summary = "Changed intake semantics".into();
    assert!(matches!(
        store.mutate_with_intake(&r.id, request.clone(), changed),
        Err(Error::IdempotencyConflict)
    ));
    for field in ["receipt", "body", "missing_receipt", "extra"] {
        let mut forged = request.clone();
        if let Operation::PutTourStep {
            step,
        } = &mut forged.operation
        {
            match field {
                "receipt" => {
                    step.history[0].context_intake.as_mut().unwrap().summary = "forged".into()
                }
                "body" => step.history[0].body = "forged".into(),
                "missing_receipt" => step.history[0].context_intake = None,
                _ => step.history.push(step.current.clone()),
            }
        }
        assert!(
            store
                .mutate_with_intake(&r.id, forged, intake.clone())
                .is_err(),
            "{field}"
        );
    }
    let mut forged = request.clone();
    if let Operation::PutTourStep {
        step,
    } = &mut forged.operation
    {
        step.current.context_intake = Some(intake.clone());
    }
    assert!(store
        .mutate_with_intake(&r.id, forged, intake.clone())
        .is_err());
    assert_eq!(store.load(&r.id).unwrap(), r);
    assert_eq!(store.object_history(&r.id).unwrap(), journal);
}

fn receipt(candidate: &Candidate, tour: bool) -> ContextIntakeReceipt {
    ContextIntakeReceipt {
        id: if tour { "tour" } else { "review" }.into(),
        phase: if tour { "before_tour" } else { "before_review" }.into(),
        owner_chat_id: "owner".into(),
        repository: candidate.repository.clone(),
        base_oid: candidate.base_oid.clone(),
        head_oid: candidate.head_oid.clone(),
        candidate_id: tour.then(|| candidate.id.clone()),
        captured_at_ms: 100,
        input_reference: ContextInputReference {
            kind: "user_directive".into(),
            message_id: Some("message".into()),
            tool_call_id: None,
            status: "cited_user_intent".into(),
            content_hash: content_hash(b"review this"),
        },
        consultations: vec![],
        gaps: vec![ContextIntakeGap {
            category: "web".into(),
            status: "declined".into(),
            reason: "User declined further intake".into(),
        }],
        summary: "Scoped review context".into(),
        privacy: vec![],
    }
}
fn mutation(r: &Review, key: &str, operation: Operation) -> Mutation {
    Mutation {
        expected_revision: r.revision,
        request_key: key.into(),
        actor: Actor {
            kind: ActorKind::Agent,
            id: "owner".into(),
        },
        at_ms: 100,
        operation,
    }
}
fn fixture() -> (tempfile::TempDir, ReviewStore, Candidate) {
    let root = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(root.path()).unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::new("Test", "test@localhost", &git2::Time::new(1, 0)).unwrap();
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, "fixture", &tree, &[])
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
    (root, store, candidate)
}
fn tour(candidate: &Candidate, version: u64, fallback: Option<String>) -> TourStep {
    TourStep {
        id: "topic".into(),
        current: TourVersion {
            context_intake: None,
            candidate_id: Some(candidate.id.clone()),
            blocks: vec![RichBlock {
                id: "motion".into(),
                revision: 1,
                content: RichBlockContent::Html {
                    html: "<div>Explanation</div>".into(),
                    anchors: vec![],
                    static_fallback: fallback,
                },
            }],
            version,
            title: "Topic".into(),
            body: "Explanation".into(),
            anchors: vec![],
        },
        history: vec![],
    }
}
#[test]
fn receipts_are_atomic_restart_safe_replay_stable_and_historical() {
    let (root, store, candidate) = fixture();
    let before_review = receipt(&candidate, false);
    let r = store
        .create_with_intake(
            "owner",
            "Review",
            Origin::Local,
            "create",
            before_review.clone(),
        )
        .unwrap();
    assert_eq!(r.context_intake, Some(before_review.clone()));
    let mut retry = before_review.clone();
    retry.captured_at_ms += 1;
    assert_eq!(
        store
            .create_with_intake("owner", "Review", Origin::Local, "create", retry)
            .unwrap(),
        r
    );
    let r = store
        .mutate(
            &r.id,
            mutation(
                &r,
                "stage",
                Operation::StageCandidate {
                    candidate: candidate.clone(),
                },
            ),
        )
        .unwrap();
    let r = store
        .mutate(
            &r.id,
            mutation(
                &r,
                "activate",
                Operation::ActivateCandidate {
                    candidate_id: candidate.id.clone(),
                },
            ),
        )
        .unwrap();
    let before_tour = receipt(&candidate, true);
    let request = mutation(
        &r,
        "tour",
        Operation::PutTourStep {
            step: tour(&candidate, 1, Some("Static explanation".into())),
        },
    );
    let r = store
        .mutate_with_intake(&r.id, request.clone(), before_tour.clone())
        .unwrap();
    assert_eq!(r.tour[0].current.context_intake, Some(before_tour.clone()));
    assert_eq!(
        store.intake_for_request(&r.id, "tour").unwrap(),
        Some(before_tour.clone())
    );
    let mut retry = before_tour.clone();
    retry.captured_at_ms += 5;
    assert_eq!(
        store
            .mutate_with_intake(&r.id, request.clone(), retry)
            .unwrap(),
        r
    );
    let mut changed = before_tour.clone();
    changed.summary = "Changed context".into();
    assert!(matches!(
        store.mutate_with_intake(&r.id, request.clone(), changed),
        Err(Error::IdempotencyConflict)
    ));
    let mut next_receipt = before_tour.clone();
    next_receipt.id = "next".into();
    let r = store
        .mutate_with_intake(
            &r.id,
            mutation(
                &r,
                "next",
                Operation::PutTourStep {
                    step: tour(&candidate, 2, Some("New static explanation".into())),
                },
            ),
            next_receipt.clone(),
        )
        .unwrap();
    assert_eq!(
        r.tour[0].history[0].context_intake,
        Some(before_tour.clone())
    );
    assert_eq!(r.tour[0].current.context_intake, Some(next_receipt));
    assert_eq!(store.object_history(&r.id).unwrap().len(), 1);
    drop(store);
    let store = ReviewStore::open(root.path()).unwrap();
    assert_eq!(store.load(&r.id).unwrap(), r);
    assert_eq!(
        store
            .intake_for_request(&format!("create:owner"), "create")
            .unwrap(),
        Some(before_review)
    );
    assert_eq!(
        store
            .mutate_with_intake(&r.id, request, before_tour)
            .unwrap(),
        r
    );
}
#[test]
fn forged_mismatched_and_invalid_fallbacks_rollback_without_echoing_values() {
    let (_root, store, candidate) = fixture();
    let mut bad = receipt(&candidate, false);
    bad.owner_chat_id = "secret-value".into();
    let err = store
        .create_with_intake("owner", "Review", Origin::Local, "bad", bad)
        .unwrap_err();
    assert!(!err.to_string().contains("secret-value"));
    assert!(store.get_for_chat("owner").unwrap().is_none());
    let r = store
        .create("owner", "Review", Origin::Local, "create")
        .unwrap();
    let r = store
        .mutate(
            &r.id,
            mutation(
                &r,
                "stage",
                Operation::StageCandidate {
                    candidate: candidate.clone(),
                },
            ),
        )
        .unwrap();
    let r = store
        .mutate(
            &r.id,
            mutation(
                &r,
                "activate",
                Operation::ActivateCandidate {
                    candidate_id: candidate.id.clone(),
                },
            ),
        )
        .unwrap();
    let valid = receipt(&candidate, true);
    let mut forged = tour(&candidate, 1, None);
    forged.current.context_intake = Some(valid.clone());
    assert!(store
        .mutate(
            &r.id,
            mutation(
                &r,
                "forged",
                Operation::PutTourStep {
                    step: forged
                }
            )
        )
        .is_err());
    for field in ["owner", "phase", "candidate", "head", "repository"] {
        let mut bad = valid.clone();
        match field {
            "owner" => bad.owner_chat_id = "other".into(),
            "phase" => bad.phase = "before_review".into(),
            "candidate" => bad.candidate_id = Some("other".into()),
            "head" => bad.head_oid = "other".into(),
            _ => bad.repository = "other".into(),
        }
        assert!(store
            .mutate_with_intake(
                &r.id,
                mutation(
                    &r,
                    field,
                    Operation::PutTourStep {
                        step: tour(&candidate, 1, None)
                    }
                ),
                bad
            )
            .is_err());
    }
    for fallback in [" ".into(), "x".repeat(65_537)] {
        assert!(store
            .mutate_with_intake(
                &r.id,
                mutation(
                    &r,
                    "fallback",
                    Operation::PutTourStep {
                        step: tour(&candidate, 1, Some(fallback))
                    }
                ),
                valid.clone()
            )
            .is_err());
    }
    assert_eq!(store.load(&r.id).unwrap(), r);
    assert!(store
        .intake_for_request(&r.id, "fallback")
        .unwrap()
        .is_none());
    let mut wrong_version = tour(&candidate, 1, None);
    wrong_version.current.candidate_id = None;
    assert!(store
        .mutate_with_intake(
            &r.id,
            mutation(
                &r,
                "wrong-version",
                Operation::PutTourStep {
                    step: wrong_version
                }
            ),
            valid
        )
        .is_err());
}
#[test]
fn old_serialization_and_receipt_bounds_are_explicit() {
    let (_root, store, candidate) = fixture();
    let r = store
        .create("owner", "Review", Origin::Local, "create")
        .unwrap();
    let json = serde_json::to_value(&r).unwrap();
    assert!(json.get("context_intake").is_none());
    assert!(serde_json::from_value::<Review>(json)
        .unwrap()
        .context_intake
        .is_none());
    let old_html = serde_json::json!({"kind":"html", "html":"<p>Old</p>"});
    assert!(matches!(
        serde_json::from_value::<RichBlockContent>(old_html).unwrap(),
        RichBlockContent::Html {
            static_fallback: None,
            ..
        }
    ));
    let version = tour(&candidate, 1, None).current;
    let json = serde_json::to_value(version).unwrap();
    assert!(json.get("context_intake").is_none());
    assert!(serde_json::from_value::<TourVersion>(json)
        .unwrap()
        .context_intake
        .is_none());
    let good = receipt(&candidate, false);
    validate_context_intake(&good).unwrap();
    let mut bad = good.clone();
    bad.gaps[0].category = "everything".into();
    assert!(validate_context_intake(&bad).is_err());
    let mut bad = good.clone();
    bad.gaps[0].status = "retrieved".into();
    assert!(validate_context_intake(&bad).is_err());
    let mut bad = good.clone();
    bad.phase = "anytime".into();
    assert!(validate_context_intake(&bad).is_err());
    let mut bad = good;
    bad.summary = "x".repeat(16_385);
    assert!(validate_context_intake(&bad).is_err());
}

#[test]
fn stack_receipts_publish_atomically_and_recover_for_replay() {
    let (root, store, mut candidate) = fixture();
    candidate = store
        .capture(
            root.path(),
            CaptureRequest {
                base_oid: candidate.base_oid.clone(),
                head_oid: candidate.head_oid.clone(),
                policy: CapturePolicy::default(),
                origin: Origin::Imported,
            },
        )
        .unwrap();
    let ids: Vec<_> = (1..=2)
        .map(|n| format!("00000000-0000-4000-8000-{n:012}"))
        .collect();
    let manifest = StackManifest {
        schema_version: SCHEMA_VERSION,
        id: "stack".into(),
        owner_chat_id: "owner".into(),
        revision: 0,
        title: "Stack".into(),
        repository: root.path().to_str().unwrap().into(),
        source: StackSource {
            kind: "provided_manifest".into(),
            reference: Some("fixture".into()),
        },
        root_base_ref: candidate.base_oid.clone(),
        root_base_oid: Some(candidate.base_oid.clone()),
        scope_choice: StackScopeChoice {
            scope: StackScope::Whole,
            selected_member_id: ids[0].clone(),
            confirmed: true,
            explicit_request: true,
        },
        members: ids
            .into_iter()
            .enumerate()
            .map(|(i, id)| StackMember {
                id,
                branch_ref: format!("branch-{i}"),
                pr_identity: None,
                parent_member_id: None,
                declared_base_ref: candidate.base_oid.clone(),
                resolved_base_oid: Some(candidate.base_oid.clone()),
                head_oid: Some(candidate.base_oid.clone()),
                review_id: None,
                state: StackMemberState::Ready,
                reason: None,
            })
            .collect(),
    };

    store
        .put_stack_preparation("owner", "A", &manifest)
        .unwrap();
    let candidates: Vec<_> = manifest
        .members
        .iter()
        .map(|m| (m.id.clone(), candidate.clone()))
        .collect();
    let intakes: Vec<_> = manifest
        .members
        .iter()
        .map(|m| (m.id.clone(), receipt(&candidate, false)))
        .collect();
    let mut bad = intakes.clone();
    bad[1].1.head_oid = "wrong".into();
    assert!(store
        .publish_stack_import_with_intakes(
            manifest.clone(),
            "A",
            "publish",
            candidates.clone(),
            bad
        )
        .is_err());
    assert!(store.list_for_chat("owner").unwrap().is_empty());
    assert!(store
        .intakes_for_stack_request("owner", "A", "publish")
        .unwrap()
        .is_none());
    let result = store
        .publish_stack_import_with_intakes(
            manifest.clone(),
            "A",
            "publish",
            candidates.clone(),
            intakes.clone(),
        )
        .unwrap();
    for member in &result.members {
        assert_eq!(
            store
                .load(member.review_id.as_ref().unwrap())
                .unwrap()
                .context_intake,
            Some(receipt(&candidate, false))
        );
    }
    assert_eq!(
        store
            .intakes_for_stack_request("owner", "A", "publish")
            .unwrap(),
        Some(intakes.clone())
    );
    let mut retries = intakes.clone();
    for (_, receipt) in &mut retries {
        receipt.captured_at_ms += 5;
    }
    assert_eq!(
        store
            .publish_stack_import_with_intakes(
                manifest.clone(),
                "A",
                "publish",
                candidates.clone(),
                retries
            )
            .unwrap(),
        result
    );
    let mut changed = intakes.clone();
    changed[0].1.summary = "Different".into();
    assert!(matches!(
        store.publish_stack_import_with_intakes(manifest, "A", "publish", candidates, changed),
        Err(Error::IdempotencyConflict)
    ));
    drop(store);
    let store = ReviewStore::open(root.path()).unwrap();
    assert_eq!(
        store
            .intakes_for_stack_request("owner", "A", "publish")
            .unwrap(),
        Some(intakes)
    );
}
