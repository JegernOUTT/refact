import { describe, it, expect } from "vitest";
import { getHistoryTree, HistoryState, ChatHistoryItem } from "./historySlice";

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
    automatic_patch: false,
    last_user_message_id: "",
    ...overrides,
  };
}

describe("getHistoryTree", () => {
  it("returns empty array for empty state", () => {
    const state: HistoryState = {};
    const result = getHistoryTree({ history: state });
    expect(result).toEqual([]);
  });

  it("returns flat list when no parent_id relationships exist", () => {
    const state: HistoryState = {
      chat1: createHistoryItem("chat1", "Chat 1", {
        updatedAt: "2024-01-03T00:00:00Z",
      }),
      chat2: createHistoryItem("chat2", "Chat 2", {
        updatedAt: "2024-01-02T00:00:00Z",
      }),
      chat3: createHistoryItem("chat3", "Chat 3", {
        updatedAt: "2024-01-01T00:00:00Z",
      }),
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
      orphan: createHistoryItem("orphan", "Orphan", {
        updatedAt: "2024-01-02T00:00:00Z",
        parent_id: "nonexistent",
      }),
      regular: createHistoryItem("regular", "Regular", {
        updatedAt: "2024-01-01T00:00:00Z",
      }),
    };

    const result = getHistoryTree({ history: state });

    expect(result).toHaveLength(2);
    expect(result.map((n) => n.id)).toContain("orphan");
    expect(result.map((n) => n.id)).toContain("regular");
  });

  it("sorts roots and children by updatedAt descending", () => {
    const state: HistoryState = {
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
    };

    const result = getHistoryTree({ history: state });

    expect(result[0].children[0].id).toBe("child_new");
    expect(result[0].children[1].id).toBe("child_mid");
    expect(result[0].children[2].id).toBe("child_old");
  });

  it("filters out task chats from tree", () => {
    const state: HistoryState = {
      task_chat: createHistoryItem("task_chat", "Task Chat", {
        task_id: "task-123",
      }),
      regular: createHistoryItem("regular", "Regular Chat"),
    };

    const result = getHistoryTree({ history: state });

    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("regular");
  });

  it("inverts handoff relationship - handoff becomes root with parent as child", () => {
    const state: HistoryState = {
      original: createHistoryItem("original", "Original Chat", {
        updatedAt: "2024-01-01T00:00:00Z",
      }),
      handoff: createHistoryItem("handoff", "Handoff Chat", {
        updatedAt: "2024-01-02T00:00:00Z",
        parent_id: "original",
        link_type: "handoff",
      }),
    };

    const result = getHistoryTree({ history: state });

    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("handoff");
    expect(result[0].children).toHaveLength(1);
    expect(result[0].children[0].id).toBe("original");
  });

  it("keeps subagent as child of parent", () => {
    const state: HistoryState = {
      parent: createHistoryItem("parent", "Parent Chat", {
        updatedAt: "2024-01-02T00:00:00Z",
      }),
      subagent: createHistoryItem("subagent", "Subagent Chat", {
        updatedAt: "2024-01-01T00:00:00Z",
        parent_id: "parent",
        link_type: "subagent",
      }),
    };

    const result = getHistoryTree({ history: state });

    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("parent");
    expect(result[0].children).toHaveLength(1);
    expect(result[0].children[0].id).toBe("subagent");
  });
});
