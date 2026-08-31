import { afterEach, describe, expect, test, vi } from "vitest";
import { screen } from "@testing-library/react";
import { BackgroundAgentCard } from "../components/BackgroundAgentCard";
import { ToolContent } from "../components/ChatContent/ToolsContent";
import {
  applyChatEvent,
  createChatWithId,
} from "../features/Chat/Thread/actions";
import { chatReducer } from "../features/Chat/Thread/reducer";
import {
  selectActiveBackgroundAgents,
  selectAgentsAggregateUsage,
  selectBackgroundAgent,
  selectBackgroundAgentsByThread,
  selectBackgroundAgentsTree,
  selectPendingAgentQuestions,
  selectToolResultById,
} from "../features/Chat/Thread/selectors";
import type { Chat, ChatThreadRuntime } from "../features/Chat/Thread/types";
import {
  normalizeBackgroundAgentSummary,
  subscribeToChatEvents,
  type ChatEventEnvelope,
} from "../services/refact/chatSubscription";
import type {
  BackgroundAgentSummary,
  ChatMessages,
  ToolCall,
  ToolMessage,
} from "../services/refact/types";
import { render } from "../utils/test-utils";

type SelectorRootState = Parameters<typeof selectBackgroundAgentsByThread>[0];

const chatId = "parent-chat";

const subagentToolCall: ToolCall = {
  id: "call-bg",
  index: 0,
  type: "function",
  function: {
    name: "subagent",
    arguments: JSON.stringify({
      task: "Inspect the frogs",
      expected_result: "frog facts",
    }),
  },
};

function makeAgent(
  overrides: Partial<BackgroundAgentSummary> = {},
): BackgroundAgentSummary {
  return {
    agent_id: "bgagent-1",
    parent_chat_id: chatId,
    child_chat_id: "child-chat",
    kind: "subagent",
    status: "running",
    title: "Inspect the frogs",
    progress: "Reading files",
    step_count: 3,
    last_activity: "cat frog.py",
    target_files: ["src/frog.ts"],
    edited_files: ["src/toad.ts"],
    diff_summary: null,
    conflict_summary: null,
    result_summary: null,
    error: null,
    started_at: null,
    finished_at: null,
    change_seq: 1,
    model_type: null,
    current_tool: null,
    goal_summary: null,
    plan_present: false,
    worktree_branch: null,
    merge_status: null,
    pending_questions: 0,
    questions: [],
    tokens_used: 0,
    cost_usd: null,
    ...overrides,
  };
}

function makeRuntime(id: string): ChatThreadRuntime {
  return {
    thread: {
      id,
      messages: [],
      title: "Test Chat",
      model: "gpt-4",
      tool_use: "agent",
      new_chat_suggested: { wasSuggested: false },
      boost_reasoning: false,
      increase_max_tokens: false,
      include_project_info: true,
      auto_enrichment_enabled: false,
    },
    streaming: false,
    waiting_for_response: false,
    prevent_send: false,
    error: null,
    queued_items: [],
    send_immediately: false,
    attached_images: [],
    attached_text_files: [],
    background_agents: {},
    confirmation: {
      pause: false,
      pause_reasons: [],
      status: {
        wasInteracted: false,
        confirmationStatus: true,
      },
    },
    snapshot_received: true,
    task_widget_expanded: false,
    memory_enrichment_user_touched: false,
    manual_preview_items: [],
    manual_preview_ran: false,
  };
}

function makeState(id = chatId): Chat {
  return {
    current_thread_id: id,
    open_thread_ids: [id],
    threads: { [id]: makeRuntime(id) },
    system_prompt: {},
    tool_use: "agent",
    sse_refresh_requested: null,
    stream_version: 0,
  };
}

