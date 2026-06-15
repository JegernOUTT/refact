import { beforeEach, describe, expect, it } from "vitest";
import {
  clearAskQuestionsDraft,
  loadAskQuestionsDraft,
  loadPersistedActiveTab,
  loadPersistedChatTabs,
  loadPersistedTasksUIState,
  loadTaskWorkspaceTab,
  saveAskQuestionsDraft,
  savePersistedActiveTab,
  savePersistedChatTabs,
  savePersistedTasksUIState,
  saveTaskWorkspaceTab,
} from "../chatUiPersistence";
import {
  getProjectStorageNamespace,
  setProjectStorageNamespace,
  setProjectStorageNamespaceFromProjectInfo,
} from "../chatUiPersistence";

describe("chatUiPersistence", () => {
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

  it("uses a stable hashed namespace for equivalent multi-root identities", () => {
    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/workspace/b/", "/workspace/a", "/workspace/a/"],
      projectName: "fallback",
    });
    const first = getProjectStorageNamespace();

    setProjectStorageNamespaceFromProjectInfo({
      workspaceRoots: ["/workspace/a", "/workspace/b"],
      projectName: "other-fallback",
    });

    expect(first?.startsWith("v2:")).toBe(true);
    expect(getProjectStorageNamespace()).toBe(first);
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

  it("excludes Buddy chats from normal persisted chat tabs", () => {
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
      openThreadIds: ["chat-a"],
      currentThreadId: "chat-a",
      tabs: [
        {
          id: "chat-a",
          title: "Research",
          mode: "agent",
          tool_use: "agent",
          session_state: undefined,
          is_buddy_chat: undefined,
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

    savePersistedActiveTab({ type: "dashboard" });
    expect(loadPersistedActiveTab()).toEqual({ type: "dashboard" });
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
