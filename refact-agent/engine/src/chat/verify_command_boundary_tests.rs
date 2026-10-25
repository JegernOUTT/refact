use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tracing::instrument::WithSubscriber;

use super::evidence::{
    unverifiable_goal_criteria, validate_goal_verify_commands, EvidenceDefaults, SpecAuthority,
    VerificationSpec, EVIDENCE_POLICY_AUTHORITY,
};
use super::types::{ChatCommand, ChatSession, CommandRequest, GoalBudget, GoalCriterion};
use super::verify_cmd::{VerifyCommandPolicy, VERIFY_COMMAND_DESCRIPTION};
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::tools_description::Tool;

async fn tool(app: &AppState, name: &str) -> Box<dyn Tool + Send> {
    crate::tools::tools_list::get_available_tools(app.gcx.clone())
        .await
        .into_iter()
        .find(|tool| tool.tool_description().name == name)
        .unwrap()
}

fn criterion(command: &str) -> GoalCriterion {
    GoalCriterion {
        id: "C1".into(),
        text: "Verify the result".into(),
        verify_command: Some(command.into()),
        ..Default::default()
    }
}

fn accept(command: &str) -> Result<VerificationSpec, super::evidence::SpecRejection> {
    VerificationSpec::propose(
        "criterion:C1",
        command,
        SpecAuthority::AgentProposal {
            agent: "parent".into(),
        },
        &EvidenceDefaults {
            timeout_secs: 60,
            output_cap_bytes: 4000,
        },
    )
    .accept(
        &VerifyCommandPolicy::permissive(),
        EVIDENCE_POLICY_AUTHORITY,
    )
}

#[test]
fn git_log_verify_command_is_accepted_for_agent_proposals() {
    let spec = accept("git log -1 --name-only --format=").unwrap();
    assert!(spec.is_accepted());
    assert_eq!(spec.argv, ["git", "log", "-1", "--name-only", "--format="]);
    for command in [
        "grep -qx tmp/smoke/s2.txt paths.txt",
        "rg --files",
        "test -f file.txt",
        "ls -l",
        "diff old.txt new.txt",
        "cmp old.txt new.txt",
        "jq . report.json",
        "wc -l paths.txt",
        "head paths.txt",
        "tail paths.txt",
        "cd engine && FOO=bar git status --short",
    ] {
        assert!(accept(command).is_ok(), "{command}");
    }
}

#[test]
fn pipeline_is_rejected_with_the_shell_operator_reason() {
    for command in [
        "git log -1 --name-only --format= | grep -qx tmp/smoke/s2.txt",
        "git status > out",
        "git status; ls",
        "git status && ls",
        "git status &",
        "cd engine && git status && ls",
    ] {
        let reason = accept(command).unwrap_err().reason;
        assert!(reason.contains("shell operators (`|`, `>`, `;`, `&&` except a leading `cd <dir> &&`) are not supported in verify_command; use a single command"), "{reason}");
        assert!(!reason.contains("prose"), "{reason}");
    }
}

#[test]
fn prose_is_still_rejected_as_prose() {
    for text in [
        "make sure the tests pass",
        "go through the migration list",
        "Run the lifecycle suite",
    ] {
        let reason = accept(text).unwrap_err().reason;
        assert!(reason.contains("prose"), "{reason}");
        assert!(reason.contains("contains prose tokens"), "{reason}");
    }
}

#[test]
fn unrecognised_program_gets_its_own_reason() {
    for program in ["notatool", "sh", "bash", "rm", "curl"] {
        let reason = accept(&format!("{program} --version")).unwrap_err().reason;
        assert!(
            reason.starts_with(&format!(
                "program `{program}` is not a recognised verification command; recognised: "
            )),
            "{reason}"
        );
        assert!(!reason.contains("prose"), "{reason}");
    }
}

#[test]
fn malformed_quoting_gets_its_own_reason() {
    let reason = accept("git log '").unwrap_err().reason;
    assert!(reason.contains("malformed quoting"), "{reason}");
    assert!(!reason.contains("prose"), "{reason}");
}

