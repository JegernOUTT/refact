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
    let policy = gcx
        .privacy_policy_load
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .policy
        .clone();
    let compiled = policy.compile().map_err(|error| refact_privacy::Refusal {
        destination: destination.clone(),
        offending: Vec::new(),
        message: format!("privacy policy failed to compile: {error}"),
    })?;
    refact_privacy::clear(value, &destination, &compiled)
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
    let policy = gcx
        .privacy_policy_load
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .policy
        .clone();
    let compiled = policy.compile().map_err(|error| refact_privacy::Refusal {
        destination: destination.clone(),
        offending: Vec::new(),
        message: format!("privacy policy failed to compile: {error}"),
    })?;
    refact_privacy::clear(value, &destination, &compiled)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(refusal.message.contains("privacy policy failed to compile"));
    }
}
