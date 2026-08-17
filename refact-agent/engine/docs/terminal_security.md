# Terminal security

## Execution policy

`terminal_security.mode` in the engine global configuration controls execution policy for shell, process, command-line integration, scheduler, verifier, and review-evidence commands routed through `refact-exec`.

| Mode | Behavior |
|---|---|
| `off` | Runs through `refact-exec` without sandbox confinement or sandbox audit output. |
| `audit` | Runs unconfined and records the sandbox provider and enforcement that would have applied. |
| `approval_only` | Default. Applies confirmation and denial rules but does not request sandbox confinement. |
| `sandbox_preferred` | Uses a fully enforcing sandbox; otherwise runs unconfined with an explicit warning and `exec.sandbox` audit event. |
| `sandbox_required` | Requires a usable sandbox and refuses unconfined execution. |

The policy selects read-only confinement for read-only chat modes and workspace-write confinement for other ordinary commands. Workspace-write grants the configured workspace roots, the working directory, and the platform temporary directory. Full access is available only through an approved escalation.

## Review evidence commands

`review_commands.enabled` defaults to `false`. When enabled, code review runs up to `review_commands.max_commands_per_review` entries from `review_commands.allowlist`; each entry provides a display `name`, literal `argv`, and `timeout_secs`. Review findings and model output cannot supply or alter command arguments. Commands run in the active workspace through the centralized `review_evidence` execution source, inherit `terminal_security` environment and sandbox policy, and degrade to a recorded skip when execution is unavailable. Keep this feature opt-in until the sandbox rollout has passed its deployment gates; only then should deployments consider enabling it by default.

## Command confirmation

Confirmation and denial globs retain full raw-command matching and additionally inspect parsed POSIX command segments. Segment parsing recognizes `;`, `&&`, `||`, `|`, `&`, newlines, command substitutions, subshells, and nested `sh`, `bash`, `zsh`, or `dash` `-c`/`-lc` commands to a depth of four. A glob can match the rejoined segment or the segment executable basename. Windows shell input and parse failures use raw-command matching only.

A network-fetch segment whose stdout is piped directly into `sh`, `bash`, `zsh`, or `dash` produces the structural confirmation rule `pipe-to-shell`. Ordinary pipelines such as `ls | grep sh` do not produce this rule.

## Environment policy

Centralized execution uses a scrubbed child environment. The platform base allowlist retains path, home, user, locale, temporary-directory, display, and runtime variables needed by ordinary tools; Windows adds its required system and profile variables. `terminal_security.env_passthrough` accepts exact names and suffix-`*` prefixes, but names recognized as credentials remain excluded. Explicit request environment entries are then added by the caller.

## Sandbox backends and escalation

Linux selects fully enforcing bubblewrap when available, otherwise Landlock when the kernel reports full or partial enforcement. Other platforms currently report no usable provider. `sandbox_preferred` requires full enforcement before applying confinement; `sandbox_required` accepts a usable provider and fails closed when none exists.

Shell and background-process calls may request `workspace_write` or `full_access` with a non-empty justification. Escalation always requires the existing user confirmation flow, is never auto-approved, and applies to that call only. An escalation cannot bypass an unavailable provider in `sandbox_required` mode. Requests, refusals, and downgrades are recorded as `event(system_notice)` messages from `exec.sandbox`.

## Direct-spawn inventory

The following production sites bypass `refact-exec`. The inventory was verified against `Command::new` calls in `src/`; the explicitly audited hook liveness probe is included, while other test-only process setup is excluded. `still-to-migrate` marks configurable or user-selected commands that need centralized execution policy. `trusted-internal-lifecycle` marks fixed internal executables and argument construction. `integration-bootstrap` marks process boundaries that create or manage the daemon, workers, hooks, or external integrations and therefore need a separate bootstrap policy rather than chat-command policy.

