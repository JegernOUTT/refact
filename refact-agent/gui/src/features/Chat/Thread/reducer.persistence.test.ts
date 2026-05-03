import { afterEach, describe, expect, it, vi } from "vitest";

const persistenceModulePath = "../../../utils/chatUiPersistence";

describe("chatReducer persisted startup", () => {
  afterEach(() => {
    vi.doUnmock(persistenceModulePath);
    vi.resetModules();
  });

  it("hydrates lightweight runtimes for persisted non-Buddy open tabs", async () => {
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

    expect(state.open_thread_ids).toEqual(["chat-a"]);
    expect(state.current_thread_id).toBe("chat-a");
    expect(state.threads["chat-a"]?.thread.title).toBe("Explore tab");
    expect(state.threads["chat-a"]?.thread.mode).toBe("explore");
    expect(state.threads["chat-a"]?.thread.tool_use).toBe("explore");
    expect(state.threads["chat-a"]?.session_state).toBe("completed");
    expect(state.threads["chat-b"]).toBeUndefined();
  });
});
