## Rust core + cross-device UI
![UI streamed from the Rust core, opened in a browser](media/any-device.gif)
One Rust binary becomes the whole control room: `refact-lsp` speaks LSP to editors, serves the React chat over Axum HTTP, and streams live agent state from the same origin. Open the chat UI in any browser that can reach the engine on your trusted local network, and the IDE becomes just one window into the same running brain.

- `refact-lsp` is both an Axum HTTP server and a tower-lsp server, so code completion, chat commands, caps, tools, checkpoints, and SSE snapshots all terminate in the same Rust process.
- The embedded React UI is served by the engine itself, with engine-origin candidates injected into `index.html` so browser, VSCode webview, and JetBrains JCEF hosts talk back to the right local process.
- IDE plugins stay thin: they provide file context and editor actions through LSP, HTTP, and a postMessage bridge while the engine owns sessions, tools, queues, providers, indexes, and streaming state.
- The standalone browser surface is real but scoped: use any browser that can reach the local engine origin, including a phone or tablet on the LAN when you intentionally bind it on a trusted network.
- ~12 background tokio workers keep the workspace hot: file discovery, caps refresh, git shadow cleanup, VecDB, knowledge graph, trajectory memos, notifications, agent monitors, stats, browser cleanup, Buddy, scheduler, and AST indexing.
- Local intelligence runs in-process: tree-sitter AST indexing covers 8 languages, SQLite+vec0 powers semantic search, and both feed the same agent/tool pipeline that the UI watches over SSE.
- One shared `GlobalContext` ties HTTP, LSP, background services, provider registries, task events, exec runtime, voice, Buddy, and workspace state together without turning the editor into the backend.

→ Deep dive: [Architecture](https://github.com/JegernOUTT/refact/wiki/Architecture), [GUI Architecture](https://github.com/JegernOUTT/refact/wiki/GUI-Architecture)
