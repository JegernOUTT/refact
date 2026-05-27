import { describe, expect, test, vi } from "vitest";
import { screen } from "@testing-library/react";
import { BackgroundAgentCard } from "../components/BackgroundAgentCard";
import {
  applyChatEvent,
  createChatWithId,
} from "../features/Chat/Thread/actions";
import { chatReducer } from "../features/Chat/Thread/reducer";
import {
  selectBackgroundAgent,
  selectBackgroundAgentsByThread,
} from "../features/Chat/Thread/selectors";
import type { Chat, ChatThreadRuntime } from "../features/Chat/Thread/types";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";
import type { BackgroundAgentSummary } from "../services/refact/types";
import { render } from "../utils/test-utils";

type SelectorRootState = Parameters<typeof selectBackgroundAgentsByThread>[0];

const chatId = "parent-chat";

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
  backgroundAgents: BackgroundAgentSummary[],
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
    background_agents: backgroundAgents,
  };
}

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

  test("reducer hydrates background agents from snapshot and resets prior state", () => {
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
      [snapshotAgent.agent_id]: snapshotAgent,
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

    expect(screen.getByText("running")).toBeInTheDocument();
    expect(screen.getByText("Subagent: Inspect the frogs")).toBeInTheDocument();
    expect(screen.getByText("src/frog.ts")).toBeInTheDocument();
    expect(screen.getByText("src/toad.ts")).toBeInTheDocument();
  });

  test("BackgroundAgentCard renders error badge when error is set", () => {
    render(<BackgroundAgentCard agent={makeAgent({ error: "boom" })} />);

    expect(screen.getByText("error")).toBeInTheDocument();
    expect(screen.getByText("boom")).toBeInTheDocument();
  });

  test("BackgroundAgentCard renders conflict badge when conflict_summary is set", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({ conflict_summary: "src/frog.ts conflicted" })}
      />,
    );

    expect(screen.getByText("⚠ conflicts")).toBeInTheDocument();
    expect(screen.getByText("src/frog.ts conflicted")).toBeInTheDocument();
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
      screen.getByRole("button", { name: "Open child trajectory" }),
    );

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
