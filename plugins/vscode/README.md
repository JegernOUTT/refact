# Refact VS Code Plugin

This VS Code extension is part of the [JegernOUTT/refact](https://github.com/JegernOUTT/refact) monorepo.

## Development

```bash
cd plugins/vscode
npm ci
npm install ../../refact-agent/gui/refact-chat-js-*.tgz --no-save
npm run compile
npm run lint
```

Build the GUI package first if the tarball is not present:

```bash
cd refact-agent/gui
npm ci
npm run build
npm pack
```

The extension packages the local `refact` engine and `refact-chat-js` UI artifacts through the monorepo GitHub Actions workflows.

## Repository history

This code was migrated from the archived standalone VS Code plugin repository. Historical releases and tags remain available there for reference.

## Issues

Please report issues in the monorepo: <https://github.com/JegernOUTT/refact/issues>.
