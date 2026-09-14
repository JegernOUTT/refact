# Multi-agent parity program

Status: APPROVED — implementation authorized and under way, with the scope amendment recorded below.

**Approval update:** The user approved implementation of M1–M10 in this session in worktree `481322af-2e95-4924-ac25-c2b3089ea601`. All operational caps must be editable in Settings with generous defaults; periodic significant trajectory/task changes may refresh model context. This approval supersedes the proposal status above and earlier numerical defaults. Initial implementation defaults are 32 active per engine, 16 per project/root, 1,024 pending and 4,096 outstanding runs; other limits follow the shared settings schema. Goals retain unlimited-by-default effort budgets. Milestones remain subject to their test/review gates; approval is not a completion claim.

**Scope amendment:** After approval the user narrowed the program to local-only coordination. The previously numbered M7 (external harness, ACP client/server, experimental A2A), M8 (remote/federated coordination) and M9 (autonomous evolution) milestones are dropped in full, as are M4's aggregate whole-tree resource budgets; the hook control work that lived in M7 survives and moves to M2/M10. Capacity limits — concurrency caps, queue bounds and rate limits — stay, and are not budgets. Eight collaboration additions (A–H) are adopted and specified in section 10. M6's governed code-mode orchestration is explicitly retained. This paragraph is the only remaining reference to the dropped milestone numbers and exists for traceability with earlier revisions.

Baseline inspected: Refact `0e93e01c9`. Local competitor checkouts are research evidence, not a claim about current commercial releases. Recheck source anchors when execution begins.

## 1. Decisions and scope

The user selected:

- One full-parity implementation program for local coordination, interaction, evidence and adaptive scheduling. Coordination across machines, control of third-party agent binaries and autonomous self-evolution are outside it.
- Keep chat/subagent and task-board runtimes distinct. Share coordination, scheduling, verification and related infrastructure; do not replace them with a third workflow runtime.
- Automatically provide same-project live-work summaries and relevant overlaps to a new chat. Retrieve detailed transcripts on demand, with provenance. Cross-project access is explicitly scoped.
- Preserve unlimited-by-default goal budgets and configurable nesting. Bound active concurrency through capacity limits; aggregate tree resource budgets are dropped.
- Make the first production milestone implementation-ready; outline the rest. Wait for explicit confirmation before implementing.

Parity means matching the useful, observable capabilities researched, not copying every tool spelling, duplicating each competitor's architecture, reproducing bugs, or claiming an experimental feature is production-ready. Every capability below must ultimately have a runnable acceptance scenario. Advanced features that remain in the program are not silently dropped; they may ship disabled until their security gates pass. Capabilities the user removed in the scope amendment are recorded as dropped above, not quietly omitted here.

### Corrections to the research used by this plan

1. Tool-call concurrency, launch concurrency, resident sessions and active execution are different limits. A bounded tool batch does not establish a global lifetime cap on background children.
2. Refact already assigns `Queued` and now has shared delivery infrastructure with `preempt`, `append`, `when_idle` and a separate wake flag (`src/chat/delivery.rs`). Extend this implementation; do not introduce another competing transport.
3. Refact already has hook types and implementations (`crates/refact-ext/src/hooks.rs`, `src/ext/hooks_runner.rs`). Audit and extend their control effects, rather than assuming hooks are absent.
4. Excluding the current root from automatic memory retrieval is intentional self-recall avoidance. Preserve it. Improve scope, provenance, freshness and trust labeling instead.
5. String/regex/anchored/line edits pass an intra-operation preimage. Whole-file/create/patch paths differ. Do not claim every write has no protection or that an intra-operation preimage represents the model's last read.
6. `verify_hint` is prose. Never automatically execute it. Structured-output validation, a checked todo, a reviewed fragment and an LLM verdict are different evidence levels.
7. Worktree/task leases do not lock every source file; directory deletion protection is not edit-collision prevention. Same logical path in two separate worktrees is an integration conflict candidate, not simultaneous mutation of one physical file.
8. Claims such as "only product", "best", "third place", "unrestricted access", and "no implementation anywhere" were not established sufficiently to become requirements. Feature-gated/missing source, type-only contracts and docs-only Antigravity evidence remain qualified.

## 2. Target architecture: two runtimes, shared services

Keep ownership of execution in the current runtimes:

- **Chat runtime:** interactive chats, background subagents, their trajectories, tool loop and chat goal behavior.
- **Task-board runtime:** planners, cards, board workflow, task agents, merge and post-merge checks.

Introduce shared services with adapters at existing boundaries:

| Shared service | Responsibility | Must not become |
|---|---|---|
| Work identity/catalog | Project, checkout, session, run, task and host identity; discovery; status/attention/provenance projections | Another transcript or board authority |
| Admission scheduler | Queues, execution permits, parking, fairness, rate/resource limits | A replacement for either runtime's workflow |
| Delivery/interaction | Existing delivery pipeline, receipts, mailbox waits, questions, authorized steering | A new tool-result-incompatible message path |
| File coordination | Visible-read stamps, attributed writes, overlap signals, cooperative locks, explicit claims | A promise that arbitrary shell/editor writes are intercepted |
| Evidence/verification | Explicit verification specs, policy-gated execution, artifacts, verdicts and freshness | A shell executor for arbitrary recalled prose |
| Recovery/ownership | Run generations, durable intents, leases, outbox, reconciliation | Blind replay of side effects |
| Backend adapters | Native Refact execution backends; capability description and model/tool restriction validation | A way to bypass local privacy or tool authority |

Add a small dependency-light `refact-work-core` crate for shared identities, scheduler state machine and projection contracts. Put engine adapters in `src/coordination/`. Future persistence can be a separate module/crate if dependencies warrant it. Neither `refact-agents-core` nor `refact-tasks` should depend on each other. Existing transcript and board stores remain authoritative for their own content.

### Stable identity model

- `ProjectKey`: canonical local repository common-dir/source-root identity, compatible with existing worktree project hashes. Non-git projects use canonical workspace roots. Do not key on basename, branch or git origin URL alone. Multiple independent clones do not silently share authority.
- `CheckoutKey`: canonical effective filesystem checkout/worktree, separate from logical project identity.
- `WorkKey`: project + runtime kind + stable chat/card/agent reference. It survives an engine restart; engine-instance identity is not part of this stable key.
- `RunKey`: WorkKey + attempt/generation. Current ownership separately records host/engine-instance identity. Resuming an attempt is distinguishable from replaying a stale completion. Board agents inherit the owning planner/task root for aggregate scheduling, rather than inventing a new root to bypass a cap.
- `LogicalFileKey`: ProjectKey + normalized repository-relative path.
- `PhysicalFileKey`: canonical physical file/path identity in a checkout, respecting OS case behavior, symlinks and file aliases.

Reuse the existing project/worktree resolver and extend it with alias tests; do not reuse string-only overlap normalization as an authorization or locking boundary.

### Cross-cutting invariants

1. Visible metadata is not permission to read, message, cancel, execute or merge. Apply authorization separately at every boundary.
2. Filter metadata for project scope and destination-model privacy before constructing tool results, startup context, logs or artifact previews. Titles and summaries can be sensitive too.
3. User input, cancellation, explicit pause and approval denials take precedence over autonomous continuation.
4. Accepted, queued, delivered, consumed, completed, verified and merged are distinct states. Never report one as another.
5. Schema-valid output is not verified success. Budget exhaustion, timeout, unavailable evidence and unknown outcome are not success.
6. Do not hold application/session/board locks while awaiting permits, user input, model responses, network calls or subprocesses. Acquire application context before inner locks; copy identifiers and drop guards before coordination waits.
7. Any spawned monitor/worker has an owner, cancellation path and joined shutdown. No unattended retry loop or notification amplification loop.
8. Capacity is reserved atomically before execution side effects. Retrying an accepted request does not spawn another child.
9. Protect existing user changes and preserve interrupted worktrees. A merge conflict never triggers silent discard or overwrite.
10. Unsupported backend capabilities fail before dispatch. Explicitly requested models/tool restrictions may not be silently ignored.

