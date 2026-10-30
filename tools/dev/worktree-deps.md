# Explicit checkout preparation

`setup-cache.sh` defaults to **dry-run**. `--status` is read-only; `--apply`
explicitly updates the selected user Cargo config, with a unique backup and an
atomic replacement. No invocation installs anything, edits `.bashrc`, starts or
contacts sccache, or builds source. Python 3.11 `tomllib`, or an already-installed
`tomli`, is required. On a Python 3.10 host without either, the script fails before
writes; select an existing interpreter with `SETUP_CACHE_PYTHON=/path/to/python3.11`.
There is no automatic parser installation.

```sh
bash tools/dev/setup-cache.sh --status --cwd "$CHECKOUT/refact-agent/engine"
bash tools/dev/setup-cache.sh --dry-run --cwd "$CHECKOUT/refact-agent/engine"
bash tools/dev/setup-cache.sh --apply --cwd "$CHECKOUT/refact-agent/engine"
```

The script parses the active legacy `config` or `config.toml` in Cargo home and
then each ancestor `.cargo` directory, nearest last, plus supported Cargo wrapper
and incremental environment overrides. It assumes ordinary Cargo config discovery
from `--cwd`; CLI `--config`, unstable includes, and alternate tool invocations are
not inferred. Ambiguous/unsupported configuration fails closed. Unrelated TOML,
comments and custom wrappers are preserved. Candidate edits are accepted only
when a full TOML parse equals the original document plus the requested keys; a
layout the editor cannot extend safely is refused. Explicit conflicting settings
are never silently replaced. Run with exclusive access to the user config;
atomic replacement protects against partial writes, not a hostile concurrent
same-user editor.

`build.incremental = false` is persistent configuration, **not evidence that the
current product environment honors it**. The output separately reports current
resolution and an explicit per-command `CARGO_INCREMENTAL=0 RUSTC_WRAPPER=sccache`
environment recommendation. Refact's scrubbed child environment does not inherit
these variables by default: a later runtime integration must supply authorized
request env/passthrough and account for higher-priority Cargo config. This script
changes no engine policy. It does not change profiles/features or establish
cache hits or speedup. Mutable Cargo `target` outputs remain per checkout;
compiler and package-download caches are not universal reusable build outputs.

## Offline dependency consumer

`worktree-deps.py` is a Python 3.10+ POSIX stdlib consumer, not an installer or a
source-attestation command. It runs no Node/npm/subprocesses/network/postinstall.
Its normal source is a **managed install snapshot** whose trusted producer
already recorded the completed install, patches and environment. Arbitrary old
`node_modules` next to a current lockfile is not accepted as install evidence.
There is deliberately no `bless-existing-main` command. Product producer/controller
wiring belongs to SYS-CACHE-RUNTIME; it is not implemented by this source card.

The importing runtime can use the `Environment`, `SourceReceipt` and
`CheckoutReceipt` typed contracts in the helper and call `prepare(...)`, or use:

```sh
python3 tools/dev/worktree-deps.py \
  --checkout "$OWNED_CHECKOUT" --package "$OWNED_CHECKOUT/refact-agent/gui" \
  --source "$MANAGED_INSTALL/gui" \
  --source-receipt "$TRUSTED_SOURCE_RECEIPT" \
  --source-receipt-sha256 "$CONTROLLER_PINNED_SOURCE_SHA256" \
  --ownership-receipt "$TRUSTED_CHECKOUT_RECEIPT" \
  --ownership-receipt-sha256 "$CONTROLLER_PINNED_OWNERSHIP_SHA256" \
  --environment "$TRUSTED_TARGET_ENVIRONMENT"
```

Default is dry-run; add `--apply` only with authority to create absent outputs.
Absolute canonical roots, caller ownership, a linked Git checkout back-reference,
and a separately pinned checkout-controller receipt are required. Being in the
worktree registry or supplying a known path alone is not authorization. Main
checkouts and symlinked destination packages/`node_modules` are refused. Any
existing output without an exact ownership/content receipt is preserved and
refused, even if it looks disposable. There is no deletion/replacement option.
An interrupted copy is retained, not cleaned automatically; its owner must inspect
and explicitly move/remove it, or choose a fresh owned checkout.

### Managed producer contract (schema 1)

A trusted producer must create a fresh private managed install, not attest an
arbitrary mutable main directory. After its separately authorized install and
patch procedure completes, while inputs and dependencies are quiescent, it records:

