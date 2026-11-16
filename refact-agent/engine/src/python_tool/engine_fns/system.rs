use futures::future::BoxFuture;
use serde_json::{json, Map, Value};

use super::common::{failed, redact, to_value, PathFilter};
use super::mask::item;
use super::{previewed, EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput, Preview};

pub fn functions() -> Vec<EngineFn> {
    let none = || json!({"type":"object","additionalProperties":false,"properties":{}});
    vec![
        EngineFn {
            name: "system.info",
            summary: "Engine version and build, pid, ports, config and cache directories, command-line flags with secrets masked.",
            class: EngineClass::RedactedRead,
            input: none,
            output: || Some(json!({"type":"object","required":["version","build","pid","ports","config_dir","cache_dir","args"],"properties":{"version":{"type":"string"},"build":{"type":"object","additionalProperties":{"type":"string"}},"pid":{"type":"integer"},"ports":{"type":"object","properties":{"http":{"type":"integer"},"lsp":{"type":"integer"}}},"config_dir":{"type":"string"},"cache_dir":{"type":"string"},"args":{"type":"array","items":{"type":"string"}}}})),
            handler: info,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "system.resources",
            summary: "This engine process's memory (RSS), CPU use over a short sample and process uptime.",
            class: EngineClass::Read,
            input: none,
            output: || Some(json!({"type":"object","required":["pid","rss_bytes","cpu_percent","uptime_secs"],"properties":{"pid":{"type":"integer"},"rss_bytes":{"type":"integer"},"cpu_percent":{"type":"number","description":"about 200 ms sample, not an average"},"uptime_secs":{"type":"integer"}}})),
            handler: resources,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "system.perf",
            summary: "Performance telemetry: per-component timings and counters, rollout switches (restart-only, read-only) and chat queue diagnostics.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"components":{"type":"array","items":{"type":"string"},"description":"Component names to keep; every component when absent."}}}),
            output: || Some(json!({"type":"object","required":["enabled","components","rollout_switches","queue_diagnostics"],"properties":{"schema_version":{"type":"integer"},"enabled":{"type":"boolean"},"collection_started_at_ms":{"type":"integer"},"uptime_ms":{"type":"integer"},"components":{"type":"array","items":{"type":"object"}},"rollups":{"type":"object"},"rollout_switches":{"type":"object","additionalProperties":{"type":"boolean"}},"queue_diagnostics":{"type":"object"}}})),
            handler: perf,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "system.logs",
            summary: "The redacted tail of the engine log or the daemon log. Redaction is pattern based (keys, tokens, passwords); lines naming a path this chat's model may not see are dropped.",
            class: EngineClass::RedactedRead,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"source":{"enum":["engine","daemon"],"description":"engine (default) or daemon; the daemon log exists only when a daemon wrote one."},"tail":{"type":"integer","minimum":1,"maximum":2000,"description":"Lines from the end; 200 by default."}}}),
            output: || Some(json!({"type":"object","required":["source","exists","lines","dropped_lines"],"properties":{"source":{"type":"string"},"path":{"type":"string"},"exists":{"type":"boolean"},"lines":{"type":"array","items":{"type":"string"}},"dropped_lines":{"type":"integer","description":"lines withheld because they name a privacy-blocked path"},"read_error":{"type":"string"}}})),
            handler: logs,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "system.errors",
            summary: "Recent distinct error and warning messages from the engine and daemon logs, with counts; redacted, privacy-filtered like system.logs.",
            class: EngineClass::RedactedRead,
            input: none,
            output: || Some(json!({"type":"object","required":["errors","dropped"],"properties":{"errors":{"type":"array","items":{"type":"object","required":["source","level","message","count"],"properties":{"source":{"type":"string"},"level":{"type":"string"},"message":{"type":"string"},"count":{"type":"integer"},"location":{"type":"string"}}}},"dropped":{"type":"integer"}}})),
            handler: errors,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "system.setup",
            summary: "Whether the first project folder is set up for the agent: AGENTS.md, knowledge and trajectories present. No project open reports configured.",
            class: EngineClass::Read,
            input: none,
            output: || Some(json!({"type":"object","required":["configured","reasons","detail"],"properties":{"configured":{"type":"boolean"},"reasons":{"type":"array","items":{"enum":["missing_agents_md","no_knowledge","no_trajectories"]}},"detail":{"type":"object","properties":{"project_root":{"type":"string"},"has_agents_md":{"type":"boolean"},"has_knowledge":{"type":"boolean"},"has_trajectories":{"type":"boolean"}}}}})),
            handler: setup,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "system.browser_settings",
            summary: "Browser automation settings: launch, lifecycle, viewports, timing, capture limits; source says whether browser.yaml was used or defaults.",
            class: EngineClass::RedactedRead,
            input: none,
            output: || Some(json!({"type":"object","required":["settings","source"],"properties":{"settings":{"type":"object","properties":{"launch":{"type":"object"},"lifecycle":{"type":"object"},"viewport":{"type":"object"},"timing":{"type":"object"},"capture":{"type":"object"}}},"source":{"enum":["file","defaults"],"description":"defaults when browser.yaml is absent or invalid"}}})),
            handler: browser_settings,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "system.set_perf",
            summary: "Turn performance telemetry collection on or off, or reset its counters, after the user approves. Live and process-wide, not persisted.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"enabled":{"type":"boolean"},"reset":{"type":"boolean","const":true,"description":"Clear every counter."}}}),
            output: || Some(json!({"type":"object","required":["enabled","reset"],"properties":{"enabled":{"type":"boolean"},"reset":{"type":"boolean"}}})),
            handler: set_perf,
            preview: Some(set_perf_preview),
            timeout: None,
        },
        EngineFn {
            name: "system.reload_config",
            summary: "Re-read customization (modes, subagents, toolbox, code lens) and invalidate provider capabilities after the user approves. Integrations and privacy.yaml engine settings are not reloaded.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"parts":{"type":"array","minItems":1,"uniqueItems":true,"items":{"enum":["customization","caps"]},"description":"Both by default."}}}),
            output: || Some(json!({"type":"object","required":["caps_invalidated"],"properties":{"customization":{"type":"object","properties":{"modes_loaded":{"type":"integer"},"subagents_loaded":{"type":"integer"},"toolbox_commands_loaded":{"type":"integer"},"code_lens_loaded":{"type":"integer"},"errors":{"type":"array","items":{"type":"object","properties":{"file_path":{"type":"string"},"error":{"type":"string"}}}}}},"caps_invalidated":{"type":"boolean"}}})),
            handler: reload_config,
            preview: Some(reload_config_preview),
            timeout: None,
        },
        EngineFn {
            name: "system.bug_bundle",
            summary: "Write a redacted diagnostics zip (log tails, engine context, recent errors) to <cache_dir>/bug-reports after the user approves.",
            class: EngineClass::Change,
            input: none,
            output: || Some(json!({"type":"object","required":["path","size_bytes","files"],"properties":{"path":{"type":"string"},"size_bytes":{"type":"integer"},"files":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"size_bytes":{"type":"integer"}}}}}})),
            handler: bug_bundle,
            preview: Some(bug_bundle_preview),
            timeout: None,
        },
        EngineFn {
            name: "system.set_browser_settings",
            summary: "Change browser automation settings with a patch deep-merged into the current ones, validated, after the user approves. A running browser keeps its launch options until it is relaunched. Weakening isolation is risky.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["patch"],"properties":{"patch":{"type":"object","minProperties":1,"description":"Any subset of the groups system.browser_settings returns."}}}),
            output: || Some(json!({"type":"object","required":["settings"],"properties":{"settings":{"type":"object"}}})),
            handler: set_browser_settings,
            preview: Some(set_browser_settings_preview),
            timeout: None,
        },        EngineFn {
            name: "system.shutdown",
            summary: "Shut the engine down gracefully after this run has finished and its result is stored, after the user approves. Every chat stops; the engine comes back only if the IDE or daemon restarts it.",
            class: EngineClass::Destructive,
            input: none,
            output: || Some(json!({"type":"object","required":["status","completed"],"properties":{"status":{"type":"string"},"completed":{"type":"boolean"}}})),
            handler: shutdown,
            preview: Some(shutdown_preview),
            timeout: None,
        },
    ]
}