## 3. Milestone map and dependencies

| Milestone | User-visible outcome | Depends on |
|---|---|---|
| **M1 — Local safe coordination** | A new chat sees other project work; both agent runtimes share bounded scheduling; built-in edits detect stale reads; board claims are serialized across cooperating local engines | Approval |
| **M2 — First-class interaction and scope** | Peer messaging, blocking questions, delivery receipts, shared task claiming and enforceable scoped tool access | M1 |
| **M3 — Durable execution and recovery** | Cross-process presence/scheduling, resumable children, safe replay, durable file activity and worktree integration | M1; M2 interaction contracts |
| **M4 — Resource governance and adaptive scheduling** | Capacity limits, provider fairness/AIMD, bounded retries, residency control | M1; M3 durable accounting |
| **M5 — Evidence-backed goals and plans** | Executable verification contracts, independent review, wait barriers, goal decomposition and bounded durable plans | M1; M2; M3 for crash recovery |
| **M6 — Teams, DAGs and convergence** | Shared task DAGs, reusable teams, wave scheduling, structured outcomes, governed workflow scripts | M2–M5 |
| **M10 — Full observability and rollout qualification** | Consistent GUI/TUI/IDE controls, inspectable provenance, interoperable recovery, end-to-end parity certification | Incremental throughout; final gate after M6 |

Do not run multiple implementation agents against shared scheduler/session/board files simultaneously. M1 is itself several reviewable changes, not a single large commit.

## 4. M1: implementation-ready specification

### 4.1 Boundary and exclusions

M1 ships a usable local vertical slice, not speculative distributed infrastructure:

- Cross-root, same-project discovery for top-level chats, child agents, task planners and board agents hosted by the same engine.
- A background execution scheduler shared by the two runtimes in that engine.
- Read-only model tools and HTTP/UI surfaces; automatic bounded project activity context.
- Per-chat visible-read tracking and attributed built-in writes.
- Cooperative local cross-process file critical sections and board transaction locks.
- Existing history, worktree merge and task workflows remain available.

M1 does **not** claim cross-process live attribution, persistent queued execution, crash-resume or universally intercepted writes. Those have assigned milestones. Coordination across machines, control of third-party agent binaries and tree-wide billing enforcement are outside the program entirely. Restarted queued/running work is reported interrupted; it is not automatically replayed. Goals and their current defaults are unchanged.

### 4.2 Shared work projection

Define a `WorkSummary` with:

- WorkKey/RunKey and existing chat/agent/task/card references; runtime kind.
- ProjectKey, CheckoutKey, branch/worktree reference and host/engine identity.
- Title, runtime status, execution state, attention reason, current tool name, progress summary.
- Separate `observed_at`, `last_activity_at` and `last_progress_at`; no inferred progress from heartbeat alone.
- `freshness = live | recent | stale | unknown` and `coverage = complete_for_scope | partial | unavailable`.
- Declared target paths and observed reads/writes kept separately, each with provenance and observation time.
- Usage fields only where measured; unknown values are null, not zero.

`live` means a source-validated session/runner in this engine, not simply an uncompleted disk record. An idle loaded session remains idle; it is not advertised as running. Persisted status without a live source is recent/stale/unknown. Recent-window default: 60 minutes (`recent_window_secs=3600`). Deleted/merged worktrees lose active-location status but remain inspectable historical records.

Build the catalog from narrow snapshots of the session map, background-agent registry, indexed board metadata and worktree inventory. Do not scan every trajectory JSON or every board on each model turn. Bootstrap indexed summaries once, then update through events; missing/stale sources carry explicit coverage flags. Root and project projections share the same logic, while existing root-scoped tools keep their defaults.

### 4.3 Model and HTTP surface

Add two read-only model tools rather than many overlapping commands:

1. `work_overview(scope="project"|"root", path?, states?, cursor?, limit=25)` — top-level work, children, task/card references, active/queued/waiting state and observed overlap. Hard maximum page size 100. Default same-project scope.
2. `work_inspect(selector={kind:"work", work_id}|{kind:"files", paths}|{kind:"worktree", worktree_id}|{kind:"project_worktrees"}, cursor?, limit=25)` — a discriminated selector, bounded detail and ownership references. File selection accepts 1–50 paths, resolved within the authorized project; worktree inventory is always for the caller's project. Hard maximum page size 100. No raw transcript body in M1.

Return structured data plus a concise readable rendering, snapshot sequence, `observed_at`, coverage and omission cursor. Schema/alias registration must follow current tool conventions. Existing `agents_overview`, `agent_list/status/wait/result/cancel`, board tools and worktree merge retain identity and behavior; they reuse projections where appropriate but do not silently expand mutation authority.

Add `GET /v1/work`, `GET /v1/work/:id` and a snapshot+sequence-based subscription under the existing authenticated local API conventions. Validate caller scope on reads and all existing mutation paths; supplying a guessed ID is not authorization. Reconnection after a sequence gap fetches a fresh snapshot. The GUI and model read the same projection.

#### M1 schema, scope and privacy details

`work_overview.states` filters shared execution states `queued | running | parked | idle | waiting_user | completed | failed | cancelled | interrupted | unknown`; raw adapter statuses remain available separately. Omitted states default to nonterminal live work plus recent terminal summaries. Both tools are read-only by construction, and therefore marked `allow_parallel=true`, with no new aliases. `work_inspect` is a JSON-Schema `oneOf` discriminated by selector `kind`, with no ambiguous optional-selector combinations. Existing history-search/context tools remain the detailed retrieval surface; M1 adds provenance and destination/project checks to them rather than requiring a new transcript tool.

Cursors are opaque, versioned, expiry-bounded tokens tied to project, caller visibility, filter hash and projection generation, with deterministic `(observed_at, work_id)` ordering. A stale or mismatched cursor returns `cursor_expired`/`invalid_cursor`, not a silently spliced page. Defaults are specified above; invalid selectors, limits, states and paths are rejected. Errors have `{code, message, retryable, request_id, details?}`; codes include `invalid_argument`, `not_found_or_not_visible`, `policy_unavailable`, `scope_denied`, `queue_full`, `cancelled`, `lock_timeout`, `stale_read`, `unsupported`, `unavailable`. Details are independently privacy-filtered. Queue errors identify no hidden peer.

Model output uses the existing destination boundary `src/privacy/destinations.rs::clear_for_model`, `Destination::from_model_record`, and privacy records/registered worktree aliases in `src/privacy/records.rs`; do not substitute the generic `AllowToSendAnywhere` filesystem predicate for a destination-model decision. Carry source attribution into a privacy-audited projection wrapper; unknown/unavailable policy or unresolved destination fails closed with `policy_unavailable` and no sensitive rows. Authorize the filesystem/project scope before collecting bodies; re-evaluate at preparation/delivery time using current policy/model generation. Cached capsules are keyed by both and discarded on change. No denied-row counts are exposed to a model that cannot know those rows exist.

