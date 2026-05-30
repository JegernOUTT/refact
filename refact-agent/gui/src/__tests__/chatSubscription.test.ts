/**
 * Chat Subscription Service Tests
 *
 * Tests for the fetch-based SSE chat subscription system.
 *
 * Run with: npm run test:no-watch -- chatSubscription
 */

/* eslint-disable @typescript-eslint/require-await */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  subscribeToChatEvents,
  applyDeltaOps,
  type ChatEventEnvelope,
  type DeltaOp,
} from "../services/refact/chatSubscription";
import type { AssistantMessage } from "../services/refact/types";

type TestMessage = AssistantMessage & {
  reasoning_content?: string;
  thinking_blocks?: unknown[];
  citations?: unknown[];
  usage?: unknown;
};

const mockFetch = vi.fn();

describe("chatSubscription", () => {
  describe("applyDeltaOps", () => {
    it("should append content to string content", () => {
      const message: TestMessage = {
        role: "assistant",
        content: "Hello",
      };

      const ops: DeltaOp[] = [{ op: "append_content", text: " world" }];

      const result = applyDeltaOps(message, ops) as TestMessage;
      expect(result.content).toBe("Hello world");
    });

    it("should initialize content if not a string", () => {
      const message: TestMessage = {
        role: "assistant",
        content: undefined as unknown as string,
      };

      const ops: DeltaOp[] = [{ op: "append_content", text: "Hello" }];

      const result = applyDeltaOps(message, ops) as TestMessage;
      expect(result.content).toBe("Hello");
    });

    it("should append reasoning content", () => {
      const message: TestMessage = {
        role: "assistant",
        content: "",
        reasoning_content: "Step 1: ",
      };

      const ops: DeltaOp[] = [{ op: "append_reasoning", text: "analyze" }];

      const result = applyDeltaOps(message, ops) as TestMessage;
      expect(result.reasoning_content).toBe("Step 1: analyze");
    });

    it("should initialize reasoning content if empty", () => {
      const message: TestMessage = {
        role: "assistant",
        content: "",
      };

      const ops: DeltaOp[] = [{ op: "append_reasoning", text: "thinking" }];

      const result = applyDeltaOps(message, ops) as TestMessage;
      expect(result.reasoning_content).toBe("thinking");
    });

    it("should set tool calls", () => {
      const message: TestMessage = {
        role: "assistant",
        content: "",
      };

      const toolCalls = [
        { id: "call_1", function: { name: "test", arguments: "{}" } },
      ];
      const ops: DeltaOp[] = [{ op: "set_tool_calls", tool_calls: toolCalls }];

      const result = applyDeltaOps(message, ops) as TestMessage;
      expect(result.tool_calls).toEqual(toolCalls);
    });

    it("should set thinking blocks", () => {
      const message: TestMessage = {
        role: "assistant",
        content: "",
      };

      const blocks = [{ thinking: "reasoning here" }];
      const ops: DeltaOp[] = [{ op: "set_thinking_blocks", blocks }];

      const result = applyDeltaOps(message, ops) as TestMessage;
      expect(result.thinking_blocks).toEqual(blocks);
    });

    it("should add citations", () => {
      const message: TestMessage = {
        role: "assistant",
        content: "",
      };

      const citation1 = { url: "http://example.com/1" };
      const citation2 = { url: "http://example.com/2" };
      const ops: DeltaOp[] = [
        { op: "add_citation", citation: citation1 },
        { op: "add_citation", citation: citation2 },
      ];

      const result = applyDeltaOps(message, ops) as TestMessage;
      expect(result.citations).toEqual([citation1, citation2]);
    });

    it("should set usage", () => {
      const message: TestMessage = {
        role: "assistant",
        content: "",
      };

      const usage = { prompt_tokens: 100, completion_tokens: 50 };
      const ops: DeltaOp[] = [{ op: "set_usage", usage }];

      const result = applyDeltaOps(message, ops) as TestMessage;
      expect(result.usage).toEqual(usage);
    });

    it("should apply multiple ops in sequence", () => {
      const message: TestMessage = {
        role: "assistant",
        content: "",
      };

      const ops: DeltaOp[] = [
        { op: "append_content", text: "Hello" },
        { op: "append_content", text: " " },
        { op: "append_content", text: "world" },
        { op: "append_reasoning", text: "thinking..." },
        {
          op: "set_tool_calls",
          tool_calls: [
            { id: "1", function: { name: "test", arguments: "{}" } },
          ],
        },
      ];

      const result = applyDeltaOps(message, ops) as TestMessage;
      expect(result.content).toBe("Hello world");
      expect(result.reasoning_content).toBe("thinking...");
      expect(result.tool_calls).toHaveLength(1);
    });
  });

  describe("subscribeToChatEvents", () => {
    beforeEach(() => {
      global.fetch = mockFetch;
      mockFetch.mockReset();
    });

    afterEach(() => {
      vi.restoreAllMocks();
    });

    it("should make fetch request with correct URL and headers", () => {
      const chatId = "test-chat-123";
      const port = 8001;
      const apiKey = "test-key";

      mockFetch.mockResolvedValueOnce({
        ok: true,
        body: {
          getReader: () => ({
            read: vi.fn().mockResolvedValue({ done: true }),
          }),
        },
      });

      subscribeToChatEvents(
        chatId,
        { host: "vscode", lspPort: port },
        {
          onEvent: vi.fn(),
          onError: vi.fn(),
        },
        apiKey,
      );

      expect(mockFetch).toHaveBeenCalledWith(
        `http://127.0.0.1:${port}/v1/chats/subscribe?chat_id=${chatId}`,
        expect.objectContaining({
          method: "GET",
          headers: { Authorization: "Bearer test-key" },
        }),
      );
    });

    it("uses a relative SSE URL in Vite and engine-served web mode", () => {
      const read = vi.fn().mockResolvedValue({ done: true });
      mockFetch.mockResolvedValue({
        ok: true,
        body: { getReader: () => ({ read }) },
      });

      subscribeToChatEvents(
        "chat/1",
        { host: "web", dev: true },
        {
          onEvent: vi.fn(),
          onError: vi.fn(),
        },
      );
      subscribeToChatEvents(
        "chat/2",
        { host: "web", engineServed: true, lspUrl: "http://127.0.0.1:8001" },
        {
          onEvent: vi.fn(),
          onError: vi.fn(),
        },
      );

      expect(mockFetch).toHaveBeenNthCalledWith(
        1,
        "/v1/chats/subscribe?chat_id=chat%2F1",
        expect.objectContaining({ method: "GET" }),
      );
      expect(mockFetch).toHaveBeenNthCalledWith(
        2,
        "/v1/chats/subscribe?chat_id=chat%2F2",
        expect.objectContaining({ method: "GET" }),
      );
    });

    it("uses the configured remote origin in web mode", () => {
      mockFetch.mockResolvedValue({
        ok: true,
        body: {
          getReader: () => ({
            read: vi.fn().mockResolvedValue({ done: true }),
          }),
        },
      });

      subscribeToChatEvents(
        "remote-chat",
        { host: "web", lspUrl: "https://remote.example.com/proxy/v1/ping" },
        {
          onEvent: vi.fn(),
          onError: vi.fn(),
        },
      );

      expect(mockFetch).toHaveBeenCalledWith(
        "https://remote.example.com/proxy/v1/chats/subscribe?chat_id=remote-chat",
        expect.objectContaining({ method: "GET" }),
      );
    });

    it("should normalize CRLF line endings", async () => {
      const onEvent = vi.fn();
      const encoder = new TextEncoder();

      const events =
        'data: {"type":"snapshot","seq":"1","chat_id":"test","background_agents":[]}\r\n\r\n';

      mockFetch.mockResolvedValueOnce({
        ok: true,
        body: {
          getReader: () => {
            let called = false;
            return {
              read: async () => {
                if (called) return { done: true, value: undefined };
                called = true;
                return { done: false, value: encoder.encode(events) };
              },
            };
          },
        },
      });

      subscribeToChatEvents(
        "test",
        { host: "vscode", lspPort: 8001 },
        {
          onEvent,
          onError: vi.fn(),
        },
      );

      await new Promise((resolve) => setTimeout(resolve, 10));

      expect(onEvent).toHaveBeenCalledWith(
        expect.objectContaining({ type: "snapshot" }),
      );
    });

    it.each([
      ["missing", { type: "snapshot", seq: "1", chat_id: "test" }],
      [
        "null",
        {
          type: "snapshot",
          seq: "1",
          chat_id: "test",
          background_agents: null,
        },
      ],
    ])(
      "should default %s snapshot background agents to an empty list",
      async (_case, payload) => {
        const onEvent = vi.fn();
        const encoder = new TextEncoder();
        const events = `data: ${JSON.stringify(payload)}\n\n`;

        mockFetch.mockResolvedValueOnce({
          ok: true,
          body: {
            getReader: () => {
              let called = false;
              return {
                read: async () => {
                  if (called) return { done: true, value: undefined };
                  called = true;
                  return { done: false, value: encoder.encode(events) };
                },
              };
            },
          },
        });

        subscribeToChatEvents(
          "test",
          { host: "vscode", lspPort: 8001 },
          {
            onEvent,
            onError: vi.fn(),
          },
        );

        await new Promise((resolve) => setTimeout(resolve, 10));

        expect(onEvent).toHaveBeenCalledWith(
          expect.objectContaining({
            type: "snapshot",
            background_agents: [],
          }),
        );
      },
    );

    it("should parse process_completed envelope without throwing", async () => {
      const onEvent = vi.fn<(event: ChatEventEnvelope) => void>();
      const onError = vi.fn();
      const encoder = new TextEncoder();
      const event: ChatEventEnvelope = {
        chat_id: "test",
        seq: "2",
        type: "process_completed",
        process_id: "exec_done",
        status: "exited",
        exit_code: 0,
        short_description: "test process",
        mode: "background",
      };

      mockFetch.mockResolvedValueOnce({
        ok: true,
        body: {
          getReader: () => {
            let called = false;
            return {
              read: async () => {
                if (called) return { done: true, value: undefined };
                called = true;
                return {
                  done: false,
                  value: encoder.encode(`data: ${JSON.stringify(event)}\n\n`),
                };
              },
            };
          },
        },
      });

      subscribeToChatEvents(
        "test",
        { host: "vscode", lspPort: 8001 },
        {
          onEvent,
          onError,
        },
      );

      await new Promise((resolve) => setTimeout(resolve, 10));

      expect(onError).not.toHaveBeenCalled();
      expect(onEvent).toHaveBeenCalledWith(event);
    });

    it("should call onDisconnected on normal stream close", async () => {
      const onDisconnected = vi.fn();

      mockFetch.mockResolvedValueOnce({
        ok: true,
        body: {
          getReader: () => ({
            read: vi.fn().mockResolvedValue({ done: true }),
          }),
        },
      });

      subscribeToChatEvents(
        "test",
        { host: "vscode", lspPort: 8001 },
        {
          onEvent: vi.fn(),
          onError: vi.fn(),
          onDisconnected,
        },
      );

      await new Promise((resolve) => setTimeout(resolve, 10));

      expect(onDisconnected).toHaveBeenCalled();
    });

    it("should call onError on idle timeout", async () => {
      vi.useFakeTimers();
      const onError = vi.fn();
      const pendingRead = new Promise<{
        done: boolean;
        value: Uint8Array | undefined;
      }>((resolve) => {
        void resolve;
      });
      const read = vi.fn(() => pendingRead);

      mockFetch.mockResolvedValueOnce({
        ok: true,
        body: {
          getReader: () => ({ read }),
        },
      });

      const unsubscribe = subscribeToChatEvents(
        "test",
        { host: "vscode", lspPort: 8001 },
        {
          onEvent: vi.fn(),
          onError,
        },
        undefined,
        { idleTimeoutMs: 10 },
      );

      await Promise.resolve();
      await vi.advanceTimersByTimeAsync(10);

      expect(onError).toHaveBeenCalledWith(
        expect.objectContaining({ message: "SSE idle timeout" }),
      );
      unsubscribe();
      vi.useRealTimers();
    });
  });
});