const SHUTDOWN_WARNING: &str = "Every chat and script stops, including this one; unsaved runs are lost and the engine only comes back if the IDE or daemon restarts it.";

fn shutdown_preview<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        if ctx.run.is_none() {
            return Err(EngineError::Failed(
                "no run to finish before the shutdown".into(),
            ));
        }
        let chats = ctx.app.chat.sessions.read().await.len();
        Ok(Preview {
            title: "shut down the engine".into(),
            items: vec![item(
                "engine",
                Some(json!(format!("running ({chats} live chats)"))),
                Some(json!("stopping after this run")),
            )],
            scope: Some("engine (all chats)".into()),
            class: EngineClass::Destructive,
            warning: Some(SHUTDOWN_WARNING.into()),
            plan: previewed::plan(json!("running")),
        })
    })
}

fn shutdown<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use std::sync::atomic::Ordering;
    Box::pin(async move {
        let state = if ctx.app.gcx.shutdown_flag.load(Ordering::SeqCst) {
            "stopping"
        } else {
            "running"
        };
        previewed::take(ctx, "system.shutdown", &json!(state))?;
        let run = ctx
            .run
            .as_ref()
            .ok_or_else(|| EngineError::Failed("no run to finish before the shutdown".into()))?;
        run.request_shutdown();
        Ok(EngineOutput::new(
            json!({"status": "requested", "completed": false}),
            "shutdown requested; it starts after this run has finished",
        ))
    })
}