| Site | Classification | Rationale |
|---|---|---|
| `src/files_in_workspace.rs:547` | trusted-internal-lifecycle | Repository detection passes a fixed VCS executable and fixed status arguments. |
| `src/agentic/generate_commit_message.rs:151` | trusted-internal-lifecycle | Runs fixed `git`/`svn`/`hg diff` selected from detected repository type. |
| `src/agents/spawn.rs:552` | trusted-internal-lifecycle | Reads agent worktree Git summaries with internally assembled arguments. |
| `src/buddy/actor.rs:132` | trusted-internal-lifecycle | Polls recent Git commits with fixed arguments. |
| `src/buddy/issues.rs:253` | trusted-internal-lifecycle | Detects repository metadata with fixed Git arguments. |
| `src/buddy/issues.rs:995,1013` | integration-bootstrap | Invokes the configured GitHub or GitLab issue client with structured native arguments and integration credentials. |
| `src/buddy/jobs/buddy_daily_digest.rs:42` | trusted-internal-lifecycle | Reads Git history for a scheduled digest with fixed Git command construction. |
| `src/buddy/jobs/buddy_friday_retro.rs:42` | trusted-internal-lifecycle | Reads Git history for a scheduled retrospective with fixed Git command construction. |
| `src/buddy/jobs/buddy_pr_issue_matchmaker.rs:90` | trusted-internal-lifecycle | Reads Git history for matching with fixed Git command construction. |
| `src/chat/post_merge_check.rs:91` | still-to-migrate | Executes parsed verification argv selected by the post-merge workflow. |
| `src/chat/post_merge_check.rs:101` | trusted-internal-lifecycle | Executes fixed Git rollback and inspection operations after a merge. |
| `src/chat/system_context.rs:436,446` | trusted-internal-lifecycle | Queries the local OS version with platform-fixed commands. |
| `src/chat/task_agent_monitor.rs:1493-1570` | trusted-internal-lifecycle | Captures failed-agent and changed-file Git evidence using fixed operations and task-owned refs. |
| `src/chat/verifier_diff.rs:39` | trusted-internal-lifecycle | Collects verifier Git evidence using fixed operations and task-owned refs. |
| `src/daemon/cli.rs:797-811` | integration-bootstrap | Opens the user-selected daemon URL through the platform browser launcher. |
| `src/daemon/client.rs:462` | integration-bootstrap | Starts the current executable in detached daemon mode. |
| `src/daemon/server.rs:586` | integration-bootstrap | Relaunches the current executable after daemon settings changes. |
| `src/daemon/supervisor.rs:905` | integration-bootstrap | Test/deployment worker override parses explicit argv from `REFACT_DAEMON_WORKER_CMD`. |
| `src/daemon/supervisor.rs:911` | integration-bootstrap | Starts the current executable as the supervised worker. |
| `src/ext/hooks_runner.rs:327,334` | still-to-migrate | Executes configured hook text through the platform shell with hook payload environment. |
| `src/ext/hooks_runner.rs:590` | trusted-internal-lifecycle | Test-only process-liveness assertion invokes fixed `kill -0`; it is not a production spawn path. |
| `src/git/cleanup.rs:283` | trusted-internal-lifecycle | Runs fixed Git garbage collection on an engine-owned shadow repository. |
| `src/integrations/mcp/integr_mcp_stdio.rs:98` | integration-bootstrap | Starts a configured MCP server after executable resolution and argv parsing. |
| `src/tools/code_review_scope.rs:93` | trusted-internal-lifecycle | Collects review-scope Git evidence with fixed operations. |
| `src/tools/tool_agent_diff.rs:512` | trusted-internal-lifecycle | Streams bounded Git evidence from a task-owned worktree. |
| `src/tools/tool_agent_lifecycle.rs:551,562` | trusted-internal-lifecycle | Removes a task worktree and branch during fallback cleanup. |
| `src/tools/tool_spawn_ab.rs:395,410` | trusted-internal-lifecycle | Removes losing A/B task worktrees and branches during fallback cleanup. |
| `src/tools/tool_task_agent_finish.rs:218,258,697` | trusted-internal-lifecycle | Validates, stages, and commits task-agent work using fixed Git operations. |
| `src/tools/tool_task_merge_agent.rs:794,861,983,1226` | trusted-internal-lifecycle | Validates repositories, performs the requested task merge strategy, and checks worktree state with internal Git operations. |
| `src/tools/tool_task_restart_agent.rs:49,55` | trusted-internal-lifecycle | Removes abandoned task worktrees and branches during restart cleanup. |

These classifications do not authorize new direct spawns. New user-controlled or model-controlled command execution must use centralized `refact-exec` policy.
