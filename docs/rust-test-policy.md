# Rust test admission and suite budget

Rust tests are a maintained regression budget, not an output quota. A test must detect a distinct important defect that the retained suite would otherwise miss. Coverage percentage, a production call, a plausible name, and successful execution do not independently justify admission.

The workspace ceiling is **6,500 Rust test cases**, enforced against both physical definitions and declared meaningful cases, reduced from the audited 12,800 baseline. The historical desired retained range was 30–50% of that baseline; the approved ceiling is slightly above that range. The ceiling is not a target to fill and cannot be raised merely to admit more tests. Any proposed budget change requires explicit maintainer approval and a coverage/cost trade-off review.

The recovery/main integration explicitly approved preserving all coverage and raising the prior
6,400 ceiling to 6,500 rather than deleting tests. Independent review counted 16, 17 and 7
meaningful scenarios in three previously undercounted queue/task-goal tests. The approved limit
retains these scenarios and the recovery regressions; it does not represent a coverage reduction
or permission to undercount packed workloads. No tests are deleted by this budget change.
The approval changes only the ceiling: independent source review, truthful failure witnesses,
full meaningful-case accounting and integration verification remain mandatory.

## Counting without loopholes

The budget includes every tracked Rust test definition and Rust documentation example, including ignored, feature/platform/cfg-gated, compile-fail, no-run, integration, and physically disconnected modules. Moving a test, changing its name, adding an ignore, disconnecting a module or introducing a feature gate does not remove its cost from the budget.

Physical definitions are a structural lower bound, not a complete workload measure. A table row, parameterized expansion, property workload and repeated expensive scenario still has cost inside a single function. Do not hide unchanged scenarios in giant tables, aggregate tests or generated cases to meet the ceiling. New tables and generators require explicit meaningful-case accounting and independent review; unsupported discovery must fail visibly rather than report an incomplete passing count.

Keep subprocess test entry points that retained tests invoke by name. A normally env-skipped child can be essential executable machinery; removing its annotation without changing dispatch breaks the retained parent.

## Mandatory admission gates

All new tests, materially changed tests and changes to test-support behavior must satisfy these gates. Extend a clear existing scenario when that is cheaper and equally discriminating. Do not add a new case if an existing one already detects the same failure.

| Gate | Required evidence |
|---|---|
| Distinct important failure | Name the concrete faulty behavior and why it matters. Compare the nearest retained test and identify the missing failure detection. Different literals, names, variants, languages or layers are not automatically different contracts. |
| Shipping implementation | Drive actual production logic or an externally consumed artifact. Mocks provide IO, time and observations; they must not implement the algorithm being tested. A test-only wrapper is acceptable only when the shipping algorithm still performs the relevant decision. |
| Discriminating oracle | Identify the asserted output, identity, state transition, refusal or side effect that the faulty behavior violates. Prefer exact semantic values where incorrect results would satisfy a weaker assertion. Cardinality and bounds are sufficient when they are the intended contract. |
| Non-vacuous prerequisites | Establish that the adversary and required state exist. Require nonempty loaded identities before universal assertions, a changed value before cleanup, the secret before redaction, an existing forbidden target for confidentiality, and a genuinely pending waiter before notification/cancellation. |
| Failure witness | Demonstrate a plausible before-fix or deliberately broken production behavior failing at the intended assertion, then the corrected behavior passing. Ordinary tests may supply a precise reviewed counterfactual where execution is disproportionate; critical tests require executed evidence. |
| Deterministic execution | Use isolated fixtures, controlled clocks and readiness handshakes. Sleeps are not readiness barriers. Avoid ambient repositories, credentials, external services and timing-based correctness claims. Bound waits and clean up owned tasks/processes. |
| Cheapest sufficient layer | Choose the lowest-cost layer that proves the defect. Integration/E2E tests must catch a wiring or boundary failure absent from lower layers. Do not repeat unit + integration + E2E by default. |
| Budget and review | Count all meaningful added scenarios and their cost, identify removed/replaced coverage, and obtain independent semantic approval. Passing a structural checker is not test-quality approval. |