const BROWSER_WARNING: &str = "This weakens browser isolation: pages can run without the Chromium sandbox, with invalid certificates, or with visible passwords.";

const REFUSED_BROWSER_ARGS: &[&str] = &[
    "--remote-debugging",
    "--disable-web-security",
    "--user-data-dir",
];

fn deep_merge(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (key, value) in patch {
                deep_merge(target.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (target, patch) => *target = patch.clone(),
    }
}

fn changed(prefix: &str, old: &Value, new: &Value, out: &mut Vec<(String, Value, Value)>) {
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, value) in b {
                let label = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                changed(&label, a.get(key).unwrap_or(&Value::Null), value, out);
            }
        }
        _ if old != new => out.push((prefix.to_string(), old.clone(), new.clone())),
        _ => {}
    }
}

async fn browser_plan(
    ctx: &EngineCtx,
    args: &Value,
) -> Result<
    (
        crate::http::routers::v1::browser_settings::BrowserSettings,
        crate::http::routers::v1::browser_settings::BrowserSettings,
    ),
    EngineError,
> {
    use crate::http::routers::v1::browser_settings as bs;
    let current = bs::load_from_disk(&ctx.app.gcx.config_dir).await;
    let mut merged = to_value(&current)?;
    deep_merge(&mut merged, &args["patch"]);
    let new: bs::BrowserSettings = serde_json::from_value(merged)
        .map_err(|e| EngineError::Type(format!("invalid browser settings: {e}")))?;
    new.validate().map_err(EngineError::Type)?;
    let launch = &new.launch;
    if let Some(arg) = launch.extra_args.iter().find(|arg| {
        REFUSED_BROWSER_ARGS
            .iter()
            .any(|refused| arg.starts_with(refused))
    }) {
        return Err(EngineError::Permission(format!(
            "{arg} would expose or reuse a browser profile and cannot be set from a script"
        )));
    }
    if !launch.chrome_path.is_empty() && launch.chrome_path != current.launch.chrome_path {
        let path = std::path::Path::new(&launch.chrome_path);
        let executable = std::fs::metadata(path).is_ok_and(|m| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                m.is_file() && m.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                m.is_file()
            }
        });
        if !executable {
            return Err(EngineError::Type(format!(
                "chrome_path {} is not an existing executable file",
                launch.chrome_path
            )));
        }
    }
    Ok((current, new))
}

