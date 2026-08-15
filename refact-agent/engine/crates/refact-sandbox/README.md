# refact-sandbox

`refact-sandbox` selects and probes the Linux confinement mechanism used by `refact-exec`. Probes execute a confined child and require both a successful command and an observed write denial; executable or kernel version checks are not availability signals.

Bubblewrap is selected only when its filesystem probe is fully enforced. Landlock is the fallback and reports `Partial` after a successful denial because it restricts filesystem access but does not provide the requested network isolation. Unsupported platforms and Linux hosts where neither mechanism enforces return the `noop` provider with `Unusable`; callers that request confinement receive a `sandbox:` error rather than unconfined execution.

`WorkspaceWrite` grants the request working directory and the platform temporary directory writable beneath a read-only root. `ReadOnly` grants no writable paths. `FullAccess` grants the filesystem writable while retaining any network isolation available from the selected provider.
