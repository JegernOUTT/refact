//! Opens a review in one call: a single comparison, or every PR of a stack.

use std::path::PathBuf;

use refact_review_core::*;
use serde_json::{json, Value};

use super::{candidate, changed_files, describe, find_member, member_label, owner, Ccx, Outcome};
use crate::review::{context, service, stack_entry, stacks};

pub struct StackEntry {
    pub pr: Option<u64>,
    pub branch: String,
    pub head: Option<String>,
    pub base: Option<String>,
}

#[derive(Default)]
pub struct OpenInput {
    pub repository: Option<String>,
    pub title: Option<String>,
    pub target: Option<String>,
    pub base: Option<String>,
    pub head: Option<String>,
    pub local_changes: bool,
    pub stack: Vec<StackEntry>,
    pub focus: Option<String>,
}

/// `owner/name` on a forge host, from a PR link or a Git remote.
pub fn repository_identity(reference: &str) -> Option<(String, String)> {
    let cleaned = reference.trim().trim_end_matches(".git");
    let rest = cleaned
        .split_once("://")
        .map_or(cleaned, |(_, rest)| rest)
        .trim_start_matches("git@");
    let (host, path) = rest.split_once(['/', ':'])?;
    if !host.contains('.') {
        return None;
    }
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let (host, owner, name) = if host.ends_with("graphite.com") || host.ends_with("graphite.dev") {
        // app.graphite.com/github/pr/<owner>/<name>/<number>
        let _provider = parts.next()?;
        let _kind = parts.next()?;
        ("github.com", parts.next()?, parts.next()?)
    } else {
        (host, parts.next()?, parts.next()?)
    };
    Some((host.to_string(), format!("{owner}/{name}")))
}

struct Resolved {
    members: Vec<stack_entry::OpenMember>,
    origin: Option<String>,
}

fn resolve(repository: PathBuf, entries: Vec<StackEntry>) -> Outcome<Resolved> {
    let repo = git2::Repository::open(&repository)
        .map_err(|error| format!("`repository` is not a Git repository: {}", error.message()))?;
    let commit = |reference: &str, field: String| {
        repo.revparse_single(reference)
            .and_then(|object| object.peel_to_commit())
            .map(|commit| commit.id().to_string())
            .map_err(|_| {
                format!(
                    "`{field}`: `{reference}` is not a commit in this repository; fetch it first"
                )
            })
    };
    let mut members: Vec<stack_entry::OpenMember> = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let head_ref = entry.head.as_deref().unwrap_or(&entry.branch);
        let head = commit(head_ref, format!("stack[{index}].head"))?;
        let base = match (&entry.base, members.last()) {
            (Some(base), _) => commit(base, format!("stack[{index}].base"))?,
            (None, Some(previous)) => previous.head_oid.clone(),
            (None, None) => {
                return Err("`stack[0].base` is required: the commit the bottom PR starts from, e.g. `git merge-base <trunk> <head>`".into())
            }
        };
        let live = commit(&entry.branch, String::new()).ok().as_ref() == Some(&head);
        let key = entry
            .pr
            .map_or_else(|| format!("m{}", index + 1), |pr| format!("pr{pr}"));
        if members
            .iter()
            .any(|member| member.key == key || member.branch_ref == entry.branch)
        {
            return Err(format!("`stack[{index}]` repeats an earlier PR or branch"));
        }
        members.push(stack_entry::OpenMember {
            key,
            branch_ref: entry.branch,
            pr: entry.pr,
            base_oid: base,
            head_oid: head,
            branch_is_head: live,
        });
    }
    let origin = repo
        .find_remote("origin")
        .ok()
        .and_then(|remote| remote.url().map(str::to_string));
    Ok(Resolved {
        members,
        origin,
    })
}

