import { describe, it, expect, vi, beforeEach } from "vitest";
import { chatReducer } from "../features/Chat/Thread/reducer";
import { newChatAction, applyChatEvent } from "../features/Chat/Thread/actions";
import type { Chat } from "../features/Chat/Thread/types";
import {
  subscribeToChatEvents,
  type ChatEventEnvelope,
  type EventEnvelope,
} from "../services/refact/chatSubscription";
import type { ChatMessage } from "../services/refact/types";

function createSnapshotEvent(
  chatId: string,
  messages: ChatMessage[],
  seq = "1",
): ChatEventEnvelope {
  return {
    chat_id: chatId,
    seq,
    type: "snapshot",
    thread: {
      id: chatId,
      title: "Stress Test",
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
      queued_items: [],
    },
    background_agents: [],
    messages,
  };
}

function createMockReader(chunks: Uint8Array[]) {
  let index = 0;
  return {
    read: vi.fn(() => {
      if (index >= chunks.length) {
        return Promise.resolve({ done: true, value: undefined });
      }
      return Promise.resolve({ done: false, value: chunks[index++] });
    }),
  };
}

function createMockFetch(chunks: Uint8Array[]) {
  return vi.fn().mockResolvedValue({
    ok: true,
    body: {
      getReader: () => createMockReader(chunks),
    },
  });
}

describe("Chat Streaming + Large History Stress", () => {
  let initialState: Chat;
  let chatId: string;

  beforeEach(() => {
    vi.clearAllMocks();
    const emptyState = chatReducer(undefined, { type: "@@INIT" });
    initialState = chatReducer(emptyState, newChatAction(undefined));
    chatId = initialState.current_thread_id;
  });

  it("handles large history plus many stream deltas", () => {
    const historySize = 1200;
    const chunkCount = 1500;
    const chunkText = "abcdefghijklmnopqrstuvwxyz";

    const messages: ChatMessage[] = Array.from(
      { length: historySize },
      (_, i) =>
        i % 2 === 0
          ? {
              role: "user",
              content: `user-${i}`,
              message_id: `u-${i}`,
            }
          : {
              role: "assistant",
              content: `assistant-${i}`,
              message_id: `a-${i}`,
            },
    );

    const snapshot = createSnapshotEvent(chatId, messages);
    let state = chatReducer(initialState, applyChatEvent(snapshot));

    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq: "2",
        type: "stream_started",
        message_id: "stream-stress",
      }),
    );

    const startedAt = Date.now();
    for (let i = 0; i < chunkCount; i++) {
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: String(i + 3),
          type: "stream_delta",
          message_id: "stream-stress",
          ops: [{ op: "append_content", text: chunkText }],
        }),
      );
    }
    const elapsedMs = Date.now() - startedAt;

    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq: String(chunkCount + 3),
        type: "stream_finished",
        message_id: "stream-stress",
        finish_reason: "stop",
      }),
    );

    const runtime = state.threads[chatId];
    if (!runtime) throw new Error(`Runtime not found for chat ${chatId}`);
    const finalMessage =
      runtime.thread.messages[runtime.thread.messages.length - 1];

    expect(runtime.thread.messages).toHaveLength(historySize + 1);
    expect(finalMessage.role).toBe("assistant");
    expect(finalMessage.content).toBe(chunkText.repeat(chunkCount));
    expect(runtime.streaming).toBe(false);
    expect(elapsedMs).toBeLessThan(10_000);
  });

  it("keeps reducer stable under many duplicate seq events", () => {
    const snapshot = createSnapshotEvent(chatId, [
      { role: "user", content: "hello", message_id: "u1" },
    ]);
    let state = chatReducer(initialState, applyChatEvent(snapshot));

    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq: "2",
        type: "stream_started",
        message_id: "msg-dup",
      }),
    );

    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq: "3",
        type: "stream_delta",
        message_id: "msg-dup",
        ops: [{ op: "append_content", text: "base" }],
      }),
    );

    for (let i = 0; i < 1000; i++) {
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "3",
          type: "stream_delta",
          message_id: "msg-dup",
          ops: [{ op: "append_content", text: "_duplicate_should_not_apply" }],
        }),
      );
    }

    const runtime = state.threads[chatId];
    if (!runtime) throw new Error(`Runtime not found for chat ${chatId}`);
    const finalMessage =
      runtime.thread.messages[runtime.thread.messages.length - 1];
    expect(finalMessage.content).toBe("base");
    expect(runtime.last_applied_seq).toBe("3");
  });

  it("parses many SSE events in a single payload", async () => {
    const eventCount = 2000;
    const encoder = new TextEncoder();
    const events: EventEnvelope[] = [];

    const payload = Array.from({ length: eventCount }, (_, i) => {
      const event: EventEnvelope = {
        chat_id: "stress-chat",
        seq: String(i + 1),
        type: "pause_cleared",
      };
      return `data: ${JSON.stringify(event)}\n\n`;
    }).join("");

    global.fetch = createMockFetch([encoder.encode(payload)]);

    subscribeToChatEvents(
      "stress-chat",
      { host: "vscode", lspPort: 8001 },
      {
        onEvent: (e) => events.push(e),
        onError: vi.fn(),
      },
    );

    await new Promise((resolve) => setTimeout(resolve, 40));

    expect(events).toHaveLength(eventCount);
    expect(events[0].seq).toBe("1");
    expect(events[eventCount - 1].seq).toBe(String(eventCount));
  });

  it("parses a 9 MiB initial snapshot split across chunks", async () => {
    const encoder = new TextEncoder();
    const largeContent = "x".repeat(9 * 1024 * 1024);
    const snapshot = createSnapshotEvent("large-snapshot-chat", [
      {
        role: "user",
        content: largeContent,
        message_id: "large-message",
      },
    ]);
    const payload = `data: ${JSON.stringify(snapshot)}\n\n`;
    const chunkSize = 64 * 1024;
    const chunks: Uint8Array[] = [];
    for (let offset = 0; offset < payload.length; offset += chunkSize) {
      chunks.push(encoder.encode(payload.slice(offset, offset + chunkSize)));
    }
    global.fetch = createMockFetch(chunks);

    const onError = vi.fn<(error: Error) => void>();
    let unsubscribe: () => void = () => undefined;
    const received = new Promise<EventEnvelope>((resolve, reject) => {
      unsubscribe = subscribeToChatEvents(
        "large-snapshot-chat",
        { host: "vscode", lspPort: 8001 },
        {
          onEvent: resolve,
          onError: (error) => {
            onError(error);
            reject(error);
          },
        },
      );
    });

    const event = await received;
    unsubscribe();

    expect(payload.length).toBeGreaterThan(8_000_000);
    expect(onError).not.toHaveBeenCalled();
    expect(event.type).toBe("snapshot");
    if (event.type !== "snapshot") throw new Error("Expected snapshot event");
    expect(event.messages).toHaveLength(1);
    expect(event.messages[0].content).toBe(largeContent);
  });
});
