# Review swarm pipeline

## Overview

The `review(what_to_check, files, depth)` tool runs a Rust-orchestrated, fixed stage graph. `what_to_check` is an optional focus, `files` are seed paths rather than a closed scope, and `depth` is `normal` or `deep`. If omitted, depth defaults to `normal` unless `review_swarm.default_depth` overrides it. Legacy `quick` and `standard` values parse as `normal`.

Agents have no wall-clock timeouts. Every agent instance runs behind an idle watchdog fed by its own monitored subchat channel: LLM streaming deltas (including thinking progress, throttled to 750ms), tool-call step markers, and verifier traffic all count as activity. An agent is killed only after `idle_timeout_secs` (default 240, floored at 30) with no observed activity, producing an `idle_timeout:no_activity_for_<N>s` coverage reason; A3 and A4 use `exec_idle_timeout_secs` (default 1800) because long silent test or build commands are normal for them. A killed static enrichment agent falls back to its raw deterministic findings with an `enrichment_idle_timeout:*` reason instead of losing them.

The stages are:

1. **Gather and scope.** A gather subagent expands the supplied files and conversation context into the review scope, changed-file set, and diff context, subject to configured file and token budgets.
2. **Mechanical preflight.** If `ReviewCommandsConfig` is enabled, its literal argv commands run through the centralized `review_evidence` foreground policy. A non-zero exit or execution failure stops the graph before any review agent runs. If disabled, the stage is recorded as skipped.
3. **Static swarm.** S1-S6 deterministic scans run concurrently. S1-S5 emit raw findings; S6 emits file-risk facts used during merge. Each S1-S5 scan that produced findings then becomes an enrichment agent: a tool-using subchat that receives the raw findings, searches the codebase with read-only tools (`cat`, `tree`, `glob`, `regex_search`, `symbol_def`, `semantic_search`), and proves, refutes, or enriches every raw hit. Confirmed findings become `verified` with the deterministic fact preserved as evidence, refuted findings are dropped with a `static_refuted:N` marker, untouched raw findings survive unchanged as `unverified`, and up to five directly related new discoveries may be added. Enrichment failure or timeout falls back to the raw findings with an `enrichment_failed`/`enrichment_timeout` coverage reason, so the deterministic signal is never lost.
4. **LLM and agentic swarm.** The depth selects fixed one-shot and tool-using agent instances. Static enrichment agents join the same bounded wave. Concurrency is bounded by `max_parallel`, and each family has a timeout.
5. **Per-instance evidence and verification.** Each L1-L3 instance plus A1 and A2 independently receives deterministic evidence and blind verification before its output joins the swarm. A3 and A4 collect deterministic evidence but skip the blind verifier because their execution/browser evidence is the verification. Static enrichment agents also skip the blind verifier: their findings are anchored by the deterministic fact plus the agent's own codebase investigation. The verifier sees the claim and evidence, but not reviewer rationale, prose, or candidate confidence.
6. **Merge and report.** Rejected findings are removed, nearby duplicates are clustered across agents, risk facts and A3 refutations are applied, stable IDs and rank tiers are assigned, and findings are sorted.

The result is Markdown followed by one fenced `json` block containing the complete `ReviewReport`.

## Depth levels

| Depth | Agents |
|---|---|
| `normal` | S1-S6 scans plus their enrichment agents (spawned only when a scan finds something); L1, L2, and L3 ensembles over their configured `chat`, `chat2`, and `thinking` slots; A1 repository-context exploration; A2 research. |
| `deep` | Everything in normal; A3 test execution and A4 browser review. |

The built-in default is `normal`. Disabled agents, missing prompts, unavailable facilities, and unresolved model slots produce skipped or failed coverage rows.

## Agent roster

