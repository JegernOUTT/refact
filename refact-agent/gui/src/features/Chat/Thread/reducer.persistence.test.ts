import { afterEach, describe, expect, it, vi } from "vitest";

const persistenceModulePath = "../../../utils/chatUiPersistence";

describe("chatReducer persisted startup", () => {
  afterEach(() => {
    vi.doUnmock(persistenceModulePath);
    vi.resetModules();
  });

  it("hydrates lightweight runtimes for persisted chat tabs including Buddy", async () => {
    vi.resetModules();
    vi.doMock(persistenceModulePath, async () => {
      const actual = await vi.importActual<
        typeof import("../../../utils/chatUiPersistence")
      >(persistenceModulePath);

      return {
        ...actual,
        loadPersistedChatTabs: () => ({
          openThreadIds: ["chat-a", "chat-b"],
          currentThreadId: "chat-b",
          tabs: [
            {
              id: "chat-a",
              title: "Explore tab",
              mode: "EXPLORE",
              tool_use: "explore",
              session_state: "completed",
            },
            {
              id: "chat-b",
              title: "Buddy tab",
              mode: "buddy",
              tool_use: "agent",
              session_state: "generating",
              is_buddy_chat: true,
            },
          ],
        }),
      };
    });

    const { chatReducer } = await import("./reducer");
    const state = chatReducer(undefined, { type: "@@INIT" });

    expect(state.open_thread_ids).toEqual(["chat-a", "chat-b"]);
    expect(state.current_thread_id).toBe("chat-b");
    expect(state.threads["chat-a"]?.thread.title).toBe("Explore tab");
    expect(state.threads["chat-a"]?.thread.mode).toBe("explore");
    expect(state.threads["chat-a"]?.thread.tool_use).toBe("explore");
    expect(state.threads["chat-a"]?.session_state).toBe("completed");
    expect(state.threads["chat-b"]?.thread.title).toBe("Buddy tab");
    expect(state.threads["chat-b"]?.thread.buddy_meta).toEqual({
      is_buddy_chat: true,
      buddy_chat_kind: "conversation",
      workflow_id: null,
    });
  });
});

it("reorders duplicate-agent tie winners with persisted runtime membership", async () => {
  vi.resetModules();
  let order = ["a", "b"];
  vi.doMock(persistenceModulePath, async () => {
    const actual = await vi.importActual<
      typeof import("../../../utils/chatUiPersistence")
    >(persistenceModulePath);
    return {
      ...actual,
      loadPersistedChatTabs: () => ({
        openThreadIds: order,
        currentThreadId: order[0],
        tabs: order.map((id) => ({ id, title: id })),
      }),
    };
  });
  try {
    const { chatReducer } = await import("./reducer");
    const { applyChatEvent, hydratePersistedChatTabs, closeThread } =
      await import("./actions");
    const { selectBackgroundAgentPool } = await import("./selectors");
    let state = chatReducer(undefined, { type: "@@INIT" });
    const agent = {
      agent_id: "shared",
      parent_chat_id: "a",
      child_chat_id: "child",
      kind: "subagent",
      status: "running",
      title: "a",
      progress: null,
      step_count: 0,
      last_activity: null,
      target_files: [],
      edited_files: [],
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
    } satisfies import("../../../services/refact/types").BackgroundAgentSummary;
    for (const id of order)
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: id,
          seq: "1",
          type: "background_agent_updated",
          agent: { ...agent, title: id },
        }),
      );
    const pool = () =>
      selectBackgroundAgentPool({
        chat: state,
      } as import("../../../app/store").RootState);
    expect(pool().shared.title).toBe("a");
    order = ["b", "a"];
    state = chatReducer(state, hydratePersistedChatTabs());
    expect(Object.keys(state.threads)).toEqual(order);
    expect(state.metadata?.ids).toEqual(order);
    expect(pool().shared.title).toBe("b");
    state = chatReducer(state, closeThread({ id: "b", force: true }));
    expect(pool().shared.title).toBe("a");
  } finally {
    vi.doUnmock(persistenceModulePath);
    vi.resetModules();
  }
});
