# Open-source agents: OpenCode, Aider, OpenHands

## Evidence and scope

Window: **2026-08-06 00:00:00 UTC–2026-11-06 05:20:45.675182 UTC**, using GitHub `published_at`. This chapter is a **GitHub-release-based chronology**, not an exhaustive blog, marketplace, commit, or deployment survey. Full captured bodies and all record URLs are retained in [github-releases.json](github-releases.json). The per-version excerpts below are selective: up to eight substantive note lines per captured non-prerelease record. They do not assert that every change is included or that a cumulative note dates the first launch of each capability.

GitHub non-prerelease flags describe records, not necessarily generally available applications. OpenCode has a separately published process-daemon component; OpenHands includes evidence-storage records. Release notes, including security and performance claims, are maintainer statements and were not tested hands-on.

| Product | Non-prerelease / prerelease records | Listing coverage |
|---|---:|---|
| anomalyco/opencode | 37 / 0 | 2 pages; Partial: practical boundary reached; subsequent probe failed, cause unknown |
| Aider-AI/aider | 0 / 0 | 1 pages; Listing exhausted at capture |
| OpenHands/OpenHands | 30 / 0 | 2 pages; Listing exhausted at capture |

## 1. OpenCode (Anomaly)

**Identity:** [anomalyco/opencode](https://github.com/anomalyco/opencode), not the older similarly named Go project. Captured application versions run from v1.18.15 through v2.2.5; process-daemon-v0.5.0 is a component release, not application v0.5.0. The collection is partial, and version gaps are not filled by inference.

**Key documented changes:** [v2.2.1, October 29](https://github.com/anomalyco/opencode/releases/tag/v2.2.1) adds opt-in scheduled prompts (`experimental.schedules`) and location/worktree actions; [v2.2.3, October 30](https://github.com/anomalyco/opencode/releases/tag/v2.2.3) separates managed integration, MCP, skill and plugin policy actions; [v2.2.5, October 30](https://github.com/anomalyco/opencode/releases/tag/v2.2.5) adds a percentage compaction buffer, worktree shortcuts and large-MCP-message handling. These feature dates are release-publication dates, not proof of first appearance on development branches.

### Captured release chronology

- **2026-08-07 — [v1.18.15](https://github.com/anomalyco/opencode/releases/tag/v1.18.15) — non-prerelease record.**
  - Chronological message ordering now stays correct even when imported or legacy message IDs are out of order.
  - Revert and fork actions now use real message chronology instead of message ID ordering.
  - Truncation cleanup now removes stale files by file timestamp more reliably.
  - Repeated compaction now keeps earlier tool-call history in summaries instead of dropping orphaned results.
  - Blob-based attachments now load correctly in the web UI. (@rexdotsh)
  - Added much broader locale coverage across the desktop app.
  - Export full session transcripts as JSON from the UI.
  - Session timelines now respect chronological message boundaries more reliably.
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-08-10 — [v1.18.16](https://github.com/anomalyco/opencode/releases/tag/v1.18.16) — non-prerelease record.**
  - Ignore unknown top-level config fields instead of failing config parsing
  - Register projects opened from Home so they are available to the rest of the app
  - Open the project menu with a right-click in Home
  - Fall back to listing and matching local directories when the project picker server does not support search
  - Use 词元 instead of 令牌 for token-related labels in Simplified Chinese (@Speechlessmanbilibili)
  - Keep the macOS app running after the last window closes and reopen a window when the app is activated
  - @Speechlessmanbilibili:
  - fix(i18n): use 词元 instead of 令牌 for token in zh locale (#40977)

- **2026-08-12 — [v1.18.17](https://github.com/anomalyco/opencode/releases/tag/v1.18.17) — non-prerelease record.**
  - Made session compaction keep complete recent turns and produce clearer summaries for smaller models.
  - Added MERGE Gateway reasoning variants so those model options work correctly. (@MatthewFeroz)
  - Capped automatic session retries and added jitter to reduce repeated retry storms.
  - Enabled PDF attachments for GitHub Copilot models that advertise PDF vision support. (@stevenao)
  - Applied the correct sampling defaults to DeepSeek V4 Flash on supported providers.
  - Routed all Muse family models to the correct Meta system prompt. (@mreso)
  - Updated Chinese developer terminology to use more widely recognized translations.
  - Used the server's current default model instead of stale local config when choosing the default model.
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-08-13 — [v1.18.18](https://github.com/anomalyco/opencode/releases/tag/v1.18.18) — non-prerelease record.**
  - Select the Kimi system prompt correctly for official Moonshot and Kimi providers
  - Fix xhigh reasoning effort for xai models

- **2026-08-20 — [v1.18.19](https://github.com/anomalyco/opencode/releases/tag/v1.18.19) — non-prerelease record.**
  - Added native OpenAI and Anthropic passthroughs for Cloudflare AI Gateway models.
  - Matched Codex rate limits more closely to ChatGPT subscription limits. (@GameOn223)
  - Removed built-in Qwen sampling defaults that could send unsupported settings.
  - Properly show authenticated providers in `/connect`. (@OpeOginni)
  - Ignore malformed model pricing instead of breaking usage cost calculation.
  - Fall back when OpenAI websocket requests exceed message size limits.
  - Forward ChatGPT workspace compute residency to Codex requests.
  - Updated the default Console URL used by account connection flows.
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-08-21 — [v1.18.20](https://github.com/anomalyco/opencode/releases/tag/v1.18.20) — non-prerelease record.**
  - Surface failed subagent tool calls with a resumable `task_id`.
  - Retry provider responses that end with `finish_reason: network_error`.
  - Retry more network error variants, including `network-error` and `network_error`.
  - Surface resumable subagent failures instead of returning an empty result.
  - Preserve Cerebras `max_completion_tokens` without applying an extra output cap.
  - Answer permission requests triggered by subagents during `opencode run`.
  - Retry xAI capacity and temporary unavailability stream errors.

- **2026-08-21 — [v1.18.21](https://github.com/anomalyco/opencode/releases/tag/v1.18.21) — non-prerelease record.**
  - Continue responses when a model reports an unknown finish reason instead of stopping early
  - Route Vertex AI `eu` and `us` multi-region Gemini requests through REP endpoints
  - Keep file search results visible while the next search is loading
  - Register the archive session command in both desktop layouts (@NathanTCode)
  - @NathanTCode:
  - fix(app): register archive session command in both layouts (#41741)

- **2026-08-24 — [v1.18.22](https://github.com/anomalyco/opencode/releases/tag/v1.18.22) — non-prerelease record.**
  - Removed outdated OpenCode Go first-month discount messaging and pricing.
  - Fixed OpenCode device login links when servers return relative verification URLs or use a base path.
  - Fixed `textVerbosity` being sent to OpenAI-compatible providers that do not support it. (@joelstucki-taulia)
  - Updated the Amazon Bedrock provider for compatibility fixes.
  - Keep model provider headers visible while scrolling the model picker.
  - @joelstucki-taulia:
  - fix(provider): guard textVerbosity injection for @ai-sdk/openai-compatible providers (#43915)

- **2026-08-25 — [v1.18.23](https://github.com/anomalyco/opencode/releases/tag/v1.18.23) — non-prerelease record.**
  - Fixed Cloudflare AI Gateway routing for third-party providers so non-Workers models work through the gateway's REST API. (@superhighfives)
  - Fixed Anthropic models through Cloudflare AI Gateway by converting dotted model IDs like `claude-haiku-4.5` to the dashed slug Anthropic expects. (@superhighfives)
  - Fixed parent session IDs being sent in request headers for session-aware providers.
  - Fixed GitHub auth for immutable OIDC subject tokens.
  - @superhighfives:
  - fix(provider): send Anthropic's dashed native slug through the AI Gateway (#44281)
  - fix(provider): route non-native Cloudflare AI Gateway providers via the REST API (#44828)

- **2026-08-28 — [v1.18.24](https://github.com/anomalyco/opencode/releases/tag/v1.18.24) — non-prerelease record.**
  - Bedrock reasoning responses no longer get cached into unreplayable empty messages.
  - Azure providers can now sign in with Microsoft Entra ID through the Azure CLI instead of requiring an API key.
  - V1 now reads supported V2 config fields so newer config files keep working in more mixed setups.
  - Archived sessions disappear from the Home list immediately. (@NathanTCode)
  - @NathanTCode:
  - fix(app): drop archived sessions from home list right away (#44905)
  - @OpeOginni:
  - fix(ui): restore focus in stacked dialogs (#44928)
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-08-28 — [v1.18.25](https://github.com/anomalyco/opencode/releases/tag/v1.18.25) — non-prerelease record.**
  - Fixed Azure authentication so Azure CLI sign-in works without requiring Bun.

- **2026-09-01 — [v1.18.26](https://github.com/anomalyco/opencode/releases/tag/v1.18.26) — non-prerelease record.**
  - Claude 5 sessions now tolerate stale thinking blocks instead of failing after prompt or tool changes.
  - Bedrock GPT-5.6 models now accept `none` reasoning effort.
  - Bedrock reasoning and replay handling is more reliable. (@pengzh1)
  - Tool call timing now stays accurate when tools update their metadata while still running. (@bartlettroscoe)
  - `apply_patch` no longer emits an empty move path in permission metadata. (@altendky)
  - Azure CLI sign-in now asks for the resource name directly instead of querying Azure management APIs.
  - Session renames now save reliably from the title editor and tab context menu.
  - @altendky:
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-09-02 — [v1.18.27](https://github.com/anomalyco/opencode/releases/tag/v1.18.27) — non-prerelease record.**
  - Default provider header timeouts to five minutes so slow model startups fail less often.
  - Default streamed chunk timeouts to five minutes, with `false` supported to disable them.
  - Let Anthropic `thinking.blockBinding` opt out via config when you need to keep the provider default. (@dkindlund)
  - Limit Anthropic thinking block binding to Claude 5.1+ models so older deployments do not reject requests.
  - Avoid unhandled errors when canceling timed-out SSE reads. (@AlexanderWillner)
  - @AlexanderWillner:
  - fix(provider): handle SSE reader cancel rejections (#44944)
  - @dkindlund:
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-09-04 — [v1.18.28](https://github.com/anomalyco/opencode/releases/tag/v1.18.28) — non-prerelease record.**
  - Send the session ID as GitHub Copilot's interaction header to improve request tracking across a session.
  - Use the desktop client ID during OpenCode account device authentication.
  - Increase the open-in app icon size for better visibility.
  - @MVS-source:

- **2026-09-04 — [v1.18.29](https://github.com/anomalyco/opencode/releases/tag/v1.18.29) — non-prerelease record.**
  - Allow Codex OAuth model filtering to recognize integer GPT versions like `gpt-6`
  - Fixes issue of gpt-6-astra not showing up for openai subscription users
  - @Peter267:
  - @heimoshuiyu:
  - feat(console): add quota reset support action (#47313)

- **2026-09-09 — [v1.18.30](https://github.com/anomalyco/opencode/releases/tag/v1.18.30) — non-prerelease record.**
  - Added the Astra system prompt for GPT-6 models.
  - Preserved Bedrock DeepSeek model IDs, including ARN-based IDs, so they resolve correctly. (@YeEmrick)
  - Updated the Azure provider SDK to pick up compatibility fixes.
  - Updated the OpenAI provider SDK to pick up compatibility fixes.
  - Added reasoning effort variants for supported GitLab GPT and Claude models. (@far-ouq)
  - @heimoshuiyu:
  - fix(console): openai usage normalization and tier threshold config (#47342)
  - @far-ouq:
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-09-14 — [v1.18.31](https://github.com/anomalyco/opencode/releases/tag/v1.18.31) — non-prerelease record.**
  - Restored ACP session model, effort, mode, and reasoning chunk boundaries when loading, resuming, or forking sessions. (@JacobNWolf)
  - Show remote config authentication errors during startup and exit with a failure status.
  - Request summarized adaptive thinking for GitHub Copilot models.
  - @heimoshuiyu:
  - feat(console): add batch workspace block endpoints (#48491)
  - @JacobNWolf:
  - fix(acp): restore session options and reasoning boundaries (#48225)
  - @R-Taneja:
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-09-21 — [v1.18.32](https://github.com/anomalyco/opencode/releases/tag/v1.18.32) — non-prerelease record.**
  - Fixed Bedrock image attachments so they are only hoisted for Claude, Nova, and Llama 4 models.
  - Fixed Together AI streaming usage reporting.
  - @dc85:
  - feat: add Grok 4.7 to Zen and Go (#50288)

- **2026-09-28 — [v1.18.33](https://github.com/anomalyco/opencode/releases/tag/v1.18.33) — non-prerelease record.**
  - Cloudflare AI Gateway models now honor provider response and stream timeouts. (@danlapid)
  - MCP browser launch failures are now reported when the launcher exits immediately.
  - Debug configuration output now redacts credentials and sensitive headers.
  - Gemini thinking defaults and effort options now match the supported controls across model generations. (@markmcd)
  - @dc85:
  - @vglafirov:
  - maint(gitlab): bump gitlab-ai-provider to 6.16.0 (#50742)
  - @markmcd:
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-09-30 — [v1.18.34](https://github.com/anomalyco/opencode/releases/tag/v1.18.34) — non-prerelease record.**
  - Send namespaced session and parent-session identity headers with model requests.
  - Re-sign locally compiled macOS binaries so they run reliably on macOS 27+. (@ryangamerdev)
  - Sign macOS CLI release binaries with a Developer ID.
  - @dc85:
  - @metal-huang:
  - fix(tui): use path.sep for plugin name extraction in /status dialog (#52328)
  - @ryangamerdev:
  - fix(opencode): ad-hoc re-sign darwin binaries after local compile (#52183)

- **2026-10-06 — [v1.18.35](https://github.com/anomalyco/opencode/releases/tag/v1.18.35) — non-prerelease record.**
  - Added canonical redirects and JSON and Markdown data formats for agent-readable stats.
  - xAI tool results now include supported images, while unsupported image formats are skipped. (@Jaaneek)
  - @dc85:
  - @jm0ney337:
  - @Jaaneek:
  - fix(opencode): bump @ai-sdk/xai to 3.0.139 so tool-result images reach xAI (#53549)

- **2026-10-11 — [v1.19.0](https://github.com/anomalyco/opencode/releases/tag/v1.19.0) — non-prerelease record.**
  - Run `opencode upgrade --major` to upgrade to OpenCode 2.
  - Allow the OpenAI `ultrafast` service tier.
  - Validate PTY connection origins and tickets before checking whether a PTY exists.
  - Use `/update` to install OpenCode updates or upgrade to OpenCode 2; updates are no longer installed automatically.
  - @dc85:
  - fix(stats): attribute exo usage to an unknown provider (#53621)

- **2026-10-15 — [v1.19.1](https://github.com/anomalyco/opencode/releases/tag/v1.19.1) — non-prerelease record.**
  - Session sharing has been discontinued. Existing share links and session-sharing commands are no longer available, and importing sessions from share URLs is unsupported.

- **2026-10-16 — [v1.19.2](https://github.com/anomalyco/opencode/releases/tag/v1.19.2) — non-prerelease record.**
  - OpenCode 1 now stays on the 1.x release line: update checks use only the npm `opencode-ai` latest version, so 1.x installs are never offered a 2.x build as a regular update.
  - Upgrading to OpenCode 2 remains a separate, explicit choice through the 2.0 offer in `/update` or `opencode upgrade --major`.
  - Fix upgrades for installs managed by yarn.
  - Skip 1.x updates for Homebrew Core installs, which now track OpenCode 2, and check tap and Chocolatey availability before offering an update.
  - The install script refuses to install 2.x versions.

- **2026-10-16 — [v1.19.3](https://github.com/anomalyco/opencode/releases/tag/v1.19.3) — non-prerelease record.**
  - Configure model reasoning options to generate reasoning variants, replacing catalog variants while preserving your configured variant overrides.

- **2026-10-22 — [v1.19.4](https://github.com/anomalyco/opencode/releases/tag/v1.19.4) — non-prerelease record.**
  - Upgraded the bundled Bun runtime to 1.4.4, which includes [security fixes](https://bun.com/blog/bun-v1.4.4#security) for Brotli, TLS, HTTP/1.1, HTTP/2, HTTP/3, WebSocket, and `fetch`, and updates root certificates to NSS 3.131.1.

- **2026-10-27 — [v2.2.0](https://github.com/anomalyco/opencode/releases/tag/v2.2.0) — non-prerelease record.**
  - Create, install and share [Desktop plugins](https://opencode.ai/v2/docs/build/plugins/gui) (experimental) ([#55246](https://github.com/anomalyco/opencode/pull/55246), [#59469](https://github.com/anomalyco/opencode/pull/59469), [#59489](https://github.com/anomalyco/opencode/pull/59489))
  - Refreshed the desktop app experience ([#56898](https://github.com/anomalyco/opencode/pull/56898), [#58865](https://github.com/anomalyco/opencode/pull/58865), [#55838](https://github.com/anomalyco/opencode/pull/55838), [#57962](https://github.com/anomalyco/opencode/pull/57962))
  - Add experimental image and decision generation APIs to plugins and sdk, allowing image generation or decision model (think typesafe, clef, etc) workflows. ([#59190](https://github.com/anomalyco/opencode/pull/59190), [#59380](https://github.com/anomalyco/opencode/pull/59380), [#58758](https://github.com/anomalyco/opencode/pull/58758))
  - Plugins can now attach and update keyed session instructions with ctx.session.instructions.entry.put/list/remove. Instead of editing the system prompt and busting the prompt cache, each change is added as a small update at the next step. The entries also survive compaction and reverts, and they don't start an extra model turn. ([#56550](https://github.com/anomalyco/opencode/pull/56550))
  - Restored the Todo tool in desktop via opt-in ([#59092](https://github.com/anomalyco/opencode/pull/59092))
  - Add configurable prompt cache TTLs per provider/model ([#59157](https://github.com/anomalyco/opencode/pull/59157))
  - Add experimental image and decision generation API endpoints. ([#59190](https://github.com/anomalyco/opencode/pull/59190))
  - Add image and decision entrypoints for catalog providers. ([#59380](https://github.com/anomalyco/opencode/pull/59380))
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-10-28 — [v1.19.5](https://github.com/anomalyco/opencode/releases/tag/v1.19.5) — non-prerelease record.**
  - Failed TUI worker requests now return an error instead of leaving the caller hanging. (@Nowaker)
  - @Nowaker:
  - fix(opencode): answer a failed TUI worker request instead of hanging (#54936)

- **2026-10-28 — [process-daemon-v0.3.2](https://github.com/anomalyco/opencode/releases/tag/process-daemon-v0.3.2) — component release.**
  - opencode-pty 0.3.2 (protocol 7) from packages/process-daemon, published to npm as @opencode/pty@0.3.2.

- **2026-10-29 — [v2.2.1](https://github.com/anomalyco/opencode/releases/tag/v2.2.1) — non-prerelease record.**
  - **Keep Working Toward a Goal**: Run `/goal <objective>` in the TUI or Desktop to let the session continue until a reviewer confirms the objective is met. Use `/goal pause`, `/goal resume`, or `/goal clear` to control it. ([#59437](https://github.com/anomalyco/opencode/pull/59437))
  - **Generate Images**: Enable `experimental.image_generate` to let agents create or edit images with the new `image_generate` tool. You can optionally set its default model with `experimental.image_generate.model`. ([#59939](https://github.com/anomalyco/opencode/pull/59939), [#60096](https://github.com/anomalyco/opencode/pull/60096))
  - **Inspect Background Shells**: Click a running shell in the Desktop activity pill to see its live output and stop it without leaving the menu. ([#59496](https://github.com/anomalyco/opencode/pull/59496))
  - **New Default Model Picker**: Search favorites, recent models, providers, and variants from the redesigned model picker, which is now the default in Desktop. ([#59997](https://github.com/anomalyco/opencode/pull/59997))
  - The fal, Black Forest Labs, Replicate, and Stability providers are removed, along with their `./providers/fal`, `./providers/black-forest-labs`, `./providers/replicate`, and `./providers/stability` entry points. ([#59977](https://github.com/anomalyco/opencode/pull/59977))
  - Five legacy entry points are removed. Import the nested paths instead, which 2.2.0 already exports: `./providers/amazon-bedrock-mantle` → `./providers/amazon-bedrock/mantle`, and `./providers/google-vertex-chat`, `-interactions`, `-mistral`, and `-responses` → `./providers/google-vertex/chat`, `/interactions`, `/mistral`, and `/responses`. ([#59506](https://github.com/anomalyco/opencode/pull/59506))
  - Each provider's `configure()` input type is now named `Config`, for example `Groq.LanguageModelOptions` → `Groq.Config`, and `model()` settings are optional when every setting is optional. ([#59765](https://github.com/anomalyco/opencode/pull/59765))
  - Add `/goal <objective>` with pause, resume, and clear controls, plus automatic progress review until the objective is met. ([#59437](https://github.com/anomalyco/opencode/pull/59437))
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-10-29 — [process-daemon-v0.3.3](https://github.com/anomalyco/opencode/releases/tag/process-daemon-v0.3.3) — component release.**
  - opencode-pty 0.3.3 (protocol 7) from packages/process-daemon, published to npm as @opencode/pty@0.3.3.

- **2026-10-29 — [v2.2.2](https://github.com/anomalyco/opencode/releases/tag/v2.2.2) — non-prerelease record.**
  - **Image Generation by Default**: When a connected provider supports image models, agents can now use `experimental_image_generate` without additional configuration. Set its preferred model through `experimental.image_generate.model`. ([#60258](https://github.com/anomalyco/opencode/pull/60258))
  - **Faster Worktree Selection**: On the new-session screen, click the current Git branch to open that project’s “Run session in” worktree picker directly. ([#60263](https://github.com/anomalyco/opencode/pull/60263))
  - **Unified Quotes and Annotations**: Timeline quotes, terminal quotes, and browser annotations now share a compact optional-comment editor, while source highlights remain attached to local drafts. ([#60164](https://github.com/anomalyco/opencode/pull/60164))
  - Enabled `experimental_image_generate` by default when an image-capable provider is connected. ([#60258](https://github.com/anomalyco/opencode/pull/60258))
  - Recorded the start time of each session’s latest execution.
  - Rejected persistent terminal requests whose working directory no longer exists.
  - Preserved media when sessions are forked or imported.
  - Associated image-generation notices with the correct background job.
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-10-29 — [process-daemon-v0.4.0](https://github.com/anomalyco/opencode/releases/tag/process-daemon-v0.4.0) — component release.**
  - opencode-pty 0.4.0 (protocol 7) from packages/process-daemon, published to npm as @opencode/pty@0.4.0.

- **2026-10-30 — [v2.2.3](https://github.com/anomalyco/opencode/releases/tag/v2.2.3) — non-prerelease record.**
  - **Use Claude Code in OpenCode**: Install Claude Code, run `claude auth login`, then select **Claude Agent** in `/connect` to use your Claude Code login and models. ([#58782](https://github.com/anomalyco/opencode/pull/58782))
  - **Faster Long Sessions in Desktop**: Streaming replies and long conversations now do much less rendering work. ([#60489](https://github.com/anomalyco/opencode/pull/60489), [#60521](https://github.com/anomalyco/opencode/pull/60521), [#60513](https://github.com/anomalyco/opencode/pull/60513), [#60481](https://github.com/anomalyco/opencode/pull/60481))
  - **Desktop Stability Fixes**: Fixes side-panel tabs, repeated "Session not found" toasts, and **Open file** over HTTP. ([#60430](https://github.com/anomalyco/opencode/pull/60430), [#60436](https://github.com/anomalyco/opencode/pull/60436), [#60459](https://github.com/anomalyco/opencode/pull/60459), [#60462](https://github.com/anomalyco/opencode/pull/60462))
  - Added the built-in **Claude Agent** provider. Install Claude Code, run `claude auth login`, and select **Claude Agent** in `/connect`; `OPENCODE_CLAUDE_PATH` can override the Claude executable. Thanks [@Tarquinen](https://github.com/Tarquinen). ([#58782](https://github.com/anomalyco/opencode/pull/58782))
  - Added Code Mode JavaScript checks to experimental schedules, including statically authorized tool calls, persistent state, and custom wake messages. ([#60140](https://github.com/anomalyco/opencode/pull/60140))
  - Added separate managed-policy actions for connecting to integrations and using integrations, MCP servers, skills, and plugins. ([#60315](https://github.com/anomalyco/opencode/pull/60315))
  - Preserved plugin-provided model variants and delayed dynamic provider fallback until external SDK hooks have run. ([#60097](https://github.com/anomalyco/opencode/pull/60097), [#60380](https://github.com/anomalyco/opencode/pull/60380))
  - Preserved permission reasons and feedback in MCP failures, and made Code Mode failures match direct tool-call failures. ([#60102](https://github.com/anomalyco/opencode/pull/60102), [#60109](https://github.com/anomalyco/opencode/pull/60109))
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-10-30 — [v2.2.4](https://github.com/anomalyco/opencode/releases/tag/v2.2.4) — non-prerelease record.**
  - **Persistent Desktop Terminals**: Desktop terminals now belong to the session, survive server restarts with their scrollback, and stay synchronized with the TUI and other windows. Renamed terminals also keep their names across restarts. ([#60346](https://github.com/anomalyco/opencode/pull/60346))
  - **Publish Interactive Artifacts**: Enable `"experimental": { "artifacts": true }` in `opencode.json` to let agents publish, update, and serve HTML or static artifacts through the built-in local provider or plugin-provided storage. ([#60351](https://github.com/anomalyco/opencode/pull/60351))
  - **Guided Claude Agent Setup**: Select **Claude Agent** from the provider picker to copy `claude auth login`, install Claude Code, and verify the connection before continuing to model selection. ([#60603](https://github.com/anomalyco/opencode/pull/60603))
  - **Choose a Base Branch Quickly**: While creating a worktree, open **Choose base branch** with `⌘⌥.` on macOS or `Ctrl+Alt+.` on Windows and Linux. The command is also available in the command palette and Settings › Shortcuts. ([#60579](https://github.com/anomalyco/opencode/pull/60579))
  - Added opt-in artifacts with `artifact_create`, `artifact_read`, `artifact_update`, and `artifact_delete`, including a built-in local static-file provider. ([#60351](https://github.com/anomalyco/opencode/pull/60351))
  - Made enabled schedules run under `opencode serve` and coordinated multiple server processes so a schedule fires only once. Schedules remain opt-in through `experimental.schedules`. ([#60478](https://github.com/anomalyco/opencode/pull/60478))
  - Raised OpenCode Go limits for DeepSeek V4.1 Flash. ([#60625](https://github.com/anomalyco/opencode/pull/60625))
  - Allowed user, project, and remote configuration to override Console-supplied provider names, limits, variants, settings, and headers. ([#60597](https://github.com/anomalyco/opencode/issues/60597))
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-10-30 — [v2.2.5](https://github.com/anomalyco/opencode/releases/tag/v2.2.5) — non-prerelease record.**
  - **Worktree and Shell Mode Shortcuts**: Press `⌘⌥O` on macOS or `Ctrl+Alt+O` on Windows and Linux to choose a worktree, and `⌘⌥B` or `Ctrl+Alt+B` to choose its base branch. `⌘⇧X` or `Ctrl+Shift+X` now toggles shell mode in the composer. ([#60713](https://github.com/anomalyco/opencode/pull/60713))
  - **Send Quotes From the Comment Popup**: Hold `⌘` or `Ctrl` in the quote popup to turn its checkmark into send, or press `⌘Enter` or `Ctrl+Enter`, to send just that quote and comment while leaving the rest of the composer untouched. ([#60640](https://github.com/anomalyco/opencode/pull/60640))
  - **Percentage Compaction Buffer**: Set `"compaction": { "buffer": "20%" }` in `opencode.json` to keep a share of each model's context window free, so one setting fits models with small and large windows. Token counts still work. ([#60647](https://github.com/anomalyco/opencode/pull/60647))
  - `compaction.buffer` accepts a percentage of the model's context window, such as `"20%"`, in addition to a token count. ([#60647](https://github.com/anomalyco/opencode/pull/60647))
  - When `read` misses because a folder is missing, the error names that folder and the nearest one that exists. ([#60688](https://github.com/anomalyco/opencode/pull/60688))
  - When `edit` can't find `oldString`, the error shows the file's closest lines, or says the change is already there. ([#60689](https://github.com/anomalyco/opencode/pull/60689))
  - Calls to a tool named `bash` now run as `shell` instead of failing, with the same permissions. A plugin tool actually named `bash` still takes precedence. ([#60687](https://github.com/anomalyco/opencode/pull/60687))
  - MCP servers that send stdio messages larger than 10 MiB no longer get disconnected, and large messages are read much faster. ([#60694](https://github.com/anomalyco/opencode/pull/60694))
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-11-05 — [process-daemon-v0.5.0](https://github.com/anomalyco/opencode/releases/tag/process-daemon-v0.5.0) — component release.**
  - opencode-pty 0.5.0 (protocol 8) from packages/process-daemon, published to npm as @opencode/pty@0.5.0.

## 2. Aider

No in-window GitHub release record was captured; the API returned a short page and the public release listing was exhausted at capture. This is **not proof that Aider had no development, package releases, or other changes**. No in-window launch is inferred from current documentation.

Latest preceding GitHub release: [v0.86.0](https://github.com/Aider-AI/aider/releases/tag/v0.86.0) published 2025-08-09T17:42:19Z (outside the window; baseline only).

Sources and gaps: [GitHub Releases](https://github.com/Aider-AI/aider/releases) and its recorded API page are the dated evidence for this chapter. Aider website release history, package registries, commits and social channels were not independently exhausted in this fallback chapter.

## 3. OpenHands

**Repository:** [OpenHands/OpenHands](https://github.com/OpenHands/OpenHands), including its historical All-Hands-AI alias. This is the main repository only, not a full inventory of separately released SDK/CLI/runtime repositories. The GitHub listing was exhausted, but that says nothing about unrecorded server deployments.

**Key direction in the captured notes:** workflow automations and setup; connected repositories and enterprise Git providers; automation templates, integration permissions, and service-account execution. [v1.36.0, November 5](https://github.com/OpenHands/OpenHands/releases/tag/v1.36.0) includes Data Center template/setup work, repository-provider matching and service-account automations. The separately published `pr19394-evidence` record is evidence storage, not a shipping application release.

### Captured release chronology

- **2026-08-07 — [v1.11.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.11.0) — non-prerelease record.**
  - feat: show per-run LLM cost in the Activity Log and exports by @hieptl in https://github.com/OpenHands/OpenHands/pull/16351
  - feat: reorder Customize navigation by @DevinVinson in https://github.com/OpenHands/OpenHands/pull/16358
  - feat(settings): polish Agent Canvas version update UI by @FraterCCCLXIII in https://github.com/OpenHands/OpenHands/pull/16343
  - feat(home): keep Automations pane visible when empty by @neubig in https://github.com/OpenHands/OpenHands/pull/16372
  - feat: add a typed agent action for launching local or Cloud child conversations by @hieptl in https://github.com/OpenHands/OpenHands/pull/16380
  - feat: add automation tag filter and recognition by @hieptl in https://github.com/OpenHands/OpenHands/pull/16388
  - feat(conversations): add tag chips, overflow, and hovercard labels by @FraterCCCLXIII in https://github.com/OpenHands/OpenHands/pull/16346
  - fix: ship multi-size app icons for Windows and macOS by @hieptl in https://github.com/OpenHands/OpenHands/pull/16352
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-08-07 — [v1.12.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.12.0) — non-prerelease record.**
  - feat: clarify free OpenHands model endpoints by @juanmichelini in https://github.com/OpenHands/OpenHands/pull/16281

- **2026-08-13 — [v1.13.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.13.0) — non-prerelease record.**
  - feat(conversations): add client-side conversation archive by @FraterCCCLXIII in https://github.com/OpenHands/OpenHands/pull/16100
  - feat(chat): add inline markdown artifact previews by @FraterCCCLXIII in https://github.com/OpenHands/OpenHands/pull/16185
  - feat: add ready-for-dev issue readiness gate with type-specific criteria by @all-hands-bot in https://github.com/OpenHands/OpenHands/pull/16449
  - feat: context window usage meter, usage drawer, and manual compaction by @georgeglarson in https://github.com/OpenHands/OpenHands/pull/16311
  - fix: add postinstall message showing how to start agent-canvas by @all-hands-bot in https://github.com/OpenHands/OpenHands/pull/16338
  - fix: configure agent-server telemetry in Canvas launchers by @malhotra5 in https://github.com/OpenHands/OpenHands/pull/16348
  - fix: make overflow menus usable on touch devices by @FraterCCCLXIII in https://github.com/OpenHands/OpenHands/pull/16101
  - fix: make sidebar "Load more" discover folders, not folder contents by @FraterCCCLXIII in https://github.com/OpenHands/OpenHands/pull/16179
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-08-17 — [v1.14.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.14.0) — non-prerelease record.**
  - feat(canvas): present structured error outcomes by @neubig in https://github.com/OpenHands/OpenHands/pull/16231
  - feat: add LLM pre-flight validation to prevent saving misconfigured profiles by @neubig in https://github.com/OpenHands/OpenHands/pull/16417
  - feat(automations): add a Git Sync page by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/16521
  - feat: set Canvas default model to Kimi K3 and tag it free by @juanmichelini in https://github.com/OpenHands/OpenHands/pull/16657
  - fix(onboarding): preselect OpenHands LLM provider after picking OpenHands agent by @juanmichelini in https://github.com/OpenHands/OpenHands/pull/16531
  - fix: preserve backend scope in conversation links by @neubig in https://github.com/OpenHands/OpenHands/pull/16091
  - fix(dev): stop baking absolute VITE_BACKEND_BASE_URL in npm run dev by @neubig in https://github.com/OpenHands/OpenHands/pull/16605
  - fix(automations): set local responder URL from browser origin by @adbcodes in https://github.com/OpenHands/OpenHands/pull/16332
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-08-21 — [v1.15.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.15.0) — non-prerelease record.**
  - feat(sidebar): add getting started checklist with settings toggle by @FraterCCCLXIII in https://github.com/OpenHands/OpenHands/pull/16182
  - feat: show workspace path in Files view by @HurrairaBaloch in https://github.com/OpenHands/OpenHands/pull/16362
  - feat(automations): install a catalog entry that ships a script bundle by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/16680
  - feat: add LLM provider-connections UI (local agent-server) by @juanmichelini in https://github.com/OpenHands/OpenHands/pull/16616
  - feat: polish automations dashboard, recommended rail, and Add/Import flow by @FraterCCCLXIII in https://github.com/OpenHands/OpenHands/pull/16688
  - feat(conversation): add overview panel and unified commits drawer by @FraterCCCLXIII in https://github.com/OpenHands/OpenHands/pull/16230
  - feat: send template provenance for versioned setup entries by @hieptl in https://github.com/OpenHands/OpenHands/pull/16458
  - feat: instrument onboarding checklist link clicks by @hieptl in https://github.com/OpenHands/OpenHands/pull/16777
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-08-27 — [v1.16.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.16.0) — non-prerelease record.**
  - feat(settings): select supported LLM providers by @keeltrace in https://github.com/OpenHands/OpenHands/pull/16772
  - feat: add linux desktop installer build (CI + deb maintainer fix) by @georgeglarson in https://github.com/OpenHands/OpenHands/pull/16656
  - feat(automations): show a run's live phase by @aartem1 in https://github.com/OpenHands/OpenHands/pull/16740
  - feat(settings): expose the LLM-switching toggle in Agent settings by @georgeglarson in https://github.com/OpenHands/OpenHands/pull/16441
  - feat(skills): replace the all-on skill catalog with an explicit allow-list by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/16860
  - feat: link chat file paths to the Files drawer by @ShashwatXD in https://github.com/OpenHands/OpenHands/pull/16181
  - feat: pin a favorite sidebar page as the home route by @hieptl in https://github.com/OpenHands/OpenHands/pull/16868
  - feat: skip onboarding when a user-added Local backend has a usable LLM by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/16931
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-09-09 — [v1.17.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.17.0) — non-prerelease record.**
  - feat: show manifest-declared value statements on dashboard cards and rows by @hieptl in https://github.com/OpenHands/OpenHands/pull/16921
  - feat: Make automation run UI task-outcome aware by @malhotra5 in https://github.com/OpenHands/OpenHands/pull/16937
  - feat: enable LLM provider connections on cloud by @juanmichelini in https://github.com/OpenHands/OpenHands/pull/16757
  - feat: allow editing a custom cron expression from the edit-automation modal by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/16964
  - feat: unified cloud settings entrypoint for Agent Canvas by @jpelletier1 in https://github.com/OpenHands/OpenHands/pull/16883
  - feat: add local Agent Canvas Planner support by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/16178
  - feat(conversation-panel): add conversation tags and filtering by @georgeglarson in https://github.com/OpenHands/OpenHands/pull/16600
  - feat: split automation permissions into view/manage with creator escape hatch by @tofarr in https://github.com/OpenHands/OpenHands/pull/17105
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-09-11 — [v1.18.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.18.0) — non-prerelease record.**
  - feat: click an image attachment to open it full size by @palrohitg in https://github.com/OpenHands/OpenHands/pull/17179
  - feat: only the creator can turn an automation back on by @hieptl in https://github.com/OpenHands/OpenHands/pull/17190
  - feat: allow editing automations on cloud backends by @hieptl in https://github.com/OpenHands/OpenHands/pull/17191
  - feat: show which identity an automation runs as by @hieptl in https://github.com/OpenHands/OpenHands/pull/17138
  - feat: use DB-driven free/default model flags in Canvas by @juanmichelini in https://github.com/OpenHands/OpenHands/pull/16685
  - fix: pass the active org from the gear icon to cloud settings by @hieptl in https://github.com/OpenHands/OpenHands/pull/17256
  - fix: hide unlisted settings pages from the command menu when locked to a Cloud host by @hieptl in https://github.com/OpenHands/OpenHands/pull/17254
  - fix: open Cloud settings in the same tab when locked to a Cloud host by @hieptl in https://github.com/OpenHands/OpenHands/pull/17253
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-09-16 — [v1.19.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.19.0) — non-prerelease record.**
  - feat: scope an agent profile to specific MCP servers by @simonrosenberg in https://github.com/OpenHands/OpenHands/pull/17289
  - feat: add GPT-6 Astra model support by @neubig in https://github.com/OpenHands/OpenHands/pull/17164
  - feat: track authenticated Canvas arrivals by @neubig in https://github.com/OpenHands/OpenHands/pull/17083
  - feat: surface automation disablement reason in the UI by @malhotra5 in https://github.com/OpenHands/OpenHands/pull/17425
  - fix: recognize Azure DevOps SSH remotes by @Chocolatine75 in https://github.com/OpenHands/OpenHands/pull/17375
  - fix(static-server): HTML-escape injected runtime config and set Cache-Control: no-store on credential injection by @Tyagiquamar in https://github.com/OpenHands/OpenHands/pull/17175
  - fix: avoid redundant automation SDK version refetches by @workvahedi in https://github.com/OpenHands/OpenHands/pull/16991
  - fix(acp): tie the client ACP registry pin to agent-server, drop the codex shim by @simonrosenberg in https://github.com/OpenHands/OpenHands/pull/17423
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-09-17 — [v1.20.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.20.0) — non-prerelease record.**
  - feat(agent-profiles): select secrets available to a profile by @simonrosenberg in https://github.com/OpenHands/OpenHands/pull/17237
  - feat: forward Docker conversation runtime settings by @neubig in https://github.com/OpenHands/OpenHands/pull/17462
  - feat(automations): select saved agent profiles by @neubig in https://github.com/OpenHands/OpenHands/pull/17396

- **2026-09-22 — [v1.21.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.21.0) — non-prerelease record.**
  - feat: request Docker execution workspaces from Docker servers by @neubig in https://github.com/OpenHands/OpenHands/pull/17177
  - fix: prevent stale conversation history merges by @adbcodes in https://github.com/OpenHands/OpenHands/pull/16913
  - fix: query the Forgejo host for Forgejo PR and issue lists by @Harsh23Kashyap in https://github.com/OpenHands/OpenHands/pull/17486
  - fix(apps): accept GitHub tree URLs as app source by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/17504
  - fix: Disable PostHog autocapture  by @malhotra5 in https://github.com/OpenHands/OpenHands/pull/17502
  - fix(static-server): serve current assets after rebuilding Canvas by @neubig in https://github.com/OpenHands/OpenHands/pull/17402
  - fix: show non-resumable local conversations as archived by @neubig in https://github.com/OpenHands/OpenHands/pull/17500
  - fix: advertise host services to Docker conversations by @neubig in https://github.com/OpenHands/OpenHands/pull/17518
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-09-22 — [v1.22.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.22.0) — non-prerelease record.**
  - feat: test remote MCP servers on cloud backends via the app server by @hieptl in https://github.com/OpenHands/OpenHands/pull/17276
  - feat: open Git Sync to org admins on cloud backends by @hieptl in https://github.com/OpenHands/OpenHands/pull/17216
  - feat(i18n): translate recently added Turkish UI strings by @alkank in https://github.com/OpenHands/OpenHands/pull/17581
  - fix: show the add automation menu to org members by @hieptl in https://github.com/OpenHands/OpenHands/pull/17526
  - fix(canvas): keep the base path on links that leave the app by @tofarr in https://github.com/OpenHands/OpenHands/pull/17617
  - fix: preserve sibling MCP servers when saving to a cloud backend by @hieptl in https://github.com/OpenHands/OpenHands/pull/17467
  - fix: link home automation rows to the automation view by @hieptl in https://github.com/OpenHands/OpenHands/pull/17466
  - fix(canvas): hide cloud orgs the server marks is_visible=false by @tofarr in https://github.com/OpenHands/OpenHands/pull/17624
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-09-23 — [v1.23.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.23.0) — non-prerelease record.**
  - feat(desktop): universal macOS DMG with per-arch bundled runtimes by @harsh-batheja in https://github.com/OpenHands/OpenHands/pull/17225
  - feat: Support Light+ and Solarized Light theme by @davideuler in https://github.com/OpenHands/OpenHands/pull/16632
  - feat: tag Agent Canvas telemetry by deployment kind (OSS-13530) by @malhotra5 in https://github.com/OpenHands/OpenHands/pull/17529
  - fix: refresh Canvas image packages by @neubig in https://github.com/OpenHands/OpenHands/pull/17633
  - fix(chat): restore 11px mode label on the change-agent button by @Harsh23Kashyap in https://github.com/OpenHands/OpenHands/pull/17616
  - fix(test): drain MSW's in-flight requests instead of 30 event-loop turns by @alanhuangyoo in https://github.com/OpenHands/OpenHands/pull/16890
  - fix(chat): avoid re-rendering unchanged historical Markdown by @Jokasa7 in https://github.com/OpenHands/OpenHands/pull/17015
  - fix: load logs for script automation runs and show their script by @hieptl in https://github.com/OpenHands/OpenHands/pull/17519
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-09-25 — [v1.24.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.24.0) — non-prerelease record.**
  - feat: toggle all workspace folders from the Conversations header by @kuldeepsinh19 in https://github.com/OpenHands/OpenHands/pull/17409
  - feat: scope OpenAI subscription caches by backend by @uczltw6 in https://github.com/OpenHands/OpenHands/pull/16699
  - feat(home): reframe splash copy around day-to-day engineering work by @errmakov in https://github.com/OpenHands/OpenHands/pull/17545
  - feat(conversation): open shared automation conversations read-only on cloud by @hieptl in https://github.com/OpenHands/OpenHands/pull/17683
  - fix: cap conversation Skills/Hooks/Tools dialogs at 90vw by @gokay-ai in https://github.com/OpenHands/OpenHands/pull/17586
  - fix(automation): accept single-start stepped cron fields in the schedule validator by @Harsh23Kashyap in https://github.com/OpenHands/OpenHands/pull/17274
  - fix(canvas): call agent-server runtime directly instead of /api/cloud-proxy (405 on verification confirm & compact-context) by @juanmichelini in https://github.com/OpenHands/OpenHands/pull/17511
  - fix: render ACP tool-call content blocks in chat cards by @Chocolatine75 in https://github.com/OpenHands/OpenHands/pull/17407
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-10-06 — [v1.25.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.25.0) — non-prerelease record.**
  - feat: outline the back navigation control on settings sub-pages by @errmakov in https://github.com/OpenHands/OpenHands/pull/17587
  - feat(settings): bulk-add provider models as LLM profiles by @georgeglarson in https://github.com/OpenHands/OpenHands/pull/16426
  - feat: add direct-prompt Model Router settings by @juanmichelini in https://github.com/OpenHands/OpenHands/pull/16147
  - feat(canvas): retire streaming slots by event id and delete the text-matching reconciler by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/17465
  - feat(apps): add Update action for installed Canvas Apps by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/17736
  - feat(settings): per-connection bulk-add LLM profiles by @juanmichelini in https://github.com/OpenHands/OpenHands/pull/17723
  - feat: add Model Router "Run at conversation start" toggle by @juanmichelini in https://github.com/OpenHands/OpenHands/pull/17797
  - feat(automations): show templates on cloud backends and offer native git integrations by @hieptl in https://github.com/OpenHands/OpenHands/pull/17830
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-10-08 — [v1.26.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.26.0) — non-prerelease record.**
  - feat(apps): add authenticated backend view helper by @neubig in https://github.com/OpenHands/OpenHands/pull/17656
  - feat(chat): render empty-response corrective nudge as an informational note by @errmakov in https://github.com/OpenHands/OpenHands/pull/17864
  - feat: verify-openhands skill with control-openhands CLI and the full feature map by @enyst in https://github.com/OpenHands/OpenHands/pull/17961
  - feat(verify-openhands): daily pass support and CLI fixes by @enyst in https://github.com/OpenHands/OpenHands/pull/18093
  - feat(verify-openhands): map 20 more behaviors, with live recipes by @enyst in https://github.com/OpenHands/OpenHands/pull/18105
  - feat(backends): explain a suspended workspace and offer a switch by @hieptl in https://github.com/OpenHands/OpenHands/pull/17988
  - feat(onboarding): show the Super Admin setup guide on enterprise backends by @hieptl in https://github.com/OpenHands/OpenHands/pull/17969
  - feat(automations): narrow needs review status by @DevinVinson in https://github.com/OpenHands/OpenHands/pull/17682
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-10-12 — [v1.27.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.27.0) — non-prerelease record.**
  - feat(verify-openhands): drive the mock-LLM e2e behaviors on DeepSeek and map the gaps by @enyst in https://github.com/OpenHands/OpenHands/pull/18191
  - feat(verify-openhands): record videos of the browser with `browser record` by @enyst in https://github.com/OpenHands/OpenHands/pull/18211
  - feat(acp): add OpenCode and Pi as built-in ACP agents on local backends by @Placidoe in https://github.com/OpenHands/OpenHands/pull/17758
  - feat: Add splash page mode toggle by @malhotra5 in https://github.com/OpenHands/OpenHands/pull/17199
  - feat: add automate setup split panel by @malhotra5 in https://github.com/OpenHands/OpenHands/pull/17200
  - feat: add agent-assisted automation form updates by @malhotra5 in https://github.com/OpenHands/OpenHands/pull/17201
  - feat: add persisted automation draft management UI by @malhotra5 in https://github.com/OpenHands/OpenHands/pull/17242
  - feat: finalize automation setup draft flow by @malhotra5 in https://github.com/OpenHands/OpenHands/pull/17688
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-10-12 — [v1.27.1](https://github.com/OpenHands/OpenHands/releases/tag/v1.27.1) — non-prerelease record.**
  - fix(docker): copy MARS web bridge modules into the image by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/18343
  - fix(themes): make warning feedback theme-aware by @enyst in https://github.com/OpenHands/OpenHands/pull/18333

- **2026-10-19 — [v1.28.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.28.0) — non-prerelease record.**
  - feat(verify-openhands): executable known bugs and a repeatable fix proof by @enyst in https://github.com/OpenHands/OpenHands/pull/18348
  - feat(i18n): add Bahasa Indonesia (id) locale by @analisaperlengkapan in https://github.com/OpenHands/OpenHands/pull/18067
  - feat: add close control to Getting started checklist by @DevinVinson in https://github.com/OpenHands/OpenHands/pull/18185
  - feat(secrets): let the agent request a secret through an in-conversation modal by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/18408
  - feat(verify-openhands): adopt the shared known-bug contract by @enyst in https://github.com/OpenHands/OpenHands/pull/18355
  - feat(chat): smooth, reading-pace streaming with word fade-in by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/18491
  - feat(secrets): ask for requested secrets inline in the chat by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/18575
  - feat(automations): hide templates that overlap built-in integrations on self-hosted instances (OHE-3611) by @hieptl in https://github.com/OpenHands/OpenHands/pull/18565
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-10-19 — [v1.28.1](https://github.com/OpenHands/OpenHands/releases/tag/v1.28.1) — non-prerelease record.**
  - * chore: bump agent-server to 1.56.0 by @all-hands-bot in https://github.com/OpenHands/OpenHands/pull/18670
  - * chore: remove merged PR artifacts by @all-hands-bot in https://github.com/OpenHands/OpenHands/pull/18570

- **2026-10-20 — [v1.29.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.29.0) — non-prerelease record.**
  - feat(backends): show full organization names in the backend selector by @ak684 in https://github.com/OpenHands/OpenHands/pull/18667
  - feat(launchers): read public skills from the bundled extensions package by @simonrosenberg in https://github.com/OpenHands/OpenHands/pull/18683
  - feat(agent-profiles): choose skills per profile from the server catalog by @simonrosenberg in https://github.com/OpenHands/OpenHands/pull/18686
  - feat(agent-profiles): edit condenser and verification per profile by @simonrosenberg in https://github.com/OpenHands/OpenHands/pull/18679
  - feat(agent-profiles): choose a Model Router per profile by @simonrosenberg in https://github.com/OpenHands/OpenHands/pull/18685
  - feat(home): dismiss recommended automations rail from Home and Application settings by @lx3133584 in https://github.com/OpenHands/OpenHands/pull/18538
  - feat(agent-profiles): launch `default` by id; profiles own their settings by @simonrosenberg in https://github.com/OpenHands/OpenHands/pull/18720
  - fix(conversations): save a connected repository on the cloud conversation by @henriquehirako in https://github.com/OpenHands/OpenHands/pull/18158
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-10-22 — [v1.30.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.30.0) — non-prerelease record.**
  - feat(automations): reflect the built-in integration's connection state in the setup choice modal (OHE-3612) by @hieptl in https://github.com/OpenHands/OpenHands/pull/18566
  - fix(chat): let a Cloud member pick the LLM profile on the home page (OHE-3669) by @hieptl in https://github.com/OpenHands/OpenHands/pull/18729
  - fix(sidebar): refresh the conversation list when a cloud start task is ready (OSS-14604) by @hieptl in https://github.com/OpenHands/OpenHands/pull/18741
  - fix(sidebar): hide the Getting started checklist on a canvas locked to a Cloud host (OHE-3683) by @hieptl in https://github.com/OpenHands/OpenHands/pull/18739
  - fix(canvas): persist workspace grouping via server-side conversation tags by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/18722
  - fix(agent-profiles): keep the `default` rules local and save home LLM picks on Cloud by @simonrosenberg in https://github.com/OpenHands/OpenHands/pull/18747
  - fix(frontend): link Azure DevOps repositories to their _git page by @ak684 in https://github.com/OpenHands/OpenHands/pull/18753
  - fix: OHE-3677 make the default primary color white (OHE-3677) by @tofarr in https://github.com/OpenHands/OpenHands/pull/18726
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-10-22 — [v1.30.1](https://github.com/OpenHands/OpenHands/releases/tag/v1.30.1) — non-prerelease record.**
  - refactor(diff): express the Markdown preview cap in CSS by @enyst in https://github.com/OpenHands/OpenHands/pull/18762
  - refactor(automations): express the activity track height in CSS by @enyst in https://github.com/OpenHands/OpenHands/pull/18788

- **2026-10-22 — [v1.31.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.31.0) — non-prerelease record.**
  - feat(automations): hide the GitHub token secret field on self-hosted instances when GitHub is connected (OHE-3699) by @hieptl in https://github.com/OpenHands/OpenHands/pull/18829
  - fix(mcp): install OAuth-only library entries through the MCP server's own OAuth by @ak684 in https://github.com/OpenHands/OpenHands/pull/18780
  - fix: keep automation repositories when editing by @ak684 in https://github.com/OpenHands/OpenHands/pull/18776
  - fix(mars): forward typed session key to web bridge on non-baked binds by @lx3133584 in https://github.com/OpenHands/OpenHands/pull/18465
  - fix(mars): preserve DigitalOcean sessions through auth recovery by @smolpaws in https://github.com/OpenHands/OpenHands/pull/18604
  - fix(mcp): say the sign-in was not completed when an OAuth install pop-up is abandoned by @ak684 in https://github.com/OpenHands/OpenHands/pull/18816

- **2026-10-23 — [v1.32.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.32.0) — non-prerelease record.**
  - feat(home): store pinned automations in agent-server settings by @abhayymishraa in https://github.com/OpenHands/OpenHands/pull/18190
  - feat(mcp): let the install dialog choose between installable library options by @ak684 in https://github.com/OpenHands/OpenHands/pull/18836
  - feat(automations): list every cloneable provider's repositories in the automation picker by @ak684 in https://github.com/OpenHands/OpenHands/pull/18821
  - feat(canvas): quiet backend logs by default by @neubig in https://github.com/OpenHands/OpenHands/pull/18830

- **2026-10-23 — [v1.33.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.33.0) — non-prerelease record.**
  - feat(verify-openhands): launch --mars-web and --lock-to-cloud by @enyst in https://github.com/OpenHands/OpenHands/pull/18848
  - feat(digitalocean): keep the web build signed in across restarts by @enyst in https://github.com/OpenHands/OpenHands/pull/18705
  - fix(automations): render the shipped current_phase on live runs by @aartem1 in https://github.com/OpenHands/OpenHands/pull/17122
  - fix(verify-openhands): bug run puts back the viewport a block changed by @enyst in https://github.com/OpenHands/OpenHands/pull/18850
  - fix(digitalocean): default new agents to the smallest VM by @smolpaws in https://github.com/OpenHands/OpenHands/pull/18702
  - fix: mask secret values in request and Settings forms by @neubig in https://github.com/OpenHands/OpenHands/pull/18870
  - refactor(files): express tree indentation in CSS by @enyst in https://github.com/OpenHands/OpenHands/pull/18804
  - refactor(sidebar): express mobile drawer duration in CSS by @enyst in https://github.com/OpenHands/OpenHands/pull/18805

- **2026-10-27 — [v1.34.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.34.0) — non-prerelease record.**
  - feat(verify): stand in for DigitalOcean so Managed Agents can be verified by @enyst in https://github.com/OpenHands/OpenHands/pull/18867
  - feat(canvas): per-conversation Docker runtime and Settings → Runtime page by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/18436
  - feat(automations): filter and edit automation tags by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/18767
  - feat(conversation): label the share toggle "Share with Org users" in team orgs (OHE-3765) by @hieptl in https://github.com/OpenHands/OpenHands/pull/18986
  - feat(sidebar): start a new conversation in the row's sandbox (OHE-3689) by @hieptl in https://github.com/OpenHands/OpenHands/pull/18912
  - fix: pass session API key to static-server via env instead of argv by @putramkti in https://github.com/OpenHands/OpenHands/pull/16420
  - fix(chat): keep drag-selection inside the message feed by @analisaperlengkapan in https://github.com/OpenHands/OpenHands/pull/18680
  - fix(mars): isolate invalid OAuth relay attempts by @Fieldnote-Echo in https://github.com/OpenHands/OpenHands/pull/18892
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-11-03 — [v1.35.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.35.0) — non-prerelease record.**
  - feat(home): remember and clear the cloud repository selection by @tofarr in https://github.com/OpenHands/OpenHands/pull/19053
  - feat(automations): store picked Bitbucket Data Center repositories as clone URLs by @ak684 in https://github.com/OpenHands/OpenHands/pull/19023
  - feat(automations): list each event source's own event types by @ak684 in https://github.com/OpenHands/OpenHands/pull/19025
  - feat: persist workspace/repo metadata as server-side conversation tags by @BSmick6 in https://github.com/OpenHands/OpenHands/pull/16666
  - feat(files): add a Download button to the Files tab (OHE-3834) by @hieptl in https://github.com/OpenHands/OpenHands/pull/19076
  - feat(conversations): store sidebar pin/archive state in server-side tags by @neubig in https://github.com/OpenHands/OpenHands/pull/18697
  - feat(sidebar): drag the sidebar edge to resize it by @ak684 in https://github.com/OpenHands/OpenHands/pull/19097
  - feat(sidebar): make sidebar text one size larger by @ak684 in https://github.com/OpenHands/OpenHands/pull/19113
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-11-05 — [v1.36.0](https://github.com/OpenHands/OpenHands/releases/tag/v1.36.0) — non-prerelease record.**
  - feat(automations): open templates without the missing-integration notice by @ak684 in https://github.com/OpenHands/OpenHands/pull/19297
  - feat(automations): list a script automation's files collapsed by @VascoSch92 in https://github.com/OpenHands/OpenHands/pull/19319
  - feat(automations): suggest default templates and promote ones using what users set up by @ak684 in https://github.com/OpenHands/OpenHands/pull/19300
  - feat(automations): tell the setup assistant the instance's event sources by @ak684 in https://github.com/OpenHands/OpenHands/pull/19335
  - feat(automations): ship the Data Center templates with extensions 0.39.0 by @ak684 in https://github.com/OpenHands/OpenHands/pull/19344
  - feat(automations): send a template's picked repository only to services that clone its provider by @ak684 in https://github.com/OpenHands/OpenHands/pull/19347
  - feat(automations): list Bitbucket Data Center and Azure DevOps repositories in template setup by @ak684 in https://github.com/OpenHands/OpenHands/pull/19340
  - feat(automations): use the repository picker in the Data Center templates with extensions 0.40.0 by @ak684 in https://github.com/OpenHands/OpenHands/pull/19355
  - More captured note lines are available at the release link and in the raw inventory; this excerpt is not exhaustive.

- **2026-11-05 — [pr19394-evidence](https://github.com/OpenHands/OpenHands/releases/tag/pr19394-evidence) — evidence storage — NOT a product release.**
  - The maintainer explicitly identifies this as temporary evidence storage. Its install boilerplate is not counted as a new application capability.
## Coverage limitations and comparison

- Application, SDK, component and evidence-store records must not be collapsed into one comparable release count. Non-prerelease metadata does not override feature-level experimental labels.
- OpenCode pagination is partial; in-window records listed deeper could be absent. Its failed subsequent request has no retained HTTP cause, so rate limiting is not asserted.
- Aider has an explicit evidence gap. OpenHands main-repository coverage is not all OpenHands ecosystem coverage.
- Summaries above are selected verbatim release-note lines, not an independent implementation assessment. Release-note repetition does not identify a capability’s first shipping date.
- Implications for Refact are evaluation topics: opt-in scheduling, predictable policy migration, compaction controls, reviewable worktrees, and understandable integration setup. This chapter does not establish whether Refact lacks any of them.
