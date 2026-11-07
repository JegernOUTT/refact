use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::sync_authority::{self, SyncCaller, SyncSelector};
use crate::worktrees::types::{WorktreeSyncRequest, WorktreeSyncResponse};

pub struct ToolWorktreeSync;

pub(crate) async fn sync_under_lease(
    app: &crate::app_state::AppState,
    service: &WorktreeService,
    id: &str,
    request: WorktreeSyncRequest,
    actor: String,
    authority: Option<refact_worktrees::locking::PhaseValidator>,
) -> Result<WorktreeSyncResponse, String> {
    let registry = service.load_registry().await?;
    let view = registry
        .records
        .into_iter()
        .find(|r| r.meta.id == id)
        .ok_or("Source worktree not found")?;
    let mut _transitions = Vec::new();
    if !matches!(request, WorktreeSyncRequest::Status { .. }) {
        let sessions: Vec<_> = app.chat.sessions.read().await.values().cloned().collect();
        for session in sessions {
            let attached = session
                .lock()
                .await
                .thread
                .worktree
                .as_ref()
                .is_some_and(|w| w == &view.meta);
            if attached {
                _transitions.push(crate::chat::worktree_lifecycle::try_transition_lock(
                    &session,
                )?);
            }
        }
    }
    let mut service = service
        .acquire_merge_permit()
        .await?
        .with_sync_actor(actor.clone());
    if !matches!(request, WorktreeSyncRequest::Status { .. }) {
        let app = app.clone();
        let meta = view.meta.clone();
        let registry_service = service.clone();
        let runtime = tokio::runtime::Handle::current();
        service = service.with_merge_authority(Arc::new(move || {
            runtime.block_on(async {
                let registry = registry_service.load_registry().await?;
                let record = registry
                    .records
                    .iter()
                    .find(|r| r.meta == meta)
                    .ok_or("Source registration changed")?;
                sync_authority::ensure_source_quiescent(
                    &app,
                    &meta,
                    &record.references,
                    Some(&actor),
                )
                .await
            })
        }));
    }
    if let Some(authority) = authority {
        service = service.with_merge_authority(authority);
    }
    let Some(leases) =
        crate::coordination::leases::merge_leases_for(app, service.source_workspace_root()).await?
    else {
        return service.sync_worktree(id, request).await;
    };
    let gate = crate::coordination::merge_gate::MergeGate::new(
        leases.clone(),
        &crate::file_activity::settings(),
    );
    let slot = gate
        .acquire(&crate::coordination::merge_gate::checkout_key(
            &view.meta.root,
        ))
        .await
        .map_err(|e| e.to_string())?;
    let service = service.with_merge_authority(leases.phase_validator(slot.holder().clone()));
    let mut renewal =
        crate::coordination::leases::LeaseRenewal::spawn(leases, slot.holder().clone());
    let result = service.sync_worktree(id, request).await;
    if let Err(error) = renewal.shutdown().await {
        tracing::warn!(%error, "sync lease release failed");
    }
    result
}

struct SyncArgs {
    worktree_id: Option<String>,
    source_workspace_root: Option<String>,
    selector: SyncSelector,
    request: WorktreeSyncRequest,
}

fn parse_args(args: &HashMap<String, Value>) -> Result<SyncArgs, String> {
    let mut object: serde_json::Map<String, Value> = args.clone().into_iter().collect();
    fn take_string(
        object: &mut serde_json::Map<String, Value>,
        key: &str,
    ) -> Result<Option<String>, String> {
        match object.remove(key) {
            None => Ok(None),
            Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s)),
            _ => Err(format!("{key} must be a nonempty string")),
        }
    }
    let worktree_id = take_string(&mut object, "worktree_id")?;
    let source_workspace_root = take_string(&mut object, "source_workspace_root")?;
    let card = take_string(&mut object, "card_id")?;
    let child = take_string(&mut object, "child_chat_id")?;
    let selector = match (card, child) {
        (None, None) => SyncSelector::Own,
        (Some(card), None) => SyncSelector::Card(card, worktree_id.clone()),
        (None, Some(child)) => SyncSelector::Child(child),
        _ => return Err("Pass at most one of card_id or child_chat_id".into()),
    };
    let request = serde_json::from_value(Value::Object(object))
        .map_err(|e| format!("invalid sync request: {e}"))?;
    Ok(SyncArgs {
        worktree_id,
        source_workspace_root,
        selector,
        request,
    })
}