#[test]
fn goal_validation_reuses_acceptance_and_shell_policy() {
    let policy =
        VerifyCommandPolicy::from_shell_policy(&crate::tools::shell_gate::ShellGatePolicy {
            deny: vec!["git *".into()],
            ..Default::default()
        });
    let error = validate_goal_verify_commands(&[criterion("git status")], &policy).unwrap_err();
    assert!(
        error.contains("criterion `C1` verify_command: Denied by shell policy"),
        "{error}"
    );
    assert!(
        validate_goal_verify_commands(&[criterion(" ")], &VerifyCommandPolicy::permissive())
            .is_err()
    );
}

async fn context() -> (
    tempfile::TempDir,
    AppState,
    Arc<Mutex<AtCommandsContext>>,
    Arc<Mutex<ChatSession>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        dir.path().join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
    let app = AppState::from_gcx(gcx).await;
    let mut session = ChatSession::new("verify-boundary".into());
    session.thread.mode = "agent".into();
    let session = Arc::new(Mutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert("verify-boundary".into(), session.clone());
    let ccx = Arc::new(Mutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            vec![],
            "verify-boundary".into(),
            None,
            "model".into(),
            None,
            None,
        )
        .await,
    ));
    (dir, app, ccx, session)
}

#[tokio::test]
async fn set_goal_with_unacceptable_verify_command_is_refused_and_installs_nothing() {
    let (_dir, app, ccx, session) = context().await;
    let mut tool = tool(&app, "set_goal").await;
    for command in [
        "git log | grep paths",
        "notatool check",
        "make sure the tests pass",
        "git log '",
        " ",
    ] {
        let args = HashMap::from([
            ("content".into(), json!("Ship")),
            ("criteria".into(), json!([criterion(command)])),
        ]);
        let error = tool
            .tool_execute(ccx.clone(), &"call".into(), &args)
            .await
            .unwrap_err();
        assert!(error.contains("criterion `C1` verify_command:"), "{error}");
        let session = session.lock().await;
        assert!(session.goal.is_none());
        assert!(session.messages.is_empty());
        assert!(session.goal_ledger.is_empty());
        assert!(session.pending_deliveries.is_empty());
        assert!(session.post_tool_side_effects.is_empty());
    }
    let args = HashMap::from([
        ("content".into(), json!("Ship")),
        (
            "criteria".into(),
            json!([criterion("git log -1 --name-only --format=")]),
        ),
    ]);
    tool.tool_execute(ccx, &"accepted".into(), &args)
        .await
        .unwrap();
    let session = session.lock().await;
    let projection = session.try_accepted_control_projection().unwrap();
    let goal =
        super::goal_snapshot_from_messages(&projection.messages, projection.goal.as_ref()).unwrap();
    assert_eq!(
        goal.criteria,
        vec![criterion("git log -1 --name-only --format=")]
    );
}

#[tokio::test]
async fn subagent_goal_with_unacceptable_verify_command_is_refused_before_spawn() {
    let (_dir, app, ccx, session) = context().await;
    let args = HashMap::from([
        ("task".into(), json!("Ship")),
        ("expected_result".into(), json!("Done")),
        ("worktree".into(), json!("isolated")),
        (
            "goal".into(),
            json!({"content":"Ship", "criteria":[criterion("git log -1 --name-only --format= | grep -qx tmp/smoke/s2.txt")]}),
        ),
    ]);
    let error = tool(&app, "subagent")
        .await
        .tool_execute(ccx, &"call".into(), &args)
        .await
        .unwrap_err();
    assert!(error.contains("shell operators"), "{error}");
    assert!(app.agents.list_all().await.is_empty());
    assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    assert_eq!(app.chat.sessions.read().await.len(), 1);
    assert!(session.lock().await.messages.is_empty());
}