Critical contracts include authorization and privacy/secret boundaries, destructive filesystem/worktree operations, durable data integrity and migration, ownership/epoch/fencing/idempotency, concurrency ordering, cancellation/shutdown and provider signature preservation. Treat ambiguity as critical until an independent reviewer accepts a narrower classification.

For critical admission, execute a focused failure witness against the actual guard/consumer, not a copied fixture implementation. Record the failing command, before-fix revision or mutant patch fingerprint, the intended failing assertion/diagnostic, nonzero exit status, the restored/corrected passing run and evidence references. An unrelated compilation error, unavailable dependency or absent fixture is not a valid witness. Revert temporary mutations before verification. Evidence must not contain secrets.

## Reject by default

- Assertions of caller-assigned literals, enum identity, debug names, constant aliases, direct getter/assignment smoke or a test-local reimplementation.
- Source-code substring/order checks represented as runtime behavior. Dead code can retain the searched fragments while behavior is broken.
- Symmetric serde roundtrips with no distinct domain data-loss risk, fixed wire requirement or compatibility purpose.
- Generic success/nonempty/substring assertions when wrong behavior can satisfy the central claim. A useful narrow oracle may be retained, but must not be described as proving a stronger invariant.
- Large fixtures that add no failure detection beyond smaller deterministic inputs, or a stress name without an asserted structural contract.
- Duplicated failure detection at multiple layers without a distinct boundary risk.
- Tests written because a function/branch/file lacks a test, to meet an arbitrary coverage percentage or to increase test count.
- Uncalibrated elapsed-time limits in the default correctness suite. Workload performance belongs in the deliberate performance lane; a timeout safety test needs a controlled genuinely pending operation, not a closed port or an immediately failing peer.

## Narrow legitimate exceptions

Small pure parser and policy tests can be the cheapest strong oracle. Independently fixed wire keys, populated semantic persistence, meaningful safety defaults, cross-component registry coherence, delivered model-facing instructions, license attribution and generated-artifact integrity can be real product contracts.

State the exception precisely. A policy default is not proof every caller consumes it. Model guidance is not runtime authorization. An artifact size ceiling is not executable correctness. A roundtrip may catch populated data loss while missing coordinated wire renames. Preserve honest narrow coverage without promoting it into an end-to-end guarantee.

## Review record

For every admitted or materially changed test, provide a current fingerprint-bound record covering:

1. Concrete failure mode and importance.
2. Nearest retained test and why it cannot detect that failure.
3. Exact oracle and non-vacuous fixture prerequisites.
4. Failure witness, including executed failing/passing evidence for critical contracts.
5. Chosen layer, meaningful case count and why a cheaper existing scenario is insufficient.
6. Independent review evidence and any deliberately removed coverage.

Changes to shared fixtures, test helpers, snapshots, scripts or generated inputs can alter tests without changing their bodies. Include a reviewed impact mapping for affected tests; unchanged body fingerprints are not a bypass. Exact body-preserving moves do not require inventing a new behavioral justification, but still must preserve execution context and dependencies.

The structural gate checks the count, discoverability, fingerprints and evidence-record consistency. It cannot authenticate a reviewer name, prove prose true, establish mutation sensitivity or prevent all workload packing. The independent reviewer must inspect the implementation and oracle, verify the witness and case accounting, and reject unjustified growth. Actual independent approval is enforced by repository review protections, not by a JSON name.

The pull-request admission workflow executes the comparison base's checker against the proposed
checkout as data. It never executes the proposed checker or tests in that trusted job. Checker
changes therefore cannot relax their own admission check. Normal engine CI also runs the current
inventory check and checker self-tests. The trusted job becomes available only after the workflow
and checker exist on the base branch; its initial rollout needs manual review. Maintainers must
configure the trusted admission status and independent reviews as required branch protections;
adding workflow files does not configure repository settings or prove that protection is enabled.

