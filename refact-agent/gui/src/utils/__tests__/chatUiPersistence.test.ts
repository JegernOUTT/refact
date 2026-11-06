import { beforeEach, describe, expect, it } from "vitest";
import {
  clearAskQuestionsDraft,
  loadAskQuestionsDraft,
  loadPersistedActiveTab,
  loadPersistedChatTabs,
  loadPersistedWorkspace,
  loadPersistedTasksUIState,
  loadTaskWorkspaceTab,
  saveAskQuestionsDraft,
  savePersistedActiveTab,
  savePersistedChatTabs,
  savePersistedWorkspace,
  savePersistedTasksUIState,
  saveTaskWorkspaceTab,
} from "../chatUiPersistence";
import {
  getProjectStorageNamespace,
  setProjectStorageNamespace,
  setProjectStorageNamespaceFromProjectInfo,
} from "../chatUiPersistence";
import { makeSurfaceKey } from "../../features/Workspace/surfaceKey";
import { collectTabIds } from "../../features/ChatPanes/panesTree";
import type { WorkspaceState } from "../../features/Workspace/workspaceSlice";

const LEGACY_PANE_LAYOUT_STORAGE_KEY = "refact:chat-ui:panes:v1";
const WORKSPACE_STORAGE_KEY = "refact:chat-ui:workspace:v1";

function workspaceStorageKey(): string {
  return `refact:project:${getProjectStorageNamespace()}:${WORKSPACE_STORAGE_KEY}`;
}

const chatSurface = (id: string) => makeSurfaceKey("chat", id);

