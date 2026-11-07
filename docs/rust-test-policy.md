# Rust testing guidance

Tests should detect important faulty behavior through shipping logic, not reproduce the
algorithm in a fixture. Prefer the cheapest layer with a discriminating oracle: assert
the relevant output, identity, state transition, refusal or side effect. Different names
or literals alone do not establish distinct coverage. Avoid duplicating the same failure
detection across unit, integration and E2E layers without a boundary-specific reason.

## Reliable behavioral checks

- Establish non-vacuous prerequisites: load identities before universal assertions,
  introduce a changed value before cleanup, and ensure a waiter is genuinely pending
  before checking notification or cancellation.
- Use isolated fixtures, controlled clocks and readiness handshakes. Sleeps are not
  readiness barriers. Bound waits and clean up owned tasks and processes.
- Avoid ambient repositories, credentials and external services in ordinary tests.
  Live-service, feature-gated and platform-specific scenarios require deliberate runs.
- Mocks should supply IO, time or observations, not the decision being tested. Generic
  success, nonempty or substring assertions must not stand in for a stronger contract.
- Preserve subprocess test entry points invoked by retained tests, including normally
  environment-skipped children. Review helper, fixture and dispatch dependencies when
  changing or removing tests.

## Independent review and critical contracts

Obtain independent code review of implementation and assertions. Pay particular attention
to authorization, privacy and secrets, destructive filesystem/worktree operations, durable
data integrity and migration, ownership/fencing/idempotency, concurrency ordering,
cancellation/shutdown and provider signature preservation. Review whether realistic faulty
behavior would pass the test; use focused before-fix or temporary mutation runs when useful
to resolve uncertainty, and restore any temporary changes before final verification.
An unrelated compilation or fixture failure does not demonstrate a behavioral regression.

Explain deliberately removed coverage honestly. Do not call useful coverage duplicate
without a retained test that actually subsumes its implementation path and assertions.
Do not weaken assertions or runtime safety to make checks pass. Review test-support changes
for effects on their consumers even when test bodies are unchanged.

## Execute real verification

Run affected crate/module checks first, followed by workspace compilation, library,
integration and documentation tests in the CI profile as appropriate. Keep formatting
checks and independent code review. For example, from `refact-agent/engine`:

```bash
cargo fmt --check
cargo check --workspace --profile ci-test
RUST_MIN_STACK=16777216 cargo test --workspace --profile ci-test --lib
RUST_MIN_STACK=16777216 cargo test --workspace --profile ci-test --test '*'
cargo test --workspace --profile ci-test --doc
```

The stack setting is scoped to test commands; it does not alter normal engine execution.
Run additional build, daemon E2E, feature/platform and ignored live-browser lanes when
relevant. Report commands actually executed and their results, along with skipped or
unavailable scenarios; a default skipped test does not validate its enabled behavior.

Performance benchmarks remain separate from the normal correctness suite. Measure
compile/link time separately from warmed execution under comparable conditions. Test
counts and deleted lines do not prove speedup.