## Reduction changes

Reduction may intentionally remove useful lower-priority cases under the budget. Say so. Do not rename such loss as strict duplication or claim no coverage loss. A duplicate removal requires actual retained implementation/path/assertion subsumption. A consolidation must move every promised distinct case before deleting declarations. A weak test with an important intended contract requires a real replacement before claiming that contract is protected.

The historical full-body audit and budgeted selection are revision-specific. Revalidate bodies and retained dependencies when integrating newer changes; never apply line-based deletion plans to a different source revision blindly.

### Recorded reduction and accepted losses

The audited baseline `d1f0a26c7fa6182d114e36c49098560fb799fbbb` contained 12,800
physical cases. Selection retained 6,057 after 60 critical cases were recovered by independent
review. Reconciliation with `38a7d749485358f009bddc7cf843241c9e590cf6` preserved all
100 newly named tests, 112 changed retained tests and 85 changed proposed cuts; it also honored
56 obsolete retained cases removed on that branch. The resulting suite contains **6,186 physical
cases**, 48.33% of the original baseline. Against that newer branch, 6,601 of 12,787 cases were
removed. No retained scenarios were transferred into aggregate functions to obtain these counts.

These are physical-source counts, not a compiler-selected, executed or complete workload count.
Ignored, platform, feature and orphan cases remain separately relevant. Existing tables retained
from the newer branch still contain multiple inputs; the reduction did not introduce those tables.

Useful coverage was deliberately sacrificed: cosmetic/prose/default variants, secondary legacy
and parser permutations, UI golden breadth, advisory telemetry/ranking, and some lower-priority
language/provider variants. These losses are not all duplicates or fake tests. Security, privacy,
authority, durable-write/migration, cancellation and subprocess-worker protection received separate
critical review. That review is not a proof that every possible regression remains detected.

Known pre-existing oracle limits remain explicit: the scoped-shell warning test did not observe
the actual child working directory, and browser password serialization coverage lacks complete
per-channel positive controls. A retained test name does not cure either limitation. No speedup
is inferred from case or deleted-line counts.

`tools/dev/rust_test_reduction.json.gz` contains deterministic, metadata-only per-ID decisions,
coverage losses, rescue provenance and the integration reconciliation. Inspect it with
`python3 -m gzip -d -c tools/dev/rust_test_reduction.json.gz`; it is evidence for this reduction,
not an allowlist for future tests or a substitute for executing the verification lanes below.

## Verification

Run the local test-budget check and its focused self-tests before expensive builds. For Rust changes, perform affected crate/module and integration checks first, then workspace compilation, library, integration and documentation tests in the CI profile. Feature/platform/ignored live-browser paths need their corresponding deliberate lanes; a default skipped test is not validation of its enabled scenario.

Library and integration test commands set `RUST_MIN_STACK=16777216`. The unchanged launch/resume
regression aborted at the default 2 MiB stack and passed at 4, 8, 16 and 32 MiB in focused direct
binary runs. This is an explicit test-harness resource budget, not a relaxed assertion, ignored
test or performance result. It is scoped to test commands rather than Cargo's global environment;
normal engine execution does not inherit it from repository configuration. A focused pass does
not replace the complete suite run, and no exact offending async frame is claimed without a trace.

After a reduction batch, independently review retained critical contracts and helper/subprocess dependencies, inspect `git diff --check`, and confirm no production behavior changed merely to make tests pass. Do not lower quality gates, add blanket warning suppression or replace real assertions with weaker smoke.

Measure compile/link time separately from warmed test execution using the same toolchain, profile, features, cache policy, machine and thread count across repeated runs. Counts and deleted lines do not prove speedup. Performance benchmarks remain separate from the normal correctness gate.