HTTP work routes use a scope resolver deriving the allowed project from trusted engine/workspace state, not a request-supplied filesystem root. Require and validate the existing daemon bearer credential when configured (reuse the authorization helper/pattern in `src/http/routers/v1/hooks.rs`); worker-only compatibility must not silently make a new remotely reachable endpoint public. Legacy no-token mode is limited to the already-supported local transport and local operator view, with no new cross-project access. Work routes stay on the local transport plus the existing bearer credential; there is no successor milestone that opens them to other hosts. A model-scoped HTTP request also resolves an existing caller chat rather than trusting a spoofable root ID as authority.

Cross-project recall is denied by default on model-facing tools. A later explicitly scoped grant identifies source and destination projects, read versus control permissions, requesting principal and expiry; a guessed trajectory ID, common user directory or common model is not a grant. Same-project recall returns source trajectory/range/revision/time, labels content historical and untrusted, and never inherits operational instructions from it.

### 4.4 Automatic awareness

At the first prepared model turn, and after a material state change at a safe later boundary, add a project-work context capsule:

- Cap: 1,600 tokens, at most 24 rows (`capsule_token_budget`, `capsule_max_rows`); prioritize actual physical-file overlap, waiting decisions and active work.
- Include sanitized title/IDs/state/location and relevant approved relative paths. Never automatically include another chat's transcript, raw tool output or reasoning.
- Count overflow and provide a tool cursor; do not inline large registries.
- Include source timestamp and scope. Empty means "no observed work in this engine/scope", not "safe to edit".
- Exclude self and duplicate child summaries. Debounce repeated updates and avoid mutating cached historical prefixes or repeatedly appending identical notices.
- Recheck privacy when the model/provider, worktree or project changes. Fully disallowed rows are omitted without leaking their titles or sensitive counts.
- Capsule defaults on when M1 is released, with user opt-out. Semantic knowledge summaries remain a separate, labeled channel; do not reverse current-root exclusion. Align retrieval defaults with the user's choice: automatically injected memory may include vetted knowledge/session summaries and source references, but another trajectory's detailed/raw message excerpts require explicit retrieval. Preserve an explicit user opt-in to richer automatic recall rather than silently retaining it as the new default. Apply provenance and destination privacy to both paths.

Material changes are work start/terminal transition, queued↔running↔parked changes, a new/answered approval or question, a new physical-file conflict, changed declared scope and checkout/merge relocation. Progress-text-only updates and heartbeats do not trigger another capsule. Debounce default is 5 seconds, delivering at most one coalesced capsule at the next legal prepare boundary; a write-blocking conflict is also returned directly by the edit tool and does not wait on the debounce. Append notices after the immutable cached prefix, never rewrite past message bytes; provider cache-stability and duplicate-injection tests are mandatory. A policy/model switch forces re-evaluation before the next outbound request even inside the debounce window.

Top-level chats also record their work state and tool activity. Introspection that only registers subagents fails M1.

### 4.5 Shared scheduler semantics

Release defaults as implemented in `crates/refact-work-core/src/settings.rs` (configuration, not hard-coded product assumptions):

- 32 active background execution permits per engine, shared across both runtimes.
- 16 active permits per project and 16 per root tree, with effective limits being their intersection.
- 1,024 pending admission requests per engine; reject excess explicitly instead of allocating unbounded coroutine/task state.
- Keep the existing effective depth behavior; expose its configuration without accidentally adding or removing a level through off-by-one changes.
- Goals remain unlimited by default. No hidden cost/token cap is introduced by M1.

A background execution permit covers an admitted agent's active run, not merely its launch tool call. Interactive top-level chats are visible but are not charged against this background pool in M1; provider-request limits including interactive traffic arrive in M4. Report this distinction in settings and metrics.

Use weighted round-robin between roots, FIFO within each eligible lane, and bounded aging/priority for resuming work. Do not allow strict resume priority to starve new work indefinitely. A project cap cannot be evaded by spawning from the other runtime.

**Spawn sequence:** validate permissions/model/depth/scope → reserve a bounded queue ticket and idempotency key → create a queued record/claim using the runtime adapter → return the stable work/agent/chat ID → owned dispatcher obtains an execution permit → prepare worktree and launch → update active status. No worktree/process/model request before admission. Queued responses truthfully omit not-yet-created paths and supply status. A retry with the same launch key returns the same record.

**Board adapter:** preserve card identity and workflow semantics. Add an additive execution-state/queued marker tied to `agent_chat_id` so a queued card is not spawnable a second time. Do not add arbitrary new board columns or mark queued work complete. On queue/record failure, rollback only if the claim still belongs to that request.

**Parking:** the runner owns permits. An agent waiting on a child, user approval, a tracked question, or a supported process/timer wait parks at a safe boundary and releases active capacity; before generating or executing tools again it reacquires through the same scheduler. Never release while parallel mutating tool calls are still running. Spawning itself does not await a child's execution, so a nested spawn cannot block its own parent's tool batch waiting for a slot. Instrument existing wait APIs and show `parked` separately from `queued` and `running`.

A runner-level suspension barrier tracks non-wait tool calls and registered wait operations separately. Do not wait for the entire parallel tool batch to finish before parking: a pending child-wait in that batch would prevent the child from ever receiving a slot. Park only after non-wait calls drain and every remaining operation is a recognized suspendable wait. Wake/completion bookkeeping can settle those waits without a permit; one runner-level reacquisition gates any new model request or executable tool. Deduplicate concurrent wakeups. Arbitrary opaque tools that hide a child dependency are not automatically parkable; reject unsupported dependency waits or use their bounded cancellation/timeout contract rather than promise deadlock freedom for them.

Do not introduce a separate small parked-session cap that deadlocks an already accepted dependency chain. In M1 parked residency is observable, new queue admission is bounded, and memory watermarks warn/reject further new work explicitly; checkpoint eviction and bounded residency arrive in M3/M4. A rejected new spawn has a terminal structured error, not a child ID the parent waits on forever.

Cancellation of a queued ticket creates no side effects. Cancellation/terminalization releases a permit exactly once, including panic, failed setup and rollback. Lowering a live cap drains existing activity without forcibly killing it; starts/resumes wait until compliant. Goal monitors and task monitors understand queued/parked states and must not classify intentional waiting as provider failure or stall.

#### Admission/permit state machine and wait ownership

| State | Owned resource | Legal exits |
|---|---|---|
| `ticketed` | One bounded outstanding/queue reservation; no execution permit | `queued`, `rejected`, `cancelled` |
| `queued` | Runtime record + queue ticket | `admitted`, `cancelled`, `interrupted` |
| `admitted` | Unique RAII execution permit, before setup | `running`, `failed`, `cancelled` |
| `running` | Permit held by runner supervisor | `parking`, `completed`, `failed`, `cancelled`, `interrupted` |
| `parking` | Permit retained while non-wait operations drain | `parked`, `running`, terminal |
| `parked` | Work identity + outstanding reservation; no execution permit | `ready_to_resume`, terminal |
| `ready_to_resume` | One deduplicated scheduler re-entry request | `running` after permit acquisition, terminal |
| terminal | No queue ticket or execution permit | New explicit attempt only |

A typed execution guard has one owner. Synchronous guard drop releases the in-memory permit and notifies the owned dispatcher; it does not spawn cleanup work from Drop. Supervisors store and join handles. One terminalization operation removes queued/re-entry tickets and marks the runtime adapter exactly once; cancellation racing with admission checks generation/state before setup. Model/property tests explore legal transitions and injected errors/panics, proving permit count equals the number of live guards and that no ticket becomes two runs.

No permit-holding helper may await child completion without registering a suspension point. Audit `spawn_and_wait` and every caller as part of M1-04; current scoped source search finds its callers in agent tests, not a proven live deadlock. Both it and any newly introduced sync convenience path use the runner's wait registration, rather than acquiring another independent guard. Test a full-cap population of parents all spawning/waiting, not only a single parent.

