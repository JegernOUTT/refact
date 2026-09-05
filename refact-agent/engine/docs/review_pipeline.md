# Review pipeline

`review` runs a multi-stage code review. Every stage is an agent: it gets a task, a preferred
tool list, a fallback instruction, and the review scope, and it investigates with tools. The
pipeline itself never reads compiler output and never judges a finding; it schedules stages,
fact-checks their output deterministically, merges duplicates, and renders the report.

## Flow

1. **Scope.** The requested `files`, the git diff `base..HEAD` (base defaults to the merge-base
   with the upstream or main branch), and, in `scope_mode: adjacent`, one hop of reverse
   dependencies from `pr_blast`. Generated files, lockfiles and snapshots are excluded. There is
   no gather subagent: scope comes from arguments and git, not from a model.
2. **Stage selection.** The catalog is filtered by `depth` (or by an explicit `stages` list) and
   by each stage's `applies_when`. Every stage that is not scheduled still gets a row saying why.
3. **Parallel stages.** Scheduled stages run through a bounded pool (`parallel_depth`). There is
   no wall-clock budget: a stage is killed only when it stops responding for longer than the idle
   timeout. A silent stage becomes `timed_out`, a stage that panics or breaks the output contract
   becomes `failed`, and stages skipped because the review was cancelled become `not_run`.
4. **Deterministic fact-checking.** For each finding the pipeline checks that the quoted evidence
   really exists in the file within ±3 lines of the stated range (relocating the finding when the
   quote is found elsewhere), and whether the range intersects a hunk of `base..HEAD`, falling
   back to `git blame` for moved code. These two booleans, `evidence_present` and
   `introduced_by_diff`, are facts about the repository, not opinions about the finding.
5. **Merge.** Findings on the same file with overlapping ranges or equivalent claims collapse into
   one, keeping every location and every reporting stage in `reported_by`.
6. **Adversarial pass.** In `deep` (or when requested) one more agent receives the merged findings
   and tries to disprove each. It returns `supported`/`unsupported` verdicts. An `unsupported`
   verdict only sets `disputed{stage,reason}`, which moves the finding into the hypotheses
   section. It cannot delete, edit, promote or re-rank anything.
7. **Report.** Supported findings (verified evidence or an executed reproduction) are grouped by
   severity; everything else is listed under hypotheses with the reason it is unverified. Stage
   rows and coverage are always rendered.

## Stage catalog

Stages are YAML files, one per stage, shipped in
`crates/refact-yaml-configs/src/defaults/review_stages/` and copied on first run to
`~/.config/refact/review_stages/`; a project can override any of them in
`.refact/review_stages/<id>.yaml`.

| stage | depth | what it uniquely catches |
|---|---|---|
| `mechanical` | normal | compiler, type, lint and test diagnostics for the affected packages; one finding per diagnostic with the command as reproduction |
| `diff` | normal | local correctness defects introduced by the hunks |
| `impact` | normal | cross-file breakage: consumers, serialized formats, persisted state, tests asserting the old shape |
| `spec` | normal | requirements from `plan`/`what_to_check` that the diff does not satisfy |
| `security` | normal | injection, authz, secrets, crypto, unsafe deserialization along a reachable path |
| `dependencies` | normal | imports and manifest changes that do not resolve |
| `simplicity` | normal | complexity, duplication and dead code the diff introduced |
| `concurrency` | opt-in | lock ordering, cancellation, shutdown and interleavings |
| `tests` | deep | untested changed behaviour, tests that cannot fail, failing existing tests |
| `execution` | deep | defects reproduced by an executed command; may create throwaway test files and must delete them |
| `browser` | deep | rendered behaviour, console errors, failed requests, contrast |
| `adversarial` | deep, post-merge | claims that do not survive a hostile second read |

Fields: `id`, `title`, `phase` (`parallel`/`post_merge`), `contract` (`findings`/`verdicts`),
`depth` (`normal`/`deep`/`opt_in`), `writes_allowed`, `applies_when`
(`always`/`extensions`/`path_globs`), `preferred_tools`, `fallback`, `task`.

Every stage gets `shell` plus the read/search/process tools on top of its `preferred_tools`, and
runs autonomously: confirmations are disabled, so the allowlist is what bounds it.

## Output contract

A stage's final message must contain one JSON object:

```json
{"stage":"diff",
 "findings":[{"title":"…","severity":"blocker|high|medium|low|note","file":"…",
   "line_start":10,"line_end":14,"claim":"…","evidence":"…verbatim…",
   "reproduction":"… or null","fix":"…"}],
 "summary":"…",
 "coverage":{"files_read":["…"],"commands_run":[{"cmd":"…","exit":0}],
             "tools_unavailable":["…"],"stopped_early":null}}
```

The adversarial stage returns `verdicts:[{id,verdict,reason}]` instead of `findings`. Malformed
output gets exactly one repair turn ("re-emit JSON only, here is the parse error"); if that fails
the stage is `failed{output_contract}` and its raw text is written to the scratch directory.

## Arguments

`what_to_check`, `files`, `base`, `plan`, `scope_mode`, `stages`, `depth`, `parallel_depth`,
`variants` (1-3 model variants per stage), `browser` + `browser_scenario`.

The browser stage never runs without a scenario: `browser: true` requires `browser_scenario`, and
supplying a scenario schedules the stage at any depth. Without one the stage is reported as
`not run — no browser_scenario given` rather than guessing what to click.

## Configuration

`review_agents.yaml` (`review:` section) sets the defaults: `parallel_depth`, `variants`,
`idle_timeout_secs`, `max_steps`, `max_files`, `model_slot`, `variant_slots`, and per-stage
overrides under `stages:` (`enabled`, `model_slot`, `max_steps`).

## Artifacts

The rendered markdown is what the model sees. The full `ReviewReport` is attached to the tool
result metering as `review_report`, file references as `review_refs`, and each stage's raw answer
and coverage is written to `.refact/review_scratch/<review-id>/<stage>.json`.

## Benchmark

`tools/dev/review_bench.sh prepare` checks out the pinned oracle commit in a worktree and prints
the review call; `tools/dev/review_bench.sh score <report.json>` scores a saved report against
`tools/dev/review_bench_oracle.json` (24 known defects, 9 known false positives) and prints
recall, reproduced share, false positives, per-stage status and wall clock.
