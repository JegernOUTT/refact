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
    background_agents: [],
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

  it("handles every required concurrent chat level without data loss", () => {
    const historySize = 120;
    const chunksPerChat = 24;
    const chunkText = "Hello world streaming text. ";

    for (const chatCount of [1, 4, 8, 16, 32]) {
      const chatIds: string[] = [];
      let state = baseState;

      for (let chat = 0; chat < chatCount; chat++) {
        state = chatReducer(state, newChatAction(undefined));
        chatIds.push(state.current_thread_id);
      }

      for (const chatId of chatIds) {
        state = chatReducer(
          state,
          applyChatEvent(createSnapshotEvent(chatId, makeHistory(historySize))),
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

      for (let chunk = 0; chunk < chunksPerChat; chunk++) {
        for (const chatId of chatIds) {
          state = chatReducer(
            state,
            applyChatEvent({
              chat_id: chatId,
              seq: String(chunk + 3),
              type: "stream_delta",
              message_id: `stream-${chatId}`,
              ops: [{ op: "append_content", text: chunkText }],
            }),
          );
        }
      }

      for (const chatId of chatIds) {
        state = chatReducer(
          state,
          applyChatEvent({
            chat_id: chatId,
            seq: String(chunksPerChat + 3),
            type: "stream_finished",
            message_id: `stream-${chatId}`,
            finish_reason: "stop",
          }),
        );
        const runtime = state.threads[chatId];
        if (!runtime) throw new Error(`Runtime not found for chat ${chatId}`);
        const lastMessage = runtime.thread.messages.at(-1);
        expect(runtime.thread.messages).toHaveLength(historySize + 1);
        expect(lastMessage?.role).toBe("assistant");
        expect(lastMessage?.content).toBe(chunkText.repeat(chunksPerChat));
        expect(runtime.streaming).toBe(false);
        expect(runtime.waiting_for_response).toBe(false);
        expect(runtime.snapshot_received).toBe(true);
      }
    }
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

  it("recovers each active/background mix from a sequence-gap snapshot", () => {
    for (const chatCount of [1, 4, 8, 16, 32]) {
      const chatIds: string[] = [];
      let state = baseState;

      for (let chat = 0; chat < chatCount; chat++) {
        state = chatReducer(state, newChatAction(undefined));
        chatIds.push(state.current_thread_id);
      }
      for (const chatId of chatIds) {
        state = chatReducer(
          state,
          applyChatEvent(createSnapshotEvent(chatId, makeHistory(4))),
        );
        state = chatReducer(
          state,
          applyChatEvent({
            chat_id: chatId,
            seq: "2",
            type: "stream_started",
            message_id: `gap-${chatId}`,
          }),
        );
        state = chatReducer(
          state,
          applyChatEvent({
            chat_id: chatId,
            seq: "3",
            type: "stream_delta",
            message_id: `gap-${chatId}`,
            ops: [{ op: "append_content", text: "before-gap" }],
          }),
        );
      }

      const recoveredChatId = chatIds[0];
      const recoveredMessages = [
        ...makeHistory(4),
        {
          role: "assistant" as const,
          content: "snapshot-recovered",
          message_id: `gap-${recoveredChatId}`,
        },
      ];
      const snapshot = createSnapshotEvent(recoveredChatId, recoveredMessages, "0");
      if (snapshot.type === "snapshot") snapshot.runtime.state = "generating";
      state = chatReducer(state, applyChatEvent(snapshot));

      const recoveredRuntime = state.threads[recoveredChatId];
      if (!recoveredRuntime) {
        throw new Error(`Runtime not found for chat ${recoveredChatId}`);
      }
      expect(recoveredRuntime.last_applied_seq).toBe("0");
      expect(recoveredRuntime.thread.messages.at(-1)?.content).toBe(
        "snapshot-recovered",
      );
      expect(recoveredRuntime.streaming).toBe(true);

      for (const backgroundChatId of chatIds.slice(1)) {
        const backgroundRuntime = state.threads[backgroundChatId];
        if (!backgroundRuntime) {
          throw new Error(`Runtime not found for chat ${backgroundChatId}`);
        }
        expect(backgroundRuntime.last_applied_seq).toBe("3");
        expect(backgroundRuntime.thread.messages.at(-1)?.content).toBe("before-gap");
        expect(backgroundRuntime.streaming).toBe(true);
      }
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