#[tokio::test]
async fn update_goal_appends_the_note_and_warns_about_an_unverifiable_criterion() {
    let (_dir, app, ccx, session) = context().await;
    let mut goal =
        super::internal_roles::goal("agent", 1, "Ship", None, true, GoalBudget::default());
    goal.extra.get_mut("goal").unwrap()["criteria"] = json!([criterion("git log -1 | grep x")]);
    session.lock().await.queue_post_tool_side_effect(goal);
    let (_, result) = tool(&app, "update_goal")
        .await
        .tool_execute(
            ccx,
            &"call".into(),
            &HashMap::from([("note".into(), json!("Update"))]),
        )
        .await
        .expect("an installed criterion must not prevent a progress note");
    let crate::call_validation::ContextEnum::ChatMessage(result) = &result[0] else {
        panic!("expected tool result");
    };
    let text = result.content.content_text_only();
    let (result, advisory) = text.split_once("\n\nAdvisory:\n").unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(result).unwrap(),
        json!({"seq":1,"truncated":false})
    );
    assert!(
        advisory.contains("criterion `C1` verify_command can never run:"),
        "{advisory}"
    );
    assert!(advisory.contains("shell operators"), "{advisory}");
    let session = session.lock().await;
    assert_eq!(session.post_tool_side_effects.len(), 1);
    let projection = session.try_accepted_control_projection().unwrap();
    let deltas: Vec<_> = projection
        .messages
        .iter()
        .filter(|message| {
            message
                .extra
                .get("event")
                .is_some_and(|event| event["subkind"] == "goal_delta")
        })
        .collect();
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].content.content_text_only(), "Update");
    assert_eq!(deltas[0].extra["event"]["payload"]["seq"], 1);
    let goal =
        super::goal_snapshot_from_messages(&projection.messages, projection.goal.as_ref()).unwrap();
    assert_eq!(goal.criteria, vec![criterion("git log -1 | grep x")]);
    assert!(session.messages.is_empty());
}

#[tokio::test]
async fn update_goal_without_criteria_problems_has_no_advisory() {
    for criteria in [vec![], vec![criterion("git status --short")]] {
        let (_dir, app, ccx, session) = context().await;
        session.lock().await.install_goal_with_criteria(
            "agent",
            "Ship",
            true,
            GoalBudget::default(),
            criteria,
        );
        let (_, result) = tool(&app, "update_goal")
            .await
            .tool_execute(
                ccx.clone(),
                &"call".into(),
                &HashMap::from([("note".into(), json!("Update"))]),
            )
            .await
            .unwrap();
        let crate::call_validation::ContextEnum::ChatMessage(result) = &result[0] else {
            panic!("expected tool result");
        };
        let text = result.content.content_text_only();
        let value: Value =
            serde_json::from_str(&text).expect("clean result stays JSON without advisory");
        assert_eq!(value["seq"], 1);
        assert_eq!(value["truncated"], false);
        assert!(!text.contains("Advisory"), "{text}");
    }
}

#[test]
fn goal_advisory_lists_every_refused_criterion_under_current_policy() {
    let mut session = ChatSession::new("legacy-goal".into());
    let mut denied = criterion("git status");
    denied.id = "C2".into();
    let mut allowed = criterion("rg --files");
    allowed.id = "C3".into();
    let prose = GoalCriterion {
        id: "C4".into(),
        text: "Judge this".into(),
        ..Default::default()
    };
    session.install_goal_with_criteria(
        "agent",
        "Ship",
        true,
        GoalBudget::default(),
        vec![criterion("git log -1 | grep x"), denied, allowed, prose],
    );
    let policy =
        VerifyCommandPolicy::from_shell_policy(&crate::tools::shell_gate::ShellGatePolicy {
            deny: vec!["git *".into()],
            ..Default::default()
        });
    let advisory = unverifiable_goal_criteria(&session, &policy);
    assert_eq!(advisory.len(), 2, "{advisory:?}");
    assert!(advisory[0].starts_with("criterion `C1` verify_command can never run:"));
    assert!(advisory[0].contains("shell operators"));
    assert!(advisory[1]
        .starts_with("criterion `C2` verify_command can never run: Denied by shell policy"));
}

