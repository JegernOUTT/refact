import { describe, it, expect } from "vitest";
import {
  buildHistoryTree,
  type ChatHistoryItem,
} from "../features/History/historySlice";
import { buildDotTrail } from "../features/Dashboard/components/DotTrail/buildDotTrail";

const createItem = (
  id: string,
  overrides: Partial<ChatHistoryItem> = {},
): ChatHistoryItem => ({
  id,
  title: `Chat ${id}`,
  model: "gpt-4",
  mode: "AGENT",
  tool_use: "agent",
  messages: [],
  boost_reasoning: false,
  context_tokens_cap: undefined,
  include_project_info: true,
  increase_max_tokens: false,

  project_name: undefined,
  isTitleGenerated: true,
  createdAt: "2024-01-01T00:00:00Z",
  last_user_message_id: "",
  updatedAt: overrides.updatedAt ?? "2024-01-01T00:00:00Z",
  ...overrides,
});

describe("buildHistoryTree", () => {
  describe("basic tree building", () => {
    it("should return empty array for empty input", () => {
      const result = buildHistoryTree({});
      expect(result).toEqual([]);
    });

    it("should return single root for single item", () => {
      const chats = { a: createItem("a") };
      const result = buildHistoryTree(chats);
      expect(result).toHaveLength(1);
      expect(result[0].id).toBe("a");
      expect(result[0].children).toHaveLength(0);
    });

    it("should return multiple roots for unrelated items", () => {
      const chats = {
        a: createItem("a", { updatedAt: "2024-01-03T00:00:00Z" }),
        b: createItem("b", { updatedAt: "2024-01-02T00:00:00Z" }),
        c: createItem("c", { updatedAt: "2024-01-01T00:00:00Z" }),
      };
      const result = buildHistoryTree(chats);
      expect(result).toHaveLength(3);
      expect(result[0].id).toBe("a");
      expect(result[1].id).toBe("b");
      expect(result[2].id).toBe("c");
    });
  });

  describe("handoff chains", () => {
    it("should handle single handoff (A -> B)", () => {
      const chats = {
        a: createItem("a", { updatedAt: "2024-01-01T00:00:00Z" }),
        b: createItem("b", {
          parent_id: "a",
          link_type: "handoff",
          updatedAt: "2024-01-02T00:00:00Z",
        }),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(1);
      expect(result[0].id).toBe("b");
      expect(result[0].children).toHaveLength(1);
      expect(result[0].children[0].id).toBe("a");
    });

    it("should handle double handoff chain (A -> B -> C)", () => {
      const chats = {
        a: createItem("a", { updatedAt: "2024-01-01T00:00:00Z" }),
        b: createItem("b", {
          parent_id: "a",
          link_type: "handoff",
          updatedAt: "2024-01-02T00:00:00Z",
        }),
        c: createItem("c", {
          parent_id: "b",
          link_type: "handoff",
          updatedAt: "2024-01-03T00:00:00Z",
        }),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(1);
      expect(result[0].id).toBe("c");
      expect(result[0].children).toHaveLength(1);
      expect(result[0].children[0].id).toBe("b");
      expect(result[0].children[0].children).toHaveLength(1);
      expect(result[0].children[0].children[0].id).toBe("a");
    });

    it("should handle triple handoff chain (A -> B -> C -> D)", () => {
      const chats = {
        a: createItem("a", { updatedAt: "2024-01-01T00:00:00Z" }),
        b: createItem("b", {
          parent_id: "a",
          link_type: "handoff",
          updatedAt: "2024-01-02T00:00:00Z",
        }),
        c: createItem("c", {
          parent_id: "b",
          link_type: "handoff",
          updatedAt: "2024-01-03T00:00:00Z",
        }),
        d: createItem("d", {
          parent_id: "c",
          link_type: "handoff",
          updatedAt: "2024-01-04T00:00:00Z",
        }),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(1);
      expect(result[0].id).toBe("d");
      expect(result[0].children[0].id).toBe("c");
      expect(result[0].children[0].children[0].id).toBe("b");
      expect(result[0].children[0].children[0].children[0].id).toBe("a");
    });
  });

  describe("subagent links", () => {
    it("should hide subagent rows and expose them as parent bubbles", () => {
      const chats = {
        a: createItem("a", { updatedAt: "2024-01-01T00:00:00Z" }),
        b: createItem("b", {
          parent_id: "a",
          link_type: "subagent",
          updatedAt: "2024-01-02T00:00:00Z",
        }),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(1);
      expect(result[0].id).toBe("a");
      expect(result[0].children).toHaveLength(0);
      expect(result[0].bubbleChildren).toHaveLength(1);
      expect(result[0].bubbleChildren[0].id).toBe("b");
    });

    it("should hide gather-files rows and expose them as parent bubbles", () => {
      const chats = {
        a: createItem("a", { updatedAt: "2024-01-01T00:00:00Z" }),
        b: createItem("b", {
          parent_id: "a",
          link_type: "gather_files",
          title: "Strategic Planning: Gathering Files",
          updatedAt: "2024-01-02T00:00:00Z",
        }),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(1);
      expect(result[0].id).toBe("a");
      expect(result[0].children).toHaveLength(0);
      expect(result[0].bubbleChildren.map((child) => child.id)).toEqual(["b"]);
    });
  });

  describe("mixed links", () => {
    it("should handle handoff with subagent child", () => {
      const chats = {
        a: createItem("a", { updatedAt: "2024-01-01T00:00:00Z" }),
        b: createItem("b", {
          parent_id: "a",
          link_type: "handoff",
          updatedAt: "2024-01-02T00:00:00Z",
        }),
        c: createItem("c", {
          parent_id: "b",
          link_type: "subagent",
          updatedAt: "2024-01-03T00:00:00Z",
        }),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(1);
      expect(result[0].id).toBe("b");
      expect(result[0].children).toHaveLength(1);
      expect(result[0].bubbleChildren).toHaveLength(1);
      const childIds = result[0].children.map((c) => c.id).sort();
      expect(childIds).toEqual(["a"]);
      expect(result[0].bubbleChildren[0].id).toBe("c");
    });

    it("should build bubbles only for subagents of the displayed trajectory", () => {
      const chats = {
        a: createItem("a", { updatedAt: "2024-01-01T00:00:00Z" }),
        b: createItem("b", {
          parent_id: "a",
          link_type: "handoff",
          updatedAt: "2024-01-02T00:00:00Z",
        }),
        c: createItem("c", {
          parent_id: "a",
          link_type: "subagent",
          updatedAt: "2024-01-03T00:00:00Z",
        }),
        d: createItem("d", {
          parent_id: "b",
          link_type: "gather_files",
          updatedAt: "2024-01-04T00:00:00Z",
        }),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(1);
      expect(result[0].id).toBe("b");
      expect(buildDotTrail(result[0]).map((dot) => dot.chatId)).toEqual(["d"]);
      expect(buildDotTrail(result[0].children[0]).map((dot) => dot.chatId)).toEqual([
        "c",
      ]);
    });
  });

  describe("cycle prevention", () => {
    it("should not create cycles with self-reference", () => {
      const chats = {
        a: createItem("a", { parent_id: "a", link_type: "handoff" }),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(1);
      expect(result[0].id).toBe("a");
      expect(result[0].children).toHaveLength(0);
    });

    it("should not create cycles with mutual reference", () => {
      const chats = {
        a: createItem("a", {
          parent_id: "b",
          link_type: "handoff",
          updatedAt: "2024-01-02T00:00:00Z",
        }),
        b: createItem("b", {
          parent_id: "a",
          link_type: "handoff",
          updatedAt: "2024-01-01T00:00:00Z",
        }),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(1);
      expect(result[0].children).toHaveLength(1);
      expect(result[0].children[0].children).toHaveLength(0);
    });
  });

  describe("task filtering", () => {
    it("should exclude items with task_id", () => {
      const chats = {
        a: createItem("a"),
        b: createItem("b", { task_id: "task-1" }),
        c: createItem("c"),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(2);
      expect(result.map((r) => r.id).sort()).toEqual(["a", "c"]);
    });

    it("should exclude items with mode task_agent", () => {
      const chats = {
        a: createItem("a"),
        b: createItem("b", { mode: "task_agent" }),
        c: createItem("c"),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(2);
      expect(result.map((r) => r.id).sort()).toEqual(["a", "c"]);
    });

    it("should exclude items with mode task_planner", () => {
      const chats = {
        a: createItem("a"),
        b: createItem("b", { mode: "task_planner" }),
        c: createItem("c"),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(2);
      expect(result.map((r) => r.id).sort()).toEqual(["a", "c"]);
    });
  });

  describe("sorting", () => {
    it("should sort roots by updatedAt descending", () => {
      const chats = {
        a: createItem("a", { updatedAt: "2024-01-01T00:00:00Z" }),
        b: createItem("b", { updatedAt: "2024-01-03T00:00:00Z" }),
        c: createItem("c", { updatedAt: "2024-01-02T00:00:00Z" }),
      };
      const result = buildHistoryTree(chats);

      expect(result[0].id).toBe("b");
      expect(result[1].id).toBe("c");
      expect(result[2].id).toBe("a");
    });

    it("should sort visible children by updatedAt descending", () => {
      const chats = {
        a: createItem("a", { updatedAt: "2024-01-04T00:00:00Z" }),
        b: createItem("b", {
          parent_id: "a",
          updatedAt: "2024-01-01T00:00:00Z",
        }),
        c: createItem("c", {
          parent_id: "a",
          updatedAt: "2024-01-03T00:00:00Z",
        }),
        d: createItem("d", {
          parent_id: "a",
          updatedAt: "2024-01-02T00:00:00Z",
        }),
      };
      const result = buildHistoryTree(chats);

      expect(result[0].children[0].id).toBe("c");
      expect(result[0].children[1].id).toBe("d");
      expect(result[0].children[2].id).toBe("b");
    });

    it("should sort bubble children by updatedAt descending", () => {
      const chats = {
        a: createItem("a", { updatedAt: "2024-01-04T00:00:00Z" }),
        b: createItem("b", {
          parent_id: "a",
          link_type: "subagent",
          updatedAt: "2024-01-01T00:00:00Z",
        }),
        c: createItem("c", {
          parent_id: "a",
          link_type: "gather_files",
          updatedAt: "2024-01-03T00:00:00Z",
        }),
        d: createItem("d", {
          parent_id: "a",
          link_type: "code_review",
          updatedAt: "2024-01-02T00:00:00Z",
        }),
      };
      const result = buildHistoryTree(chats);

      expect(result[0].children).toHaveLength(0);
      expect(result[0].bubbleChildren[0].id).toBe("c");
      expect(result[0].bubbleChildren[1].id).toBe("d");
      expect(result[0].bubbleChildren[2].id).toBe("b");
    });
  });

  describe("missing parent handling", () => {
    it("should treat item as root if parent_id not found", () => {
      const chats = {
        a: createItem("a", { parent_id: "nonexistent", link_type: "handoff" }),
      };
      const result = buildHistoryTree(chats);

      expect(result).toHaveLength(1);
      expect(result[0].id).toBe("a");
    });
  });
});
