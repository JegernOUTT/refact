import { describe, it, expect } from "vitest";
import {
  getHistoryTree,
  HistoryState,
  ChatHistoryItem,
  historySlice,
} from "./historySlice";

function createHistoryItem(
  id: string,
  title: string,
  overrides: Partial<ChatHistoryItem> = {},
): ChatHistoryItem {
  return {
    id,
    title,
    createdAt: "2024-01-01T00:00:00Z",
    updatedAt: "2024-01-01T00:00:00Z",
    model: "gpt-4",
    mode: "AGENT",
    tool_use: "agent",
    messages: [],
    boost_reasoning: false,
    include_project_info: true,
    increase_max_tokens: false,

    last_user_message_id: "",
    ...overrides,
  };
}

const defaultPagination = {
  cursor: null,
  hasMore: true,
  totalCount: null,
  generation: 0,
};

describe("getHistoryTree", () => {
  it("returns empty array for empty state", () => {
    const state: HistoryState = {
      chats: {},
      isLoading: false,
      loadError: null,
      pagination: defaultPagination,
    };
    const result = getHistoryTree({ history: state });
    expect(result).toEqual([]);
  });

  it("returns flat list when no parent_id relationships exist", () => {
    const state: HistoryState = {
      chats: {
        chat1: createHistoryItem("chat1", "Chat 1", {
          updatedAt: "2024-01-03T00:00:00Z",
        }),
        chat2: createHistoryItem("chat2", "Chat 2", {
          updatedAt: "2024-01-02T00:00:00Z",
        }),
        chat3: createHistoryItem("chat3", "Chat 3", {
          updatedAt: "2024-01-01T00:00:00Z",
        }),
      },
      isLoading: false,
      loadError: null,
      pagination: defaultPagination,
    };

    const result = getHistoryTree({ history: state });

    expect(result).toHaveLength(3);
    expect(result[0].id).toBe("chat1");
    expect(result[1].id).toBe("chat2");
    expect(result[2].id).toBe("chat3");
    expect(result[0].children).toEqual([]);
  });

  it("builds tree structure with parent_id relationships", () => {
    const state: HistoryState = {
      chats: {
        parent: createHistoryItem("parent", "Parent Chat", {
          updatedAt: "2024-01-03T00:00:00Z",
        }),
        child1: createHistoryItem("child1", "Child 1", {
          updatedAt: "2024-01-02T00:00:00Z",
          parent_id: "parent",
        }),
        child2: createHistoryItem("child2", "Child 2", {
          updatedAt: "2024-01-01T00:00:00Z",
          parent_id: "parent",
        }),
      },
      isLoading: false,
      loadError: null,
      pagination: defaultPagination,
    };

    const result = getHistoryTree({ history: state });

    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("parent");
    expect(result[0].children).toHaveLength(2);
    expect(result[0].children[0].id).toBe("child1");
    expect(result[0].children[1].id).toBe("child2");
  });

  it("handles nested tree structure", () => {
    const state: HistoryState = {
      chats: {
        root: createHistoryItem("root", "Root", {
          updatedAt: "2024-01-04T00:00:00Z",
        }),
        level1: createHistoryItem("level1", "Level 1", {
          updatedAt: "2024-01-03T00:00:00Z",
          parent_id: "root",
        }),
        level2: createHistoryItem("level2", "Level 2", {
          updatedAt: "2024-01-02T00:00:00Z",
          parent_id: "level1",
        }),
      },
      isLoading: false,
      loadError: null,
      pagination: defaultPagination,
    };

    const result = getHistoryTree({ history: state });

    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("root");
    expect(result[0].children).toHaveLength(1);
    expect(result[0].children[0].id).toBe("level1");
    expect(result[0].children[0].children).toHaveLength(1);
    expect(result[0].children[0].children[0].id).toBe("level2");
  });

  it("treats items with missing parent as roots", () => {
    const state: HistoryState = {
      chats: {
        orphan: createHistoryItem("orphan", "Orphan", {
          updatedAt: "2024-01-02T00:00:00Z",
          parent_id: "nonexistent",
        }),
        regular: createHistoryItem("regular", "Regular", {
          updatedAt: "2024-01-01T00:00:00Z",
        }),
      },
      isLoading: false,
      loadError: null,
      pagination: defaultPagination,
    };

    const result = getHistoryTree({ history: state });

    expect(result).toHaveLength(2);
    expect(result.map((n: { id: string }) => n.id)).toContain("orphan");
    expect(result.map((n: { id: string }) => n.id)).toContain("regular");
  });

  it("sorts roots and children by updatedAt descending", () => {
    const state: HistoryState = {
      chats: {
        parent: createHistoryItem("parent", "Parent", {
          updatedAt: "2024-01-01T00:00:00Z",
        }),
        child_old: createHistoryItem("child_old", "Old Child", {
          updatedAt: "2024-01-01T00:00:00Z",
          parent_id: "parent",
        }),
        child_new: createHistoryItem("child_new", "New Child", {
          updatedAt: "2024-01-03T00:00:00Z",
          parent_id: "parent",
        }),
        child_mid: createHistoryItem("child_mid", "Mid Child", {
          updatedAt: "2024-01-02T00:00:00Z",
          parent_id: "parent",
        }),
      },
      isLoading: false,
      loadError: null,
      pagination: defaultPagination,
    };

    const result = getHistoryTree({ history: state });

    expect(result[0].children[0].id).toBe("child_new");
    expect(result[0].children[1].id).toBe("child_mid");
    expect(result[0].children[2].id).toBe("child_old");
  });

  it("filters out task planner/agent chats but keeps other task-scoped chats", () => {
    const state: HistoryState = {
      chats: {
        planner_chat: createHistoryItem("planner_chat", "Planner Chat", {
          task_id: "task-123",
          mode: "task_planner",
        }),
        agent_chat: createHistoryItem("agent_chat", "Agent Chat", {
          task_id: "task-123",
          mode: "task_agent",
        }),
        review_chat: createHistoryItem("review_chat", "Review Chat", {
          task_id: "task-123",
          mode: "review",
        }),
        regular: createHistoryItem("regular", "Regular Chat"),
      },
      isLoading: false,
      loadError: null,
      pagination: defaultPagination,
    };

    const result = getHistoryTree({ history: state });

    expect(result.map((n: { id: string }) => n.id).sort()).toEqual([
      "regular",
      "review_chat",
    ]);
  });

  it("inverts handoff relationship - handoff becomes root with parent as child", () => {
    const state: HistoryState = {
      chats: {
        original: createHistoryItem("original", "Original Chat", {
          updatedAt: "2024-01-01T00:00:00Z",
        }),
        handoff: createHistoryItem("handoff", "Handoff Chat", {
          updatedAt: "2024-01-02T00:00:00Z",
          parent_id: "original",
          link_type: "handoff",
        }),
      },
      isLoading: false,
      loadError: null,
      pagination: defaultPagination,
    };

    const result = getHistoryTree({ history: state });

    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("handoff");
    expect(result[0].children).toHaveLength(1);
    expect(result[0].children[0].id).toBe("original");
  });

  it("keeps subagent hidden from rows and available as a bubble", () => {
    const state: HistoryState = {
      chats: {
        parent: createHistoryItem("parent", "Parent Chat", {
          updatedAt: "2024-01-02T00:00:00Z",
        }),
        subagent: createHistoryItem("subagent", "Subagent Chat", {
          updatedAt: "2024-01-01T00:00:00Z",
          parent_id: "parent",
          link_type: "subagent",
        }),
      },
      isLoading: false,
      loadError: null,
      pagination: defaultPagination,
    };

    const result = getHistoryTree({ history: state });

    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("parent");
    expect(result[0].children).toHaveLength(0);
    expect(result[0].bubbleChildren).toHaveLength(1);
    expect(result[0].bubbleChildren[0].id).toBe("subagent");
  });
});