`allow_parallel` is only a scheduling hint, not proof of read-only behavior: a shell/MCP call marked parallel-capable may still mutate. The park barrier tracks every non-wait call until completion irrespective of that flag. Known wait calls may remain pending without an execution permit; their result bookkeeping cannot execute a fresh tool or model call until reacquisition. Do not require all tool-call IDs to disappear before parking, which would deadlock a pending wait.

Preserve current public wait defaults and maximums after verifying them in source. A timeout settles the wait outcome once and queues one resume request with original enqueue-age credit; it does not auto-poll, generate or consume goal turns while awaiting capacity. Fair lane scheduling bounds progress by admission opportunities, not a false fixed wall-clock response promise. Timeout and child completion racing coalesce into one resume. Add full-cap timeout/resume/cancellation tests.

#### M1 configuration and bounded-state contract

Use existing runtime-settings loading and validation, with a namespaced coordination section. Implemented defaults in `crates/refact-work-core/src/settings.rs`: `max_active_engine=32`, `max_active_project=16`, `max_active_root=16`, `max_pending=1024`, `max_outstanding=4096`, `recent_window_secs=3600`, `capsule_token_budget=1600`, `capsule_max_rows=24`, `max_read_stamps=16384`, `max_write_records=65536`, `lock_timeout_secs=30`, `lock_poll_interval_ms=25`. Validate active/queue/record limits as positive integers; zero does not mean unlimited capacity. GoalBudget zero/None semantics remain unchanged. Active-limit changes apply to future acquisition; never shrink or discard accepted state. Cancellation, resume and completion are not refused because a creation limit is full.

The outstanding-run ceiling bounds active + parked + queued resident work and rejects new spawns explicitly. Configuration precedence is explicit per-run narrowing (cannot widen administrator limits), per-project override, then user-global defaults, with existing environment override conventions applied by the settings loader and tested. Add `max_subchat_depth` defaulting to the current constant value of 5 and centralize admission through a predicate that preserves today's effective boundary at every call site; tests pin the old behavior before removing hardcoded guards. `debounce_secs` defaults to 5 and `min_interval_secs` to 60. Invalid settings are rejected, not coerced into unlimited execution. It is separate from goals' effort budgets and from the pending queue bound. Evicted read stamps mean `unknown_read`, not a fabricated current observation. Record eviction is deterministic LRU, with active write transactions pinned until completion. Sidecar lock files are stable keys and are not deleted while cooperating engines can hold them; metadata GC must not create two independent lock domains for the same path.

For new queued work, persist/record immutable validated launch arguments under the runtime's existing record and deduplicate by `(root, tool_call_id, batch_index, caller_request_id when supplied)`. The dispatcher never derives authority from an edited display summary. User-requested retries after a terminal failure create a new attempt ID; transport retries of the same accepted request do not. M1 accepts bounded launch intents but does not resume them after an engine crash.

### 4.6 File coordination and visible-read staleness

Track the bytes actually supplied to the model, not arbitrary internal disk reads:

- `ReadStamp`: chat/attempt, PhysicalFileKey, content digest, source kind (disk or editor buffer), visible ranges/completeness, observation time.
- `WriteRecord`: physical and logical file keys, writer chat/run/tool call if known, before/after digests, time and checkout. Initially process-local, bounded per chat/project; durable attribution arrives in M3.
- `FileActivity`: declared intent, observed read, observed write, external change, claim and integration overlap are separate concepts.

At built-in editing boundaries, resolve and authorize paths first, acquire an OS-backed cooperative lock on a stable sidecar key (not a lock on an inode that may be replaced), re-read, compare the applicable visible-read stamp/preimage, apply and record. Use an async-safe wrapper around blocking OS-lock operations with timeout and cancellation. For multi-path rename/patch operations acquire locks in canonical order and prevalidate all paths; state that current multi-file edits are not an atomic transaction: M1 prevalidates/serializes cooperating edits but adds no automatic multi-file rollback.

- A stale previously observed file produces a structured refusal naming the other writer when attribution is permitted, otherwise "external or unknown writer". Include a re-read action.
- Whole-file replacement of an existing file requires a current complete read (or explicit user-authorized overwrite path); partial reads cannot authorize blind whole-file overwrite.
- Contextual patch/replace operations can use visible context with exact preimage checks. Keep current useful matching behavior; do not reject unrelated work solely because another session declared the same file.
- Creating a genuinely new file uses an absence precondition; it cannot silently overwrite a file another writer just created.
- Apply the same protection to create, replace, regex, anchored, line, patch, rename/delete and undo paths where supported. Audit HTTP/IDE saves and buffer-vs-disk behavior explicitly; no bypass through a forgotten mutation tool.
- Preserve current permissions, encoding, newline and symlink/hardlink semantics. Atomic replacement/crash durability is separately qualified in M3; no claim that compare-and-write alone is crash-safe.
- Sidecar locks serialize cooperating Refact writers across local processes. They do not constrain arbitrary shells, editors, formatters, processes on other machines or older Refact versions. Rechecking a digest reduces external-race risk but cannot make non-cooperating writes atomic.

Same physical file + stale read is a write conflict. Same logical path across separate worktrees is an advisory integration overlap. No OS-wide lowercase normalization; symlink and hardlink alias behavior gets platform tests. Retain unknown coverage rather than inventing attribution from a shell command string. Filesystem watchers can invalidate digests but cannot identify a writer reliably.

Concrete critical-section rules: lock keys include the canonical parent+leaf path (also for absent files) and an existing file's stable platform file ID when available, so hardlink aliases share an additional identity lock. Resolve the complete key set, acquire in sorted order, then re-resolve/re-stat; if the identity or alias set changed, release and retry a bounded number of times rather than adding locks out of order. Deny unsupported unsafe alias mutations instead of claiming coverage. On supported local filesystems, preserve the current link semantics; future atomic-replace work must requalify identity handling. Use a strong content digest such as SHA-256 or an existing suitable project hash implementation, never mtime alone. A write updates this chat's read stamp only when the resulting contents are actually exposed in its tool result or otherwise available under the tool's documented read contract; an internal write is not automatically a complete model-visible read.

### 4.7 Board mutation and cross-process claim safety

Reuse the existing board lock/atomic-write paths, but extend the entire read–validate–mutate–save transaction with a stable OS sidecar lock shared by all cooperating processes:

1. Acquire in-process board mutex, then OS sidecar lock with bounded wait.
2. Reload current board under the lock.
3. Validate optional expected revision and card/agent claim state.
4. Mutate and increment the existing revision; write atomically.
5. Release locks before emitting events, launching agents, creating worktrees or waiting on coordination.

Every board writer, including direct save, restart, repair, merge, GUI update and cancellation, must use the same transaction helper. Merely comparing `rev` before an unlocked rename is not cross-process CAS. Do not add a TTL that lets a still-running old worker lose ownership silently. Full lease fencing comes in M3.

Enforce this structurally: make the raw board persistence primitive private and require a non-forgeable `BoardTransactionGuard` for writes. Public updates take an expected revision or a mutation closure; callers cannot save a previously loaded whole board outside the transaction. Keep task initialization as a separate create-only guarded path. Move planner-sequence allocation (`next_planner_chat_id`) into the same canonical task transaction or reserve a unique attempt ID atomically, preserving legacy ID parsing. Both lock lookup and sidecar location derive from the canonical physical task directory, not an arbitrary task-id string or user path spelling; symlink/case aliases must share one lock.

