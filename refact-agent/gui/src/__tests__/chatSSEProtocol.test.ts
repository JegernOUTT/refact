/**
 * SSE Protocol Completeness & Correctness Tests
 *
 * Tests all ChatEvent types from backend (engine/src/chat/types.rs)
 * Validates event structure, sequence numbers, and state transitions
 *
 * Run with: npm run test:no-watch -- chatSSEProtocol
 */

/* eslint-disable @typescript-eslint/no-explicit-any, @typescript-eslint/no-unsafe-member-access, @typescript-eslint/no-unsafe-assignment, @typescript-eslint/require-await, @typescript-eslint/ban-ts-comment */
// @ts-nocheck - Testing runtime behavior with discriminated unions
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  subscribeToChatEvents,
  applyDeltaOps,
  type EventEnvelope,
  type DeltaOp,
} from "../services/refact/chatSubscription";
import type { ChatMessage } from "../services/refact/types";

const createMockReader = (chunks: string[]) => {
  let index = 0;
  return {
    read: vi.fn(async () => {
      if (index >= chunks.length) {
        return { done: true, value: undefined };
      }
      const encoder = new TextEncoder();
      return { done: false, value: encoder.encode(chunks[index++]) };
    }),
  };
};

const createMockFetch = (chunks: string[]) => {
  return vi.fn().mockResolvedValue({
    ok: true,
    body: {
      getReader: () => createMockReader(chunks),
    },
  });
};

describe("SSE Protocol - Event Types", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("Snapshot Event", () => {
    it("should parse snapshot with all fields", async () => {
      const snapshot: EventEnvelope = {
        chat_id: "test-123",
        seq: "0",
        type: "snapshot",
        thread: {
          id: "test-123",
          title: "Test Chat",
          model: "gpt-4",
          mode: "AGENT",
          tool_use: "agent",
          boost_reasoning: false,
          context_tokens_cap: null,
          include_project_info: true,
          checkpoints_enabled: true,
          is_title_generated: false,
        },
        runtime: {
          state: "idle",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
        },
        background_agents: [],
        messages: [],
      };

      const events: EventEnvelope[] = [];
      const mockFetch = createMockFetch([
        `data: ${JSON.stringify(snapshot)}\n\n`,
      ]);
      global.fetch = mockFetch;

      subscribeToChatEvents(
        "test-123",
        { host: "vscode", lspPort: 8001 },
        {
          onEvent: (e) => events.push(e),
          onError: vi.fn(),
        },
      );

      await new Promise((resolve) => setTimeout(resolve, 10));

      expect(events).toHaveLength(1);
      expect(events[0].type).toBe("snapshot");
      expect(events[0].seq).toBe("0");
      expect((events[0] as any).thread.id).toBe("test-123");
      expect((events[0] as any).runtime.state).toBe("idle");
    });
  });
});

describe("SSE Protocol - Sequence Numbers", () => {
  it("should accept string sequence numbers", async () => {
    const event: EventEnvelope = {
      chat_id: "test-123",
      seq: "42",
      type: "pause_cleared",
    };

    const events: EventEnvelope[] = [];
    const mockFetch = createMockFetch([`data: ${JSON.stringify(event)}\n\n`]);
    global.fetch = mockFetch;

    subscribeToChatEvents(
      "test-123",
      { host: "vscode", lspPort: 8001 },
      {
        onEvent: (e) => events.push(e),
        onError: vi.fn(),
      },
    );

    await new Promise((resolve) => setTimeout(resolve, 10));

    expect(events[0].seq).toBe("42");
  });

  it("should accept numeric sequence numbers", async () => {
    const event = {
      chat_id: "test-123",
      seq: 42,
      type: "pause_cleared",
    };

    const events: EventEnvelope[] = [];
    const mockFetch = createMockFetch([`data: ${JSON.stringify(event)}\n\n`]);
    global.fetch = mockFetch;

    subscribeToChatEvents(
      "test-123",
      { host: "vscode", lspPort: 8001 },
      {
        onEvent: (e) => events.push(e),
        onError: vi.fn(),
      },
    );

    await new Promise((resolve) => setTimeout(resolve, 10));

    expect(events[0].seq).toBe("42");
  });

  it("should handle monotonically increasing sequences", async () => {
    const events: EventEnvelope[] = [];
    const mockFetch = createMockFetch([
      `data: ${JSON.stringify({
        chat_id: "test",
        seq: "1",
        type: "pause_cleared",
      })}\n\n`,
      `data: ${JSON.stringify({
        chat_id: "test",
        seq: "2",
        type: "pause_cleared",
      })}\n\n`,
      `data: ${JSON.stringify({
        chat_id: "test",
        seq: "3",
        type: "pause_cleared",
      })}\n\n`,
    ]);
    global.fetch = mockFetch;

    subscribeToChatEvents(
      "test",
      { host: "vscode", lspPort: 8001 },
      {
        onEvent: (e) => events.push(e),
        onError: vi.fn(),
      },
    );

    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(events).toHaveLength(3);
    expect(events[0].seq).toBe("1");
    expect(events[1].seq).toBe("2");
    expect(events[2].seq).toBe("3");
  });
});