describe("pagination reducers", () => {
  it("replaceSnapshotHistory stores pagination metadata with first page", () => {
    const state: HistoryState = {
      chats: {},
      isLoading: false,
      loadError: null,
      pagination: {
        cursor: null,
        hasMore: true,
        totalCount: null,
        generation: 0,
      },
    };

    const result = historySlice.reducer(
      state,
      historySlice.actions.replaceSnapshotHistory({
        items: [
          {
            id: "chat1",
            title: "Test Chat",
            created_at: "2024-01-01T00:00:00Z",
            updated_at: "2024-01-01T00:00:00Z",
            model: "gpt-4",
            mode: "AGENT",
            message_count: 5,
            total_lines_added: 0,
            total_lines_removed: 0,
            tasks_total: 0,
            tasks_done: 0,
            tasks_failed: 0,
          },
        ],
        pagination: { cursor: "next-cursor", hasMore: true, totalCount: 10 },
      }),
    );

    expect(result.pagination).toEqual({
      cursor: "next-cursor",
      hasMore: true,
      totalCount: 10,
      generation: 1,
    });
    expect(result.chats.chat1).toBeDefined();
  });

  it("replaceSnapshotHistory treats missing next cursor as terminal", () => {
    const state: HistoryState = {
      chats: {},
      isLoading: false,
      loadError: null,
      pagination: {
        cursor: "previous-cursor",
        hasMore: true,
        totalCount: null,
        generation: 0,
      },
    };

    const result = historySlice.reducer(
      state,
      historySlice.actions.replaceSnapshotHistory({
        append: true,
        items: [],
        pagination: { cursor: null, hasMore: true, totalCount: 1 },
      }),
    );

    expect(result.pagination).toMatchObject({
      cursor: null,
      hasMore: false,
      totalCount: 1,
    });
  });

  it("replaceSnapshotHistory keeps pagination when appending without new metadata", () => {
    const state: HistoryState = {
      chats: {
        chat1: createHistoryItem("chat1", "First"),
      },
      isLoading: false,
      loadError: null,
      pagination: {
        cursor: "page-2",
        hasMore: true,
        totalCount: null,
        generation: 0,
      },
    };

    const result = historySlice.reducer(
      state,
      historySlice.actions.replaceSnapshotHistory({
        append: true,
        items: [
          {
            id: "chat2",
            title: "Second",
            created_at: "2024-01-01T00:00:00Z",
            updated_at: "2024-01-01T00:00:00Z",
            model: "gpt-4",
            mode: "AGENT",
            message_count: 5,
            total_lines_added: 0,
            total_lines_removed: 0,
            tasks_total: 0,
            tasks_done: 0,
            tasks_failed: 0,
          },
        ],
      }),
    );

    expect(result.chats.chat1).toBeDefined();
    expect(result.chats.chat2).toBeDefined();
    expect(result.pagination).toEqual({
      cursor: "page-2",
      hasMore: true,
      totalCount: null,
      generation: 0,
    });
  });

  it("replaceSnapshotHistory appends later pages without pruning existing chats", () => {
    const state: HistoryState = {
      chats: {
        chat1: createHistoryItem("chat1", "First"),
      },
      isLoading: false,
      loadError: null,
      pagination: {
        cursor: "page-2",
        hasMore: true,
        totalCount: null,
        generation: 0,
      },
    };

    const result = historySlice.reducer(
      state,
      historySlice.actions.replaceSnapshotHistory({
        append: true,
        items: [
          {
            id: "chat2",
            title: "Second",
            created_at: "2024-01-01T00:00:00Z",
            updated_at: "2024-01-01T00:00:00Z",
            model: "gpt-4",
            mode: "AGENT",
            message_count: 5,
            total_lines_added: 0,
            total_lines_removed: 0,
            tasks_total: 0,
            tasks_done: 0,
            tasks_failed: 0,
          },
        ],
        pagination: { cursor: null, hasMore: false, totalCount: 2 },
      }),
    );

    expect(result.chats.chat1).toBeDefined();
    expect(result.chats.chat2).toBeDefined();
    expect(result.pagination).toEqual({
      cursor: null,
      hasMore: false,
      totalCount: 2,
      generation: 0,
    });
  });

  it("setPagination updates cursor and hasMore", () => {
    const state: HistoryState = {
      chats: {},
      isLoading: false,
      loadError: null,
      pagination: {
        cursor: null,
        hasMore: true,
        totalCount: null,
        generation: 0,
      },
    };

    const result = historySlice.reducer(
      state,
      historySlice.actions.setPagination({
        cursor: "next-cursor",
        hasMore: true,
      }),
    );

    expect(result.pagination.cursor).toBe("next-cursor");
    expect(result.pagination.hasMore).toBe(true);
  });

  it("setPagination treats missing next cursor as terminal", () => {
    const state: HistoryState = {
      chats: {},
      isLoading: false,
      loadError: null,
      pagination: {
        cursor: "previous-cursor",
        hasMore: true,
        totalCount: null,
        generation: 0,
      },
    };

    const result = historySlice.reducer(
      state,
      historySlice.actions.setPagination({
        cursor: null,
        hasMore: true,
      }),
    );

    expect(result.pagination.cursor).toBeNull();
    expect(result.pagination.hasMore).toBe(false);
  });

  it("setPagination sets hasMore to false when no more pages", () => {
    const state: HistoryState = {
      chats: {},
      isLoading: false,
      loadError: null,
      pagination: {
        cursor: "some-cursor",
        hasMore: true,
        totalCount: null,
        generation: 0,
      },
    };

    const result = historySlice.reducer(
      state,
      historySlice.actions.setPagination({
        cursor: null,
        hasMore: false,
      }),
    );

    expect(result.pagination.cursor).toBeNull();
    expect(result.pagination.hasMore).toBe(false);
  });
});