function makeSnapshot(
  id: string,
  backgroundAgents: unknown[],
  seq = "1",
): ChatEventEnvelope {
  return {
    chat_id: id,
    seq,
    type: "snapshot",
    thread: {
      id,
      title: "Snapshot Chat",
      model: "gpt-4",
      mode: "AGENT",
      tool_use: "agent",
      boost_reasoning: false,
      context_tokens_cap: null,
      include_project_info: true,
      checkpoints_enabled: true,
      is_title_generated: false,
    },
    runtime: {
      state: "idle",
      paused: false,
      error: null,
      queue_size: 0,
      pause_reasons: [],
      queued_items: [],
    },
    messages: [],
    background_agents: backgroundAgents as BackgroundAgentSummary[],
  };
}

function makeToolResult(overrides: Partial<ToolMessage> = {}): ToolMessage {
  return {
    role: "tool",
    content: "started background agent",
    tool_call_id: subagentToolCall.id ?? "call-bg",
    tool_failed: false,
    ...overrides,
  };
}

function renderToolContent(
  messages: ChatMessages,
  backgroundAgents: Record<string, BackgroundAgentSummary> = {},
  toolCall = subagentToolCall,
) {
  const state = makeState();
  const runtime = state.threads[chatId];
  if (!runtime) throw new Error("missing runtime");
  runtime.thread.messages = messages;
  runtime.background_agents = backgroundAgents;

  return render(<ToolContent toolCalls={[toolCall]} />, {
    preloadedState: { chat: state },
  });
}

async function parseSnapshot(
  backgroundAgents: unknown[],
): Promise<ChatEventEnvelope> {
  const onEvent = vi.fn<(event: ChatEventEnvelope) => void>();
  const event = makeSnapshot(chatId, backgroundAgents);

  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>(() =>
      Promise.resolve(
        new Response(`data: ${JSON.stringify(event)}\n\n`, {
          status: 200,
          headers: { "Content-Type": "text/event-stream" },
        }),
      ),
    ),
  );

  subscribeToChatEvents(
    chatId,
    { host: "vscode", lspPort: 8001 },
    {
      onEvent,
      onError: vi.fn(),
    },
  );

  await new Promise((resolve) => setTimeout(resolve, 10));

  expect(onEvent).toHaveBeenCalledTimes(1);
  return onEvent.mock.calls[0][0];
}

