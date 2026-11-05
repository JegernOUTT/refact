# GUI artifact production and embedding (schema 1)

This is a source-only implementation handoff. Compilation, fixture execution,
protected-controller positive qualification, critical mutant/restored-pass witnesses,
independent semantic review and admission are **NOT_RUN**. No cache performance,
artifact verification PASS, browser-loaded identity or security qualification is
claimed by this document or by static formatting.

## Modes and prerequisites

- Normal `cargo build` / `cargo build --release` remains an explicit local GUI build
  request whenever Cargo runs the build script. Unchanged ordinary dev builds can
  remain Cargo-Fresh without rerunning it; release builds refresh every invocation.
  There is no new consent flag. A refresh requires already installed local Node,
  npm, `node_modules/typescript/bin/tsc` and `node_modules/vite/bin/vite.js`. Missing
  prerequisites fail clearly; the build script never runs `npm ci`, `npx`, an
  installer, a remote tool resolver or an automatic build/download fallback.
  Dependencies must be separately prepared by an authorized operator. This makes
  offline and normal builds use the same explicit prerequisite rule.
- `REFACT_USE_PREBUILT_GUI=1` requests only a pinned prebuilt consumer. Missing,
  untrusted, mismatched, corrupt or incomplete input fails without running Node,
  npm, TypeScript or Vite. Local Git may be read to reproduce the version commit.
- `REFACT_SKIP_GUI_BUILD=1` takes precedence and is the existing API-only bypass.
  It explicitly reports that existing assets are **NOT validated**. A successful
  skipped Cargo check does not qualify GUI embedding.
- `REFACT_GUI_DIR` still selects the source GUI package, not a trusted controller
  or authority source. Relative paths are engine-relative. Source/artifact and
  destination ancestors must be real directories, without symlinks/reparse points.
- `NODE_OPTIONS` is inherited byte-for-byte. The default 8192 MiB heap flag is
  appended only if neither heap-flag spelling is present. Existing preload,
  transport/safety and caller heap flags are never removed or overwritten. The
  merged options reach identity probes and every local compiler process. Existing
  `REFACT_OFFLINE_NETNS`, Cargo/npm offline values and other inherited guards remain.
  These flags are not an OS network boundary; centrally authorized execution must
  still use the contained offline runner/transport canary policy.

## Reachable producer

`build.rs` calls the shared `gui_artifact` module to inventory inputs before running
installed Node with local tsc and both Vite builds. It sets `npm_package_version`
from the checked package manifest, preserving version injection in `vite.config.ts`.
After build success it rechecks inputs, inventories `dist/chat`, and atomically
writes `dist/gui-artifact.json`. It then stages and publishes the same manifest's
bytes into engine assets. The producer is not a disconnected type/helper.
Every actual build-script run retains full source/toolchain validation, local
compilation (unless prebuilt), artifact validation and publication. There is no
receipt or partial-validation fast path inside the script.

### Development freshness versus strict validation

Ordinary dev builds use Cargo's conventional file-mtime/environment freshness,
not per-invocation integrity validation. They watch all inventoried source files
and existing source directories, existing optional environment/config paths, known
VITE_* environment names, the resolved Node/npm executable paths, exact installed
TypeScript/Vite scripts and package manifests, and `node_modules/.package-lock.json`
when present. They do not recursively watch the GUI root, `node_modules`, or excluded
output directories. Generated outputs must stay in excluded output locations, not
inside watched source directories. The engine build-support directory remains
watched. Git HEAD, the current symbolic ref and existing packed-refs are resolved
with worktree-aware `git rev-parse --path-format=absolute --git-path`; only existing
files are watched, never the whole Git directory. An absent controller is not a dev
watch. Unchanged ordinary dev builds may therefore remain Cargo-Fresh.

This does not detect every possible change: newly created root `.env`/config files
or root source directories, entirely new arbitrary VITE_* environment names,
mtime-preserving edits, replacements behind an unchanged resolved tool path, and
changes to other installed dependency files may evade these watches. Git ref
creation can likewise be missed when no watched existing file changes. For an
explicit full refresh use `REFACT_GUI_ALWAYS_BUILD=1 cargo build`. Setting this flag
to `1` and later unsetting it triggers another rebuild because the flag is watched.

For `PROFILE=release`, `REFACT_USE_PREBUILT_GUI=1`, or
`REFACT_GUI_ALWAYS_BUILD=1`, an intentionally absent OUT_DIR sentinel forces the
script to run on every GUI-enabled Cargo invocation, preserving strict validation
of full current inputs, including newly added VITE_* names and mtime-preserving
changes. Prebuilt mode retains its controller pin and full artifact checks;
release/forced local mode retains full local compilation. Skip mode takes priority
over all three, emits no GUI/controller watches and performs no GUI validation.
No measured warm-build performance or security qualification is claimed here.

The producer candidate records:

- `schema: 1`, `kind: "refact-gui-artifact"`,
  `producer: "refact-engine-build-v1"`;
