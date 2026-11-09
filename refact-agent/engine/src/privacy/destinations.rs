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

fn clear_current<T: PrivacyAudited>(
    gcx: &SharedGlobalContext,
    value: T,
    destination: &Destination,
) -> Result<Cleared<T>, refact_privacy::Refusal> {
    let refuse = |message: String| refact_privacy::Refusal {
        destination: destination.clone(),
        offending: Vec::new(),
        message,
    };
    let snapshot = super::classification_cache::snapshot(gcx).map_err(&refuse)?;
    let mut records = value
        .privacy_records()
        .map_err(|error| refuse(error.message))?;
    let mut refreshed = Vec::with_capacity(records.len());
    for (index, record) in &records {
        if record.path.is_empty() || record.path.contains('\0') {
            return Err(refuse("malformed privacy record path".into()));
        }
        let classified = snapshot.classify(gcx, std::path::Path::new(&record.path));
        let mut current = record.clone();
        current.zone = classified.zone.name;
        refreshed.push((*index, current));
    }
    records.extend(refreshed);
    struct Audit(Vec<(usize, refact_privacy::FileRecord)>);
    impl PrivacyAudited for Audit {
        fn privacy_records(
            &self,
        ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError>
        {
            Ok(self.0.clone())
        }
    }
    refact_privacy::clear(Audit(records), destination, &snapshot.compiled)?;
    snapshot.finish(gcx).map_err(&refuse)?;
    refact_privacy::clear(value, destination, &snapshot.compiled)
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
        assert!(clear_for_model(&gcx, Records(vec![carried]), &trusted).is_err());
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