describe("SSE Protocol - Edge Cases", () => {
  it("should skip [DONE] marker", async () => {
    const events: EventEnvelope[] = [];
    const mockFetch = createMockFetch([
      `data: ${JSON.stringify({
        chat_id: "test",
        seq: "1",
        type: "pause_cleared",
      })}\n\n`,
      `data: [DONE]\n\n`,
    ]);
    global.fetch = mockFetch;

    subscribeToChatEvents(
      "test",
      { host: "vscode", lspPort: 8001 },
      {
        onEvent: (e) => events.push(e),
        onError: vi.fn(),
      },
    );

    await new Promise((resolve) => setTimeout(resolve, 10));

    expect(events).toHaveLength(1);
  });

  it("should handle malformed JSON gracefully", async () => {
    const events: EventEnvelope[] = [];
    const errors: Error[] = [];
    const mockFetch = createMockFetch([
      `data: {invalid json}\n\n`,
      `data: ${JSON.stringify({
        chat_id: "test",
        seq: "1",
        type: "pause_cleared",
      })}\n\n`,
    ]);
    global.fetch = mockFetch;

    subscribeToChatEvents(
      "test",
      { host: "vscode", lspPort: 8001 },
      {
        onEvent: (e) => events.push(e),
        onError: (e) => errors.push(e),
      },
    );

    await new Promise((resolve) => setTimeout(resolve, 10));

    expect(events).toHaveLength(1);
    expect(events[0].type).toBe("pause_cleared");
  });

  it("should handle very long sequence numbers", async () => {
    const event: EventEnvelope = {
      chat_id: "test-123",
      seq: "999999999999",
      type: "pause_cleared",
    };

    const events: EventEnvelope[] = [];
    const mockFetch = createMockFetch([`data: ${JSON.stringify(event)}\n\n`]);
    global.fetch = mockFetch;

    subscribeToChatEvents(
      "test-123",
      { host: "vscode", lspPort: 8001 },
      {
        onEvent: (e) => events.push(e),
        onError: vi.fn(),
      },
    );

    await new Promise((resolve) => setTimeout(resolve, 10));

    expect(events[0].seq).toBe("999999999999");
  });

  it("should handle rapid event sequence", async () => {
    const events: EventEnvelope[] = [];
    const mockFetch = createMockFetch([
      `data: ${JSON.stringify({
        chat_id: "test",
        seq: "1",
        type: "stream_started",
        message_id: "msg-1",
      })}\n\n`,
      `data: ${JSON.stringify({
        chat_id: "test",
        seq: "2",
        type: "stream_delta",
        message_id: "msg-1",
        ops: [{ op: "append_content", text: "H" }],
      })}\n\n`,
      `data: ${JSON.stringify({
        chat_id: "test",
        seq: "3",
        type: "stream_delta",
        message_id: "msg-1",
        ops: [{ op: "append_content", text: "i" }],
      })}\n\n`,
      `data: ${JSON.stringify({
        chat_id: "test",
        seq: "4",
        type: "stream_finished",
        message_id: "msg-1",
        finish_reason: "stop",
      })}\n\n`,
    ]);
    global.fetch = mockFetch;

    subscribeToChatEvents(
      "test",
      { host: "vscode", lspPort: 8001 },
      {
        onEvent: (e) => events.push(e),
        onError: vi.fn(),
      },
    );

    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(events).toHaveLength(4);
    expect(events[0].type).toBe("stream_started");
    expect(events[1].type).toBe("stream_delta");
    expect(events[2].type).toBe("stream_delta");
    expect(events[3].type).toBe("stream_finished");
  });
});

describe("DeltaOp Application - merge_extra", () => {
  it("should merge extra fields into message.extra", () => {
    const message: ChatMessage = {
      role: "assistant",
      content: "test",
      message_id: "msg-1",
    };

    const ops: DeltaOp[] = [{ op: "merge_extra", extra: { metering_a: 100 } }];

    const result = applyDeltaOps(message, ops) as any;
    expect(result.extra).toEqual({ metering_a: 100 });
  });

  it("should merge multiple extra fields incrementally", () => {
    const message: ChatMessage = {
      role: "assistant",
      content: "test",
      message_id: "msg-1",
    };

    const ops: DeltaOp[] = [
      { op: "merge_extra", extra: { metering_a: 100 } },
      { op: "merge_extra", extra: { metering_b: 200 } },
      { op: "merge_extra", extra: { metering_a: 150 } },
    ];

    const result = applyDeltaOps(message, ops) as any;
    expect(result.extra).toEqual({ metering_a: 150, metering_b: 200 });
  });
});
