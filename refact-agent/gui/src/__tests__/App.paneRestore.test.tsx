import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../app/store";
import { InnerApp } from "../features/App";
import { setBackendStatus } from "../features/Connection";
import type { PaneNode } from "../features/ChatPanes";
import { selectFocusedWorkspaceChatId } from "../features/Workspace";
import { makeSurfaceKey } from "../features/Workspace/surfaceKey";
import {
  getProjectStorageNamespace,
  savePersistedActiveTab,
  savePersistedChatTabs,
  savePersistedTasksUIState,
  savePersistedWorkspace,
  setProjectStorageNamespace,
  setProjectStorageNamespaceFromProjectInfo,
} from "../utils/chatUiPersistence";
import { render, waitFor } from "../utils/test-utils";
import {
  chatLinks,
  chatSessionAbort,
  chatSessionCommand,
  chatSessionSubscribe,
  emptyTasks,
  goodCaps,
  goodPing,
  goodPrompts,
  goodTools,
  goodUser,
  noCommandPreview,
  noCompletions,
  server,
  sidebarSubscribe,
} from "../utils/mockServer";

vi.mock("../features/Tasks", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  const actual =
    await vi.importActual<typeof import("../features/Tasks")>(
      "../features/Tasks",
    );

  return {
    ...actual,
    TaskWorkspace: ({ taskId }: { taskId: string }) =>
      React.createElement(
        "section",
        { "data-testid": "task-workspace" },
        taskId,
      ),
  };
});

vi.mock("../features/Buddy/BuddyHome", async () => {
  const React = await vi.importActual<typeof import("react")>("react");

  return {
    BuddyHome: () =>
      React.createElement("section", { "data-testid": "buddy-home" }, "Buddy"),
  };
});

const appHandlers = [
  goodPing,
  goodUser,
  goodCaps,
  goodTools,
  goodPrompts,
  chatLinks,
  chatSessionSubscribe,
  chatSessionCommand,
  chatSessionAbort,
  emptyTasks,
  noCommandPreview,
  noCompletions,
  http.get("*/v1/chat-modes", () =>
    HttpResponse.json({ modes: [], errors: [] }),
  ),
  http.get("*/v1/setup/status", () =>
    HttpResponse.json({
      configured: true,
      reasons: [],
      detail: {
        project_root: "/tmp/refact-test",
        has_agents_md: true,
        has_knowledge: true,
        has_trajectories: true,
      },
    }),
  ),
  http.get("*/v1/voice/status", () => HttpResponse.json({ available: false })),
  http.get("*/v1/chats/:chatId/skills-status", () =>
    HttpResponse.json({
      skills_available: 0,
      skills_included: [],
      skills_enabled: false,
      active_skill: null,
    }),
  ),
  http.get("*/v1/buddy/opportunities", () =>
    HttpResponse.json({ opportunities: [] }),
  ),
  http.get("*/v1/worktrees", () =>
    HttpResponse.json({
      project_hash: "test",
      source_workspace_root: "/tmp/refact-test",
      worktrees: [],
    }),
  ),
];

const baseConfig = {
  host: "vscode" as const,
  lspPort: 8001,
  apiKey: "test",
  themeProps: {},
};

function createSidebarSnapshotHandler(
  tasks: { id: string; name: string }[] = [],
) {
  return http.get("*/v1/sidebar/subscribe", () => {
    const encoder = new TextEncoder();
    const stream = new ReadableStream({
      start(controller) {
        const events = [
          {
            protocol_version: 2,
            seq: 0,
            subscription_id: "test-sidebar",
            event: {
              type: "section_snapshot",
              section: "workspace",
              status: "ready",
              snapshot: { workspace_roots: ["/tmp/refact-test"] },
            },
          },
          {
            protocol_version: 2,
            seq: 1,
            subscription_id: "test-sidebar",
            event: {
              type: "section_snapshot",
              section: "chats",
              status: "ready",
              snapshot: { trajectories: [] },
            },
          },
          {
            protocol_version: 2,
            seq: 2,
            subscription_id: "test-sidebar",
            event: {
              type: "section_snapshot",
              section: "tasks",
              status: "ready",
              snapshot: { tasks },
            },
          },
          {
            protocol_version: 2,
            seq: 3,
            subscription_id: "test-sidebar",
            event: {
              type: "section_snapshot",
              section: "buddy",
              status: "ready",
              snapshot: { buddy: null },
            },
          },
        ];
        for (const event of events) {
          controller.enqueue(
            encoder.encode(`data: ${JSON.stringify(event)}\n\n`),
          );
        }
      },
    });

    return new HttpResponse(stream, {
      headers: {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      },
    });
  });
}

