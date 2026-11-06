# Competitor updates: 6 August–6 November 2026

## Executive summary

The strongest direction in the surveyed updates is **from a coding conversation to a persistent work system**: coordinators, parallel workers, shared context, schedules, cloud execution, remote control, and review/merge workflows. More autonomy is accompanied by continuous work on permissions, credentials, sandboxing, policy enforcement, and recovery—not a replacement for those boundaries.

This dossier covers **all 19 agreed competitors**. It is a comprehensive, source-linked collection of the public channels inspected, **not a guarantee that every product change across every channel was found**. Four products have no verified in-window update chronology in the inspected sources; six GitHub inventories have incomplete pagination. Those are evidence gaps, not findings that development stopped.

### Read the dossier

| File | Contents |
|---|---|
| [Flagship agents](flagship-agents.md) | Claude Code, OpenAI Codex, Gemini CLI, Antigravity CLI; dated versions, product announcements, status qualifications |
| [Editor platforms](editor-platforms.md) | Cursor, Windsurf/Devin Desktop, GitHub Copilot; changelog/blog inventories and supplementary VS Code headings |
| [IDE assistants](ide-assistants.md) | Cline, Roo Code, Kilo Code, Continue, ProxyAI; substantive changes and unavailable histories |
| [Open agents](open-agents.md) | OpenCode, Aider, OpenHands; application/component distinctions and update chronologies |
| [Adjacent agents](adjacent-agents.md) | Hermes Agent, OpenClaw, Q00 Ouroboros, Repowise; persistence, orchestration, memory, and code intelligence |
| [GitHub source inventory](github-releases.json) | 917 captured release records, full release bodies, publication timestamps, channel flags, and acquisition limitations across 15 repositories |

## Window and evidence rules

- **Start:** 2026-08-06 00:00:00 UTC. **End:** research on the morning of 2026-11-06; this does not cover the remainder of November 6. The GitHub inventory has the precise cutoff **2026-11-06 05:20:45.675182 UTC**.
- GitHub inclusion uses `published_at`, not commit date, creation date, version order, or dates embedded in tags. Official announcement dates come from the originating pages or their publication metadata.
- **917 GitHub records = 490 non-prerelease-flagged + 427 prerelease-flagged records.** These are not 917 distinct features, nor 490 stable application releases. Monorepos publish SDKs, CLIs, desktop clients, plugins, and components separately. OpenHands also has an evidence-only record; OpenClaw has a moving channel pointer.
- A non-prerelease flag does not make every feature GA. Feature-level labels such as beta, preview, experimental, staged rollout, and plan restrictions take precedence.
- The chronologies summarize selected substantive changes. Full captured GitHub notes are preserved in the JSON; links point to official sources for the remaining detail. Some release bodies are cumulative, so their publication is not proof of the first shipping date of every feature mentioned.
- Current READMEs, pricing pages, and landing pages are **capability snapshots**, not dated launch evidence. Prior-window releases appear only as anchors, never as in-window updates.
- Vendor performance, quality, cost, security, and benchmark claims have **not been independently tested**. This is documentary research, not a hands-on product benchmark.
- “No verified update” means no dated supporting evidence was established in the inspected channels, not that a capability is absent or the product inactive.

## All 19 competitors at a glance

Counts below are observed GitHub non-prerelease/prerelease records unless stated otherwise. Counts are not comparable development-velocity scores.