async function reduceParsedSnapshot(
  backgroundAgents: unknown[],
): Promise<Chat> {
  const event = await parseSnapshot(backgroundAgents);
  return chatReducer(makeState(), applyChatEvent(event));
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("background agents", () => {
  test("reducer adds agent on BackgroundAgentUpdated", () => {
    const agent = makeAgent();
    const state = chatReducer(
      makeState(),
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "background_agent_updated",
        agent,
      }),
    );

    expect(state.threads[chatId]?.background_agents).toEqual({
      [agent.agent_id]: agent,
    });
  });

  test("reducer replaces duplicate agent_id with latest update", () => {
    const first = makeAgent({
      current_tool: "cat",
      questions: [],
      change_seq: 1,
    });
    const latest = makeAgent({
      current_tool: "search_pattern",
      model_type: "thinking",
      pending_questions: 1,
      questions: [{ id: "q-1", text: "Continue?" }],
      tokens_used: 42,
      cost_usd: 0.5,
      change_seq: 2,
    });
    let state = chatReducer(
      makeState(),
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "background_agent_updated",
        agent: first,
      }),
    );
    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq: "2",
        type: "background_agent_updated",
        agent: latest,
      }),
    );

    expect(state.threads[chatId]?.background_agents[latest.agent_id]).toEqual(
      latest,
    );
  });

  test("selectors build a cycle-safe three-level tree and aggregate its usage", () => {
    const root = makeAgent({
      agent_id: "root",
      child_chat_id: "child-1",
      last_activity: "2026-01-01T00:00:00Z",
      tokens_used: 10,
      cost_usd: 0.1,
      questions: [{ id: "root-question", text: "Need input?" }],
    });
    const child = makeAgent({
      agent_id: "child",
      parent_chat_id: "child-1",
      child_chat_id: "child-2",
      last_activity: "2026-01-02T00:00:00Z",
      tokens_used: 20,
      cost_usd: 0.2,
      questions: [
        { id: "answered", text: "Done?", answer: "Yes" },
        { id: "child-question", text: "Proceed?", answer: null },
      ],
    });
    const grandchild = makeAgent({
      agent_id: "grandchild",
      parent_chat_id: "child-2",
      child_chat_id: "child-1",
      tokens_used: 30,
      cost_usd: null,
      status: "queued",
    });
    const orphan = makeAgent({
      agent_id: "orphan",
      parent_chat_id: "missing",
      tokens_used: 100,
      cost_usd: 1,
    });
    const state = makeState();
    const runtime = state.threads[chatId];
    if (!runtime) throw new Error("missing runtime");
    runtime.background_agents = {
      root,
      child,
      grandchild,
      orphan,
    };
    const rootState = { chat: state } as SelectorRootState;

    expect(selectBackgroundAgentsTree(rootState, chatId)).toEqual([
      {
        agent: root,
        children: [
          {
            agent: child,
            children: [{ agent: grandchild, children: [] }],
          },
        ],
      },
    ]);
    expect(selectAgentsAggregateUsage(rootState, chatId)).toEqual({
      tokensTotal: 60,
      costTotal: 0.30000000000000004,
      runningCount: 2,
    });
    expect(selectPendingAgentQuestions(rootState, chatId)).toEqual([
      root.questions?.[0],
      child.questions?.[1],
    ]);
    expect(
      selectActiveBackgroundAgents(rootState, chatId).map(
        (agent) => agent.agent_id,
      ),
    ).toEqual(["child", "root", "grandchild", "orphan"]);
  });

  test("selectors return null cost when no tree agent reports cost", () => {
    const agent = makeAgent({ cost_usd: null, tokens_used: 8 });
    const state = makeState();
    const runtime = state.threads[chatId];
    if (!runtime) throw new Error("missing runtime");
    runtime.background_agents = { [agent.agent_id]: agent };

    expect(
      selectAgentsAggregateUsage({ chat: state } as SelectorRootState, chatId),
    ).toEqual({ tokensTotal: 8, costTotal: null, runningCount: 1 });
  });

  test("reducer replaces duplicate agent_id with latest status update", () => {
    const first = makeAgent({ progress: "Queued", change_seq: 1 });
    const latest = makeAgent({
      progress: "Done",
      status: "completed",
      change_seq: 2,
    });
    let state = chatReducer(
      makeState(),
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "background_agent_updated",
        agent: first,
      }),
    );

    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq: "2",
        type: "background_agent_updated",
        agent: latest,
      }),
    );

    expect(state.threads[chatId]?.background_agents).toEqual({
      [latest.agent_id]: latest,
    });
  });

  test("reducer ignores stale same-agent running update after terminal status", () => {
    const completed = makeAgent({ status: "completed", change_seq: 5 });
    const staleRunning = makeAgent({ status: "running", change_seq: 4 });
    const initial = makeState();
    const runtime = initial.threads[chatId];
    if (!runtime) throw new Error("missing runtime");
    runtime.background_agents = { [completed.agent_id]: completed };

    const state = chatReducer(
      initial,
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "background_agent_updated",
        agent: staleRunning,
      }),
    );

    expect(state.threads[chatId]?.background_agents).toEqual({
      [completed.agent_id]: completed,
    });
  });

  test("reducer ignores an unknown-sequence update after a known update", () => {
    const existing = makeAgent({ change_seq: 1, progress: "Known update" });
    const unknownSequence = normalizeBackgroundAgentSummary({
      agent_id: existing.agent_id,
      parent_chat_id: chatId,
      child_chat_id: null,
      kind: "subagent",
      status: "running",
      title: "Unknown update",
      progress: "Unknown update",
      last_activity: null,
      diff_summary: null,
      conflict_summary: null,
      result_summary: null,
      error: null,
      started_at: null,
      finished_at: null,
    });
    const initial = makeState();
    const runtime = initial.threads[chatId];
    if (!runtime) throw new Error("missing runtime");
    runtime.background_agents = { [existing.agent_id]: existing };

    const state = chatReducer(
      initial,
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "background_agent_updated",
        agent: unknownSequence,
      }),
    );

    expect(state.threads[chatId]?.background_agents).toEqual({
      [existing.agent_id]: existing,
    });
  });

  test("reducer ignores same-sequence non-terminal regression", () => {
    const active = makeAgent({
      status: "running",
      progress: "Applying patch",
      change_seq: 5,
    });
    const staleQueued = makeAgent({
      status: "running",
      progress: "Queued",
      change_seq: 5,
    });
    const initial = makeState();
    const runtime = initial.threads[chatId];
    if (!runtime) throw new Error("missing runtime");
    runtime.background_agents = { [active.agent_id]: active };

    const state = chatReducer(
      initial,
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "background_agent_updated",
        agent: staleQueued,
      }),
    );

    expect(state.threads[chatId]?.background_agents).toEqual({
      [active.agent_id]: active,
    });
  });

  test("reducer accepts same-sequence terminal update over running state", () => {
    const running = makeAgent({ status: "running", change_seq: 5 });
    const completed = makeAgent({ status: "completed", change_seq: 5 });
    const initial = makeState();
    const runtime = initial.threads[chatId];
    if (!runtime) throw new Error("missing runtime");
    runtime.background_agents = { [running.agent_id]: running };

    const state = chatReducer(
      initial,
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "background_agent_updated",
        agent: completed,
      }),
    );

    expect(state.threads[chatId]?.background_agents).toEqual({
      [completed.agent_id]: completed,
    });
  });

  test("snapshot does not revert terminal background agent to running", () => {
    const completed = makeAgent({ status: "completed", change_seq: 5 });
    const staleRunning = makeAgent({ status: "running", change_seq: 4 });
    const initial = makeState();
    const runtime = initial.threads[chatId];
    if (!runtime) throw new Error("missing runtime");
    runtime.background_agents = { [completed.agent_id]: completed };

    const state = chatReducer(
      initial,
      applyChatEvent(makeSnapshot(chatId, [staleRunning])),
    );

    expect(state.threads[chatId]?.background_agents).toEqual({
      [completed.agent_id]: completed,
    });
  });

  test("reducer merges background agents from snapshot with prior state", () => {
    const oldAgent = makeAgent({ agent_id: "old-agent" });
    const snapshotAgent = makeAgent({ agent_id: "snapshot-agent" });
    const initial = makeState();
    const runtime = initial.threads[chatId];
    if (!runtime) throw new Error("missing runtime");
    runtime.background_agents = { [oldAgent.agent_id]: oldAgent };

    const state = chatReducer(
      initial,
      applyChatEvent(makeSnapshot(chatId, [snapshotAgent])),
    );

    expect(state.threads[chatId]?.background_agents).toEqual({
      [oldAgent.agent_id]: oldAgent,
      [snapshotAgent.agent_id]: snapshotAgent,
    });
  });

  test("snapshot agent with null target_files defaults to empty list", async () => {
    const agent = {
      ...makeAgent({ agent_id: "null-target-files" }),
      target_files: null,
    };

    const state = await reduceParsedSnapshot([agent]);

    expect(
      state.threads[chatId]?.background_agents[agent.agent_id]?.target_files,
    ).toEqual([]);
  });

  test("snapshot agent with null agent_id is skipped", async () => {
    const invalidAgent = {
      ...makeAgent(),
      agent_id: null,
    };

    const state = await reduceParsedSnapshot([invalidAgent]);

    expect(state.threads[chatId]?.background_agents).toEqual({});
  });

  test("snapshot with valid, null, valid agents keeps the valid entries", async () => {
    const first = makeAgent({ agent_id: "valid-1" });
    const second = makeAgent({ agent_id: "valid-2" });

    const state = await reduceParsedSnapshot([first, null, second]);

    expect(state.threads[chatId]?.background_agents).toEqual({
      [first.agent_id]: first,
      [second.agent_id]: second,
    });
  });

  test("empty snapshot background agent list preserves prior state map", async () => {
    const initial = makeState();
    const oldAgent = makeAgent({ agent_id: "old-agent" });
    const runtime = initial.threads[chatId];
    if (!runtime) throw new Error("missing runtime");
    runtime.background_agents = { [oldAgent.agent_id]: oldAgent };
    const event = await parseSnapshot([]);

    const state = chatReducer(initial, applyChatEvent(event));

    expect(state.threads[chatId]?.background_agents).toEqual({
      [oldAgent.agent_id]: oldAgent,
    });
  });

  test("selectBackgroundAgentsByThread returns the right map", () => {
    const agent = makeAgent();
    const state = chatReducer(
      makeState(),
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "background_agent_updated",
        agent,
      }),
    );
    const rootState = { chat: state } as SelectorRootState;

    expect(selectBackgroundAgentsByThread(rootState, chatId)).toEqual({
      [agent.agent_id]: agent,
    });
    expect(selectBackgroundAgent(rootState, chatId, agent.agent_id)).toEqual(
      agent,
    );
  });

  test("BackgroundAgentCard renders status badge, title, target_files, and edited_files", () => {
    render(<BackgroundAgentCard agent={makeAgent()} />);

    expect(screen.getByTestId("background-agent-status-dot")).toHaveAttribute(
      "aria-label",
      "Background agent status: Running",
    );
    expect(screen.getByText("Inspect the frogs")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "1 target files" }),
    ).not.toBeInTheDocument();
  });

  test("BackgroundAgentCard renders error badge when error is set", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({ status: "failed", error: "boom" })}
      />,
    );

    expect(screen.getByTestId("background-agent-status-dot")).toHaveAttribute(
      "aria-label",
      "Background agent status: Failed",
    );
    expect(screen.queryByText("boom")).not.toBeInTheDocument();
  });

  test("BackgroundAgentCard renders conflict badge when conflict_summary is set", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          status: "completed",
          conflict_summary: "src/frog.ts conflicted",
        })}
      />,
    );

    expect(screen.getByTestId("background-agent-card")).toBeInTheDocument();
  });

  test("ToolContent renders BackgroundAgentCard from flattened tool fields", () => {
    renderToolContent([
      makeToolResult({
        background_agent_id: "bgagent-flat",
        background_agent_kind: "subagent",
        child_chat_id: "child-flat",
        background_agent_status: "running",
        target_files: ["src/flat.ts"],
      }),
    ]);

    expect(screen.getByTestId("background-agent-card")).toBeInTheDocument();
    expect(screen.getByTestId("background-agent-status-dot")).toHaveAttribute(
      "aria-label",
      "Background agent status: Running",
    );
    expect(
      screen.queryByRole("button", { name: "Copy agent id" }),
    ).not.toBeInTheDocument();
  });

  test("ToolContent keeps rendering BackgroundAgentCard from nested extra fallback", () => {
    renderToolContent([
      makeToolResult({
        extra: {
          background_agent_id: "bgagent-extra",
          background_agent_kind: "subagent",
          child_chat_id: "child-extra",
          background_agent_status: "running",
          target_files: ["src/extra.ts"],
        },
      }),
    ]);

    expect(screen.getByTestId("background-agent-card")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Expand background agent details" }),
    ).toBeInTheDocument();
  });

  test("ToolContent synthesizes introspection fields and supports legacy delegates", () => {
    const delegateToolCall: ToolCall = {
      ...subagentToolCall,
      id: "call-delegate",
      function: {
        name: "delegate",
        arguments: JSON.stringify({
          task: "Inspect frogs",
          expected_result: "frog facts",
          model_type: "thinking",
          model_name: "openai/gpt-5.6-terra",
          goal: "Document frog behavior",
          plan: "Read the frog files",
          worktree: true,
          auto_merge: true,
          target_files: ["src/frog.ts"],
        }),
      },
    };
    renderToolContent(
      [
        makeToolResult({
          tool_call_id: "call-delegate",
          extra: {
            background_agent_id: "bgagent-introspection",
            background_agent_kind: "delegate",
            background_agent_status: "running",
            model: "openai/gpt-5.6-terra",
            model_type: "thinking",
            current_tool: "shell: cargo test",
            goal_summary: "Document frog behavior",
            plan_present: true,
            worktree_branch: "refact/task/frogs",
            merge_status: "pending",
            pending_questions: 2,
            tokens_used: 12_300,
            cost_usd: 0.04,
          },
        }),
      ],
      {},
      delegateToolCall,
    );

    expect(screen.getByTestId("background-agent-kind-delegate")).toBeVisible();
    expect(screen.getByTestId("background-agent-model")).toHaveTextContent(
      "thinking",
    );
    expect(
      screen.getByTestId("background-agent-current-tool"),
    ).toHaveTextContent("shell: cargo test");
    expect(screen.getByTestId("background-agent-questions")).toHaveTextContent(
      "❓2",
    );
    expect(screen.getByTestId("background-agent-merge")).toHaveTextContent(
      "Pending",
    );
  });

  test("BackgroundAgentUpdated state overrides the flattened placeholder card", () => {
    renderToolContent(
      [
        makeToolResult({
          background_agent_id: "bgagent-updated",
          background_agent_kind: "subagent",
          child_chat_id: "child-updated",
          background_agent_status: "running",
        }),
      ],
      {
        "bgagent-updated": makeAgent({
          agent_id: "bgagent-updated",
          title: "Updated frog report",
          status: "completed",
          progress: "Done reading frogs",
          child_chat_id: "child-updated",
        }),
      },
    );

    expect(screen.getByTestId("background-agent-card")).toBeInTheDocument();
    expect(screen.getByText("Updated frog report")).toBeInTheDocument();
    expect(screen.getByTestId("background-agent-status-dot")).toHaveAttribute(
      "aria-label",
      "Background agent status: Completed",
    );
  });

  test("flattened message_added event keeps background agent fields on selected tool result", () => {
    const event = JSON.parse(
      JSON.stringify({
        chat_id: chatId,
        seq: "1",
        type: "message_added",
        index: 0,
        message: {
          role: "tool",
          content: "started from SSE",
          tool_call_id: "call-sse",
          tool_failed: false,
          background_agent_id: "bgagent-sse",
          background_agent_kind: "subagent",
          child_chat_id: "child-sse",
          background_agent_status: "running",
          target_files: ["src/sse.ts"],
        },
      }),
    ) as ChatEventEnvelope;

    const state = chatReducer(makeState(), applyChatEvent(event));
    const result = selectToolResultById(
      { chat: state } as Parameters<typeof selectToolResultById>[0],
      "call-sse",
    );

    expect(result?.background_agent_id).toBe("bgagent-sse");
    expect(result?.child_chat_id).toBe("child-sse");
    expect(result?.target_files).toEqual(["src/sse.ts"]);
  });

  test("clicking Open child trajectory calls onOpenTrajectory with child chat id", async () => {
    const onOpenTrajectory = vi.fn();
    const { user } = render(
      <BackgroundAgentCard
        agent={makeAgent()}
        onOpenTrajectory={onOpenTrajectory}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );
    await user.click(screen.getByRole("button", { name: "Open trajectory" }));

    expect(onOpenTrajectory).toHaveBeenCalledWith("child-chat");
  });

  test("reducer initializes background_agents for created threads", () => {
    const state = chatReducer(
      makeState(),
      createChatWithId({ id: "new-chat" }),
    );

    expect(state.threads["new-chat"]?.background_agents).toEqual({});
  });
});
