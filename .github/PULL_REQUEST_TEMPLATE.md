## Summary

<!-- What changed and why? Link the related issue, for example: Closes #123 -->
Closes #

## Changes

- <!-- List the main changes in this PR. -->

## How tested

- [ ] Engine: `cargo check` / `cargo test`
- [ ] Rust tests: budget/admission check against the PR base and checker self-tests
- [ ] Chat GUI: `npm run types` / `npm run lint` / `npm test`
- [ ] Plugins: compile VS Code / JetBrains plugin if touched

## Rust test admission (if tests or test support change)

- Distinct important failure and nearest retained test's missing detection:
- Shipping implementation, exact oracle and non-vacuous prerequisites:
- Failure witness; critical contracts require executed before-fix/mutant failure and restored pass:
- Cheapest sufficient layer, meaningful cases added/removed and deliberate coverage loss:
- Fingerprint-bound `tools/dev/test_admissions.json` records and independent semantic review:

See [Rust test admission policy](../docs/rust-test-policy.md). Count and schema checks do not establish the truth of evidence or independent approval.

## Checklist

- [ ] Local checks pass via `tools/dev/check.sh`
- [ ] Reviewed
- [ ] CI green