fn instructions(response: &WorktreeSyncResponse) -> &'static str {
    use crate::worktrees::types::WorktreeSyncStatus as S;
    match response.status {
        S::Idle => "Call start with expected_source_head and expected_target_head copied from this status.",
        S::Conflicts => "Edit or choose each conflicted path, then call resolve with the operation_id and current revision, then continue; or abort.",
        S::TargetAdvanced => "Target moved; continue the pinned operation or abort, then refresh status and start again before landing.",
        S::RecoveryRequired => "Inspect warnings; abort (force only if normal abort is refused) or ask the user to recover.",
        _ => "Use status to inspect; pass the current operation_id and revision for any further action.",
    }
}

#[async_trait]
impl Tool for ToolWorktreeSync {
    fn tool_description(&self) -> ToolDesc {
        let props = json!({
            "worktree_id":{"type":"string","minLength":1},
            "source_workspace_root":{"type":"string","minLength":1},
            "card_id":{"type":"string","minLength":1,"description":"Planner only: completed card in your owned board whose stopped agent checkout is the source."},
            "child_chat_id":{"type":"string","minLength":1,"description":"Root chat only: your direct stopped subagent child whose isolated checkout is the source."},
            "action":{"enum":["status","recover","start","resolve","continue","abort","restore"]},
            "expected_journal_digest":{"type":"string","minLength":1,"description":"recovery_journal_digest copied from Status; quarantines unreadable or orphaned sync metadata without Git mutation."},
            "target_branch":{"type":"string"}, "expected_source_head":{"type":"string"},
            "expected_target_head":{"type":"string"}, "checkpoint":{"type":"boolean","description":"Explicitly consent to checkpointing dirty source work."},
            "archive_id":{"type":"string","minLength":1,"description":"Stable ID from recovery_archives; omission selects primary, never a path."},
            "operation_id":{"type":"string"}, "expected_revision":{"type":"integer","minimum":0},
            "resolutions":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["path","choice"],"properties":{"path":{"type":"string"},"choice":{"enum":["working_tree","source","target","delete"]}}}},
            "commit_message":{"type":"string"}, "force":{"type":"boolean","description":"Abort with preservation backup if normal abort cannot safely restore."}
        });
        let variants = [
            ("status", vec![]),
            ("recover", vec!["expected_journal_digest"]),
            (
                "start",
                vec!["expected_source_head", "expected_target_head"],
            ),
            (
                "resolve",
                vec!["operation_id", "expected_revision", "resolutions"],
            ),
            ("continue", vec!["operation_id", "expected_revision"]),
            ("abort", vec!["operation_id", "expected_revision"]),
            ("restore", vec!["operation_id", "expected_revision"]),
        ];
        let common = [
            "worktree_id",
            "source_workspace_root",
            "card_id",
            "child_chat_id",
        ];
        let one_of: Vec<Value> = variants.into_iter().map(|(action, required)| {
            let mut names = common.to_vec();
            names.extend(match action {
                "start" => vec!["target_branch", "expected_source_head", "expected_target_head", "checkpoint"],
                "resolve" => vec!["operation_id", "expected_revision", "resolutions"],
                "continue" => vec!["operation_id", "expected_revision", "commit_message"],
                "abort" => vec!["operation_id", "expected_revision", "force"],
                "restore" => vec!["operation_id", "expected_revision", "archive_id"],
                "recover" => vec!["expected_journal_digest"],
                "status" => vec!["target_branch"],
                _ => vec![],
            });
            let mut properties = serde_json::Map::new();
            properties.insert("action".into(), json!({"const":action}));
            for name in names { properties.insert(name.into(), props[name].clone()); }
            let mut required_fields = vec!["action"];
            required_fields.extend(required);
            json!({"type":"object", "additionalProperties":false, "required":required_fields, "properties":properties})
        }).collect();
        ToolDesc { name:"worktree_sync".into(), display_name:"Sync Worktree Source".into(), source:ToolSource { source_type:ToolSourceType::Builtin, config_path:String::new() }, experimental:false, allow_parallel:false,
            description:"Synchronize the authorized source worktree with its target branch without landing it. Status returns pinned heads; start requires both heads. Resolve, continue, abort and restore require the current operation ID and revision. Restore is for a preserved force-abort backup and requires unchanged post-abort state.".into(),
            input_schema:json!({"oneOf":one_of}), output_schema:None, annotations:None }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<Mutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let parsed = parse_args(args)?;
        let recovery = matches!(
            parsed.request,
            WorktreeSyncRequest::Status { .. }
                | WorktreeSyncRequest::Abort { .. }
                | WorktreeSyncRequest::Recover { .. }
        );
        let caller = SyncCaller::from_context(&ccx).await;
        let mut authority = sync_authority::resolve(&caller, &parsed.selector, recovery).await?;
        let selected = authority.worktree.clone();
        if parsed
            .worktree_id
            .as_ref()
            .is_some_and(|id| id != &selected.id)
        {
            return Err("Requested worktree is not the authorized source; worktree_id only narrows the expectation".into());
        }
        if parsed
            .source_workspace_root
            .as_ref()
            .is_some_and(|root| std::path::Path::new(root) != selected.source_workspace_root)
        {
            return Err("Requested source root does not match authorized source".into());
        }
        if let WorktreeSyncRequest::Resolve {
            resolutions, ..
        } = &parsed.request
        {
            authority
                .clear_source_paths(&caller, resolutions.iter().map(|r| r.path.as_str()))
                .await?;
        }
        let service = WorktreeService::new_async(
            caller.app().gcx.cache_dir.clone(),
            selected.source_workspace_root.clone(),
        )
        .await?;
        let again = sync_authority::resolve(&caller, &parsed.selector, recovery).await?;
        if !authority.same(&again) {
            return Err("Source synchronization authority changed; request Status again".into());
        }
        let validator =
            authority.phase_validator(caller.clone(), parsed.selector.clone(), recovery);
        let response = sync_under_lease(
            caller.app(),
            &service,
            &selected.id,
            parsed.request,
            authority.actor.clone(),
            Some(validator),
        )
        .await?;
        authority
            .clear_source_paths(&caller, response.conflicts.iter().map(|c| c.path.as_str()))
            .await?;
        let content = serde_json::to_string(&json!({
            "sync": response,
            "authority": authority.provenance,
            "next": instructions(&response),
        }))
        .map_err(|e| e.to_string())?;
        let mut message = ChatMessage {
            role: "tool".into(),
            content: ChatContent::SimpleText(content),
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        };
        crate::privacy::records::merge_records(&mut message, authority.records.clone());
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_is_strict_for_each_action() {
        let args = |value: Value| value.as_object().unwrap().clone().into_iter().collect();
        assert!(parse_args(&args(json!({"action":"status"}))).is_ok());
        assert!(parse_args(&args(json!({"action":"status","force":true}))).is_err());
        assert!(parse_args(&args(json!({"action":"start","expected_source_head":"a","expected_target_head":"b","checkpoint":true}))).is_ok());
        assert!(parse_args(&args(
            json!({"action":"abort","operation_id":"a","expected_revision":1,"checkpoint":true})
        ))
        .is_err());
        assert!(parse_args(&args(
            json!({"action":"status","card_id":"a","child_chat_id":"b"})
        ))
        .is_err());
        assert!(matches!(
            parse_args(&args(json!({"action":"status","child_chat_id":"b"})))
                .unwrap()
                .selector,
            SyncSelector::Child(_)
        ));
    }

    use crate::chat::types::ChatSession;
    use refact_runtime_api::SessionState;

    struct Fixture {
        _temp: tempfile::TempDir,
        source: std::path::PathBuf,
        worktree: crate::worktrees::types::WorktreeMeta,
        session: Arc<Mutex<ChatSession>>,
        ccx: Arc<Mutex<AtCommandsContext>>,
    }

    async fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        let source = dunce::canonicalize(&source).unwrap();
        let git = |root: &std::path::Path, args: &[&str]| {
            crate::worktrees::git::run_git(root, args).unwrap()
        };
        git(&source, &["init"]);
        git(&source, &["checkout", "-b", "main"]);
        git(&source, &["config", "user.email", "test@example.invalid"]);
        git(&source, &["config", "user.name", "Test"]);
        std::fs::write(source.join("file"), "base\n").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-m", "base"]);
        let cache = temp.path().join("cache");
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            temp.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "test/model".into(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: "test/model".into(),
                    n_ctx: 200_000,
                    ..Default::default()
                },
                supports_tools: true,
                supports_agent: true,
                ..Default::default()
            }),
        );
        let (project_root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
        {
            let mut state = gcx.caps_state.write().await;
            state.caps = Some(Arc::new(caps));
            state.loaded_project_root = Some(project_root);
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let worktree = service
            .create_chat_worktree("sync-root")
            .await
            .unwrap()
            .worktree
            .meta;
        let mut session = ChatSession::new("sync-root".into());
        session.thread.model = "test/model".into();
        session.work_project_root = Some(source.clone());
        session.thread.worktree = Some(worktree.clone());
        session.set_runtime_state(SessionState::ExecutingTools, None);
        let abort = session.abort_flag.clone();
        let session = Arc::new(Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert("sync-root".into(), session.clone());
        let ccx = AtCommandsContext::new_with_abort(
            app,
            4096,
            10,
            false,
            vec![],
            "sync-root".into(),
            None,
            "test/model".into(),
            None,
            Some(worktree.clone()),
            Some(abort),
        )
        .await;
        Fixture {
            _temp: temp,
            source,
            worktree,
            session,
            ccx: Arc::new(Mutex::new(ccx)),
        }
    }

    async fn call(fixture: &Fixture, args: Value) -> Result<Value, String> {
        let args: HashMap<String, Value> = args.as_object().unwrap().clone().into_iter().collect();
        let (_, out) = ToolWorktreeSync
            .tool_execute(fixture.ccx.clone(), &"call".to_string(), &args)
            .await?;
        let ContextEnum::ChatMessage(message) = &out[0] else {
            panic!("tool message expected")
        };
        Ok(serde_json::from_str(&message.content.content_text_only()).unwrap())
    }

    async fn http(f: &Fixture, request: Value) -> Result<Value, String> {
        use tower::ServiceExt;
        let app = f.ccx.lock().await.app.clone();
        let router = crate::http::routers::v1::make_v1_router(app.clone()).with_state(app);
        let response = router
            .oneshot(
                hyper::Request::builder()
                    .method("POST")
                    .uri(format!("/worktrees/{}/sync", f.worktree.id))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        if status.is_success() {
            Ok(value)
        } else {
            Err(value.to_string())
        }
    }

    #[tokio::test]
    async fn http_start_refuses_busy_writer_and_model_continues_user_conflict() {
        let f = fixture().await;
        conflict(&f);
        let status = http(&f, json!({"action":"status"})).await.unwrap();
        let request = json!({"action":"start","expected_source_head":status["source_head"],"expected_target_head":status["target_head"]});
        let before =
            crate::worktrees::git::run_git(&f.worktree.root, &["status", "--porcelain=v1"])
                .unwrap();
        let error = http(&f, request.clone()).await.unwrap_err();
        assert!(error.contains("writer"), "{error}");
        assert_eq!(
            before,
            crate::worktrees::git::run_git(&f.worktree.root, &["status", "--porcelain=v1"])
                .unwrap()
        );
        assert!(http(&f, json!({"action":"status"})).await.unwrap()["operation"].is_null());
        f.session
            .lock()
            .await
            .set_runtime_state(SessionState::Idle, None);
        let started = http(&f, request).await.unwrap();
        assert_eq!(started["status"], "conflicts", "{started}");
        f.session
            .lock()
            .await
            .set_runtime_state(SessionState::ExecutingTools, None);
        let op = &started["operation"];
        let resolved = call(&f, json!({"action":"resolve","operation_id":op["operation_id"],"expected_revision":op["revision"],"resolutions":[{"path":"file","choice":"source"}]})).await.unwrap();
        let op = &resolved["sync"]["operation"];
        let done = call(&f, json!({"action":"continue","operation_id":op["operation_id"],"expected_revision":op["revision"]})).await.unwrap();
        assert_eq!(done["sync"]["status"], "completed", "{done}");
    }

    #[tokio::test]
    async fn http_foreign_attached_writer_refuses_and_missing_root_recovers() {
        let f = fixture().await;
        conflict(&f);
        f.session
            .lock()
            .await
            .set_runtime_state(SessionState::Idle, None);
        let app = f.ccx.lock().await.app.clone();
        let mut foreign = ChatSession::new("foreign".into());
        foreign.thread.worktree = Some(f.worktree.clone());
        foreign.set_runtime_state(SessionState::Generating, None);
        app.chat
            .sessions
            .write()
            .await
            .insert("foreign".into(), Arc::new(Mutex::new(foreign)));
        let status = http(&f, json!({"action":"status"})).await.unwrap();
        let request = json!({"action":"start","expected_source_head":status["source_head"],"expected_target_head":status["target_head"]});
        assert!(http(&f, request.clone())
            .await
            .unwrap_err()
            .contains("writer"));
        app.chat.sessions.write().await.remove("foreign");
        let started = http(&f, request).await.unwrap();
        assert_eq!(started["status"], "conflicts");
        let held = f.worktree.root.with_extension("held");
        std::fs::rename(&f.worktree.root, &held).unwrap();
        let status = http(&f, json!({"action":"status"})).await.unwrap();
        assert_eq!(status["status"], "recovery_required", "{status}");
        let recovered = http(
            &f,
            json!({"action":"recover","expected_journal_digest":status["recovery_journal_digest"]}),
        )
        .await
        .unwrap();
        assert_eq!(recovered["status"], "aborted", "{recovered}");
        assert!(held.join("file").exists());
    }

    fn conflict(fixture: &Fixture) {
        let git = |root: &std::path::Path, args: &[&str]| {
            crate::worktrees::git::run_git(root, args).unwrap()
        };
        std::fs::write(fixture.worktree.root.join("file"), "source\n").unwrap();
        git(&fixture.worktree.root, &["commit", "-am", "source"]);
        std::fs::write(fixture.source.join("file"), "target\n").unwrap();
        git(&fixture.source, &["commit", "-am", "target"]);
    }

    #[tokio::test]
    async fn own_scope_synchronizes_through_conflicts() {
        let f = fixture().await;
        conflict(&f);
        let status = call(&f, json!({"action":"status"})).await.unwrap();
        assert_eq!(status["authority"]["selector"]["kind"], "own");
        assert_eq!(status["authority"]["actor"], "chat:sync-root");
        let started = call(
            &f,
            json!({"action":"start",
            "expected_source_head":status["sync"]["source_head"],
            "expected_target_head":status["sync"]["target_head"]}),
        )
        .await
        .unwrap();
        assert_eq!(started["sync"]["status"], "conflicts", "{started}");
        assert_eq!(started["sync"]["conflicts"][0]["path"], "file");
        let op = started["sync"]["operation"].clone();
        std::fs::write(f.worktree.root.join("file"), "merged\n").unwrap();
        let resolved = call(
            &f,
            json!({"action":"resolve","operation_id":op["operation_id"],
            "expected_revision":op["revision"],
            "resolutions":[{"path":"file","choice":"working_tree"}]}),
        )
        .await
        .unwrap();
        let op = resolved["sync"]["operation"].clone();
        let done = call(
            &f,
            json!({"action":"continue","operation_id":op["operation_id"],
            "expected_revision":op["revision"]}),
        )
        .await
        .unwrap();
        assert_eq!(done["sync"]["status"], "completed", "{done}");
        assert_eq!(
            std::fs::read_to_string(f.worktree.root.join("file")).unwrap(),
            "merged\n"
        );
    }

    #[tokio::test]
    async fn planner_reassigned_card_recovers_original_registered_source() {
        use crate::chat::types::TaskMeta;
        let mut f = fixture().await;
        let app = f.ccx.lock().await.app.clone();
        let service = WorktreeService::new(app.gcx.cache_dir.clone(), f.source.clone()).unwrap();
        f.worktree = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                kind: Some("task_agent".into()),
                task_id: Some("task".into()),
                card_id: Some("T-1".into()),
                agent_id: Some("worker".into()),
                chat_id: Some("worker-chat".into()),
                branch: Some("refact/task/task/card/T-1/worker".into()),
                base_branch: Some("main".into()),
                ..Default::default()
            })
            .await
            .unwrap()
            .worktree
            .meta;
        let task = TaskMeta {
            task_id: "task".into(),
            role: "planner".into(),
            ..Default::default()
        };
        {
            let mut parent = f.session.lock().await;
            parent.thread.worktree = None;
            parent.thread.task_meta = Some(task.clone());
            parent.thread.mode = "task_planner".into();
        }
        let mut child = ChatSession::new("worker-chat".into());
        child.thread = crate::tools::tool_task_spawn_agent::build_agent_thread_params(
            "worker-chat",
            "Checkout",
            "test/model",
            "task",
            "worker",
            "T-1",
            "sync-root",
            f.worktree.clone(),
        );
        child.set_runtime_state(SessionState::Completed, None);
        app.chat
            .sessions
            .write()
            .await
            .insert("worker-chat".into(), Arc::new(Mutex::new(child)));
        let abort_flag = f.session.lock().await.abort_flag.clone();
        f.ccx = Arc::new(Mutex::new(
            AtCommandsContext::new_with_abort(
                app.clone(),
                4096,
                10,
                false,
                vec![],
                "sync-root".into(),
                None,
                "test/model".into(),
                Some(task),
                None,
                Some(abort_flag),
            )
            .await,
        ));
        let mut board: crate::tasks::types::TaskBoard = serde_json::from_value(json!({"rev":7,"owner_chat_id":"sync-root","cards":[{
            "id":"T-1","title":"Checkout","column":"done","created_at":"now","assignee":"worker",
            "agent_chat_id":"worker-chat","agent_worktree":f.worktree.root,"agent_worktree_name":f.worktree.id,"agent_branch":f.worktree.branch
        }]})).unwrap();
        let board_path = f.source.join(".refact/tasks/task/board.yaml");
        std::fs::create_dir_all(board_path.parent().unwrap()).unwrap();
        std::fs::write(&board_path, serde_yaml::to_string(&board).unwrap()).unwrap();
        conflict(&f);
        let status = call(&f, json!({"action":"status","card_id":"T-1"}))
            .await
            .unwrap();
        let started = call(&f, json!({"action":"start","card_id":"T-1","expected_source_head":status["sync"]["source_head"],"expected_target_head":status["sync"]["target_head"]})).await.unwrap();
        assert_eq!(started["sync"]["status"], "conflicts", "{started}");
        let card = &mut board.cards[0];
        card.assignee = Some("replacement".into());
        card.agent_chat_id = Some("replacement-chat".into());
        card.agent_worktree_name = None;
        card.agent_worktree = None;
        card.agent_branch = None;
        board.rev += 1;
        std::fs::write(&board_path, serde_yaml::to_string(&board).unwrap()).unwrap();
        let status = call(
            &f,
            json!({"action":"status","card_id":"T-1","worktree_id":f.worktree.id}),
        )
        .await
        .unwrap();
        let op = &status["sync"]["operation"];
        let aborted = call(&f, json!({"action":"abort","card_id":"T-1","worktree_id":f.worktree.id,"operation_id":op["operation_id"],"expected_revision":op["revision"]})).await.unwrap();
        assert_eq!(aborted["sync"]["status"], "aborted", "{aborted}");
        board.owner_chat_id = Some("other-owner".into());
        std::fs::write(&board_path, serde_yaml::to_string(&board).unwrap()).unwrap();
        assert!(call(
            &f,
            json!({"action":"status","card_id":"T-1","worktree_id":f.worktree.id})
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn stopped_child_sync_and_active_operation_restart_refusal() {
        use crate::agents::types::{BgAgentKind, CreateAgentRequest};
        let mut f = fixture().await;
        let app = f.ccx.lock().await.app.clone();
        f.worktree = WorktreeService::new(app.gcx.cache_dir.clone(), f.source.clone())
            .unwrap()
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                kind: Some("subagent".into()),
                chat_id: Some("sync-child".into()),
                branch: Some("refact/subagent/sync-child".into()),
                base_branch: Some("main".into()),
                ..Default::default()
            })
            .await
            .unwrap()
            .worktree
            .meta;
        let (agent, _, _) = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: "sync-root".into(),
                parent_root_chat_id: Some("sync-root".into()),
                parent_tool_call_id: None,
                kind: BgAgentKind::Subagent,
                config_name: "test".into(),
                title: "Child".into(),
                prompt: "work".into(),
                target_files: vec![],
                model: "test/model".into(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: Some(f.worktree.id.clone()),
                worktree_branch: f.worktree.branch.clone(),
            })
            .await
            .unwrap();
        app.agents
            .mark_running(&agent.agent_id, "sync-child".into())
            .await
            .unwrap();
        let mut child = ChatSession::new("sync-child".into());
        child.thread.parent_id = Some("sync-root".into());
        child.thread.root_chat_id = Some("sync-root".into());
        child.thread.worktree = Some(f.worktree.clone());
        child.set_runtime_state(SessionState::Generating, None);
        let child = Arc::new(Mutex::new(child));
        app.chat
            .sessions
            .write()
            .await
            .insert("sync-child".into(), child.clone());
        conflict(&f);
        assert!(
            call(&f, json!({"action":"status","child_chat_id":"sync-child"}))
                .await
                .unwrap_err()
                .contains("stopped")
        );
        app.agents
            .mark_cancelled(&agent.agent_id, None)
            .await
            .unwrap();
        child
            .lock()
            .await
            .set_runtime_state(SessionState::Completed, None);
        let status = call(&f, json!({"action":"status","child_chat_id":"sync-child"}))
            .await
            .unwrap();
        let started = call(&f, json!({"action":"start","child_chat_id":"sync-child","expected_source_head":status["sync"]["source_head"],"expected_target_head":status["sync"]["target_head"]})).await.unwrap();
        assert_eq!(started["sync"]["status"], "conflicts", "{started}");
        let record = app.agents.get_any(&agent.agent_id).await.unwrap();
        let error = crate::agents::spawn::resume_agent(
            app.clone(),
            "sync-root",
            &agent.agent_id,
            None,
            record.change_seq,
            None,
            false,
        )
        .await
        .unwrap_err();
        assert!(error.contains("active native synchronization"), "{error}");
        assert_eq!(
            app.agents
                .get_any(&agent.agent_id)
                .await
                .unwrap()
                .run_generation,
            record.run_generation
        );
        let op = &started["sync"]["operation"];
        let resolved = call(&f, json!({"action":"resolve","child_chat_id":"sync-child","operation_id":op["operation_id"],"expected_revision":op["revision"],"resolutions":[{"path":"file","choice":"source"}]})).await.unwrap();
        let op = &resolved["sync"]["operation"];
        let done = call(&f, json!({"action":"continue","child_chat_id":"sync-child","operation_id":op["operation_id"],"expected_revision":op["revision"]})).await.unwrap();
        assert_eq!(done["sync"]["status"], "completed", "{done}");
    }

    #[tokio::test]
    async fn foreign_or_unbound_sources_are_denied() {
        let f = fixture().await;
        let foreign = WorktreeService::new(
            f.worktree
                .source_workspace_root
                .parent()
                .unwrap()
                .join("cache"),
            f.source.clone(),
        )
        .unwrap()
        .create_chat_worktree("other-chat")
        .await
        .unwrap()
        .worktree
        .meta;
        let error = call(&f, json!({"action":"status","worktree_id":foreign.id}))
            .await
            .unwrap_err();
        assert!(error.contains("not the authorized source"), "{error}");
        let error = call(
            &f,
            json!({"action":"status","child_chat_id":"not-my-child"}),
        )
        .await
        .unwrap_err();
        assert!(error.contains("unauthorized"), "{error}");
        let error = call(&f, json!({"action":"status","card_id":"T-1"}))
            .await
            .unwrap_err();
        assert!(error.contains("unauthorized"), "{error}");
        f.session.lock().await.thread.worktree = Some(foreign);
        let error = call(&f, json!({"action":"status"})).await.unwrap_err();
        assert!(error.contains("unauthorized"), "{error}");
    }

    #[tokio::test]
    async fn revoked_caller_cannot_continue_mutation() {
        let f = fixture().await;
        conflict(&f);
        let status = call(&f, json!({"action":"status"})).await.unwrap();
        let started = call(
            &f,
            json!({"action":"start",
            "expected_source_head":status["sync"]["source_head"],
            "expected_target_head":status["sync"]["target_head"]}),
        )
        .await
        .unwrap();
        let op = started["sync"]["operation"].clone();
        f.session
            .lock()
            .await
            .set_runtime_state(SessionState::Completed, None);
        let error = call(
            &f,
            json!({"action":"abort","operation_id":op["operation_id"],
            "expected_revision":op["revision"]}),
        )
        .await
        .unwrap_err();
        assert!(error.contains("unauthorized"), "{error}");
        f.session
            .lock()
            .await
            .set_runtime_state(SessionState::ExecutingTools, None);
        let aborted = call(
            &f,
            json!({"action":"abort","operation_id":op["operation_id"],
            "expected_revision":op["revision"]}),
        )
        .await
        .unwrap();
        assert_eq!(aborted["sync"]["status"], "aborted", "{aborted}");
    }
}
