use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::mask::item;
use super::{previewed, EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput, Preview};
use crate::buddy::types::BuddyPage;

const PAGES: &[&str] = &[
    "buddy",
    "stats",
    "customization",
    "providers",
    "default_models",
    "integrations",
    "extensions",
    "marketplace_hub",
    "marketplace",
    "skills_marketplace",
    "commands_marketplace",
    "delegates_marketplace",
    "tasks_list",
    "task_workspace",
    "knowledge_graph",
    "worktrees",
];

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "gui.voice",
            summary: "Voice input status: compiled into this build, model loaded, model name, download progress.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["enabled","model_loaded","model_name","is_downloading","download_progress"],"properties":{"enabled":{"type":"boolean","description":"compiled in; not a setting"},"model_loaded":{"type":"boolean"},"model_name":{"type":"string"},"is_downloading":{"type":"boolean"},"download_progress":{"type":"integer","minimum":0,"maximum":100}}})),
            handler: voice,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "gui.open_page",
            summary: "Ask every Refact GUI attached to this engine to open a page, after the user approves. sent=true means queued, not shown.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["page"],"properties":{"page":{"enum":PAGES},"task_id":{"type":"string","minLength":1,"description":"Required for task_workspace."}}}),
            output: || Some(json!({"type":"object","required":["page","sent"],"properties":{"page":{"type":"string"},"sent":{"type":"boolean"}}})),
            handler: open_page,
            preview: Some(open_page_preview),
            timeout: None,
        },
    ]
}

fn voice<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let service = ctx.app.gcx.voice_service.clone();
        let enabled = crate::voice::VoiceService::is_enabled();
        let model_loaded = service.is_model_loaded().await;
        let model_name = service.model_name().await;
        let text = if !enabled {
            "voice not compiled in".to_string()
        } else if model_loaded {
            format!("voice enabled, model {model_name} loaded")
        } else {
            format!("voice enabled, model {model_name} not loaded")
        };
        Ok(EngineOutput::new(
            json!({"enabled":enabled,"model_loaded":model_loaded,"model_name":model_name,"is_downloading":service.is_downloading(),"download_progress":service.download_progress()}),
            text,
        ))
    })
}

/// Builds the page and its label; checks the task exists and the GUI channel is up.
async fn plan(ctx: &EngineCtx, args: &Value) -> Result<(BuddyPage, String), EngineError> {
    let page = args["page"].as_str().unwrap_or_default();
    let task_id = args.get("task_id").and_then(Value::as_str);
    let (page_value, label) = match (page, task_id) {
        ("task_workspace", Some(task_id)) => {
            let meta = crate::tasks::storage::load_task_meta(ctx.app.gcx.clone(), task_id)
                .await
                .map_err(|_| EngineError::Type(format!("no task with id {task_id:?}")))?;
            (
                json!({"type":page,"task_id":task_id}),
                format!("{page} ({})", meta.name),
            )
        }
        ("task_workspace", None) => {
            return Err(EngineError::Type("task_workspace needs task_id".into()))
        }
        (_, Some(_)) => {
            return Err(EngineError::Type(format!(
                "task_id applies only to task_workspace, not {page}"
            )))
        }
        (_, None) => (json!({"type":page}), page.to_string()),
    };
    let page = serde_json::from_value(page_value)
        .map_err(|e| EngineError::Type(format!("invalid page: {e}")))?;
    if ctx.app.buddy.buddy.lock().await.is_none() {
        return Err(EngineError::Failed(
            "buddy service not initialized: no GUI is attached through the buddy channel".into(),
        ));
    }
    Ok((page, label))
}

fn open_page_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let (_, label) = plan(ctx, args).await?;
        let approved = previewed::plan(json!(label));
        Ok(Preview {
            plan: approved,
            title: "open a page in the GUI".into(),
            items: vec![item("page", None, Some(json!(label)))],
            scope: Some("GUI of this engine".into()),
            class: EngineClass::Change,
            warning: None,
        })
    })
}

fn open_page<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let (page, label) = plan(ctx, args).await?;
        previewed::take(ctx, "gui.open_page", &json!(label))?;
        let name = args["page"].as_str().unwrap_or_default().to_string();
        {
            let buddy = ctx.app.buddy.buddy.lock().await;
            let service = buddy
                .as_ref()
                .ok_or_else(|| EngineError::Failed("buddy service not initialized".into()))?;
            service.send_navigation(page);
        }
        Ok(EngineOutput::new(
            json!({"page":name,"sent":true}),
            format!("asked the GUI to open {name}"),
        ))
    })
}