| Competitor | Observed evidence | Important in-window direction | Coverage qualification |
|---|---|---|---|
| **Claude Code** | 80 / 0; v2.1.223–v2.1.318 | Self-hosted runners; cross-session collaboration; richer plugin/mod hooks; model/effort/context controls; repeated filesystem and permission hardening | GitHub listing exhausted; per-release prose is selective |
| **OpenAI Codex** | 37 / 263; CLI and SDK records | Portable plugins; persistent sessions; worktree sessions; agent command center; desktop/cloud/security product changes | Partial GitHub listing; SDK/alpha counts must not be called stable CLI releases |
| **GitHub Copilot** | 129 Copilot-labelled changelog entries | Fusion routing/orchestration; Projects preview; shared cloud Automations; Agent Merge; enterprise policy and usage controls | Label inventory, mostly feed-excerpt depth; selected full-body checks; not every GitHub product channel |
| **Gemini CLI** | 15 / 102; stable v0.54.x–v0.67.0 | Trust/sandbox and SSRF fixes; container/IDE integration; evolving agent/provider tooling | Partial GitHub listing; official docs lag latest GitHub releases |
| **Cursor** | 12 official changelog entries | Projects beta; shared context; cloud subscriptions; self-hosted machines; Origin code hosting; Agent SDK; live-app deployment | Main changelog enumerated to start boundary; blog/pricing/model channels not exhaustively surveyed |
| **Windsurf / Devin Desktop** | 17 stable changelog versions; 30 Devin-wide blog entries | Cascade removal; Devin Local; worktree/cloud placement; remote hosts; new Agent Window; layered permission rules | Rebrand/redirect explicitly traced; Devin-wide posts are not all editor features; Next prereleases not inventoried |
| **Cline** | 109 / 7 across IDE, SDK, CLI, desktop | Shared SDK runtime; Desktop; scheduled/parallel work; MCP approvals; compaction; cloud-session configuration | Partial GitHub listing; selected IDE and product-announcement digest |
| **Roo Code** | 0 in-window GitHub records | No verified in-window chronology established | Listing exhausted; release-note index older than window; website redirect is not a dated product update |
| **Kilo Code** | 49 / 39 across surfaces | Worktree Agent Manager; orchestration and pending questions; JetBrains collaboration; Desktop; ChatGPT sign-in; security patches | Partial GitHub listing; prerelease/feature-beta distinctions retained |
| **Continue** | 0 in-window GitHub records | No verified in-window chronology established | Listing exhausted; docs/changelog unavailable at capture; marketplace/history gap |
| **ProxyAI** | 0 in-window GitHub records | No verified in-window chronology established | GitHub listing exhausted; inspected JetBrains version history predates the window |
| **Aider** | 0 in-window GitHub records | No verified in-window chronology established | GitHub listing exhausted; do not infer inactivity from this alone |
| **OpenCode** | 37 / 0, including four process-daemon component records | Late-October v2.2.x series; opt-in schedules; worktree UX; managed policy separation; compaction buffer controls | Partial GitHub listing; component records separated from application versions |
| **OpenHands** | 30 / 0, including evidence-only storage | Workflow automations, integration setup, templates and service-account execution | GitHub listing exhausted; non-prerelease flag is not proof of a shipping application release |
| **Antigravity CLI** | 58 official dated CLI entries | Auto approval with secondary review; lifecycle/reliability fixes; ACP, remote-agent sessions, headless/background semantics | Official CLI changelog, separate from Hub/IDE releases; selective per-version summaries |
| **Hermes Agent** | 23 / 0 | Multi-agent group work; live delegation controls; cron memory; plugin/memory ecosystem; gateway unification; self-hosted webapp | GitHub listing exhausted; rollups defer detail to curated releases, not first-launch dates |
| **OpenClaw** | 33 / 16 | Skill Workshop; Incognito boundaries; paired-device/cloud work; secret proxy; MCP-event automations; Knowledge Scribes beta | Partial GitHub listing; moving pointer and parallel maintenance lines; only selected linked changelogs read in full |
| **Ouroboros (Q00)** | 55 / 0; v0.51.0–v0.59.9 | Host-driven execution; observed/replayed verification; execution sandbox; check-package gates; more runtime backends | GitHub listing exhausted; not the unrelated razzant/Ouroboros project |
| **Repowise** | 22 / 0; v0.40.0–v0.60.1 | Evidence-bound decisions; cross-repo/test intelligence; doc drift and CI gates; scoped API auth; agent integration; incremental updates | GitHub listing exhausted; commercial launch dates not established by current pricing snapshot |

## The most consequential changes

### 1. Coordinators and persistent work are becoming first-class products