M1 transaction atomicity means concurrent cooperating writers cannot interleave the read/claim/write sequence. It does not claim power-loss durability: audit existing tmp/rename helpers, report platform-specific remove-then-rename windows and lack of fsync honestly, preserve recoverable backups where necessary, and implement crash-durable replacement in M3. Add failure injection so a failed write is never acknowledged as a successful claim.

Use idempotent claim/request IDs and conditional rollback. If claim succeeded but the scheduler/launch failed, reconciliation marks the known request failed and clears only its own claim. Existing contradictory states are surfaced for repair; do not guess which agent to kill.

Cooperative-lock rollout requires no mixed-version board writers on a shared workspace. Provide a capability/version marker and a migration warning; full protection cannot be claimed against binaries that ignore it. Network filesystem lock guarantees are excluded from M1 and must be detected/documented rather than assumed.

### 4.8 Concrete change units and file targets

All paths below are relative to `refact-agent/engine/` unless prefixed otherwise. New filenames are proposed implementation targets; existing symbol anchors are more durable than line numbers.

| ID | Change | Primary targets | Depends on |
|---|---|---|---|
| M1-01 | Shared identities, work projection and capability/coverage contract | new `crates/refact-work-core/{Cargo.toml,src/lib.rs,src/types.rs}`; workspace manifests | — |
| M1-02 | In-memory scheduler with queue tickets, cancel, fairness and park/resume | new `refact-work-core/src/{scheduler,tests}.rs` | 01 |
| M1-03 | Engine service ownership and read-only catalog adapters | new `src/coordination/{mod,catalog,chat_adapter,board_adapter}.rs`; `app_state.rs`, `global_context.rs`, `background_tasks.rs` | 01 |
| M1-04 | Chat-agent scheduler integration and truthful queued responses | `src/agents/{spawn,monitor}.rs`; `crates/refact-agents-core/src/{registry,types}.rs`; `src/tools/{tool_subagent,tool_background_agents}.rs`; `src/subchat.rs` | 02,03 |
| M1-05 | Board transaction lock and conditional claims | `src/tasks/storage.rs`; `src/tools/tool_task_spawn_agent.rs`; all save/restart/merge callers; `crates/refact-tasks/src/types.rs` if additive fields needed | 01 |
| M1-06 | Board scheduler integration and safe waiting/monitor state | `tool_task_spawn_agent.rs`, `tool_task_batch.rs`; `src/chat/task_agent_monitor.rs`; board wait/approval paths | 02,03,05 |
| M1-07 | Read/write activity context and cooperative edit transaction helper | new `src/coordination/file_activity.rs`; `src/tools/file_edit/{auxiliary,undo_history,tool_apply_patch,tool_create_textdoc}.rs`; read/render/postprocessing and other mutation entry points | 01,03 |
| M1-08 | Work tools, indexed HTTP projection, permissions and subscriptions | new `src/tools/tool_work_overview.rs`, `tool_work_inspect.rs`; `tools_list.rs`; new router; trajectory/worktree listing adapters | 03,07 |
| M1-09 | Startup/relevant-overlap capsule and source labeling | `src/chat/generation.rs`, `src/chat/system_context.rs`, `src/knowledge/enrichment.rs`, `tool_subagent.rs`; shared delivery helpers | 08 |
| M1-10 | Minimal GUI Work panel, queue/attention/overlap display and settings | GUI new `features/Workspace/WorkPanel/`, `services/refact/work.ts`; existing BackgroundAgentCard, Worktrees, Tasks, settings and subscriptions | 04,06,08,09 |
| M1-11 | Integration, adversarial race tests, docs and staged enablement | engine `tests/` and existing agents/board/file tests; GUI unit/integration fixtures; user documentation | all |

Read adapters must not call worktree-list operations with hidden destructive cleanup as a side effect. Extract pure inventory where necessary. Do not add full-record scans to generation or feed private raw records directly into tool output.

### 4.9 Deterministic acceptance tests

M1 is complete only when all scenarios pass in both runtime adapters where applicable:

1. **New-chat awareness:** chat A and board agent B run in project P. New root C sees both with truthful state, permitted titles, checkout and relevant overlap; it does not need their IDs first.
2. **Privacy isolation:** a different project or privacy-incompatible target never leaks through title, path, progress, count, capsule, pagination or subscription. Switching model re-evaluates disclosure.
3. **No history scan on each turn:** large synthetic history with thousands of entries still answers via indexed summaries; spy counters prove no per-trajectory body reads in the hot path. Performance timings live in the separate performance suite.
4. **Real admission:** 12 non-finishing fake runs across both adapters with cap 4 yield 4 executing and 8 queued; no queued worktree/model/process side effects. A second root receives capacity fairly.
5. **Nested waits:** with one active permit, a parent launches a child and waits; parent parks, child runs, parent resumes. Test mixed-runtime waits, approval, timeout, cancellation and multiple parallel tool calls still draining.
6. **Idempotency/cleanup:** duplicate spawn, queue-full, worktree failure, panic, cancelled queue and failed board write produce no leaked permit, duplicate worker or orphaned claim.
7. **Intentional waiting:** queued/parked/approval states do not trigger stuck-agent interruption or repeated goal nudges. A paused user goal stays paused.
8. **Visible-read protection:** A reads; B writes; A's edit fails with stale detail. Run all mutation families, partial reads, new-file races, rename/delete and undo. Test buffer/disk divergence.
9. **Cooperating processes:** two subprocesses race on one edit/board claim using real lock helpers and barriers; one claim wins, neither silently overwrites a stale source. Use temporary repos, no live user data.
10. **Worktree distinction:** same relative file in separate worktrees is an integration warning, not a physical lock conflict. Aliases of the same physical file cannot bypass critical-section serialization.
11. **Freshness and restart:** a disk-only `running` row is not reported as a live runner. Restart reports interrupted work honestly; removed worktrees remain historical, not active.
12. **UX:** GUI and tool results agree; zero/one/many work items, queued, parked, stale, unavailable, denied and overlap states render; SSE gap/reconnect and existing sidebar pagination remain correct.
13. **Compatibility:** existing trajectories/cards/agent records load; root-scoped tool behavior and old aliases remain; target-file scope descriptions do not claim enforcement beyond actual tool paths.
14. **Shutdown:** all waiting tasks wake/cancel, no monitor is detached, both background-task wiring paths are covered.
15. **Structural board authority:** raw persistence is inaccessible outside the guarded transaction; compilation/API tests and a source-call-site inventory prove no direct-save escape. Concurrent planner-ID allocations under canonical/symlink-alias task paths remain unique.
16. **Settings and depth:** precedence, invalid values, live cap reduction, queue/resident bounds and unchanged effective default nesting are pinned by tests. Explicit finite goal budgets and unlimited zero/None goal defaults both survive migration.

Use fake providers/runtimes, controlled barriers and fake clocks; no paid model calls or flaky wall-clock assertions in correctness tests. Run targeted Rust tests first, then `cargo check`, `cargo test --lib`, doc tests and relevant integration binaries. GUI changes require types, zero-warning lint, formatting and Vitest, plus deterministic mocked-browser scenarios. Use existing component-specific check tools. Record actual exit status; blocked/incomplete checks are not passes.

### 4.10 Migration and release

Implement in isolated worktrees only after approval. Start with identities/catalog and scheduler tests; integrate adapters serially where files overlap. First run read-only discovery and metrics in shadow mode, then enable queue enforcement in development, then file/board protection, then the capsule/UI. Ship the complete M1 contract together after acceptance, not a default-on half-integrated safety feature.

