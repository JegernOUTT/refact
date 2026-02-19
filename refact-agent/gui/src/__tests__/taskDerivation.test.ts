import { describe, it, expect } from "vitest";
import type { ChatMessages } from "../services/refact/types";
import type { TodoItem } from "../features/Chat/Thread/types";

const normalizeTaskStatus = (status: unknown): TodoItem["status"] | null => {
  if (typeof status !== "string") return null;
  switch (status.toLowerCase()) {
    case "pending":
      return "pending";
    case "in_progress":
    case "in-progress":
    case "inprogress":
      return "in_progress";
    case "completed":
    case "done":
    case "complete":
      return "completed";
    case "failed":
    case "error":
      return "failed";
    default:
      return null;
  }
};

const sanitizeText = (text: string, maxLen: number): string => {
  return (
    text
      // eslint-disable-next-line no-control-regex
      .replace(/[\x00-\x1F\x7F]/g, "")
      .trim()
      .slice(0, maxLen)
  );
};

const parseTasksFromArgs = (argsStr: string): TodoItem[] | null => {
  try {
    const args = JSON.parse(argsStr) as unknown;
    if (!args || typeof args !== "object") return null;
    const tasksArray = (args as Record<string, unknown>).tasks;
    if (!Array.isArray(tasksArray)) return null;

    if (tasksArray.length === 0) return [];

    const result: TodoItem[] = [];
    const seenIds = new Set<string>();

    for (const item of tasksArray) {
      if (!item || typeof item !== "object") continue;
      const t = item as Record<string, unknown>;

      const rawId =
        typeof t.id === "string"
          ? t.id
          : typeof t.id === "number"
            ? String(t.id)
            : null;
      if (!rawId) continue;

      const id = sanitizeText(rawId, 50);
      if (!id || seenIds.has(id)) continue;
      seenIds.add(id);

      const rawContent = typeof t.content === "string" ? t.content : null;
      if (!rawContent) continue;

      const content = sanitizeText(rawContent, 500);
      if (!content) continue;

      const status = normalizeTaskStatus(t.status);
      if (!status) continue;

      result.push({ id, content, status });
    }
    return result.length > 0 ? result : null;
  } catch {
    return null;
  }
};

type ToolMessage = {
  role: "tool";
  tool_call_id: string;
  tool_failed?: boolean;
  content: string;
};

const deriveTasksFromMessages = (
  messages: ChatMessages,
  toolMessages: ToolMessage[],
): TodoItem[] => {
  const successfulToolIds = new Set(
    toolMessages.filter((m) => !m.tool_failed).map((m) => m.tool_call_id),
  );

  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (msg.role !== "assistant" || !("tool_calls" in msg) || !msg.tool_calls)
      continue;

    for (let j = msg.tool_calls.length - 1; j >= 0; j--) {
      const tc = msg.tool_calls[j];
      if (tc.function.name !== "tasks_set" || !tc.id) continue;
      if (!successfulToolIds.has(tc.id)) continue;

      const parsed = parseTasksFromArgs(tc.function.arguments);
      if (parsed !== null) return parsed;
    }
  }

  return [];
};

describe("normalizeTaskStatus", () => {
  it("normalizes standard statuses", () => {
    expect(normalizeTaskStatus("pending")).toBe("pending");
    expect(normalizeTaskStatus("in_progress")).toBe("in_progress");
    expect(normalizeTaskStatus("completed")).toBe("completed");
    expect(normalizeTaskStatus("failed")).toBe("failed");
  });

  it("normalizes aliases", () => {
    expect(normalizeTaskStatus("done")).toBe("completed");
    expect(normalizeTaskStatus("complete")).toBe("completed");
    expect(normalizeTaskStatus("inprogress")).toBe("in_progress");
    expect(normalizeTaskStatus("in-progress")).toBe("in_progress");
    expect(normalizeTaskStatus("error")).toBe("failed");
  });

  it("is case insensitive", () => {
    expect(normalizeTaskStatus("PENDING")).toBe("pending");
    expect(normalizeTaskStatus("In_Progress")).toBe("in_progress");
    expect(normalizeTaskStatus("DONE")).toBe("completed");
  });

  it("returns null for invalid statuses", () => {
    expect(normalizeTaskStatus("invalid")).toBe(null);
    expect(normalizeTaskStatus("")).toBe(null);
    expect(normalizeTaskStatus(123)).toBe(null);
    expect(normalizeTaskStatus(null)).toBe(null);
    expect(normalizeTaskStatus(undefined)).toBe(null);
  });
});

