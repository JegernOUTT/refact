# refact-ai

This npm package installs the standalone `refact` binary from the matching
[`engine/v<version>` GitHub Release](https://github.com/JegernOUTT/refact/releases).
The install script selects the archive for the current operating system and CPU,
downloads its `.sha256` sidecar, verifies the archive, and installs a `refact`
command. It has no runtime dependencies.

```sh
npm install --global refact-ai
refact version
```

Node.js 18 or newer is required. Supported targets are Windows x86, x64, and
ARM64; Linux x64 and ARM64; and macOS x64 and ARM64.

To use a release mirror, set `REFACT_RELEASE_BASE_URL` to the equivalent of
`https://github.com/JegernOUTT/refact/releases/download`. The mirror must retain
the `engine/v<version>/<archive>` layout and checksum sidecars.

If installation is interrupted or the machine is offline, restore connectivity
and run:

```sh
npm rebuild refact-ai
```

Maintainers stamp `__REFACT_VERSION__` before publishing. Dry-run resolution and
the offline test suite do not perform network requests:

```sh
node postinstall.js --dry-run
npm test
```
