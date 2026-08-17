# Code review pipeline

The `code_review` tool runs a structured evidence pipeline instead of treating one reviewer response as the final verdict. This document describes the runtime stages and the repeatable benchmark used for rollout decisions.

## Pipeline

1. **Scope** resolves seed files, expands relevant files through the gatherer, records changed files and diff context, and applies the configured file, candidate, and token budgets.
2. **Mechanical checks** run the literal argv entries from `review_commands.allowlist` before candidate generation. This stage is default-off. When enabled, every command uses the centralized `review_evidence` foreground execution policy and records its name, argv, exit status, and bounded output excerpt. Any non-zero exit or execution failure stops the pipeline before an LLM reviewer is called.
3. **Candidates** asks a recall-oriented reviewer for a fenced JSON envelope only when mechanical checks are disabled or pass. Each candidate has a file, line range, category, severity, confidence, falsifiable claim, and private rationale. The rationale is discarded before the report leaves this stage.
4. **Deterministic evidence** validates that each candidate is inside the review scope, rejects invalid ranges, and attaches bounded excerpts, diff hunks, and CodeGraph facts without spawning commands.
5. **Command evidence** reuses the mechanical results instead of rerunning commands and attaches bounded output only to High or Critical candidates.
6. **Blind verification** sends each claim and evidence bundle to an independent verifier. It never receives reviewer rationale, reviewer prose, or candidate confidence. The verifier returns `verified`, `downgraded`, `rejected`, or `needs_human_validation`; rejected candidates are counted in `checks_performed` before final removal.
7. **Deduplication and ranking** removes rejected findings, assigns stable IDs, coalesces same-file same-category findings with overlapping or five-line-near ranges, and ranks by verification status, severity, and confidence. A clean result still contains scope, checks, and a non-empty summary.

The machine-readable result is `ReviewReport`: a scope summary, retained findings, global `checks_performed`, summary, and pipeline metadata. The metadata lists each stage's `completed`, `failed`, or `skipped` status, its reason when applicable, the pipeline stop reason, and the optional `MechanicalResult`. A failed mechanical result is the report output and keeps all check excerpts while marking every later stage skipped. Each `ReviewFinding` carries its stable ID, category, severity, confidence, verification status, file and inclusive line range, claim, evidence, optional impact and remediation, and finding-local checks. The Markdown tool result ends with the complete report in a fenced JSON block.

## Benchmark corpus

`tests/code_review_fixtures/` contains ten fresh, self-contained frog-themed Python fixtures. Every fixture directory has a `manifest.json` with `id`, `kind`, `scenario`, `description`, and `seeded_defects`. A seeded defect declares `file`, inclusive `line_range`, `category`, `severity`, and `description`.

The corpus covers five seeded cases (off-by-one loop, swallowed fallible operation, path traversal, sibling API inconsistency, and missing test update), three clean cases (rename refactor, documentation-only change, and equivalent-logic reshuffle), and two duplicate-bait cases (one defect visible from two files and near-identical copied defects). `cargo test --test code_review_bench` parses every manifest, validates all declared ranges, enforces the 5/3/2 composition, and checks that each fixture remains under 200 source lines.

## Metrics

The integration test computes the following pure metrics from a `ReviewReport` and manifest:

- **Seeded recall** is matched seeded defects divided by all seeded defects. A match requires the same category, the same file or a report path ending in that fixture-relative file, and inclusive range overlap. Adjacent ranges and category mismatches do not match.
- **Seeded High/Critical recall** applies the same matching rule to manifest seeds whose severity is High or Critical and is the rollout recall gate.
- **Precision proxy** is verified findings matching a seed divided by all verified findings. It is a corpus-oriented proxy, not a claim of real-world precision, because unseeded fixture findings are treated as unmatched.
- **Unsupported rate** is the sum of `verifier_rejected:N` counters divided by retained findings plus rejected candidates. The verifier records these counters before finalization removes rejected candidates. The denominator also reconstructs candidates absorbed by deduplication from retained findings' `deduped_from:<id>` markers.
- **Duplicate rate** is post-dedup T-13 invariant violations plus manually scored semantic near-duplicates, divided by retained findings. Structural violations use the production rule: same normalized file, same category, and overlapping or at most five-line-near ranges. The manual hook is `REFACT_CODE_REVIEW_MANUAL_DUPLICATES='{"fixture_id":1}'`; each value is the number of extra retained findings judged to duplicate another claim.
- **Clean false positives** are verified findings on `clean` fixtures. The output also counts High or Critical unverified findings on clean fixtures because rollout forbids those even though they are not false positives by the primary metric.

