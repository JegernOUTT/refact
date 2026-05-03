import { beforeEach, describe, expect, it } from "vitest";
import {
  clearAskQuestionsDraft,
  loadAskQuestionsDraft,
  loadPersistedActiveTab,
  loadPersistedChatTabs,
  loadPersistedTasksUIState,
  loadTaskWorkspaceLayout,
  saveAskQuestionsDraft,
  savePersistedActiveTab,
  savePersistedChatTabs,
  savePersistedTasksUIState,
  saveTaskWorkspaceLayout,
} from "../chatUiPersistence";

describe("chatUiPersistence", () => {
  beforeEach(() => {
    localStorage.clear();
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

  it("persists task workspace layout per task", () => {
    const defaults = {
      chatExpanded: false,
      panelsExpanded: false,
      boardHeightPx: 180,
    };

    saveTaskWorkspaceLayout("task-1", {
      chatExpanded: true,
      panelsExpanded: true,
      boardHeightPx: 260,
    });

    expect(loadTaskWorkspaceLayout("task-1", defaults)).toEqual({
      chatExpanded: true,
      panelsExpanded: true,
      boardHeightPx: 260,
    });
    expect(loadTaskWorkspaceLayout("task-2", defaults)).toEqual(defaults);
  });
});