describe("error handling reducers", () => {
  it("setHistoryLoadError sets error without affecting pagination", () => {
    const state: HistoryState = {
      chats: {},
      isLoading: true,
      loadError: null,
      pagination: {
        cursor: "some-cursor",
        hasMore: true,
        totalCount: null,
        generation: 0,
      },
    };

    const result = historySlice.reducer(
      state,
      historySlice.actions.setHistoryLoadError("Network error"),
    );

    expect(result.loadError).toBe("Network error");
    expect(result.isLoading).toBe(false);
    expect(result.pagination.hasMore).toBe(true);
    expect(result.pagination.cursor).toBe("some-cursor");
  });

  it("setHistoryLoadError clears error when null is passed", () => {
    const state: HistoryState = {
      chats: {},
      isLoading: false,
      loadError: "Previous error",
      pagination: {
        cursor: null,
        hasMore: true,
        totalCount: null,
        generation: 0,
      },
    };

    const result = historySlice.reducer(
      state,
      historySlice.actions.setHistoryLoadError(null),
    );

    expect(result.loadError).toBeNull();
  });

  it("setHistoryLoading clears error when loading starts", () => {
    const state: HistoryState = {
      chats: {},
      isLoading: false,
      loadError: "Previous error",
      pagination: {
        cursor: null,
        hasMore: true,
        totalCount: null,
        generation: 0,
      },
    };

    const result = historySlice.reducer(
      state,
      historySlice.actions.setHistoryLoading(true),
    );

    expect(result.isLoading).toBe(true);
    expect(result.loadError).toBeNull();
  });
});