A fixture with no seeds has seeded recall `1.0`; a report with no verified findings has precision proxy `1.0`; rates with a zero denominator are `0.0`.

## Run the benchmark

Deterministic parsing and metric tests run in ordinary CI:

```bash
cd refact-agent/engine
cargo test --test code_review_bench
```

The live benchmark is ignored and must target an already-running engine whose workspace can read this checkout. The engine's Default Models must be valid, and `REFACT_CODE_REVIEW_MODEL` names the outer tool request model; the configured `code_review`, `code_review_gather_files`, and `code_review_verifier` subagents select their own models. Run:

```bash
cd refact-agent/engine
REFACT_CODE_REVIEW_ENGINE_URL=http://127.0.0.1:8001 \
REFACT_CODE_REVIEW_MODEL=provider/model \
REFACT_CODE_REVIEW_OUTPUT_DIR=/tmp/code-review-bench/current \
cargo test --test code_review_bench -- --ignored --nocapture
```

The ignored test initializes the engine workspace, invokes `/v1/tools-execute` once per fixture, prints a tab-separated metric row, and optionally saves each final `ReviewReport` under `REFACT_CODE_REVIEW_OUTPUT_DIR`. Use the same engine build, provider account, model IDs, fixture commit, and manual duplicate scores for compared runs. Do not run this LLM-dependent benchmark in CI.

## Quality gates

A rollout candidate passes only when all of these hold against the captured pre-pipeline baseline:

| Metric | Gate |
| --- | --- |
| Unsupported-finding rate | At least 50% lower than baseline |
| Duplicate rate | Less than 10% |
| Clean fixtures | Zero verified findings and zero High or Critical unverified findings |
| Seeded High/Critical recall | Not worse than baseline |

Precision proxy is recorded for diagnosis but has no independent rollout threshold. Review raw reports before accepting a run so manifest mistakes or genuine unseeded defects do not masquerade as regressions.

## Capture and compare a baseline

1. Copy the current `code_review` project-local subagent configuration to `.refact/subagents/code_review.yaml` and replace only its reviewer prompt with the legacy single-pass review prompt. Keep scope budgets, models, corpus commit, command-evidence setting, and all other configuration fixed. Save the replaced file outside `.refact/subagents/` for restoration.
2. Start the same engine build and run the ignored benchmark with an empty output directory such as `/tmp/code-review-bench/legacy`. Record the printed table, manual duplicate-score JSON, provider/model IDs, fixture commit, and final reports.
3. Restore the structured `code_review` configuration, restart or reload the engine registry, and rerun into `/tmp/code-review-bench/current` with identical environment and manual scoring rules.
4. Aggregate candidates and rejected counts before dividing for unsupported rate; aggregate duplicate violations and retained findings before dividing for duplicate rate. Compare clean counts directly. Compute seeded High/Critical recall from only manifest seeds whose severity is High or Critical.
5. Add the two aggregate rows to this table in the rollout change under review; do not overwrite historical values from a different model or fixture revision.

| Run | Fixture commit | Model set | Unsupported rate | Duplicate rate | Clean verified | Clean High/Critical unverified | Seeded High/Critical recall |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| Legacy prompt | _capture before rollout_ | _record exact IDs_ | _pending_ | _pending_ | _pending_ | _pending_ | _pending_ |
| Structured pipeline | _same commit_ | _same IDs_ | _pending_ | _pending_ | _pending_ | _pending_ | _pending_ |

## Rollout stages

1. **Structured pipeline:** deploy scope, candidate schema, deterministic evidence, blind verification, final deduplication, and report rendering while command evidence remains disabled. Capture the baseline and candidate benchmark rows.
2. **Verifier default-on:** enable blind verification by default only after the quality gates pass and sampled reports confirm that the verifier remains blind to reviewer rationale.
3. **Mechanical and command evidence Linux dogfood:** opt selected Linux environments into `review_commands` only after centralized execution policy and sandbox rollout are ready. Enabling the setting activates the pre-candidate mechanical gate and reuses its results as candidate evidence. Keep literal argv allowlisting and bounded output mandatory; the setting remains default-off elsewhere until its security and quality signals are acceptable.

GUI dashboards, CI-run LLM tests, and statistical-significance machinery are outside this benchmark.
