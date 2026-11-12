//! Shared automation wire facade. Desired configuration is never an authorization grant.
use refact_scheduler_core::automation::{CatalogDocument, Definition, DefinitionOwner};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use super::generic_runtime::GenericAutomationRuntime;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionRequest {
    pub expected_catalog_revision: u64,
    pub expected_revision: Option<u64>,
    pub definition: Definition,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionRequest {
    pub expected_catalog_revision: u64,
    pub expected_revision: u64,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    pub expected_catalog_revision: u64,
    pub expected_revision: u64,
    pub operation: DesiredControl,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredControl {
    Enable,
    Disable,
    RunNow,
    Revoke,
}

pub async fn active(
    app: &crate::app_state::AppState,
) -> Result<Arc<GenericAutomationRuntime>, String> {
    let root = crate::files_correction::get_active_project_path(app.gcx.clone())
        .await
        .ok_or("no active project for automations")?;
    super::generic_runtime::runtime(app.gcx.clone(), root).await
}

pub fn check_revision(
    doc: &CatalogDocument,
    id: &str,
    catalog: u64,
    revision: u64,
) -> Result<usize, String> {
    if doc.revision != catalog {
        return Err("catalog revision conflict".into());
    }
    let index = doc
        .definitions
        .iter()
        .position(|d| d.id == id)
        .ok_or("automation not found")?;
    if doc.definitions[index].revision != revision {
        return Err("definition revision conflict".into());
    }
    project_only(&doc.definitions[index])?;
    Ok(index)
}
fn project_only(def: &Definition) -> Result<(), String> {
    if !matches!(def.owner, DefinitionOwner::Project) {
        return Err("App definitions must be edited through the guarded App manifest API".into());
    }
    Ok(())
}
pub async fn list(rt: &Arc<GenericAutomationRuntime>, id: Option<&str>) -> Result<Value, String> {
    let doc = rt.list()?;
    let mut entries = Vec::new();
    let snapshots = rt
        .sources
        .snapshot(super::generic_runtime::PROJECT_SOURCE_OWNER)
        .await;
    let wake = rt.wake()?;
    for definition in &doc.definitions {
        if id.is_some_and(|id| id != definition.id) {
            continue;
        }
        let authorization = rt.status(&definition.id)?;
        let source_id = super::generic_runtime::GenericAutomationRuntime::source_id(definition)?;
        let snapshot = snapshots.iter().find(|s| s.source_id == source_id);
        let execution = snapshot.map(|s| json!({"state":s.status,"generation":s.generation,"sequence":s.seq,"background":s.health["background"]}));
        let freshness = snapshot.map(|s| json!({"last_data_at_ms":s.health["last_data_at_ms"],"data_age_ms":s.health["data_age_ms"],"retry_at_ms":s.health["retry_at_ms"],"error_class":s.health["error_class"],"capabilities":s.health["capabilities"]}));
        entries.push(json!({"definition":definition,"security_digest":definition.security_digest()?,
            "status":{"authorization":authorization,"execution":execution,"freshness":freshness,"delivery":null},"project_wake":wake,
            "capabilities":{"desired_control":["enable","disable","run_now","revoke"],"approval":"trusted_human_flow_only"}}));
    }
    if id.is_some() && entries.is_empty() {
        return Err("automation not found".into());
    }
    Ok(
        json!({"catalog_revision":doc.revision,"scope":"project_definitions","definitions":entries,
        "status_note":"null dimensions are not observed by this facade; they do not imply success"}),
    )
}
pub async fn save(
    rt: &Arc<GenericAutomationRuntime>,
    id: Option<&str>,
    request: DefinitionRequest,
) -> Result<Value, String> {
    project_only(&request.definition)?;
    request.definition.validate()?;
    let mut doc = rt.list()?;
    if doc.revision != request.expected_catalog_revision {
        return Err("catalog revision conflict".into());
    }
    if let Some(id) = id {
        if request.definition.id != id {
            return Err("path/definition identity mismatch".into());
        }
        let index = check_revision(
            &doc,
            id,
            request.expected_catalog_revision,
            request
                .expected_revision
                .ok_or("expected_revision required for replacement")?,
        )?;
        if request.definition.revision <= doc.definitions[index].revision {
            return Err("definition revision must increase".into());
        }
        doc.definitions[index] = request.definition;
    } else {
        if request.expected_revision.is_some() {
            return Err("create must omit expected_revision".into());
        }
        if doc
            .definitions
            .iter()
            .any(|d| d.id == request.definition.id)
        {
            return Err("automation already exists".into());
        }
        doc.definitions.push(request.definition);
    }
    let saved = rt
        .save(request.expected_catalog_revision, doc.definitions)
        .await?;
    Ok(json!({"catalog_revision":saved.revision,"authorization":"not_granted_by_this_operation"}))
}
pub async fn delete(
    rt: &Arc<GenericAutomationRuntime>,
    id: &str,
    request: RevisionRequest,
) -> Result<Value, String> {
    let mut doc = rt.list()?;
    let index = check_revision(
        &doc,
        id,
        request.expected_catalog_revision,
        request.expected_revision,
    )?;
    doc.definitions.remove(index);
    let saved = rt
        .save(request.expected_catalog_revision, doc.definitions)
        .await?;
    Ok(json!({"catalog_revision":saved.revision,"removed":true}))
}
pub async fn control(
    rt: &Arc<GenericAutomationRuntime>,
    id: &str,
    request: ControlRequest,
) -> Result<Value, String> {
    match request.operation {
        DesiredControl::RunNow => {
            return rt
                .run_exact(
                    id,
                    request.expected_catalog_revision,
                    request.expected_revision,
                )
                .await;
        }
        DesiredControl::Revoke => {
            rt.revoke_exact(
                id,
                request.expected_catalog_revision,
                request.expected_revision,
            )
            .await?;
            return Ok(json!({"revoked":true}));
        }
        DesiredControl::Enable | DesiredControl::Disable => {}
    }
    let mut doc = rt.list()?;
    let index = check_revision(
        &doc,
        id,
        request.expected_catalog_revision,
        request.expected_revision,
    )?;
    let def = &mut doc.definitions[index];
    def.enabled = matches!(request.operation, DesiredControl::Enable);
    def.revision = def.revision.checked_add(1).ok_or("revision overflow")?;
    let saved = rt
        .save(request.expected_catalog_revision, doc.definitions)
        .await?;
    Ok(json!({"catalog_revision":saved.revision,"authorization":"not_granted_by_this_operation"}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn definition() -> Definition {
        serde_json::from_value(json!({"schema_version":1,"id":"test","revision":1,"owner":{"kind":"project"},"description":"test","enabled":true,"trigger":{"kind":"continuous"},"action":{"kind":"collect","driver":{"type":"manual"}},"deliveries":[],"lifecycle":{"kind":"on_demand"},"policy":{"idempotent":false,"max_pending_events":10,"max_event_bytes":1024}})).unwrap()
    }
    #[test]
    fn cas_and_app_ownership_are_independent_fences() {
        let mut doc = CatalogDocument {
            revision: 4,
            definitions: vec![definition()],
            ..Default::default()
        };
        assert_eq!(check_revision(&doc, "test", 4, 1).unwrap(), 0);
        assert!(check_revision(&doc, "test", 3, 1)
            .unwrap_err()
            .contains("catalog revision"));
        assert!(check_revision(&doc, "test", 4, 2)
            .unwrap_err()
            .contains("definition revision"));
        doc.definitions[0].owner = DefinitionOwner::App {
            app_id: "forged".into(),
            source_id: "s".into(),
        };
        assert!(check_revision(&doc, "test", 4, 1)
            .unwrap_err()
            .contains("guarded App"));
    }
    #[test]
    fn wire_rejects_approval_and_unknown_control() {
        assert!(serde_json::from_value::<ControlRequest>(json!({"expected_catalog_revision":0,"expected_revision":1,"operation":"enable","approved":true})).is_err());
        assert!(serde_json::from_value::<ControlRequest>(
            json!({"expected_catalog_revision":0,"expected_revision":1,"operation":"approve"})
        )
        .is_err());
        assert!(
            serde_json::from_value::<RevisionRequest>(json!({"expected_catalog_revision":0}))
                .is_err()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejected_host_policy_does_not_commit_desired_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let sources = refact_scheduler_core::sources::AutomationService::new(temp.path()).unwrap();
        let runtime =
            GenericAutomationRuntime::new(temp.path().into(), &gcx, sources.clone()).unwrap();
        let initial = runtime.save(0, vec![definition()]).await.unwrap();
        let mut forged = definition();
        forged.revision += 1;
        if let refact_scheduler_core::automation::AutomationAction::Collect {
            policy, ..
        } = &mut forged.action
        {
            policy.owner_definition_digest = Some("forged".into());
        } else {
            panic!("collector fixture required");
        }
        let error = save(
            &runtime,
            Some("test"),
            DefinitionRequest {
                expected_catalog_revision: initial.revision,
                expected_revision: Some(1),
                definition: forged,
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("host-only"), "{error}");
        assert_eq!(
            serde_json::to_value(runtime.list().unwrap()).unwrap(),
            serde_json::to_value(initial).unwrap()
        );
        sources.shutdown().await;
    }
}
