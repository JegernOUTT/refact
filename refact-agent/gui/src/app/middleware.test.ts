import { describe, expect, it } from "vitest";
import { setUpStore } from "./store";
import type { ChatThreadRuntime } from "../features/Chat/Thread/types";

function makeThread(id: string): ChatThreadRuntime {
  return {
    thread: {
      id,
      messages: [],
      title: "",
      model: "",
      last_user_message_id: "",
      new_chat_suggested: { wasSuggested: false },
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

describe("task delete middleware", () => {
  it("task_delete_does_not_close_thread_with_overlapping_substring_id", () => {
    const THREAD_ID = "tabc-foo";
    const TASK_ID = "abc";

    const store = setUpStore({
      chat: {
        current_thread_id: THREAD_ID,
        open_thread_ids: [THREAD_ID],
        threads: { [THREAD_ID]: makeThread(THREAD_ID) },
        system_prompt: {},
        tool_use: "explore" as const,
        sse_refresh_requested: null,
        stream_version: 0,
      },
    });

    store.dispatch({
      type: "tasksApi/executeMutation/fulfilled",
      payload: { deleted: true },
      meta: {
        requestId: "test-req",
        requestStatus: "fulfilled",
        arg: {
          endpointName: "deleteTask",
          originalArgs: TASK_ID,
          type: "mutation",
        },
      },
    });

    const state = store.getState();
    expect(state.chat.open_thread_ids).toContain(THREAD_ID);
    expect(state.chat.threads[THREAD_ID]).toBeDefined();
  });
});