#[tokio::test]
async fn http_update_goal_with_an_unverifiable_criterion_is_not_a_bad_request() {
    let (_dir, app, _ccx, session) = context().await;
    let log_file = tempfile::NamedTempFile::new_in(_dir.path()).unwrap();
    let writer = log_file.reopen().unwrap();
    let subscriber = tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .with_writer(move || writer.try_clone().unwrap())
            .finish(),
    );
    for command in [
        ChatCommand::SetGoal {
            plan: None,
            content: "Ship".into(),
            budget: None,
            criteria: Some(vec![criterion("git log -1 | grep x")]),
        },
        ChatCommand::UpdateGoal {
            plan: None,
            note: Some("Update".into()),
        },
    ] {
        let update = matches!(command, ChatCommand::UpdateGoal { .. });
        if update {
            let mut session = session.lock().await;
            session.install_goal_with_criteria(
                "agent",
                "Ship",
                true,
                GoalBudget::default(),
                vec![criterion("git log -1 | grep x")],
            );
            session
                .queue_processor_running
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let before = session.lock().await.messages.clone();
        let request = CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: uuid::Uuid::new_v4().to_string(),
            priority: false,
            command,
        };
        let response = super::handle_v1_chat_command(
            State(app.clone()),
            Path("verify-boundary".into()),
            serde_json::to_vec(&request).unwrap().into(),
        )
        .with_subscriber(subscriber.clone())
        .await
        .unwrap();
        assert_eq!(
            response.status(),
            if update {
                axum::http::StatusCode::ACCEPTED
            } else {
                axum::http::StatusCode::BAD_REQUEST
            }
        );
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        if update {
            assert_eq!(body, json!({"status":"accepted"}));
        } else {
            assert!(body.to_string().contains("shell operators"), "{body}");
        }
        let session = session.lock().await;
        assert_eq!(
            serde_json::to_value(&session.messages).unwrap(),
            serde_json::to_value(before).unwrap()
        );
        assert_eq!(session.command_queue.len(), usize::from(update));
        assert!(session.pending_deliveries.is_empty());
    }
    let processor_running = session.lock().await.queue_processor_running.clone();
    let processor = tokio::spawn(
        super::process_command_queue(app, session.clone(), processor_running)
            .with_subscriber(subscriber),
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let mut session = session.lock().await;
            if session.messages.iter().any(|message| {
                message
                    .extra
                    .get("event")
                    .is_some_and(|event| event["subkind"] == "goal_delta")
            }) {
                session.closed = true;
                session.queue_notify.notify_one();
                break;
            }
            drop(session);
            tokio::task::yield_now().await;
        }
        processor.await.unwrap();
    })
    .await
    .expect("queued UpdateGoal must append the note despite installed criteria");
    let session = session.lock().await;
    let deltas: Vec<_> = session
        .messages
        .iter()
        .filter(|message| {
            message
                .extra
                .get("event")
                .is_some_and(|event| event["subkind"] == "goal_delta")
        })
        .collect();
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].content.content_text_only(), "Update");
    assert_eq!(
        session.goal.as_ref().unwrap().criteria,
        vec![criterion("git log -1 | grep x")]
    );
    assert!(session.command_queue.is_empty());
    let logs = std::fs::read_to_string(log_file.path()).unwrap();
    for source in [
        "UpdateGoal accepted with advisory",
        "UpdateGoal appended with advisory",
    ] {
        let advisory = logs
            .lines()
            .find(|line| line.contains(source))
            .expect(source);
        assert!(
            advisory.contains("criterion `C1` verify_command can never run:"),
            "{advisory}"
        );
        assert!(advisory.contains("shell operators"), "{advisory}");
    }
}

#[test]
fn goal_advisory_tolerates_an_unreadable_projection() {
    let mut session = ChatSession::new("invalid-projection".into());
    let mut invalid =
        crate::call_validation::ChatMessage::new("compression_report".into(), "broken".into());
    invalid.extra.insert(
        "compression_report".into(),
        json!({"kind":"reconstructed_history", "schema_version":999}),
    );
    session.messages.push(invalid);
    assert!(session.try_accepted_control_projection().is_err());
    assert!(unverifiable_goal_criteria(&session, &VerifyCommandPolicy::permissive()).is_empty());
}

#[tokio::test]
async fn verify_command_schema_descriptions_document_the_shared_constraints() {
    let (_dir, app, _ccx, _session) = context().await;
    let set_goal = tool(&app, "set_goal").await.tool_description().input_schema;
    assert_eq!(
        set_goal["properties"]["criteria"]["items"]["properties"]["verify_command"]["description"],
        VERIFY_COMMAND_DESCRIPTION
    );
    let subagent = tool(&app, "subagent").await.tool_description().input_schema;
    assert!(subagent["properties"]["goal"]["description"]
        .as_str()
        .unwrap()
        .contains(VERIFY_COMMAND_DESCRIPTION));
}