const renderApp = () => {
  const store = setUpStore({
    config: baseConfig,
    pages: [{ name: "history" }],
  });
  store.dispatch(setBackendStatus({ status: "online" }));

  const view = render(<InnerApp />, { store });

  return { ...view, store };
};

function twoPaneRoot(): PaneNode {
  const chatA = makeSurfaceKey("chat", "chat-a");
  const chatB = makeSurfaceKey("chat", "chat-b");

  return {
    kind: "split",
    id: "root:split:row",
    dir: "row",
    sizes: [0.35, 0.65],
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
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
  sessionStorage.clear();
  setProjectStorageNamespace(undefined);
});

describe("App workspace restore", () => {
  it("restores persisted workspace structure and reconciles current thread to the focused pane", async () => {
    server.use(...appHandlers, sidebarSubscribe);
    const chatA = makeSurfaceKey("chat", "chat-a");
    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/tmp/refact-test"],
      projectName: "refact-test",
    });
    const namespace = getProjectStorageNamespace();
    savePersistedChatTabs({
      openThreadIds: ["chat-a", "chat-b"],
      currentThreadId: "chat-a",
      tabs: [
        { id: "chat-a", title: "Chat A", mode: "agent" },
        { id: "chat-b", title: "Chat B", mode: "agent" },
      ],
    });
    savePersistedWorkspace({
      tabs: [chatA],
      activeTabId: chatA,
      groups: {
        [chatA]: { root: twoPaneRoot(), focusedLeafId: "right" },
      },
    });
    savePersistedActiveTab({ type: "chat", id: "chat-a" });
    setProjectStorageNamespace(undefined);
    sessionStorage.setItem(
      "refact:chat-ui:project-storage-namespace:v1",
      namespace ?? "",
    );

    const { store } = renderApp();

    await waitFor(() => {
      expect(selectFocusedWorkspaceChatId(store.getState())).toBe("chat-b");
      expect(store.getState().workspace.groups[chatA]?.focusedLeafId).toBe(
        "right",
      );
      expect(store.getState().workspace.groups[chatA]?.root).toEqual(
        twoPaneRoot(),
      );
      expect(store.getState().chat.current_thread_id).toBe("chat-b");
      expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
    });

    expect(store.getState().workspace.tabs).toEqual([chatA]);
  });

  it("prunes dangling workspace chats and degrades missing layouts to a single tab fallback", async () => {
    server.use(...appHandlers, createSidebarSnapshotHandler());
    const chatA = makeSurfaceKey("chat", "chat-a");
    const missingLeft = makeSurfaceKey("chat", "missing-left");
    const missingRight = makeSurfaceKey("chat", "missing-right");
    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/tmp/refact-test"],
      projectName: "refact-test",
    });
    const namespace = getProjectStorageNamespace();
    savePersistedChatTabs({
      openThreadIds: ["chat-a"],
      currentThreadId: "chat-a",
      tabs: [{ id: "chat-a", title: "Chat A", mode: "agent" }],
    });
    savePersistedWorkspace({
      tabs: [chatA, missingLeft],
      activeTabId: missingLeft,
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
                tabIds: [missingLeft],
                activeTabId: missingLeft,
              },
              {
                kind: "leaf",
                id: "right",
                tabIds: [missingRight],
                activeTabId: missingRight,
              },
            ],
          },
          focusedLeafId: "right",
        },
      },
    });
    savePersistedActiveTab({ type: "chat", id: "chat-a" });
    setProjectStorageNamespace(undefined);
    sessionStorage.setItem(
      "refact:chat-ui:project-storage-namespace:v1",
      namespace ?? "",
    );

    const { store } = renderApp();

    await waitFor(() => {
      expect(store.getState().chat.open_thread_ids).toEqual(["chat-a"]);
      expect(store.getState().workspace).toEqual({
        tabs: [chatA],
        activeTabId: chatA,
        groups: {},
      });
      expect(store.getState().chat.current_thread_id).toBe("chat-a");
    });
  });

  it("restores task nav tabs from tasks UI without adding pane surfaces", async () => {
    server.use(
      ...appHandlers,
      createSidebarSnapshotHandler([{ id: "task-a", name: "Task Alpha" }]),
    );
    const chatA = makeSurfaceKey("chat", "chat-a");
    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/tmp/refact-test"],
      projectName: "refact-test",
    });
    const namespace = getProjectStorageNamespace();
    savePersistedChatTabs({
      openThreadIds: ["chat-a"],
      currentThreadId: "chat-a",
      tabs: [{ id: "chat-a", title: "Chat A", mode: "agent" }],
    });
    savePersistedWorkspace({
      tabs: [chatA],
      activeTabId: chatA,
      groups: {},
    });
    savePersistedTasksUIState({
      openTasks: [
        {
          id: "task-a",
          name: "Task Alpha",
          plannerChats: [],
          activeChat: null,
        },
      ],
    });
    savePersistedActiveTab({ type: "task", taskId: "task-a" });
    setProjectStorageNamespace(undefined);
    sessionStorage.setItem(
      "refact:chat-ui:project-storage-namespace:v1",
      namespace ?? "",
    );

    const { store } = renderApp();

    await waitFor(() => {
      expect(store.getState().workspace).toEqual({
        tabs: [chatA],
        activeTabId: chatA,
        groups: {},
      });
      expect(store.getState().tasksUI.openTasks).toEqual([
        {
          id: "task-a",
          name: "Task Alpha",
          plannerChats: [],
          activeChat: null,
        },
      ]);
      expect(store.getState().pages.at(-1)).toEqual({
        name: "task workspace",
        taskId: "task-a",
      });
    });
  });

  it("falls back when the persisted active chat or task no longer resolves", async () => {
    server.use(...appHandlers, createSidebarSnapshotHandler());
    const chatA = makeSurfaceKey("chat", "chat-a");
    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/tmp/refact-test"],
      projectName: "refact-test",
    });
    const namespace = getProjectStorageNamespace();
    savePersistedChatTabs({
      openThreadIds: ["chat-a"],
      currentThreadId: "chat-a",
      tabs: [{ id: "chat-a", title: "Chat A", mode: "agent" }],
    });
    savePersistedWorkspace({
      tabs: [chatA],
      activeTabId: chatA,
      groups: {},
    });
    savePersistedActiveTab({ type: "chat", id: "missing-chat" });
    setProjectStorageNamespace(undefined);
    sessionStorage.setItem(
      "refact:chat-ui:project-storage-namespace:v1",
      namespace ?? "",
    );

    const { store, unmount } = renderApp();

    await waitFor(() => {
      expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
      expect(store.getState().chat.current_thread_id).toBe("chat-a");
      expect(store.getState().workspace.tabs).toEqual([chatA]);
    });

    unmount();
    localStorage.clear();
    sessionStorage.clear();
    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/tmp/refact-test"],
      projectName: "refact-test",
    });
    const nextNamespace = getProjectStorageNamespace();
    savePersistedChatTabs({
      openThreadIds: ["chat-a"],
      currentThreadId: "chat-a",
      tabs: [{ id: "chat-a", title: "Chat A", mode: "agent" }],
    });
    savePersistedWorkspace({
      tabs: [chatA],
      activeTabId: chatA,
      groups: {},
    });
    savePersistedActiveTab({ type: "task", taskId: "missing-task" });
    setProjectStorageNamespace(undefined);
    sessionStorage.setItem(
      "refact:chat-ui:project-storage-namespace:v1",
      nextNamespace ?? "",
    );

    const { store: nextStore } = renderApp();

    await waitFor(() => {
      expect(nextStore.getState().pages.at(-1)).toEqual({ name: "history" });
      expect(nextStore.getState().workspace).toEqual({
        tabs: [chatA],
        activeTabId: chatA,
        groups: {},
      });
    });
  });
});