Configuration/schema additions are backward-readable. Existing finite budgets remain explicit; none are overwritten. A scheduler-disable rollback stops new admission and drains/reports accepted work before returning to old behavior; it must not launch queued work twice. File/board lock rollback requires all writers to agree on capability version. Retain existing stores and migration backups; no mass trajectory rewrite.

## 5. M2 — Interaction, shared work and scoped authority

Extend the existing delivery path with a stable envelope: message ID, sender/recipient WorkKey, run generation, correlation/question ID, message kind, delivery mode, wake policy, TTL, authority source and receipt. Preserve structured tool-result pairing and provider thinking signatures.

- Separate informational mail (`wake=false`), follow-up work (`wake=true`), safe-boundary steering and explicit preemption. Existing append/when_idle/preempt map into this contract. Expose accurate queued/delivered/consumed/expired/superseded/failed receipts; handle a steer arriving after the last boundary as undelivered, never silently lost.
- Parent/child/descendant and consented peer messaging, team broadcast with dedup/rate limits, mailbox listing and await-any updates. Discovery does not authorize arbitrary cancellation. Unrelated roots in the same project can exchange permitted informational mail; preemption/mutation requires explicit control authority.
- Generic tracked questions can block safely with timeout, cancellation and durable correlation. Parent plan approval and permission escalation use the same question service but distinct decision types. A parent cannot grant authority it does not possess.
- Standardize status/result/wait/cancel visibility across the authorized tree, while preserving current tool names. Add close/resume and continued-agent turns without misleading tool-call-as-completion semantics.
- Separate reusable agent profile (model, tools, role, skills, effort, hooks, scope) from run instance. Support fresh, shared-completed-prefix/fork and resumable contexts, with prompt-cache preservation and privacy gating.
- Enforce explicit path scope on built-in mutations, tool allow/deny lists at both visibility and execution, inherited deny ceilings and sandbox policy. `target_files` remains declared intent unless strict edit scope is active; shell/MCP authority requires actual sandbox/integration restrictions. Never market an edit-tool allowlist as a shell sandbox.
- Add explicit team/task board discovery and atomic claims, dependency checks, owner/reviewer role validation and safe handoff. The board stays the board runtime's authority; teams may reference cards rather than create a shadow copy of them.
- Extend existing hooks: tool pre/post, permissions, session/agent start/stop, compaction pre/post, user input, notification and goal/verification events. Trusted command/MCP/prompt/agent handlers, synchronous control vs asynchronous observation, deterministic deny precedence, rewritten-input revalidation and recursion caps. Untrusted project hooks are not auto-executed. M10 exposes hook activity, denials and rewrites in the observability surfaces.

Gate: one root can discover a permitted peer, send a correlated question, wait without occupying execution capacity, receive an answer exactly once at the logical layer, and cannot use that address to exceed permissions or overwrite the peer's work.

## 6. M3 — Durable presence, recovery and integration

Add a versioned per-user **local** coordination store (SQLite using existing dependency versions), with project-scoped authorization and durable records for engine instances, run intents/generations, queued work, leases, delivery outbox and observed activity. Existing trajectories and boards are not replaced. Do not place a SQLite WAL database on a shared network filesystem and call it coordination consensus; this store is local to one user on one machine.

- Engine presence includes PID, process-start identity, host identity and heartbeat. Distinguish alive-but-idle, unreachable and dead; PID reuse is not proof of ownership.
- Durable admission and claims use atomic transactions, owner tokens and fencing generations. A worker whose lease expires loses authorization to perform coordinator-mediated actions. Reclaiming a lease is not sufficient protection from stale writers unless their writes are fenced too.
- Explicit launch protocol across coordination store and board/trajectory stores: persisted intent → conditional runtime record/claim → acknowledged launch → terminal outbox. Use idempotency and recovery reconciliation; there is no cross-store transaction by assertion.
- Outbox/inbox delivery is at-least-once transport with message-ID dedup for exactly-once logical insertion, not exactly-once external side effects. Pending operations whose external outcome is unknown require inspection/approval before retry.
- Resume/cold-resume from persisted completed boundaries; fresh/fork/resume preserve identities, tool permissions, approval provenance, budgets and goal revisions. Never automatically arm an old autonomous goal merely because a session was loaded. Resume previously authorized work only under a persisted resume policy; explicit pauses remain pauses.
- Persist file last-writer/read-stamp provenance with bounded retention and deletion controls. Separate observed activity from explicit cooperative file/workspace leases. Provide stale-holder diagnostics and manual, authorized takeover.
- Serialize merges per target checkout/repository, compare dirty baselines, rebase/reverify where required, preserve failure worktrees, and implement a bounded resolver workflow. Reverify merged output; a passing isolated branch is not proof the combined checkout passes.
- Qualify crash-safe file replacement separately: temporary file, flush policy, atomic rename and metadata/encoding/link handling. Multi-file patch commits are journaled or explicitly partial, never advertised as atomic without implementation.
- Add durable summary indexes for who worked on/touched a file, source message ranges and omission artifacts. Rebuild in the background; no bulk parse at chat startup. Historical records are labeled historical.

Gate: kill the owning engine between each launch/delivery/claim/merge boundary; restart does not double-run, silently replay unknown writes, lose acknowledged messages or report dead work active. Two local engines share limits and claims without split-brain. Coordination stays inside one machine; no cross-machine successor is planned.

## 7. M4 — Capacity limits, fairness and adaptive execution

- Scope limits independently: machine/host, engine, provider credential/rate bucket, project, root tree, team, resident sessions, active runs, concurrent model requests, subprocesses and verification jobs.
- Aggregate whole-tree turn/token/time/cost/spawn budgets are out of scope, together with per-child reservations and reconciliation, monetary ceilings, cached-usage double-counting rules and pricing exposure. Usage may be measured and displayed; it is never enforced as a spend ceiling.
- Capacity limits are not budgets. They bound how much work runs at once — concurrency, queue depth, resident sessions, request rate — and never terminate work for exceeding a token, time or money total. GoalBudget stays unlimited by default, and M4 introduces no implicit cap on it. New explicit limits preserve provenance and can be relaxed/resumed by authorized users.
- AIMD uses provider-specific concurrency/rate-limit signals: halve on qualifying pressure, honor Retry-After, grow after sustained success, never exceed configured ceilings. Separate quota exhaustion, auth failures, malformed requests and real 429 pressure. Jittered bounded retries, circuit breakers and explicit cooldown reason.
- Fairness, starvation tests, backpressure, shared-request vs active-agent metrics and parked-session checkpoint/eviction. Live user input gets priority without starving background work permanently.
- Distinguish no activity from no material progress: repeated identical tool/input/output, unchanged evidence/worktree, blocked reasons, provider transport failures and deliberate wait barriers. Do not abort a legitimately slow stream based only on a generic short silence timer.

Gate: fake quota/rate/error traces prove the limiter adapts only to the correct signals, never exceeds its configured capacity limits, yields fairly and cannot lose track of active child work.

## 8. M5 — Evidence-backed goals and bounded durable plans

### Explicit verification contract

Add `VerificationSpec` alongside `verify_hint`, not by changing the hint's meaning. Fields include spec ID/revision, criterion IDs, required/optional, command argv or separately authorized shell form, confined cwd, environment allowlist, timeout, output cap, expected exits, artifact expectations/assertions, toolchain/dependency inputs and authorization source. Commands from a child report remain untrusted proposals until accepted under policy.

Extract/reuse the existing board verification runner and post-merge runner through a shared evidence service. Tests/builds can write outputs or execute repo code, so use an appropriately isolated environment and the existing command/privacy policy; an allegedly read-only verifier profile is not sufficient.

