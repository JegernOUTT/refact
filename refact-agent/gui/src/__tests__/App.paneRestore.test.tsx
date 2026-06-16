import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../app/store";
import { InnerApp } from "../features/App";
import { setBackendStatus } from "../features/Connection";
import { findLeaf, type PaneNode } from "../features/ChatPanes";
import {
  getProjectStorageNamespace,
  savePersistedActiveTab,
  savePersistedChatTabs,
  savePersistedPaneLayout,
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
  server,
  sidebarSubscribe,
} from "../utils/mockServer";

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
  return {
    kind: "split",
    id: "root:split:row",
    dir: "row",
    sizes: [0.35, 0.65],
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
  localStorage.clear();
  sessionStorage.clear();
  setProjectStorageNamespace(undefined);
});

describe("App pane restore", () => {
  it("restores persisted pane structure and reconciles current thread to the focused pane", async () => {
    server.use(...appHandlers, sidebarSubscribe);
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
    savePersistedPaneLayout({ root: twoPaneRoot(), focusedLeafId: "right" });
    savePersistedActiveTab({ type: "chat", id: "chat-a" });
    setProjectStorageNamespace(undefined);
    sessionStorage.setItem(
      "refact:chat-ui:project-storage-namespace:v1",
      namespace ?? "",
    );

    const { store } = renderApp();

    await waitFor(() => {
      expect(store.getState().panes.focusedLeafId).toBe("right");
      expect(findLeaf(store.getState().panes.root, "right")?.activeTabId).toBe(
        "chat-b",
      );
      expect(store.getState().chat.current_thread_id).toBe("chat-b");
      expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
    });

    const root = store.getState().panes.root;
    expect(root).toEqual(twoPaneRoot());
  });

  it("prunes dangling pane tabs and degrades missing layouts to a single leaf fallback", async () => {
    server.use(
      ...appHandlers,
      http.get("*/v1/sidebar/subscribe", () => {
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
      }),
    );
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
    savePersistedPaneLayout({
      root: {
        kind: "split",
        id: "root:split:row",
        dir: "row",
        sizes: [0.5, 0.5],
        children: [
          {
            kind: "leaf",
            id: "left",
            tabIds: ["missing-left"],
            activeTabId: "missing-left",
          },
          {
            kind: "leaf",
            id: "right",
            tabIds: ["missing-right"],
            activeTabId: "missing-right",
          },
        ],
      },
      focusedLeafId: "right",
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
      expect(store.getState().panes.root).toEqual({
        kind: "leaf",
        id: "root",
        tabIds: ["chat-a"],
        activeTabId: "chat-a",
      });
      expect(store.getState().panes.focusedLeafId).toBe("root");
      expect(store.getState().chat.current_thread_id).toBe("chat-a");
    });
  });
});
