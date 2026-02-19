import { describe, it, expect, beforeEach } from "vitest";
import { chatReducer } from "../features/Chat/Thread/reducer";
import { newChatAction, applyChatEvent } from "../features/Chat/Thread/actions";
import type { Chat } from "../features/Chat/Thread/types";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";
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
    messages,
  };
}

function makeHistory(count: number): ChatMessage[] {
  return Array.from({ length: count }, (_, i) =>
    i % 2 === 0
      ? { role: "user", content: `user-${i}`, message_id: `u-${i}` }
      : { role: "assistant", content: `assistant-${i}`, message_id: `a-${i}` },
  );
}

describe("Multi-Chat Streaming Stress Tests", () => {
  let baseState: Chat;

  beforeEach(() => {
    const emptyState = chatReducer(undefined, { type: "@@INIT" });
    baseState = chatReducer(emptyState, newChatAction(undefined));
  });

  it("handles 3 concurrent streaming chats without data loss", () => {
    const CHAT_COUNT = 3;
    const HISTORY_SIZE = 1200;
    const CHUNKS_PER_CHAT = 500;
    const CHUNK_TEXT = "Hello world streaming text. ";

    const chatIds: string[] = [];
    let state = baseState;

    for (let c = 0; c < CHAT_COUNT; c++) {
      state = chatReducer(state, newChatAction(undefined));
      chatIds.push(state.current_thread_id);
    }

    for (const chatId of chatIds) {
      const snapshot = createSnapshotEvent(chatId, makeHistory(HISTORY_SIZE));
      state = chatReducer(state, applyChatEvent(snapshot));
    }

    for (const chatId of chatIds) {
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "2",
          type: "stream_started",
          message_id: `stream-${chatId}`,
        }),
      );
    }

    for (const chatId of chatIds) {
      const rt = state.threads[chatId];
      if (!rt) throw new Error(`Runtime not found for chat ${chatId}`);
      expect(rt.streaming).toBe(true);
      expect(rt.waiting_for_response).toBe(true);
    }

    const startedAt = Date.now();

    for (let i = 0; i < CHUNKS_PER_CHAT; i++) {
      for (const chatId of chatIds) {
        state = chatReducer(
          state,
          applyChatEvent({
            chat_id: chatId,
            seq: String(i + 3),
            type: "stream_delta",
            message_id: `stream-${chatId}`,
            ops: [{ op: "append_content", text: CHUNK_TEXT }],
          }),
        );
      }
    }

    const streamElapsedMs = Date.now() - startedAt;

    for (const chatId of chatIds) {
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: String(CHUNKS_PER_CHAT + 3),
          type: "stream_finished",
          message_id: `stream-${chatId}`,
          finish_reason: "stop",
        }),
      );
    }

    for (const chatId of chatIds) {
      const rt = state.threads[chatId];
      if (!rt) throw new Error(`Runtime not found for chat ${chatId}`);
      const msgs = rt.thread.messages;
      expect(msgs).toHaveLength(HISTORY_SIZE + 1);

      const lastMsg = msgs[msgs.length - 1];
      expect(lastMsg.role).toBe("assistant");
      expect(lastMsg.content).toBe(CHUNK_TEXT.repeat(CHUNKS_PER_CHAT));

      expect(rt.streaming).toBe(false);
      expect(rt.waiting_for_response).toBe(false);
      expect(rt.snapshot_received).toBe(true);
    }

    expect(streamElapsedMs).toBeLessThan(15_000);
  });

  it("handles interleaved deltas with reasoning + tool_calls across 3 chats", () => {
    const CHAT_COUNT = 3;
    const HISTORY_SIZE = 200;
    const CHUNKS = 100;

    const chatIds: string[] = [];
    let state = baseState;

    for (let c = 0; c < CHAT_COUNT; c++) {
      state = chatReducer(state, newChatAction(undefined));
      chatIds.push(state.current_thread_id);
    }

    for (const chatId of chatIds) {
      state = chatReducer(
        state,
        applyChatEvent(createSnapshotEvent(chatId, makeHistory(HISTORY_SIZE))),
      );
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "2",
          type: "stream_started",
          message_id: `stream-${chatId}`,
        }),
      );
    }

    for (let i = 0; i < CHUNKS; i++) {
      for (const chatId of chatIds) {
        state = chatReducer(
          state,
          applyChatEvent({
            chat_id: chatId,
            seq: String(i * 2 + 3),
            type: "stream_delta",
            message_id: `stream-${chatId}`,
            ops: [
              { op: "append_content", text: `c${i} ` },
              { op: "append_reasoning", text: `r${i} ` },
            ],
          }),
        );

        if (i === CHUNKS - 1) {
          state = chatReducer(
            state,
            applyChatEvent({
              chat_id: chatId,
              seq: String(i * 2 + 4),
              type: "stream_delta",
              message_id: `stream-${chatId}`,
              ops: [
                {
                  op: "set_tool_calls",
                  tool_calls: [
                    {
                      id: `tc-${chatId}`,
                      type: "function",
                      function: {
                        name: "cat",
                        arguments: '{"paths":"test.ts"}',
                      },
                    },
                  ],
                },
              ],
            }),
          );
        }
      }
    }

    for (const chatId of chatIds) {
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: String(CHUNKS * 2 + 5),
          type: "stream_finished",
          message_id: `stream-${chatId}`,
          finish_reason: "tool_calls",
        }),
      );
    }

    for (const chatId of chatIds) {
      const rt = state.threads[chatId];
      if (!rt) throw new Error(`Runtime not found for chat ${chatId}`);
      const lastMsg = rt.thread.messages[rt.thread.messages.length - 1];

      const expectedContent = Array.from(
        { length: CHUNKS },
        (_, i) => `c${i} `,
      ).join("");
      expect(lastMsg.content).toBe(expectedContent);

      if ("reasoning_content" in lastMsg) {
        const expectedReasoning = Array.from(
          { length: CHUNKS },
          (_, i) => `r${i} `,
        ).join("");
        expect(lastMsg.reasoning_content).toBe(expectedReasoning);
      }

      if ("tool_calls" in lastMsg && lastMsg.tool_calls) {
        expect(lastMsg.tool_calls).toHaveLength(1);
        expect(lastMsg.tool_calls[0].id).toBe(`tc-${chatId}`);
      }
    }
  });

  it("handles large batched ops (coalesced deltas) correctly", () => {
    let state = baseState;
    state = chatReducer(state, newChatAction(undefined));
    const chatId = state.current_thread_id;

    state = chatReducer(
      state,
      applyChatEvent(createSnapshotEvent(chatId, makeHistory(100))),
    );
    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq: "2",
        type: "stream_started",
        message_id: "stream-batch",
      }),
    );

    const batchedOps = Array.from({ length: 200 }, (_, i) => ({
      op: "append_content" as const,
      text: `chunk${i}-`,
    }));

    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq: "3",
        type: "stream_delta",
        message_id: "stream-batch",
        ops: batchedOps,
      }),
    );

    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq: "4",
        type: "stream_finished",
        message_id: "stream-batch",
        finish_reason: "stop",
      }),
    );

    const rt = state.threads[chatId];
    if (!rt) throw new Error(`Runtime not found for chat ${chatId}`);
    const lastMsg = rt.thread.messages[rt.thread.messages.length - 1];
    const expectedContent = Array.from(
      { length: 200 },
      (_, i) => `chunk${i}-`,
    ).join("");
    expect(lastMsg.content).toBe(expectedContent);
  });

  it("correctly skips duplicate seq events across all 3 chats", () => {
    const CHAT_COUNT = 3;
    const chatIds: string[] = [];
    let state = baseState;

    for (let c = 0; c < CHAT_COUNT; c++) {
      state = chatReducer(state, newChatAction(undefined));
      chatIds.push(state.current_thread_id);
    }

    for (const chatId of chatIds) {
      state = chatReducer(
        state,
        applyChatEvent(
          createSnapshotEvent(chatId, [
            { role: "user", content: "hi", message_id: "u1" },
          ]),
        ),
      );
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "2",
          type: "stream_started",
          message_id: `s-${chatId}`,
        }),
      );
    }

    for (const chatId of chatIds) {
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "3",
          type: "stream_delta",
          message_id: `s-${chatId}`,
          ops: [{ op: "append_content", text: "real" }],
        }),
      );

      for (let dup = 0; dup < 50; dup++) {
        state = chatReducer(
          state,
          applyChatEvent({
            chat_id: chatId,
            seq: "3",
            type: "stream_delta",
            message_id: `s-${chatId}`,
            ops: [{ op: "append_content", text: "_dup" }],
          }),
        );
      }
    }

    for (const chatId of chatIds) {
      const rt = state.threads[chatId];
      if (!rt) throw new Error(`Runtime not found for chat ${chatId}`);
      const lastMsg = rt.thread.messages[rt.thread.messages.length - 1];
      expect(lastMsg.content).toBe("real");
      expect(rt.last_applied_seq).toBe("3");
    }
  });

  it("handles snapshot mid-stream (reconnect scenario) for one of 3 chats", () => {
    const CHAT_COUNT = 3;
    const chatIds: string[] = [];
    let state = baseState;

    for (let c = 0; c < CHAT_COUNT; c++) {
      state = chatReducer(state, newChatAction(undefined));
      chatIds.push(state.current_thread_id);
    }

    for (const chatId of chatIds) {
      state = chatReducer(
        state,
        applyChatEvent(createSnapshotEvent(chatId, makeHistory(50))),
      );
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "2",
          type: "stream_started",
          message_id: `s-${chatId}`,
        }),
      );
      for (let i = 0; i < 10; i++) {
        state = chatReducer(
          state,
          applyChatEvent({
            chat_id: chatId,
            seq: String(i + 3),
            type: "stream_delta",
            message_id: `s-${chatId}`,
            ops: [{ op: "append_content", text: "x" }],
          }),
        );
      }
    }

    const reconnectChatId = chatIds[1];
    const freshMessages: ChatMessage[] = [
      ...makeHistory(50),
      {
        role: "assistant",
        content: "full recovered content",
        message_id: `s-${reconnectChatId}`,
      },
    ];

    const reconnectSnapshot = createSnapshotEvent(
      reconnectChatId,
      freshMessages,
      "0",
    );
    if (reconnectSnapshot.type === "snapshot") {
      reconnectSnapshot.runtime.state = "generating";
    }
    state = chatReducer(state, applyChatEvent(reconnectSnapshot));

    const reconnectedRt = state.threads[reconnectChatId];
    if (!reconnectedRt)
      throw new Error(`Runtime not found for chat ${reconnectChatId}`);
    expect(reconnectedRt.thread.messages).toHaveLength(51);
    expect(
      reconnectedRt.thread.messages[reconnectedRt.thread.messages.length - 1]
        .content,
    ).toBe("full recovered content");
    expect(reconnectedRt.streaming).toBe(true);

    for (const chatId of chatIds) {
      if (chatId === reconnectChatId) continue;
      const rt = state.threads[chatId];
      if (!rt) throw new Error(`Runtime not found for chat ${chatId}`);
      expect(rt.streaming).toBe(true);
      const lastMsg = rt.thread.messages[rt.thread.messages.length - 1];
      expect(lastMsg.content).toBe("x".repeat(10));
    }
  });
});
