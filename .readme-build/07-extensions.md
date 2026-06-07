## Modern extension surface — MCP, skills, slash commands, hooks, subagents & marketplace
![MCP, skills, and subagents](media/mcp-skills.gif)
Plug in almost anything — local CLIs, HTTP/SSE services, project rituals, specialist agents, reusable prompts — without dumping the whole universe into the model. Refact keeps the surface sharp: discover late, load only what matters, and fan out focused workers when the job wants a swarm.

- **MCP without context spam:** connect stdio servers or Streamable HTTP/SSE endpoints, including OAuth-capable configs, then let lazy discovery do the heavy lifting: `tool_search` finds the right capability and `mcp_call` runs it only when needed.
- **Skills on demand:** focused instruction packs stay out of the prompt until the task calls for them; load a skill with `load_skill`, work inside its guidance, then `deload_skill` to compact the run back into a clean report.
- **Slash-command workflows:** project and installed commands turn repeatable rituals into one-keystroke launches — reviews, migrations, diagnostics, release prep, or whatever your team keeps retyping.
- **Hooks for automation seams:** pre/post-tool and lifecycle hooks let extensions react around tool calls, sessions, and subagent runs, so policy, logging, formatting, and handoff glue can live beside the workflow instead of inside the chat.
- **Subagents as real tools:** project-defined subagents can expose schemas, run as first-class agentic tools, and execute in parallel when marked safe — perfect for investigation swarms, code reading, research, and scoped background work.
- **Marketplace-installed powers:** Skill, Command, and Subagent extensions can be browsed and installed from the marketplace, with the GUI Extensions page managing creation, editing, sources, and installed packs.
- **The payoff:** huge extension catalogs stay calm, because Refact surfaces the exact capability at the exact moment: search it, load it, run it, summarize it, and keep moving.

→ Deep dive: [MCP](https://github.com/JegernOUTT/refact/wiki/MCP), [Skills, Commands & Hooks](https://github.com/JegernOUTT/refact/wiki/Skills-Commands-Hooks), [Subagents](https://github.com/JegernOUTT/refact/wiki/Subagents), [Marketplace](https://github.com/JegernOUTT/refact/wiki/Marketplace)
