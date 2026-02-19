import { describe, it, expect } from "vitest";
import { applyDeltaOps, DeltaOp } from "../services/refact/chatSubscription";
import type { ChatMessage } from "../services/refact/types";
import { selectToolResultById } from "../features/Chat/Thread/selectors";
import type { RootState } from "../app/store";

describe("applyDeltaOps", () => {
  it("appends content correctly across multiple deltas", () => {
    const initial: ChatMessage = { role: "assistant", content: "" };
    const ops1: DeltaOp[] = [{ op: "append_content", text: "Hello" }];
    const ops2: DeltaOp[] = [{ op: "append_content", text: " World" }];

    const after1 = applyDeltaOps(initial, ops1);
    const after2 = applyDeltaOps(after1, ops2);

    expect(after1.content).toBe("Hello");
    expect(after2.content).toBe("Hello World");
  });

  it("appends reasoning correctly", () => {
    const initial: ChatMessage = { role: "assistant", content: "" };
    const ops: DeltaOp[] = [
      { op: "append_reasoning", text: "Thinking..." },
      { op: "append_reasoning", text: " More thoughts." },
    ];

    const result = applyDeltaOps(initial, ops);
    expect((result as { reasoning_content?: string }).reasoning_content).toBe(
      "Thinking... More thoughts.",
    );
  });

  it("add_citation does not mutate prior message objects", () => {
    const initial: ChatMessage = { role: "assistant", content: "test" };
    const citation1 = { url: "http://example.com/1", title: "Example 1" };
    const citation2 = { url: "http://example.com/2", title: "Example 2" };

    const after1 = applyDeltaOps(initial, [
      { op: "add_citation", citation: citation1 },
    ]);
    const after2 = applyDeltaOps(after1, [
      { op: "add_citation", citation: citation2 },
    ]);

    const initialCitations = (initial as { citations?: unknown[] }).citations;
    const after1Citations = (after1 as { citations?: unknown[] }).citations;
    const after2Citations = (after2 as { citations?: unknown[] }).citations;

    expect(initialCitations).toBeUndefined();
    expect(after1Citations).toHaveLength(1);
    expect(after2Citations).toHaveLength(2);
    expect(after1Citations).not.toBe(after2Citations);
  });

  it("handles set_tool_calls", () => {
    const initial: ChatMessage = { role: "assistant", content: "" };
    const toolCalls = [
      { id: "1", function: { name: "test", arguments: "{}" } },
    ];
    const ops: DeltaOp[] = [{ op: "set_tool_calls", tool_calls: toolCalls }];

    const result = applyDeltaOps(initial, ops);
    expect((result as { tool_calls?: unknown[] }).tool_calls).toEqual(
      toolCalls,
    );
  });

  it("handles set_thinking_blocks", () => {
    const initial: ChatMessage = { role: "assistant", content: "" };
    const blocks = [{ thinking: "test thought" }];
    const ops: DeltaOp[] = [{ op: "set_thinking_blocks", blocks }];

    const result = applyDeltaOps(initial, ops);
    expect((result as { thinking_blocks?: unknown[] }).thinking_blocks).toEqual(
      blocks,
    );
  });

  it("handles set_usage", () => {
    const initial: ChatMessage = { role: "assistant", content: "" };
    const usage = { prompt_tokens: 100, completion_tokens: 50 };
    const ops: DeltaOp[] = [{ op: "set_usage", usage }];

    const result = applyDeltaOps(initial, ops);
    expect((result as { usage?: unknown }).usage).toEqual(usage);
  });

  it("handles merge_extra", () => {
    const initial: ChatMessage = {
      role: "assistant",
      content: "",
      extra: { a: 1 },
    } as ChatMessage & { extra: Record<string, unknown> };
    const ops: DeltaOp[] = [{ op: "merge_extra", extra: { b: 2 } }];

    const result = applyDeltaOps(initial, ops);
    expect((result as { extra?: Record<string, unknown> }).extra).toEqual({
      a: 1,
      b: 2,
    });
  });
});

describe("selectToolResultById optimization", () => {
  it("finds tool result from end without array copy", () => {
    const mockState = {
      chat: {
        current_thread_id: "test",
        threads: {
          test: {
            thread: {
              messages: [
                { role: "tool", tool_call_id: "id1", content: "first" },
                { role: "tool", tool_call_id: "id2", content: "second" },
                { role: "tool", tool_call_id: "id1", content: "third" },
              ],
            },
          },
        },
      },
    } as unknown as RootState;

    const result = selectToolResultById(mockState, "id1");
    expect(result?.content).toBe("third");
  });

  it("returns undefined for missing id", () => {
    const mockState = {
      chat: {
        current_thread_id: "test",
        threads: {
          test: {
            thread: {
              messages: [
                { role: "tool", tool_call_id: "id1", content: "first" },
              ],
            },
          },
        },
      },
    } as unknown as RootState;

    const result = selectToolResultById(mockState, "nonexistent");
    expect(result).toBeUndefined();
  });
});
