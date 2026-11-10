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
  createChatEventMultiplexer,
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

    it("should replace partial reasoning with the completed summary", () => {
      const message: TestMessage = {
        role: "assistant",
        content: "",
        reasoning_content: "**Checking parser**",
      };

      const ops: DeltaOp[] = [
        {
          op: "set_reasoning",
          text: "**Checking parser**\n\nThe completed reasoning body.",
        },
      ];

      const result = applyDeltaOps(message, ops) as TestMessage;
      expect(result.reasoning_content).toBe(
        "**Checking parser**\n\nThe completed reasoning body.",
      );
    });

    it("should discard earlier reasoning appends when a replacement follows", () => {
      const message: TestMessage = {
        role: "assistant",
        content: "",
        reasoning_content: "Existing partial",
      };

      const ops: DeltaOp[] = [
        { op: "append_reasoning", text: " streamed header" },
        { op: "set_reasoning", text: "Completed summary" },
        { op: "append_reasoning", text: " tail" },
      ];

      const result = applyDeltaOps(message, ops) as TestMessage;
      expect(result.reasoning_content).toBe("Completed summary tail");
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
        `http://127.0.0.1:${port}/v1/chats/subscribe?chat_id=${chatId}&snapshot_chunks=1`,
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
        "/v1/chats/subscribe?chat_id=chat%2F1&snapshot_chunks=1",
        expect.objectContaining({ method: "GET" }),
      );
      expect(mockFetch).toHaveBeenNthCalledWith(
        2,
        "/v1/chats/subscribe?chat_id=chat%2F2&snapshot_chunks=1",
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
        {
          host: "web",
          lspUrl: "https://remote.example.com/proxy/v1/ping",
          lspPort: 0,
        },
        {
          onEvent: vi.fn(),
          onError: vi.fn(),
        },
      );

      expect(mockFetch).toHaveBeenCalledWith(
        "https://remote.example.com/proxy/v1/chats/subscribe?chat_id=remote-chat&snapshot_chunks=1",
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

    it("preserves a fragmented multi-megabyte diff snapshot without repeated full splits", async () => {
      const diff = "-old line\n+new 猫 line\n".repeat(80_000);
      const message = {
        role: "diff",
        content: [
          { file_name: "large.txt", lines_remove: diff, lines_add: diff },
        ],
      };
      const payload = {
        type: "snapshot",
        seq: 42,
        chat_id: "test",
        messages: [message],
        background_agents: [],
      };
      const bytes = new TextEncoder().encode(
        `data: ${JSON.stringify(payload)}\r\n\r\n`,
      );
      expect(bytes.length).toBeGreaterThan(1_000_000);
      const onEvent = vi.fn();
      const onError = vi.fn();
      let offset = 0;
      let finish!: () => void;
      const finished = new Promise<void>((resolve) => {
        finish = resolve;
      });
      mockFetch.mockResolvedValueOnce({
        ok: true,
        body: {
          getReader: () => ({
            read: async () => {
              if (offset >= bytes.length) return { done: true };
              const value = bytes.subarray(offset, offset + 4093);
              offset += value.length;
              return { done: false, value };
            },
          }),
        },
      });
      const split = vi.spyOn(String.prototype, "split");
      const unsubscribe = subscribeToChatEvents(
        "test",
        { host: "vscode", lspPort: 8001 },
        { onEvent, onError, onDisconnected: finish },
      );
      await finished;
      const largeSplits = split.mock.contexts.flatMap((input, index) =>
        typeof input === "string" && input.length > 1_000_000
          ? [split.mock.calls[index][0]]
          : [],
      );
      split.mockRestore();
      unsubscribe();
      expect(largeSplits).toEqual(["\n"]);
      expect(onError).not.toHaveBeenCalled();
      expect(onEvent).toHaveBeenCalledExactlyOnceWith({
        ...payload,
        seq: "42",
      });
    });

    it("parses fragmented multiline data, comments, no-data and multiple events", async () => {
      const input =
        ': heartbeat\r\n\r\nevent: ignored\r\rdata: {"type":"snapshot",\r\ndata: "seq":"1","chat_id":"test"}\r\n\r\ndata: [DONE]\n\ndata: {"type":"stream_started","seq":"2","chat_id":"test","message_id":"m"}\n\n';
      const bytes = new TextEncoder().encode(input);
      let offset = 0;
      let finish!: () => void;
      const finished = new Promise<void>((resolve) => {
        finish = resolve;
      });
      const onEvent = vi.fn<(event: ChatEventEnvelope) => void>();
      const onError = vi.fn();
      mockFetch.mockResolvedValueOnce({
        ok: true,
        body: {
          getReader: () => ({
            read: async () =>
              offset < bytes.length
                ? { done: false, value: bytes.subarray(offset, ++offset) }
                : { done: true },
          }),
        },
      });
      subscribeToChatEvents(
        "test",
        { lspPort: 8001 },
        {
          onEvent,
          onError,
          onDisconnected: finish,
        },
      );
      await finished;
      expect(onError).not.toHaveBeenCalled();
      expect(
        onEvent.mock.calls.map(([event]: [ChatEventEnvelope]) => event.type),
      ).toEqual(["snapshot", "stream_started"]);
    });

    it("does not publish a pending fragmented frame after unsubscribe", async () => {
      vi.useFakeTimers();
      let resolveRead!: (value: { done: boolean; value: Uint8Array }) => void;
      const pending = new Promise<{ done: boolean; value: Uint8Array }>(
        (resolve) => {
          resolveRead = resolve;
        },
      );
      const read = vi
        .fn()
        .mockResolvedValueOnce({
          done: false,
          value: new TextEncoder().encode('data: {"type":"snapshot",'),
        })
        .mockImplementation(() => pending);
      const callbacks = {
        onEvent: vi.fn(),
        onError: vi.fn(),
        onConnected: vi.fn(),
        onActivity: vi.fn(),
        onDisconnected: vi.fn(),
      };
      mockFetch.mockResolvedValueOnce({
        ok: true,
        body: { getReader: () => ({ read }) },
      });
      const unsubscribe = subscribeToChatEvents(
        "test",
        { lspPort: 8001 },
        callbacks,
      );
      await vi.advanceTimersByTimeAsync(0);
      expect(read).toHaveBeenCalledTimes(2);
      expect(callbacks.onActivity).toHaveBeenCalledOnce();
      unsubscribe();
      resolveRead({
        done: false,
        value: new TextEncoder().encode('"seq":"1","chat_id":"test"}\n\n'),
      });
      await vi.advanceTimersByTimeAsync(0);
      expect(callbacks.onEvent).not.toHaveBeenCalled();
      expect(callbacks.onError).not.toHaveBeenCalled();
      expect(callbacks.onDisconnected).not.toHaveBeenCalled();
      expect(callbacks.onActivity).toHaveBeenCalledOnce();
      expect(callbacks.onConnected).toHaveBeenCalledOnce();
      expect(vi.getTimerCount()).toBe(0);
      vi.useRealTimers();
    });

    it("reassembles chunked snapshots before publishing them", async () => {
      const onEvent = vi.fn<(event: ChatEventEnvelope) => void>();
      const encoder = new TextEncoder();
      const snapshot = {
        type: "snapshot",
        seq: "7",
        chat_id: "test",
        thread: {
          id: "test",
          title: "Chunked",
          model: "model",
          mode: "agent",
          tool_use: "agent",
          boost_reasoning: false,
          context_tokens_cap: null,
          include_project_info: true,
          checkpoints_enabled: false,
          is_title_generated: false,
        },
        runtime: {
          state: "idle",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [],
        background_agents: [],
        snapshot_chunked: true,
        chunk_count: 2,
        message_count: 2,
      };
      const events = [
        snapshot,
        {
          type: "snapshot_chunk",
          seq: "7",
          chat_id: "test",
          chunk_index: 0,
          chunk_count: 2,
          messages: [{ role: "user", content: "first" }],
        },
        {
          type: "snapshot_chunk",
          seq: "7",
          chat_id: "test",
          chunk_index: 1,
          chunk_count: 2,
          messages: [{ role: "assistant", content: "second" }],
        },
        {
          type: "snapshot_end",
          seq: "7",
          chat_id: "test",
          chunk_count: 2,
          message_count: 2,
        },
      ]
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join("");

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
        { onEvent, onError: vi.fn() },
      );

      await new Promise((resolve) => setTimeout(resolve, 10));
      expect(onEvent).toHaveBeenCalledTimes(1);
      expect(onEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: "snapshot",
          seq: "7",
          messages: [
            expect.objectContaining({ content: "first" }),
            expect.objectContaining({ content: "second" }),
          ],
        }),
      );
    });

    it("reports an incomplete chunked snapshot as a reconnectable error", async () => {
      const onEvent = vi.fn<(event: ChatEventEnvelope) => void>();
      const onError = vi.fn();
      const onDisconnected = vi.fn();
      const encoder = new TextEncoder();
      const events = [
        {
          type: "snapshot",
          seq: "7",
          chat_id: "test",
          thread: {
            id: "test",
            title: "Chunked",
            model: "model",
            mode: "agent",
            tool_use: "agent",
            boost_reasoning: false,
            context_tokens_cap: null,
            include_project_info: true,
            checkpoints_enabled: false,
            is_title_generated: false,
          },
          runtime: {
            state: "idle",
            paused: false,
            error: null,
            queue_size: 0,
            pause_reasons: [],
            queued_items: [],
          },
          messages: [],
          background_agents: [],
          snapshot_chunked: true,
          chunk_count: 2,
          message_count: 2,
        },
        {
          type: "snapshot_chunk",
          seq: "7",
          chat_id: "test",
          chunk_index: 0,
          chunk_count: 2,
          messages: [{ role: "user", content: "first" }],
        },
      ]
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join("");

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
        { onEvent, onError, onDisconnected },
      );

      await new Promise((resolve) => setTimeout(resolve, 10));
      expect(onEvent).not.toHaveBeenCalled();
      expect(onDisconnected).not.toHaveBeenCalled();
      expect(onError).toHaveBeenCalledWith(
        expect.objectContaining({
          message:
            "SSE stream ended before chunked snapshot completed; reconnecting",
        }),
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

    it.each<
      [
        string,
        Record<string, string | number | boolean>,
        { process_id: string; command_preview: string; started_at: number },
      ]
    >([
      [
        "camelCase",
        {
          processId: "exec-camel",
          commandPreview: "npm run test",
          mode: "foreground",
          tty: false,
          status: "running",
          startedAt: 123,
        },
        {
          process_id: "exec-camel",
          command_preview: "npm run test",
          started_at: 123,
        },
      ],
      [
        "snake_case",
        {
          process_id: "exec-snake",
          command_preview: "npm run dev",
          mode: "background",
          tty: true,
          status: "starting",
          started_at: 456,
        },
        {
          process_id: "exec-snake",
          command_preview: "npm run dev",
          started_at: 456,
        },
      ],
    ])(
      "normalizes %s exec process spawn events",
      async (_case, process, expectedProcess) => {
        const onEvent = vi.fn<(event: ChatEventEnvelope) => void>();
        const encoder = new TextEncoder();
        const event = {
          chat_id: "test",
          seq: "3",
          type: "exec_process_spawned",
          process,
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
          { onEvent, onError: vi.fn() },
        );

        await new Promise((resolve) => setTimeout(resolve, 10));

        const received = onEvent.mock.calls[0]?.[0];
        expect(received).toMatchObject({
          type: "exec_process_spawned",
          process: expectedProcess,
        });
      },
    );

    it("drops malformed exec process spawn events", async () => {
      const onEvent = vi.fn<(event: ChatEventEnvelope) => void>();
      const encoder = new TextEncoder();
      const event = {
        chat_id: "test",
        seq: "3",
        type: "exec_process_spawned",
        process: { processId: "exec-malformed", tty: "yes" },
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
        { onEvent, onError: vi.fn() },
      );

      await new Promise((resolve) => setTimeout(resolve, 10));

      expect(onEvent).not.toHaveBeenCalled();
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

    it("should not call onDisconnected for internal abort errors", async () => {
      vi.useFakeTimers();
      const onError = vi.fn();
      const onDisconnected = vi.fn();

      mockFetch.mockImplementationOnce((_url: string, init?: RequestInit) => {
        return new Promise((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => {
            reject(Object.assign(new Error("Aborted"), { name: "AbortError" }));
          });
        });
      });

      const unsubscribe = subscribeToChatEvents(
        "test",
        { host: "vscode", lspPort: 8001 },
        {
          onEvent: vi.fn(),
          onError,
          onDisconnected,
        },
        undefined,
        { connectTimeoutMs: 10 },
      );

      await Promise.resolve();
      await vi.advanceTimersByTimeAsync(10);
      await Promise.resolve();

      expect(onError).toHaveBeenCalledOnce();
      expect(onError).toHaveBeenCalledWith(
        expect.objectContaining({ message: "SSE connect timeout" }),
      );
      expect(onDisconnected).not.toHaveBeenCalled();
      unsubscribe();
      vi.useRealTimers();
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

describe("multiplex chat transport", () => {
  let cleanups: (() => void)[] = [];
  afterEach(() => {
    cleanups.forEach((cleanup) => cleanup());
    cleanups = [];
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it.each(["absent", "incomplete chunked"])(
    "times out a %s child despite healthy sibling snapshots and heartbeats",
    async (missing) => {
      vi.useFakeTimers();
      let stream!: ReadableStreamDefaultController<Uint8Array>;
      let signal!: AbortSignal;
      vi.stubGlobal(
        "fetch",
        vi.fn((_url: string, init: RequestInit) => {
          if (!init.signal) throw new Error("Missing abort signal");
          signal = init.signal;
          return Promise.resolve(
            new Response(
              new ReadableStream<Uint8Array>({
                start(controller) {
                  stream = controller;
                },
              }),
            ),
          );
        }),
      );
      const mux = createChatEventMultiplexer({ lspPort: 8001 }, undefined, {
        connectTimeoutMs: 100,
        idleTimeoutMs: 200,
      });
      const healthy = {
        onEvent: vi.fn(),
        onError: vi.fn(),
        onConnected: vi.fn(),
      };
      const child = {
        onEvent: vi.fn(),
        onError: vi.fn(),
        onConnected: vi.fn(),
      };
      cleanups.push(
        mux.subscribe("healthy", healthy),
        mux.subscribe("child", child),
      );
      await vi.advanceTimersByTimeAsync(0);
      expect(healthy.onConnected).not.toHaveBeenCalled();
      const send = (event: object) =>
        stream.enqueue(
          new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`),
        );
      if (missing === "incomplete chunked")
        send({
          type: "snapshot",
          chat_id: "child",
          seq: "1",
          snapshot_chunked: true,
          chunk_count: 1,
          message_count: 1,
          messages: [],
          background_agents: [],
        });
      send({
        type: "snapshot",
        chat_id: "healthy",
        seq: "1",
        messages: [],
        background_agents: [],
      });
      await vi.advanceTimersByTimeAsync(50);
      expect(healthy.onConnected).toHaveBeenCalledOnce();
      expect(child.onConnected).not.toHaveBeenCalled();
      stream.enqueue(new TextEncoder().encode(": heartbeat\n\n"));
      await vi.advanceTimersByTimeAsync(49);
      expect(child.onError).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(child.onError).toHaveBeenCalledExactlyOnceWith(
        expect.objectContaining({
          message: "SSE chat snapshot timeout",
        }),
      );
      expect(child.onConnected).not.toHaveBeenCalled();
      expect(child.onEvent).not.toHaveBeenCalled();
      expect(signal.aborted).toBe(false);
      expect(healthy.onError).not.toHaveBeenCalled();
      cleanups.forEach((cleanup) => cleanup());
      expect(signal.aborted).toBe(true);
      expect(vi.getTimerCount()).toBe(0);
    },
  );

  it("reports overflow asynchronously without throwing or opening another transport", async () => {
    vi.useFakeTimers();
    const fetcher = vi.fn(() => new Promise<Response>(() => undefined));
    vi.stubGlobal("fetch", fetcher);
    const mux = createChatEventMultiplexer({ lspPort: 8001 });
    for (let id = 0; id < 128; id++) {
      cleanups.push(
        mux.subscribe(String(id), { onEvent: vi.fn(), onError: vi.fn() }),
      );
    }
    const onError = vi.fn();
    expect(() =>
      cleanups.push(mux.subscribe("overflow", { onEvent: vi.fn(), onError })),
    ).not.toThrow();
    expect(onError).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(0);
    expect(onError).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({
        message: "Too many visible chat subscriptions",
      }),
    );
    expect(fetcher).toHaveBeenCalledTimes(1);
    cleanups.forEach((cleanup) => cleanup());
    expect(vi.getTimerCount()).toBe(0);
  });

  it("shares one fetch, isolates interleaved snapshots, and fences retired pending reads", async () => {
    vi.useFakeTimers();
    const streams: ReadableStreamDefaultController<Uint8Array>[] = [];
    const signals: AbortSignal[] = [];
    const fetcher = vi.fn((_url: string, init: RequestInit) => {
      if (!init.signal) throw new Error("Missing abort signal");
      signals.push(init.signal);
      return Promise.resolve(
        new Response(
          new ReadableStream<Uint8Array>({
            start(controller) {
              streams.push(controller);
            },
          }),
        ),
      );
    });
    vi.stubGlobal("fetch", fetcher);
    const mux = createChatEventMultiplexer({ lspPort: 8001 });
    const a = vi.fn<(event: ChatEventEnvelope) => void>();
    const b = vi.fn<(event: ChatEventEnvelope) => void>();
    const duplicate = vi.fn<(event: ChatEventEnvelope) => void>();
    const errors = vi.fn();
    const removeA = mux.subscribe("a", { onEvent: a, onError: errors });
    const removeB = mux.subscribe("b", { onEvent: b, onError: errors });
    const removeDuplicate = mux.subscribe("a", {
      onEvent: duplicate,
      onError: errors,
    });
    cleanups.push(removeA, removeB, removeDuplicate);
    await vi.advanceTimersByTimeAsync(0);
    expect(fetcher).toHaveBeenCalledTimes(1);
    const url = new URL(fetcher.mock.calls[0][0], "http://localhost");
    expect(url.pathname).toBe("/v1/chats/subscribe-many");
    expect(JSON.parse(url.searchParams.get("chat_ids") ?? "null")).toEqual([
      "a",
      "b",
    ]);
    const send = (index: number, event: unknown) =>
      streams[index].enqueue(
        new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`),
      );
    const snapshot = (chat_id: string, seq: string) => ({
      type: "snapshot",
      chat_id,
      seq,
      snapshot_chunked: true,
      chunk_count: 1,
      message_count: 1,
      background_agents: [],
    });
    send(0, snapshot("a", "10"));
    send(0, snapshot("b", "90"));
    send(0, {
      type: "snapshot_chunk",
      chat_id: "b",
      seq: "90",
      chunk_index: 0,
      chunk_count: 1,
      messages: [{ role: "user", content: "B" }],
    });
    send(0, {
      type: "snapshot_chunk",
      chat_id: "a",
      seq: "10",
      chunk_index: 0,
      chunk_count: 1,
      messages: [{ role: "user", content: "A" }],
    });
    send(0, {
      type: "snapshot_end",
      chat_id: "b",
      seq: "90",
      chunk_count: 1,
      message_count: 1,
    });
    send(0, {
      type: "snapshot_end",
      chat_id: "a",
      seq: "10",
      chunk_count: 1,
      message_count: 1,
    });
    send(0, {
      type: "stream_started",
      chat_id: "unknown",
      seq: "11",
      message_id: "wrong",
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(a.mock.calls.map(([event]) => event)).toEqual([
      expect.objectContaining({
        chat_id: "a",
        seq: "10",
        messages: [{ role: "user", content: "A" }],
      }),
    ]);
    expect(b).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({
        chat_id: "b",
        seq: "90",
        messages: [{ role: "user", content: "B" }],
      }),
    );
    expect(duplicate.mock.calls).toEqual(a.mock.calls);
    removeA();
    expect(signals[0].aborted).toBe(false);
    removeDuplicate();
    expect(signals[0].aborted).toBe(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(fetcher).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(100);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(signals[0].aborted).toBe(true);
    send(0, {
      type: "stream_started",
      chat_id: "b",
      seq: "91",
      message_id: "retired",
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(b).toHaveBeenCalledTimes(1);
    send(1, {
      type: "snapshot",
      chat_id: "b",
      seq: "100",
      messages: [],
      background_agents: [],
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(b).toHaveBeenLastCalledWith(expect.objectContaining({ seq: "100" }));
    removeB();
    expect(signals[1].aborted).toBe(true);
    streams[1].close();
    await vi.advanceTimersByTimeAsync(0);
    expect(errors).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("multiplex per-chat isolation", () => {
  type Consumer = {
    onEvent: ReturnType<typeof vi.fn<(event: ChatEventEnvelope) => void>>;
    onError: ReturnType<typeof vi.fn<(error: Error) => void>>;
    onConnected: ReturnType<typeof vi.fn<() => void>>;
    onDisconnected: ReturnType<typeof vi.fn<() => void>>;
  };
  let cleanups: (() => void)[] = [];
  const consumer = (): Consumer => ({
    onEvent: vi.fn<(event: ChatEventEnvelope) => void>(),
    onError: vi.fn<(error: Error) => void>(),
    onConnected: vi.fn<() => void>(),
    onDisconnected: vi.fn<() => void>(),
  });
  const setup = () => {
    vi.useFakeTimers();
    const streams: ReadableStreamDefaultController<Uint8Array>[] = [];
    const signals: AbortSignal[] = [];
    const fetcher = vi.fn((_url: string, init: RequestInit) => {
      if (!init.signal) throw new Error("Missing abort signal");
      signals.push(init.signal);
      return Promise.resolve(
        new Response(
          new ReadableStream<Uint8Array>({
            start(controller) {
              streams.push(controller);
            },
          }),
        ),
      );
    });
    vi.stubGlobal("fetch", fetcher);
    const mux = createChatEventMultiplexer({ lspPort: 8001 }, undefined, {
      connectTimeoutMs: 100,
      idleTimeoutMs: 10_000,
    });
    const send = (index: number, event: object) =>
      streams[index].enqueue(
        new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`),
      );
    const snapshot = (chat_id: string, seq = "1") => ({
      type: "snapshot",
      chat_id,
      seq,
      messages: [],
      background_agents: [],
    });
    const queue = (chat_id: string, seq: string) => ({
      type: "queue_updated",
      chat_id,
      seq,
      queue_size: 0,
      queued_items: [],
    });
    return { fetcher, signals, streams, mux, send, snapshot, queue };
  };
  afterEach(() => {
    cleanups.forEach((cleanup) => cleanup());
    cleanups = [];
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("sends the per_chat_close opt-in", async () => {
    const { fetcher, mux } = setup();
    cleanups.push(mux.subscribe("a", consumer()));
    await vi.advanceTimersByTimeAsync(0);
    const url = new URL(fetcher.mock.calls[0][0], "http://localhost");
    expect(url.searchParams.get("per_chat_close")).toBe("1");
    expect(url.searchParams.get("snapshot_chunks")).toBe("1");
  });

  it("fails only the chat missing its snapshot and keeps the shared fetch", async () => {
    const { fetcher, signals, mux, send, snapshot, queue } = setup();
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a), mux.subscribe("b", b));
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    await vi.advanceTimersByTimeAsync(100);
    expect(b.onError).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ message: "SSE chat snapshot timeout" }),
    );
    expect(a.onError).not.toHaveBeenCalled();
    expect(signals[0].aborted).toBe(false);
    send(0, queue("a", "2"));
    await vi.advanceTimersByTimeAsync(0);
    expect(a.onEvent).toHaveBeenLastCalledWith(
      expect.objectContaining({ type: "queue_updated", seq: "2" }),
    );

    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    await vi.advanceTimersByTimeAsync(200);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(b2.onError).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ message: "SSE chat snapshot timeout" }),
    );
    const b3 = consumer();
    cleanups.push(mux.subscribe("b", b3));
    await vi.advanceTimersByTimeAsync(50);
    send(0, snapshot("b", "7"));
    await vi.advanceTimersByTimeAsync(0);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(b3.onConnected).toHaveBeenCalledOnce();
    expect(b3.onEvent).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ type: "snapshot", chat_id: "b", seq: "7" }),
    );
    await vi.advanceTimersByTimeAsync(200);
    expect(b3.onError).not.toHaveBeenCalled();
    expect(b.onEvent).not.toHaveBeenCalled();
  });

  it("handles subscription_closed for one chat without aborting the others", async () => {
    const { fetcher, signals, mux, send, snapshot, queue } = setup();
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a), mux.subscribe("b", b));
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    send(0, snapshot("b"));
    send(0, {
      chat_id: "b",
      type: "subscription_closed",
      reason: "ended",
      retryable: false,
      message: "Chat was deleted",
      extra_field: { ignored: true },
    });
    send(0, queue("b", "2"));
    send(0, queue("a", "2"));
    await vi.advanceTimersByTimeAsync(0);
    expect(b.onError).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({
        message: "Chat was deleted",
        retryable: false,
        reason: "ended",
      }),
    );
    expect(b.onEvent).toHaveBeenCalledOnce();
    expect(a.onError).not.toHaveBeenCalled();
    expect(a.onEvent).toHaveBeenLastCalledWith(
      expect.objectContaining({ type: "queue_updated", seq: "2" }),
    );
    expect(signals[0].aborted).toBe(false);

    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    await vi.advanceTimersByTimeAsync(100);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(signals[0].aborted).toBe(true);
  });

  it("fails a chat closed before bootstrap immediately with a default message", async () => {
    const { signals, mux, send, snapshot } = setup();
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a), mux.subscribe("b", b));
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    send(0, {
      chat_id: "b",
      type: "subscription_closed",
      reason: "unavailable",
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(b.onError).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ message: "Chat subscription closed" }),
    );
    await vi.advanceTimersByTimeAsync(500);
    expect(b.onError).toHaveBeenCalledOnce();
    expect(a.onError).not.toHaveBeenCalled();
    expect(signals[0].aborted).toBe(false);
  });

  it("discards a partial chunked snapshot when its chat is closed", async () => {
    const { mux, send, snapshot, streams } = setup();
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a), mux.subscribe("b", b));
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    send(0, {
      type: "snapshot",
      chat_id: "b",
      seq: "5",
      snapshot_chunked: true,
      chunk_count: 2,
      message_count: 2,
      background_agents: [],
    });
    send(0, {
      type: "snapshot_chunk",
      chat_id: "b",
      seq: "5",
      chunk_index: 0,
      chunk_count: 2,
      messages: [{ role: "user", content: "x" }],
    });
    send(0, { chat_id: "b", type: "subscription_closed", reason: "ended" });
    await vi.advanceTimersByTimeAsync(0);
    expect(b.onError).toHaveBeenCalledOnce();
    streams[0].close();
    await vi.advanceTimersByTimeAsync(0);
    expect(a.onError).not.toHaveBeenCalled();
    expect(a.onDisconnected).toHaveBeenCalledOnce();
    expect(b.onEvent).not.toHaveBeenCalled();
  });

  it("coalesces an unsubscribe followed by a subscribe into one rebuild", async () => {
    const { fetcher, signals, mux, send, snapshot } = setup();
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a));
    const removeB = mux.subscribe("b", b);
    await vi.advanceTimersByTimeAsync(0);
    expect(fetcher).toHaveBeenCalledTimes(1);
    send(0, snapshot("a"));
    send(0, snapshot("b"));
    await vi.advanceTimersByTimeAsync(0);
    removeB();
    await vi.advanceTimersByTimeAsync(50);
    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    expect(signals[0].aborted).toBe(false);
    await vi.advanceTimersByTimeAsync(29);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(signals[0].aborted).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(signals[0].aborted).toBe(true);
    send(1, snapshot("a", "2"));
    send(1, snapshot("b", "3"));
    await vi.advanceTimersByTimeAsync(0);
    expect(b2.onEvent).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ seq: "3" }),
    );
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(signals[1].aborted).toBe(false);
  });

  const chunkedB = (send: (index: number, event: object) => void) => {
    send(0, {
      type: "snapshot",
      chat_id: "b",
      seq: "5",
      snapshot_chunked: true,
      chunk_count: 2,
      message_count: 2,
      background_agents: [],
    });
    send(0, {
      type: "snapshot_chunk",
      chat_id: "b",
      seq: "5",
      chunk_index: 0,
      chunk_count: 2,
      messages: [{ role: "user", content: "x" }],
    });
  };

  it("ends normally despite a partial snapshot left by a chat that timed out", async () => {
    const { mux, send, snapshot, streams } = setup();
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a), mux.subscribe("b", b));
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    chunkedB(send);
    await vi.advanceTimersByTimeAsync(100);
    expect(b.onError).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ message: "SSE chat snapshot timeout" }),
    );
    streams[0].close();
    await vi.advanceTimersByTimeAsync(0);
    expect(a.onError).not.toHaveBeenCalled();
    expect(a.onDisconnected).toHaveBeenCalledOnce();
  });

  it("delivers a late chunked snapshot to a chat re-attached after its timeout", async () => {
    const { fetcher, mux, send, snapshot, streams } = setup();
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a), mux.subscribe("b", b));
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    chunkedB(send);
    await vi.advanceTimersByTimeAsync(100);
    expect(b.onError).toHaveBeenCalledOnce();
    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    send(0, {
      type: "snapshot_chunk",
      chat_id: "b",
      seq: "5",
      chunk_index: 1,
      chunk_count: 2,
      messages: [{ role: "user", content: "y" }],
    });
    send(0, {
      type: "snapshot_end",
      chat_id: "b",
      seq: "5",
      chunk_count: 2,
      message_count: 2,
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(b2.onEvent).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({
        type: "snapshot",
        messages: [
          { role: "user", content: "x" },
          { role: "user", content: "y" },
        ],
      }),
    );
    streams[0].close();
    await vi.advanceTimersByTimeAsync(0);
    expect(a.onError).not.toHaveBeenCalled();
    expect(b2.onError).not.toHaveBeenCalled();
    expect(b2.onDisconnected).toHaveBeenCalledOnce();
  });

  it("still fails when a re-attached chat's snapshot is cut off by the end of stream", async () => {
    const { mux, send, snapshot, streams } = setup();
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a), mux.subscribe("b", b));
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    chunkedB(send);
    await vi.advanceTimersByTimeAsync(100);
    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    streams[0].close();
    await vi.advanceTimersByTimeAsync(0);
    for (const c of [a, b2])
      expect(c.onError).toHaveBeenCalledExactlyOnceWith(
        expect.objectContaining({
          message:
            "SSE stream ended before chunked snapshot completed; reconnecting",
        }),
      );
  });

  it("starts each chat's bootstrap window when response headers arrive", async () => {
    vi.useFakeTimers();
    let stream!: ReadableStreamDefaultController<Uint8Array>;
    vi.stubGlobal(
      "fetch",
      vi.fn(
        () =>
          new Promise<Response>((resolve) => {
            setTimeout(() => {
              resolve(
                new Response(
                  new ReadableStream<Uint8Array>({
                    start(controller) {
                      stream = controller;
                    },
                  }),
                ),
              );
            }, 60);
          }),
      ),
    );
    const mux = createChatEventMultiplexer({ lspPort: 8001 }, undefined, {
      connectTimeoutMs: 100,
      idleTimeoutMs: 10_000,
    });
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a), mux.subscribe("b", b));
    await vi.advanceTimersByTimeAsync(60);
    stream.enqueue(
      new TextEncoder().encode(
        `data: ${JSON.stringify({
          type: "snapshot",
          chat_id: "a",
          seq: "1",
          messages: [],
          background_agents: [],
        })}\n\n`,
      ),
    );
    await vi.advanceTimersByTimeAsync(99);
    expect(b.onError).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(b.onError).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ message: "SSE chat snapshot timeout" }),
    );
    expect(a.onError).not.toHaveBeenCalled();
    expect(a.onConnected).toHaveBeenCalledOnce();
  });

  it("cancels a scheduled rebuild that is no longer needed", async () => {
    const { fetcher, signals, mux, send, snapshot, queue } = setup();
    const a = consumer();
    cleanups.push(mux.subscribe("a", a));
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    await vi.advanceTimersByTimeAsync(0);
    const removeB = mux.subscribe("b", consumer());
    await vi.advanceTimersByTimeAsync(40);
    removeB();
    await vi.advanceTimersByTimeAsync(500);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(signals[0].aborted).toBe(false);
    send(0, queue("a", "2"));
    await vi.advanceTimersByTimeAsync(0);
    expect(a.onEvent).toHaveBeenLastCalledWith(
      expect.objectContaining({ type: "queue_updated", seq: "2" }),
    );
    expect(a.onError).not.toHaveBeenCalled();
  });

  it("still fails every chat on a connection-level error", async () => {
    const { mux, send, snapshot, streams } = setup();
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a), mux.subscribe("b", b));
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    send(0, snapshot("b"));
    await vi.advanceTimersByTimeAsync(0);
    streams[0].error(new Error("network reset"));
    await vi.advanceTimersByTimeAsync(0);
    expect(a.onError).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ message: "network reset" }),
    );
    expect(b.onError).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ message: "network reset" }),
    );
  });

  it("parks a first snapshot that completes after a chat's bootstrap timeout and hands it to the next consumer", async () => {
    const { fetcher, signals, mux, send, snapshot, queue } = setup();
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a), mux.subscribe("b", b));
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    await vi.advanceTimersByTimeAsync(100);
    expect(b.onError).toHaveBeenCalledOnce();
    chunkedB(send);
    send(0, {
      type: "snapshot_chunk",
      chat_id: "b",
      seq: "5",
      chunk_index: 1,
      chunk_count: 2,
      messages: [{ role: "user", content: "y" }],
    });
    send(0, {
      type: "snapshot_end",
      chat_id: "b",
      seq: "5",
      chunk_count: 2,
      message_count: 2,
    });
    send(0, queue("b", "6"));
    send(0, queue("a", "2"));
    send(0, queue("b", "7"));
    await vi.advanceTimersByTimeAsync(2000);
    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    await vi.advanceTimersByTimeAsync(0);
    send(0, queue("b", "8"));
    await vi.advanceTimersByTimeAsync(500);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(signals[0].aborted).toBe(false);
    expect(b2.onConnected).toHaveBeenCalledOnce();
    expect(
      b2.onEvent.mock.calls.map(([event]) => [event.type, event.seq]),
    ).toEqual([
      ["snapshot", "5"],
      ["queue_updated", "6"],
      ["queue_updated", "7"],
      ["queue_updated", "8"],
    ]);
    expect(b2.onEvent.mock.calls[0][0]).toEqual(
      expect.objectContaining({
        messages: [
          { role: "user", content: "x" },
          { role: "user", content: "y" },
        ],
      }),
    );
    send(0, queue("a", "3"));
    await vi.advanceTimersByTimeAsync(0);
    expect(a.onEvent).toHaveBeenLastCalledWith(
      expect.objectContaining({ type: "queue_updated", seq: "3" }),
    );
    expect(a.onError).not.toHaveBeenCalled();
    expect(b2.onError).not.toHaveBeenCalled();
    expect(b.onEvent).not.toHaveBeenCalled();
  });

  it("parks a first snapshot that arrives between an unsubscribe and a resubscribe", async () => {
    const { fetcher, signals, mux, send, snapshot, queue } = setup();
    const a = consumer();
    cleanups.push(mux.subscribe("a", a));
    const removeB = mux.subscribe("b", consumer());
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    await vi.advanceTimersByTimeAsync(0);
    removeB();
    send(0, snapshot("b", "4"));
    send(0, queue("b", "5"));
    await vi.advanceTimersByTimeAsync(50);
    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(signals[0].aborted).toBe(false);
    expect(
      b2.onEvent.mock.calls.map(([event]) => [event.type, event.seq]),
    ).toEqual([
      ["snapshot", "4"],
      ["queue_updated", "5"],
    ]);
    send(0, queue("a", "2"));
    await vi.advanceTimersByTimeAsync(0);
    expect(a.onEvent).toHaveBeenLastCalledWith(
      expect.objectContaining({ type: "queue_updated", seq: "2" }),
    );
  });

  it("falls back to a rebuild when parked events exceed the bound", async () => {
    const { fetcher, signals, mux, send, snapshot, queue } = setup();
    const a = consumer();
    cleanups.push(mux.subscribe("a", a));
    const removeB = mux.subscribe("b", consumer());
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    await vi.advanceTimersByTimeAsync(0);
    removeB();
    send(0, snapshot("b", "1"));
    for (let seq = 2; seq <= 300; seq++) send(0, queue("b", String(seq)));
    await vi.advanceTimersByTimeAsync(10);
    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    await vi.advanceTimersByTimeAsync(100);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(signals[0].aborted).toBe(true);
    expect(b2.onEvent).not.toHaveBeenCalled();
    send(1, snapshot("b", "400"));
    await vi.advanceTimersByTimeAsync(0);
    expect(b2.onEvent).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ type: "snapshot", seq: "400" }),
    );
  });

  const parkAfterTimeout = async (ids: string[]) => {
    const env = setup();
    const a = consumer();
    cleanups.push(env.mux.subscribe("a", a));
    for (const id of ids) cleanups.push(env.mux.subscribe(id, consumer()));
    await vi.advanceTimersByTimeAsync(0);
    env.send(0, env.snapshot("a"));
    await vi.advanceTimersByTimeAsync(100);
    return { ...env, a };
  };

  it("rebuilds for a consumer whose parked data overflows before the handover", async () => {
    const { fetcher, signals, mux, send, snapshot, queue, streams, a } =
      await parkAfterTimeout(["b"]);
    send(0, snapshot("b", "1"));
    for (let seq = 2; seq <= 251; seq++) send(0, queue("b", String(seq)));
    await vi.advanceTimersByTimeAsync(10);
    const burst = Array.from(
      { length: 10 },
      (_, index) =>
        `data: ${JSON.stringify(queue("b", String(252 + index)))}\n\n`,
    ).join("");
    streams[0].enqueue(new TextEncoder().encode(burst));
    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    await vi.advanceTimersByTimeAsync(100);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(signals[0].aborted).toBe(true);
    expect(b2.onEvent).not.toHaveBeenCalled();
    send(1, snapshot("b", "500"));
    send(1, snapshot("a", "500"));
    await vi.advanceTimersByTimeAsync(0);
    expect(b2.onEvent).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ type: "snapshot", seq: "500" }),
    );
    expect(a.onError).not.toHaveBeenCalled();
  });

  it("rebuilds for a consumer that throws during the handover and keeps other chats flowing", async () => {
    const { fetcher, mux, send, snapshot, queue, a } = await parkAfterTimeout([
      "b",
    ]);
    send(0, snapshot("b", "4"));
    send(0, queue("b", "5"));
    send(0, queue("b", "6"));
    await vi.advanceTimersByTimeAsync(10);
    const b2 = consumer();
    let calls = 0;
    b2.onEvent.mockImplementation(() => {
      calls += 1;
      if (calls === 2) throw new Error("consumer failed");
    });
    cleanups.push(mux.subscribe("b", b2));
    send(0, queue("a", "2"));
    await vi.advanceTimersByTimeAsync(0);
    expect(a.onEvent).toHaveBeenLastCalledWith(
      expect.objectContaining({ type: "queue_updated", seq: "2" }),
    );
    await vi.advanceTimersByTimeAsync(100);
    expect(fetcher).toHaveBeenCalledTimes(2);
    send(1, snapshot("b", "9"));
    send(1, snapshot("a", "9"));
    await vi.advanceTimersByTimeAsync(0);
    expect(b2.onEvent).toHaveBeenLastCalledWith(
      expect.objectContaining({ type: "snapshot", seq: "9" }),
    );
    expect(a.onEvent).toHaveBeenLastCalledWith(
      expect.objectContaining({ type: "snapshot", seq: "9" }),
    );
  });

  it("does not park beyond the total bound across chats", async () => {
    const ids = ["b", "c", "d", "e", "f"];
    const { fetcher, mux, send, snapshot } = await parkAfterTimeout(ids);
    const content = "x".repeat(7 * 1024 * 1024);
    for (const id of ids)
      send(0, {
        ...snapshot(id, "3"),
        messages: [{ role: "user", content }],
      });
    await vi.advanceTimersByTimeAsync(10);
    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    await vi.advanceTimersByTimeAsync(0);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(b2.onEvent).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ type: "snapshot", chat_id: "b", seq: "3" }),
    );
    const f2 = consumer();
    cleanups.push(mux.subscribe("f", f2));
    await vi.advanceTimersByTimeAsync(100);
    expect(f2.onEvent).not.toHaveBeenCalled();
    expect(fetcher).toHaveBeenCalledTimes(2);
  });

  it("releases parked data when the chat is closed or the connection stops", async () => {
    const { fetcher, mux, send, snapshot } = setup();
    const a = consumer();
    const removeA = mux.subscribe("a", a);
    const removeB = mux.subscribe("b", consumer());
    const removeC = mux.subscribe("c", consumer());
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    await vi.advanceTimersByTimeAsync(0);
    removeB();
    removeC();
    send(0, snapshot("b", "3"));
    send(0, snapshot("c", "3"));
    send(0, { chat_id: "c", type: "subscription_closed", reason: "ended" });
    await vi.advanceTimersByTimeAsync(10);
    const c2 = consumer();
    const removeC2 = mux.subscribe("c", c2);
    await vi.advanceTimersByTimeAsync(0);
    expect(c2.onEvent).not.toHaveBeenCalled();
    removeC2();
    removeA();
    await vi.advanceTimersByTimeAsync(0);
    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    await vi.advanceTimersByTimeAsync(0);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(b2.onEvent).not.toHaveBeenCalled();
  });

  it("accepts snapshot chunks for a timed-out chat that has no consumer without disturbing others", async () => {
    const { fetcher, signals, mux, send, snapshot, queue, streams } = setup();
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a), mux.subscribe("b", b));
    await vi.advanceTimersByTimeAsync(0);
    send(0, snapshot("a"));
    chunkedB(send);
    await vi.advanceTimersByTimeAsync(100);
    expect(b.onError).toHaveBeenCalledOnce();
    send(0, {
      type: "snapshot_chunk",
      chat_id: "b",
      seq: "5",
      chunk_index: 1,
      chunk_count: 2,
      messages: [{ role: "user", content: "y" }],
    });
    send(0, queue("a", "2"));
    send(0, {
      type: "snapshot_end",
      chat_id: "b",
      seq: "5",
      chunk_count: 2,
      message_count: 2,
    });
    send(0, queue("a", "3"));
    await vi.advanceTimersByTimeAsync(0);
    expect(a.onEvent.mock.calls.map(([event]) => event.seq)).toEqual([
      "1",
      "2",
      "3",
    ]);
    expect(b.onEvent).not.toHaveBeenCalled();
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(signals[0].aborted).toBe(false);
    streams[0].close();
    await vi.advanceTimersByTimeAsync(0);
    expect(a.onError).not.toHaveBeenCalled();
    expect(a.onDisconnected).toHaveBeenCalledOnce();
  });

  const delayedHeaders = (delayMs: number | null) => {
    vi.useFakeTimers();
    let stream!: ReadableStreamDefaultController<Uint8Array>;
    const signals: AbortSignal[] = [];
    const fetcher = vi.fn(
      (_url: string, init: RequestInit) =>
        new Promise<Response>((resolve) => {
          if (init.signal) signals.push(init.signal);
          if (delayMs === null) return;
          setTimeout(() => {
            resolve(
              new Response(
                new ReadableStream<Uint8Array>({
                  start(controller) {
                    stream = controller;
                  },
                }),
              ),
            );
          }, delayMs);
        }),
    );
    vi.stubGlobal("fetch", fetcher);
    const mux = createChatEventMultiplexer({ lspPort: 8001 }, undefined, {
      connectTimeoutMs: 100,
      idleTimeoutMs: 10_000,
    });
    const sendSnapshot = (chat_id: string) =>
      stream.enqueue(
        new TextEncoder().encode(
          `data: ${JSON.stringify({
            type: "snapshot",
            chat_id,
            seq: "1",
            messages: [],
            background_agents: [],
          })}\n\n`,
        ),
      );
    return { fetcher, signals, mux, sendSnapshot };
  };

  it("re-attaches a chat resubscribed before headers and gives it a full window after them", async () => {
    const { fetcher, signals, mux, sendSnapshot } = delayedHeaders(60);
    const a = consumer();
    cleanups.push(mux.subscribe("a", a));
    const removeB = mux.subscribe("b", consumer());
    await vi.advanceTimersByTimeAsync(10);
    removeB();
    await vi.advanceTimersByTimeAsync(10);
    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    await vi.advanceTimersByTimeAsync(40);
    sendSnapshot("a");
    await vi.advanceTimersByTimeAsync(99);
    expect(b2.onError).not.toHaveBeenCalled();
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(signals[0].aborted).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(b2.onError).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ message: "SSE chat snapshot timeout" }),
    );
    expect(a.onError).not.toHaveBeenCalled();
  });

  it("keeps a chat resubscribed before headers under the connect timeout", async () => {
    const { fetcher, signals, mux } = delayedHeaders(null);
    const a = consumer();
    cleanups.push(mux.subscribe("a", a));
    const removeB = mux.subscribe("b", consumer());
    await vi.advanceTimersByTimeAsync(10);
    removeB();
    await vi.advanceTimersByTimeAsync(10);
    const b2 = consumer();
    cleanups.push(mux.subscribe("b", b2));
    await vi.advanceTimersByTimeAsync(79);
    expect(b2.onError).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    for (const c of [a, b2])
      expect(c.onError).toHaveBeenCalledExactlyOnceWith(
        expect.objectContaining({ message: "SSE connect timeout" }),
      );
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(signals[0].aborted).toBe(true);
  });

  it("keeps the connect timeout connection-wide before headers", async () => {
    vi.useFakeTimers();
    const signals: AbortSignal[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((_url: string, init: RequestInit) => {
        if (init.signal) signals.push(init.signal);
        return new Promise<Response>(() => undefined);
      }),
    );
    const mux = createChatEventMultiplexer({ lspPort: 8001 }, undefined, {
      connectTimeoutMs: 100,
    });
    const a = consumer();
    const b = consumer();
    cleanups.push(mux.subscribe("a", a), mux.subscribe("b", b));
    await vi.advanceTimersByTimeAsync(100);
    for (const c of [a, b])
      expect(c.onError).toHaveBeenCalledExactlyOnceWith(
        expect.objectContaining({ message: "SSE connect timeout" }),
      );
    expect(signals[0].aborted).toBe(true);
  });
});
