use refact_apps_core::StateScope;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;
use crate::app_state::AppState;
#[derive(Clone)]
pub enum Caller {
    Host,
    HostChat {
        chat_id: String,
    },
    Tool {
        chat_id: String,
        model: Option<String>,
    },
}
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Help {},
    Inspect {
        app_id: Uuid,
    },
    Details {
        app_id: Uuid,
    },
    ContextStage {
        view_id: Uuid,
        view_token: String,
        text: String,
        label: Option<String>,
    },
    Open {
        app_id: Uuid,
        chat_id: Option<String>,
        binding_id: Option<Uuid>,
    },
    ToolEventEmit {
        app_id: Uuid,
        chat_id: Option<String>,
        binding_id: Option<Uuid>,
        channel: String,
        data: Value,
    },
    ViewHeartbeat {
        view_id: Uuid,
        view_token: String,
        #[serde(default)]
        visible: Option<bool>,
        #[serde(default)]
        focused: Option<bool>,
        #[serde(default)]
        ready: Option<bool>,
    },
    ViewList {},
    ViewControl {
        view_id: Uuid,
        operation: String,
    },
    EventList {},
    EventRead {
        seq: u64,
    },
    EventSchema {},
    SourceList {
        app_id: Uuid,
    },
    SourceRead {
        app_id: Uuid,
        source_id: String,
    },
    SourceStatus {
        app_id: Uuid,
        source_id: String,
    },
    List {},
    Get {
        app_id: Uuid,
    },
    Create {
        slug: String,
        name: String,
    },
    ReadFile {
        app_id: Uuid,
        path: String,
    },
    WriteFile {
        app_id: Uuid,
        path: String,
        content: String,
        expected_revision: String,
    },
    Publish {
        app_id: Uuid,
        expected_revision: String,
    },
    Rollback {
        app_id: Uuid,
        revision: String,
        expected_active: Option<String>,
    },
    Rename {
        app_id: Uuid,
        new_slug: String,
        expected_revision: String,
    },
    Clone {
        app_id: Uuid,
        new_slug: String,
    },
    Archive {
        app_id: Uuid,
        expected_revision: String,
    },
    Remove {
        app_id: Uuid,
        expected_revision: String,
    },
    StateGet {
        app_id: Uuid,
        scope: StateScope,
        chat_id: Option<String>,
        view_id: Option<Uuid>,
        view_token: Option<String>,
    },
    StateSet {
        app_id: Uuid,
        scope: StateScope,
        chat_id: Option<String>,
        view_id: Option<Uuid>,
        view_token: Option<String>,
        expected_revision: u64,
        value: Value,
    },
    ViewRegister {
        app_id: Uuid,
        chat_id: Option<String>,
        binding_id: Option<Uuid>,
    },
    ViewUnregister {
        view_id: Uuid,
        view_token: String,
    },
    Capture {
        view_id: Uuid,
    },
    CaptureResponse {
        view_id: Uuid,
        view_token: String,
        request_id: Uuid,
        data_url: Option<String>,
        #[serde(default)]
        metadata: Value,
        #[serde(default)]
        error: Option<String>,
    },
    EventEmit {
        view_id: Uuid,
        view_token: String,
        channel: String,
        data: Value,
    },
    ActionRequest {
        view_id: Uuid,
        view_token: String,
        action_id: String,
        payload: Value,
        request_id: Uuid,
    },
    SourceControl {
        app_id: Uuid,
        source_id: String,
        operation: String,
        #[serde(default)]
        confirmed: bool,
    },
}
async fn checked_view(
    app: &AppState,
    runtime: &super::AppRuntime,
    id: Uuid,
    token: Option<&str>,
) -> Result<super::views::View, String> {
    let view = runtime.views.lock().await.check(id, token)?;
    super::validate_binding(
        app,
        runtime,
        view.app_id,
        view.chat_id.as_deref(),
        view.binding_id,
    )
    .await?;
    let record = runtime.store.get_by_id(view.app_id).await?;
    if record.active_revision.as_deref() != Some(&view.revision) {
        runtime.views.lock().await.remove(id);
        return Err("view published revision changed".into());
    }
    Ok(view)
}
async fn state_authority(
    app: &AppState,
    runtime: &super::AppRuntime,
    caller: &Caller,
    app_id: Uuid,
    scope: &StateScope,
    chat_id: Option<&str>,
    view_id: Option<Uuid>,
    token: Option<&str>,
) -> Result<(), String> {
    let binding = match scope {
        StateScope::Binding {
            binding_id,
        } => Some(*binding_id),
        _ => None,
    };
    match caller {
        Caller::Host
        | Caller::HostChat {
            ..
        } => {
            let view = checked_view(
                app,
                runtime,
                view_id.ok_or("host state access requires view")?,
                token,
            )
            .await?;
            if view.app_id != app_id
                || view.chat_id.as_deref() != chat_id
                || binding != view.binding_id
            {
                return Err("state scope does not match view".into());
            }
            if matches!(scope, StateScope::Shared) {
                return Err("shared state requires trusted tool authority".into());
            }
        }
        Caller::Tool {
            chat_id: own, ..
        } => {
            if chat_id.is_some_and(|id| id != own) {
                return Err("tool cannot target another chat state".into());
            }
            super::validate_binding(app, runtime, app_id, chat_id, binding).await?;
        }
    }
    Ok(())
}
fn serial(value: impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}
pub async fn execute(app: &AppState, caller: Caller, command: Command) -> Result<Value, String> {
    if app
        .runtime
        .shutdown_flag
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return Err("app runtime shutting down".into());
    }
    let runtime = super::runtime(app, &caller).await?;
    let metadata_only = matches!(
        &command,
        Command::List {}
            | Command::Help {}
            | Command::Details { .. }
            | Command::Get { .. }
            | Command::Create { .. }
            | Command::ReadFile { .. }
            | Command::WriteFile { .. }
            | Command::Publish { .. }
            | Command::Rollback { .. }
            | Command::Rename { .. }
            | Command::Clone { .. }
            | Command::Archive { .. }
            | Command::Remove { .. }
            | Command::ViewList {}
            | Command::Open { .. }
            | Command::ViewControl { .. }
            | Command::EventSchema {}
            | Command::SourceControl { .. }
    );
    let _privacy_records = if metadata_only {
        super::privacy::preflight_metadata(app, &runtime, &caller).await?
    } else {
        super::privacy::preflight(app, &runtime, &caller).await?
    };
    let output = match command {
        Command::Help {} => return Ok(super::protocol::authoring_contract()),
        Command::Details {
            app_id,
        } => {
            if !matches!(caller, Caller::Host | Caller::HostChat { .. }) {
                return Err("app catalog details are host-only".into());
            }
            return super::details::get(app, &runtime, app_id).await;
        }
        Command::Inspect {
            app_id,
        } => {
            require_bound_app(app, &caller, app_id).await?;
            let record = serial(runtime.store.get_by_id(app_id).await?)?;
            let sources = serial(runtime.automation.snapshot(&app_id.to_string()).await)?;
            let chat_id = match &caller {
                Caller::Tool {
                    chat_id, ..
                }
                | Caller::HostChat {
                    chat_id,
                } => Some(chat_id.as_str()),
                Caller::Host => None,
            };
            let views = runtime.views.lock().await.list(chat_id);
            let views = Value::Array(
                views
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|view| view["app_id"].as_str() == Some(&app_id.to_string()))
                    .cloned()
                    .collect(),
            );
            let events = scoped_events(app, &runtime, &caller)
                .await?
                .into_iter()
                .filter(|event| event.data["app_id"].as_str() == Some(&app_id.to_string()))
                .collect::<Vec<_>>();
            let mut result = super::protocol::diagnostics(record, sources, views, serial(events)?);
            result["source_records"] = serial(&_privacy_records)?;
            return Ok(result);
        }
        Command::ContextStage {
            view_id,
            view_token,
            text,
            label,
        } => {
            if !matches!(caller, Caller::Host | Caller::HostChat { .. }) {
                return Err("context staging is host-only".into());
            }
            let view = checked_view(app, &runtime, view_id, Some(&view_token)).await?;
            return super::context::stage(app, &runtime, view_id, &view, text, label).await;
        }
        Command::Open {
            app_id,
            chat_id,
            binding_id,
        } => {
            if let Caller::Tool {
                chat_id: own, ..
            } = &caller
            {
                if chat_id.as_ref() != Some(own) {
                    return Err("tool open requires its own chat binding".into());
                }
            }
            super::validate_binding(app, &runtime, app_id, chat_id.as_deref(), binding_id).await?;
            let record = runtime.store.get_by_id(app_id).await?;
            runtime.store.published(&record.slug).await?;
            let request_id = Uuid::new_v4();
            runtime.events.emit("view_request",json!({"request_id":request_id,"app_id":app_id,"chat_id":chat_id,"binding_id":binding_id,"method":"open"}));
            return Ok(json!({"status":"requested","request_id":request_id}));
        }
        Command::ToolEventEmit {
            app_id,
            chat_id,
            binding_id,
            channel,
            data,
        } => {
            let Caller::Tool {
                chat_id: own, ..
            } = &caller
            else {
                return Err("tool event requires tool authority".into());
            };
            if chat_id.as_ref() != Some(own) {
                return Err("tool event requires its own chat binding".into());
            }
            super::validate_binding(app, &runtime, app_id, chat_id.as_deref(), binding_id).await?;
            refact_apps_core::validate_identifier(&channel)?;
            if serde_json::to_vec(&data).map_err(|e| e.to_string())?.len() > 65536 {
                return Err("event too large".into());
            }
            runtime.events.emit(
                "app_event",
                json!({"app_id":app_id,"binding_id":binding_id,"channel":channel,"data":data}),
            );
            return Ok(json!({"accepted":true}));
        }
        Command::ViewHeartbeat {
            view_id,
            view_token,
            visible,
            focused,
            ready,
        } => {
            checked_view(app, &runtime, view_id, Some(&view_token)).await?;
            runtime.views.lock().await.heartbeat(
                view_id,
                Some(&view_token),
                super::views::ViewPresence {
                    visible,
                    focused,
                    ready,
                },
            )?;
            return Ok(json!({"alive":true,"lease_ms":super::views::LEASE_MS}));
        }
        Command::ViewList {} => {
            return Ok(runtime.views.lock().await.list(match &caller {
                Caller::Tool {
                    chat_id, ..
                } => Some(chat_id),
                _ => None,
            }))
        }
        Command::ViewControl {
            view_id,
            operation,
        } => {
            let view = runtime.views.lock().await.lookup(view_id)?;
            if let Caller::Tool {
                chat_id, ..
            } = &caller
            {
                if view.chat_id.as_ref() != Some(chat_id) {
                    return Err("view belongs to another chat".into());
                }
            }
            if !["focus", "close", "open"].contains(&operation.as_str()) {
                return Err("invalid view operation".into());
            }
            runtime.events.emit(
                "view_request",
                json!({"view_id":view_id,"method":operation,"generation":view.generation}),
            );
            return Ok(json!({"status":"requested","operation":operation}));
        }
        Command::EventList {} => return serial(scoped_events(app, &runtime, &caller).await?),
        Command::EventRead {
            seq,
        } => {
            return serial(
                scoped_events(app, &runtime, &caller)
                    .await?
                    .into_iter()
                    .find(|e| e.seq == seq)
                    .ok_or("event unavailable or expired")?,
            )
        }
        Command::EventSchema {} => {
            return Ok(
                json!({"envelope":{"generation":"uuid","seq":"u64","kind":"string","data":"json"},"capacity":256}),
            )
        }
        Command::SourceList {
            app_id,
        } => {
            require_bound_app(app, &caller, app_id).await?;
            return serial(runtime.automation.snapshot(&app_id.to_string()).await);
        }
        Command::SourceRead {
            app_id,
            source_id,
        }
        | Command::SourceStatus {
            app_id,
            source_id,
        } => {
            require_bound_app(app, &caller, app_id).await?;
            return serial(
                runtime
                    .automation
                    .snapshot(&app_id.to_string())
                    .await
                    .into_iter()
                    .find(|s| s.source_id == source_id)
                    .ok_or("source unavailable")?,
            );
        }
        Command::List {} => {
            let (listing, mut artwork) = runtime.store.list_projected(super::icon::project).await?;
            let mut result = serial(&listing)?;
            if let Some(apps) = result["apps"].as_array_mut() {
                for (value, record) in apps.iter_mut().zip(&listing.apps) {
                    if let Some(Some(icon)) = artwork.remove(&record.slug) {
                        value["icon_artwork"] = serial(icon)?;
                    }
                }
            }
            return Ok(result);
        }
        Command::Get {
            app_id,
        } => return serial(runtime.store.get_by_id(app_id).await?),
        Command::Create {
            slug,
            name,
        } => serial(runtime.store.create(&slug, &name).await?)?,
        Command::ReadFile {
            app_id,
            path,
        } => {
            let record = runtime.store.get_by_id(app_id).await?;
            let bytes = runtime.store.read_file(&record.slug, &path).await?;
            return Ok(
                json!({"app_id":app_id,"path":path,"content":String::from_utf8(bytes).map_err(|_|"binary resource requires file tools")?,"revision":record.revision,"source_records":[{"path":runtime.root.join(".refact/apps").join(&record.slug).join(&path)}]}),
            );
        }
        Command::WriteFile {
            app_id,
            path,
            content,
            expected_revision,
        } => {
            let record = runtime.store.get_by_id(app_id).await?;
            serial(
                runtime
                    .store
                    .write_file(
                        &record.slug,
                        &path,
                        content.into_bytes(),
                        &expected_revision,
                    )
                    .await?,
            )?
        }
        Command::Publish {
            app_id,
            expected_revision,
        } => {
            let _activation = runtime.activation.lock().await;
            let record = runtime.store.get_by_id(app_id).await?;
            if record.manifest.sources.len() > 128 {
                return Err("source definition limit exceeded".into());
            }
            let bundle = runtime
                .store
                .publish(&record.slug, &expected_revision)
                .await?;
            runtime
                .automation
                .configure(
                    &app_id.to_string(),
                    bundle.revision.clone(),
                    bundle.manifest.sources.clone(),
                )
                .await?;
            runtime
                .process_sources
                .unconfigure(&app_id.to_string())
                .await;
            runtime.views.lock().await.invalidate(app_id);
            json!({"app_id":app_id,"revision":bundle.revision,"manifest":bundle.manifest})
        }
        Command::Rollback {
            app_id,
            revision,
            expected_active,
        } => {
            let _activation = runtime.activation.lock().await;
            let record = runtime.store.get_by_id(app_id).await?;
            let bundle = runtime
                .store
                .rollback(&record.slug, &revision, expected_active.as_deref())
                .await?;
            runtime
                .automation
                .configure(
                    &app_id.to_string(),
                    bundle.revision.clone(),
                    bundle.manifest.sources.clone(),
                )
                .await?;
            runtime
                .process_sources
                .unconfigure(&app_id.to_string())
                .await;
            runtime.views.lock().await.invalidate(app_id);
            json!({"app_id":app_id,"revision":bundle.revision,"manifest":bundle.manifest})
        }
        Command::Rename {
            app_id,
            new_slug,
            expected_revision,
        } => {
            let record = runtime.store.get_by_id(app_id).await?;
            serial(
                runtime
                    .store
                    .rename(&record.slug, &new_slug, &expected_revision)
                    .await?,
            )?
        }
        Command::Clone {
            app_id,
            new_slug,
        } => {
            let record = runtime.store.get_by_id(app_id).await?;
            serial(runtime.store.clone_app(&record.slug, &new_slug).await?)?
        }
        Command::Archive {
            app_id,
            expected_revision,
        } => {
            let _activation = runtime.activation.lock().await;
            let record = runtime.store.get_by_id(app_id).await?;
            let id = runtime
                .store
                .archive(&record.slug, &expected_revision)
                .await?;
            runtime.views.lock().await.invalidate(app_id);
            runtime
                .process_sources
                .unconfigure(&app_id.to_string())
                .await;
            runtime.automation.unconfigure(&app_id.to_string()).await?;
            json!({"archive_id":id})
        }
        Command::Remove {
            app_id,
            expected_revision,
        } => {
            let _activation = runtime.activation.lock().await;
            let record = runtime.store.get_by_id(app_id).await?;
            runtime
                .store
                .remove(&record.slug, &expected_revision)
                .await?;
            runtime.views.lock().await.invalidate(app_id);
            runtime
                .process_sources
                .unconfigure(&app_id.to_string())
                .await;
            runtime.automation.unconfigure(&app_id.to_string()).await?;
            json!({"removed":true})
        }
        Command::StateGet {
            app_id,
            scope,
            chat_id,
            view_id,
            view_token,
        } => {
            state_authority(
                app,
                &runtime,
                &caller,
                app_id,
                &scope,
                chat_id.as_deref(),
                view_id,
                view_token.as_deref(),
            )
            .await?;
            let record = runtime.store.get_by_id(app_id).await?;
            return serial(runtime.store.read_state(&record.slug, scope).await?);
        }
        Command::StateSet {
            app_id,
            scope,
            chat_id,
            view_id,
            view_token,
            expected_revision,
            value,
        } => {
            state_authority(
                app,
                &runtime,
                &caller,
                app_id,
                &scope,
                chat_id.as_deref(),
                view_id,
                view_token.as_deref(),
            )
            .await?;
            let record = runtime.store.get_by_id(app_id).await?;
            let state = runtime
                .store
                .write_state(&record.slug, scope.clone(), expected_revision, value)
                .await?;
            runtime.events.emit(
                "state_changed",
                json!({"app_id":app_id,"scope":scope,"revision":state.revision}),
            );
            return serial(state);
        }
        Command::ViewRegister {
            app_id,
            chat_id,
            binding_id,
        } => {
            if !matches!(caller, Caller::Host | Caller::HostChat { .. }) {
                return Err("view registration is host-only".into());
            }
            super::validate_binding(app, &runtime, app_id, chat_id.as_deref(), binding_id).await?;
            let record = runtime.store.get_by_id(app_id).await?;
            let bundle = runtime.store.published(&record.slug).await?;
            return runtime.views.lock().await.register(
                app_id,
                chat_id,
                binding_id,
                bundle.revision,
            );
        }
        Command::ViewUnregister {
            view_id,
            view_token,
        } => {
            let _activation = runtime.activation.lock().await;
            if !matches!(caller, Caller::Host | Caller::HostChat { .. }) {
                return Err("view removal is host-only".into());
            }
            runtime
                .views
                .lock()
                .await
                .check(view_id, Some(&view_token))?;
            runtime.views.lock().await.remove(view_id);
            return Ok(json!({"removed":true}));
        }
        Command::Capture {
            view_id,
        } => {
            let Caller::Tool {
                chat_id, ..
            } = &caller
            else {
                return Err("capture requests require trusted tool authority".into());
            };
            let view = runtime.views.lock().await.lookup(view_id)?;
            if view.chat_id.as_ref() != Some(chat_id) {
                return Err("capture view belongs to another chat".into());
            }
            super::validate_binding(
                app,
                &runtime,
                view.app_id,
                view.chat_id.as_deref(),
                view.binding_id,
            )
            .await?;
            let (request_id, rx) = runtime.views.lock().await.request(view_id, &view)?;
            runtime.events.emit("view_request",json!({"view_id":view_id,"request_id":request_id,"method":"capture","generation":view.generation,"revision":view.revision}));
            let result = tokio::time::timeout(std::time::Duration::from_secs(20), rx).await;
            runtime.views.lock().await.cancel(request_id);
            let mut result = result
                .map_err(|_| "capture timed out")?
                .map_err(|_| "capture view disconnected")?;
            if result["status"] == "error" {
                return Err(format!(
                    "capture failed: {}",
                    result["error"].as_str().unwrap_or("app error")
                ));
            }
            if let Some(object) = result.as_object_mut() {
                object.insert("source_records".into(), serial(_privacy_records)?);
            }
            return Ok(result);
        }
        Command::CaptureResponse {
            view_id,
            view_token,
            request_id,
            data_url,
            metadata,
            error,
        } => {
            if !matches!(caller, Caller::Host | Caller::HostChat { .. }) {
                return Err("capture upload is host-only".into());
            }
            let view = checked_view(app, &runtime, view_id, Some(&view_token)).await?;
            if let Some(data_url) = &data_url {
                validate_capture(data_url)?;
            }
            if let Some(error) = error {
                runtime
                    .views
                    .lock()
                    .await
                    .reject(request_id, view_id, &view, &error)?;
                return Ok(json!({"accepted":true}));
            }
            let mut metadata = metadata;
            if let Some(object) = metadata.as_object_mut() {
                object.remove("dataUrl");
                object.remove("data_url");
            }
            if serde_json::to_vec(&metadata)
                .map_err(|e| e.to_string())?
                .len()
                > 16384
            {
                return Err("capture metadata too large".into());
            }
            if !metadata.is_null() && !metadata.is_object() {
                return Err("invalid capture metadata".into());
            }
            let status = if data_url.is_some() {
                "captured"
            } else {
                "unsupported"
            };
            let result = json!({"view_id":view_id,"app_id":view.app_id,"chat_id":view.chat_id,"binding_id":view.binding_id,"revision":view.revision,"generation":view.generation,"data_url":data_url,"metadata":metadata,"method":"dom_raster","trust":"app_reported","status":status,"source_records":[]});
            runtime
                .views
                .lock()
                .await
                .respond(request_id, view_id, &view, result)?;
            return Ok(json!({"accepted":true}));
        }
        Command::EventEmit {
            view_id,
            view_token,
            channel,
            data,
        } => {
            let view = checked_view(app, &runtime, view_id, Some(&view_token)).await?;
            refact_apps_core::validate_identifier(&channel)?;
            if serde_json::to_vec(&data).map_err(|e| e.to_string())?.len() > 64 * 1024 {
                return Err("event exceeds limit".into());
            }
            runtime.events.emit(
                "app_event",
                json!({"app_id":view.app_id,"binding_id":view.binding_id,"chat_id":view.chat_id,"view_id":view_id,"channel":channel,"data":data}),
            );
            return Ok(json!({"accepted":true}));
        }
        Command::ActionRequest {
            view_id,
            view_token,
            action_id,
            payload,
            request_id,
        } => {
            let view = checked_view(app, &runtime, view_id, Some(&view_token)).await?;
            return super::actions::execute(
                app, &runtime, view_id, &view, &action_id, payload, request_id,
            )
            .await;
        }
        Command::SourceControl {
            app_id: _,
            source_id: _,
            operation: _,
            confirmed: _,
        } => {
            return Err(
                "source control requires server-recorded approval; unsupported by this caller"
                    .into(),
            )
        }
    };
    runtime.events.emit("catalog_changed", json!({}));
    Ok(output)
}
fn validate_capture(data_url: &str) -> Result<(), String> {
    use base64::Engine;
    if data_url.len() > 8 * 1024 * 1024 {
        return Err("capture exceeds byte limit".into());
    }
    let data = data_url
        .strip_prefix("data:image/png;base64,")
        .ok_or("capture must be PNG data URL")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| "invalid capture encoding")?;
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err("invalid PNG capture signature".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 16_777_216
    {
        return Err("capture pixel limit exceeded".into());
    }
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|_| "capture PNG decoding failed")?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_rejects_active_and_unbounded_payloads() {
        assert!(validate_capture("data:text/html;base64,SGk=").is_err());
        assert!(validate_capture("data:image/png;base64,SGk=").is_err());
    }
    #[test]
    fn unknown_command_fields_rejected() {
        assert!(
            serde_json::from_value::<Command>(json!({"action":"list","root":"/other"})).is_err()
        );
    }
}

