//! Declarative view actions. No adapter here grants host capabilities.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use refact_apps_core::{ActionOperation, StateScope};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use serde_json::{json, Value};
use uuid::Uuid;

const MAX_RECEIPT_BYTES: usize = 16 * 1024 * 1024;
const RECEIPT_TTL: Duration = Duration::from_secs(600);
const MAX_RECEIPTS_PER_VIEW: usize = 128;
const MAX_RECEIPTS: usize = 16_384;

fn canonicalize_payload(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for value in object.values_mut() {
                canonicalize_payload(value);
            }
            object.sort_keys();
        }
        Value::Array(array) => {
            for value in array {
                canonicalize_payload(value);
            }
        }
        Value::Number(number) if number.is_f64() && number.as_f64() == Some(0.0) => {
            *number = serde_json::Number::from_f64(0.0).unwrap();
        }
        _ => {}
    }
}

struct Receipt {
    action_id: String,
    payload_digest: [u8; 32],
    result: Result<Value, String>,
    expires: Instant,
    completed: bool,
    reserved_bytes: usize,
}

impl Receipt {
    fn finish(&mut self, result: &Result<Value, String>, available: usize) {
        let bytes = serde_json::to_vec(result)
            .map(|bytes| bytes.len())
            .unwrap_or(usize::MAX);
        if bytes <= available {
            self.result = result.clone();
            self.reserved_bytes += bytes;
        } else {
            self.result = Err(
                "action completed; result was not retained; request will not be replayed".into(),
            );
        }
    }
}

