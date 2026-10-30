# Managed worktree preparation (schema 1)

`WorktreeService::create_worktree` prepares only a newly published registered linked
checkout, after releasing registry and repository transaction locks. Manual, chat
and board isolated creation already use this shared service. Inherit mode does not
call creation. Preparation failure retains the usable Git checkout and reports a
fixed diagnostic; it never rolls back Git or installs anything. Inventory/get/list
only read the bounded report; they never prepare, copy, install or garbage-collect.

## Local outputs and diagnostic API

The provisioner creates an absent `refact-agent/engine/target` in a unique sibling
and publishes it with no replacement. Its ownership marker binds checkout ID and
root device/inode. Existing targets without that exact marker, including symlinks,
are preserved and degraded. Vite/TypeScript caches and project dist are never
shared. The runtime execution adapter must reject conflicting Cargo target/wrapper
configuration before using `CacheExecutionContext::cargo_target()`; this method
returns a checked checkout-local path, not execution permission or an environment
mutation. An external command that ignores this adapter can still choose its own
Cargo configuration. This service does not claim to constrain arbitrary commands.

`WorktreeRecordView.cache_report` is optional/serde-default. It appears under
create.worktree and on get/list. The schema has fixed component fields
`cargo_target`, `compiler_cache`, `dependencies`, each with `outcome`
(`applied`, `reused`, `miss`, `degraded`), fixed `reason`, and optional lowercase
64-hex `key_ref`. `trust` is `unverified` or `root_protected_host_store`.
No paths, environment values, dependency contents or arbitrary errors are returned.
The report is stored as `refact-cache-report.json` in the linked Git administrative
directory, capped at 4 KiB on read. Invalid/unknown fields, schema, enums or keys
produce degraded/corrupt_blob. It is untrusted display data, NEVER authority.
Each report-only cache read has an independent 4,097-byte content budget: at most
4 KiB accepted payload plus one overflow-detection byte if the file grows after
the size check. It does not prepare or mutate outputs. This does not bound the
surrounding service's registry/listing work or libgit2's internal reads.

`WorktreeService::cache_execution_context(meta)` reconstructs a non-serializable,
private-field `CacheExecutionContext` from the current service registry, exact
registered metadata, current branch/HEAD, repository common directory, linked Git
back-reference and root/admin directory identities. It also captures the protected
controller digest or the precise reason trusted configuration is unavailable.
`validate_current()` repeats these checks and any available controller pin before
use. Execution consumers must reconstruct after sandbox resolution and repeat the
check at use; holding this object does not widen a sandbox or grant permission.
The compiler-cache component always reports degraded/unsafe_daemon_boundary.
No sccache executable or host daemon is run/contacted, and no global config is written.

## Explicit operator import/publish route

This is an offline consumer of an independently trusted producer, not an installer,
attestation service, permission store or a command for blessing old main dependencies.
An administrator can opt in by importing an already completed trusted managed
install under `/var/lib/refact/worktree-cache/<64-lowercase-hex-entry>/` and publishing
`/etc/refact/worktree-cache/controller.json`. No helper is auto-run. Do not copy or
hash arbitrary main `node_modules` and call it trusted. The administrator must obtain
the complete producer receipt SHA256 independently of the imported tree/sidecar,
verify the producer and install provenance through the administrator's trusted
channel, and only then publish that pin in the controller. Digest integrity alone
is not provenance. Import into fresh administrator-owned files/directories under
the protected store; never just chown/chmod a same-UID writable tree, hardlink an
untrusted inode, or preserve producer write handles. Permissions do not revoke
already-open writable descriptors. The install must be private/quiescent while produced and remain
immutable while consumed. Administrator replacement must use a new entry, not mutate
an in-use one; publish a replacement controller only after completing the entry.

The fixed controller shape is:

```json
{
  "schema": 1,
  "kind": "refact-worktree-cache-controller",
  "environment": {
    "node": "operator-pinned-exact-node-version",
    "node_abi": "operator-pinned-node-abi",
    "platform": "linux",
    "arch": "x64",
    "libc": "glibc",
    "npm": "operator-pinned-exact-npm-version",
    "install_policy": "operator-pinned-complete-policy-fingerprint"
  },
  "packages": [{
    "package": "refact-agent/gui",
    "entry": "64-lowercase-hex-entry-name",
    "receipt_sha256": "independently-obtained-complete-producer-json-sha256"
  }]
}
```