| ID | Family | Checks | Evidence emitted | Skip conditions |
|---|---|---|---|---|
| `s1_security` | static | CodeGraph security rules over scoped files; enrichment agent judges exploitability, data flow, and fixture-versus-production context. | `static_fact`, plus `excerpt`/`diff_hunk`/`symbol` on enriched findings | `disabled`, `codegraph_unavailable` |
| `s2_dead_code` | static | Cross-file unreachable symbols above `min_confidence`; enrichment agent searches for dynamic dispatch, registrations, exports, macros, and feature-gated usage before confirming. | `static_fact`, plus deterministic evidence on enriched findings | `disabled`, `codegraph_unavailable`, `index_building`, `index_unavailable`, `dead_code_unavailable` |
| `s3_duplication` | static | Cross-file clone pairs intersecting the scope; enrichment agent separates true DRY violations from generated code, fixtures, and justified variants, naming the extraction target. | `static_fact`, plus deterministic evidence on enriched findings | `disabled`, `codegraph_unavailable` |
| `s4_test_integrity` | static | Deleted or skipped tests, reduced/weakened assertions, widened tolerances, harness and snapshot changes, and implementation/test literal overlap; enrichment agent checks whether deleted tests moved, skips are justified, and overlaps are hardcoded expectations. | `static_fact` plus `s4:*` markers, plus deterministic evidence on enriched findings | `disabled`, `no_diff` |
| `s5_dependencies` | static | Added Rust, JavaScript, and Python imports absent from an available dependency manifest; enrichment agent searches workspace manifests, lockfiles, vendored sources, and path dependencies before confirming a hallucinated dependency. | `static_fact`, plus deterministic evidence on enriched findings | `disabled`, `no_diff_patch` |
| `s6_git_enrichment` | static | Churn percentile, recent hotspot, fan-in, and ownership/bus-factor risk. It enriches other findings rather than emitting findings. | hot-file `static_fact` added at merge | `disabled`, `no_git_history_or_graph` |
| `l1_diff` | oneshot | General diff review across all candidate categories. | Deterministic `excerpt`, `diff_hunk`, and `symbol`; reused `command_output` where eligible | `disabled`, `prompt_not_configured`, unusable model slot, idle-timeout/failure |
| `l2_simplicity` | oneshot | Unjustified complexity, unnecessary abstraction or edits, needless dependencies, duplication, comment slop, and dead scaffolding. | Same deterministic evidence as L1 | `disabled`, `prompt_not_configured`, unusable model slot, idle-timeout/failure |
| `l3_spec` | oneshot | Reconstructs intent and checks missing requirements, scope creep, contradictions, half-migrations, and misread edge cases. | Same deterministic evidence as L1 | `disabled`, `prompt_not_configured`, unusable model slot, idle-timeout/failure |
| `a1_repo_context` | agentic | Repository conventions, sibling implementations, end-to-end wiring, cross-module consistency, stale references, and reuse. | Deterministic evidence from candidate locations, followed by blind verification | `disabled`, `prompt_not_configured`, unusable model slot, idle-timeout/failure |
| `a2_research` | agentic | Internal precedent, external reinvention, third-party API/version correctness, and dependency license or supply-chain concerns. | Deterministic evidence; researched facts summarized in claims | `disabled`, `prompt_not_configured`, unusable model slot, idle-timeout/failure |
| `a3_execution` | agentic | Builds, checks, related suites, targeted reproductions, mutation probes, reward-hack tests, repeated runs, and boundary/error paths. | `execution_output`, `mutation_probe`, plus deterministic evidence | depth below deep, `disabled`, `execution_disabled`, `prompt_not_configured`, unusable model slot, idle-timeout/failure |
| `a4_browser` | agentic | Changed UI routes and interactions, console/network failures, desktop/mobile rendering, and basic accessibility. | `console_log`, `screenshot`, plus deterministic evidence | depth below deep, `disabled`, `prompt_not_configured`, `chrome_unavailable`, unusable model slot, idle-timeout/failure |

## Candidate envelope and merge

Every LLM agent ends with a candidate envelope. Reviewer rationale is private generation context and is not copied into `ReviewFinding` or shown to the verifier.

```json
{
  "summary": "2-4 sentences",
  "candidates": [{
    "file": "path/as/provided",
    "line1": 1,
    "line2": 10,
    "category": "correctness|consistency|security|tests|maintainability|performance|spec_compliance",
    "severity": "low|medium|high|critical",
    "confidence": 0.0,
    "claim": "one falsifiable sentence",
    "rationale": "private candidate-generation rationale"
  }]
}
```

A3 may additionally return a top-level `"refuted": ["rf-..."]` array. Static enrichment agents extend the envelope in two ways: each candidate may carry `"confirms": <1-based raw finding index>` to prove a specific raw hit, and a top-level `"refuted": [{"index": 2, "reason": "..."}]` array disproves raw hits with the evidence the agent found. Agent provenance is stored in `sources` as `agent@slot`, for example `l1_diff@chat2`, `a3_execution@thinking`, or `s1_security@light` for enriched static findings; raw static findings that skipped or survived enrichment untouched keep the plain static agent ID.

Before cross-agent merge, each L1-L3 and A1 instance independently validates scope and ranges, attaches bounded deterministic evidence, and invokes blind verification according to verifier selection rules. A2-A4 validate and attach evidence without blind verification. A verifier verdict is `verified`, `downgraded`, `rejected`, or `needs_human_validation`; rejected candidates are counted in `checks_performed` and then removed.

