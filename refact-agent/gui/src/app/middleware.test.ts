import { waitFor } from "@testing-library/react";
import type { UnknownAction } from "@reduxjs/toolkit";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setUpStore, type RootState } from "./store";
import { chatReducer } from "../features/Chat/Thread/reducer";
import {
  closeThread,
  applyChatEvent,
  createChatWithId,
  newBuddyChatAction,
  newChatAction,
  openBuddyChat,
  setChatModel,
  setMaxNewTokens,
  switchToThread,
} from "../features/Chat/Thread";
import { setCurrentProjectInfo } from "../features/Chat/currentProject";
import type { ChatThreadRuntime } from "../features/Chat/Thread/types";
import {
  getProjectStorageNamespace,
  savePersistedChatTabs,
  savePersistedWorkspace,
  setProjectStorageNamespace,
  setProjectStorageNamespaceFromProjectInfo,
} from "../utils/chatUiPersistence";
import {
  closePane as closeWorkspacePane,
  closeTab as closeWorkspaceTab,
  focusPane as focusWorkspacePane,
  selectFocusedWorkspaceChatId,
  setPaneActive as setWorkspacePaneActive,
  workspaceSlice,
  type WorkspaceState,
} from "../features/Workspace";
import { makeSurfaceKey } from "../features/Workspace/surfaceKey";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";

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

