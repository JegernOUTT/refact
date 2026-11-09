//! Human-only, ephemeral authority for an exact generic automation definition.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use crate::{
    at_commands::at_commands::AtCommandsContext, call_validation::ChatToolCall,
    chat::types::ChatSession,
};

pub(crate) const HUMAN_AUTOMATION_APPROVAL: &str = "exact human automation approval required";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalArgs {
    action: String,
    operation: String,
    id: String,
    expected_catalog_revision: u64,
    expected_revision: u64,
    security_digest: String,
    endpoint_grant: Option<EndpointGrantArgs>,
    file_grant: Option<refact_scheduler_core::sources::FileGrant>,
}

// A strict adapter, rather than deserializing the core grant directly: ignored fields
// must not turn a displayed human decision into broader runtime authority.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EndpointGrantArgs {
    origin: String,
    allowed_private_ips: Vec<std::net::IpAddr>,
}
impl EndpointGrantArgs {
    fn validate(&self) -> Result<(), String> {
        let url = reqwest::Url::parse(&self.origin).map_err(|_| "invalid endpoint origin")?;
        if self.origin.len() > 2048
            || !matches!(url.scheme(), "http" | "https" | "ws" | "wss")
            || url.origin().ascii_serialization() != self.origin
            || !url.username().is_empty()
            || url.password().is_some()
            || self.allowed_private_ips.is_empty()
            || self.allowed_private_ips.len() > 64
            || self
                .allowed_private_ips
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != self.allowed_private_ips.len()
        {
            return Err(
                "endpoint grant requires a canonical origin and 1..64 distinct exact IPs".into(),
            );
        }
        Ok(())
    }
}
fn parse(args: &Value) -> Result<ApprovalArgs, String> {
    let parsed: ApprovalArgs = serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
    if parsed.action != "automation"
        || !matches!(
            parsed.operation.as_str(),
            "approve" | "approve_endpoint" | "approve_file"
        )
        || parsed.id.is_empty()
        || parsed.security_digest.is_empty()
    {
        return Err("invalid exact automation approval request".into());
    }
    match (
        parsed.operation.as_str(),
        parsed.endpoint_grant.as_ref(),
        parsed.file_grant.as_ref(),
    ) {
        ("approve_endpoint", Some(grant), None) if args.get("file_grant").is_none() => {
            grant.validate()?
        }
        ("approve_file", None, Some(grant)) if args.get("endpoint_grant").is_none() => {
            if grant.path.is_empty()
                || grant.path.len() > 4096
                || grant.path.contains('\\')
                || !std::path::Path::new(&grant.path)
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
            {
                return Err("file grant requires an exact project-relative path".into());
            }
        }
        ("approve", None, None)
            if args.get("endpoint_grant").is_none() && args.get("file_grant").is_none() => {}
        _ => return Err("grant fields must match the exact approval operation".into()),
    }
    Ok(parsed)
}
fn is_approval(name: &str, args: &Value) -> bool {
    crate::agents::tool_authority::canonical(name) == "schedule"
        && args.get("action").and_then(Value::as_str) == Some("automation")
        && matches!(
            args.get("operation").and_then(Value::as_str),
            Some("approve" | "approve_endpoint" | "approve_file")
        )
}

// No Serialize/Deserialize: a restored trajectory cannot restore authority.
pub(crate) struct AutomationApprovalReceipt {
    chat_id: String,
    project: PathBuf,
    args: Value,
    created: Instant,
    accepted: bool,
}
impl AutomationApprovalReceipt {
    fn validate(
        &self,
        chat_id: &str,
        project: &std::path::Path,
        args: &Value,
        accepted: bool,
    ) -> Result<(), String> {
        if self.created.elapsed() >= Duration::from_secs(900)
            || self.chat_id != chat_id
            || self.project != project
            || self.args != *args
            || (accepted && !self.accepted)
        {
            return Err("fresh exact human automation approval required; call/project/arguments changed or expired".into());
        }
        Ok(())
    }
}

async fn runtime(
    ccx: &Arc<Mutex<AtCommandsContext>>,
) -> Result<
    (
        crate::app_state::AppState,
        String,
        PathBuf,
        Arc<super::generic_runtime::GenericAutomationRuntime>,
    ),
    String,
