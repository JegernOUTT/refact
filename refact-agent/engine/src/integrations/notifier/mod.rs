use std::sync::Arc;

use async_trait::async_trait;

use crate::global_context::GlobalContext;

pub mod integr_notifier_telegram;

#[async_trait]
pub trait NotifierBackend: Send + Sync {
    async fn send(&self, target: Option<&str>, text: &str) -> Result<(), String>;
}

pub async fn notifier_by_integration_id(
    gcx: Arc<GlobalContext>,
    integration_id: &str,
) -> Option<Box<dyn NotifierBackend>> {
    match notifier_by_integration_id_result(gcx, integration_id).await {
        Ok(backend) => Some(backend),
        Err(error) => {
            tracing::warn!("failed to resolve notifier integration {integration_id}: {error}");
            None
        }
    }
}

async fn notifier_by_integration_id_result(
    gcx: Arc<GlobalContext>,
    integration_id: &str,
) -> Result<Box<dyn NotifierBackend>, String> {
    let integration_id = integration_id.trim();
    if integration_id.is_empty() {
        return Err("integration_id is required".to_string());
    }

    let active_project_path = crate::files_correction::get_active_project_path(gcx.clone()).await;
    let (config_dirs, global_config_dir) =
        crate::integrations::setting_up_integrations::get_config_dirs(
            gcx.clone(),
            &active_project_path,
        )
        .await;
    let (integrations_yaml_path, is_inside_container, allow_experimental) = (
        gcx.cmdline.integrations_yaml.clone(),
        gcx.cmdline.inside_container,
        gcx.cmdline.experimental,
    );
    let lst = crate::integrations::integrations_list(allow_experimental);
    let mut error_log = Vec::new();
    let vars_for_replacements =
        crate::integrations::setting_up_integrations::get_vars_for_replacements(
            gcx.clone(),
            &mut error_log,
        )
        .await;
    let records = crate::integrations::setting_up_integrations::read_integrations_d(
        &config_dirs,
        &global_config_dir,
        &integrations_yaml_path,
        &vars_for_replacements,
        &lst,
        &mut error_log,
        &["**/*".to_string()],
        false,
    );
    for error in error_log {
        tracing::warn!("{error}");
    }

    let record = records.into_iter().rev().find(|record| {
        record.integr_name == integration_id
            && record.integr_config_exists
            && ((!is_inside_container && record.on_your_laptop)
                || (is_inside_container && record.when_isolated))
    });
    let record = record.ok_or_else(|| format!("integration `{integration_id}` not found"))?;

    match integration_id {
        integr_notifier_telegram::INTEGRATION_ID => {
            integr_notifier_telegram::backend_from_config(
                gcx,
                record.integr_config_path,
                &record.config_unparsed,
            )
            .await
        }
        _ => Err(format!("integration `{integration_id}` is not a notifier")),
    }
}

pub(crate) async fn configured_notifier_backend(
    gcx: Arc<GlobalContext>,
    integration_id: &str,
) -> Result<Box<dyn NotifierBackend>, String> {
    notifier_by_integration_id_result(gcx, integration_id).await
}