function handoffEvent(
  sourceChatId: string,
  content: Record<string, unknown>,
): ChatEventEnvelope {
  return {
    chat_id: sourceChatId,
    seq: "1",
    type: "message_added",
    index: 1,
    message: {
      role: "tool",
      content: JSON.stringify({ type: "handoff_to_mode", ...content }),
      tool_call_id: "call-handoff",
    },
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

const chatSurface = (id: string) => makeSurfaceKey("chat", id);

const setWorkspaceActionType = "test/setWorkspace";

function installUnsanitizedWorkspaceReducer(
  store: ReturnType<typeof setUpStore>,
): void {
  const initialState = store.getState();
  store.replaceReducer(
    (state: RootState | undefined, action: UnknownAction) => {
      const current = state ?? initialState;
      if (action.type === setWorkspaceActionType) {
        return { ...current, workspace: action.payload as WorkspaceState };
      }

      return {
        ...current,
        chat: chatReducer(current.chat, action),
        workspace: workspaceSlice.reducer(current.workspace, action),
      };
    },
  );
}

function setWorkspace(workspace: WorkspaceState): UnknownAction {
  return { type: setWorkspaceActionType, payload: workspace };
}

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
  sessionStorage.clear();
  setProjectStorageNamespace(undefined);
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

describe("workspace routing middleware", () => {
  it("hydrates workspace only after the project namespace is trusted", async () => {
    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/workspace/project-a"],
      projectName: "project-a",
    });
    const namespace = getProjectStorageNamespace();
    savePersistedChatTabs({
      openThreadIds: ["chat-a", "chat-b"],
      currentThreadId: "chat-a",
      tabs: [{ id: "chat-a" }, { id: "chat-b" }],
    });
    savePersistedWorkspace({
      tabs: [chatSurface("chat-a")],
      activeTabId: chatSurface("chat-a"),
      groups: {
        [chatSurface("chat-a")]: {
          root: {
            kind: "split",
            id: "root:split:row",
            dir: "row",
            sizes: [0.5, 0.5],
            children: [
              {
                kind: "leaf",
                id: "left",
                tabIds: [chatSurface("chat-a")],
                activeTabId: chatSurface("chat-a"),
              },
              {
                kind: "leaf",
                id: "right",
                tabIds: [chatSurface("chat-b")],
                activeTabId: chatSurface("chat-b"),
              },
            ],
          },
          focusedLeafId: "right",
        },
      },
    });
    setProjectStorageNamespace(undefined);
    sessionStorage.setItem(
      "refact:chat-ui:project-storage-namespace:v1",
      namespace ?? "",
    );

    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
    });

    expect(store.getState().workspace.tabs).toEqual([]);

    store.dispatch(
      setCurrentProjectInfo({
        name: "project-a",
        workspaceRoots: ["/workspace/project-a"],
      }),
    );

    await waitFor(() => {
      expect(store.getState().workspace.tabs).toEqual([chatSurface("chat-a")]);
      expect(selectFocusedWorkspaceChatId(store.getState())).toBe("chat-b");
      expect(store.getState().chat.current_thread_id).toBe("chat-b");
    });
  });

  it("reconciles dangling workspace surfaces and syncs current_thread_id", async () => {
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-a", ["chat-a", "chat-b"]),
      workspace: {
        tabs: [chatSurface("chat-a")],
        activeTabId: chatSurface("chat-a"),
        groups: {
          [chatSurface("chat-a")]: {
            root: {
              kind: "split",
              id: "root:split:row",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                {
                  kind: "leaf",
                  id: "left",
                  tabIds: [chatSurface("chat-a")],
                  activeTabId: chatSurface("chat-a"),
                },
                {
                  kind: "leaf",
                  id: "right",
                  tabIds: [chatSurface("chat-b")],
                  activeTabId: chatSurface("chat-b"),
                },
              ],
            },
            focusedLeafId: "right",
          },
        },
      },
    });

    store.dispatch(closeThread({ id: "chat-b" }));

    await waitFor(() => {
      expect(store.getState().workspace.groups).toEqual({});
      expect(store.getState().workspace.tabs).toEqual([chatSurface("chat-a")]);
      expect(store.getState().chat.current_thread_id).toBe("chat-a");
    });
  });

  it("syncs current_thread_id to the focused active workspace pane", async () => {
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-a", ["chat-a", "chat-b"]),
      workspace: {
        tabs: [chatSurface("chat-a")],
        activeTabId: chatSurface("chat-a"),
        groups: {
          [chatSurface("chat-a")]: {
            root: {
              kind: "split",
              id: "root:split:row",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                {
                  kind: "leaf",
                  id: "left",
                  tabIds: [chatSurface("chat-a")],
                  activeTabId: chatSurface("chat-a"),
                },
                {
                  kind: "leaf",
                  id: "right",
                  tabIds: [chatSurface("chat-b")],
                  activeTabId: chatSurface("chat-b"),
                },
              ],
            },
            focusedLeafId: "left",
          },
        },
      },
    });

    store.dispatch(
      setWorkspacePaneActive({
        tabId: chatSurface("chat-a"),
        leafId: "right",
        surfaceKey: chatSurface("chat-b"),
      }),
    );

    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe("chat-b");
    });

    store.dispatch(
      focusWorkspacePane({ tabId: chatSurface("chat-a"), leafId: "left" }),
    );

    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe("chat-a");
    });

    store.dispatch(
      closeWorkspacePane({ tabId: chatSurface("chat-a"), leafId: "left" }),
    );

    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe("chat-b");
    });
  });

  it("creates and selects exactly one workspace tab for visible chat opens", async () => {
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
    });

    store.dispatch(newChatAction({ title: "Visible Chat" }));

    await waitFor(() => {
      const chatId = store.getState().chat.current_thread_id;
      expect(chatId).toBeTruthy();
      expect(store.getState().workspace.tabs).toEqual([chatSurface(chatId)]);
      expect(store.getState().workspace.activeTabId).toBe(chatSurface(chatId));
    });

    const chatId = store.getState().chat.current_thread_id;
    store.dispatch(switchToThread({ id: chatId }));

    await waitFor(() => {
      expect(store.getState().workspace.tabs).toEqual([chatSurface(chatId)]);
      expect(store.getState().workspace.activeTabId).toBe(chatSurface(chatId));
    });
  });

  it("selects an existing workspace tab when switching visible chats", async () => {
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-a", ["chat-a", "chat-b"]),
      workspace: {
        tabs: [chatSurface("chat-a"), chatSurface("chat-b")],
        activeTabId: chatSurface("chat-a"),
        groups: {},
      },
    });

    store.dispatch(switchToThread({ id: "chat-b" }));

    await waitFor(() => {
      expect(store.getState().workspace.tabs).toEqual([
        chatSurface("chat-a"),
        chatSurface("chat-b"),
      ]);
      expect(store.getState().workspace.activeTabId).toBe(
        chatSurface("chat-b"),
      );
      expect(store.getState().chat.current_thread_id).toBe("chat-b");
    });
  });

  it("keeps task-internal openTab false switches out of workspace tabs", async () => {
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-a", ["chat-a"]),
      workspace: {
        tabs: [chatSurface("chat-a")],
        activeTabId: chatSurface("chat-a"),
        groups: {},
      },
    });

    store.dispatch(
      createChatWithId({
        id: "task-hidden",
        title: "Task Hidden",
        openTab: false,
      }),
    );
    store.dispatch(switchToThread({ id: "task-hidden", openTab: false }));

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(store.getState().chat.current_thread_id).toBe("task-hidden");
    expect(store.getState().workspace.tabs).toEqual([chatSurface("chat-a")]);
    expect(store.getState().workspace.activeTabId).toBe(chatSurface("chat-a"));
  });

  it("closing a workspace chat tab closes the matching thread", async () => {
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-a", ["chat-a", "chat-b"]),
      workspace: {
        tabs: [chatSurface("chat-a"), chatSurface("chat-b")],
        activeTabId: chatSurface("chat-b"),
        groups: {},
      },
    });

    store.dispatch(closeWorkspaceTab(chatSurface("chat-b")));

    await waitFor(() => {
      expect(store.getState().workspace.tabs).toEqual([chatSurface("chat-a")]);
      expect(store.getState().chat.open_thread_ids).toEqual(["chat-a"]);
      expect(store.getState().chat.threads["chat-b"]).toBeUndefined();
      expect(store.getState().chat.current_thread_id).toBe("chat-a");
    });
  });

  it("closing a tab preserves grouped chats that survive elsewhere", async () => {
    const chatA = chatSurface("chat-a");
    const chatB = chatSurface("chat-b");
    const chatC = chatSurface("chat-c");
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-a", ["chat-a", "chat-b", "chat-c"]),
      workspace: {
        tabs: [chatA, chatB, chatC],
        activeTabId: chatA,
        groups: {
          [chatA]: {
            root: {
              kind: "split",
              id: "root:split:row",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                {
                  kind: "leaf",
                  id: "left",
                  tabIds: [chatA],
                  activeTabId: chatA,
                },
                {
                  kind: "leaf",
                  id: "right",
                  tabIds: [chatB],
                  activeTabId: chatB,
                },
              ],
            },
            focusedLeafId: "right",
          },
        },
      },
    });

    store.dispatch(closeWorkspaceTab(chatA));

    await waitFor(() => {
      expect(store.getState().workspace.tabs).toEqual([chatB, chatC]);
      expect(store.getState().chat.open_thread_ids).toEqual([
        "chat-b",
        "chat-c",
      ]);
      expect(store.getState().chat.threads["chat-a"]).toBeUndefined();
      expect(store.getState().chat.threads["chat-b"]).toBeDefined();
    });
  });

  it("closing a grouped workspace chat tab closes all grouped threads without ghosts", async () => {
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-b", ["chat-a", "chat-b", "chat-c"]),
      workspace: {
        tabs: [chatSurface("chat-a"), chatSurface("chat-c")],
        activeTabId: chatSurface("chat-a"),
        groups: {
          [chatSurface("chat-a")]: {
            root: {
              kind: "split",
              id: "root:split:row",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                {
                  kind: "leaf",
                  id: "left",
                  tabIds: [chatSurface("chat-a")],
                  activeTabId: chatSurface("chat-a"),
                },
                {
                  kind: "leaf",
                  id: "right",
                  tabIds: [chatSurface("chat-b")],
                  activeTabId: chatSurface("chat-b"),
                },
              ],
            },
            focusedLeafId: "right",
          },
        },
      },
    });

    store.dispatch(closeWorkspaceTab(chatSurface("chat-a")));

    await waitFor(() => {
      expect(store.getState().workspace).toEqual({
        tabs: [chatSurface("chat-c")],
        activeTabId: chatSurface("chat-c"),
        groups: {},
      });
      expect(store.getState().chat.open_thread_ids).toEqual(["chat-c"]);
      expect(store.getState().chat.threads["chat-a"]).toBeUndefined();
      expect(store.getState().chat.threads["chat-b"]).toBeUndefined();
      expect(store.getState().chat.current_thread_id).toBe("chat-c");
    });
  });

  it("closing a duplicate pane preserves the surviving duplicate chat", async () => {
    const chatA = chatSurface("chat-a");
    const chatB = chatSurface("chat-b");
    const chatC = chatSurface("chat-c");
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-a", ["chat-a", "chat-b", "chat-c"]),
    });
    installUnsanitizedWorkspaceReducer(store);
    store.dispatch(
      setWorkspace({
        tabs: [chatA, chatC],
        activeTabId: chatA,
        groups: {
          [chatA]: {
            root: {
              kind: "split",
              id: "root:split:row",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                {
                  kind: "leaf",
                  id: "left",
                  tabIds: [chatB],
                  activeTabId: chatB,
                },
                {
                  kind: "leaf",
                  id: "right",
                  tabIds: [chatA, chatB],
                  activeTabId: chatB,
                },
              ],
            },
            focusedLeafId: "right",
          },
        },
      }),
    );

    store.dispatch(closeWorkspacePane({ tabId: chatA, leafId: "left" }));

    await waitFor(() => {
      expect(store.getState().workspace.tabs).toEqual([chatA, chatB, chatC]);
      expect(store.getState().chat.open_thread_ids).toEqual([
        "chat-a",
        "chat-b",
        "chat-c",
      ]);
      expect(store.getState().chat.threads["chat-b"]).toBeDefined();
    });
  });

  it("closing a normal pane closes only chats removed from workspace", async () => {
    const chatA = chatSurface("chat-a");
    const chatB = chatSurface("chat-b");
    const chatC = chatSurface("chat-c");
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
      chat: makeChatState("chat-b", ["chat-a", "chat-b", "chat-c"]),
      workspace: {
        tabs: [chatA, chatC],
        activeTabId: chatA,
        groups: {
          [chatA]: {
            root: {
              kind: "split",
              id: "root:split:row",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                {
                  kind: "leaf",
                  id: "left",
                  tabIds: [chatA],
                  activeTabId: chatA,
                },
                {
                  kind: "leaf",
                  id: "right",
                  tabIds: [chatB],
                  activeTabId: chatB,
                },
              ],
            },
            focusedLeafId: "left",
          },
        },
      },
    });

    store.dispatch(closeWorkspacePane({ tabId: chatA, leafId: "right" }));

    await waitFor(() => {
      expect(store.getState().workspace.tabs).toEqual([chatA, chatC]);
      expect(store.getState().chat.open_thread_ids).toEqual([
        "chat-a",
        "chat-c",
      ]);
      expect(store.getState().chat.threads["chat-b"]).toBeUndefined();
      expect(store.getState().chat.threads["chat-a"]).toBeDefined();
      expect(store.getState().chat.threads["chat-c"]).toBeDefined();
    });
  });

  it("opens buddy chats as exactly one workspace tab", async () => {
    const store = setUpStore({
      config: { host: "web", lspPort: 8001, themeProps: {} },
    });

    store.dispatch(newBuddyChatAction({ chat_id: "buddy-chat" }));
    store.dispatch(
      openBuddyChat({ chat_id: "buddy-chat", title: "Buddy Chat" }),
    );

    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe("buddy-chat");
      expect(store.getState().workspace.tabs).toEqual([
        chatSurface("buddy-chat"),
      ]);
      expect(store.getState().workspace.activeTabId).toBe(
        chatSurface("buddy-chat"),
      );
    });
  });
});