- `inputs.inventory`: sorted depth-first directory and regular-file records covering
  the entire GUI package, including source, hidden config, environment files, root
  configuration, public files, package.json, package-lock.json and patches;
- exclusions only for node_modules, dist, .git, .refact, coverage, storybook-static,
  test-results, playwright-report, .cache, .vite, .eslintcache, .tsbuildinfo and
  generated Vite timestamp config files. These must remain outputs, not imported
  build inputs. Source/config imports outside this inventory are not authorized by
  this producer contract. Custom external imports require a new producer version;
- producer hashes for build.rs, the shared validator, trusted-controller reader and
  engine assets/chat/index.html; exact package/lock root identity is checked;
- GUI package version and the Vite commit label, plus optional SHA256s (not raw
  values) for DEBUG, REFACT_LSP_PORT, incoming NODE_OPTIONS, NODE_ENV, all VITE_*
  variables and all four Vite CI commit variables. Absent and present-empty remain distinct;
- toolchain: exact Node version/ABI/executable digest, npm version/resolved launcher
  digest, TypeScript and Vite versions, producer host platform/arch/libc. Installed
  dependencies and actual toolchain provenance are independently verified by the
  trusted producer/operator, not inferred from package version strings;
- `inputs_sha256`: SHA256 of serde's compact field-order JSON encoding of `inputs`;
- `artifacts`: complete directory/file inventory with relative paths, sizes and
  SHA256s. Nonempty index.js, index.umd.cjs, style.css and index.d.ts are mandatory.
  No links, reparse points, hardlinked Unix files, special files or traversal are
  accepted. Consumer recomputes all input and artifact inventories, including extra
  files, not just the four required names.

Producer candidate creation is not provenance. An unconfined same-UID worker can
modify both dist and its self-manifest; those two objects alone are never trusted.
Normal local build is the explicit user build intent, not a claim that arbitrary
checkout code is safe to execute.

## Independent controller pin and operator import

Linux prebuilt mode reads only the fixed path
`/etc/refact/worktree-cache/gui-controller.json`, using no-follow directory and
regular-file descriptors from `/` through every ancestor. All opened handles must
be root-owned, not group/world writable, and effectively non-writable by the
consumer (including ACL-granted access); regular files must have one link. Consumer
must be non-root, have unchanged real/effective host UID, initial UID mapping and
zero effective/permitted capabilities. Unsupported hosts or boundaries refuse.
The build script never creates, updates or self-selects this controller.

Shape:

```json
{
  "schema": 1,
  "kind": "refact-gui-artifact-controller",
  "manifest_sha256": "independently-obtained-complete-gui-artifact-json-sha256",
  "toolchain": {
    "node": "exact-producer-node-version",
    "node_abi": "exact-producer-node-abi",
    "node_sha256": "exact-producer-node-executable-sha256",
    "npm": "exact-producer-npm-version",
    "npm_cli_sha256": "exact-producer-resolved-npm-launcher-sha256",
    "typescript": "exact-producer-typescript-version",
    "vite": "exact-producer-vite-version",
    "platform": "linux",
    "arch": "x64",
    "libc": "glibc"
  }
}
```

Placeholders are not valid digest values. Operator must independently obtain the
complete manifest SHA256 through a trusted producer channel and verify that the
producer actually built these exact inputs with the recorded toolchain and trusted
installed dependencies. Do not bless an arbitrary checkout/main dist by hashing
its self-manifest. Import a completed quiescent managed producer package/dist into
the authorized checkout, with identical source/config/producer environment, then
publish a fresh administrator-owned controller inode atomically. Never just chmod
or chown an untrusted controller inode: outstanding writable descriptors survive
permission changes. Controller is a provenance pin, not a grant store, sandbox
exception or permission to execute any command. Normal/prebuilt/skip mode intent
and the owning execution policy still govern every invocation.

Consumer requires pinned manifest bytes, exact controller toolchain, current full
input equality, host platform equality and artifact integrity before publication.
It does not execute a toolchain merely to consume static assets: the controller
pins the authorized producer toolchain, not a future PATH resolution. Controller
and pinned manifest/input/artifact checks repeat immediately before publication.
Changing the pin midway refuses; missing entries never fall through to a build.

The boundary assumes genuine trusted host filesystems and administrator-controlled
mount namespace. Privileged, mount-controlling or service-process-controlling
agents are not covered. Local sources/output must be exclusively held/quiescent;
this is not a sandbox against concurrent adversarial same-UID checkout writers.
No universal same-UID FullAccess safety or self-manifest attestation is claimed.

## Staging, ownership and retained prior output

All validation precedes prior asset mutation. New private files are staged under
`engine/assets/.refact-gui-*/chat`, outside RustEmbed's `assets/chat` root. Source,
stage and current inputs are rehashed; an ownership marker binds engine-path digest
and artifact inventory. Marker contains no raw environment or filesystem paths.
Prebuilt mode replaces only matching owned output; unowned existing output refuses.
An explicit normal build may migrate a legacy regular-only tree by retaining it,
never deleting it. Divergent or malformed owned output refuses in either mode.