- Gates run before the semantic judge. Required failing/missing/unrun gates prevent completion. `passed`, `failed`, `blocked`, `unavailable`, `not_run`, `superseded` stay distinct.
- Store executed command, exit, bounded logs/artifact references, environment/toolchain fingerprint, start/end and exact worktree/commit/content revision. Verify expected artifacts and assertions, not merely the existence of a tool call.
- Cache only evidence with declared deterministic inputs; include file contents, untracked inputs, command/env/dependencies/toolchain and verification policy. Network/time-dependent tests default non-cacheable. Replaying a cached failure does not pretend a command ran again. Revalidate after merge.
- Configurable independent reviewer model/tool policy, adversarial probes, evidence-backed per-criterion verdicts, bounded repair/escalation and parent spot-check. A different model is additional review, not mathematical independence or guaranteed truth.
- Extend existing epoch/trajectory guards to spec, goal, filesystem and artifact revisions; stale approval/verdict cannot complete changed work.

### Goals/plans

- Structured sub-goal DAG with ownership/delegation, required vs optional edges, child evidence references and parent aggregation. Parent cannot infer success from a child's free-text `DONE`.
- Typed `wait` verdict/barrier on runtime process identity, child result, CI run/event or deadline. No goal rounds/model calls while waiting; recheck on event. Pauses/snoozes/cancellation purge queued nudges and superseded verification jobs.
- Durable revisioned approval contract, execution steps/todos and user-edited plans. Preserve a single active top-level goal/plan interface while permitting explicit child-goal composition.
- Bound provider-visible plan/goal context through explicit checkpoints: immutable approved base and audit deltas retained on disk, deterministic current projection/checkpoint with approval provenance injected. Do not silently rewrite approved intent or simply drop `Never`-preserved messages. Audit history stays recoverable; old trajectories migrate non-destructively.
- Goal restore has separate persisted status and authorization-to-autocontinue. Configurable goal limits remain unlimited by default.

Gate: fake success text cannot defeat a failing required gate; skipped verification cannot become PASS; changing a file/spec/goal invalidates stale success; waiting consumes no autonomous rounds; context remains bounded over hundreds of deltas without losing approved constraints.

## 9. M6 — Teams, work DAGs and structured convergence

- Reusable named teams and roles with permitted peer mail, shared tasks/dependencies, claim/heartbeat/reclaim, mission log and a consistent live roster. Preserve separate chat and board adapters.
- Parallel/sequential/pipeline/wave execution with dependency and predicted file-overlap ordering; declared and observed conflict data both feed scheduling. Repeated agent IDs in a pipeline must advance by position, not first-match lookup.
- Typed assignment TASK / DELIVERABLE / SCOPE / VERIFY generated from the same accepted contract later validated. Optional undispatched lanes are explicit; an unrun lane is never "found nothing".
- Structured output schemas with at most a configured bounded correction retry, context-headroom-aware result budgets and recoverable omitted artifacts.
- Outcomes with declared required sections, source-run/evidence provenance, authenticated review ownership and finalize gate. Self-review is explicitly marked or forbidden by policy; unresolved contradictory/rejected required fragments block finalization, unlike a weak "one approved fragment exists" check.
- Governed code/workflow mode for model-authored orchestration: schema-described host tools, confined interpreter/worker, no ambient fs/net/eval authority, cancellation and capacity limits. Worker threads are not a security boundary; OS isolation when needed. Durable DAG execution is implemented as a service used by the existing runtimes, not a third product workflow.
- Context isolation and memory: fresh/forked/reused team members, project summaries/decisions/corrections, provenance-aware history search, per-agent private vs shared memory. Prevent stale/foreign text from becoming instructions. Eventual structured result+artifact convergence is distinct from git integration.

Gate: a mixed-runtime DAG with fan-out, failed/optional lane, dependency, review and merge cannot finalize before all required verified outcomes; messages and claims remain correct across restart.

## 10. Adopted collaboration additions

Eight additions were adopted with the scope amendment. All are local, reuse the shared services above and add no new runtime. Each has a coverage-ledger row below and is complete only under the same completion definition.

| ID | Addition | Slot |
|---|---|---|
| A | Wait-graph deadlock detection | M2 graph; M4 detector |
| B | Handoff sufficiency feedback | M2 transport; M5 evidence link |
| C | Human attention budget | M2 accounting; M10 surface |
| D | Structured disagreement and adjudication | M5, M6 |
| E | Shared fact ledger with contradiction detection | M6 |
| F | Cross-agent causal trace and deterministic replay | Recorded from M1; viewer in M10 |
| G | Decomposition advisor | M4, M6 |
| H | Value-based priority | M4 |

- **A — Wait-graph deadlock detection.** Every wait edge — child, approval, tracked question, cooperative lock, permit — is recorded in the M2 interaction graph; M4 runs a bounded cycle detector over it. A detected cycle produces a structured diagnosis naming the edges and an authorized break action (cancel, park or escalate to the user), never an automatic kill chosen by the detector. Unsupported opaque dependency waits are reported unknown, not proven deadlock-free.
- **B — Handoff sufficiency feedback.** A receiving agent scores the handoff it was given against the accepted contract: missing files, unstated constraints, absent evidence, ambiguous acceptance. M2 carries the feedback to the delivering work; M5 attaches it to that run's evidence so later decomposition can use it. Feedback is advisory and never silently rewrites an accepted contract.
- **C — Human attention budget.** Every user-facing interrupt — approval, tracked question, notification — is a counted, deduplicated request carrying a reason and urgency in M2, and M10 shows the budget with its current consumers. Exceeding the budget coalesces or defers low-urgency interrupts only; it never auto-answers a question, auto-approves a decision or drops a required approval.
- **D — Structured disagreement and adjudication.** Conflicting required outcomes become a typed disagreement record (claim, evidence references, confidence, author) instead of a last-writer-wins overwrite. M5 supplies the evidence references; M6 routes the record to an adjudicator role and stores the decision with its rationale. An unresolved required disagreement blocks finalization.
- **E — Shared fact ledger.** M6 keeps a project-scoped ledger of asserted facts with source run, evidence reference, observation time and validity window. Contradiction detection flags conflicting assertions about the same subject. Each fact is labeled asserted, verified or stale; an unverified or stale fact is context, never an instruction, and never replaces a verification verdict.
- **F — Cross-agent causal trace and deterministic replay.** From M1 onward every admission, delivery, claim, edit, verification and terminal transition records its causal parents and a monotonic sequence; M10 ships the viewer. Replay reconstructs the recorded decision sequence and does not re-execute side effects; effects that were never recorded are reported unknown rather than reconstructed.
- **G — Decomposition advisor.** M4's observed overlap, wait graphs and queue behavior, together with M6's handoff feedback, produce proposals for a different split: more or fewer lanes, different boundaries, different ordering. Proposals are advisory and require planner or user acceptance before any DAG changes.
- **H — Value-based priority.** Admission accepts an explicit priority/value label per run and uses it as a lane weight inside M4's existing fairness rules. Priority reorders admission opportunities only; it never raises a cap, cancels other work or starves a lane, because aging still applies.

## 11. Coverage ledger for everything researched

Each row becomes an executable parity scenario during its milestone; this is the scope checklist, not proof it is already implemented.