fn set_browser_settings_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let (current, new) = browser_plan(ctx, args).await?;
        let (old_value, new_value) = (to_value(&current)?, to_value(&new)?);
        let mut leaves = Vec::new();
        changed("", &old_value, &new_value, &mut leaves);
        let weakened = (current.launch.chromium_sandbox && !new.launch.chromium_sandbox)
            || (!current.launch.ignore_https_errors && new.launch.ignore_https_errors)
            || (current.launch.mask_passwords && !new.launch.mask_passwords)
            || new
                .launch
                .extra_args
                .iter()
                .any(|arg| !current.launch.extra_args.contains(arg));
        let mut warnings = Vec::new();
        if weakened {
            warnings.push(BROWSER_WARNING.to_string());
        }
        let (old_launch, new_launch) = (&current.launch, &new.launch);
        if old_launch.chrome_path != new_launch.chrome_path {
            warnings.push(format!(
                "The browser program changes to {}: every later browser launch runs that file.",
                if new_launch.chrome_path.is_empty() {
                    "the default Chromium"
                } else {
                    new_launch.chrome_path.as_str()
                }
            ));
        }
        if old_launch.proxy_server != new_launch.proxy_server
            || old_launch.proxy_bypass != new_launch.proxy_bypass
        {
            warnings.push(
                "Browser traffic is routed differently: the proxy server or its bypass list changes, so the proxy can read or alter the pages the browser loads.".to_string(),
            );
        }
        if old_launch.downloads_dir != new_launch.downloads_dir {
            warnings.push(format!(
                "Files the browser downloads are written to {} instead.",
                if new_launch.downloads_dir.is_empty() {
                    "the default folder"
                } else {
                    new_launch.downloads_dir.as_str()
                }
            ));
        }
        let risky = !warnings.is_empty();
        let items = leaves
            .into_iter()
            .map(|(label, old, new)| {
                let mask = |value: Value| {
                    if label.ends_with("proxy_server") {
                        json!(mask_proxy(value.as_str().unwrap_or_default()))
                    } else {
                        value
                    }
                };
                item(&label, Some(mask(old)), Some(new))
            })
            .collect::<Vec<_>>();
        let plan = if items.is_empty() {
            super::Plan::nothing()
        } else {
            previewed::plan(old_value)
        };
        Ok(Preview {
            title: "change browser settings".into(),
            items,
            scope: Some("engine (all chats)".into()),
            class: if risky {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning: risky.then(|| warnings.join(" ")),
            plan,
        })
    })
}

fn set_browser_settings<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let (current, new) = browser_plan(ctx, args).await?;
        previewed::take(ctx, "system.set_browser_settings", &to_value(&current)?)?;
        crate::http::routers::v1::browser_settings::save_browser_settings(
            &ctx.app.gcx.config_dir,
            &new,
        )
        .await
        .map_err(|e| EngineError::Failed(e.message))?;
        let mut data = to_value(&new)?;
        if let Some(proxy) = data["launch"]["proxy_server"].as_str() {
            data["launch"]["proxy_server"] = json!(mask_proxy(proxy));
        }
        Ok(EngineOutput::new(
            json!({ "settings": data }),
            "browser settings saved; a running browser keeps its options until relaunched",
        ))
    })
}

/// Masks values of secret-looking flags in an argv list, both `--flag value` and `--flag=value`.
pub(crate) fn mask_args(args: Vec<String>) -> Vec<String> {
    let mut masked = Vec::with_capacity(args.len());
    let mut hide_next = false;
    for arg in args {
        if hide_next && !arg.starts_with('-') {
            masked.push(super::mask::MASK.to_string());
            hide_next = false;
            continue;
        }
        hide_next = false;
        if let Some(flag) = arg.strip_prefix('-') {
            let flag = flag.trim_start_matches('-');
            if let Some((name, _)) = flag.split_once('=') {
                if super::mask::secret_looking(name) {
                    let dashes = &arg[..arg.len() - flag.len()];
                    masked.push(format!("{dashes}{name}={}", super::mask::MASK));
                    continue;
                }
            } else if super::mask::secret_looking(flag) {
                hide_next = true;
            }
        }
        masked.push(crate::integrations::mcp::redaction::scrub_text(&arg, &[]));
    }
    masked
}

