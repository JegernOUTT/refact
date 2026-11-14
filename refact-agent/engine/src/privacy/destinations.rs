use refact_privacy::{Cleared, Destination, DestinationId, DestinationKind, PrivacyAudited};

use crate::caps::{BaseModelRecord, EmbeddingModelRecord};
use crate::global_context::SharedGlobalContext;

pub(crate) trait DestinationExt<T> {
    fn from_model_record(model_rec: &T) -> Destination;
}

impl DestinationExt<BaseModelRecord> for Destination {
    fn from_model_record(model_rec: &BaseModelRecord) -> Destination {
        Destination {
            id: DestinationId(
                model_rec
                    .id
                    .split_once('/')
                    .map_or(model_rec.id.as_str(), |(provider, _)| provider)
                    .to_string(),
            ),
            kind: DestinationKind::Provider,
            display_name: model_rec.id.clone(),
        }
    }
}

impl DestinationExt<EmbeddingModelRecord> for Destination {
    fn from_model_record(model_rec: &EmbeddingModelRecord) -> Destination {
        Destination::from_model_record(&model_rec.base)
    }
}

pub(crate) fn clear_for_model<T: PrivacyAudited>(
    gcx: &SharedGlobalContext,
    value: T,
    model_rec: &BaseModelRecord,
) -> Result<Cleared<T>, refact_privacy::Refusal> {
    let destination = Destination::from_model_record(model_rec);
    clear_current(gcx, value, &destination)
}