Cross-agent merge clusters findings only when normalized file and category match and inclusive ranges overlap or are no more than five lines apart. The highest-priority member supplies the surviving claim. Merge unions distinct provenance and evidence, keeps the strongest verification status and confidence, records `deduped_from:<id>`, sets severity to the lower median of member severities, and caps evidence at eight entries. If S6 marks the file hot (churn percentile at least 0.85, a temporal hotspot, or fan-in at least 8), severity is bumped one level and a hot-file fact is attached; Critical remains Critical.

Rank tiers, strongest first, are:

1. `execution_reproduced`: contains `execution_output` or `mutation_probe` evidence.
2. `corroborated`: has at least two distinct sources.
3. `verified`: independently verified with one source.
4. `needs_human_validation`.
5. `unverified`.
6. `downgraded`.

Within a tier, findings sort by severity, confidence, stable ID, and location. When A3 lists a stable finding ID in `refuted`, that finding is downgraded and receives an `a3_refuted` marker before final merge; the report also records `a3_refuted:N`.

## Report contract

The Markdown report contains:

- **Review summary:** selected depth, scope size, focus, diff base, and verdict.
- **Assumed intent:** the first non-empty L3 summary, when L3 ran.
- **Findings:** grouped by rank tier, with severity, location, claim, sources, verification status, confidence, evidence, optional impact/remediation, and local checks.
- **Agent coverage:** columns `agent`, `model`, `status`, `reason`, `candidates`, `survived`, `steps`, and `ms`.
- **Checks performed:** global evidence, verifier, deduplication, refutation, and command markers.
- **Machine contract:** a final fenced JSON serialization of `ReviewReport`.

The JSON shape is:

```json
{
  "scope": {"files_reviewed": [], "focus": null, "diff_base": null},
  "findings": [{
    "id": "rf-1234abcd",
    "category": "correctness",
    "severity": "high",
    "confidence": 0.8,
    "verification_status": "verified",
    "rank_tier": "verified",
    "sources": ["l1_diff@thinking"],
    "file": "src/lib.rs",
    "line1": 10,
    "line2": 12,
    "claim": "A falsifiable claim.",
    "evidence": [{"kind": "excerpt", "path": "src/lib.rs", "line1": 10, "line2": 12, "content": "..."}],
    "impact": null,
    "remediation": null,
    "checks_performed": []
  }],
  "checks_performed": [],
  "summary": "...",
  "assumed_intent": "...",
  "pipeline": {
    "stages": [{"name": "mechanical", "status": "skipped", "reason": "review_commands_disabled"}],
    "stopped_reason": null,
    "mechanical": null,
    "depth": "normal",
    "agents": [{"agent": "l1_diff@thinking", "model": "provider/model", "status": "ran", "candidates": 1, "survived": 1, "duration_ms": 1000, "steps": 1}]
  }
}
```

A clean review still has a non-empty summary, scope, pipeline metadata, coverage, and checks. A failed mechanical preflight instead returns its `MechanicalResult`, records `mechanical_checks_failed`, and marks all swarm and merge stages skipped.

## Configuration

The swarm uses the single `review_agents.yaml` subagent configuration. The built-in file is `crates/refact-yaml-configs/src/defaults/subagents/review_agents.yaml`; a project can override it at `.refact/subagents/review_agents.yaml`.

Top-level `subchat` supplies base model parameters (`stateful`, `model_type`, context and output budgets, RAG budget, reasoning effort, and cache control). `prompts.reviewer` is the default L1 candidate prompt and `prompts.guardrails` is appended to the tool result.

| `review_swarm` area | Knobs |
|---|---|
| Scheduling | `default_depth`, `max_parallel`, `idle_timeout_secs`, `exec_idle_timeout_secs` |
| Gather | `model_slot`, `system_prompt`, `retry_prompt`, `tools`, `max_steps`, `max_files`, `n_ctx`, `max_new_tokens`, `temperature` |
| Verifier | `model_slot`, `prompt`, `n_ctx`, `max_new_tokens`, `temperature` |
| Static enrichment | shared `static_enrichment_prompt`; per-check `agent` block with `enabled`, `model_slot`, `max_steps`, `tools`, optional `prompt`, `n_ctx`, `max_new_tokens` |
| S1-S6 | `enabled`; S2 also has `min_confidence`; S6 also has `max_commits`; S1-S5 also carry the `agent` enrichment block |
| L1-L3 | `enabled`, `ensemble`, optional `prompt`, and optional `n_ctx`, `max_new_tokens`, `tokens_for_rag`, `temperature` overrides |
| A1-A2 | `enabled`, `model_slot`, `max_steps`, `tools`, optional `prompt`, `n_ctx`, and `max_new_tokens` |
| A3 | Agentic knobs plus `allow_execution` and `mutation_probe_cap` |
| A4 | Agentic knobs plus `app_url`, `dev_server_command`, and `allow_dev_server_boot` |