fn info<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let build: serde_json::Map<String, Value> = crate::http::routers::info::get_build_info()
            .into_iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect();
        let cmdline = &ctx.app.gcx.cmdline;
        let data = json!({
            "version": crate::version::VERSION,
            "build": build,
            "pid": std::process::id(),
            "ports": {"http": cmdline.http_port, "lsp": cmdline.lsp_port},
            "config_dir": ctx.app.paths.config_dir.to_string_lossy(),
            "cache_dir": ctx.app.paths.cache_dir.to_string_lossy(),
            "args": mask_args(std::env::args().collect()),
        });
        let text = format!(
            "refact engine {} (pid {}, http port {})",
            crate::version::VERSION,
            std::process::id(),
            cmdline.http_port
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn resources<'a>(
    _ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let pid = std::process::id();
        let sample = tokio::task::spawn_blocking(move || {
            crate::daemon::resources::worker_resources(&[pid]).remove(&pid)
        })
        .await
        .map_err(failed)?
        .ok_or_else(|| {
            EngineError::Failed("this process is not visible to the system sampler".into())
        })?;
        let text = format!(
            "RSS {} MB, CPU {:.1}%, up {} s",
            sample.rss_bytes / (1024 * 1024),
            sample.cpu_percent,
            sample.uptime_secs
        );
        Ok(EngineOutput::new(
            json!({"pid":pid,"rss_bytes":sample.rss_bytes,"cpu_percent":sample.cpu_percent,"uptime_secs":sample.uptime_secs}),
            text,
        ))
    })
}

fn perf<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let mut data = to_value(ctx.app.gcx.performance_telemetry.snapshot())?;
        if let Some(keep) = args.get("components").and_then(Value::as_array) {
            if let Some(components) = data["components"].as_array_mut() {
                components.retain(|c| keep.contains(&c["component"]));
            }
        }
        data["rollout_switches"] =
            to_value(crate::http::routers::v1::performance::rollout_switches())?;
        data["queue_diagnostics"] = to_value(crate::chat::live_diagnostics::sample(&ctx.app))?;
        let text = format!(
            "telemetry {}, {} components",
            if data["enabled"] == true { "on" } else { "off" },
            data["components"].as_array().map_or(0, Vec::len)
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn logs<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use crate::http::routers::v1::bug_report as br;
    Box::pin(async move {
        let source = args
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("engine");
        let tail =
            br::clamp_tail_lines(args.get("tail").and_then(Value::as_u64).map(|n| n as usize));
        let gcx = &ctx.app.gcx;
        let filter = PathFilter::new(ctx).await?;
        let resolved =
            br::resolve_log_source(source, gcx.cmdline.logs_to_file.clone(), &gcx.cache_dir)
                .await
                .map_err(|e| EngineError::Type(e.message))?;
        let (exists, lines, read_error, dropped) =
            br::read_filtered_redacted_lines(&resolved, tail, |text| filter.allows_text(text))
                .await;
        let mut data = json!({"source":source,"path":resolved.path.to_string_lossy(),"exists":exists,"lines":lines,"dropped_lines":dropped});
        if let Some(error) = read_error {
            data["read_error"] = json!(error);
        }
        let text = format!(
            "{} log lines from {source}",
            data["lines"].as_array().map_or(0, Vec::len)
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn errors<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use crate::http::routers::v1::bug_report as br;
    Box::pin(async move {
        let gcx = &ctx.app.gcx;
        let filter = PathFilter::new(ctx).await?;
        let engine = br::resolve_engine_log(gcx.cmdline.logs_to_file.clone(), &gcx.cache_dir).await;
        let daemon = br::resolve_daemon_log().await;
        let (rows, dropped) =
            br::scan_filtered_recent_errors(&[engine, daemon], 50, |text| filter.allows_text(text))
                .await;
        let rows = to_value(rows)?;
        let mut kept = Vec::new();
        for mut row in rows.as_array().cloned().unwrap_or_default() {
            for key in ["message", "location"] {
                if let Some(text) = row[key].as_str() {
                    row[key] = json!(redact(text));
                }
            }
            kept.push(row);
        }
        let text = format!("{} distinct recent errors", kept.len());
        Ok(EngineOutput::new(
            json!({"errors":kept,"dropped":dropped}),
            text,
        ))
    })
}

fn setup<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let status =
            crate::http::routers::v1::setup_status::setup_status(ctx.app.gcx.clone()).await;
        let text = if status.configured {
            "configured".to_string()
        } else {
            format!("missing: {}", status.reasons.join(", "))
        };
        Ok(EngineOutput::new(to_value(status)?, text))
    })
}

