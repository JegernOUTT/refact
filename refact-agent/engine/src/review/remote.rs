//! Carries comments between a GitHub pull request and the workspace through the `gh` CLI,
//! with no model in the loop. Graphite stacks are GitHub pull requests and travel the same way.

use std::{path::PathBuf, process::Stdio, time::Duration};

use async_trait::async_trait;
use refact_review_core::*;
use serde::Serialize;
use serde_json::{json, Value};

use super::authoring::comments::{self, Outgoing, RemoteComment};
use super::authoring::{candidate, describe, open::repository_identity, Outcome, Target};
use super::service;
use crate::app_state::AppState;

pub const SOURCE: &str = "github";
const PAGE: usize = 100;
const MAX_PAGES: usize = 10;
const MARK: &str = "<!-- refact-review:";

#[async_trait]
pub trait Forge: Send + Sync {
    async fn get(&self, path: &str) -> Outcome<Value>;
    async fn post(&self, path: &str, body: Value) -> Outcome<Value>;
}

pub struct Gh {
    pub host: String,
}

impl Gh {
    async fn run(&self, method: &str, path: &str, body: Option<Value>) -> Outcome<Value> {
        if cfg!(test) {
            return Err("gh is never started from tests".into());
        }
        let mut command = tokio::process::Command::new("gh");
        command.args([
            "api",
            "--method",
            method,
            "-H",
            "Accept: application/vnd.github+json",
        ]);
        if self.host != "github.com" {
            command.args(["--hostname", &self.host]);
        }
        command.arg(path);
        if body.is_some() {
            command.args(["--input", "-"]);
        }
        command
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("NO_COLOR", "1")
            .stdin(if body.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let exchange = async {
            let mut child = command.spawn().map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => "the gh CLI is not installed".to_string(),
                _ => format!("gh could not start: {error}"),
            })?;
            if let (Some(body), Some(mut stdin)) = (body, child.stdin.take()) {
                use tokio::io::AsyncWriteExt;
                stdin
                    .write_all(body.to_string().as_bytes())
                    .await
                    .map_err(|error| format!("gh did not take the request: {error}"))?;
            }
            child
                .wait_with_output()
                .await
                .map_err(|error| format!("gh did not finish: {error}"))
        };
        let output = tokio::time::timeout(Duration::from_secs(30), exchange)
            .await
            .map_err(|_| "gh did not answer within 30 seconds".to_string())??;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = stderr
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("gh failed");
            return Err(reason.chars().take(300).collect());
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|_| "gh returned an answer that is not JSON".to_string())
    }
}

#[async_trait]
impl Forge for Gh {
    async fn get(&self, path: &str) -> Outcome<Value> {
        self.run("GET", path, None).await
    }
    async fn post(&self, path: &str, body: Value) -> Outcome<Value> {
        self.run("POST", path, Some(body)).await
    }
}

/// The pull request a GitHub or Graphite link names.
pub fn pr_identity(reference: &str) -> Option<PrIdentity> {
    let (host, repository) = repository_identity(reference)?;
    let (owner, name) = repository.split_once('/')?;
    let path = reference.split(['?', '#']).next()?;
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let at = parts
        .windows(2)
        .position(|pair| pair[0] == owner && pair[1] == name)?;
    let number = match &parts[at + 2..] {
        ["pull" | "pulls", number, ..] => number,
        [number, ..] if reference.contains("graphite.") => number,
        _ => return None,
    };
    Some(PrIdentity {
        host,
        repository,
        number: number.parse().ok().filter(|number| *number > 0)?,
    })
}

pub fn pr_of(review: &Review, stack: Option<&StackManifest>) -> Option<PrIdentity> {
    stack
        .and_then(|stack| {
            stack
                .members
                .iter()
                .find(|member| member.review_id.as_deref() == Some(review.id.as_str()))
        })
        .and_then(|member| member.pr_identity.clone())
        .or_else(|| pr_identity(&review.target_reference))
}