pub(crate) fn clear_current<T: PrivacyAudited>(
    gcx: &SharedGlobalContext,
    value: T,
    destination: &Destination,
) -> Result<Cleared<T>, refact_privacy::Refusal> {
    let refuse = |message: String| refact_privacy::Refusal {
        destination: destination.clone(),
        offending: Vec::new(),
        message,
    };
    let records = value
        .privacy_records()
        .map_err(|error| refuse(error.message))?;
    for (_, record) in &records {
        if record.path.is_empty() || record.path.contains('\0') {
            return Err(refuse("malformed privacy record path".into()));
        }
    }
    struct Audit(Vec<(usize, refact_privacy::FileRecord)>);
    impl PrivacyAudited for Audit {
        fn privacy_records(
            &self,
        ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError>
        {
            Ok(self.0.clone())
        }
    }
    const MAX_ATTEMPTS: usize = 3;
    for attempt in 0..MAX_ATTEMPTS {
        let snapshot = match super::classification_cache::snapshot(gcx) {
            Ok(snapshot) => snapshot,
            Err(message)
                if message == refact_privacy::gate::PRIVACY_CONTEXT_CHANGED
                    && attempt + 1 < MAX_ATTEMPTS =>
            {
                continue;
            }
            Err(message) => return Err(refuse(message)),
        };
        let mut refreshed = records.clone();
        for (index, record) in &records {
            let classified = snapshot.classify(gcx, std::path::Path::new(&record.path));
            let mut current = record.clone();
            current.zone = classified.zone.name;
            refreshed.push((*index, current));
        }
        refact_privacy::clear(Audit(refreshed), destination, &snapshot.compiled)?;
        #[cfg(test)]
        super::classification_cache::observe(gcx, "clearance");
        match snapshot.finish(gcx) {
            Ok(()) => return refact_privacy::clear(value, destination, &snapshot.compiled),
            Err(message)
                if message == refact_privacy::gate::PRIVACY_CONTEXT_CHANGED
                    && attempt + 1 < MAX_ATTEMPTS => {}
            Err(message) => return Err(refuse(message)),
        }
    }
    unreachable!()
}

pub(crate) fn clear_for_mcp<T: PrivacyAudited>(
    gcx: &SharedGlobalContext,
    value: T,
    server_name: &str,
) -> Result<Cleared<T>, refact_privacy::Refusal> {
    let destination = Destination {
        id: DestinationId(server_name.to_string()),
        kind: DestinationKind::Mcp,
        display_name: server_name.to_string(),
    };
    clear_current(gcx, value, &destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct CountedAudit {
        calls: Arc<AtomicUsize>,
        records:
            Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError>,
    }

    impl PrivacyAudited for CountedAudit {
        fn privacy_records(
            &self,
        ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError>
        {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.records.clone()
        }
    }

    fn counted_audit(gcx: &SharedGlobalContext) -> (CountedAudit, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let record = refact_privacy::FileRecord {
            path: gcx
                .cache_dir
                .join("payload.txt")
                .to_string_lossy()
                .into_owned(),
            zone: "normal".into(),
            attribution: refact_privacy::Attribution::Declared,
        };
        (
            CountedAudit {
                calls: calls.clone(),
                records: Ok(vec![(7, record)]),
            },
            calls,
        )
    }

    #[tokio::test]
    async fn local_clearance_recovers_once_at_snapshot_or_finish_without_reauditing_retries() {
        for phase in ["snapshot", "clearance"] {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            let (audit, calls) = counted_audit(&gcx);
            let observations = Arc::new(AtomicUsize::new(0));
            let count = observations.clone();
            *gcx.privacy_classification.observer.lock().unwrap() =
                Some(Box::new(move |gcx, event| {
                    if event == phase && count.fetch_add(1, Ordering::SeqCst) == 0 {
                        super::super::classification_cache::invalidate(gcx);
                    }
                }));

            let cleared = clear_for_mcp(&gcx, audit, "other").unwrap_or_else(|e| panic!("{e}"));

            assert_eq!(cleared.records.as_ref().unwrap()[0].0, 7);
            assert_eq!(observations.load(Ordering::SeqCst), 2, "{phase}");
            assert_eq!(
                calls.load(Ordering::SeqCst),
                2,
                "initial and final audit only"
            );
            assert_eq!(
                gcx.privacy_classification
                    .classifications
                    .load(Ordering::SeqCst),
                if phase == "snapshot" { 1 } else { 2 }
            );
        }
    }

    #[tokio::test]
    async fn local_clearance_context_churn_exhausts_three_total_attempts() {
        for phase in ["snapshot", "clearance", "mixed"] {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            let (audit, calls) = counted_audit(&gcx);
            let changes = Arc::new(AtomicUsize::new(0));
            let count = changes.clone();
            *gcx.privacy_classification.observer.lock().unwrap() =
                Some(Box::new(move |gcx, event| {
                    let next = count.load(Ordering::SeqCst);
                    let change_here = event == phase
                        || (phase == "mixed"
                            && event == if next == 1 { "clearance" } else { "snapshot" });
                    if change_here {
                        count.fetch_add(1, Ordering::SeqCst);
                        super::super::classification_cache::invalidate(gcx);
                    }
                }));

            let refusal = clear_for_mcp(&gcx, audit, "other").err().unwrap();

            assert_eq!(changes.load(Ordering::SeqCst), 3, "{phase}");
            assert_eq!(
                calls.load(Ordering::SeqCst),
                1,
                "must not consume or re-audit on churn"
            );
            assert!(refusal.offending.is_empty());
            assert_eq!(
                refusal.message,
                refact_privacy::gate::PRIVACY_CONTEXT_CHANGED
            );
            assert_eq!(refusal.model_facing(), "Request not sent: user privacy policy — privacy context kept changing, so verification could not complete. Try again once the privacy context is stable.");
        }
    }

    #[tokio::test]
    async fn local_clearance_retry_observes_tightened_patterns_and_carried_restrictions() {
        for carried in [false, true] {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            let (mut audit, calls) = counted_audit(&gcx);
            if carried {
                audit.records.as_mut().unwrap()[0].1.zone = "secrets".into();
                gcx.privacy_policy_load.write().unwrap().policy =
                    Arc::new(refact_privacy::PrivacyPolicy {
                        zones: vec![refact_privacy::Zone {
                            name: "secrets".into(),
                            patterns: vec!["unrelated.txt".into()],
                            send_to: vec!["*".into()],
                            on_shell_read: refact_privacy::ShellBehavior::Withhold,
                        }],
                        ..Default::default()
                    });
            }
            let changes = Arc::new(AtomicUsize::new(0));
            let count = changes.clone();
            *gcx.privacy_classification.observer.lock().unwrap() =
                Some(Box::new(move |gcx, event| {
                    if event == "clearance" {
                        count.fetch_add(1, Ordering::SeqCst);
                        gcx.privacy_policy_load.write().unwrap().policy =
                            Arc::new(refact_privacy::PrivacyPolicy {
                                blocked: if carried {
                                    vec![]
                                } else {
                                    vec!["payload.txt".into()]
                                },
                                zones: if carried {
                                    vec![refact_privacy::Zone {
                                        name: "secrets".into(),
                                        patterns: vec!["unrelated.txt".into()],
                                        send_to: vec!["trusted".into()],
                                        on_shell_read: refact_privacy::ShellBehavior::Withhold,
                                    }]
                                } else {
                                    vec![]
                                },
                                ..Default::default()
                            });
                    }
                }));

            let model = BaseModelRecord {
                id: "other/model".into(),
                ..Default::default()
            };
            let refusal = clear_for_model(&gcx, audit, &model).err().unwrap();

            assert_eq!(refusal.offending.len(), 1);
            assert_eq!(refusal.offending[0].0, 7);
            assert_eq!(
                refusal.offending[0].1.zone,
                if carried { "secrets" } else { "blocked" }
            );
            assert_eq!(changes.load(Ordering::SeqCst), 1, "denial must not retry");
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                gcx.privacy_classification
                    .classifications
                    .load(Ordering::SeqCst),
                2
            );
        }
    }

    #[tokio::test]
    async fn local_clearance_does_not_retry_bad_metadata_paths_or_unavailable_policy() {
        for failure in ["metadata", "path", "policy", "finish-policy"] {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            let (mut audit, calls) = counted_audit(&gcx);
            match failure {
                "metadata" => {
                    let mut message =
                        refact_core::chat_types::ChatMessage::new("tool".into(), "result".into());
                    message.extra.insert(
                        "privacy".into(),
                        serde_json::json!({"files": "private/internal/path"}),
                    );
                    audit.records = refact_privacy::records_from_messages(&[message]);
                    assert!(audit.records.is_err());
                }
                "path" => audit.records.as_mut().unwrap()[0].1.path.clear(),
                "policy" => {
                    gcx.privacy_policy_load.write().unwrap().error =
                        Some("private/internal/path".into())
                }
                _ => {}
            }
            let events = Arc::new(AtomicUsize::new(0));
            let count = events.clone();
            *gcx.privacy_classification.observer.lock().unwrap() =
                Some(Box::new(move |gcx, event| {
                    count.fetch_add(1, Ordering::SeqCst);
                    if failure == "finish-policy" && event == "clearance" {
                        gcx.privacy_policy_load.write().unwrap().error =
                            Some("private/internal/path".into());
                    }
                }));

            let refusal = clear_for_mcp(&gcx, audit, "other").err().unwrap();

            assert!(refusal.offending.is_empty());
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                events.load(Ordering::SeqCst),
                if failure == "finish-policy" { 2 } else { 0 }
            );
            assert_eq!(refusal.model_facing(), "Request not sent: user privacy policy — privacy metadata or policy could not be verified. Resolve the privacy verification problem before trying again.");
        }
    }

    #[tokio::test]
    async fn local_clearance_final_audit_failure_still_refuses_without_retry() {
        struct FinalAuditFailure(CountedAudit);
        impl PrivacyAudited for FinalAuditFailure {
            fn privacy_records(
                &self,
            ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError>
            {
                if self.0.calls.load(Ordering::SeqCst) > 0 {
                    self.0.calls.fetch_add(1, Ordering::SeqCst);
                    return Err(refact_privacy::PrivacyAuditError {
                        message_index: 7,
                        message: "final audit failed at /private/internal/path".into(),
                    });
                }
                self.0.privacy_records()
            }
        }
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (audit, calls) = counted_audit(&gcx);
        let refusal = clear_for_mcp(&gcx, FinalAuditFailure(audit), "other")
            .err()
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(refusal.offending.is_empty());
        assert_eq!(
            refusal.message,
            "final audit failed at /private/internal/path"
        );
        assert_eq!(refusal.model_facing(), "Request not sent: user privacy policy — privacy metadata or policy could not be verified. Resolve the privacy verification problem before trying again.");
    }

    struct Records(Vec<refact_privacy::FileRecord>);
    impl PrivacyAudited for Records {
        fn privacy_records(
            &self,
        ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError>
        {
            Ok(self.0.iter().cloned().enumerate().collect())
        }
    }

    #[tokio::test]
    async fn egress_refreshes_patterns_and_keeps_carried_obligations() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let path = gcx.cache_dir.join("sensitive.txt");
        std::fs::write(&path, "fixture").unwrap();
        let record = crate::privacy::records::declared_file_record(&gcx, &path).unwrap();
        let trusted = BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        };
        let other = BaseModelRecord {
            id: "other/model".into(),
            ..Default::default()
        };
        assert!(clear_for_model(&gcx, Records(vec![record.clone()]), &other).is_ok());
        gcx.privacy_policy_load.write().unwrap().policy =
            std::sync::Arc::new(refact_privacy::PrivacyPolicy {
                zones: vec![refact_privacy::Zone {
                    name: "secrets".into(),
                    patterns: vec!["sensitive.txt".into()],
                    send_to: vec!["trusted".into()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                }],
                ..Default::default()
            });
        assert!(clear_for_model(&gcx, Records(vec![record.clone()]), &trusted).is_ok());
        assert!(clear_for_model(&gcx, Records(vec![record.clone()]), &other).is_err());
        let mut carried = record.clone();
        carried.zone = "effective:secrets+removed".into();
        assert!(clear_for_model(&gcx, Records(vec![carried.clone()]), &trusted).is_ok());
        assert!(clear_for_model(&gcx, Records(vec![carried.clone()]), &other).is_err());
        gcx.privacy_policy_load.write().unwrap().policy =
            Arc::new(refact_privacy::PrivacyPolicy::default());
        assert!(clear_for_model(&gcx, Records(vec![carried]), &other).is_ok());
        let mut malformed = record.clone();
        malformed.path.clear();
        assert!(clear_for_model(&gcx, Records(vec![malformed]), &trusted).is_err());
        gcx.privacy_policy_load.write().unwrap().error = Some("fixture failure".into());
        assert!(clear_for_model(&gcx, Records(vec![record]), &trusted).is_err());
    }

    struct EmptyAudit;

    impl PrivacyAudited for EmptyAudit {
        fn privacy_records(
            &self,
        ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError>
        {
            Ok(Vec::new())
        }
    }

    #[test]
    fn model_destination_uses_provider_prefix() {
        let model = BaseModelRecord {
            id: "trusted/model".to_string(),
            ..Default::default()
        };

        let destination = Destination::from_model_record(&model);

        assert_eq!(destination.id.0, "trusted");
        assert_eq!(destination.kind, DestinationKind::Provider);
        assert_eq!(destination.display_name, "trusted/model");
    }

    #[test]
    fn embedding_destination_uses_provider_prefix() {
        let model = EmbeddingModelRecord {
            base: BaseModelRecord {
                id: "embeddings/text-v1".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let destination = Destination::from_model_record(&model);

        assert_eq!(destination.id.0, "embeddings");
        assert_eq!(destination.kind, DestinationKind::Provider);
        assert_eq!(destination.display_name, "embeddings/text-v1");
    }

    #[tokio::test]
    async fn poisoned_policy_lock_with_invalid_state_fails_closed() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let poison = gcx.clone();
        let _ = std::thread::spawn(move || {
            let mut load = poison.privacy_policy_load.write().unwrap();
            load.policy = std::sync::Arc::new(refact_privacy::PrivacyPolicy {
                blocked: Vec::new(),
                zones: vec![refact_privacy::Zone {
                    name: "normal".to_string(),
                    patterns: vec!["[".to_string()],
                    send_to: vec!["*".to_string()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                }],
                subagents: refact_privacy::SubagentPolicy::default(),
                ..Default::default()
            });
            panic!("poison privacy policy lock");
        })
        .join();
        let model = BaseModelRecord {
            id: "trusted/model".to_string(),
            ..Default::default()
        };

        let refusal = match clear_for_model(&gcx, EmptyAudit, &model) {
            Ok(_) => panic!("invalid recovered policy must fail closed"),
            Err(refusal) => refusal,
        };

        assert!(refusal.offending.is_empty());
        assert!(refusal.message.contains("privacy policy unavailable"));
    }
}
