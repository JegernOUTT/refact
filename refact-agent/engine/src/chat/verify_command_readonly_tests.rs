use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;

use super::evidence::{
    validate_goal_verify_commands, EvidenceDefaults, SpecAuthority, SpecRejection,
    VerificationSpec, EVIDENCE_POLICY_AUTHORITY,
};
use super::types::{ChatSession, GoalCriterion};
use super::verify_cmd::{VerifyCommandPolicy, VERIFY_COMMAND_DESCRIPTION};
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;

fn criterion(command: &str) -> GoalCriterion {
    GoalCriterion {
        id: "C1".into(),
        text: "Verify the result".into(),
        verify_command: Some(command.into()),
        ..Default::default()
    }
}

fn accept_with_authority(
    command: &str,
    authority: SpecAuthority,
) -> Result<VerificationSpec, SpecRejection> {
    VerificationSpec::propose(
        "criterion:C1",
        command,
        authority,
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

fn accept(command: &str) -> Result<VerificationSpec, SpecRejection> {
    accept_with_authority(
        command,
        SpecAuthority::AgentProposal {
            agent: "parent".into(),
        },
    )
}

fn assert_refused(command: &str, expected_reason: &str) {
    let rejection = accept(command).expect_err(command);
    assert!(
        rejection.reason.contains(expected_reason),
        "{command}: {}",
        rejection.reason
    );
    assert_eq!(rejection.verdict, super::evidence::EvidenceVerdict::Blocked);
    assert_eq!(
        rejection.outcome,
        crate::tasks::types::VerificationOutcome::Rejected
    );
    let error =
        validate_goal_verify_commands(&[criterion(command)], &VerifyCommandPolicy::permissive())
            .expect_err(command);
    assert!(error.contains(expected_reason), "{command}: {error}");
}

fn assert_accepted(command: &str) {
    assert!(accept(command).expect(command).is_accepted());
    validate_goal_verify_commands(&[criterion(command)], &VerifyCommandPolicy::permissive())
        .expect(command);
}

#[test]
fn git_mutating_subcommands_are_refused() {
    for (subcommand, args) in [
        ("push", "origin main --force"),
        ("reset", "--hard"),
        ("clean", "-fdx"),
        ("stash", ""),
        ("checkout", "-- ."),
        ("commit", "-m verified"),
        ("switch", "main"),
        ("restore", "."),
        ("branch", "-D old"),
        ("tag", "v1"),
        ("config", "core.pager true"),
        ("remote", "remove origin"),
        ("fetch", "origin"),
        ("pull", ""),
        ("merge", "main"),
        ("rebase", "main"),
        ("am", "change.patch"),
        ("apply", "change.patch"),
        ("gc", ""),
        ("prune", ""),
        ("update-ref", "refs/heads/main HEAD"),
        ("worktree", "list"),
        ("submodule", "status"),
    ] {
        assert_refused(
            &format!("git {subcommand} {args}"),
            &format!("git subcommand `{subcommand}` is not read-only; allowed:"),
        );
    }
}

#[test]
fn git_config_injection_is_refused() {
    for command in [
        "git -c alias.x=!true x",
        "git -calias.x=!true x",
        "git --config-env=core.pager=X log",
        "git --config-env core.pager=X log",
        "git --exec-path=bin log",
        "git --exec-path bin log",
        "git --git-dir=repo log",
        "git --git-dir repo log",
        "git --work-tree=tree log",
        "git --work-tree tree log",
        "git --namespace=other log",
        "git --namespace other log",
        "git --no-pager -c core.fsmonitor=program status",
    ] {
        assert_refused(command, "can change configuration or execution");
    }
    for command in ["git -C sub log", "git -Csub log"] {
        assert_refused(command, "use a leading `cd <dir> &&`");
    }
    for command in ["git -p log", "git --paginate log"] {
        assert_refused(command, "runs a pager");
    }
}

#[test]
fn git_read_only_commands_still_pass() {
    for command in [
        "git log -1 --name-only --format=",
        "git diff --check",
        "git --no-pager show --stat HEAD",
        "cd sub && git status --short",
    ] {
        assert_accepted(command);
    }
    for subcommand in [
        "log",
        "show",
        "diff",
        "status",
        "rev-parse",
        "ls-files",
        "ls-tree",
        "cat-file",
        "merge-base",
        "describe",
        "blame",
        "grep",
        "shortlog",
        "rev-list",
        "name-rev",
        "show-ref",
        "for-each-ref",
        "diff-tree",
        "diff-index",
        "diff-files",
        "check-ignore",
        "check-attr",
        "cherry",
        "range-diff",
        "count-objects",
    ] {
        assert_accepted(&format!("git {subcommand}"));
    }
}

#[test]
fn git_output_and_external_programs_are_refused() {
    for (command, reason) in [
        ("git diff --output=x.patch", "writes a file"),
        ("git diff --output x.patch", "writes a file"),
        ("git log --ext-diff", "runs an external diff program"),
        (
            "git diff --textconv",
            "runs an external text conversion program",
        ),
        (
            "git cat-file --filters HEAD:src/lib.rs",
            "runs external filters",
        ),
        ("git grep -Ovim x", "opens files in a pager program"),
        ("git grep -O vim x", "opens files in a pager program"),
        (
            "git grep --open-files-in-pager=vim x",
            "opens files in a pager program",
        ),
        (
            "git grep --open-files-in-pager x",
            "opens files in a pager program",
        ),
        ("git diff -- --output=x.patch", "writes a file"),
    ] {
        assert_refused(command, reason);
    }
}

#[test]
fn rg_pre_is_refused() {
    for command in [
        "rg --pre program x",
        "rg --pre=program x",
        "rg --pre-glob=src x",
        "rg --pre-glob src x",
        "rg x -- --pre=program",
    ] {
        assert_refused(command, "enables an external preprocessor");
    }
    assert_accepted("rg --files src");
}

#[test]
fn jq_env_is_refused() {
    for command in ["jq -n env", "jq -n env.HOME", "jq -n env,env.PATH"] {
        assert_refused(command, "jq identifier `env` exposes environment variables");
    }
    for command in [
        "jq -e .ok out.json",
        "jq -e .environment out.json",
        "jq -e .my_env out.json",
        "jq --rawfile data tmp/data.txt .ok out.json",
        "jq --slurpfile data tmp/data.json .ok out.json",
    ] {
        assert_accepted(command);
    }
}

#[test]
fn paths_outside_the_worktree_are_refused() {
    for command in [
        "head -c 10 /etc/hostname",
        "grep -r x ../other",
        "diff a.txt /tmp/b.txt",
        "cmp --ignore-initial=0 a.txt ../b",
        "test -f tmp/../secret",
        "ls --ignore=/etc/hostname",
        "rg --ignore-file=../other x",
        "jq --rawfile data /etc/hostname .ok out.json",
        "jq --slurpfile data ../other.json .ok out.json",
        "cargo check --manifest-path=/tmp/Cargo.toml",
        "npm test --prefix=../other",
    ] {
        assert_refused(
            command,
            "paths outside the worktree are not allowed in an agent-proposed verify_command",
        );
    }
    assert_refused("cmp --ignore-initial=0 a ../b", "contains prose tokens");
}

#[test]
fn relative_paths_still_pass() {
    for command in [
        "test -f tmp/smoke/s2.txt",
        "wc -l src/lib.rs",
        "cargo check --manifest-path=refact-agent/engine/Cargo.toml",
        "go test ./...",
        "cd sub && test -f ./tmp/smoke/s2.txt",
        "cmp --ignore-initial=0 a.txt b.txt",
    ] {
        assert_accepted(command);
    }
}

#[test]
fn planner_authority_verify_lines_are_unchanged() {
    for command in [
        "git push origin main --force",
        "git -c alias.x=!true x",
        "rg --pre program x",
        "jq -n env",
        "head -c 10 /etc/hostname",
    ] {
        for authority in [
            SpecAuthority::CardInstructions,
            SpecAuthority::Operator {
                operator: "user".into(),
            },
        ] {
            assert!(accept_with_authority(command, authority)
                .expect(command)
                .is_accepted());
        }
    }
}

#[test]
fn description_documents_read_only_and_worktree_paths() {
    assert!(VERIFY_COMMAND_DESCRIPTION.contains("read-only"));
    assert!(VERIFY_COMMAND_DESCRIPTION.contains("paths inside the worktree"));
}

#[test]
fn unparseable_prose_gets_the_prose_reason() {
    for command in [
        "review pipeline (agentic, not an executable command)",
        "REVIEW pipeline (agentic, not AN executable command)",
        "FOO=bar review pipeline (agentic, not an executable command)",
        "cd sub && FOO=bar review pipeline (agentic, not an executable command)",
        "cd 'dir with spaces' && FOO='two words' review (AN executable command)",
    ] {
        assert_refused(
            command,
            "prose, not an executable command (contains prose tokens)",
        );
    }
    let rejection = accept_with_authority(
        "review pipeline (agentic, not an executable command)",
        SpecAuthority::CardInstructions,
    )
    .unwrap_err();
    assert!(rejection.reason.contains("shell grouping"), "{rejection:?}");
}

#[test]
fn unparseable_command_keeps_its_syntax_reason() {
    for (command, reason) in [
        ("grep -q \"it works\" f | wc", "shell operators"),
        ("git log '", "malformed quoting"),
        ("git log 'it", "malformed quoting"),
        ("FOO=bar grep -q \"it works\" f | wc", "shell operators"),
        (
            "cd sub && FOO=bar grep -q \"it works\" f | wc",
            "shell operators",
        ),
        (
            "cd 'dir with spaces' && FOO='two words' grep -q \"it works\" f | wc",
            "shell operators",
        ),
        ("notatool (check)", "shell grouping"),
    ] {
        assert_refused(command, reason);
        assert!(
            !accept(command).unwrap_err().reason.contains("prose"),
            "{command}"
        );
    }
}

#[tokio::test]
async fn set_goal_with_git_push_verify_command_is_refused_and_installs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        dir.path().join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
    let app = AppState::from_gcx(gcx).await;
    let mut session = ChatSession::new("verify-readonly".into());
    session.thread.mode = "agent".into();
    let session = Arc::new(Mutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert("verify-readonly".into(), session.clone());
    let ccx = Arc::new(Mutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            vec![],
            "verify-readonly".into(),
            None,
            "model".into(),
            None,
            None,
        )
        .await,
    ));
    let mut tool = crate::tools::tools_list::get_available_tools(app.gcx.clone())
        .await
        .into_iter()
        .find(|tool| tool.tool_description().name == "set_goal")
        .unwrap();
    let args = HashMap::from([
        ("content".into(), json!("Ship")),
        (
            "criteria".into(),
            json!([criterion("git push origin main --force")]),
        ),
    ]);
    let error = tool
        .tool_execute(ccx, &"call".into(), &args)
        .await
        .unwrap_err();
    assert!(error.contains("criterion `C1` verify_command:"), "{error}");
    assert!(
        error.contains("git subcommand `push` is not read-only"),
        "{error}"
    );
    let session = session.lock().await;
    assert!(session.goal.is_none());
    assert!(session.messages.is_empty());
    assert!(session.goal_ledger.is_empty());
    assert!(session.pending_deliveries.is_empty());
    assert!(session.post_tool_side_effects.is_empty());
}