| Capability family | Research inspiration / evidence category | Delivery |
|---|---|---|
| New-chat live project overview; unrelated-root work; attention/current tool | Kilo Agent Manager; Claude PID registry; Refact existing session/worktree projections | M1; cross-process M3 |
| Historical search/read, worktree-family scoping, source ranges and untrusted labels | Kilo recall; Codex indexed listing; Refact knowledge/trajectory search | M1 boundaries; M3 indexes; M6 recall |
| Shared project memory, session digests, corrections, consolidation, recoverable omitted output | Claude memory; Hermes locked memory; Kilo memory; Repowise omission references | M3, M6 |
| Read stamps, named writer warnings, per-path serialization, integration overlaps | Hermes file state; Claude stale reads; OpenCode edit mutex; Refact preimages/worktrees | M1; durable M3 |
| Cross-process claim, lease/fencing, stale-holder/PID-reuse detection | Claude atomic task claims; Codex internal leases; Hermes session leases; Ouroboros worktree ownership | M1 board lock; M3 |
| Active concurrency vs launch cap; fair queues, parked parents, structured errors | Cline team queue; Codex admission; OpenClaw reservations; Refact separate stacks | M1, M4 |
| Per-provider AIMD, Retry-After, quota handling and rate/capacity limits (no tree cost/token accounting) | Ouroboros adaptive control; Codex shared token budget; Cline cost telemetry | M4 |
| Parent/child/peer live mail, broadcast, no-wake vs follow-up, interrupt, missed steer | Codex; Cline; Hermes; Claude; Ouroboros Synapse; current Refact delivery | M2, durable M3 |
| Generic questions/waits, plan/permission escalation, graceful shutdown | Codex mailbox wait; Claude approval; Refact tracked questions | M2 |
| Fresh/fork/full or bounded history, resumable/cold children, background promotion/extension | Claude fork; Codex; Kilo/OpenCode jobs; DeepSeek continuable agents | M2, M3 |
| Profiles/personas, tool/model/effort selection, deny ceilings, mode/file constraints | Roo modes; Claude profiles; ProxyAI Task; OpenCode/Kilo permissions | M2 |
| Worktree creation, inventory, nested integration, post-merge validation, conflict resolver | Refact existing merge; Kilo AM; Ouroboros coordinator; Hermes retention | M1 inventory; M3 integration |
| Typed assignment, task DAG/claiming, parallel/sequence/pipeline/waves | Ouroboros assignments; Cline teams; Kilo waves | M6 |
| Structured child schemas and correction retry; mission log; reviewed outcome sections | Hermes output schemas; Cline outcome protocol | M5 evidence; M6 convergence |
| Explicit durable goals, unlimited defaults, pause/resume/quiescence, CAS revisions | Refact; Codex; DeepSeek; Antigravity changelog only | Preserve M1; strengthen M5 |
| Executed gates/artifacts/assertions, adversarial review, stale verdict guards, cached evidence | Hermes gates; Ouroboros evidence; Claude reviewer; Refact board runner | M5 |
| Wait barriers on child/process/CI/deadline, no wasted continuation | Hermes goal wait; Refact process/scheduler/snooze infrastructure | M2 wait; M5 goal integration |
| Plan approval, files/task state, bounded compaction-safe current plan + full audit | Refact hidden roles; Roo/Claude/OpenCode plan artifacts; DeepSeek event plan state | M5 |
| Hook control events/types and safe extensions | Codex and Claude; Refact existing hooks | M2 control; M10 visibility |
| Governed code-mode orchestration, tool catalog search, bounded intermediate data | Kilo CodeMode; DeepSeek workflow engine; Refact lazy catalog | M6 |
| Human surfaces and diagnostic visibility | All applicable runtimes; Refact GUI/TUI/IDE clients | M1 minimal, M10 complete |
| Wait-graph deadlock detection with authorized break actions | Adopted addition A | M2 graph; M4 detector |
| Handoff sufficiency feedback into later decomposition | Adopted addition B | M2, M5 |
| Human attention budget for approvals, questions and notifications | Adopted addition C | M2, M10 |
| Structured disagreement records and adjudication decisions | Adopted addition D | M5, M6 |
| Shared fact ledger with contradiction detection and trust labels | Adopted addition E | M6 |
| Cross-agent causal trace and deterministic replay of recorded decisions | Adopted addition F | Recorded M1; viewer M10 |
| Decomposition advisor proposals requiring acceptance | Adopted addition G | M4, M6 |
| Value-based priority as a fairness weight in admission | Adopted addition H | M4 |

Continue/Repowise are not assumed to be agent orchestrators. Antigravity internals are unavailable; cover documented custom-agent/model/nesting/goal/background lifecycle behavior with our own contracts. Unreached DeepSeek introspection paths remain evidence gaps to close before their parity claim; the capabilities they inspired that survive the scope amendment are covered above, and the external-backend rows they also inspired are recorded as dropped rather than quietly claimed.

### Dependency and guarantee clarifications

- M3 supplies **execution checkpoints** for parked-session residency control in M4. M5's approved plan/goal projection checkpoint is a different feature; M4 does not depend on rewriting or compacting approved plan text.
- M4's capacity limits are not budgets. An unlimited GoalBudget is never overridden by an admission cap, and no milestone enforces a token, time or monetary ceiling on a work tree.
- Workflow/DAG support in M6 has no separate session identity, transcript store, queue or autonomous permission policy: it submits runs through a chosen existing runtime adapter and shared services. This enforces the two-runtime decision beyond a naming convention.
- M1 cross-process stale-write failures identify an **unknown/external writer**, even if the other process is Refact; named cross-process attribution arrives in M3. M1 only claims serialization for cooperating local writers, never global file ownership.

## 12. M10: qualification, observability and completion definition

Throughout the program, expose the same concepts in tools, HTTP, GUI, TUI and IDE clients: queued/running/parked/stale, waiting questions, capacity-limit reasons, controlled peer actions, workspace/file attribution, merge state, evidence verdicts, hook activity, attention-budget consumers, causal trace and recovery actions. Prefer extending existing panels/routes over a parallel dashboard. Metadata is cheap and bounded; transcript/artifact bodies load on demand.

Add structured metrics and traces for queue latency, permit counts, provider pressure, unknown attribution, stale-write rejections, delivery lag/duplicates/expiry, abandoned claims, recovery decisions, evidence cache validity, generation progress and context payload size. Metrics omit secrets. Log an auditable explanation for automatic pause, denial, retry, merge refusal and goal completion.

Program-wide test matrix:

- Chat and board runtimes; root/child/peer; one/two engines; same/separate checkout; git/non-git; Linux/macOS/Windows; approved/denied privacy destination.
- Active/queued/parked/cancelled/stale/restarted; absent/index-only/loaded histories; streams/tool batches in flight; lost notifications and duplicate completion.
- Unit state-machine/property tests, deterministic fake-provider integration, real cooperative subprocess locks and browser interaction.
- Fault injection at persistence/claim/launch/execute/verify/merge/ack boundaries. Slow/memory/IO measurements belong to the explicit performance suite.

Completion requires each coverage-ledger row to link to an implemented path, a public/agent-facing reachability test, a failure-path test and an observed passing verification result. A type definition, passing unit test of an unused helper, UI-only feature when the agent needs it, or a marketing statement is not parity.

## 13. Approval and execution discipline

This document is the approved plan of record, as amended above. Its defaults and behavior changes are the ones implementation must match; where code and this document disagree, report the discrepancy and fix whichever is wrong rather than letting them drift.

After approval: install the approved revision as execution ground truth, start M1-01, run targeted tests after each unit, perform spec review before quality review, and run combined component gates before each milestone is considered complete. Later milestones receive similarly detailed implementation cards immediately before execution, within the scope and architectural decisions approved here. Newly discovered conflicts with this plan must be reported and resolved explicitly; full parity does not authorize unrelated rewrites.