> {
    let (app, chat_id, model, scope) = {
        let context = ccx.lock().await;
        (
            context.app.clone(),
            context.chat_id.clone(),
            context.current_model.clone(),
            context
                .execution_scope
                .as_ref()
                .map(|scope| scope.effective_root().to_path_buf()),
        )
    };
    let rt = crate::apps::runtime(
        &app,
        &crate::apps::Caller::Tool {
            chat_id: chat_id.clone(),
            model: Some(model),
        },
    )
    .await?;
    let project = tokio::fs::canonicalize(&rt.root)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(scope) = scope {
        if tokio::fs::canonicalize(scope)
            .await
            .map_err(|e| e.to_string())?
            != project
        {
            return Err("automation project does not match execution scope".into());
        }
    }
    Ok((app, chat_id, project, rt.generic.clone()))
}
fn check(
    rt: &super::generic_runtime::GenericAutomationRuntime,
    args: &ApprovalArgs,
) -> Result<(), String> {
    let doc = rt.list()?;
    let index = super::automation_api::check_revision(
        &doc,
        &args.id,
        args.expected_catalog_revision,
        args.expected_revision,
    )?;
    if doc.definitions[index].security_digest()? != args.security_digest {
        return Err("automation security digest changed".into());
    }
    Ok(())
}

pub(crate) async fn inspect_automation_approval(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    call: &ChatToolCall,
    args: &HashMap<String, Value>,
) -> Result<bool, String> {
    let value = serde_json::to_value(args).map_err(|e| e.to_string())?;
    if !is_approval(&call.function.name, &value) {
        return Ok(false);
    }
    let parsed = parse(&value)?;
    let (app, chat_id, project, rt) = runtime(ccx).await?;
    check(&rt, &parsed)?;
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&chat_id)
        .cloned()
        .ok_or("caller chat unavailable")?;
    let mut session = session.lock().await;
    if session.closed {
        return Err("caller chat closed".into());
    }
    session
        .automation_approval_receipts
        .retain(|_, r| r.created.elapsed() < Duration::from_secs(900));
    if let Some(receipt) = session.automation_approval_receipts.get(&call.id) {
        receipt.validate(&chat_id, &project, &value, false)?;
    } else {
        if session.automation_approval_receipts.len() >= 128 {
            return Err("automation approval capacity reached".into());
        }
        session.automation_approval_receipts.insert(
            call.id.clone(),
            AutomationApprovalReceipt {
                chat_id,
                project,
                args: value,
                created: Instant::now(),
                accepted: false,
            },
        );
    }
    Ok(true)
}

/// Called only by the actual user decision path, never by model policy.
pub(crate) fn accept_automation_approval(
    session: &mut ChatSession,
    id: &str,
) -> Result<(), String> {
    let call = session
        .messages
        .iter()
        .rev()
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .find(|c| c.id == id);
    let Some(call) = call else {
        if session.automation_approval_receipts.contains_key(id) {
            return Err("automation approval call disappeared".into());
        }
        return Ok(());
    };
    if crate::agents::tool_authority::canonical(&call.function.name) != "schedule" {
        if session.automation_approval_receipts.remove(id).is_some() {
            return Err("automation approval tool changed".into());
        }
        return Ok(());
    }
    let value: Value = serde_json::from_str(&call.function.arguments).map_err(|e| e.to_string())?;
    if !is_approval(&call.function.name, &value) {
        if session.automation_approval_receipts.remove(id).is_some() {
            return Err("automation approval tool changed".into());
        }
        return Ok(());
    }
    let receipt = session
        .automation_approval_receipts
        .get_mut(id)
        .ok_or(HUMAN_AUTOMATION_APPROVAL)?;
    receipt.validate(&session.chat_id, &receipt.project, &value, false)?;
    receipt.accepted = true;
    Ok(())
}

