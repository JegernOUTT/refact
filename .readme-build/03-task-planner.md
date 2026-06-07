## Task planner & agent fleets
![Task planner board with per-card agents](media/agent-task-planner.gif)
Describe a feature once; the planner cracks it into cards, launches a tiny swarm, and keeps every agent in its own git worktree so experiments can collide with reality without colliding with each other. Watch the board light up as sandboxed agents build, test, report, race, and get squash-merged back in dependency order.

- Planner chats turn broad goals into executable cards with target files, priorities, assumptions, follow-ups, and structured final reports instead of one mega-prompt trying to hold the whole quest in its paws.
- The board is real Kanban: `planned`, `doing`, `done`, `failed`, and `regressed` columns, plus `depends_on` edges that separate ready cards from blocked cards so the fleet runs in the right order.
- Each spawned card agent gets its own branch and isolated worktree under the worktree registry; edits, tests, crashes, and spicy gremlin detours stay quarantined until the planner chooses to merge.
- Agents self-verify before finishing, then a verifier can re-check the card, capture command results, concerns, and recommendations, and leave a planner-visible report on the card.
- The planner owns the merge loop: inspect the agent diff, merge or squash the branch, run post-merge checks, auto-mark regressions, and preserve dirty worktrees when a run needs inspection or rescue.
- A/B card racing lets the planner spawn two variants for one card, compare their worktrees, then pick the winner instead of arguing with vibes in a comment thread.
- Live steering is built in: pause, resume, cancel, restart fresh or resume from a retained worktree, broadcast guidance, answer agent questions, and inspect pulses for state, last activity, tool calls, edits, and blockers.
- The payoff is ridiculous in the best way: describe the feature, watch a fleet of sandboxed agents build and test the pieces in parallel, then let the planner merge the clean winners into one coherent change.

→ Deep dive: [Task Planner and Cards](https://github.com/JegernOUTT/refact/wiki/Task-Planner-and-Cards), [Worktrees](https://github.com/JegernOUTT/refact/wiki/Worktrees)
