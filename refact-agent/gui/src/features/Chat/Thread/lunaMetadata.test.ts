import { describe, expect, it } from "vitest";
import type { RootState } from "../../../app/store";
import type { ChatThreadRuntime } from "./types";
import { selectTabsDisplayData } from "./selectors";
import { chatReducer } from "./reducer";
import { applyChatEvent, updateOpenThread, saveTitle } from "./actions";

function runtime(id: string): ChatThreadRuntime {
  return {
    thread: {
      id,
      messages: [],
      title: "Initial title",
      model: "test-model",
      last_user_message_id: "",
      tool_use: "agent",
      mode: "agent",
      new_chat_suggested: { wasSuggested: false },
      boost_reasoning: false,
      is_task_chat: false,
      buddy_meta: {
        is_buddy_chat: false,
        buddy_chat_kind: "conversation",
        workflow_id: null,
      },
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
      status: { wasInteracted: false, confirmationStatus: true },
    },
    snapshot_received: true,
    task_widget_expanded: false,
    memory_enrichment_user_touched: false,
    manual_preview_items: [],
    manual_preview_ran: false,
  };
}

function stateWith(changedRuntime: ChatThreadRuntime): RootState {
  const second = runtime("second");
  return {
    chat: chatReducer(
      {
        current_thread_id: "second",
        open_thread_ids: ["first", "second"],
        threads: { first: changedRuntime, second },
        system_prompt: {},
        tool_use: "agent",
        checkpoints_enabled: true,
        sse_refresh_requested: null,
        stream_version: 0,
      },
      saveTitle({
        id: "first",
        title: "Initial title",
        isTitleGenerated: false,
      }),
    ),
    history: { chats: {} },
    notifications: { pendingByThread: {}, lastSeenByThread: {} },
  } as RootState;
}

type DisplayChange = {
  name: string;
  change: (thread: ChatThreadRuntime["thread"]) => ChatThreadRuntime["thread"];
  expected: Record<string, unknown>;
};

const singleFieldChanges: DisplayChange[] = [
  {
    name: "title",
    change: (thread) => ({ ...thread, title: "Changed title" }),
    expected: { title: "Changed title" },
  },
  {
    name: "mode",
    change: (thread) => ({ ...thread, mode: "explore" }),
    expected: { mode: "explore" },
  },
  {
    name: "task flag",
    change: (thread) => ({ ...thread, is_task_chat: true }),
    expected: { is_task_chat: true },
  },
  {
    name: "buddy flag",
    change: (thread) => ({
      ...thread,
      buddy_meta: {
        is_buddy_chat: true,
        buddy_chat_kind: "conversation",
        workflow_id: null,
      },
    }),
    expected: { is_buddy_chat: true },
  },
];

describe("tab metadata projection invalidation", () => {
  it.each(singleFieldChanges)(
    "updates the tab row for only $name",
    ({ change, expected }) => {
      const first = runtime("first");
      const state = stateWith(first);
      const before = selectTabsDisplayData(state);
      expect(state.chat.metadata).toBeDefined();
      const updated = change(first.thread);
      const chat = chatReducer(
        state.chat,
        updateOpenThread({
          id: "first",
          thread: { ...updated, isTitleGenerated: false },
        }),
      );
      const after = selectTabsDisplayData({ ...state, chat });

      expect(after).not.toBe(before);
      expect(after[0]).not.toBe(before[0]);
      expect(after[1]).toBe(before[1]);
      expect(after[0]).toMatchObject(expected);
    },
  );

  it("keeps tab data stable when transcript changes without display metadata", () => {
    const first = runtime("first");
    const state = stateWith(first);
    const before = selectTabsDisplayData(state);
    expect(state.chat.metadata).toBeDefined();
    const updated = {
      ...first,
      thread: {
        ...first.thread,
        messages: [{ role: "assistant" as const, content: "stream chunk" }],
      },
    };
    const chat = chatReducer(
      state.chat,
      applyChatEvent({
        chat_id: "first",
        seq: "1",
        type: "message_added",
        index: 0,
        message: updated.thread.messages[0],
      }),
    );
    const after = selectTabsDisplayData({ ...state, chat });

    expect(after).toBe(before);
    expect(after[0]).toBe(before[0]);
    expect(after[1]).toBe(before[1]);
  });
});
