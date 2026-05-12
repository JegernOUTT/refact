# Contributing to the Refact JetBrains Plugin

The JetBrains plugin now lives in the Refact monorepo under `plugins/intellij`.

## Setup

```bash
cd plugins/intellij
./gradlew check
```

For local UI updates, build `refact-agent/gui` and copy its `dist` directory into `plugins/intellij/src/main/resources/webview/dist`, or use `plugins/intellij/update-dependencies.sh` from the monorepo checkout.

## Issues

Report plugin issues in the monorepo issue tracker: <https://github.com/smallcloudai/refact/issues>.
