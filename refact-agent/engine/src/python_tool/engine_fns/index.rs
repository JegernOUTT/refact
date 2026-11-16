use std::sync::atomic::{AtomicBool, Ordering};

use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::common::{redact, to_value, PathFilter};
use super::mask::item;
use super::{previewed, EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput, Preview};

/// One engine-function rescan at a time: the walk is CPU and IO heavy.
static RESCAN_RUNNING: AtomicBool = AtomicBool::new(false);

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "index.status",
            summary: "CodeGraph, VecDB and file-index progress, sizes and errors (a snapshot; call again to watch progress). Error strings are redacted.",
            class: EngineClass::RedactedRead,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"parts":{"type":"array","minItems":1,"uniqueItems":true,"items":{"enum":["codegraph","vecdb","file_index"]},"description":"Every part by default."}}}),
            output: || Some(json!({"type":"object","properties":{"codegraph":{"type":"object","properties":{"state":{"type":"string"},"error":{"type":"string"},"queued":{"type":"integer"},"counts":{"type":"object"},"cross_file_edges":{"type":"integer"},"cross_file_ready":{"type":"boolean"},"throughput_files_per_min":{"type":"number"},"eta_seconds":{"type":"integer"},"parse_failures":{"type":"integer"},"parse_failure_paths":{"type":"array","items":{"type":"string"}},"parse_failure_note":{"type":"string"}}},"vecdb":{"type":"object","properties":{"alive":{"enum":["turned_off","starting","working","error"]},"error":{"type":"string"},"status":{"type":"object"}}},"file_index":{"type":"object"}}})),
            handler: status,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "index.settings",
            summary: "Indexing blocklist and additional indexing directories of the global scope or the project scope (the first workspace folder).",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"required":["scope"],"properties":{"scope":{"enum":["global","project"]}}}),
            output: || Some(json!({"type":"object","required":["scope","path","blocklist","additional_indexing_dirs","project_available"],"properties":{"scope":{"type":"string"},"path":{"type":"string"},"blocklist":{"type":"array","items":{"type":"string"}},"additional_indexing_dirs":{"type":"array","items":{"type":"string"}},"project_available":{"type":"boolean"}}})),
            handler: settings,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "index.rescan",
            summary: "Re-walk every workspace folder and enqueue all files for CodeGraph and VecDB indexing, after the user approves. Returns when the walk has enumerated the files; poll index.status for progress.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"vecdb_only":{"type":"boolean","description":"Enqueue for VecDB only; false by default."}}}),
            output: || Some(json!({"type":"object","required":["files_enqueued"],"properties":{"files_enqueued":{"type":"integer"}}})),
            handler: rescan,
            preview: Some(rescan_preview),
            timeout: Some(std::time::Duration::from_secs(600)),
        },
        EngineFn {
            name: "index.set_settings",
            summary: "Replace the indexing blocklist and/or the additional indexing directories of the global or project scope, after the user approves; a list left out keeps its value. Indexing more files is risky.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["scope"],"properties":{"scope":{"enum":["global","project"]},"blocklist":{"type":"array","items":{"type":"string"}},"additional_indexing_dirs":{"type":"array","items":{"type":"string"}}}}),
            output: || Some(json!({"type":"object","required":["scope","blocklist","additional_indexing_dirs"],"properties":{"scope":{"type":"string"},"path":{"type":"string"},"blocklist":{"type":"array","items":{"type":"string"}},"additional_indexing_dirs":{"type":"array","items":{"type":"string"}}}})),
            handler: set_settings,
            preview: Some(set_settings_preview),
            timeout: Some(std::time::Duration::from_secs(600)),
        },
    ]
}

const MORE_INDEXED_WARNING: &str = "More files will be indexed, and with VecDB enabled their contents are sent to the embedding provider.";

