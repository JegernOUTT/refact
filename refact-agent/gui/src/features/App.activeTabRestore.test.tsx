import { http, HttpResponse } from "msw";
import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../app/store";
import { setCurrentProjectInfo } from "./Chat/currentProject";
import { setBackendStatus } from "./Connection";
import { InnerApp } from "./App";
import { makeSurfaceKey } from "./Workspace/surfaceKey";
import {
  getProjectStorageNamespace,
  savePersistedActiveTab,
  savePersistedChatTabs,
  savePersistedTasksUIState,
  savePersistedWorkspace,
  setProjectStorageNamespace,
  setProjectStorageNamespaceFromProjectInfo,
} from "../utils/chatUiPersistence";
import { render, screen, waitFor } from "../utils/test-utils";
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
} from "../utils/mockServer";

vi.mock("./Tasks", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  const actual = await vi.importActual<typeof import("./Tasks")>("./Tasks");

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

vi.mock("./Buddy/BuddyHome", async () => {
  const React = await vi.importActual<typeof import("react")>("react");

  return {
    BuddyHome: () =>
      React.createElement("section", { "data-testid": "buddy-home" }, "Buddy"),
  };
});

const PROJECT_STORAGE_NAMESPACE_SESSION_KEY =
  "refact:chat-ui:project-storage-namespace:v1";

function activeTabStorageKey(namespace: string) {
  return `refact:project:${namespace}:refact:chat-ui:active-tab:v1`;
}

function writePersistedActiveTab(
  namespace: string,
  activeTab: Parameters<typeof savePersistedActiveTab>[0],
) {
  localStorage.setItem(
    activeTabStorageKey(namespace),
    JSON.stringify({ version: 1, activeTab, updatedAt: Date.now() }),
  );
}

const baseConfig = {
  host: "vscode" as const,
  lspPort: 8001,
  apiKey: "test",
  themeProps: {},
};

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
        project_root: "/tmp/project-a",
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
      source_workspace_root: "/tmp/project-a",
      worktrees: [],
    }),
  ),
];

function createSidebarSnapshotHandler(workspaceRoots: string[]) {
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
              snapshot: { workspace_roots: workspaceRoots },
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
              snapshot: { tasks: [] },
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

function namespaceFor(projectName: string, workspaceRoot: string) {
  setProjectStorageNamespaceFromProjectInfo({
    workspaceRoots: [workspaceRoot],
    projectName,
  });
  const namespace = getProjectStorageNamespace();
  if (!namespace) throw new Error(`missing namespace for ${projectName}`);
  return namespace;
}

function saveProjectAState() {
  const chatA = makeSurfaceKey("chat", "chat-a");
  const namespace = namespaceFor("project-a", "/tmp/project-a");

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
  savePersistedActiveTab({ type: "chat", id: "chat-a" });

  return namespace;
}

function saveProjectBState() {
  const chatB = makeSurfaceKey("chat", "chat-b");
  const namespace = namespaceFor("project-b", "/tmp/project-b");

  savePersistedChatTabs({
    openThreadIds: ["chat-b"],
    currentThreadId: "chat-b",
    tabs: [{ id: "chat-b", title: "Chat B", mode: "agent" }],
  });
  savePersistedWorkspace({
    tabs: [chatB],
    activeTabId: chatB,
    groups: {},
  });
  savePersistedTasksUIState({
    openTasks: [
      {
        id: "task-b",
        name: "Task B",
        plannerChats: [],
        activeChat: null,
      },
    ],
  });
  savePersistedActiveTab({ type: "task", taskId: "task-b" });

  return namespace;
}

function renderApp() {
  const store = setUpStore({
    config: baseConfig,
    pages: [{ name: "history" }],
  });
  store.dispatch(setBackendStatus({ status: "online" }));

  const view = render(<InnerApp />, { store });

  return { ...view, store };
}

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
  sessionStorage.clear();
  setProjectStorageNamespace(undefined);
});

describe("App active tab restore", () => {
  it("resets active-tab restore refs when the project namespace changes", async () => {
    server.use(
      ...appHandlers,
      createSidebarSnapshotHandler(["/tmp/project-a"]),
    );
    const namespaceA = saveProjectAState();
    saveProjectBState();
    setProjectStorageNamespace(undefined);
    sessionStorage.setItem(PROJECT_STORAGE_NAMESPACE_SESSION_KEY, namespaceA);

    const { store } = renderApp();

    await waitFor(() => {
      expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
      expect(store.getState().chat.current_thread_id).toBe("chat-a");
    });

    act(() => {
      store.dispatch(
        setCurrentProjectInfo({
          name: "project-b",
          workspaceRoots: ["/tmp/project-b"],
        }),
      );
    });

    await waitFor(() => {
      expect(store.getState().pages.at(-1)).toEqual({
        name: "task workspace",
        taskId: "task-b",
      });
    });
    expect(await screen.findByTestId("task-workspace")).toHaveTextContent(
      "task-b",
    );
    expect(getProjectStorageNamespace()).not.toBe(namespaceA);
  });

  it("resets active-tab restore refs after untrusted transition to a different project", async () => {
    server.use(
      ...appHandlers,
      createSidebarSnapshotHandler(["/tmp/project-a"]),
    );
    const namespaceA = saveProjectAState();
    const namespaceB = saveProjectBState();
    setProjectStorageNamespace(undefined);
    sessionStorage.setItem(PROJECT_STORAGE_NAMESPACE_SESSION_KEY, namespaceA);

    const { store } = renderApp();

    await waitFor(() => {
      expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
      expect(store.getState().chat.current_thread_id).toBe("chat-a");
    });

    act(() => {
      store.dispatch(setCurrentProjectInfo({ name: "", workspaceRoots: [] }));
    });

    await waitFor(() => {
      expect(store.getState().chat.open_thread_ids).toEqual([]);
    });

    act(() => {
      store.dispatch(
        setCurrentProjectInfo({
          name: "project-b",
          workspaceRoots: ["/tmp/project-b"],
        }),
      );
    });

    await waitFor(() => {
      expect(store.getState().pages.at(-1)).toEqual({
        name: "task workspace",
        taskId: "task-b",
      });
    });
    expect(await screen.findByTestId("task-workspace")).toHaveTextContent(
      "task-b",
    );
    expect(getProjectStorageNamespace()).toBe(namespaceB);
  });

  it("preserves active-tab restore refs after untrusted transition to the same project", async () => {
    server.use(
      ...appHandlers,
      createSidebarSnapshotHandler(["/tmp/project-a"]),
    );
    const namespaceA = saveProjectAState();
    setProjectStorageNamespace(undefined);
    sessionStorage.setItem(PROJECT_STORAGE_NAMESPACE_SESSION_KEY, namespaceA);

    const { store } = renderApp();

    await waitFor(() => {
      expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
      expect(store.getState().chat.current_thread_id).toBe("chat-a");
    });

    writePersistedActiveTab(namespaceA, { type: "buddy" });

    act(() => {
      store.dispatch(setCurrentProjectInfo({ name: "", workspaceRoots: [] }));
    });

    await waitFor(() => {
      expect(store.getState().chat.open_thread_ids).toEqual([]);
    });

    act(() => {
      store.dispatch(
        setCurrentProjectInfo({
          name: "project-a",
          workspaceRoots: ["/tmp/project-a"],
        }),
      );
    });

    await waitFor(() => {
      expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
      expect(store.getState().chat.current_thread_id).toBe("chat-a");
    });
    expect(screen.queryByTestId("buddy-home")).toBeNull();
    expect(getProjectStorageNamespace()).toBe(namespaceA);
  });
});