- `Environment`: all seven nonempty string fields `node`, `node_abi`, `platform`
  (Node naming), `arch` (Node naming), `libc`, `npm`, `install_policy`. The controller
  obtains actual target Node version/ABI/platform/architecture/libc/npm identity
  independently; the consumer never executes Node to guess them. `install_policy`
  must identify all effective install flags, omit/include/scripts policy, global/
  user npm configuration hashes, relevant environment and platform qualifiers.
  The helper additionally checks host platform/architecture. No cross-host reuse.
- `inputs, key = inputs(managed_package, environment)`: hashes the exact
  `package.json`, v2/v3 `package-lock.json`, optional `.npmrc`, and regular patch
  files. Symlinked patch inputs are refused. `key = SHA256(encoded(inputs))`.
- `entries = inventory(managed_package / 'node_modules')` and
  `tree_sha256 = SHA256(encoded(entries))`: deterministic sorted filtered content
  inventory with file hashes, executable bits, directories and legitimate
  relative internal file/bin links. `encoded` is sorted compact JSON plus newline,
  UTF-8, as defined by the helper. The producer must finish validating the real
  install before publishing this record; recomputing a digest is not install proof.
- A `SourceReceipt` JSON object with exactly the typed contract's fields:
  `schema: 1`, `kind: "refact-managed-dependency-install"`, nonempty `producer`
  and unique `install_id`, canonical `source` package path, `inputs`, `key`,
  `tree_sha256`. The controller receives the **SHA256 of the complete receipt
  bytes independently from the trusted producer**, never by trusting a neighboring
  sidecar or a model-supplied hash. The consumer checks both current inputs and
  current filtered dependency content against that pinned record.
- A separate `CheckoutReceipt` from the authorized checkout controller:
  `schema: 1`, `kind: "refact-owned-checkout"`, nonempty `owner_id`, and `owner`
  exactly matching `destination(checkout, package)[1]` (`checkout`, `package`,
  `gitdir`, root `device`, root `inode`). Its complete-byte SHA256 is also pinned
  independently by the controller. A receipt is evidence of granted authority only
  when issued by that trusted controller; the consumer does not manufacture it.

After copying, `<package>/node_modules/.refact-deps-receipt.json` binds owner,
source receipt/hash, exact inputs/key and filtered output-tree hash. A repeat call
is read-only/idempotent only if this receipt and current content still match.
Local dependency mutation never propagates back: regular files are copied, not
hardlinked, and whole writable `node_modules` trees are never shared. The helper
preserves internal relative links ending directly in included regular files
(standard package `.bin` links), refuses absolute/escaping/directory/chained/
recursive links and special files, and skips root self-nested `node_modules`
without following it. Legitimate nested package dependencies are still copied.
Mutable Vite/TypeScript/build cache entries are filtered; package-distributed
`dist`, `build`, JavaScript and `.d.ts` files are dependencies, not disposable
checkout build output. Top-level project `dist`/caches are never copied.

Filesystem access uses no-follow directory/file descriptors. Managed source and
checkout roots must remain exclusively owned/quiescent during preparation; this
is not a sandbox against an adversarial process with the same OS user. Missing
inputs, mismatches or unsupported structures fail offline with actionable output.
No stale-dependency fallback or automatic install is attempted.

Receipt SHA256 establishes byte integrity, **not producer provenance**. Trust must
come from an operator-pinned managed import or an authenticated controller with
an enforced read-only producer/cache boundary. File permissions and ownership do
not stop a same-UID or FullAccess process from changing config, cache or receipts.
Future runtime wiring must refuse/degrade unsafe enforcement rather than silently
using a host cache daemon. This helper provides no such runtime sandbox and
explicitly refuses platforms without POSIX no-follow directory descriptors.

## Central verification handoff

Source-card runtime/fixture state: **NOT_RUN**. Run centrally in temporary fixtures
only, using an already-installed full TOML parser/interpreter (no downloads):

```sh
python3 -m unittest discover -s tools/dev -p 'test_worktree_deps.py'
bash -n tools/dev/setup-cache.sh
git diff --check
```

If the default Python has no TOML parser, use the same unittest command with an
already-installed Python 3.11+ executable (record the actual command). Fixtures
only create temporary synthetic package/linked-checkout metadata, never execute
fixture JavaScript or npm/install hooks, and never read/write real Cargo home,
`.bashrc`, main dependencies or sccache state. Fixtures are not runtime/product
qualification, producer authentication or performance evidence.
