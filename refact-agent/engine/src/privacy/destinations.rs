use refact_privacy::{Cleared, Destination, DestinationId, DestinationKind, PrivacyAudited};

use crate::caps::BaseModelRecord;
use crate::global_context::SharedGlobalContext;

pub(crate) trait DestinationExt {
    fn from_model_record(model_rec: &BaseModelRecord) -> Destination;
}

impl DestinationExt for Destination {
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

pub(crate) fn clear_for_model<T: PrivacyAudited>(
    gcx: &SharedGlobalContext,
    value: T,
    model_rec: &BaseModelRecord,
) -> Result<Cleared<T>, refact_privacy::Refusal> {
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    refact_privacy::clear(value, &Destination::from_model_record(model_rec), &policy)
}

pub(crate) fn clear_for_mcp<T: PrivacyAudited>(
    gcx: &SharedGlobalContext,
    value: T,
    server_name: &str,
) -> Result<Cleared<T>, refact_privacy::Refusal> {
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    let destination = Destination {
        id: DestinationId(server_name.to_string()),
        kind: DestinationKind::Mcp,
        display_name: server_name.to_string(),
    };
    refact_privacy::clear(value, &destination, &policy)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