describe("parseTasksFromArgs", () => {
  it("parses valid tasks", () => {
    const args = JSON.stringify({
      tasks: [
        { id: "1", content: "Task one", status: "pending" },
        { id: "2", content: "Task two", status: "in_progress" },
      ],
    });
    expect(parseTasksFromArgs(args)).toEqual([
      { id: "1", content: "Task one", status: "pending" },
      { id: "2", content: "Task two", status: "in_progress" },
    ]);
  });

  it("returns empty array for explicit empty tasks", () => {
    const args = JSON.stringify({ tasks: [] });
    expect(parseTasksFromArgs(args)).toEqual([]);
  });

  it("returns null for non-empty but all invalid tasks", () => {
    const args = JSON.stringify({
      tasks: [
        { id: "", content: "No id", status: "pending" },
        { id: "2", content: "", status: "pending" },
        { id: "3", content: "Bad status", status: "invalid" },
      ],
    });
    expect(parseTasksFromArgs(args)).toBe(null);
  });

  it("filters out invalid items but keeps valid ones", () => {
    const args = JSON.stringify({
      tasks: [
        { id: "1", content: "Valid", status: "pending" },
        { id: "", content: "Invalid", status: "pending" },
        { id: "3", content: "Also valid", status: "completed" },
      ],
    });
    expect(parseTasksFromArgs(args)).toEqual([
      { id: "1", content: "Valid", status: "pending" },
      { id: "3", content: "Also valid", status: "completed" },
    ]);
  });

  it("trims whitespace from id and content", () => {
    const args = JSON.stringify({
      tasks: [{ id: "  1  ", content: "  Task  ", status: "pending" }],
    });
    expect(parseTasksFromArgs(args)).toEqual([
      { id: "1", content: "Task", status: "pending" },
    ]);
  });

  it("rejects whitespace-only id or content", () => {
    const args = JSON.stringify({
      tasks: [{ id: "   ", content: "Task", status: "pending" }],
    });
    expect(parseTasksFromArgs(args)).toBe(null);
  });

  it("accepts numeric id", () => {
    const args = JSON.stringify({
      tasks: [{ id: 42, content: "Task", status: "pending" }],
    });
    expect(parseTasksFromArgs(args)).toEqual([
      { id: "42", content: "Task", status: "pending" },
    ]);
  });

  it("rejects object id", () => {
    const args = JSON.stringify({
      tasks: [{ id: { foo: "bar" }, content: "Task", status: "pending" }],
    });
    expect(parseTasksFromArgs(args)).toBe(null);
  });

  it("deduplicates tasks with same id", () => {
    const args = JSON.stringify({
      tasks: [
        { id: "1", content: "First", status: "pending" },
        { id: "1", content: "Duplicate", status: "completed" },
        { id: "2", content: "Second", status: "pending" },
      ],
    });
    expect(parseTasksFromArgs(args)).toEqual([
      { id: "1", content: "First", status: "pending" },
      { id: "2", content: "Second", status: "pending" },
    ]);
  });

  it("strips control characters from content", () => {
    const args = JSON.stringify({
      tasks: [{ id: "1", content: "Hello\x00\x1FWorld", status: "pending" }],
    });
    expect(parseTasksFromArgs(args)).toEqual([
      { id: "1", content: "HelloWorld", status: "pending" },
    ]);
  });

  it("truncates long content", () => {
    const longContent = "x".repeat(600);
    const args = JSON.stringify({
      tasks: [{ id: "1", content: longContent, status: "pending" }],
    });
    const result = parseTasksFromArgs(args);
    expect(result?.[0].content.length).toBe(500);
  });

  it("returns null for malformed JSON", () => {
    expect(parseTasksFromArgs("not json")).toBe(null);
    expect(parseTasksFromArgs("{incomplete")).toBe(null);
  });

  it("returns null for missing tasks field", () => {
    expect(parseTasksFromArgs(JSON.stringify({}))).toBe(null);
    expect(parseTasksFromArgs(JSON.stringify({ other: [] }))).toBe(null);
  });

  it("returns null for non-array tasks", () => {
    expect(parseTasksFromArgs(JSON.stringify({ tasks: "string" }))).toBe(null);
    expect(parseTasksFromArgs(JSON.stringify({ tasks: 123 }))).toBe(null);
  });

  it("truncates long id", () => {
    const longId = "x".repeat(100);
    const args = JSON.stringify({
      tasks: [{ id: longId, content: "Task", status: "pending" }],
    });
    const result = parseTasksFromArgs(args);
    expect(result?.[0].id.length).toBe(50);
  });

  it("handles mixed valid and invalid in large batch", () => {
    const tasks = [
      { id: "1", content: "Valid 1", status: "pending" },
      { id: "", content: "Empty id", status: "pending" },
      { id: "2", content: "", status: "pending" },
      { id: "3", content: "Valid 3", status: "invalid_status" },
      { id: "4", content: "Valid 4", status: "completed" },
      { id: null, content: "Null id", status: "pending" },
      { id: "5", content: null, status: "pending" },
      { id: "6", content: "Valid 6", status: "in_progress" },
    ];
    const args = JSON.stringify({ tasks });
    expect(parseTasksFromArgs(args)).toEqual([
      { id: "1", content: "Valid 1", status: "pending" },
      { id: "4", content: "Valid 4", status: "completed" },
      { id: "6", content: "Valid 6", status: "in_progress" },
    ]);
  });

  it("handles unicode content", () => {
    const args = JSON.stringify({
      tasks: [{ id: "1", content: "Fix bug 🐛 in auth", status: "pending" }],
    });
    expect(parseTasksFromArgs(args)).toEqual([
      { id: "1", content: "Fix bug 🐛 in auth", status: "pending" },
    ]);
  });

  it("strips tabs from content", () => {
    const args = JSON.stringify({
      tasks: [{ id: "1", content: "Before\tAfter", status: "pending" }],
    });
    const result = parseTasksFromArgs(args);
    expect(result?.[0].content).toBe("BeforeAfter");
  });

  it("handles deeply nested invalid structure", () => {
    const args = JSON.stringify({
      tasks: [[{ id: "1", content: "Nested", status: "pending" }]],
    });
    expect(parseTasksFromArgs(args)).toBe(null);
  });

  it("handles task with extra fields gracefully", () => {
    const args = JSON.stringify({
      tasks: [
        {
          id: "1",
          content: "Task",
          status: "pending",
          extra: "ignored",
          nested: { deep: true },
        },
      ],
    });
    expect(parseTasksFromArgs(args)).toEqual([
      { id: "1", content: "Task", status: "pending" },
    ]);
  });

  it("handles numeric id zero", () => {
    const args = JSON.stringify({
      tasks: [{ id: 0, content: "Task zero", status: "pending" }],
    });
    expect(parseTasksFromArgs(args)).toEqual([
      { id: "0", content: "Task zero", status: "pending" },
    ]);
  });

  it("handles boolean id by rejecting", () => {
    const args = JSON.stringify({
      tasks: [{ id: true, content: "Task", status: "pending" }],
    });
    expect(parseTasksFromArgs(args)).toBe(null);
  });

  it("handles array id by rejecting", () => {
    const args = JSON.stringify({
      tasks: [{ id: [1, 2], content: "Task", status: "pending" }],
    });
    expect(parseTasksFromArgs(args)).toBe(null);
  });

  it("preserves task order", () => {
    const args = JSON.stringify({
      tasks: [
        { id: "c", content: "Third", status: "pending" },
        { id: "a", content: "First", status: "pending" },
        { id: "b", content: "Second", status: "pending" },
      ],
    });
    const result = parseTasksFromArgs(args);
    expect(result?.map((t) => t.id)).toEqual(["c", "a", "b"]);
  });

  it("handles all statuses", () => {
    const args = JSON.stringify({
      tasks: [
        { id: "1", content: "Task 1", status: "pending" },
        { id: "2", content: "Task 2", status: "in_progress" },
        { id: "3", content: "Task 3", status: "completed" },
        { id: "4", content: "Task 4", status: "failed" },
      ],
    });
    const result = parseTasksFromArgs(args);
    expect(result?.map((t) => t.status)).toEqual([
      "pending",
      "in_progress",
      "completed",
      "failed",
    ]);
  });

  it("handles negative numeric id", () => {
    const args = JSON.stringify({
      tasks: [{ id: -1, content: "Negative", status: "pending" }],
    });
    expect(parseTasksFromArgs(args)).toEqual([
      { id: "-1", content: "Negative", status: "pending" },
    ]);
  });

  it("handles float numeric id", () => {
    const args = JSON.stringify({
      tasks: [{ id: 3.14, content: "Float", status: "pending" }],
    });
    expect(parseTasksFromArgs(args)).toEqual([
      { id: "3.14", content: "Float", status: "pending" },
    ]);
  });
});