async fn require_bound_app(app: &AppState, caller: &Caller, app_id: Uuid) -> Result<(), String> {
    let Caller::Tool {
        chat_id, ..
    } = caller
    else {
        return Ok(());
    };
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("caller unavailable")?;
    let session = session.lock().await;
    let workspace = session
        .thread
        .app_workspace
        .as_ref()
        .and_then(|w| w.supported())
        .ok_or("app not bound to caller chat")?;
    workspace.validate()?;
    if !workspace.bindings.iter().any(|b| b.app_id == app_id) {
        return Err("app not bound to caller chat".into());
    }
    Ok(())
}
async fn scoped_events(
    app: &AppState,
    runtime: &super::AppRuntime,
    caller: &Caller,
) -> Result<Vec<super::EventEnvelope>, String> {
    let Caller::Tool {
        chat_id, ..
    } = caller
    else {
        return Ok(runtime.events.snapshot());
    };
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("caller unavailable")?;
    let workspace = session
        .lock()
        .await
        .thread
        .app_workspace
        .as_ref()
        .and_then(|w| w.supported())
        .cloned()
        .ok_or("caller has no supported app workspace")?;
    workspace.validate()?;
    let mut out = vec![];
    for e in runtime.events.snapshot() {
        let app_id = e
            .data
            .get("app_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());
        let binding_id = e
            .data
            .get("binding_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());
        if let (Some(app_id), Some(binding_id)) = (app_id, binding_id) {
            if workspace
                .bindings
                .iter()
                .any(|b| b.app_id == app_id && b.binding_id == binding_id)
            {
                out.push(e);
            }
        }
    }
    Ok(out)
}