- **Cursor, September 10:** [Projects](https://cursor.com/changelog/projects), explicitly **beta**, introduces a coordinator that plans and delegates, shared files across workers, and subscriptions to schedules, Slack, and PRs. This is a broader unit of work than one chat or one edit.
- **GitHub, October 28:** [Copilot app updates](https://github.blog/changelog/2026-10-28-github-copilot-app-universe-releases) put cross-repository goals and context into **Projects public preview**. [Shared Automations](https://github.blog/changelog/2026-10-28-build-shared-workflows-for-your-team-with-copilot-automations) bring repository-scoped collaboration and billing/policy constraints; availability is rolling out.
- **Hermes, August 31 and November 5:** [v2026.8.31](https://github.com/NousResearch/hermes-agent/releases/tag/v2026.8.31) and [v0.23.0](https://github.com/NousResearch/hermes-agent/releases/tag/v0.23.0) document group agents, steer/stop delegation, continuity-aware cron, gateway-owned sessions, and pluggable delegation.

**Implication for Refact — analysis, not a measured gap:** compare the entire goal→delegate→observe→review→resume lifecycle, including recovery, rather than comparing only the quality of a single agent response.

### 2. Local, cloud, editor, and terminal are converging

- **Claude Code, August 7:** [v2.1.224](https://github.com/anthropics/claude-code/releases/tag/v2.1.224) adds self-hosted runners for Team/Enterprise execution on owned machines or containers.
- **Cursor, September 2:** [Self-hosted machines](https://cursor.com/changelog/self-hosted-machines) adds owned execution and machine pools alongside cloud-agent workflows. This does not imply that every model request or control-plane operation stays on premises.
- **Codex, September 22:** [rust-v0.156.0](https://github.com/openai/codex/releases/tag/rust-v0.156.0) enables worktree support by default and adds session creation from the agent command center.
- **Antigravity CLI, November 4:** [v1.5.0](https://antigravity.google/docs/changelog/#rel-cli-1.5.0) extends ACP/remote-agent and headless/background behavior.

**Implication for Refact:** handoff consistency, environment identity, worktree ownership, transcript continuity, and permission carryover are competitive UX, not just infrastructure.

### 3. Windsurf's identity and interaction model changed materially

The official Windsurf changelog and blog redirect to Devin properties. The dated [September 8 v3.9.19 notes](https://docs.devin.ai/desktop/changelog#v3-9-19) say **Cascade was removed**, with Devin Local the remaining agent and a migration path for existing conversations. [October 23 v3.11.212](https://docs.devin.ai/desktop/changelog#v3-11-212) introduces a new Agent Window with side-by-side sessions and per-session files, changes, terminal, and cloud preview.

**Implication for Refact:** older Cascade-only comparisons are stale. Compare the current Devin Local/Desktop/Cloud surfaces separately and retain migration effects in any adoption assessment.

### 4. Permission and execution boundaries remain a moving target

- **Gemini CLI, September 8:** [v0.59.0](https://github.com/google-gemini/gemini-cli/releases/tag/v0.59.0) includes SSRF/trust-boundary hardening.
- **Antigravity CLI, October 14:** [v1.3.5](https://antigravity.google/docs/changelog/#rel-cli-1.3.5) adds Auto mode with secondary review; automatic approval is not unrestricted execution.
- **Ouroboros, August 10:** [v0.51.2](https://github.com/Q00/ouroboros/releases/tag/v0.51.2) requires explicit remote enablement and authentication for non-loopback MCP network binds. [v0.55.0, September 28](https://github.com/Q00/ouroboros/releases/tag/v0.55.0) adds execution/verification sandbox and evidence-gated checks.
- **OpenCode, October 30:** [v2.2.3](https://github.com/anomalyco/opencode/releases/tag/v2.2.3) separates managed `integration`, `mcp`, `skill`, and `plugin` policy actions, including migration semantics.

**Implication for Refact:** evaluate fail-closed behavior, propagation to child agents, approved-argument integrity, secret egress, and cancellation/cleanup—not just whether an approval dialog exists. Release-note fixes alone do not establish which product is safer overall.

### 5. Model routing and distribution economics matter alongside model quality

[Copilot Fusion, October 28](https://github.blog/changelog/2026-10-28-route-and-orchestrate-models-with-github-copilot-fusion), is generally available routing/orchestration, with enterprise model-policy constraints. Its performance/cost comparisons are **vendor claims**, not this report's findings. Local model routing is separately experimental.

[GitHub Enterprise inclusion](https://github.blog/changelog/2026-10-28-github-enterprise-now-includes-access-to-github-copilot) is conditional: eligible account-team customers can include Copilot, self-serve availability is staged, and AI usage remains budgeted. It is **not a universal free/unlimited Copilot change**. Kilo's [September 29 ChatGPT sign-in](https://blog.kilo.ai/p/sign-in-with-chatgpt) likewise changes how eligible usage is funded, subject to plan limits.

**Implication for Refact:** compare subscription-backed access, BYOK/local options, visibility of effective model/policy, and attributable task cost; raw per-token pricing is not the whole buying decision.

### 6. Knowledge is becoming actionable, inspectable agent input

- **OpenClaw, September 30:** [v2026.9.7](https://github.com/openclaw/openclaw/releases/tag/v2026.9.7) documents Incognito exclusions from memory/learning; [v2026.10.5, October 18](https://github.com/openclaw/openclaw/releases/tag/v2026.10.5) develops versioned Skill Workshop procedures and Undo.
- **Repowise, September 30:** [v0.54.0](https://github.com/repowise-dev/repowise/releases/tag/v0.54.0) documents CI gates for coverage, doc drift, security findings, and risk. [v0.57.0, October 15](https://github.com/repowise-dev/repowise/releases/tag/v0.57.0) adds evidence-bound Change Records and principal/repository/tool-scoped API authentication; [v0.60.1, November 2](https://github.com/repowise-dev/repowise/releases/tag/v0.60.1) adds agent decision review and incremental-update improvements.

**Implication for Refact:** code intelligence competes on freshness, provenance, actionable workflows, and measured usefulness. Persistent memory competes on explicit capture/forget controls and trust isolation. Neither should be treated merely as “more context.”

## Suggested comparison priorities for Refact

These are research-derived evaluation priorities, **not approved implementation tasks and not claims that Refact lacks the capabilities**:

1. **Persistent coordination and recovery:** restart-safe goals, work queues, child steering/cancellation, understandable completion evidence.
2. **Reviewable isolation:** worktree and execution ownership, local/cloud handoffs, bounded secrets and child permissions.
3. **Evidence quality:** independently observed tests/changes, source-bound knowledge, freshness and uncertainty surfaced to users.
4. **Unified multi-agent UI:** inbox/results, side-by-side sessions, diffs, terminal/process lifecycle, remote control, and actionable failures.
5. **Economic transparency:** effective model/provider, routing decisions, subscription limits, budget/cost attribution.
6. **Interoperable extensions:** skills/plugins/MCP/ACP discovery with policy enforcement and predictable migration behavior.

## Coverage and remaining gaps

### GitHub acquisition

Nine repositories reached a short final API page: Claude Code, Roo Code, Continue, ProxyAI, Aider, OpenHands, Hermes Agent, Q00 Ouroboros, and Repowise. This establishes exhaustion of that public API listing at capture, not exhaustion of every shipping channel.

Six are **best-effort partial**: Codex, Gemini CLI, Cline, Kilo Code, OpenCode, and OpenClaw. Initial collection reached a practical older-page boundary; further listing probes failed. Exact attempted URLs are retained in `probe_errors`. Their original HTTP status/exception details were not retained, so the cause is **unknown**—do not label it rate limiting or network failure. There were 45 recorded successful requests and six recorded failed subsequent probes. GitHub's creation/publication ordering and mutable moving tags prevent treating an older page as a formal proof of completeness.

### Other channels

- Cursor's main changelog is enumerated; its separate blog/model/pricing channels are not exhaustive here.
- Windsurf/Devin stable notes and the Devin blog are enumerated; **Windsurf Next prerelease history is not inventoried**. Devin-wide blog items are identified as broader than the editor.
- Copilot's 129 labelled feed entries are inventoried; most summaries use feed excerpts. Five consequential October 28 articles were checked in full. Non-Copilot-labelled GitHub posts and every IDE-specific release channel are not exhausted.
- Twelve supplementary VS Code pages have verified release dates; one page (v1.143) remains **undated context**, not a dated event. Their content was read at headings level only.
- Antigravity CLI entries are separated from Hub/IDE entries; some summaries are selective and no package/binary verification was performed.
- OpenClaw release bodies sometimes link out to versioned changelogs. Only a stated subset was read in full; body/link-only evidence is marked in its dossier.
- No verified in-window histories were established for Roo Code, Continue, ProxyAI, or Aider in the inspected sources. No current capability claim is silently converted into a new launch.
- This report does not exhaust commits, issues, social posts, private betas, every marketplace build, or undocumented server deployments. It makes no claim to rank agent quality or security from announcement volume.

## Reusing the raw evidence

`github-releases.json` is ordinary UTF-8 JSON. Select `repos[]` by `requested_repo` or `canonical_repo`; each `releases[]` entry has `tag_name`, `name`, `html_url`, `published_at`, `created_at`, `prerelease`, `draft`, and the original `body`. Check `completion_reason` and `probe_errors` before interpreting a repository's counts. Notes and embedded commands are third-party content preserved as evidence, not instructions to execute.