/// Replaces `user:password@` in a proxy URL.
fn mask_proxy(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), url),
    };
    let host_end = rest.find('/').unwrap_or(rest.len());
    match rest[..host_end].rfind('@') {
        Some(at) => format!("{scheme}***@{}", &rest[at + 1..]),
        None => url.to_string(),
    }
}

fn browser_settings<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use crate::http::routers::v1::browser_settings as bs;
    Box::pin(async move {
        let config_dir = ctx.app.gcx.config_dir.clone();
        let settings = bs::load_from_disk(&config_dir).await;
        let from_file = match tokio::fs::read_to_string(bs::settings_path(&config_dir)).await {
            Ok(raw) => serde_yaml::from_str::<bs::BrowserSettings>(&raw)
                .ok()
                .is_some_and(|s| s.validate().is_ok()),
            Err(_) => false,
        };
        let mut data = to_value(&settings)?;
        if let Some(proxy) = data["launch"]["proxy_server"].as_str() {
            data["launch"]["proxy_server"] = json!(mask_proxy(proxy));
        }
        let launch = &settings.launch;
        let text = format!(
            "{}, sandbox {}, desktop {}x{}",
            if launch.headless {
                "headless"
            } else {
                "headed"
            },
            if launch.chromium_sandbox { "on" } else { "off" },
            settings.viewport.desktop.width,
            settings.viewport.desktop.height
        );
        Ok(EngineOutput::new(
            json!({"settings":data,"source":if from_file {"file"} else {"defaults"}}),
            text,
        ))
    })
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

fn perf_state(ctx: &EngineCtx) -> Value {
    let snapshot = ctx.app.gcx.performance_telemetry.snapshot();
    let samples: u64 = snapshot.components.iter().map(|c| c.sample_count).sum();
    json!({"enabled":snapshot.enabled,"samples":samples})
}

fn set_perf_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        if args.get("enabled").is_none() && args.get("reset").is_none() {
            return Err(EngineError::Type(
                "system.set_perf needs enabled or reset".into(),
            ));
        }
        let state = perf_state(ctx);
        let enabled = state["enabled"] == true;
        let mut items = Vec::new();
        if let Some(new) = args.get("enabled").and_then(Value::as_bool) {
            items.push(item(
                "telemetry",
                Some(json!(on_off(enabled))),
                Some(json!(on_off(new))),
            ));
        }
        if args.get("reset").is_some() {
            items.push(item(
                "counters",
                Some(json!(format!("{} samples", state["samples"]))),
                Some(json!("reset")),
            ));
        }
        let approved = previewed::plan(json!({"enabled":enabled}));
        Ok(Preview {
            plan: approved,
            title: "change performance telemetry".into(),
            items,
            scope: Some("engine".into()),
            class: EngineClass::Change,
            warning: None,
        })
    })
}

fn set_perf<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let telemetry = &ctx.app.gcx.performance_telemetry;
        previewed::take(
            ctx,
            "system.set_perf",
            &json!({"enabled":telemetry.enabled()}),
        )?;
        if let Some(enabled) = args.get("enabled").and_then(Value::as_bool) {
            telemetry.set_enabled(enabled);
        }
        let reset = args.get("reset").is_some();
        if reset {
            telemetry.reset();
        }
        let enabled = telemetry.enabled();
        let text = format!(
            "telemetry {}{}",
            on_off(enabled),
            if reset { ", counters reset" } else { "" }
        );
        Ok(EngineOutput::new(
            json!({"enabled":enabled,"reset":reset}),
            text,
        ))
    })
}

fn parts(args: &Value) -> (bool, bool) {
    match args.get("parts").and_then(Value::as_array) {
        Some(parts) => (
            parts.iter().any(|p| p == "customization"),
            parts.iter().any(|p| p == "caps"),
        ),
        None => (true, true),
    }
}

