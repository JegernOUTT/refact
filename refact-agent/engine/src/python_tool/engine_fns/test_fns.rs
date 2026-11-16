//! Functions that exist only in tests, to exercise the pipeline.

use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::mask::item;
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput, Preview};

pub fn functions() -> Vec<EngineFn> {
    let open = || json!({"type":"object","additionalProperties":false,"properties":{"n":{"type":"integer"},"note":{"type":"string"},"warn":{"type":"string"}}});
    vec![
        EngineFn {
            name: "testing.content",
            summary: "Returns file content with a privacy record.",
            class: EngineClass::Read,
            input: open,
            output: || None,
            handler: content,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "testing.secrets",
            summary: "Returns secret-looking fields.",
            class: EngineClass::RedactedRead,
            input: open,
            output: || None,
            handler: secrets,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "testing.slow",
            summary: "Sleeps past its own timeout.",
            class: EngineClass::Read,
            input: open,
            output: || None,
            handler: slow,
            preview: None,
            timeout: Some(Duration::from_millis(200)),
        },
        EngineFn {
            name: "testing.fails",
            summary: "Fails.",
            class: EngineClass::Read,
            input: open,
            output: || None,
            handler: fails,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "testing.touch",
            summary: "A change whose preview carries a secret-looking value.",
            class: EngineClass::Change,
            input: open,
            output: || None,
            handler: touch,
            preview: Some(touch_preview),
            timeout: None,
        },
        EngineFn {
            name: "testing.wipe",
            summary: "A destructive change.",
            class: EngineClass::Destructive,
            input: open,
            output: || None,
            handler: touch,
            preview: Some(wipe_preview),
            timeout: None,
        },
        EngineFn {
            name: "testing.mark",
            summary: "A change that writes a marker file and can panic, sleep past its timeout or return a large result.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["path"],"properties":{"path":{"type":"string"},"name":{"type":"string"},"mode":{"enum":["ok","panic","hang","big"]},"secret_token":{"type":"string"}}}),
            output: || None,
            handler: mark,
            preview: Some(mark_preview),
            timeout: Some(Duration::from_millis(300)),
        },
        EngineFn {
            name: "testing.unplanned",
            summary: "A change whose handler forgets the approved plan.",
            class: EngineClass::Change,
            input: open,
            output: || None,
            handler: unplanned,
            preview: Some(touch_preview),
            timeout: None,
        },
        EngineFn {
            name: "testing.ctx",
            summary: "Reports the run facts the context carries.",
            class: EngineClass::Read,
            input: open,
            output: || None,
            handler: ctx_facts,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "testing.set",
            summary: "A change that sets one labelled value, to test what a card may carry.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["label","value"],"properties":{"label":{"type":"string"},"value":{}}}),
            output: || None,
            handler: touch,
            preview: Some(set_preview),
            timeout: None,
        },
        EngineFn {
            name: "testing.unlocked",
            summary: "A change whose handler runs without the engine-wide apply lock and waits until the marker file named by path exists.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["path"],"properties":{"path":{"type":"string"}}}),
            output: || None,
            handler: unlocked,
            preview: Some(mark_preview),
            timeout: Some(Duration::from_secs(20)),
        },
        EngineFn {
            name: "testing.noop",
            summary: "A change whose plan changes nothing, or (with broken=true) lists no item.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"broken":{"type":"boolean"}}}),
            output: || None,
            handler: touch,
            preview: Some(noop_preview),
            timeout: None,
        },
    ]
}

fn mark_preview<'a>(
    _: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        Ok(Preview {
            title: "write a marker".into(),
            items: vec![item("marker", None, Some(args["path"].clone()))],
            scope: None,
            class: EngineClass::Change,
            warning: None,
            plan: super::previewed::plan(Value::Null),
        })
    })
}

fn mark<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        super::previewed::take(ctx, "testing.mark", &Value::Null)?;
        std::fs::write(
            args["path"].as_str().unwrap_or_default(),
            args["secret_token"].as_str().unwrap_or("applied"),
        )
        .unwrap();
        match args["mode"].as_str() {
            Some("panic") => panic!("handler broke after applying"),
            Some("hang") => {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(EngineOutput::default())
            }
            Some("big") => Ok(EngineOutput::new(json!("x".repeat(200_000)), "big")),
            _ => Ok(EngineOutput::new(json!({"name":args["name"]}), "marked")),
        }
    })
}

