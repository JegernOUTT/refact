import { waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setUpStore } from "./store";
import {
  newChatAction,
  setChatModel,
  setMaxNewTokens,
  switchToThread,
} from "../features/Chat/Thread";
import type { ChatThreadRuntime } from "../features/Chat/Thread/types";
import {
  findLeaf,
  focusPane,
  setPaneActiveTab,
  type PaneNode,
} from "../features/ChatPanes";
function makeThread(id: string): ChatThreadRuntime {
  const mode = id.startsWith("chat-") ? "agent" : undefined;
  const title = id
    .split("-")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");

  return {
    thread: {
      id,
      messages: [],
      title,
      model: "",
      last_user_message_id: "",
      new_chat_suggested: { wasSuggested: false },
      mode,
    },
    session_state: "idle",
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

function makeChatState(currentThreadId: string, ids: string[]) {
  return {
    current_thread_id: currentThreadId,
    open_thread_ids: ids,
    threads: Object.fromEntries(ids.map((id) => [id, makeThread(id)])),
    system_prompt: {},
    tool_use: "explore" as const,
    sse_refresh_requested: null,
    stream_version: 0,
  };
}

function twoPaneRoot(): PaneNode {
  return {
    kind: "split",
    id: "root:split:row",
    dir: "row",
    sizes: [0.5, 0.5],
    children: [
      {
        kind: "leaf",
        id: "left",
        tabIds: ["chat-a"],
        activeTabId: "chat-a",
      },
      {
        kind: "leaf",
        id: "right",
        tabIds: ["chat-b"],
        activeTabId: "chat-b",
      },
    ],
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("task delete middleware", () => {
  it("task_delete_does_not_close_thread_with_overlapping_substring_id", () => {
    const THREAD_ID = "tabc-foo";
    const TASK_ID = "abc";

    const store = setUpStore({
      config: { host: "vscode", lspPort: 8001, themeProps: {} },
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

describe("chat pane routing middleware", () => {
  it("places new chats in the focused pane", async () => {
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-a", ["chat-a", "chat-b"]),
      panes: { root: twoPaneRoot(), focusedLeafId: "right" },
    });

    store.dispatch(newChatAction({ title: "Chat C" }));

    const newChatId = store.getState().chat.current_thread_id;
    await waitFor(() => {
      const right = findLeaf(store.getState().panes.root, "right");
      expect(right?.activeTabId).toBe(newChatId);
      expect(right?.tabIds).toContain(newChatId);
    });
    expect(store.getState().chat.open_thread_ids).toContain(newChatId);
  });

  it("places switched threads in the focused pane and keeps one pane owner", async () => {
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-a", ["chat-a", "chat-b", "chat-c"]),
      panes: { root: twoPaneRoot(), focusedLeafId: "left" },
    });

    store.dispatch(switchToThread({ id: "chat-b" }));

    await waitFor(() => {
      const left = findLeaf(store.getState().panes.root, "left");
      const right = findLeaf(store.getState().panes.root, "right");
      expect(left?.activeTabId).toBe("chat-b");
      expect(left?.tabIds).toContain("chat-b");
      expect(right?.tabIds).not.toContain("chat-b");
    });
    expect(store.getState().chat.current_thread_id).toBe("chat-b");
  });

  it("switches current_thread_id when pane tab selection changes", async () => {
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-a", ["chat-a", "chat-b", "chat-c"]),
      panes: {
        root: {
          kind: "leaf",
          id: "root",
          tabIds: ["chat-a", "chat-b"],
          activeTabId: "chat-a",
        },
        focusedLeafId: "root",
      },
    });

    store.dispatch(setPaneActiveTab({ leafId: "root", tabId: "chat-b" }));

    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe("chat-b");
    });
  });

  it("switches current_thread_id when pane focus changes", async () => {
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-a", ["chat-a", "chat-b"]),
      panes: { root: twoPaneRoot(), focusedLeafId: "left" },
    });

    store.dispatch(focusPane("right"));

    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe("chat-b");
    });
  });
});

describe("context limit middleware", () => {
  it("syncs the selected model context cap to backend", async () => {
    const THREAD_ID = "context-cap-chat";
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(new Response(null, { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);

    const store = setUpStore({
      config: { host: "vscode", lspPort: 8001, themeProps: {} },
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

    store.dispatch(setMaxNewTokens(128000));

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const [, init] = fetchMock.mock.calls[0] ?? [];
    expect(init).toBeDefined();
    const body = JSON.parse(String(init?.body)) as {
      type?: string;
      patch?: Record<string, unknown>;
    };

    expect(body.type).toBe("set_params");
    expect(body.patch).toEqual({ context_tokens_cap: 128000 });
  });

  it("syncs selected model and auto context cap together", async () => {
    const THREAD_ID = "context-cap-model-chat";
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(new Response(null, { status: 200 }));
    const thread = makeThread(THREAD_ID);
    thread.thread.model = "old-model";
    thread.thread.modelMaximumContextTokens = 8192;
    thread.thread.currentMaximumContextTokens = 8192;
    thread.thread.context_tokens_cap = 8192;
    vi.stubGlobal("fetch", fetchMock);

    const store = setUpStore({
      config: { host: "vscode", lspPort: 8001, themeProps: {} },
      chat: {
        current_thread_id: THREAD_ID,
        open_thread_ids: [THREAD_ID],
        threads: { [THREAD_ID]: thread },
        system_prompt: {},
        tool_use: "explore" as const,
        sse_refresh_requested: null,
        stream_version: 0,
      },
    });

    store.dispatch(
      setChatModel({
        model: "new-model",
        modelMaxContextTokens: 128000,
        previousModelMaxContextTokens: 8192,
      }),
    );

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const [, init] = fetchMock.mock.calls[0] ?? [];
    expect(init).toBeDefined();
    const body = JSON.parse(String(init?.body)) as {
      type?: string;
      patch?: Record<string, unknown>;
    };

    expect(body.type).toBe("set_params");
    expect(body.patch).toEqual({
      model: "new-model",
      context_tokens_cap: 128000,
    });
  });

  it("does not sync unchanged model context cap", async () => {
    const THREAD_ID = "unchanged-context-cap-chat";
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(new Response(null, { status: 200 }));
    const thread = makeThread(THREAD_ID);
    thread.thread.modelMaximumContextTokens = 128000;
    thread.thread.currentMaximumContextTokens = 128000;
    thread.thread.context_tokens_cap = 128000;
    vi.stubGlobal("fetch", fetchMock);

    const store = setUpStore({
      config: { host: "vscode", lspPort: 8001, themeProps: {} },
      chat: {
        current_thread_id: THREAD_ID,
        open_thread_ids: [THREAD_ID],
        threads: { [THREAD_ID]: thread },
        system_prompt: {},
        tool_use: "explore" as const,
        sse_refresh_requested: null,
        stream_version: 0,
      },
    });

    store.dispatch(setMaxNewTokens(128000));

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
