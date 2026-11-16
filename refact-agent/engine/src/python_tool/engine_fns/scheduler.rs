use std::path::Path;

use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::common::to_value;
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput};

/// The fixed text of every credential store error; store errors can quote values.
const STORE_ERROR: &str =
    "credential store unavailable; check identity, revision, value limits, and private storage";

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "scheduler.config",
            summary: "The scheduler's startup configuration: enabled, what disabled it, durable jobs, missed-run grace, retry policy, recent-run cap. Fixed until restart.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["enabled","disabled_by","disable_durable","recent_runs_cap","missed_grace_min_ms","missed_grace_max_ms","retry","source","restart_required_to_change"],"properties":{"enabled":{"type":"boolean"},"disabled_by":{"type":"array","items":{"enum":["cli --no-scheduler","env REFACT_DISABLE_SCHEDULER"]}},"disable_durable":{"type":"boolean"},"recent_runs_cap":{"type":"integer"},"missed_grace_min_ms":{"type":"integer"},"missed_grace_max_ms":{"type":"integer"},"retry":{"type":"object","properties":{"max_attempts":{"type":"integer"},"backoff_ms":{"type":"array","items":{"type":"integer"}}}},"source":{"type":"string"},"restart_required_to_change":{"type":"boolean"}}})),
            handler: config,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "scheduler.credentials",
            summary: "Automation credential bindings of this project: identity and revisions only, never values. Needs exactly one project folder.",
            class: EngineClass::RedactedRead,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["supported","project_root","credentials"],"properties":{"supported":{"type":"boolean"},"project_root":{"type":"string"},"credentials":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"origin":{"type":"string"},"header":{"type":"string"},"tenant":{"type":"string"},"principal":{"type":"string"},"scope":{"type":"array","items":{"type":"string"}},"identity_revision":{"type":"string"},"secret_revision":{"type":"string"}}}}}})),
            handler: credentials,
            preview: None,
            timeout: None,
        },
    ]
}

fn config<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let gcx = &ctx.app.gcx;
        let mut data = to_value(&gcx.scheduler_config)?;
        let mut disabled_by = Vec::new();
        if gcx.cmdline.no_scheduler {
            disabled_by.push("cli --no-scheduler");
        }
        if refact_core::scheduler_config::scheduler_disabled_by_env() {
            disabled_by.push("env REFACT_DISABLE_SCHEDULER");
        }
        data["disabled_by"] = json!(disabled_by);
        data["source"] = json!("privacy.yaml");
        data["restart_required_to_change"] = json!(true);
        let config = &gcx.scheduler_config;
        let text = format!(
            "scheduler {}, durable jobs {}, retry {}x",
            if config.enabled {
                "enabled"
            } else {
                "disabled"
            },
            if config.disable_durable { "off" } else { "on" },
            config.retry.max_attempts
        );
        Ok(EngineOutput::new(data, text))
    })
}

/// Bindings of the store for `root`; `base` overrides the private storage directory (tests only).
pub(super) fn credentials_for(root: &Path, base: Option<&Path>) -> Result<Value, EngineError> {
    if !cfg!(unix) {
        return Ok(json!({"supported":false,"project_root":root,"credentials":[]}));
    }
    let failed = |_| EngineError::Failed(STORE_ERROR.into());
    let store = crate::scheduler::credentials::AutomationCredentialStore::open(root, base)
        .map_err(failed)?;
    let mut credentials = to_value(store.list_metadata().map_err(failed)?)?;
    for row in credentials.as_array_mut().into_iter().flatten() {
        if let Some(origin) = row["origin"].as_str() {
            row["origin"] = json!(crate::integrations::mcp::redaction::scrub_text(origin, &[]));
        }
    }
    Ok(json!({"supported":true,"project_root":root,"credentials":credentials}))
}

fn credentials<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        super::common::require_root_chat(ctx, "scheduler.credentials").await?;
        let roots = crate::files_correction::get_unscoped_project_dirs(ctx.app.gcx.clone()).await;
        if roots.len() != 1 {
            return Err(EngineError::Failed(
                "credential metadata needs exactly one open project folder".into(),
            ));
        }
        let root = std::fs::canonicalize(&roots[0])
            .map_err(|_| EngineError::Failed("project folder unavailable".into()))?;
        let data = credentials_for(&root, None)?;
        let text = format!(
            "{} automation credentials",
            data["credentials"].as_array().map_or(0, Vec::len)
        );
        Ok(EngineOutput::new(data, text))
    })
}