fn set_preview<'a>(
    _: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        Ok(Preview {
            title: "set a test value".into(),
            items: vec![item(
                args["label"].as_str().unwrap_or_default(),
                None,
                Some(args["value"].clone()),
            )],
            scope: None,
            class: EngineClass::Change,
            warning: None,
            plan: super::previewed::plan(Value::Null),
        })
    })
}

/// Waits (polling, bounded by its timeout) until `<path>.release` exists, then writes `path`.
fn unlocked<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        super::previewed::take(ctx, "testing.unlocked", &Value::Null)?;
        let path = args["path"].as_str().unwrap_or_default();
        std::fs::write(format!("{path}.started"), "").unwrap();
        while !std::path::Path::new(&format!("{path}.release")).exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        std::fs::write(path, "applied").unwrap();
        Ok(EngineOutput::new(json!(null), "released"))
    })
}

fn ctx_facts<'a>(
    ctx: &'a EngineCtx,
    _: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let destination = refact_privacy::Destination {
            id: refact_privacy::DestinationId("example".into()),
            kind: refact_privacy::DestinationKind::Provider,
            display_name: "example".into(),
        };
        Ok(EngineOutput::new(
            json!({
                "outer": ctx.outer_tool_call_id,
                "used": ctx.nested.used,
                "limit": ctx.nested.limit,
                "parallel": ctx.nested.parallel_limit,
                "llm_calls": ctx.llm.as_ref().map(|budget| budget.max_calls),
                "egress": ctx.clear_for(&destination).await.err(),
            }),
            "facts",
        ))
    })
}

fn noop_preview<'a>(
    _: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let broken = args["broken"] == true;
        Ok(Preview {
            title: "change nothing".into(),
            items: Vec::new(),
            scope: None,
            class: EngineClass::Change,
            warning: None,
            plan: if broken {
                super::previewed::plan(Value::Null)
            } else {
                super::Plan::nothing()
            },
        })
    })
}

fn content<'a>(_: &'a EngineCtx, _: &'a Value) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let mut output = EngineOutput::new(json!({"content":"zone text"}), "one file");
        output.privacy.push(
            json!({"files":[{"path":"zone_a.txt","zone":"blocked","attribution":"observed"}]}),
        );
        Ok(output)
    })
}

fn secrets<'a>(_: &'a EngineCtx, _: &'a Value) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        Ok(EngineOutput::new(
            json!({"api_key":"sk-live-1","nested":{"Authorization":"Bearer abc.def","cookie":"c=1","count_tokens":7},"args":["--token","tok-2","--port","8001"],"url":"https://user:pw@host/x?token=q1","plain":"visible"}),
            "Authorization: Bearer abc.def",
        ))
    })
}

fn slow<'a>(_: &'a EngineCtx, _: &'a Value) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(EngineOutput::default())
    })
}

fn fails<'a>(_: &'a EngineCtx, _: &'a Value) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move { Err(EngineError::Failed("broken on purpose".into())) })
}

fn touch<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        super::previewed::take(ctx, "testing.touch", &Value::Null)?;
        Ok(EngineOutput::new(json!({"n":args["n"]}), "touched"))
    })
}

fn unplanned<'a>(
    _: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move { Ok(EngineOutput::new(json!({"n":args["n"]}), "touched")) })
}

fn touch_preview<'a>(
    _: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        Ok(Preview {
            title: "touch a test value".into(),
            items: vec![
                item("n", Some(json!(1)), Some(args["n"].clone())),
                item("api_token", Some(json!("old-secret")), None),
                item("big", Some(json!("x".repeat(20_000))), Some(json!(0))),
            ],
            scope: Some("project".into()),
            class: EngineClass::Change,
            warning: args.get("warn").and_then(Value::as_str).map(String::from),
            plan: super::previewed::plan(Value::Null),
        })
    })
}

fn wipe_preview<'a>(_: &'a EngineCtx, _: &'a Value) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        Ok(Preview {
            title: "wipe a test value".into(),
            items: vec![item("n", Some(json!(1)), None)],
            scope: None,
            class: EngineClass::Read,
            warning: Some("The test value cannot be restored.".into()),
            plan: super::previewed::plan(Value::Null),
        })
    })
}