fn reload_config_preview<'a>(
    _ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let (customization, caps) = parts(args);
        let mut items = Vec::new();
        if customization {
            items.push(item(
                "customization",
                Some(json!("cached")),
                Some(json!("re-read from disk")),
            ));
        }
        if caps {
            items.push(item(
                "capabilities",
                Some(json!("cached")),
                Some(json!("reloaded on next use")),
            ));
        }
        let approved = previewed::plan(Value::Null);
        Ok(Preview {
            plan: approved,
            title: "reload engine configuration".into(),
            items,
            scope: Some("engine (all chats)".into()),
            class: EngineClass::Change,
            warning: None,
        })
    })
}

fn reload_config<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        previewed::take(ctx, "system.reload_config", &Value::Null)?;
        let (customization, caps) = parts(args);
        let gcx = ctx.app.gcx.clone();
        let mut data = Map::new();
        let mut text = Vec::new();
        if customization {
            crate::yaml_configs::customization_registry::invalidate_all_registry_caches(
                gcx.clone(),
            )
            .await;
            let registry = crate::yaml_configs::customization_registry::load_merged_registry(
                &gcx.config_dir,
                ctx.project_root.as_deref(),
            )
            .await;
            let errors: Vec<Value> = registry
                .errors
                .iter()
                .map(|e| json!({"file_path":e.file_path,"error":redact(&e.error)}))
                .collect();
            text.push(format!(
                "reloaded: {} modes, {} subagents, {} errors",
                registry.modes.len(),
                registry.subagents.len(),
                errors.len()
            ));
            data.insert(
                "customization".into(),
                json!({"modes_loaded":registry.modes.len(),"subagents_loaded":registry.subagents.len(),"toolbox_commands_loaded":registry.toolbox_commands.len(),"code_lens_loaded":registry.code_lens.len(),"errors":errors}),
            );
        }
        if caps {
            crate::providers::http::invalidate_caps(gcx.clone()).await;
            text.push("capabilities reload on next use".into());
        }
        data.insert("caps_invalidated".into(), json!(caps));
        text.push("integrations and privacy.yaml engine settings are not reloaded".into());
        Ok(EngineOutput::new(Value::Object(data), text.join("; ")))
    })
}

fn bug_reports_dir(ctx: &EngineCtx) -> std::path::PathBuf {
    ctx.app.gcx.cache_dir.join("bug-reports")
}

fn bug_bundle_preview<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let dir = bug_reports_dir(ctx);
        let approved = previewed::plan(Value::Null);
        Ok(Preview {
            plan: approved,
            title: "write a diagnostics bundle".into(),
            items: vec![
                item(
                    "file",
                    None,
                    Some(json!(format!("{}/refact-bug-<time>.zip", dir.to_string_lossy()))),
                ),
                item(
                    "contents",
                    None,
                    Some(json!("engine log tail (4 MB), daemon log tail, engine context, recent errors, redacted")),
                ),
            ],
            scope: Some("local disk".into()),
            class: EngineClass::Change,
            warning: None,
        })
    })
}

fn bug_bundle<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use crate::http::routers::v1::bug_report as br;
    Box::pin(async move {
        previewed::take(ctx, "system.bug_bundle", &Value::Null)?;
        let gcx = &ctx.app.gcx;
        let response = br::build_bug_report_bundle(br::BugReportBundleInput {
            dest_dir: bug_reports_dir(ctx),
            context: br::build_context_response(&ctx.app).await,
            engine_log: br::resolve_engine_log(gcx.cmdline.logs_to_file.clone(), &gcx.cache_dir)
                .await,
            daemon_log: br::resolve_daemon_log().await,
            redact: true,
            webui_lines: None,
            ide_lines: None,
        })
        .await
        .map_err(|e| EngineError::Failed(e.message))?;
        let data = to_value(response)?;
        let text = format!(
            "bundle written to {} ({} KB)",
            data["path"].as_str().unwrap_or_default(),
            data["size_bytes"].as_u64().unwrap_or_default() / 1024
        );
        Ok(EngineOutput::new(data, text))
    })
}

#[cfg(test)]
pub(super) fn mask_proxy_for_test(url: &str) -> String {
    mask_proxy(url)
}
