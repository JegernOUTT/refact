# B02 whole-worker acceptance

## Run already-built artifacts

```sh
sh tools/dev/buddy-operator-test.sh \
  /absolute/path/to/target/debug/deps/buddy_operator-TEST_HASH \
  /absolute/path/to/target/debug/refact-lsp
```

Parent build owner builds the `buddy_operator` integration test and `refact-lsp`
binary first. The command itself performs no build/download. A missing executable,
non-Linux host, missing bubblewrap, or unavailable network namespace exits nonzero
with `B02 REFUSED`; scenario failures are test failures, never skips.
Default CI runs deterministic refusal contracts, not the opt-in scenario.

## Boundary and real paths

- One `bwrap --unshare-all` encloses controller, fake HTTP provider/MCP, shell
  fixture, compiled production engine, and production-AppState test worker.
- Private loopback is reachable within that namespace; there is no routed device,
  host loopback access, inherited proxy environment, host home, host runtime
  sockets, or host config. `/usr` and dynamic linker libraries are read-only.
- Entire mutable tree is a fresh temporary directory removed by the launcher.
- Controller and worker check kernel namespace identity, loopback-only devices,
  zero effective capabilities, scratch HOME, and absence of live filesystem roots.
- Production `refact-lsp` runs separately with scratch HOME/project, no scheduler,
  daemon-upgrade opt-out and fake token. Its real HTTP ping must match the nonce.
- Mutating HTTP acceptance uses a separate test binary worker with the production
  `create_global_context`, `AppState`, full router, native queue and resource owner.
  Storage/actions are injected into its BuddyService only. No production rollout
  flag or live service is enabled. The worker does not start the Buddy scheduler.
- Provider response supplies the actual draft text. MCP and shell fixture effects
  are checked on disk inside the same kernel boundary.
- Preview has no file effect; missing grant is rejected; authorized apply creates
  real Markdown; receipt reads, duplicate identity and unchanged bytes survive
  worker-process restart. Native question survives restart unanswered, resolves
  through HTTP/native command processing, and both independent clients reconcile.
  Stale answers receive HTTP 409 before and after a second process restart.

## Actual browser-to-scratch backend mode (separate acceptance claim)

```sh
sh tools/dev/buddy-operator-test.sh \
  /absolute/path/to/buddy_operator-TEST_HASH \
  /absolute/path/to/refact-lsp \
  /absolute/path/to/refact-agent/gui \
  /absolute/path/to/local/ms-playwright
```

The final input is an already-installed Playwright browser artifact directory.
Nothing is downloaded. Node must be `/usr/bin/node`. GUI source/test inputs are
copied into scratch; node_modules and browser artifacts are mounted read-only at
isolated paths. No repository root, user home, host runtime socket or host network
is exposed. Vite, Playwright, Chromium, controller and worker share the same denied
network namespace. Vite's real HTTP proxy targets only the scratch worker. Chromium
also requires its own sandbox (`chromiumSandbox: true`); failure is a failed
acceptance, never an automatic `--no-sandbox` fallback.

`buddy-operator-live.tsx` only composes production Redux/Theme/BuddyDesk/native
question components. The spec has no route mocks. Work/report/action bindings and
a native question are seeded in the test worker; preview, rights, grants, execute,
receipt queries and answers use the actual production router. It checks the actual
Markdown bytes before/after apply and restart, tracks the browser-generated request
identity, and asks the Rust controller to verify persisted receipt equality and
real duplicate disposition across two process restarts using scratch file IPC.
Two browser contexts see the question; UI resolves it and the second context's real
HTTP transport submits stale identity (409), then its UI refreshes native status.
Supervision, improvement and editorial flags remain false while resource and native
decision controls remain available. This is not full application chrome/IDE host
coverage, nor a provider configuration activation test. The two-argument mode
continues to report only its HTTP acceptance claim.

Browser results/traces and IPC evidence live under scratch and are removed on exit;
stdout reports the acceptance result. Parent verification must record actual run
output before changing the evidence table below to passed.

## Host evidence

| Host | Evidence | Scope |
| --- | --- | --- |
| Current Linux execution host | `bwrap --unshare-net --ro-bind / / -- /bin/true` exited 0 | Capability probe only; full runner not yet built/executed by this agent |
| Linux with namespaces disabled / absent bwrap | Explicit nonzero refusal | No fallback to client-only filtering |
| macOS / Windows | Explicit unsupported-host refusal | No claim of OS isolation parity |
| Browser / VS Code / JetBrains | Not executed | Browser must be launched inside the same namespace to use worker loopback `127.0.0.1:39191`; no host-network forwarding or live app attachment |

This is not a browser/UI pass, provider-configuration activation pass, or proof
that production feature rollout is enabled. Full scenario must be invoked on a
capable host after compiling the final sources; the capability probe alone is
not acceptance evidence.