/// Every posted comment carries its local id, so it is recognised when it comes back.
fn marker(local_id: &str) -> String {
    format!("{MARK}{local_id} -->")
}

fn marked(body: &str) -> Option<&str> {
    let start = body.rfind(MARK)? + MARK.len();
    let end = body[start..].find(" -->")?;
    Some(&body[start..start + end])
}

/// Hidden markers that tools leave in a comment are not part of what was said.
fn visible(body: &str) -> String {
    let mut shown = String::new();
    let mut rest = body;
    while let Some(start) = rest.find("<!--") {
        shown.push_str(&rest[..start]);
        rest = rest[start..]
            .find("-->")
            .map_or("", |end| &rest[start + end + 3..]);
    }
    shown.push_str(rest);
    shown.trim().to_string()
}

fn text(item: &Value, name: &str) -> Option<String> {
    item[name]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn general(prefix: &str, item: &Value, time: &str) -> Option<RemoteComment> {
    Some(RemoteComment {
        id: format!("{prefix}:{}", item["id"].as_u64()?),
        author: text(&item["user"], "login").unwrap_or_else(|| "unknown".into()),
        body: text(item, "body")?,
        path: None,
        line: None,
        start_line: None,
        side: Side::Head,
        reply_to: None,
        url: text(item, "html_url"),
        created_at_ms: item[time]
            .as_str()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.timestamp_millis()),
    })
}

/// The text a reviewer submitted with a review, with its verdict up front.
fn review_summary(item: &Value) -> Option<RemoteComment> {
    let mut comment = general("review", item, "submitted_at")?;
    let verdict = match item["state"].as_str() {
        Some("CHANGES_REQUESTED") => "Changes requested",
        Some("APPROVED") => "Approved",
        _ => return Some(comment),
    };
    comment.body = format!("**{verdict}**\n\n{}", comment.body);
    Some(comment)
}

fn line_comment(item: &Value) -> Option<RemoteComment> {
    let mut comment = general("review_comment", item, "created_at")?;
    comment.reply_to = item["in_reply_to_id"]
        .as_u64()
        .map(|id| format!("review_comment:{id}"));
    let line = |name: &str| item[name].as_u64().map(|line| line as u32);
    match (text(item, "path"), line("line")) {
        (Some(path), Some(last)) => {
            comment.path = Some(path);
            comment.line = Some(last);
            comment.start_line = line("start_line");
            if item["side"] == "LEFT" {
                comment.side = Side::Base;
            }
        }
        // Later commits removed the line; keep where it was said.
        (Some(path), None) if comment.reply_to.is_none() => {
            let was = line("original_line").map_or(String::new(), |line| format!(":{line}"));
            comment.body = format!(
                "_On `{path}{was}`, before later commits._\n\n{}",
                comment.body
            );
        }
        _ => {}
    }
    Some(comment)
}

async fn pages(forge: &dyn Forge, path: &str) -> Outcome<Vec<Value>> {
    let mut all = Vec::new();
    for page in 1..=MAX_PAGES {
        let batch = forge
            .get(&format!("{path}?per_page={PAGE}&page={page}"))
            .await?;
        let items = batch.as_array().cloned().unwrap_or_default();
        let last = items.len() < PAGE;
        all.extend(items);
        if last {
            break;
        }
    }
    Ok(all)
}

/// Graphite posts the stack's table of contents on every PR; it is navigation, not discussion.
const STACK_LISTING: &str = "utm_source=stack-comment";

/// Line comments, general comments and review summaries of one pull request, oldest first.
pub async fn fetch(forge: &dyn Forge, pr: &PrIdentity) -> Outcome<Vec<RemoteComment>> {
    let (repository, number) = (&pr.repository, pr.number);
    let mut found: Vec<RemoteComment> = pages(
        forge,
        &format!("repos/{repository}/pulls/{number}/comments"),
    )
    .await?
    .iter()
    .filter_map(line_comment)
    .collect();
    found.extend(
        pages(
            forge,
            &format!("repos/{repository}/issues/{number}/comments"),
        )
        .await?
        .iter()
        .filter_map(|item| general("issue_comment", item, "created_at"))
        .filter(|comment| !comment.body.contains(STACK_LISTING)),
    );
    found.extend(
        pages(forge, &format!("repos/{repository}/pulls/{number}/reviews"))
            .await?
            .iter()
            .filter_map(review_summary),
    );
    found.sort_by_key(|comment| comment.created_at_ms.unwrap_or(i64::MAX));
    Ok(found)
}

