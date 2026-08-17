# @refact/vite-plugin-design

Development-only React/Vite instrumentation for Refact Design Mode. The plugin adds stable source metadata to rendered intrinsic JSX elements and injects a handshake-gated browser runtime for element picking, temporary style edits, annotation pins, and Apply serialization.

## Supported scope

- Supported: React source compiled by Vite from `.jsx` and `.tsx` files, including function, arrow-function, and class component names.
- Instrumented: intrinsic DOM JSX elements such as `<button>` and `<section>`. Custom components are not tagged directly; their rendered DOM descendants are tagged with the owning component name.
- Not supported: Vue, Svelte, Solid, Angular, server-render-only DOM, non-Vite build systems, or production builds.
- Source locations are repository-relative `file:line:column` values from the JSX parser. Source maps cannot associate a rendered DOM node with its owning JSX element.

The plugin uses Vite's `apply: "serve"` lifecycle. Production builds contain neither source attributes nor the injected runtime.

## Install

```sh
npm install --save-dev @refact/vite-plugin-design
```

Add the plugin before the React transform:

```ts
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import refactDesign from "@refact/vite-plugin-design";

export default defineConfig({
  plugins: [
    refactDesign({
      allowedParentOrigins: [
        "http://127.0.0.1:8001",
        "http://localhost:8001",
      ],
    }),
    react(),
  ],
});
```

List only exact Refact parent origins. Paths are discarded during validation because `MessageEvent.origin` contains only the origin.

## Runtime and security

Loading an instrumented app normally has no active Design behavior. The injected module only installs one `message` listener. It creates the picker overlay and interaction listeners after all of these checks pass:

1. `event.source` is the app's `window.parent`.
2. `event.origin` exactly matches an `allowedParentOrigins` entry.
3. The message is a valid `refact:set-state` payload.

The first valid state message is the handshake. The runtime then replies with `refact:design-ready`. Child-to-parent traffic is `refact:design-ready`, `refact:element-selected`, `refact:iframe-blocked`, and `refact:send-followup-turn`; parent-to-child traffic is `refact:set-state` and `refact:call-tool`. Hover and click selection emit only the documented `DesignElementSelection`: selector, role, accessible name, rectangle, selected computed-style properties, JSX source file/line, and a nullable crop data URL. It does not expose component props, application state, closures, storage, cookies, or arbitrary DOM serialization.

Temporary design edits use `HTMLElement.style.setProperty` only. `clearPendingStyleEdits()` restores the original inline declarations. Annotation pins remain in the injected overlay and are never written to the application. `apply(instruction, screenshot)` JSON-serializes `{ edits, instruction, screenshot }` into one `refact:send-followup-turn` payload; source changes remain the agent's responsibility.

The parent controls the cross-origin runtime through the existing `refact:call-tool` envelope. Supported names are `design.apply-style-edit` (`selector`, `styles`), `design.clear-style-edits`, `design.add-annotation` (`id`, `selector`, `label`), `design.remove-annotation` (`id`), and `design.apply` (`instruction`, `screenshot`). Calls received before the handshake or from any other window/origin are ignored.

## Host framing matrix

| Host | Current evidence | Required host work | Failure behavior |
| --- | --- | --- | --- |
| VS Code webview | The current webview CSP in `plugins/vscode/src/sidebar.ts` and `chatTab.ts` has no `frame-src` directive, and webview options do not set `portMapping`. | Add `frame-src http://localhost:* http://127.0.0.1:*` to the generated CSP. Evaluate `WebviewOptions.portMapping` when the dev server is remote or the webview endpoint cannot directly address the workspace loopback port; this is the VS Code-supported localhost mapping mechanism. | Without those changes the nested iframe may be blocked before this runtime loads, so the Design surface must use its CDP fallback. |
| JetBrains JCEF | Refact's `JcefSupport.isAvailable()` wraps `JBCefApp.isSupported()` and the chat surface runs in `JBCefBrowser`. Chromium/JCEF supports nested HTTP iframes in principle. | Keep the existing availability guard. A real IDE/JCEF smoke must verify a nested `http://localhost:<port>` iframe for every supported IDE baseline; this package cannot execute that host test. | Unsupported JCEF must not create the webview. A target server may still deny embedding and trigger the CDP fallback. |
| Standalone web on `:8001` | The live app is a cross-origin iframe and the parent validates both its expected origin and exact `event.source`. | The target dev server must allow the Refact origin in CSP `frame-ancestors` and must not send a conflicting `X-Frame-Options` header. | Browsers do not expose the denial header to the parent page. T-55 detects iframe errors, an explicit `refact:iframe-blocked` message where available, or its load timeout, then drops to ladder step 3. |

The plugin cannot override a target application's `X-Frame-Options` or `frame-ancestors` policy. It also does not modify this repository's GUI build; consuming applications opt in explicitly.