async fn stack_receipt(
    ccx: &Ccx,
    input: OpenInput,
    repository: String,
    call_id: &str,
) -> Outcome<context::ContextReceipt> {
    let (app, chat) = owner(ccx).await?;
    let entries = input.stack;
    let branches: Vec<String> = entries.iter().map(|entry| entry.branch.clone()).collect();
    let existing = stacks::get(&app, &chat).await?;
    let manifest = match existing {
        Some(stack) => {
            let same = stack.members.len() == branches.len()
                && stack
                    .members
                    .iter()
                    .zip(&branches)
                    .all(|(member, branch)| &member.branch_ref == branch);
            if !same {
                return Err(format!(
                    "This chat already reviews the stack {}. Review a different set of PRs in a new chat",
                    stack.members.iter().map(member_label).collect::<Vec<_>>().join(", ")
                ));
            }
            if stack_entry::is_historical(&stack) {
                let pinned = stack.clone();
                tokio::task::spawn_blocking(move || {
                    let repo = git2::Repository::open(&pinned.repository)
                        .map_err(|error| error.to_string())?;
                    let comparisons = entries
                        .into_iter()
                        .map(|entry| (entry.branch, entry.base, entry.head))
                        .collect::<Vec<_>>();
                    stacks::validate_open_comparisons(&repo, &pinned, &comparisons)
                        .map_err(describe)
                })
                .await
                .map_err(|error| error.to_string())??;
                stack
            } else {
                stacks::refresh_on_open(
                    &app,
                    &chat,
                    format!("open-stack:{call_id}"),
                    entries
                        .into_iter()
                        .map(|entry| (entry.branch, entry.base, entry.head))
                        .collect(),
                )
                .await?
            }
        }
        None => {
            let path = PathBuf::from(&repository);
            let resolved = tokio::task::spawn_blocking(move || resolve(path, entries))
                .await
                .map_err(|error| error.to_string())??;
            let identity = input
                .target
                .as_deref()
                .and_then(repository_identity)
                .or_else(|| resolved.origin.as_deref().and_then(repository_identity));
            let focus = match input
                .focus
                .as_deref()
                .map(|focus| focus.trim().trim_start_matches('#'))
            {
                Some(focus) => resolved
                    .members
                    .iter()
                    .find(|member| {
                        member.branch_ref == focus
                            || member.branch_ref.ends_with(focus)
                            || member.pr.map(|pr| pr.to_string()).as_deref() == Some(focus)
                    })
                    .map(|member| member.key.clone())
                    .ok_or("`focus` must name one PR number or branch of `stack`")?,
                None => resolved.members[0].key.clone(),
            };
            let title = input
                .title
                .clone()
                .unwrap_or_else(|| format!("Stack of {} pull requests", resolved.members.len()));
            stack_entry::open_whole(
                &app,
                &chat,
                stack_entry::OpenStack {
                    repository: repository.clone(),
                    title,
                    reference: input
                        .target
                        .clone()
                        .unwrap_or_else(|| "review_open".to_string()),
                    selected: focus,
                    identity,
                    members: resolved.members,
                    request_key: format!("open-stack:{call_id}"),
                },
            )
            .await?
        }
    };
    let member = match input.focus.as_deref() {
        Some(focus) => find_member(&manifest, focus)?.id.clone(),
        None => manifest.scope_choice.selected_member_id.clone(),
    };
    stacks::capture(ccx.clone(), member, format!("open:{call_id}")).await
}

