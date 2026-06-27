# Refact JetBrains Plugin

This JetBrains plugin is part of the [JegernOUTT/refact](https://github.com/JegernOUTT/refact) monorepo.

## Development

```bash
cd plugins/intellij
./gradlew check
```

The CI-built plugin bundles the `refact-chat-js` UI webview, while the `refact-lsp` engine is downloaded on first run from GitHub Releases by the runtime resolver. For local development, `update-dependencies.sh` can still stage a debug engine binary for `runIde`.

## Repository history

This code was migrated from the archived standalone JetBrains plugin repository. Historical releases and tags remain available there for reference.

## Issues

Please report issues in the monorepo: <https://github.com/JegernOUTT/refact/issues>.
