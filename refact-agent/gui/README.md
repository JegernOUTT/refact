# Refact Agent GUI (`refact-chat-js`)

`refact-chat-js` is the React/Vite chat UI package used by Refact IDE plugins and by standalone web development. It renders provider setup, chat threads, agent tools, tasks, knowledge, integrations, trajectories, and settings while talking to the local `refact-lsp` engine over HTTP and SSE.

The package builds browser and event-bus bundles from the monorepo source and is published under the `refact-chat-js` package name.

## Package outputs

`npm run build` creates two distribution areas:

| Output                      | Purpose                                                     |
| --------------------------- | ----------------------------------------------------------- |
| `dist/chat/index.js`        | Browser ESM bundle that exports `render` and public helpers |
| `dist/chat/index.umd.cjs`   | Browser UMD/CommonJS bundle named `RefactChat`              |
| `dist/chat/index.d.ts`      | Type declarations for the browser bundle                    |
| `dist/chat/style.css`       | Compiled chat styles                                        |
| `dist/events/index.js`      | ESM event/public API bundle for host integrations           |
| `dist/events/index.umd.cjs` | UMD/CommonJS event/public API bundle                        |
| `dist/events/index.d.ts`    | Type declarations for host event APIs                       |

The package root exports `dist/chat`, and `refact-chat-js/dist/events` exports the event/public API bundle.

## Local development

```bash
cd refact-agent/gui
npm ci

# In another terminal, run the engine on http://127.0.0.1:8001
cd ../engine
cargo run -- --http-port 8001 --logs-stderr

# Start Vite. The /v1 proxy uses REFACT_LSP_URL or http://127.0.0.1:8001.
cd ../gui
REFACT_LSP_URL="http://127.0.0.1:8001" npm run dev
```

Open the Vite URL printed by the dev command. The default host mode is `web`.

## Scripts

Scripts come from `package.json`:

| Command                    | Action                                                   |
| -------------------------- | -------------------------------------------------------- |
| `npm run dev`              | Start Vite for local development                         |
| `npm run build`            | Type-check and build browser plus event bundles          |
| `npm run preview`          | Preview the built Vite app                               |
| `npm run types`            | Run `tsc --noEmit`                                       |
| `npm run lint`             | Run ESLint with zero warnings allowed                    |
| `npm run test`             | Run Vitest unit tests excluding integration tests        |
| `npm run test:unit`        | Run unit tests once                                      |
| `npm run test:integration` | Run integration tests under `src/__tests__/integration/` |
| `npm run test:all`         | Run all Vitest tests                                     |
| `npm run format:check`     | Check Prettier formatting                                |
| `npm run format`           | Apply Prettier formatting                                |
| `npm run storybook`        | Start Storybook on port 6006                             |
| `npm run build-storybook`  | Build Storybook                                          |

For TypeScript changes, run at least:

```bash
npm run types
npm run lint
npm run test
```

## Rendering API

Browser hosts can render the UI into an existing element:

```ts
import { render } from "refact-chat-js";

const root = document.getElementById("refact-chat");
if (root) {
  render(root, {
    host: "web",
    lspUrl: "http://127.0.0.1:8001",
  });
}
```

The UMD build exposes the same API as `window.RefactChat.render`.

`render(element, config)` accepts a partial GUI `Config` object. Common fields:

| Field                  | Purpose                                                            |
| ---------------------- | ------------------------------------------------------------------ |
| `host`                 | One of `web`, `ide`, `vscode`, or `jetbrains`                      |
| `lspUrl`               | Base URL for the local `refact-lsp` HTTP API                       |
| `lspPort`              | Local engine port when `lspUrl` is not provided                    |
| `tabbed`               | Enables tab-style host behavior used by some IDE surfaces          |
| `dev`                  | Enables development behavior while previewing another host mode    |
| `themeProps`           | Radix theme options passed to the app theme wrapper                |
| `features`             | Optional feature switches for statistics, VecDB, AST, and images   |
| `keyBindings`          | Host-provided key binding labels                                   |
| `apiKey`               | Optional API key forwarded to API requests when a host requires it |
| `shiftEnterToSubmit`   | Toggles chat submit behavior                                       |
| `currentWorkspaceName` | Display name for the active workspace                              |

## Host modes and event API

Host modes choose how the GUI communicates with its environment:

- `web` runs directly in a browser and talks to `refact-lsp` through HTTP/SSE.
- `ide` is a generic postMessage host mode for IDE containers.
- `vscode` uses the VS Code webview bridge.
- `jetbrains` uses the JetBrains webview bridge.

The typed public/event API is exported from `refact-chat-js/dist/events`. It includes configuration actions, active-file and selected-snippet actions, chat thread helpers, FIM actions, IDE commands such as opening files or pasting diffs, tool-call request/response helpers, and type guards for Refact API responses.

See `src/events/index.ts`, `src/events/setup.ts`, `src/hooks/useEventBusForIDE.ts`, and `src/hooks/useEventBusForApp.ts` for the current protocol surface.

## Architecture pointers

| Path                          | Notes                                                      |
| ----------------------------- | ---------------------------------------------------------- |
| `src/lib/render/`             | Public render entry point                                  |
| `src/app/`                    | Redux store, middleware, persistence configuration         |
| `src/features/Chat/Thread/`   | Thread state, actions, selectors, reducers, and types      |
| `src/services/refact/`        | HTTP API clients, chat commands, and SSE subscription code |
| `src/components/ChatContent/` | Message, tool, diff, reasoning, and citation rendering     |
| `src/components/ChatForm/`    | Prompt form, image attachments, and tool confirmation UI   |
| `src/features/Providers/`     | BYOK/local provider setup UI                               |
| `src/features/Integrations/`  | Integration setup and status UI                            |
| `src/features/Tasks/`         | Task board UI                                              |
| `src/events/`                 | Host integration event exports                             |

## Links

- Root repository: <https://github.com/smallcloudai/refact>
- Documentation: <https://docs.refact.ai/>
- Issues: <https://github.com/smallcloudai/refact/issues>
- Discussions: <https://github.com/smallcloudai/refact/discussions>