pub(crate) async fn execute_automation_approval(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    call_id: &str,
    args: &HashMap<String, Value>,
) -> Result<Value, String> {
    let value = serde_json::to_value(args).map_err(|e| e.to_string())?;
    let (app, chat_id, project, rt) = runtime(ccx).await?;
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&chat_id)
        .cloned()
        .ok_or("caller chat unavailable")?;
    let receipt = {
        let mut session = session.lock().await;
        if session.closed {
            return Err("caller chat closed".into());
        }
        session
            .automation_approval_receipts
            .remove(call_id)
            .ok_or(HUMAN_AUTOMATION_APPROVAL)?
    };
    receipt.validate(&chat_id, &project, &value, true)?;
    let parsed = parse(&value)?;
    check(&rt, &parsed)?;
    if let Some(grant) = parsed.endpoint_grant {
        rt.approve_endpoint_exact(
            &parsed.id,
            parsed.expected_catalog_revision,
            parsed.expected_revision,
            &parsed.security_digest,
            refact_scheduler_core::sources::EndpointGrant {
                origin: grant.origin,
                allowed_private_ips: grant.allowed_private_ips,
            },
        )
        .await?;
    } else if let Some(grant) = parsed.file_grant {
        rt.approve_file_exact(
            &parsed.id,
            parsed.expected_catalog_revision,
            parsed.expected_revision,
            &parsed.security_digest,
            grant,
        )
        .await?;
    } else {
        rt.apply_exact(
            &parsed.id,
            parsed.expected_catalog_revision,
            parsed.expected_revision,
            &parsed.security_digest,
        )
        .await?;
    }
    let mut result =
        json!({"id":parsed.id,"approval":"human","security_digest":parsed.security_digest});
    if parsed.operation != "approve" {
        result["operation"] = json!(parsed.operation);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn definition() -> refact_scheduler_core::automation::Definition {
        serde_json::from_value(json!({"schema_version":1,"id":"test","revision":1,"owner":{"kind":"project"},"description":"test","enabled":true,"trigger":{"kind":"continuous"},"action":{"kind":"collect","driver":{"type":"manual"}},"deliveries":[],"lifecycle":{"kind":"on_demand"},"policy":{"idempotent":false,"max_pending_events":10,"max_event_bytes":1024}})).unwrap()
    }
    async fn fixture() -> (
        crate::app_state::AppState,
        Arc<Mutex<AtCommandsContext>>,
        ChatToolCall,
        HashMap<String, Value>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![dir.path().canonicalize().unwrap()];
        let (root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "test/model".into(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: "test/model".into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
        {
            let mut state = gcx.caps_state.write().await;
            state.caps = Some(Arc::new(caps));
            state.loaded_project_root = Some(root);
            state.last_attempted_ts = chrono::Utc::now().timestamp() as u64;
        }
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let mut session = ChatSession::new("automation-human".into());
        session.thread.model = "test/model".into();
        session.work_project_root = Some(dir.path().into());
        session.thread.auto_approve_dangerous_commands = true;
        session.thread.autonomous_no_confirm = true;
        app.chat
            .sessions
            .write()
            .await
            .insert(session.chat_id.clone(), Arc::new(Mutex::new(session)));
        let ccx = Arc::new(Mutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                10,
                false,
                vec![],
                "automation-human".into(),
                None,
                "test/model".into(),
                None,
                None,
            )
            .await,
        ));
        let def = definition();
        let rt = super::super::generic_runtime::runtime(
            app.gcx.clone(),
            dir.path().canonicalize().unwrap(),
        )
        .await
        .unwrap();
        let doc = rt.save(0, vec![def.clone()]).await.unwrap();
        let args: HashMap<String, Value> = serde_json::from_value(json!({"action":"automation","operation":"approve","id":"test","expected_catalog_revision":doc.revision,"expected_revision":1,"security_digest":def.security_digest().unwrap()})).unwrap();
        let call: ChatToolCall = serde_json::from_value(json!({"id":"approve-test","type":"function","function":{"name":"schedule","arguments":serde_json::to_string(&args).unwrap()}})).unwrap();
        let session = app.chat.sessions.read().await["automation-human"].clone();
        session
            .lock()
            .await
            .add_message(crate::call_validation::ChatMessage {
                role: "assistant".into(),
                tool_calls: Some(vec![call.clone()]),
                ..Default::default()
            });
        (app, ccx, call, args, dir)
    }

    #[tokio::test]
    async fn actual_preflight_never_autoauthorizes_and_human_acceptance_is_one_use() {
        let (app, ccx, call, args, _dir) = fixture().await;
        assert!(execute_automation_approval(&ccx, &call.id, &args)
            .await
            .is_err());
        let session = app.chat.sessions.read().await["automation-human"].clone();
        let thread = session.lock().await.thread.clone();
        let (asks, denials) = crate::chat::tools::check_tools_confirmation(
            app.clone(),
            &[call.clone()],
            &[],
            &thread,
            None,
            "agent",
            Some("test/model"),
            &[],
            "test",
        )
        .await;
        assert!(denials.is_empty(), "{denials:?}");
        assert_eq!(
            asks.len(),
            1,
            "dangerous/autonomous flags bypassed human preflight"
        );
        assert_eq!(asks[0].rule, HUMAN_AUTOMATION_APPROVAL);
        assert!(inspect_automation_approval(&ccx, &call, &args)
            .await
            .unwrap());
        assert!(crate::chat::tools::requires_actual_human_approval(
            HUMAN_AUTOMATION_APPROVAL
        ));
        // Even a parent's model-approved ID is not a human receipt.
        let session = app.chat.sessions.read().await["automation-human"].clone();
        session
            .lock()
            .await
            .runtime
            .auto_approved_tool_ids
            .push(call.id.clone());
        assert!(execute_automation_approval(&ccx, &call.id, &args)
            .await
            .is_err());
        assert!(inspect_automation_approval(&ccx, &call, &args)
            .await
            .unwrap());
        accept_automation_approval(&mut *session.lock().await, &call.id).unwrap();
        execute_automation_approval(&ccx, &call.id, &args)
            .await
            .unwrap();
        assert!(execute_automation_approval(&ccx, &call.id, &args)
            .await
            .is_err());
        app.gcx.apps.shutdown().await;
    }

    #[tokio::test]
    async fn changed_arguments_or_catalog_revision_burn_human_receipt() {
        for stale_catalog in [false, true] {
            let (app, ccx, call, mut args, dir) = fixture().await;
            assert!(inspect_automation_approval(&ccx, &call, &args)
                .await
                .unwrap());
            let session = app.chat.sessions.read().await["automation-human"].clone();
            accept_automation_approval(&mut *session.lock().await, &call.id).unwrap();
            if stale_catalog {
                let rt = super::super::generic_runtime::runtime(
                    app.gcx.clone(),
                    dir.path().canonicalize().unwrap(),
                )
                .await
                .unwrap();
                let doc = rt.list().unwrap();
                let mut defs = doc.definitions;
                defs[0].revision += 1;
                rt.save(doc.revision, defs).await.unwrap();
            } else {
                args.insert("security_digest".into(), json!("changed"));
            }
            assert!(execute_automation_approval(&ccx, &call.id, &args)
                .await
                .is_err());
            assert!(session.lock().await.automation_approval_receipts.is_empty());
            app.gcx.apps.shutdown().await;
        }
    }

    #[tokio::test]
    async fn endpoint_grant_requires_actual_human_and_binds_every_grant_argument() {
        let (app, ccx, mut call, mut args, dir) = fixture().await;
        let rt = super::super::generic_runtime::runtime(
            app.gcx.clone(),
            dir.path().canonicalize().unwrap(),
        )
        .await
        .unwrap();
        let mut def = definition();
        def.revision = 2;
        def.action = refact_scheduler_core::automation::AutomationAction::Collect {
            driver: refact_apps_core::SourceDriver::HttpPoll {
                url: "http://127.0.0.1:9999/data".into(),
                every_ms: 1000,
            },
            policy: Default::default(),
        };
        let doc = rt
            .save(rt.list().unwrap().revision, vec![def.clone()])
            .await
            .unwrap();
        rt.apply_exact(
            &def.id,
            doc.revision,
            def.revision,
            &def.security_digest().unwrap(),
        )
        .await
        .unwrap();
        args.insert("operation".into(), json!("approve_endpoint"));
        args.insert("expected_catalog_revision".into(), json!(doc.revision));
        args.insert("expected_revision".into(), json!(def.revision));
        args.insert(
            "security_digest".into(),
            json!(def.security_digest().unwrap()),
        );
        args.insert(
            "endpoint_grant".into(),
            json!({"origin":"http://127.0.0.1:9999","allowed_private_ips":["127.0.0.1"]}),
        );
        call.function.arguments = serde_json::to_string(&args).unwrap();
        let session = app.chat.sessions.read().await["automation-human"].clone();
        session
            .lock()
            .await
            .add_message(crate::call_validation::ChatMessage {
                role: "assistant".into(),
                tool_calls: Some(vec![call.clone()]),
                ..Default::default()
            });
        let thread = session.lock().await.thread.clone();
        let (asks, denials) = crate::chat::tools::check_tools_confirmation(
            app.clone(),
            &[call.clone()],
            &[],
            &thread,
            None,
            "agent",
            Some("test/model"),
            &[],
            "test",
        )
        .await;
        assert!(denials.is_empty(), "{denials:?}");
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].rule, HUMAN_AUTOMATION_APPROVAL);
        // Model/parent auto-approval is never endpoint authority, even with autonomous flags.
        session
            .lock()
            .await
            .runtime
            .auto_approved_tool_ids
            .push(call.id.clone());
        assert!(execute_automation_approval(&ccx, &call.id, &args)
            .await
            .is_err());
        for changed in [
            json!({"origin":"http://127.0.0.1:9998","allowed_private_ips":["127.0.0.1"]}),
            json!({"origin":"http://127.0.0.1:9999","allowed_private_ips":["127.0.0.2"]}),
        ] {
            assert!(inspect_automation_approval(&ccx, &call, &args)
                .await
                .unwrap());
            accept_automation_approval(&mut *session.lock().await, &call.id).unwrap();
            let mut mutated = args.clone();
            mutated.insert("endpoint_grant".into(), changed);
            assert!(execute_automation_approval(&ccx, &call.id, &mutated)
                .await
                .is_err());
            assert!(session.lock().await.automation_approval_receipts.is_empty());
        }
        assert!(inspect_automation_approval(&ccx, &call, &args)
            .await
            .unwrap());
        accept_automation_approval(&mut *session.lock().await, &call.id).unwrap();
        let result = execute_automation_approval(&ccx, &call.id, &args)
            .await
            .unwrap();
        assert_eq!(result["operation"], "approve_endpoint");
        assert!(execute_automation_approval(&ccx, &call.id, &args)
            .await
            .is_err());
        app.gcx.apps.shutdown().await;
    }

    #[test]
    fn endpoint_grant_is_required_typed_bounded_and_not_a_boolean() {
        let base = json!({"action":"automation","operation":"approve_endpoint","id":"x","expected_catalog_revision":2,"expected_revision":1,"security_digest":"digest"});
        assert!(is_approval("schedule", &base));
        assert!(parse(&base).is_err());
        for grant in [
            json!(true),
            json!({"origin":"http://127.0.0.1","allowed_private_ips":[]}),
            json!({"origin":"http://127.0.0.1/path","allowed_private_ips":["127.0.0.1"]}),
            json!({"origin":"http://127.0.0.1","allowed_private_ips":["127.0.0.0/8"]}),
            json!({"origin":"http://127.0.0.1","allowed_private_ips":["127.0.0.1"],"approved":true}),
            json!({"origin":"http://127.0.0.1","allowed_private_ips":vec!["127.0.0.1";65]}),
        ] {
            let mut args = base.clone();
            args["endpoint_grant"] = grant;
            assert!(parse(&args).is_err(), "accepted {args}");
        }
        let mut args = base;
        args["endpoint_grant"] =
            json!({"origin":"http://127.0.0.1","allowed_private_ips":["127.0.0.1","::1"]});
        assert!(parse(&args).is_ok());
        args["operation"] = json!("approve");
        assert!(parse(&args).is_err());
    }

    #[test]
    fn receipt_binds_chat_project_arguments_and_expiry_without_restoration_authority() {
        let args = json!({"action":"automation","operation":"approve","id":"x","expected_catalog_revision":2,"expected_revision":1,"security_digest":"digest"});
        let mut receipt = AutomationApprovalReceipt {
            chat_id: "chat".into(),
            project: PathBuf::from("/project"),
            args: args.clone(),
            created: Instant::now(),
            accepted: false,
        };
        assert!(receipt
            .validate("chat", std::path::Path::new("/project"), &args, true)
            .is_err());
        receipt.accepted = true;
        assert!(receipt
            .validate("chat", std::path::Path::new("/project"), &args, true)
            .is_ok());
        assert!(receipt
            .validate("other", std::path::Path::new("/project"), &args, true)
            .is_err());
        assert!(receipt
            .validate("chat", std::path::Path::new("/other"), &args, true)
            .is_err());
        receipt.created = Instant::now() - Duration::from_secs(901);
        assert!(receipt
            .validate("chat", std::path::Path::new("/project"), &args, true)
            .is_err());
        assert!(ChatSession::new("chat".into())
            .automation_approval_receipts
            .is_empty());
        let mut forged = args;
        forged["approved"] = json!(true);
        assert!(parse(&forged).is_err());
    }
}
