# Contributing to the Refact VS Code Plugin

The VS Code plugin now lives in the Refact monorepo under `plugins/vscode`.

## Setup

```bash
cd refact-agent/gui
npm ci
npm run build
npm pack
cd ../../plugins/vscode
npm ci
npm install ../../refact-agent/gui/refact-chat-js-*.tgz --no-save
npm run compile
npm run lint
```

For local packaging, build or copy the engine binary into `plugins/vscode/assets/refact`.

## Issues

Report plugin issues in the monorepo issue tracker: <https://github.com/JegernOUTT/refact/issues>.