/// Runtime-owned and serialized. Exactly-once retry receipts are retained for ten
/// minutes, bounded at 128; saturation rejects before any side effect. Pending
/// receipts fail closed if the caller is cancelled during an operation.
#[derive(Default)]
pub struct ActionRuntime {
    receipts: HashMap<(Uuid, Uuid), Receipt>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatePayload {
    expected_revision: u64,
    value: Value,
}

/// `view` must first pass checked_view; activation is revalidated inside the lock.
pub async fn execute(
    app: &crate::app_state::AppState,
    runtime: &super::AppRuntime,
    view_id: Uuid,
    view: &super::views::View,
    action_id: &str,
    payload: Value,
    request_id: Uuid,
) -> Result<Value, String> {
    refact_apps_core::validate_identifier(action_id)?;
    if request_id.is_nil() {
        return Err("action request_id must not be nil".into());
    }
    let mut canonical_payload = payload.clone();
    canonicalize_payload(&mut canonical_payload);
    let payload_digest: [u8; 32] =
        Sha256::digest(serde_json::to_vec(&canonical_payload).map_err(|e| e.to_string())?).into();
    drop(canonical_payload);
    let activation = runtime.activation.lock().await;
    let current = runtime.views.lock().await.lookup(view_id)?;
    if current.generation != view.generation || current.app_id != view.app_id {
        return Err("view generation changed".into());
    }
    if current.stale_at(Instant::now()) {
        return Err("view is not live; reopen the app tab".into());
    }
    let view = &current;
    super::validate_binding(
        app,
        runtime,
        view.app_id,
        view.chat_id.as_deref(),
        view.binding_id,
    )
    .await?;
    let record = runtime.store.get_by_id(view.app_id).await?;
    let bundle = runtime.store.published(&record.slug).await?;
    if bundle.revision != view.revision {
        return Err("view published revision changed".into());
    }
    let action = bundle
        .manifest
        .actions
        .iter()
        .find(|a| a.action_id == action_id)
        .ok_or("undeclared app action")?;
    // Event/state operations need no host capability. Refresh reuses source grants,
    // never the action's permission declarations. No nonempty permission set is
    // currently supported (network/process/internal/chat_read/chat_write all deny).
    if !action.permissions.is_empty() {
        return Err(
            "unsupported action permissions: only an empty permission set is supported".into(),
        );
    }
    let live_generations: std::collections::HashSet<_> = runtime
        .views
        .lock()
        .await
        .active()
        .into_iter()
        .map(|(_, view)| view.generation)
        .collect();
    let mut actions = runtime.actions.lock().await;
    let now = Instant::now();
    // Unknown/pending outcomes never expire into a replayable request while its
    // view is alive. Removed generations cannot pass admission again.
    actions.receipts.retain(|(generation, _), receipt| {
        live_generations.contains(generation) && (!receipt.completed || receipt.expires > now)
    });
    let key = (view.generation, request_id);
    if let Some(receipt) = actions.receipts.get(&key) {
        if receipt.action_id != action_id || receipt.payload_digest != payload_digest {
            return Err("action request_id payload conflict".into());
        }
        return receipt.result.clone();
    }
    if actions
        .receipts
        .keys()
        .filter(|(generation, _)| *generation == view.generation)
        .count()
        >= MAX_RECEIPTS_PER_VIEW
    {
        return Err("view action receipt capacity reached; completed receipts expire after ten minutes; unknown outcomes require a new view".into());
    }
    if actions.receipts.len() >= MAX_RECEIPTS {
        return Err("runtime action receipt capacity reached".into());
    }
    let reserved_bytes = 32 + action_id.len() + 128;
    if actions
        .receipts
        .values()
        .map(|receipt| receipt.reserved_bytes)
        .sum::<usize>()
        + reserved_bytes
        > MAX_RECEIPT_BYTES
    {
        return Err("runtime action receipt byte capacity reached".into());
    }
    actions.receipts.insert(
        key,
        Receipt {
            action_id: action_id.to_owned(),
            payload_digest,
            result: Err("action interrupted; outcome unknown, request will not be replayed".into()),
            expires: now + RECEIPT_TTL,
            completed: false,
            reserved_bytes,
        },
    );
    drop(actions);
    // Admission/wakeup is fenced, but network IO must never pin activation.
    if let ActionOperation::SourceRefresh {
        source_id,
    } = &action.operation
    {
        let admitted = if !payload.is_null() && payload != json!({}) {
            Err("source refresh payload must be null or an empty object".into())
        } else {
            runtime
                .automation
                .begin_refresh(&view.app_id.to_string(), source_id, Some(&view.revision))
                .await
        };
        drop(activation);
        let result = match admitted {
            Ok(wait) => wait
                .wait()
                .await
                .and_then(|event| serde_json::to_value(event).map_err(|e| e.to_string())),
            Err(error) => Err(error),
        };
        let mut actions = runtime.actions.lock().await;
        let available = MAX_RECEIPT_BYTES
            .saturating_sub(actions.receipts.values().map(|r| r.reserved_bytes).sum());
        if let Some(receipt) = actions.receipts.get_mut(&key) {
            receipt.finish(&result, available);
            receipt.completed = true;
            receipt.expires = Instant::now() + RECEIPT_TTL;
        }
        return result;
    }
    let result = match &action.operation {
        ActionOperation::EventEmit => {
            runtime.events.emit(
                "app_event",
                json!({
                    "app_id":view.app_id,"binding_id":view.binding_id,"chat_id":view.chat_id,
                    "generation":view.generation,"action_id":action_id,"request_id":request_id,
                    "channel":action.channel,"data":payload
                }),
            );
            Ok(json!({"accepted":true,"request_id":request_id}))
        }
        ActionOperation::StateSet => {
            async {
                if !payload.as_object().is_some_and(|object| {
                    object.len() == 2
                        && object.contains_key("expected_revision")
                        && object.contains_key("value")
                }) {
                    return Err(
                        "state action payload must be exactly {expected_revision,value}".into(),
                    );
                }
                let state_payload: StatePayload =
                    serde_json::from_value(payload).map_err(|_| {
                        "state action payload must be exactly {expected_revision,value}"
                    })?;
                let scope = view
                    .binding_id
                    .map(|binding_id| StateScope::Binding {
                        binding_id,
                    })
                    .unwrap_or(StateScope::Standalone);
                let state = runtime
                    .store
                    .write_state(
                        &record.slug,
                        scope.clone(),
                        state_payload.expected_revision,
                        state_payload.value,
                    )
                    .await?;
                runtime.events.emit(
                    "state_changed",
                    json!({"app_id":view.app_id,
                    "scope":scope,"binding_id":view.binding_id,"chat_id":view.chat_id,
                    "revision":state.revision,"request_id":request_id}),
                );
                serde_json::to_value(state).map_err(|e| e.to_string())
            }
            .await
        }
        ActionOperation::SourceRefresh {
            ..
        } => unreachable!("refresh handled at admission"),
    };
    let mut actions = runtime.actions.lock().await;
    let available =
        MAX_RECEIPT_BYTES.saturating_sub(actions.receipts.values().map(|r| r.reserved_bytes).sum());
    if let Some(receipt) = actions.receipts.get_mut(&key) {
        receipt.finish(&result, available);
        receipt.completed = true;
        receipt.expires = Instant::now() + RECEIPT_TTL;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn declared_actions_emit_once_and_write_only_view_state_with_cas() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let runtime = super::super::runtime(&app, &super::super::Caller::Host)
            .await
            .unwrap();
        let created = runtime.store.create("actions", "Actions").await.unwrap();
        let mut manifest = created.manifest.clone();
        manifest.actions = serde_json::from_value(json!([
            {"action_id":"notify","channel":"notices"},
            {"action_id":"save","channel":"state","operation":{"type":"state_set"}},
            {"action_id":"privileged","channel":"nope","permissions":[{"type":"process"}]}
        ]))
        .unwrap();
        let record = runtime
            .store
            .write_file(
                "actions",
                "app.json",
                serde_json::to_vec(&manifest).unwrap(),
                &created.revision,
            )
            .await
            .unwrap();
        let bundle = runtime
            .store
            .publish("actions", &record.revision)
            .await
            .unwrap();
        let registration = runtime
            .views
            .lock()
            .await
            .register(manifest.app_id, None, None, bundle.revision)
            .unwrap();
        let id = serde_json::from_value(registration["view_id"].clone()).unwrap();
        let view = runtime
            .views
            .lock()
            .await
            .check(id, registration["view_token"].as_str())
            .unwrap();
        let request = Uuid::new_v4();
        let payload: Value =
            serde_json::from_str(r#"{"a":{"x":1,"y":2},"b":[{"c":3,"d":4}],"z":-0.0}"#).unwrap();
        let reordered: Value =
            serde_json::from_str(r#"{"z":0.0,"b":[{"d":4,"c":3}],"a":{"y":2,"x":1}}"#).unwrap();
        assert_eq!(payload, reordered);
        let receipt = execute(&app, &runtime, id, &view, "notify", payload, request)
            .await
            .unwrap();
        assert_eq!(
            execute(&app, &runtime, id, &view, "notify", reordered, request)
                .await
                .unwrap(),
            receipt
        );
        assert!(execute(
            &app,
            &runtime,
            id,
            &view,
            "notify",
            json!({"a":{"x":1,"y":2},"b":[{"c":3,"d":5}],"z":0.0}),
            request
        )
        .await
        .unwrap_err()
        .contains("conflict"));
        let events = runtime.events.snapshot();
        let emitted: Vec<_> = events.iter().filter(|e| e.kind == "app_event").collect();
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].data["generation"], json!(view.generation));
        assert_eq!(emitted[0].data["channel"], "notices");
        assert!(execute(
            &app,
            &runtime,
            id,
            &view,
            "privileged",
            Value::Null,
            Uuid::new_v4()
        )
        .await
        .unwrap_err()
        .contains("permissions"));
        assert!(execute(
            &app,
            &runtime,
            id,
            &view,
            "save",
            json!({"expected_revision":0}),
            Uuid::new_v4()
        )
        .await
        .is_err());
        assert!(execute(
            &app,
            &runtime,
            id,
            &view,
            "save",
            json!({"expected_revision":0,"value":1,"scope":{"type":"shared"}}),
            Uuid::new_v4()
        )
        .await
        .is_err());
        let large_request = Uuid::new_v4();
        let large_payload =
            json!({"expected_revision":0,"value":"x".repeat(MAX_RECEIPT_BYTES + 1)});
        let large_result = execute(
            &app,
            &runtime,
            id,
            &view,
            "save",
            large_payload.clone(),
            large_request,
        )
        .await
        .unwrap();
        assert_eq!(large_result["value"], large_payload["value"]);
        let retry = execute(
            &app,
            &runtime,
            id,
            &view,
            "save",
            large_payload,
            large_request,
        )
        .await
        .unwrap_err();
        assert!(retry.contains("not retained"));
        assert!(
            runtime
                .actions
                .lock()
                .await
                .receipts
                .values()
                .map(|r| r.reserved_bytes)
                .sum::<usize>()
                <= MAX_RECEIPT_BYTES
        );
        assert_eq!(
            runtime
                .store
                .read_state("actions", StateScope::Standalone)
                .await
                .unwrap()
                .revision,
            1
        );
        let request = Uuid::new_v4();
        let payload = json!({"expected_revision":1,"value":{"saved":true}});
        let state = execute(&app, &runtime, id, &view, "save", payload.clone(), request)
            .await
            .unwrap();
        assert_eq!(state["revision"], 2);
        assert_eq!(
            execute(&app, &runtime, id, &view, "save", payload.clone(), request)
                .await
                .unwrap(),
            state
        );
        assert!(
            execute(&app, &runtime, id, &view, "save", payload, Uuid::new_v4())
                .await
                .unwrap_err()
                .contains("conflict")
        );
        assert_eq!(
            runtime
                .store
                .read_state("actions", StateScope::Shared)
                .await
                .unwrap()
                .revision,
            0
        );
        assert_eq!(
            runtime
                .store
                .read_state("actions", StateScope::Standalone)
                .await
                .unwrap()
                .value,
            json!({"saved":true})
        );
        let mut stale = view.clone();
        stale.generation = Uuid::new_v4();
        assert!(execute(
            &app,
            &runtime,
            id,
            &stale,
            "notify",
            Value::Null,
            Uuid::new_v4()
        )
        .await
        .unwrap_err()
        .contains("generation"));
        runtime.views.lock().await.remove(id);
        assert!(execute(
            &app,
            &runtime,
            id,
            &view,
            "notify",
            Value::Null,
            Uuid::new_v4()
        )
        .await
        .unwrap_err()
        .contains("unavailable"));
        assert_eq!(
            runtime
                .events
                .snapshot()
                .iter()
                .filter(|event| event.kind == "app_event")
                .count(),
            1
        );
        app.gcx.apps.shutdown().await;
    }
}