Empty agentic `tools` lists select family defaults. An absent L1 prompt falls back to `prompts.reviewer`; other missing agent prompts produce `prompt_not_configured` coverage rows.

| Slot | Default-model mapping and fallback |
|---|---|
| `chat` | `chat_default_model` |
| `chat2` | `chat_model_2`, then `chat_light_model`, then `chat_default_model` |
| `thinking` | `chat_thinking_model`, then `chat_default_model` |
| `light` | `chat_light_model`, then `chat_default_model` |

The first non-empty candidate that resolves through model capabilities is used. Ensembles therefore name logical slots, not hard-coded provider model IDs.

## Safety

Mechanical preflight accepts only literal argv entries from `ReviewCommandsConfig`; it does not interpret shell strings. Commands run through the centralized command policy with bounded captured output.

A3's agentic commands use the standard command policy. For destructive experiments and mutation probes, A3 creates a detached Git worktree under `.refact/review_scratch/<id>` at `HEAD`, applies the current binary working diff, and copies bounded untracked files. All mutation is directed to that scratch tree. If scratch creation fails, mutation probes are skipped and only non-destructive commands are allowed. The worktree is removed and pruned when A3 finishes. `allow_execution: false` skips A3; `mutation_probe_cap` bounds mutation attempts.

A4 may use a configured `app_url`. If permitted to boot an app, it starts `dev_server_command` or a detected project command through `process_start` in service mode, waits for readiness, and kills the service before finishing. With `allow_dev_server_boot: false`, it must use an already reachable URL or return no candidates. The `chrome` tool must be available.

## Benchmark

The fixture corpus contains 16 small self-contained fixtures: 11 seeded, three clean, and two duplicate-bait. Seeded coverage includes the original correctness, error-handling, security, consistency, and missing-test cases plus the agentic-slop family: `hardcoded_test_expectation`, `stub_claimed_complete`, `hallucinated_import`, `weakened_assertion`, `reward_hacked_test`, and `tautological_test`. Each manifest declares kind and seeded defects with file, inclusive line range, category, severity, description, and marker.

Ordinary tests validate manifest parsing, fixture composition, source-size and marker/range invariants, and pure metric helpers. A seed matches only when category and normalized/suffix-matched file agree and inclusive ranges overlap. Metrics include seeded recall, seeded High/Critical recall, verified precision proxy, unsupported rate, duplicate rate, clean-fixture counts, and corroborated precision. Corroborated precision is the share of findings in corroborated-or-stronger evidence tiers that match a seed. A4 browser findings are excluded from deterministic quality gates because browser availability and UI state are environment-dependent.

The live benchmark is ignored by default. It initializes an already-running engine workspace, invokes the review tool once per fixture, prints tab-separated metrics, and optionally writes each `ReviewReport`. `tests/review_bench.rs` uses:

- `REFACT_CODE_REVIEW_ENGINE_URL`: live engine base URL.
- `REFACT_CODE_REVIEW_MODEL`: outer tool-request model.
- `REFACT_CODE_REVIEW_OUTPUT_DIR`: optional report output directory.
- `REFACT_CODE_REVIEW_MANUAL_DUPLICATES`: optional JSON map of fixture IDs to manually scored semantic near-duplicate counts.

Quality gates are evaluated against a baseline captured with the same engine build, model set, fixture revision, and manual scoring:

| Metric | Gate |
|---|---|
| Unsupported-finding rate | At least 50% lower than baseline |
| Duplicate rate | Less than 10% |
| Clean fixtures | Zero verified findings and zero High/Critical unverified findings |
| Seeded High/Critical recall | Not worse than baseline |

Unsupported rate reconstructs rejected candidates from `verifier_rejected:N` and candidates absorbed by deduplication. Duplicate rate combines violations of the production same-category/same-file/five-line-near invariant with the manual semantic score. Precision metrics are diagnostic rather than independent gates. Inspect raw reports so genuine unseeded defects and manifest errors do not masquerade as metric regressions.