describe("session_state handling", () => {
  it("hydrateHistoryFromMeta includes session_state", () => {
    const state: HistoryState = {
      chats: {},
      isLoading: false,
      loadError: null,
      pagination: {
        cursor: null,
        hasMore: true,
        totalCount: null,
        generation: 0,
      },
    };

    const result = historySlice.reducer(
      state,
      historySlice.actions.hydrateHistoryFromMeta([
        {
          id: "chat1",
          title: "Test Chat",
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-01T00:00:00Z",
          model: "gpt-4",
          mode: "AGENT",
          message_count: 5,
          session_state: "generating",
          total_lines_added: 0,
          total_lines_removed: 0,
          tasks_total: 0,
          tasks_done: 0,
          tasks_failed: 0,
        },
      ]),
    );

    expect(result.chats.chat1).toBeDefined();
    expect(result.chats.chat1.session_state).toBe("generating");
  });

  it("hydrateHistoryFromMeta updates session_state for existing chats", () => {
    const state: HistoryState = {
      chats: {
        chat1: createHistoryItem("chat1", "Test Chat", {
          session_state: "idle",
        }),
      },
      isLoading: false,
      loadError: null,
      pagination: {
        cursor: null,
        hasMore: true,
        totalCount: null,
        generation: 0,
      },
    };

    const result = historySlice.reducer(
      state,
      historySlice.actions.hydrateHistoryFromMeta([
        {
          id: "chat1",
          title: "Test Chat",
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-02T00:00:00Z",
          model: "gpt-4",
          mode: "AGENT",
          message_count: 5,
          session_state: "executing_tools",
          total_lines_added: 0,
          total_lines_removed: 0,
          tasks_total: 0,
          tasks_done: 0,
          tasks_failed: 0,
        },
      ]),
    );

    expect(result.chats.chat1.session_state).toBe("executing_tools");
  });
});