async fn current_settings(
    ctx: &EngineCtx,
    scope: &str,
) -> Result<(Vec<String>, Vec<String>), EngineError> {
    let response =
        crate::http::routers::v1::settings_config::indexing_settings_load(&ctx.app, scope)
            .await
            .map_err(|e| EngineError::Failed(e.message))?;
    let value = to_value(&response.0)?;
    let list = |key: &str| -> Vec<String> {
        value["config"][key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    };
    Ok((list("blocklist"), list("additional_indexing_dirs")))
}

/// The config after the call: given lists replace, missing lists keep their value.
async fn planned_settings(
    ctx: &EngineCtx,
    args: &Value,
) -> Result<
    (
        (Vec<String>, Vec<String>),
        crate::http::routers::v1::settings_config::IndexingSettingsConfig,
    ),
    EngineError,
> {
    let scope = args["scope"].as_str().unwrap_or_default();
    if scope == "project" {
        if let Some(root) = &ctx.project_root {
            super::common::inside_project(
                root,
                &root.join(".refact").join("indexing.yaml"),
                &ctx.app.gcx.config_dir,
            )?;
        }
    }
    let current = current_settings(ctx, scope).await?;
    let given = |key: &str| -> Option<Vec<String>> {
        args.get(key).and_then(Value::as_array).map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
    };
    let config = crate::http::routers::v1::settings_config::IndexingSettingsConfig {
        blocklist: given("blocklist").unwrap_or_else(|| current.0.clone()),
        additional_indexing_dirs: given("additional_indexing_dirs")
            .unwrap_or_else(|| current.1.clone()),
    };
    let config = crate::http::routers::v1::settings_config::normalize_config(scope, config)
        .map_err(|e| EngineError::Type(e.message))?;
    let filter = PathFilter::new(ctx).await?;
    let root = ctx.project_root.clone();
    for dir in config
        .additional_indexing_dirs
        .iter()
        .filter(|dir| !current.1.contains(dir))
    {
        let path = if let Some(rest) = dir.strip_prefix("~/") {
            home::home_dir().unwrap_or_default().join(rest)
        } else if std::path::Path::new(dir).is_absolute() {
            std::path::PathBuf::from(dir)
        } else {
            root.clone().unwrap_or_default().join(dir)
        };
        if !filter.allows(&path.to_string_lossy()) {
            return Err(EngineError::Permission(format!(
                "{dir} is inside a privacy-blocked path or a zone this chat's model may not see; it cannot be indexed"
            )));
        }
    }
    Ok((current, config))
}

fn joined(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".into()
    } else {
        items.join(", ")
    }
}

fn set_settings_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let scope = args["scope"].as_str().unwrap_or_default();
        let ((old_block, old_dirs), config) = planned_settings(ctx, args).await?;
        let scope_label = if scope == "global" {
            "global (all projects)".to_string()
        } else {
            format!(
                "project {}",
                ctx.project_root
                    .as_ref()
                    .map(|r| r.to_string_lossy().into_owned())
                    .unwrap_or_default()
            )
        };
        let mut items = Vec::new();
        let mut widened = false;
        for (label, old, new) in [
            ("blocklist", &old_block, &config.blocklist),
            (
                "additional_indexing_dirs",
                &old_dirs,
                &config.additional_indexing_dirs,
            ),
        ] {
            if old == new {
                continue;
            }
            let added = new.iter().filter(|e| !old.contains(e)).count();
            let removed = old.iter().filter(|e| !new.contains(e)).count();
            widened |= if label == "blocklist" {
                removed > 0
            } else {
                added > 0
            };
            items.push(item(
                label,
                Some(json!(joined(old))),
                Some(json!(format!("{} (+{added}, -{removed})", joined(new)))),
            ));
        }
        let plan = if items.is_empty() {
            super::Plan::nothing()
        } else {
            previewed::plan(json!([old_block, old_dirs]))
        };
        Ok(Preview {
            title: "change indexing settings".into(),
            items,
            scope: Some(scope_label),
            class: if widened {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning: widened.then(|| MORE_INDEXED_WARNING.to_string()),
            plan,
        })
    })
}

fn set_settings<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        check_rescan(ctx)?;
        let scope = args["scope"].as_str().unwrap_or_default();
        let ((old_block, old_dirs), config) = planned_settings(ctx, args).await?;
        previewed::take(ctx, "index.set_settings", &json!([old_block, old_dirs]))?;
        let response = crate::http::routers::v1::settings_config::indexing_settings_save(
            &ctx.app, scope, config,
        )
        .await
        .map_err(|e| EngineError::Failed(e.message))?;
        let response = to_value(&response.0)?;
        Ok(EngineOutput::new(
            json!({
                "scope": scope,
                "path": response["path"],
                "blocklist": response["config"]["blocklist"],
                "additional_indexing_dirs": response["config"]["additional_indexing_dirs"],
            }),
            format!("saved {scope} indexing settings; re-indexing"),
        ))
    })
}

fn wanted(args: &Value, part: &str) -> bool {
    args.get("parts")
        .and_then(Value::as_array)
        .is_none_or(|parts| parts.iter().any(|p| p == part))
}

