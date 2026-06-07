## Persistent project memory
![Knowledge save and autoinjection](media/memory.gif)
Refact turns your repo into a local memory palace: plans, notes, trajectories, code semantics, and background signals live under `<project>/.refact/`, ready for the next session without leaving your machine.
Static plans are the uncrushable spine—`set_plan` pins the base, `update_plan` appends deltas, and `get_plan` synthesizes the current truth while compression is forbidden from erasing it.

- **Local by design:** project knowledge, trajectories, task memory, integrations, schedules, and VecDB state are scoped to `.refact/`, with global memory only added explicitly through the configured knowledge directory.
- **Plans that survive the squeeze:** hidden `plan` messages and `event(plan_delta)` updates are marked `Never` in chat-history compression, so long agent runs can trim noise without eating the mission briefing.
- **Graph + vector recall:** a petgraph knowledge graph links documents to tags, file refs, entities, links, and supersession edges, while SQLite + vec0 embeddings index code, Markdown, and trajectory chunks for semantic search.
- **Autoinjected context, not paste spam:** @-commands and memory injection turn recall into `context_file` messages, then token-aware postprocessing resolves paths, AST-marks useful regions, and keeps only the sharpest context slices.
- **Typed operational memory:** decisions, specs, gotchas, risks, handoffs, progress notes, postmortems, briefs, and task memories become searchable project artifacts instead of disappearing into chat scrollback.
- **Trajectories become fuel:** saved conversations under `.refact/trajectories/` are chunked into metadata plus message windows, making past tool results and decisions discoverable without replaying the whole transcript.
- **Background indexers keep watch:** AST, VecDB, knowledge cleanup, and Buddy memory observers refresh the map while respecting shutdown and project scope, so future chats start warm instead of blank.

→ Deep dive: [Memory and Knowledge](https://github.com/JegernOUTT/refact/wiki/Memory-and-Knowledge), [Hidden Roles and Plans](https://github.com/JegernOUTT/refact/wiki/Hidden-Roles-and-Plans)