Linux uses no-replace rename for an absent destination and atomic exchange for an
existing tree, retaining prior output at the staging path. macOS uses exclusive
rename; Windows uses MoveFileExW without replacement flags. These platforms
retain an existing output in a private backup before no-replace publication and
attempt rollback on publication failure only after rechecking known parent/backup
identities (Unix device/inode, Windows handle volume/file index). A changed or
unknown identity stops recovery without overwriting a foreign path. A crash between these non-Linux renames
can leave the prior tree in the reported retained directory, not deleted. Neither
interrupted stages nor prior trees are automatically removed. Owner cleanup is an
explicit separate action. Filesystems without the required rename support refuse.

Inventory passes are bounded to 30,000 charged entries/chunks, 2 GiB read bytes,
128 MiB/file, 16 MiB/JSON, 48 path levels, 1,024-byte ASCII relative paths and
255-byte components. These are per-pass cooperative limits, not a realtime syscall
or aggregate whole-build limit. The compiled producer processes are ordinary
explicit build execution subject to the caller's containment/time budget.

## Central qualification handoff — NOT_RUN

Cargo.toml uses existing workspace serde, serde_json, sha2, libc and tempfile as
explicit build dependencies. There is no runtime-engine cycle/new crate/version.
They already occur in refact-lsp's lock dependency list; verify the composed
Cargo.lock remains exact under `--locked`. CACHE-CORE separately has a pending
refact-worktrees libc dependency-list entry. Do not run Cargo update to widen this.

Seven physical fixture functions contain these distinct meaningful scenarios:
valid pinned copy/private inode and owned replacement/retained prior (2), changed
source and changed lock (2), corrupt and incomplete payload (2), escaping link and
FIFO (2 Unix-only), changed toolchain and forged self-manifest with unchanged pin
(2), late controller refusal and unowned existing output (2), guard append and
existing heap preservation plus empty options (3). Count all 15 scenarios honestly;
no workload is hidden by the seven function names. Fixtures import the actual
build-script module, not a copied validator. They use synthetic package/artifact
bytes and a tiny temporary Git repository for the actual commit-label path, never
a JavaScript compiler or live service. Controller-argument injection
checks matching/integrity but does **not** establish protected OS trust.
The nearest existing dependency-copy oracles are
`cache_reuse_input_mismatch_does_not_publish`,
`cache_reuse_corrupt_source_does_not_publish`,
`cache_reuse_escaping_source_link_does_not_publish` and
`cache_reuse_publication_never_replaces_destination`; they exercise node_modules,
not the build-script artifact consumer, so they cannot detect a return of blind
dist copying, prebuilt pin bypass, destructive asset publication or NODE_OPTIONS
replacement. The separate GUI target is the cheapest current Cargo layer that
imports shipping build-support code without running a GUI compiler or linking a
live engine harness. Independent review must validate scenario admission rather
than infer it from test names or counts. Suggested central semantic mutants:
bypass manifest pin; omit current source/lock equality; skip artifact hash/type
checks; move destructive destination replacement before validation; replace rather
than merge NODE_OPTIONS. Execute and attribute actual reached failures, then
restore/re-run; these are proposals, not executed witnesses.

Exact focused command from repository root, once central execution is authorized:

```sh
python3 tools/dev/offline_exec.py -- env REFACT_SKIP_GUI_BUILD=1 CARGO_NET_OFFLINE=true cargo test --manifest-path refact-agent/engine/Cargo.toml --offline --locked --profile ci-test -p refact-lsp --test gui_artifact -- --test-threads=1
```

API-only compilation command (not GUI embedding verification):

```sh
python3 tools/dev/offline_exec.py -- env REFACT_SKIP_GUI_BUILD=1 CARGO_NET_OFFLINE=true cargo check --manifest-path refact-agent/engine/Cargo.toml --offline --locked --profile ci-test --workspace
```

Actual embed qualification additionally requires an independently imported matching
producer/controller positive fixture plus mismatched source/lock/corrupt/link and
existing-output preservation witnesses. CORE-style protected ingress refuses the
UID-mapped `offline_exec.py` namespace; do not weaken it to make a canary pass. Use
an administrator-controlled network namespace/contained runner preserving the
initial host UID mapping and dropping all capabilities instead, then run from
engine with `REFACT_USE_PREBUILT_GUI=1`, `REFACT_SKIP_GUI_BUILD=0`, and
`CARGO_NET_OFFLINE=true`:

```sh
cargo check --offline --locked --profile ci-test --workspace
```

The appropriate contained runner and independently pinned producer import are
central prerequisites, not provided/executed here. Local-producer offline build
qualification must retain required NODE_OPTIONS transport guards and prove no
implicit installer/download; a skip-only check cannot substitute. Execute critical
mutant/before-fix failure and restored-pass witnesses, obtain independent semantic
review, record fingerprint-bound test/support admissions centrally, and run the
project test budget checker/self-tests against the trusted base before claiming
qualification. No shared ledger was edited by this source lane.