async fn single_receipt(
    ccx: &Ccx,
    input: OpenInput,
    repository: String,
    call_id: &str,
) -> Outcome<context::ContextReceipt> {
    let (app, chat) = owner(ccx).await?;
    if let Some(stack) = stacks::get(&app, &chat).await? {
        return Err(format!(
            "This chat already reviews the stack {}. Review an unrelated comparison in a new chat",
            stack
                .members
                .iter()
                .map(member_label)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let (Some(base), Some(head)) = (input.base, input.head) else {
        return Err("Pass `base` and `head` for one comparison, or `stack` for several PRs".into());
    };
    let root = service::trusted_root(&app, &chat).await?;
    let existing = service::get_for_chat(root, chat).await.map_err(describe)?;
    let target_reference = input.target.unwrap_or_else(|| head.clone());
    context::set_context(
        ccx.clone(),
        context::SetContextRequest {
            repository,
            title: input.title.unwrap_or_else(|| target_reference.clone()),
            target_reference,
            base_ref: base,
            candidate_ref: head,
            policy: CapturePolicy {
                staged: input.local_changes,
                working: input.local_changes,
                untracked: input.local_changes,
            },
            review_id: existing.as_ref().map(|review| review.id.clone()),
            expected_revision: existing.as_ref().map(|review| review.revision),
            request_key: format!("open:{call_id}"),
        },
    )
    .await
}

/// Captures the comparison, binds the investigation checkout and returns a compact receipt.
pub async fn open(ccx: &Ccx, input: OpenInput, call_id: &str) -> Outcome<(Review, Value)> {
    let (app, chat) = owner(ccx).await?;
    let repository = match input.repository.clone() {
        Some(repository) => repository,
        None => service::trusted_root(&app, &chat)
            .await?
            .to_string_lossy()
            .into_owned(),
    };
    let receipt = if input.stack.is_empty() {
        single_receipt(ccx, input, repository, call_id).await?
    } else {
        stack_receipt(ccx, input, repository, call_id).await?
    };
    let opened = super::require(ccx, None).await?;
    let discussion = crate::review::remote::pull_all(&opened).await;
    let target = super::require(ccx, None).await?;
    let current = candidate(&target.review)?;
    let changed = changed_files(current);
    let overview = super::read::overview(&target).await?;
    let value = json!({
        "opened": target.review.title,
        "checkout": receipt.worktree.root,
        "base": current.base_oid,
        "head": current.head_oid,
        "stack": overview["stack"],
        "changed_files": changed.iter().take(200).map(|(path, status)| json!({"path":path,"status":status})).collect::<Vec<_>>(),
        "changed_files_total": changed.len(),
        "pr_discussion": discussion,
        "next": "The canvas is open. Publish an orientation page now with review_tour, then launch workers and keep filling the workspace as their reports arrive.",
    });
    Ok((target.review, value))
}

#[cfg(test)]
mod tests {
    use super::repository_identity;

    #[tokio::test]
    async fn reopening_live_stack_refreshes_heads_without_changing_unchanged_reviews() {
        reopen_scenario(0).await;
    }

    #[tokio::test]
    async fn reopening_live_stack_follows_restacked_parent_and_rejects_divergence() {
        reopen_scenario(1).await;
    }

    #[tokio::test]
    async fn reopening_live_stack_rejects_conflicting_explicit_refs() {
        reopen_scenario(2).await;
    }

    #[tokio::test]
    async fn reopening_recorded_stack_checks_explicit_refs_against_pins() {
        reopen_scenario(3).await;
    }

    async fn reopen_scenario(scenario: u8) {
        use super::*;
        use crate::app_state::AppState;
        let temp = crate::test_paths::tempdir();
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let repo = git2::Repository::init(&root).unwrap();
        let signature = git2::Signature::now("Test", "test@localhost").unwrap();
        let commit = |parent: Option<git2::Oid>, content: &str| {
            let mut builder = repo.treebuilder(None).unwrap();
            builder
                .insert("file", repo.blob(content.as_bytes()).unwrap(), 0o100644)
                .unwrap();
            let tree = repo.find_tree(builder.write().unwrap()).unwrap();
            let parents = parent
                .map(|id| repo.find_commit(id).unwrap())
                .into_iter()
                .collect::<Vec<_>>();
            repo.commit(
                None,
                &signature,
                &signature,
                content,
                &tree,
                &parents.iter().collect::<Vec<_>>(),
            )
            .unwrap()
        };
        let base = commit(None, "base");
        let first = commit(Some(base), "first");
        let second = commit(Some(first), "second");
        repo.reference("refs/heads/first", first, true, "fixture")
            .unwrap();
        repo.reference("refs/heads/second", second, true, "fixture")
            .unwrap();
        repo.set_head("refs/heads/second").unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let app = AppState::from_gcx(gcx).await;
        let mut session = crate::chat::types::ChatSession::new("owner".into());
        session.work_project_root = Some(root.clone());
        session.set_runtime_state(crate::chat::types::SessionState::ExecutingTools, None);
        let abort = session.abort_flag.clone();
        app.chat.sessions.write().await.insert(
            "owner".into(),
            std::sync::Arc::new(tokio::sync::Mutex::new(session)),
        );
        let ccx = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::at_commands::at_commands::AtCommandsContext::new_with_abort(
                app.clone(),
                4096,
                10,
                false,
                vec![],
                "owner".into(),
                None,
                String::new(),
                None,
                None,
                Some(abort),
            )
            .await,
        ));
        let input = || OpenInput {
            stack: vec![
                StackEntry {
                    pr: None,
                    branch: "first".into(),
                    head: None,
                    base: Some(base.to_string()),
                },
                StackEntry {
                    pr: None,
                    branch: "second".into(),
                    head: None,
                    base: (scenario == 3).then(|| base.to_string()),
                },
            ],
            focus: Some("second".into()),
            ..Default::default()
        };
        let repository = root.to_string_lossy().into_owned();
        stack_receipt(&ccx, input(), repository.clone(), "initial")
            .await
            .unwrap();
        let store = ReviewStore::open(&root).unwrap();
        let initial = stacks::get(&app, "owner").await.unwrap().unwrap();
        let bottom = store
            .stack_review_for_member(&initial.id, &initial.members[0].id)
            .unwrap()
            .unwrap();
        let top = store
            .stack_review_for_member(&initial.id, &initial.members[1].id)
            .unwrap()
            .unwrap();
        let mut matching = input();
        matching.stack[1].head = Some(second.to_string());
        matching.stack[1].base = Some(if scenario == 3 { base } else { first }.to_string());
        stack_receipt(&ccx, matching, repository.clone(), "matching-explicit")
            .await
            .unwrap();
        assert_eq!(stacks::get(&app, "owner").await.unwrap().unwrap(), initial);
        assert_eq!(store.load(&top.id).unwrap(), top);
        if scenario == 3 {
            assert!(stack_entry::is_historical(&initial));
            for field in ["base", "head"] {
                let mut mismatched = input();
                if field == "head" {
                    mismatched.stack[1].head = Some(first.to_string());
                } else {
                    mismatched.stack[1].base = Some(first.to_string());
                }
                let error = stack_receipt(&ccx, mismatched, repository.clone(), field)
                    .await
                    .err()
                    .expect("recorded comparison mismatch must fail");
                assert!(
                    error.contains("recorded comparison") && error.contains("new chat"),
                    "{error}"
                );
                assert_eq!(stacks::get(&app, "owner").await.unwrap().unwrap(), initial);
                assert_eq!(store.load(&top.id).unwrap(), top);
            }
            return;
        }
        if scenario == 1 {
            let new_first = commit(Some(first), "new first");
            let new_second = commit(Some(new_first), "rebased second");
            repo.reference("refs/heads/first", new_first, true, "advance parent")
                .unwrap();
            repo.reference("refs/heads/second", new_second, true, "restack child")
                .unwrap();
            stack_receipt(&ccx, input(), repository.clone(), "restacked")
                .await
                .unwrap();
            let restacked = stacks::get(&app, "owner").await.unwrap().unwrap();
            assert_eq!(restacked.members[0].head_oid, Some(new_first.to_string()));
            assert_eq!(restacked.members[1].head_oid, Some(new_second.to_string()));
            assert_eq!(
                restacked.members[1].resolved_base_oid,
                Some(new_first.to_string())
            );
            assert_ne!(
                store.load(&bottom.id).unwrap().current_candidate_id,
                bottom.current_candidate_id
            );
            assert_ne!(
                store.load(&top.id).unwrap().current_candidate_id,
                top.current_candidate_id
            );
            let http_refresh =
                stacks::refresh(&app, "owner", "http-refresh".into(), restacked.revision)
                    .await
                    .unwrap();
            assert_eq!(http_refresh.revision, restacked.revision + 1);
            stack_receipt(&ccx, input(), repository.clone(), "after-http-refresh")
                .await
                .unwrap();
            assert_eq!(
                stacks::get(&app, "owner").await.unwrap().unwrap(),
                http_refresh
            );
            let restacked = http_refresh;
            let reviews = store.list_for_chat("owner").unwrap();
            let divergent = commit(Some(new_first), "divergent parent");
            repo.reference(
                "refs/heads/first",
                divergent,
                true,
                "advance without restack",
            )
            .unwrap();
            let error = stack_receipt(&ccx, input(), repository, "divergent")
                .await
                .err()
                .unwrap();
            assert!(
                error.contains("second") && error.contains("restacked"),
                "{error}"
            );
            assert_eq!(
                stacks::get(&app, "owner").await.unwrap().unwrap(),
                restacked
            );
            assert_eq!(store.list_for_chat("owner").unwrap(), reviews);
            return;
        }
        if scenario == 2 {
            let advanced = commit(Some(second), "advanced before conflicting reopen");
            repo.reference("refs/heads/second", advanced, true, "advance")
                .unwrap();
            let reviews = store.list_for_chat("owner").unwrap();
            for field in ["head", "base"] {
                let mut conflicting = input();
                if field == "head" {
                    conflicting.stack[1].head = Some(first.to_string());
                } else {
                    conflicting.stack[1].base = Some(base.to_string());
                }
                let error = stack_receipt(&ccx, conflicting, repository.clone(), field)
                    .await
                    .err()
                    .expect("conflicting refs must fail");
                assert!(
                    error.contains("live branches") && error.contains("new chat"),
                    "{error}"
                );
                assert_eq!(stacks::get(&app, "owner").await.unwrap().unwrap(), initial);
                assert_eq!(store.list_for_chat("owner").unwrap(), reviews);
            }
            return;
        }
        let updated = commit(Some(second), "updated");
        repo.reference("refs/heads/second", updated, true, "update")
            .unwrap();
        stack_receipt(&ccx, input(), repository.clone(), "updated")
            .await
            .unwrap();
        let refreshed = stacks::get(&app, "owner").await.unwrap().unwrap();
        let current = store.load(&top.id).unwrap();
        let candidate = current
            .candidates
            .iter()
            .find(|c| Some(&c.id) == current.current_candidate_id.as_ref())
            .unwrap();
        assert_eq!(
            candidate.head_oid,
            updated.to_string(),
            "reopen must capture the updated branch head"
        );
        assert_eq!(current.candidates.len(), top.candidates.len() + 2);
        assert_eq!(store.load(&bottom.id).unwrap(), bottom);
        assert!(refreshed.revision > initial.revision);
        stack_receipt(&ccx, input(), repository.clone(), "identical")
            .await
            .unwrap();
        assert_eq!(
            stacks::get(&app, "owner").await.unwrap().unwrap(),
            refreshed
        );
        assert_eq!(store.load(&top.id).unwrap(), current);
        assert_eq!(store.load(&bottom.id).unwrap(), bottom);
        let mut reordered = input();
        reordered.stack.reverse();
        let error = stack_receipt(&ccx, reordered, repository, "reordered")
            .await
            .err()
            .unwrap();
        assert!(
            error.contains("Review a different set of PRs in a new chat"),
            "{error}"
        );
    }

    #[test]
    fn forge_identity_comes_from_pr_links_and_remotes() {
        for (reference, host, name) in [
            (
                "https://github.com/acme/widgets/pull/12",
                "github.com",
                "acme/widgets",
            ),
            (
                "https://app.graphite.com/github/pr/acme/widgets/12/Some-title",
                "github.com",
                "acme/widgets",
            ),
            (
                "git@github.com:acme/widgets.git",
                "github.com",
                "acme/widgets",
            ),
            (
                "https://gitlab.example.com/acme/widgets.git",
                "gitlab.example.com",
                "acme/widgets",
            ),
        ] {
            assert_eq!(
                repository_identity(reference),
                Some((host.to_string(), name.to_string())),
                "{reference}"
            );
        }
        assert_eq!(repository_identity("feature/branch"), None);
    }
}
