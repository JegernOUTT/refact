use crate::caps::BaseModelRecord;

fn provider_id(model_rec: &BaseModelRecord) -> Result<&str, String> {
    model_rec
        .id
        .split_once('/')
        .map(|(provider_id, _)| provider_id)
        .filter(|provider_id| !provider_id.is_empty())
        .ok_or_else(|| "Command credential provider identity is unavailable".to_string())
}

pub async fn resolve(model_rec: &BaseModelRecord) -> Result<Option<String>, String> {
    let Some(spec) = model_rec.credential.as_ref() else {
        return Ok(None);
    };
    refact_providers::credential::resolve(provider_id(model_rec)?, spec, false)
        .await
        .map(Some)
}

pub async fn refresh_after_rejection(
    model_rec: &BaseModelRecord,
    rejected_value: &str,
) -> Result<Option<String>, String> {
    let Some(spec) = model_rec.credential.as_ref() else {
        return Ok(None);
    };
    refact_providers::credential::refresh_after_rejection(
        provider_id(model_rec)?,
        spec,
        rejected_value,
    )
    .await
    .map_err(|error| redact(error, Some(rejected_value)))
    .map(Some)
}

pub fn redact(text: impl Into<String>, credential: Option<&str>) -> String {
    let text = text.into();
    match credential.filter(|value| !value.is_empty()) {
        Some(value) => text.replace(value, "[REDACTED]"),
        None => text,
    }
}