describe("deriveTasksFromMessages", () => {
  const makeAssistantMsg = (toolCalls: { id: string; args: string }[]) => ({
    role: "assistant" as const,
    content: "Response",
    tool_calls: toolCalls.map((tc, index) => ({
      id: tc.id,
      index,
      type: "function" as const,
      function: { name: "tasks_set", arguments: tc.args },
    })),
  });

  const makeToolMsg = (toolCallId: string, failed = false): ToolMessage => ({
    role: "tool",
    tool_call_id: toolCallId,
    tool_failed: failed,
    content: "OK",
  });

  it("returns empty array when no tasks_set calls", () => {
    const messages: ChatMessages = [
      { role: "user", content: "Hello" },
      { role: "assistant", content: "Hi" },
    ];
    expect(deriveTasksFromMessages(messages, [])).toEqual([]);
  });

  it("ignores tasks_set without tool result", () => {
    const args = JSON.stringify({
      tasks: [{ id: "1", content: "Task", status: "pending" }],
    });
    const messages: ChatMessages = [makeAssistantMsg([{ id: "tc1", args }])];
    expect(deriveTasksFromMessages(messages, [])).toEqual([]);
  });

  it("ignores tasks_set with failed tool result", () => {
    const args = JSON.stringify({
      tasks: [{ id: "1", content: "Task", status: "pending" }],
    });
    const messages: ChatMessages = [makeAssistantMsg([{ id: "tc1", args }])];
    const toolMessages = [makeToolMsg("tc1", true)];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([]);
  });

  it("parses tasks_set with successful tool result", () => {
    const args = JSON.stringify({
      tasks: [{ id: "1", content: "Task", status: "pending" }],
    });
    const messages: ChatMessages = [makeAssistantMsg([{ id: "tc1", args }])];
    const toolMessages = [makeToolMsg("tc1", false)];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([
      { id: "1", content: "Task", status: "pending" },
    ]);
  });

  it("returns last valid tasks_set (backwards scan)", () => {
    const args1 = JSON.stringify({
      tasks: [{ id: "1", content: "First", status: "pending" }],
    });
    const args2 = JSON.stringify({
      tasks: [{ id: "2", content: "Second", status: "completed" }],
    });
    const messages: ChatMessages = [
      makeAssistantMsg([{ id: "tc1", args: args1 }]),
      makeAssistantMsg([{ id: "tc2", args: args2 }]),
    ];
    const toolMessages = [makeToolMsg("tc1"), makeToolMsg("tc2")];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([
      { id: "2", content: "Second", status: "completed" },
    ]);
  });

  it("skips invalid last tasks_set and uses previous valid one", () => {
    const validArgs = JSON.stringify({
      tasks: [{ id: "1", content: "Valid", status: "pending" }],
    });
    const invalidArgs = JSON.stringify({
      tasks: [{ id: "", content: "Invalid", status: "pending" }],
    });
    const messages: ChatMessages = [
      makeAssistantMsg([{ id: "tc1", args: validArgs }]),
      makeAssistantMsg([{ id: "tc2", args: invalidArgs }]),
    ];
    const toolMessages = [makeToolMsg("tc1"), makeToolMsg("tc2")];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([
      { id: "1", content: "Valid", status: "pending" },
    ]);
  });

  it("clears tasks when last valid tasks_set has empty array", () => {
    const args1 = JSON.stringify({
      tasks: [{ id: "1", content: "Task", status: "pending" }],
    });
    const args2 = JSON.stringify({ tasks: [] });
    const messages: ChatMessages = [
      makeAssistantMsg([{ id: "tc1", args: args1 }]),
      makeAssistantMsg([{ id: "tc2", args: args2 }]),
    ];
    const toolMessages = [makeToolMsg("tc1"), makeToolMsg("tc2")];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([]);
  });

  it("handles tool_failed undefined as success", () => {
    const args = JSON.stringify({
      tasks: [{ id: "1", content: "Task", status: "pending" }],
    });
    const messages: ChatMessages = [makeAssistantMsg([{ id: "tc1", args }])];
    const toolMessages: ToolMessage[] = [
      { role: "tool", tool_call_id: "tc1", content: "OK" },
    ];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([
      { id: "1", content: "Task", status: "pending" },
    ]);
  });

  it("handles multiple tool calls in one assistant message", () => {
    const args1 = JSON.stringify({
      tasks: [{ id: "1", content: "First", status: "pending" }],
    });
    const args2 = JSON.stringify({
      tasks: [{ id: "2", content: "Second", status: "completed" }],
    });
    const messages: ChatMessages = [
      makeAssistantMsg([
        { id: "tc1", args: args1 },
        { id: "tc2", args: args2 },
      ]),
    ];
    const toolMessages = [makeToolMsg("tc1"), makeToolMsg("tc2")];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([
      { id: "2", content: "Second", status: "completed" },
    ]);
  });

  it("ignores non-tasks_set tool calls", () => {
    const messages: ChatMessages = [
      {
        role: "assistant",
        content: "Response",
        tool_calls: [
          {
            id: "tc1",
            index: 0,
            type: "function" as const,
            function: { name: "cat", arguments: '{"path":"file.txt"}' },
          },
        ],
      },
    ];
    const toolMessages = [makeToolMsg("tc1")];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([]);
  });

  it("handles interleaved user messages", () => {
    const args1 = JSON.stringify({
      tasks: [{ id: "1", content: "First", status: "pending" }],
    });
    const args2 = JSON.stringify({
      tasks: [{ id: "2", content: "Updated", status: "completed" }],
    });
    const messages: ChatMessages = [
      makeAssistantMsg([{ id: "tc1", args: args1 }]),
      { role: "user", content: "Continue" },
      makeAssistantMsg([{ id: "tc2", args: args2 }]),
    ];
    const toolMessages = [makeToolMsg("tc1"), makeToolMsg("tc2")];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([
      { id: "2", content: "Updated", status: "completed" },
    ]);
  });

  it("handles assistant message without tool_calls", () => {
    const messages: ChatMessages = [
      { role: "assistant", content: "Just text response" },
    ];
    expect(deriveTasksFromMessages(messages, [])).toEqual([]);
  });

  it("handles tool call with empty id", () => {
    const args = JSON.stringify({
      tasks: [{ id: "1", content: "Task", status: "pending" }],
    });
    const messages: ChatMessages = [
      {
        role: "assistant",
        content: "Response",
        tool_calls: [
          {
            id: "",
            index: 0,
            type: "function" as const,
            function: { name: "tasks_set", arguments: args },
          },
        ],
      },
    ];
    const toolMessages = [makeToolMsg("")];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([]);
  });

  it("handles mixed successful and failed tool results", () => {
    const args1 = JSON.stringify({
      tasks: [{ id: "1", content: "First", status: "pending" }],
    });
    const args2 = JSON.stringify({
      tasks: [{ id: "2", content: "Second", status: "completed" }],
    });
    const messages: ChatMessages = [
      makeAssistantMsg([{ id: "tc1", args: args1 }]),
      makeAssistantMsg([{ id: "tc2", args: args2 }]),
    ];
    const toolMessages = [makeToolMsg("tc1", false), makeToolMsg("tc2", true)];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([
      { id: "1", content: "First", status: "pending" },
    ]);
  });

  it("returns empty when only failed tool results exist", () => {
    const args = JSON.stringify({
      tasks: [{ id: "1", content: "Task", status: "pending" }],
    });
    const messages: ChatMessages = [makeAssistantMsg([{ id: "tc1", args }])];
    const toolMessages = [makeToolMsg("tc1", true)];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([]);
  });

  it("handles large conversation with many tasks_set calls", () => {
    const messages: ChatMessages = [];
    const toolMessages: ToolMessage[] = [];

    for (let i = 0; i < 10; i++) {
      const args = JSON.stringify({
        tasks: [{ id: String(i), content: `Task ${i}`, status: "pending" }],
      });
      messages.push(makeAssistantMsg([{ id: `tc${i}`, args }]));
      toolMessages.push(makeToolMsg(`tc${i}`));
    }

    const result = deriveTasksFromMessages(messages, toolMessages);
    expect(result).toEqual([{ id: "9", content: "Task 9", status: "pending" }]);
  });

  it("handles tool result arriving before corresponding assistant message in array", () => {
    const args = JSON.stringify({
      tasks: [{ id: "1", content: "Task", status: "pending" }],
    });
    const messages: ChatMessages = [makeAssistantMsg([{ id: "tc1", args }])];
    const toolMessages = [makeToolMsg("tc1")];
    expect(deriveTasksFromMessages(messages, toolMessages)).toEqual([
      { id: "1", content: "Task", status: "pending" },
    ]);
  });
});