describe("handoff_to_mode middleware", () => {
  it("routes normal chat to returned task planner metadata", async () => {
    const sourceChatId = "chat-source";
    const newChatId = "planner-chat";
    const taskId = "task-1";
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(new Response(null, { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);

    const store = setUpStore({
      config: {
        host: "web",
        engineServed: true,
        lspPort: 8001,
        themeProps: {},
      },
      pages: [{ name: "history" }, { name: "chat" }],
      chat: {
        current_thread_id: sourceChatId,
        open_thread_ids: [sourceChatId],
        threads: { [sourceChatId]: makeThread(sourceChatId) },
        system_prompt: {},
        tool_use: "agent" as const,
        sse_refresh_requested: null,
        stream_version: 0,
      },
    });

    store.dispatch(
      applyChatEvent(
        handoffEvent(sourceChatId, {
          new_chat_id: newChatId,
          target_mode: "task_planner",
          task_meta: {
            task_id: taskId,
            role: "planner",
            planner_chat_id: newChatId,
          },
          parent_id: sourceChatId,
          link_type: "handoff",
          root_chat_id: newChatId,
        }),
      ),
    );

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));

    const state = store.getState();
    const plannerRuntime = state.chat.threads[newChatId];
    expect(plannerRuntime?.thread.mode).toBe("task_planner");
    expect(plannerRuntime?.thread.is_task_chat).toBe(true);
    expect(plannerRuntime?.thread.task_meta).toEqual({
      task_id: taskId,
      role: "planner",
      planner_chat_id: newChatId,
    });
    expect(plannerRuntime?.thread.parent_id).toBe(sourceChatId);
    expect(plannerRuntime?.thread.link_type).toBe("handoff");
    expect(plannerRuntime?.thread.root_chat_id).toBe(newChatId);
    expect(state.chat.current_thread_id).toBe(newChatId);
    expect(state.chat.sse_refresh_requested).toBe(newChatId);
    expect(state.tasksUI.openTasks).toEqual([
      {
        id: taskId,
        name: "Task",
        plannerChats: [
          {
            id: newChatId,
            title: "",
            createdAt: expect.any(String) as unknown as string,
            updatedAt: expect.any(String) as unknown as string,
            mode: "task_planner",
          },
        ],
        activeChat: { type: "planner", chatId: newChatId },
      },
    ]);
    expect(state.pages.at(-1)).toEqual({ name: "task workspace", taskId });
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