These placeholders are documentation, not valid hex values. At most 16 pins are
accepted; exactly one may match the GUI. No request JSON, environment variable,
checkout file, report, or model-supplied path/hash can select a controller/store.
The operator is responsible for ensuring the environment describes the actual
future target Node/npm identity and effective install flags, include/omit policy,
script policy, config/environment hashes and platform qualifiers. The consumer
checks Linux/architecture/libc against the host but does not execute Node/npm.
The execution adapter must reject an incompatible command environment. Receipt
matching is an installation-input guarantee, not a promise that an arbitrary future
command resolves the same Node binary.

The imported entry contains exact package.json, npm v2/v3 package-lock.json,
optional .npmrc, regular patches/, filtered node_modules/, and producer.json.
producer.json uses the SourceReceipt schema documented in
`tools/dev/worktree-deps.md`: schema, kind, producer, install_id, canonical source,
inputs, key, tree_sha256. Its `source` must be the final managed entry directory.
Inputs and inventories use sorted-key compact ASCII JSON with a trailing newline,
matching the Python helper for accepted ASCII paths/data. The runtime recomputes
manifest/lock consistency, exact input hashes, input key and filtered tree digest;
it does not merely trust the receipt's hash claims. The helper's existing checkout
receipt is not consumed or manufactured: the shared service supplies the actual
current registered checkout context. A Python-produced dependency output receipt
is not accepted as this service's private output ownership marker.

## Enforced boundary and limits

Automatic reuse is Linux-only. Every fixed controller/store ancestor and every
opened source directory/file is traversed with no-follow directory descriptors,
must be host-root-owned, and must not be group/world writable. Effective-UID
writability checks also refuse ACL-granted writable files/directories. Source special files,
path traversal, absolute/directory/chained/escaping links, non-ASCII or overlong paths,
and root self-nested node_modules are refused. Legitimate internal links terminating
in included regular files are reconstructed only after regular files are copied.
Root self-nested links are rejected, not silently blessed. Known mutable dependency
caches are excluded; distributed package dist/build files remain dependencies.

The consumer refuses euid 0, changed real/effective UID, any effective/permitted Linux
capabilities, and non-initial UID mappings. This is a host DAC administrator trust
boundary, NOT a same-UID chmod claim. It assumes a genuine trusted host filesystem and
administrator-controlled mount namespace; an agent able to replace host mounts,
change root-owned files, bypass DAC, or control the service process can defeat it.
A root-mapped user namespace alone is not accepted as protection. Same-UID ordinary
FullAccess agents cannot write the root-owned store under this boundary, but this
implementation does NOT claim all unconfined/same-UID agents are unable to poison
caches in arbitrary deployments. It does not protect checkout-local mutable outputs
from an adversarial concurrent same-UID checkout writer. Keep the newly created
checkout exclusively held during preparation; descriptors/no-replace publication
prevent following/replacing existing output paths, not arbitrary hostile process
interference. Unsupported/unverified deployments degrade rather than use a writable
host store or daemon. The fixture injection below does not qualify these OS assumptions.

No mutable hardlinks or whole-tree symlinks are created. Copy regular files into a
fresh private sibling, verify both inventories and inputs, repeat current checkout
and controller/producer checks, write the owned marker, then use renameat2 with
RENAME_NOREPLACE. Existing destination always wins and is preserved. Interrupted
private siblings are retained for explicit owner inspection; no automatic cleanup.

Each `CacheExecutionContext::prepare` call owns one budget shared across all of its
explicit content reads: 30-second cooperative deadline, drop-driven cancellation,
100,000 charged entries/nonempty read chunks/link payloads, and 2 GiB total bytes.
This includes controller/producer JSON, registry and Git back-reference files,
ownership markers, host-boundary proc files, package inputs, patches, dependency
files and link payloads, including every repeated pin/hash/copy/verification pass
and the checks before writing the diagnostic report. Copying a dependency chunk
charges its source read once, not again for the same bytes written to the private
copy; later reads of that copy are charged again. Usable dependency size is lower
than 2 GiB. Budget checks precede content reads, dynamic payload growth and copying;
read requests cannot exceed remaining aggregate bytes. Exhaustion fails closed,
including when no budget remains to confirm EOF or an untruncated link payload.

