## Modes, transitions, compression & scheduling
![Switching modes and compacting context](media/chat-modes.gif)
Refact does not run one generic chatbot forever; it shifts between purpose-built operating systems for thought, code, review, planning, execution, memory, and time. The wild part: those shifts become durable context, so the agent can hand itself a plan, compact the past, and wake up later without losing the plot.

- **Mode as behavior, not branding:** `ask`, `explore`, `plan`, `agent`, `quick_agent`, `debug`, `buddy`, `task_planner`, and `task_agent` ship as separate defaults with distinct prompts, toolsets, subagent policies, and execution discipline.
- **Read-only when you need bearings, sharp tools when you need motion:** Explore gathers context without editing; Plan turns decisions into implementation shape; Agent and Quick Agent can operate autonomously with file, shell, process, MCP, knowledge, and task tools.
- **Task handoffs carry their own gravity:** switching or handing off modes records hidden `mode_switch` events instead of smearing instructions into chat text, and a Task Planner handoff with an `initial_plan` auto-creates a pinned task document so the board starts with the plan already attached.
- **Context is actively re-shaped mid-flight:** before model calls, history goes through a four-stage pressure valve—deduplicate context files, compress bulky tool results, repair tool-call/tool-result ordering, then trim to the budget.
- **Compression stays visible and auditable:** deterministic and reactive compaction create first-class `compression_report` messages with token accounting, while LLM segment summaries stay as source-preserving assistant artifacts instead of invisible prompt surgery.
- **Long sessions stay coherent across provider weirdness:** hidden plan/event roles are preserved in trajectories, exempted from destructive compression where needed, and lowered into provider-safe context frames instead of leaking fake roles onto the wire.
- **The agent can schedule its own future:** `cron_create`, `cron_list`, and `cron_delete` manage session-only or durable project jobs, with deterministic jitter, missed durable one-shot catch-up, 30-day recurring auto-expire, and `cron_fire` events that enqueue the saved prompt.
- **Waiting is a first-class move too:** interruptible `sleep` avoids blocking shell processes and can emit tick events, so “check again later” becomes part of the same observable runtime instead of a sticky-note outside the system.

→ Deep dive: [Agent Modes](https://github.com/JegernOUTT/refact/wiki/Agent-Modes), [Context Compression](https://github.com/JegernOUTT/refact/wiki/Context-Compression), [Scheduler and Cron](https://github.com/JegernOUTT/refact/wiki/Scheduler-and-Cron)
