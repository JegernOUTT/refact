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
            stack
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
