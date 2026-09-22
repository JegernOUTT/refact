use crate::app_state::AppState;
use refact_buddy_core::operator_types::ProjectIdentity;
pub use refact_core::provider_types::{defaults_compare_exchange, defaults_snapshot, DefaultsScope};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultSlot {
    Chat,
    ChatModel2,
    TaskPlannerAgentModel,
    ChatLight,
    ChatThinking,
    ChatBuddy,
}
impl DefaultSlot {
    pub fn key(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::ChatModel2 => "chat_model_2",
            Self::TaskPlannerAgentModel => "task_planner_agent_model",
            Self::ChatLight => "chat_light",
            Self::ChatThinking => "chat_thinking",
            Self::ChatBuddy => "chat_buddy",
        }
    }
}
pub async fn scope_root(
    app: &AppState,
    project: &ProjectIdentity,
    scope: DefaultsScope,
) -> Result<PathBuf, String> {
    let root = PathBuf::from(&project.canonical_root);
    if !crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .contains(&root)
    {
        return Err("project scope mismatch".into());
    }
    Ok(match scope {
        DefaultsScope::Global => app.paths.config_dir.clone(),
        DefaultsScope::Project => root,
    })
}
pub fn patch(
    bytes: &Option<Vec<u8>>,
    slot: DefaultSlot,
    expected_selected: &Option<String>,
    model: &str,
) -> Result<Vec<u8>, String> {
    if model.trim().is_empty()
        || model.trim() != model
        || !model.contains('/')
        || refact_core::provider_types::is_legacy_refact_model(model)
    {
        return Err("explicit catalog model required".into());
    }
    let mut normalized = refact_core::provider_types::ProjectModelDefaults::default();
    normalized.chat = Some(refact_core::provider_types::ModelTypeDefaults {
        model: Some(model.into()),
        ..Default::default()
    });
    normalized.normalize();
    if normalized
        .chat
        .as_ref()
        .and_then(|slot| slot.model.as_deref())
        != Some(model)
    {
        return Err("model would be normalized away".into());
    }
    let mut value: serde_yaml::Value = match bytes {
        Some(b) => serde_yaml::from_slice(b).map_err(|_| "invalid defaults")?,
        None => serde_yaml::Value::Mapping(Default::default()),
    };
    let map = value.as_mapping_mut().ok_or("invalid defaults")?;
    let key = serde_yaml::Value::String(slot.key().into());
    if map.get(&key).is_some_and(serde_yaml::Value::is_null) {
        map.remove(&key);
    }
    let slot_value = map
        .entry(key)
        .or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
    let fields = slot_value.as_mapping_mut().ok_or("invalid default slot")?;
    let model_key = serde_yaml::Value::String("model".into());
    let selected = fields
        .get(&model_key)
        .map(|v| v.as_str().map(String::from).ok_or("invalid selected model"))
        .transpose()?;
    if &selected != expected_selected {
        return Err("selected model mismatch".into());
    }
    fields.insert(model_key, serde_yaml::Value::String(model.into()));
    serde_yaml::to_string(&value)
        .map(String::into_bytes)
        .map_err(|_| "invalid defaults".into())
}
pub async fn validate_candidate(app: &AppState, model: &str) -> Result<(), String> {
    let (instance, id) = model
        .split_once('/')
        .ok_or("explicit catalog model required")?;
    let registry = app.gcx.providers.read().await;
    let provider = registry.get(instance).ok_or("provider unavailable")?;
    let runtime = provider
        .build_runtime()
        .map_err(|_| "provider configuration invalid")?;
    let custom_available = provider
        .get_custom_models_only()
        .iter()
        .any(|m| m.id == id && m.enabled && m.n_ctx > 0);
    let selected = provider
        .enabled_models()
        .iter()
        .any(|m| m == id || m == model)
        && !provider
            .disabled_models()
            .iter()
            .any(|m| m == id || m == model);
    drop(registry);
    let caps = app.model.caps.read().await;
    let catalog_available = caps
        .caps
        .as_ref()
        .is_some_and(|caps| caps.chat_models.contains_key(model));
    if !runtime.enabled || (!catalog_available && !custom_available) || !selected {
        return Err("catalog model unavailable; authentication and quota not probed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn local_custom_catalog_requires_exact_enabled_candidate_without_probe() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let mut provider = crate::providers::create_provider("custom").unwrap();
        provider.provider_settings_apply(serde_yaml::from_str("enabled: true\nchat_endpoint: http://127.0.0.1:1/never-probe\nenabled_models: [chosen]\ncustom_models:\n  chosen: {n_ctx: 8192}\n  disabled: {n_ctx: 4096}\n").unwrap()).unwrap();
        app.gcx.providers.write().await.add(provider);
        assert!(validate_candidate(&app, "custom/chosen").await.is_ok());
        assert!(validate_candidate(&app, "custom/disabled").await.is_err());
        assert!(validate_candidate(&app, "custom/missing").await.is_err());
        assert!(validate_candidate(&app, "other/chosen").await.is_err());
    }

    #[tokio::test]
    async fn exact_model_patch_cas_recovery_preserves_other_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let before = Some(b"chat: {model: 'p/old', future: secret, temperature: 0.2}\nchat_buddy: {model: 'p/buddy'}\nfuture: {nested: [a, b]}\n".to_vec());
        defaults_compare_exchange(dir.path(), DefaultsScope::Global, &None, &before)
            .await
            .unwrap();
        let after = Some(
            patch(
                &before,
                DefaultSlot::Chat,
                &Some("p/old".into()),
                "p/chosen",
            )
            .unwrap(),
        );
        let a: serde_yaml::Value = serde_yaml::from_slice(after.as_ref().unwrap()).unwrap();
        let b: serde_yaml::Value = serde_yaml::from_slice(before.as_ref().unwrap()).unwrap();
        assert_eq!(a["future"], b["future"]);
        assert_eq!(a["chat_buddy"], b["chat_buddy"]);
        assert_eq!(a["chat"]["future"], b["chat"]["future"]);
        defaults_compare_exchange(dir.path(), DefaultsScope::Global, &before, &after)
            .await
            .unwrap();
        assert!(
            defaults_compare_exchange(dir.path(), DefaultsScope::Global, &before, &None)
                .await
                .is_err()
        );
        assert_eq!(
            defaults_snapshot(dir.path(), DefaultsScope::Global)
                .await
                .unwrap(),
            after
        );
        defaults_compare_exchange(dir.path(), DefaultsScope::Global, &after, &before)
            .await
            .unwrap();
        assert_eq!(
            defaults_snapshot(dir.path(), DefaultsScope::Global)
                .await
                .unwrap(),
            before
        );
        assert!(patch(
            &before,
            DefaultSlot::Chat,
            &Some("p/other".into()),
            "p/chosen"
        )
        .is_err());
        assert!(patch(&before, DefaultSlot::Chat, &Some("p/old".into()), "").is_err());
        assert_eq!(
            defaults_snapshot(dir.path(), DefaultsScope::Project)
                .await
                .unwrap(),
            None
        );
    }
}