fn posted(prefix: &str, answer: &Value) -> Outcome<(String, Option<String>)> {
    let id = answer["id"]
        .as_u64()
        .ok_or("the source did not return the id of the posted comment")?;
    Ok((format!("{prefix}:{id}"), text(answer, "html_url")))
}

/// Posts one item where it belongs: into its remote thread, on its line, or on the PR.
pub async fn post(
    forge: &dyn Forge,
    pr: &PrIdentity,
    commit: &str,
    item: &Outgoing,
) -> Outcome<(String, Option<String>)> {
    let (repository, number) = (&pr.repository, pr.number);
    let mut body = format!("{}\n\n{}", item.body.trim_end(), marker(&item.id));
    if let Some(parent) = item
        .reply_to
        .as_deref()
        .and_then(|id| id.strip_prefix("review_comment:"))
    {
        let path = format!("repos/{repository}/pulls/{number}/comments/{parent}/replies");
        return posted(
            "review_comment",
            &forge.post(&path, json!({"body": body})).await?,
        );
    }
    if let (None, Some(at)) = (&item.reply_to, &item.at) {
        let side = if at.side == Side::Base {
            "LEFT"
        } else {
            "RIGHT"
        };
        let mut request = json!({
            "body": body,
            "commit_id": commit,
            "path": at.path,
            "line": at.end_line,
            "side": side,
        });
        if at.start_line < at.end_line {
            request["start_line"] = json!(at.start_line);
            request["start_side"] = json!(side);
        }
        let path = format!("repos/{repository}/pulls/{number}/comments");
        match forge.post(&path, request).await {
            Ok(answer) => return posted("review_comment", &answer),
            // GitHub takes line comments only on lines of the PR's own diff.
            Err(error) if error.contains("HTTP 422") => {
                let lines = if at.start_line < at.end_line {
                    format!("{}-{}", at.start_line, at.end_line)
                } else {
                    at.end_line.to_string()
                };
                body = format!("`{}:{lines}`\n\n{body}", at.path);
            }
            Err(error) => return Err(error),
        }
    }
    let path = format!("repos/{repository}/issues/{number}/comments");
    posted(
        "issue_comment",
        &forge.post(&path, json!({"body": body})).await?,
    )
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Failure {
    pub id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct SyncReport {
    /// False when the engine cannot reach a pull request for this review; `reason` says why.
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<String>,
    pub imported: usize,
    pub published: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<Failure>,
}

#[derive(Clone, Copy)]
pub struct Direction {
    pub pull: bool,
    pub push: bool,
}

impl Direction {
    pub const BOTH: Direction = Direction {
        pull: true,
        push: true,
    };
    pub const PULL: Direction = Direction {
        pull: true,
        push: false,
    };
    pub const PUSH: Direction = Direction {
        pull: false,
        push: true,
    };
}

// One poster at a time, so a marked item can never be posted by two syncs at once.
static POSTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn target_of(app: &AppState, root: PathBuf, review: Review) -> Target {
    let stack = super::stacks::get(app, &review.owner_chat_id)
        .await
        .ok()
        .flatten();
    Target {
        app: app.clone(),
        chat: review.owner_chat_id.clone(),
        root,
        review,
        stack,
    }
}

async fn reload(target: &mut Target) -> Outcome<()> {
    target.review = service::load_owned(
        target.root.clone(),
        target.chat.clone(),
        target.review.id.clone(),
    )
    .await
    .map_err(describe)?;
    Ok(())
}

async fn pull(target: &mut Target, forge: &dyn Forge, pr: &PrIdentity) -> Outcome<usize> {
    let fetched = fetch(forge, pr).await?;
    reload(target).await?;
    let mut theirs = Vec::new();
    for comment in fetched {
        let ours = marked(&comment.body)
            .filter(|local| comments::kind_of(&target.review, local).is_ok())
            .map(str::to_string);
        let Some(local) = ours else {
            let body = visible(&comment.body);
            if !body.is_empty() {
                theirs.push(RemoteComment {
                    body,
                    ..comment
                });
            }
            continue;
        };
        // Posted earlier but never recorded: record it now instead of importing it as theirs.
        let recorded = target
            .review
            .remote_links
            .iter()
            .any(|link| link.local_id == local && link.state == RemoteLinkState::Published);
        if !recorded {
            target.review = comments::published(
                target,
                SOURCE,
                &local,
                &comment.id,
                comment.reply_to.clone(),
                comment.url.clone(),
            )
            .await?;
        }
    }
    let imported = comments::import(target, SOURCE.into(), theirs).await?;
    reload(target).await?;
    Ok(imported["imported"].as_u64().unwrap_or(0) as usize)
}

pub async fn sync_with(
    target: &mut Target,
    forge: &dyn Forge,
    pr: &PrIdentity,
    direction: Direction,
) -> SyncReport {
    let mut report = SyncReport {
        pull_request: Some(format!("{}#{}", pr.repository, pr.number)),
        ..Default::default()
    };
    let reached = if direction.pull {
        pull(target, forge, pr)
            .await
            .map(|imported| report.imported = imported)
    } else {
        forge
            .get(&format!("repos/{}/pulls/{}", pr.repository, pr.number))
            .await
            .map(|_| ())
    };
    if let Err(reason) = reached {
        report.reason = Some(reason);
        return report;
    }
    report.available = true;
    if !direction.push {
        return report;
    }
    let _one_poster = POSTING.lock().await;
    let commit = match reload(target)
        .await
        .and_then(|()| candidate(&target.review).cloned())
    {
        Ok(current) => current.head_oid,
        Err(reason) => {
            report.reason = Some(reason);
            return report;
        }
    };
    for item in comments::outgoing(&target.review) {
        let thread = item
            .reply_to
            .clone()
            .filter(|parent| parent.starts_with("review_comment:"));
        let saved = match post(forge, pr, &commit, &item).await {
            Ok((remote_id, url)) => {
                comments::published(target, SOURCE, &item.id, &remote_id, thread, url).await
            }
            Err(reason) => Err(reason),
        };
        match saved {
            Ok(review) => {
                target.review = review;
                report.published += 1;
            }
            Err(reason) => report.failed.push(Failure {
                id: item.id,
                reason,
            }),
        }
    }
    if report.published > 0 {
        super::chat_bridge::invalidate(&target.app, &target.review).await;
    }
    report
}

/// Syncs one review with its pull request. Never an error: the report says what happened.
pub async fn sync(target: &mut Target, direction: Direction) -> SyncReport {
    let Some(pr) = pr_of(&target.review, target.stack.as_ref()) else {
        return SyncReport {
            reason: Some("this review is not tied to a pull request".into()),
            ..Default::default()
        };
    };
    let forge = Gh {
        host: pr.host.clone(),
    };
    sync_with(target, &forge, &pr, direction).await
}

/// Brings in the discussion of every pull request this chat reviews.
pub async fn pull_all(target: &Target) -> Value {
    let mut reviews = vec![target.review.clone()];
    if let Some(stack) = target.stack.clone() {
        let (open, owner) = (target.review.id.clone(), target.chat.clone());
        let others = service::with_store(target.root.clone(), move |store| {
            let mut found = Vec::new();
            for member in &stack.members {
                if let Some(review) = store.stack_review_for_member(&stack.id, &member.id)? {
                    if review.id != open && review.owner_chat_id == owner {
                        found.push(review);
                    }
                }
            }
            Ok(found)
        })
        .await;
        reviews.extend(others.unwrap_or_default());
    }
    let reports = futures::future::join_all(reviews.into_iter().map(|review| async {
        let mut member = Target {
            app: target.app.clone(),
            chat: target.chat.clone(),
            root: target.root.clone(),
            review,
            stack: target.stack.clone(),
        };
        sync(&mut member, Direction::PULL).await
    }))
    .await;
    let imported: usize = reports.iter().map(|report| report.imported).sum();
    match reports.iter().find(|report| !report.available) {
        Some(report) if imported == 0 => json!({"imported": 0, "unavailable": report.reason}),
        _ => json!({"imported": imported}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        answers: Vec<(&'static str, Value)>,
        refuse_lines: bool,
        posts: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl Forge for Fake {
        async fn get(&self, path: &str) -> Outcome<Value> {
            Ok(self
                .answers
                .iter()
                .find(|(prefix, _)| path.starts_with(prefix) && path.ends_with("&page=1"))
                .map_or(json!([]), |(_, answer)| answer.clone()))
        }
        async fn post(&self, path: &str, body: Value) -> Outcome<Value> {
            let mut posts = self.posts.lock().unwrap();
            posts.push((path.to_string(), body));
            if self.refuse_lines && path.ends_with("/pulls/7/comments") {
                return Err("gh: Validation Failed (HTTP 422)".into());
            }
            Ok(json!({"id": 900 + posts.len(), "html_url": "https://example.invalid/c"}))
        }
    }

    fn pr() -> PrIdentity {
        PrIdentity {
            host: "github.com".into(),
            repository: "acme/widgets".into(),
            number: 7,
        }
    }

    fn item(reply_to: Option<&str>, lines: Option<(u32, u32)>) -> Outgoing {
        Outgoing {
            id: "f-1".into(),
            finding: false,
            body: "Text".into(),
            at: lines.map(|(start_line, end_line)| AnchorPosition {
                candidate_id: "c".into(),
                path: "src/a.rs".into(),
                side: Side::Head,
                start_line,
                end_line,
                content_hash: String::new(),
                context_before: vec![],
                context_after: vec![],
            }),
            reply_to: reply_to.map(str::to_string),
        }
    }

    #[test]
    fn pull_requests_are_named_by_github_and_graphite_links_only() {
        for (reference, number) in [
            ("https://github.com/acme/widgets/pull/12", Some(12)),
            (
                "https://github.com/acme/widgets/pull/12/files#diff-1",
                Some(12),
            ),
            (
                "https://app.graphite.com/github/pr/acme/widgets/34/Some-title",
                Some(34),
            ),
            ("https://github.com/acme/acme/pull/5", Some(5)),
            ("https://github.com/acme/widgets/issues/12", None),
            ("https://github.com/acme/widgets", None),
            ("feature/branch", None),
        ] {
            let found = pr_identity(reference);
            assert_eq!(found.as_ref().map(|pr| pr.number), number, "{reference}");
            if let Some(found) = found {
                assert_eq!(found.host, "github.com");
                assert!(found.repository.starts_with("acme/"));
            }
        }
    }

    #[test]
    fn hidden_markers_are_not_part_of_an_imported_comment() {
        assert_eq!(
            visible("**Changes requested**\n\n<!-- graphite-review:abc -->"),
            "**Changes requested**"
        );
        assert_eq!(visible("<!-- only a marker -->"), "");
        assert_eq!(visible("kept <!-- hidden --> too"), "kept  too");
        assert_eq!(visible("kept <!-- never closed"), "kept");
    }

    #[tokio::test]
    async fn fetched_comments_keep_threads_lines_and_order() {
        let forge = Fake {
            answers: vec![
                (
                    "repos/acme/widgets/pulls/7/comments",
                    json!([
                        {"id":2,"user":{"login":"bob"},"body":"Agreed","in_reply_to_id":1,"path":"a.rs","line":4,"created_at":"2026-01-02T00:00:00Z"},
                        {"id":1,"user":{"login":"alice"},"body":"Why?","path":"a.rs","line":4,"start_line":2,"side":"LEFT","html_url":"u1","created_at":"2026-01-01T00:00:00Z"},
                        {"id":3,"user":{"login":"alice"},"body":"Gone","path":"b.rs","line":null,"original_line":9,"created_at":"2026-01-03T00:00:00Z"},
                    ]),
                ),
                (
                    "repos/acme/widgets/issues/7/comments",
                    json!([
                        {"id":10,"user":{"login":"carol"},"body":"LGTM","created_at":"2026-01-04T00:00:00Z"},
                        {"id":11,"user":{"login":"carol"},"body":"* **#7** <a href=\"https://app.graphite.com/github/pr/acme/widgets/7?utm_source=stack-comment-icon\"></a>","created_at":"2026-01-04T00:00:01Z"},
                    ]),
                ),
                (
                    "repos/acme/widgets/pulls/7/reviews",
                    json!([
                        {"id":20,"user":{"login":"dave"},"body":"","submitted_at":"2026-01-05T00:00:00Z"},
                        {"id":21,"user":{"login":"dave"},"body":"Needs tests","state":"CHANGES_REQUESTED","submitted_at":"2026-01-06T00:00:00Z"},
                    ]),
                ),
            ],
            ..Default::default()
        };
        let found = fetch(&forge, &pr()).await.unwrap();
        let ids: Vec<&str> = found.iter().map(|comment| comment.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "review_comment:1",
                "review_comment:2",
                "review_comment:3",
                "issue_comment:10",
                "review:21"
            ]
        );
        assert_eq!((found[0].line, found[0].start_line), (Some(4), Some(2)));
        assert_eq!(found[0].side, Side::Base);
        assert_eq!(found[1].reply_to.as_deref(), Some("review_comment:1"));
        assert_eq!(found[2].path, None);
        assert!(found[2].body.starts_with("_On `b.rs:9`"));
        assert_eq!(found[3].author, "carol");
        assert_eq!(found[4].body, "**Changes requested**\n\nNeeds tests");
    }

    #[tokio::test]
    async fn items_are_posted_into_their_thread_on_their_line_or_on_the_pr() {
        let forge = Fake::default();
        let (id, url) = post(
            &forge,
            &pr(),
            "abc",
            &item(Some("review_comment:1"), Some((2, 4))),
        )
        .await
        .unwrap();
        assert_eq!(
            (id.as_str(), url.as_deref()),
            ("review_comment:901", Some("https://example.invalid/c"))
        );
        post(&forge, &pr(), "abc", &item(None, Some((2, 4))))
            .await
            .unwrap();
        post(
            &forge,
            &pr(),
            "abc",
            &item(Some("issue_comment:10"), Some((2, 4))),
        )
        .await
        .unwrap();
        let posts = forge.posts.lock().unwrap();
        assert_eq!(posts[0].0, "repos/acme/widgets/pulls/7/comments/1/replies");
        assert_eq!(posts[1].0, "repos/acme/widgets/pulls/7/comments");
        assert_eq!(
            (
                &posts[1].1["commit_id"],
                &posts[1].1["line"],
                &posts[1].1["start_line"],
                &posts[1].1["side"]
            ),
            (&json!("abc"), &json!(4), &json!(2), &json!("RIGHT"))
        );
        assert_eq!(posts[2].0, "repos/acme/widgets/issues/7/comments");
        for (_, body) in posts.iter() {
            assert_eq!(marked(body["body"].as_str().unwrap()), Some("f-1"));
        }
    }

    #[tokio::test]
    async fn a_line_outside_the_pr_diff_becomes_a_pr_comment_that_names_the_line() {
        let forge = Fake {
            refuse_lines: true,
            ..Default::default()
        };
        let (id, _) = post(&forge, &pr(), "abc", &item(None, Some((4, 4))))
            .await
            .unwrap();
        assert_eq!(id, "issue_comment:902");
        let posts = forge.posts.lock().unwrap();
        assert!(posts[1].1["body"]
            .as_str()
            .unwrap()
            .starts_with("`src/a.rs:4`\n\nText"));
    }
}