fn status<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let gcx = ctx.app.gcx.clone();
        let mut data = serde_json::Map::new();
        let mut text = Vec::new();
        if wanted(args, "codegraph") || wanted(args, "vecdb") {
            let rag = crate::http::routers::v1::status::get_rag_status(gcx.clone()).await;
            if wanted(args, "codegraph") {
                let mut codegraph = to_value(&rag.codegraph)?;
                if let Some(error) = codegraph["error"].as_str() {
                    codegraph["error"] = json!(redact(error));
                }
                let filter = PathFilter::new(ctx).await?;
                if let Some(paths) = codegraph["parse_failure_paths"].as_array_mut() {
                    paths.retain(|p| p.as_str().is_some_and(|p| filter.allows(p)));
                }
                text.push(format!(
                    "codegraph {} ({} queued)",
                    codegraph["state"].as_str().unwrap_or_default(),
                    codegraph["queued"]
                ));
                data.insert("codegraph".into(), codegraph);
            }
            if wanted(args, "vecdb") {
                let mut vecdb = json!({"alive":rag.vecdb_alive,"error":redact(&rag.vec_db_error)});
                if let Some(status) = &rag.vecdb {
                    let mut status = to_value(status)?;
                    if let Some(errors) = status["vecdb_errors"].as_object() {
                        let redacted: serde_json::Map<String, Value> =
                            errors.iter().map(|(k, v)| (redact(k), v.clone())).collect();
                        status["vecdb_errors"] = Value::Object(redacted);
                    }
                    vecdb["status"] = status;
                }
                text.push(format!("vecdb {}", rag.vecdb_alive));
                data.insert("vecdb".into(), vecdb);
            }
        }
        if wanted(args, "file_index") {
            let file_index = to_value(gcx.file_index.status())?;
            text.push(format!(
                "file index {} ({} files)",
                file_index["state"].as_str().unwrap_or_default(),
                file_index["entries"]
            ));
            data.insert("file_index".into(), file_index);
        }
        Ok(EngineOutput::new(Value::Object(data), text.join(", ")))
    })
}

fn settings<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let scope = args["scope"].as_str().unwrap_or_default();
        let response =
            crate::http::routers::v1::settings_config::indexing_settings_load(&ctx.app, scope)
                .await
                .map_err(|e| EngineError::Failed(e.message))?;
        let response = to_value(&response.0)?;
        let data = json!({
            "scope": response["scope"],
            "path": response["path"],
            "blocklist": response["config"]["blocklist"],
            "additional_indexing_dirs": response["config"]["additional_indexing_dirs"],
            "project_available": response["project_available"],
        });
        let text = format!(
            "{scope}: {} blocklist entries, {} extra dirs",
            data["blocklist"].as_array().map_or(0, Vec::len),
            data["additional_indexing_dirs"]
                .as_array()
                .map_or(0, Vec::len)
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn check_rescan(ctx: &EngineCtx) -> Result<(), EngineError> {
    if ctx.app.gcx.shutdown_flag.load(Ordering::SeqCst) {
        return Err(EngineError::Failed("the engine is shutting down".into()));
    }
    if RESCAN_RUNNING.load(Ordering::SeqCst) {
        return Err(EngineError::Failed(
            "a rescan started by an engine function is still running; poll index.status".into(),
        ));
    }
    Ok(())
}

fn rescan_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        check_rescan(ctx)?;
        let indexed = ctx.app.gcx.file_index.status().entries;
        let vecdb_only = args.get("vecdb_only").and_then(Value::as_bool) == Some(true);
        let approved = previewed::plan(Value::Null);
        Ok(Preview {
            plan: approved,
            title: "rescan the workspace index".into(),
            items: vec![item(
                "files",
                Some(json!(format!("{indexed} indexed"))),
                Some(json!(if vecdb_only {
                    "re-enqueue all workspace files for VecDB"
                } else {
                    "re-enqueue all workspace files"
                })),
            )],
            scope: Some("engine (all chats, all workspace folders)".into()),
            class: EngineClass::Change,
            warning: None,
        })
    })
}

struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        RESCAN_RUNNING.store(false, Ordering::SeqCst);
    }
}

fn rescan<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        previewed::take(ctx, "index.rescan", &Value::Null)?;
        check_rescan(ctx)?;
        if RESCAN_RUNNING.swap(true, Ordering::SeqCst) {
            return Err(EngineError::Failed(
                "a rescan started by an engine function is still running; poll index.status".into(),
            ));
        }
        let _running = Running;
        let vecdb_only = args.get("vecdb_only").and_then(Value::as_bool) == Some(true);
        let gcx = ctx.app.gcx.clone();
        crate::files_blocklist::reload_indexing_everywhere_now(gcx.clone()).await;
        let count = crate::files_in_workspace::enqueue_all_files_from_workspace_folders(
            gcx, true, vecdb_only,
        )
        .await;
        Ok(EngineOutput::new(
            json!({"files_enqueued":count}),
            format!("enqueued {count} files for indexing; poll index.status for progress"),
        ))
    })
}