Per-read payload caps remain 32 MiB per manifest/lock/config, 4 MiB per
controller/producer JSON, 8 MiB registry, 4 KiB per Git back-reference/owner marker
or uid_map, and 64 KiB proc status. The limited-file reader can consume one
overflow-detection byte beyond a per-read cap if the file grows or has synthetic
size metadata; that byte is charged within, never beyond, the aggregate ceiling
and is not accepted as payload. Relative paths remain capped at 1,024 bytes,
48 levels and 255-byte ASCII components; readlink uses at most 1,025 charged bytes
to reject a link payload exceeding 1,024 bytes.

Context construction (`from_registered`), standalone `validate_current()` and
`cargo_target()` each have a separate 17 MiB aggregate explicit-content budget per
call, with the same cooperative deadline, entry/chunk ceiling and per-read caps.
Construction shares that one budget across initial pin capture and revalidation;
`cargo_target()` shares one across current-context and local-owner validation.
These scopes precede or are independent of preparation, are not reset per read,
and do not prepare/copy dependencies. Preparation never calls the standalone
wrappers to reset its own budget: all its context rechecks use its original budget.
None of these content ceilings accounts for libgit2-internal reads or kernel
directory/stat/path-resolution I/O, nor for surrounding WorktreeService operations.
No hard realtime guarantee is made for a stalled filesystem syscall/fsync. Unsupported
filesystems lacking no-replace publication fail closed. No network/npm/scripts run.

## Central qualification (NOT_RUN in this source lane)

Cargo.lock must first add existing workspace `libc` to refact-worktrees dependencies
without updating versions. No lockfile or admission ledger was edited in this lane.

Run centrally, offline, against the composed candidate:

```sh
cargo test --offline --locked --profile ci-test -p refact-worktrees --lib cache_reuse_
cargo test --offline --locked --profile ci-test -p refact-worktrees --lib worktree_registry_create_
```

Seven physical cache tests carry twelve meaningful scenarios: private valid copy plus
owned repeat and local divergence (3), input mismatch (1), corrupt source with
unchanged producer pin (1), escaping link (1), existing output plus no-replace race
publication (2), aggregate repeated metadata budget plus changed producer pin and
revoked current registry binding (3), cancelled materialization (1).
This is twelve scenarios, not seven merely because
there are seven functions. Fixtures
inject temp controller/store directories into the actual production parser,
revalidator, inventory, materializer and publisher. Their controller pins are fixed
before adversarial mutations; no real host data or JavaScript executes. The test-only
unprotected-root allowance skips OS provenance enforcement, so **OS-trust positive
control is not established**. The public production create path remains fixed to
protected host locations. Root-protected ingress qualification requires a separately
controlled administrator fixture/namespace and independent semantic review.
Critical failure/restored-pass witnesses, exact nonzero discovery, test admissions,
budget checker and compiler/runtime qualification remain NOT_RUN, not implied by
formatting or source review. The existing create fixture is extended to assert
create/get/list report coherence without removing its branch/HEAD checks.

The existing `cache_reuse_changed_producer_pin_and_revoked_checkout_refuse` fixture
now loads nonempty fixed controller/producer metadata through the shipping loader,
then repeats the shipping revalidator with one small shared budget sufficient for
load plus one full repeat, but one byte short of a second repeat. It asserts exact
load/repeat charges and `LimitExceeded` on the next repeat without publication,
then retains its producer-pin and revoked-registry checks. This adds one aggregate
resource-accounting scenario, not a cache-authentication qualification. Removing
metadata charging from the actual reader makes the exact byte and refusal oracles
fail; that is a source counterfactual only, not an executed witness. Independent
semantic classification/review and restored/mutant runs remain central NOT_RUN.
The shared fixture now passes one budget from load into materialization; support
impact review must cover all seven cache tests centrally. No admission ledger was
updated here.