function splitWorkspace(): WorkspaceState {
  const chatA = chatSurface("chat-a");
  const chatB = chatSurface("chat-b");
  return {
    tabs: [chatA],
    activeTabId: chatA,
    groups: {
      [chatA]: {
        root: {
          kind: "split",
          id: "root:split:row",
          dir: "row",
          sizes: [0.7, 0.3],
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
    dock: { open: false, width: 340, section: "git" },
    drawer: { open: true, height: 320 },
  };
}

describe("chatUiPersistence", () => {
  it("restores ordered local file tabs and selection in a single pane", () => {
    const first = makeSurfaceKey("file", "/workspace/first.txt");
    const second = makeSurfaceKey("file", "/outside/second.html");
    const workspace: WorkspaceState = {
      tabs: [first],
      activeTabId: first,
      groups: {
        [first]: {
          root: {
            kind: "leaf",
            id: "local-files",
            tabIds: [second, first],
            activeTabId: second,
          },
          focusedLeafId: "local-files",
        },
      },
    };
    savePersistedWorkspace(workspace);

    const restored = loadPersistedWorkspace();

    expect(restored.groups[first]).toEqual(workspace.groups[first]);
    expect(restored.tabs).toEqual([first]);
    expect(restored.activeTabId).toBe(first);
  });

  beforeEach(() => {
    localStorage.clear();
    sessionStorage.clear();
    setProjectStorageNamespace("/workspace/default");
  });

  it("scopes chat UI state by project namespace", () => {
    setProjectStorageNamespace("/workspace/project-a");
    savePersistedChatTabs({
      openThreadIds: ["chat-a"],
      currentThreadId: "chat-a",
      tabs: [{ id: "chat-a", title: "Project A" }],
    });
    savePersistedActiveTab({ type: "chat", id: "chat-a" });

    setProjectStorageNamespace("/workspace/project-b");
    savePersistedChatTabs({
      openThreadIds: ["chat-b"],
      currentThreadId: "chat-b",
      tabs: [{ id: "chat-b", title: "Project B" }],
    });
    savePersistedActiveTab({ type: "chat", id: "chat-b" });

    expect(loadPersistedChatTabs().openThreadIds).toEqual(["chat-b"]);
    expect(loadPersistedActiveTab()).toEqual({ type: "chat", id: "chat-b" });

    setProjectStorageNamespace("/workspace/project-a");
    expect(loadPersistedChatTabs().openThreadIds).toEqual(["chat-a"]);
    expect(loadPersistedActiveTab()).toEqual({ type: "chat", id: "chat-a" });

    setProjectStorageNamespace(undefined);
  });

  it("does not hydrate chat tabs from a stale session namespace before project verification", () => {
    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/workspace/project-a"],
      projectName: "project-a",
      workspaceName: "fallback-a",
    });
    const projectANamespace = getProjectStorageNamespace();
    savePersistedChatTabs({
      openThreadIds: ["chat-a"],
      currentThreadId: "chat-a",
      tabs: [{ id: "chat-a", title: "Project A" }],
    });

    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/workspace/project-b"],
      projectName: "project-b",
      workspaceName: "fallback-b",
    });
    savePersistedChatTabs({
      openThreadIds: ["chat-b"],
      currentThreadId: "chat-b",
      tabs: [{ id: "chat-b", title: "Project B" }],
    });

    setProjectStorageNamespace(undefined);
    sessionStorage.setItem(
      "refact:chat-ui:project-storage-namespace:v1",
      projectANamespace ?? "",
    );

    expect(getProjectStorageNamespace()).toBe(projectANamespace);
    expect(loadPersistedChatTabs().openThreadIds).toEqual([]);

    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/workspace/project-a"],
      projectName: "project-a",
      workspaceName: "fallback-a",
    });
    expect(loadPersistedChatTabs().openThreadIds).toEqual(["chat-a"]);
  });

  it("persists opened chat tabs and the latest active chat", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a", "chat-b", "chat-a"],
      currentThreadId: "chat-b",
      tabs: [
        {
          id: "chat-a",
          title: "Research",
          mode: "EXPLORE",
          tool_use: "explore",
          session_state: "completed",
        },
        {
          id: "chat-b",
          title: "Implementation",
          mode: "agent",
          tool_use: "agent",
          session_state: "generating",
        },
      ],
    });

    expect(loadPersistedChatTabs()).toEqual({
      openThreadIds: ["chat-a", "chat-b"],
      currentThreadId: "chat-b",
      tabs: [
        {
          id: "chat-a",
          title: "Research",
          mode: "EXPLORE",
          tool_use: "explore",
          session_state: "completed",
          is_buddy_chat: undefined,
        },
        {
          id: "chat-b",
          title: "Implementation",
          mode: "agent",
          tool_use: "agent",
          session_state: "generating",
          is_buddy_chat: undefined,
        },
      ],
    });
  });

  it("persists Buddy chats as workspace chat tabs", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a", "buddy-a"],
      currentThreadId: "buddy-a",
      tabs: [
        {
          id: "chat-a",
          title: "Research",
          mode: "agent",
          tool_use: "agent",
        },
        {
          id: "buddy-a",
          title: "Buddy report",
          mode: "buddy",
          tool_use: "agent",
          is_buddy_chat: true,
        },
      ],
    });

    expect(loadPersistedChatTabs()).toEqual({
      openThreadIds: ["chat-a", "buddy-a"],
      currentThreadId: "buddy-a",
      tabs: [
        {
          id: "chat-a",
          title: "Research",
          mode: "agent",
          tool_use: "agent",
          session_state: undefined,
          is_buddy_chat: undefined,
          is_task_chat: undefined,
        },
        {
          id: "buddy-a",
          title: "Buddy report",
          mode: "buddy",
          tool_use: "agent",
          session_state: undefined,
          is_buddy_chat: true,
          is_task_chat: undefined,
        },
      ],
    });
  });

  it("persists the active toolbar tab", () => {
    savePersistedActiveTab({ type: "task", taskId: "task-1" });
    expect(loadPersistedActiveTab()).toEqual({
      type: "task",
      taskId: "task-1",
    });

    savePersistedActiveTab({ type: "chat", id: "chat-1" });
    expect(loadPersistedActiveTab()).toEqual({ type: "chat", id: "chat-1" });

    savePersistedActiveTab({ type: "buddy" });
    expect(loadPersistedActiveTab()).toEqual({ type: "buddy" });

    savePersistedActiveTab({ type: "dashboard" });
    expect(loadPersistedActiveTab()).toEqual({ type: "dashboard" });
  });

  it("round-trips workspace layout state under the project namespace", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a", "chat-b"],
      currentThreadId: "chat-b",
      tabs: [{ id: "chat-a" }, { id: "chat-b" }],
    });
    const workspace = {
      ...splitWorkspace(),
      contextChatByTab: {
        [chatSurface("chat-a")]: "chat-b",
      },
      liveEditsByChat: { "chat-a": false, "chat-b": true },
    };

    savePersistedWorkspace(workspace);

    expect(loadPersistedWorkspace()).toEqual(workspace);
  });

  it("restores the active chats dock section and migrates legacy tasks", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a", "chat-b"],
      currentThreadId: "chat-b",
      tabs: [{ id: "chat-a" }, { id: "chat-b" }],
    });
    for (const [saved, restored] of [
      ["activity", "activity"],
      ["agents", "agents"],
      ["tasks", "activity"],
    ] as const) {
      savePersistedWorkspace(splitWorkspace());
      const stored = JSON.parse(
        localStorage.getItem(workspaceStorageKey()) ?? "{}",
      ) as Record<string, unknown>;
      localStorage.setItem(
        workspaceStorageKey(),
        JSON.stringify({
          ...stored,
          dock: { open: true, width: 300, section: saved },
        }),
      );
      expect(loadPersistedWorkspace().dock).toEqual({
        open: true,
        width: 300,
        section: restored,
      });
    }
  });

  it("prunes restored contextual surfaces for closed chats", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a"],
      currentThreadId: "chat-a",
      tabs: [{ id: "chat-a" }],
    });
    const file = makeSurfaceKey("file", "/worktrees/chat-a/src/main.ts");
    const orphan = makeSurfaceKey("file", "/worktrees/chat-b/src/main.ts");
    localStorage.setItem(
      workspaceStorageKey(),
      JSON.stringify({
        version: 4,
        tabs: [chatSurface("chat-a"), file, orphan],
        activeTabId: orphan,
        groups: {},
        contextChatByTab: {
          [file]: "chat-a",
          [orphan]: "chat-b",
          missing: "chat-a",
          [chatSurface("chat-a")]: "chat-b",
        },
      }),
    );

    const restored = loadPersistedWorkspace();
    const group = restored.groups[chatSurface("chat-a")];
    if (!group) throw new Error("missing canonical restored group");
    expect(collectTabIds(group.root)).toEqual([chatSurface("chat-a"), file]);
    expect(restored).toMatchObject({
      tabs: [chatSurface("chat-a")],
      activeTabId: chatSurface("chat-a"),
      contextChatByTab: { [file]: "chat-a" },
    });
  });

  it("persists workspace panel opt-in per project and host", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a", "chat-b"],
      currentThreadId: "chat-b",
      tabs: [{ id: "chat-a" }, { id: "chat-b" }],
    });
    const workspace = { ...splitWorkspace(), panelsForced: true };

    savePersistedWorkspace(workspace, "vscode");

    expect(loadPersistedWorkspace("vscode")).toEqual(workspace);
    expect(loadPersistedWorkspace("jetbrains").panelsForced).toBeUndefined();
    expect(loadPersistedWorkspace().panelsForced).toBeUndefined();
  });

  it("migrates legacy Files and Terminal panel tabs", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a"],
      currentThreadId: "chat-a",
      tabs: [{ id: "chat-a" }],
    });
    const files = "files:main";
    const terminal = "terminal:main";
    const workspace = {
      tabs: [chatSurface("chat-a"), files, terminal],
      activeTabId: terminal,
      groups: {},
    };

    savePersistedWorkspace(workspace);

    expect(loadPersistedWorkspace()).toEqual({
      tabs: [chatSurface("chat-a")],
      activeTabId: chatSurface("chat-a"),
      groups: {
        [chatSurface("chat-a")]: {
          root: {
            kind: "leaf",
            id: "root",
            tabIds: [chatSurface("chat-a")],
            activeTabId: chatSurface("chat-a"),
          },
          focusedLeafId: "root",
        },
      },
      panelsForced: undefined,
      liveEditsByChat: undefined,
      dock: { open: true, width: 280, section: "files" },
      drawer: { open: true, height: 220 },
    });
  });

  it("restores task and buddy tabs into the canonical workspace pane", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a", "chat-b"],
      currentThreadId: "chat-b",
      tabs: [{ id: "chat-a" }, { id: "chat-b" }],
    });
    localStorage.setItem(
      workspaceStorageKey(),
      JSON.stringify({
        version: 2,
        tabs: [
          chatSurface("chat-a"),
          makeSurfaceKey("task", "task-a"),
          makeSurfaceKey("buddy", "home"),
        ],
        activeTabId: makeSurfaceKey("task", "task-a"),
        groups: {},
      }),
    );

    expect(loadPersistedWorkspace()).toEqual({
      tabs: [chatSurface("chat-a")],
      activeTabId: chatSurface("chat-a"),
      groups: {
        [chatSurface("chat-a")]: {
          root: {
            kind: "leaf",
            id: "root",
            tabIds: ["task:task-a", chatSurface("chat-a"), "buddy:home"],
            activeTabId: "task:task-a",
          },
          focusedLeafId: "root",
        },
      },
      panelsForced: undefined,
      liveEditsByChat: undefined,
      liveEditsDefault: undefined,
      dock: { open: true, width: 280, section: "files" },
      drawer: { open: false, height: 220 },
    });
  });

  it("ignores legacy pane-only persistence when loading workspace v2", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a", "chat-b"],
      currentThreadId: "chat-b",
      tabs: [{ id: "chat-a" }, { id: "chat-b" }],
    });
    localStorage.setItem(
      `refact:project:${getProjectStorageNamespace()}:${LEGACY_PANE_LAYOUT_STORAGE_KEY}`,
      JSON.stringify({
        version: 1,
        focusedLeafId: "right",
        root: {
          kind: "split",
          id: "root:split:row",
          dir: "row",
          sizes: [0.7, 0.3],
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
        },
      }),
    );

    expect(loadPersistedWorkspace()).toEqual({
      tabs: [chatSurface("chat-b")],
      activeTabId: chatSurface("chat-b"),
      groups: {},
      dock: { open: true, width: 280, section: "files" },
      drawer: { open: false, height: 220 },
    });
  });

  it("falls back to one active workspace tab for corrupt or oversized workspace v2", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a", "chat-b"],
      currentThreadId: "chat-b",
      tabs: [{ id: "chat-a" }, { id: "chat-b" }],
    });
    localStorage.setItem(
      workspaceStorageKey(),
      JSON.stringify({
        version: 2,
        tabs: Array.from({ length: 31 }, (_, index) =>
          chatSurface(`chat-${index}`),
        ),
        activeTabId: chatSurface("chat-a"),
        groups: {},
      }),
    );

    expect(loadPersistedWorkspace()).toEqual({
      tabs: [chatSurface("chat-b")],
      activeTabId: chatSurface("chat-b"),
      groups: {},
      dock: { open: true, width: 280, section: "files" },
      drawer: { open: false, height: 220 },
    });

    localStorage.setItem(workspaceStorageKey(), "not json");
    expect(loadPersistedWorkspace()).toEqual({
      tabs: [chatSurface("chat-b")],
      activeTabId: chatSurface("chat-b"),
      groups: {},
      dock: { open: true, width: 280, section: "files" },
      drawer: { open: false, height: 220 },
    });
  });

  it("falls back deterministically when the persisted active tab no longer resolves", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a", "chat-b"],
      currentThreadId: "chat-b",
      tabs: [{ id: "chat-a" }, { id: "chat-b" }],
    });
    localStorage.setItem(
      workspaceStorageKey(),
      JSON.stringify({
        version: 2,
        tabs: [chatSurface("chat-a"), chatSurface("chat-b")],
        activeTabId: chatSurface("missing"),
        groups: {},
      }),
    );

    expect(loadPersistedWorkspace()).toEqual({
      tabs: [chatSurface("chat-a")],
      activeTabId: chatSurface("chat-a"),
      groups: {
        [chatSurface("chat-a")]: {
          root: {
            kind: "leaf",
            id: "root",
            tabIds: [chatSurface("chat-a"), chatSurface("chat-b")],
            activeTabId: chatSurface("chat-a"),
          },
          focusedLeafId: "root",
        },
      },
      panelsForced: undefined,
      liveEditsByChat: undefined,
      dock: { open: true, width: 280, section: "files" },
      drawer: { open: false, height: 220 },
    });
  });

  it("prunes dangling workspace chat surfaces", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a"],
      currentThreadId: "chat-a",
      tabs: [{ id: "chat-a" }],
    });
    localStorage.setItem(
      workspaceStorageKey(),
      JSON.stringify({
        version: 2,
        tabs: [chatSurface("chat-a"), chatSurface("missing")],
        activeTabId: chatSurface("missing"),
        groups: {
          [chatSurface("chat-a")]: {
            root: {
              kind: "split",
              id: "root:split:row",
              dir: "row",
              sizes: [0.7, 0.3],
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
                  tabIds: [chatSurface("missing")],
                  activeTabId: chatSurface("missing"),
                },
              ],
            },
            focusedLeafId: "right",
          },
        },
      }),
    );

    expect(loadPersistedWorkspace()).toEqual({
      tabs: [chatSurface("chat-a")],
      activeTabId: chatSurface("chat-a"),
      groups: {
        [chatSurface("chat-a")]: {
          root: {
            kind: "leaf",
            id: "left",
            tabIds: [chatSurface("chat-a")],
            activeTabId: chatSurface("chat-a"),
          },
          focusedLeafId: "left",
        },
      },
      panelsForced: undefined,
      liveEditsByChat: undefined,
      dock: { open: true, width: 280, section: "files" },
      drawer: { open: false, height: 220 },
    });
  });

  it("loads corrupt duplicate workspace groups as safe unsplit tabs", () => {
    savePersistedChatTabs({
      openThreadIds: ["chat-a", "chat-b", "chat-c", "chat-d"],
      currentThreadId: "chat-a",
      tabs: [
        { id: "chat-a" },
        { id: "chat-b" },
        { id: "chat-c" },
        { id: "chat-d" },
      ],
    });
    localStorage.setItem(
      workspaceStorageKey(),
      JSON.stringify({
        version: 2,
        tabs: [chatSurface("chat-a"), chatSurface("chat-c")],
        activeTabId: chatSurface("chat-a"),
        groups: {
          [chatSurface("chat-a")]: {
            root: {
              kind: "split",
              id: "duplicate:split",
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
                  tabIds: [chatSurface("chat-a")],
                  activeTabId: chatSurface("chat-a"),
                },
              ],
            },
            focusedLeafId: "right",
          },
          [chatSurface("chat-c")]: {
            root: {
              kind: "split",
              id: "valid:split",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                {
                  kind: "leaf",
                  id: "valid-left",
                  tabIds: [chatSurface("chat-c")],
                  activeTabId: chatSurface("chat-c"),
                },
                {
                  kind: "leaf",
                  id: "valid-right",
                  tabIds: [chatSurface("chat-d")],
                  activeTabId: chatSurface("chat-d"),
                },
              ],
            },
            focusedLeafId: "valid-right",
          },
        },
      }),
    );

    const workspace = loadPersistedWorkspace();
    expect(workspace.tabs).toEqual([chatSurface("chat-a")]);
    expect(Object.keys(workspace.groups)).toEqual([chatSurface("chat-a")]);
    const group = workspace.groups[chatSurface("chat-a")];
    if (!group) throw new Error("missing merged legacy groups");
    expect(collectTabIds(group.root)).toEqual([
      chatSurface("chat-a"),
      chatSurface("chat-c"),
      chatSurface("chat-d"),
    ]);
  });

  it("persists task management tabs and their active child chat", () => {
    savePersistedTasksUIState({
      openTasks: [
        {
          id: "task-1",
          name: "Ship persistence",
          plannerChats: [
            {
              id: "planner-1",
              title: "Plan",
              createdAt: "2026-05-02T00:00:00Z",
              updatedAt: "2026-05-02T01:00:00Z",
              sessionState: "completed",
            },
          ],
          activeChat: { type: "agent", cardId: "T-1", chatId: "agent-1" },
        },
      ],
    });

    expect(loadPersistedTasksUIState()).toEqual({
      openTasks: [
        {
          id: "task-1",
          name: "Ship persistence",
          plannerChats: [
            {
              id: "planner-1",
              title: "Plan",
              createdAt: "2026-05-02T00:00:00Z",
              updatedAt: "2026-05-02T01:00:00Z",
              sessionState: "completed",
            },
          ],
          activeChat: { type: "agent", cardId: "T-1", chatId: "agent-1" },
        },
      ],
    });
  });

  it("restores ask-question drafts by tool call id", () => {
    saveAskQuestionsDraft(
      "tool-call-1",
      { q1: "Yes", q2: ["A", "B"] },
      "Extra context",
    );

    expect(loadAskQuestionsDraft("tool-call-1")).toMatchObject({
      answers: { q1: "Yes", q2: ["A", "B"] },
      additionalText: "Extra context",
    });

    clearAskQuestionsDraft("tool-call-1");
    expect(loadAskQuestionsDraft("tool-call-1")).toBeNull();
  });

  it("persists task workspace tab per task", () => {
    saveTaskWorkspaceTab("task-1", "memories");
    saveTaskWorkspaceTab("task-2", "board");

    expect(loadTaskWorkspaceTab("task-1")).toBe("memories");
    expect(loadTaskWorkspaceTab("task-2")).toBe("board");
    expect(loadTaskWorkspaceTab("task-3")).toBeNull();
  });
});
