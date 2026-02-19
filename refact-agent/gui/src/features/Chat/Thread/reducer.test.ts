/* eslint-disable @typescript-eslint/no-non-null-assertion */
import { expect, test, describe, beforeEach } from "vitest";
import { chatReducer } from "./reducer";
import type { Chat } from "./types";
import { newChatAction, applyChatEvent } from "./actions";
import type {
  ChatEventEnvelope,
  DeltaOp,
} from "../../../services/refact/chatSubscription";

describe("Chat Thread Reducer - Event-based (Stateless Trajectory UI)", () => {
  let initialState: Chat;
  let chatId: string;

  beforeEach(() => {
    const emptyState = chatReducer(undefined, { type: "@@INIT" });
    initialState = chatReducer(emptyState, newChatAction(undefined));
    chatId = initialState.current_thread_id;
  });

  describe("applyChatEvent - snapshot", () => {
    test("should initialize thread from snapshot event", () => {
      const event: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test Chat",
          model: "gpt-4",
          mode: "AGENT",
          tool_use: "agent",
          boost_reasoning: false,
          context_tokens_cap: 8192,
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
        messages: [
          { role: "user", content: "Hello" },
          { role: "assistant", content: "Hi there!" },
        ],
      };

      const result = chatReducer(initialState, applyChatEvent(event));
      const runtime = result.threads[chatId]!;

      expect(runtime).toBeDefined();
      expect(runtime.thread.title).toBe("Test Chat");
      expect(runtime.thread.model).toBe("gpt-4");
      expect(runtime.thread.messages).toHaveLength(2);
      expect(runtime.streaming).toBe(false);
      expect(runtime.waiting_for_response).toBe(false);
    });

    test("should handle snapshot with generating state", () => {
      const event: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
          state: "generating",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [],
      };

      const result = chatReducer(initialState, applyChatEvent(event));
      const runtime = result.threads[chatId]!;

      expect(runtime.streaming).toBe(true);
      expect(runtime.waiting_for_response).toBe(true);
    });

    test("should handle snapshot with paused state", () => {
      const event: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
          paused: true,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [],
      };

      const result = chatReducer(initialState, applyChatEvent(event));
      const runtime = result.threads[chatId]!;

      expect(runtime.confirmation.pause).toBe(true);
    });

    test("should handle snapshot with error state", () => {
      const event: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
          state: "error", // Must be "error" state for prevent_send to be true
          paused: false,
          error: "Something went wrong",
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [],
      };

      const result = chatReducer(initialState, applyChatEvent(event));
      const runtime = result.threads[chatId]!;

      expect(runtime.error).toBe("Something went wrong");
      // Allow sending even on error for recovery
      expect(runtime.prevent_send).toBe(false);
    });
  });

  describe("applyChatEvent - stream_delta", () => {
    test("should append content via delta ops", () => {
      // First set up a thread with an assistant message that has a message_id
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
          state: "generating",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [{ role: "user", content: "Hello" }],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      // Use stream_started to add assistant message with message_id
      const streamStartEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "2",
        type: "stream_started",
        message_id: "msg-1",
      };

      state = chatReducer(state, applyChatEvent(streamStartEvent));

      // Now apply a delta
      const deltaEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "3",
        type: "stream_delta",
        message_id: "msg-1",
        ops: [{ op: "append_content", text: "Hi there!" }],
      };

      state = chatReducer(state, applyChatEvent(deltaEvent));
      const runtime = state.threads[chatId]!;
      const lastMessage =
        runtime.thread.messages[runtime.thread.messages.length - 1];

      expect(lastMessage.content).toBe("Hi there!");
    });

    test("should handle reasoning content delta", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
          model: "gpt-4",
          mode: "AGENT",
          tool_use: "agent",
          boost_reasoning: true,
          context_tokens_cap: null,
          include_project_info: true,
          checkpoints_enabled: true,
          is_title_generated: false,
        },
        runtime: {
          state: "generating",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [{ role: "user", content: "Explain" }],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      // Use stream_started to add assistant message
      const streamStartEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "2",
        type: "stream_started",
        message_id: "msg-1",
      };

      state = chatReducer(state, applyChatEvent(streamStartEvent));

      const deltaEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "3",
        type: "stream_delta",
        message_id: "msg-1",
        ops: [{ op: "append_reasoning", text: "Let me think about this..." }],
      };

      state = chatReducer(state, applyChatEvent(deltaEvent));
      const runtime = state.threads[chatId]!;
      const lastMessage =
        runtime.thread.messages[runtime.thread.messages.length - 1];

      expect(lastMessage).toHaveProperty(
        "reasoning_content",
        "Let me think about this...",
      );
    });
  });

  describe("applyChatEvent - message_added", () => {
    test("should add message at index", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [{ role: "user", content: "Hello" }],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      const addEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "2",
        type: "message_added",
        message: { role: "assistant", content: "Hi!" },
        index: 1,
      };

      state = chatReducer(state, applyChatEvent(addEvent));
      const runtime = state.threads[chatId]!;

      expect(runtime.thread.messages).toHaveLength(2);
      expect(runtime.thread.messages[1].content).toBe("Hi!");
    });

    test("should replace existing message with same message_id (deduplication)", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
          state: "generating",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [{ role: "user", content: "Hello" }],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      // First, stream_started adds a placeholder with message_id
      const streamStartEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "2",
        type: "stream_started",
        message_id: "msg-123",
      };
      state = chatReducer(state, applyChatEvent(streamStartEvent));

      // Add some streaming content
      const deltaEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "3",
        type: "stream_delta",
        message_id: "msg-123",
        ops: [{ op: "append_content", text: "Streaming content..." }],
      };
      state = chatReducer(state, applyChatEvent(deltaEvent));

      // Now message_added comes with the same message_id - should REPLACE, not duplicate
      const addEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "4",
        type: "message_added",
        message: {
          role: "assistant",
          content: "Final complete content",
          message_id: "msg-123",
        },
        index: 1,
      };

      state = chatReducer(state, applyChatEvent(addEvent));
      const runtime = state.threads[chatId]!;

      // Should still have only 2 messages (user + assistant), not 3
      expect(runtime.thread.messages).toHaveLength(2);
      // Content should be the final version, not streaming version
      expect(runtime.thread.messages[1].content).toBe("Final complete content");
    });

    test("should preserve server fields and update seq when replacing assistant message", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
          state: "generating",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [{ role: "user", content: "Hello" }],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "2",
          type: "stream_started",
          message_id: "msg-assistant-1",
        }),
      );

      const deltaOps: DeltaOp[] = [
        {
          op: "set_tool_calls",
          tool_calls: [
            { id: "call-1", function: { name: "web", arguments: "{}" } },
          ],
        },
        {
          op: "add_server_content_block",
          block: { type: "text", text: "server block" },
        },
        {
          op: "merge_extra",
          extra: { metering_balance: 42 },
        },
        {
          op: "append_reasoning",
          text: "stream reasoning",
        },
      ];

      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "3",
          type: "stream_delta",
          message_id: "msg-assistant-1",
          ops: deltaOps,
        }),
      );

      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "4",
          type: "message_added",
          message: {
            role: "assistant",
            content: "Final",
            message_id: "msg-assistant-1",
          },
          index: 1,
        }),
      );

      const runtime = state.threads[chatId]!;
      const assistant = runtime.thread.messages[1];
      if (assistant.role !== "assistant") {
        throw new Error("Expected assistant message");
      }
      expect(assistant.tool_calls).toHaveLength(1);
      expect(assistant.server_content_blocks).toHaveLength(1);
      expect(assistant.extra).toEqual({ metering_balance: 42 });
      expect(assistant.reasoning_content).toBe("stream reasoning");
      expect(runtime.last_applied_seq).toBe("4");

      const replayed = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "4",
          type: "message_added",
          message: {
            role: "assistant",
            content: "Should be ignored",
            message_id: "msg-assistant-1",
          },
          index: 1,
        }),
      );

      const replayedAssistant = replayed.threads[chatId]!.thread.messages[1];
      expect(replayedAssistant.content).toBe("Final");
    });

    test("should clamp negative message_added index to start", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [
          { role: "user", content: "Existing-1", message_id: "m1" },
          { role: "assistant", content: "Existing-2", message_id: "m2" },
        ],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "2",
          type: "message_added",
          message: {
            role: "user",
            content: "Inserted at start",
            message_id: "m-new",
          },
          index: -5,
        }),
      );

      const runtime = state.threads[chatId]!;
      expect(runtime.thread.messages[0].content).toBe("Inserted at start");
      expect(runtime.thread.messages).toHaveLength(3);
    });
  });

  describe("applyChatEvent - pause_required", () => {
    test("should set pause state and reasons", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
          state: "generating",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      const pauseEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "2",
        type: "pause_required",
        reasons: [
          {
            type: "confirmation",
            tool_name: "shell",
            command: "shell rm -rf /",
            rule: "dangerous_command",
            tool_call_id: "call_123",
            integr_config_path: null,
          },
        ],
      };

      state = chatReducer(state, applyChatEvent(pauseEvent));
      const runtime = state.threads[chatId]!;

      expect(runtime.confirmation.pause).toBe(true);
      expect(runtime.confirmation.pause_reasons).toHaveLength(1);
      expect(runtime.confirmation.pause_reasons[0].tool_call_id).toBe(
        "call_123",
      );
      // Note: streaming state is controlled by sidebar SSE session_state updates
    });
  });

  describe("applyChatEvent - message_updated", () => {
    test("should update message content by message_id", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [
          { role: "user", content: "Original", message_id: "msg-user-1" },
        ],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      const updateEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "2",
        type: "message_updated",
        message_id: "msg-user-1",
        message: {
          role: "user",
          content: "Updated content",
          message_id: "msg-user-1",
        },
      };

      state = chatReducer(state, applyChatEvent(updateEvent));
      const runtime = state.threads[chatId]!;

      expect(runtime.thread.messages).toHaveLength(1);
      expect(runtime.thread.messages[0].content).toBe("Updated content");
    });

    test("should not affect other messages when updating", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [
          { role: "user", content: "First", message_id: "msg-1" },
          { role: "assistant", content: "Response", message_id: "msg-2" },
          { role: "user", content: "Second", message_id: "msg-3" },
        ],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      const updateEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "2",
        type: "message_updated",
        message_id: "msg-2",
        message: {
          role: "assistant",
          content: "Updated response",
          message_id: "msg-2",
        },
      };

      state = chatReducer(state, applyChatEvent(updateEvent));
      const runtime = state.threads[chatId]!;

      expect(runtime.thread.messages).toHaveLength(3);
      expect(runtime.thread.messages[0].content).toBe("First");
      expect(runtime.thread.messages[1].content).toBe("Updated response");
      expect(runtime.thread.messages[2].content).toBe("Second");
    });
  });

  describe("applyChatEvent - message_removed", () => {
    test("should remove message by message_id", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [
          { role: "user", content: "Hello", message_id: "msg-1" },
          { role: "assistant", content: "Hi", message_id: "msg-2" },
        ],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      const removeEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "2",
        type: "message_removed",
        message_id: "msg-2",
      };

      state = chatReducer(state, applyChatEvent(removeEvent));
      const runtime = state.threads[chatId]!;

      expect(runtime.thread.messages).toHaveLength(1);
      expect(runtime.thread.messages[0].content).toBe("Hello");
    });

    test("should handle removing non-existent message gracefully", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [{ role: "user", content: "Hello", message_id: "msg-1" }],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      const removeEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "2",
        type: "message_removed",
        message_id: "non-existent-id",
      };

      state = chatReducer(state, applyChatEvent(removeEvent));
      const runtime = state.threads[chatId]!;

      expect(runtime.thread.messages).toHaveLength(1);
    });
  });

  describe("applyChatEvent - messages_truncated", () => {
    test("should truncate messages from index", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [
          { role: "user", content: "First", message_id: "msg-1" },
          { role: "assistant", content: "Response 1", message_id: "msg-2" },
          { role: "user", content: "Second", message_id: "msg-3" },
          { role: "assistant", content: "Response 2", message_id: "msg-4" },
        ],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      const truncateEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "2",
        type: "messages_truncated",
        from_index: 2,
      };

      state = chatReducer(state, applyChatEvent(truncateEvent));
      const runtime = state.threads[chatId]!;

      expect(runtime.thread.messages).toHaveLength(2);
      expect(runtime.thread.messages[0].content).toBe("First");
      expect(runtime.thread.messages[1].content).toBe("Response 1");
    });

    test("should handle truncate from index 0", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [
          { role: "user", content: "Hello", message_id: "msg-1" },
          { role: "assistant", content: "Hi", message_id: "msg-2" },
        ],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      const truncateEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "2",
        type: "messages_truncated",
        from_index: 0,
      };

      state = chatReducer(state, applyChatEvent(truncateEvent));
      const runtime = state.threads[chatId]!;

      expect(runtime.thread.messages).toHaveLength(0);
    });

    test("should clamp negative truncate index to 0", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [
          { role: "user", content: "A", message_id: "m1" },
          { role: "assistant", content: "B", message_id: "m2" },
        ],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "2",
          type: "messages_truncated",
          from_index: -1,
        }),
      );

      const runtime = state.threads[chatId]!;
      expect(runtime.thread.messages).toHaveLength(0);
    });
  });

  describe("applyChatEvent - thread_updated", () => {
    test("should update thread params", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
          model: "gpt-3.5",
          mode: "NO_TOOLS",
          tool_use: "quick",
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
        messages: [],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      const updateEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "2",
        type: "thread_updated",
        model: "gpt-4",
        mode: "agent",
        boost_reasoning: true,
      };

      state = chatReducer(state, applyChatEvent(updateEvent));
      const runtime = state.threads[chatId]!;

      expect(runtime.thread.model).toBe("gpt-4");
      expect(runtime.thread.mode).toBe("agent");
      expect(runtime.thread.boost_reasoning).toBe(true);
    });
  });

  describe("Event sequence handling", () => {
    test("should ignore events for unknown chat_id", () => {
      const event: ChatEventEnvelope = {
        chat_id: "unknown-chat-id",
        seq: "1",
        type: "stream_started",
        message_id: "msg-1",
      };

      const result = chatReducer(initialState, applyChatEvent(event));

      expect(result.threads["unknown-chat-id"]).toBeUndefined();
    });

    test("should process events in sequence", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [{ role: "user", content: "Hi" }],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      const events: ChatEventEnvelope[] = [
        {
          chat_id: chatId,
          seq: "2",
          type: "stream_started",
          message_id: "msg-1",
        },
        {
          chat_id: chatId,
          seq: "3",
          type: "stream_delta",
          message_id: "msg-1",
          ops: [{ op: "append_content", text: "Hello!" }],
        },
        {
          chat_id: chatId,
          seq: "4",
          type: "stream_finished",
          message_id: "msg-1",
          finish_reason: "stop",
        },
      ];

      for (const event of events) {
        state = chatReducer(state, applyChatEvent(event));
      }

      const runtime = state.threads[chatId]!;
      expect(runtime.streaming).toBe(false);
      expect(runtime.waiting_for_response).toBe(false);
      expect(runtime.thread.messages).toHaveLength(2);
      expect(runtime.thread.messages[1].content).toBe("Hello!");
    });
  });

  describe("applyChatEvent - ack and ide_tool_required seq guards", () => {
    test("ack should advance last_applied_seq", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [{ role: "user", content: "Hello" }],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "5",
          type: "ack",
          client_request_id: "req-1",
          accepted: true,
          result: null,
        }),
      );

      const runtime = state.threads[chatId]!;
      expect(runtime.last_applied_seq).toBe("5");
    });

    test("ack should reject replayed seq", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [{ role: "user", content: "Hello" }],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      // Advance to seq 5 via ack
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "5",
          type: "ack",
          client_request_id: "req-1",
          accepted: true,
          result: null,
        }),
      );

      // Replay old ack at seq 3 - should be ignored
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "3",
          type: "ack",
          client_request_id: "req-2",
          accepted: true,
          result: null,
        }),
      );

      expect(state.threads[chatId]!.last_applied_seq).toBe("5");
    });

    test("ack then old message_added should be rejected", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [{ role: "user", content: "Hello", message_id: "m1" }],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      // ack advances watermark to seq 5
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "5",
          type: "ack",
          client_request_id: "req-1",
          accepted: true,
          result: null,
        }),
      );

      // Old message_added at seq 4 should be rejected
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "4",
          type: "message_added",
          message: {
            role: "assistant",
            content: "Should be rejected",
            message_id: "m-stale",
          },
          index: 1,
        }),
      );

      expect(state.threads[chatId]!.thread.messages).toHaveLength(1);
    });

    test("ide_tool_required should advance last_applied_seq with guard", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      // Advance to seq 7
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "7",
          type: "ide_tool_required",
          tool_call_id: "tc-1",
          tool_name: "shell",
          args: "{}",
        }),
      );
      expect(state.threads[chatId]!.last_applied_seq).toBe("7");

      // Replay old seq 3 should be ignored
      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "3",
          type: "ide_tool_required",
          tool_call_id: "tc-2",
          tool_name: "shell",
          args: "{}",
        }),
      );
      expect(state.threads[chatId]!.last_applied_seq).toBe("7");
    });
  });

  describe("message_index_by_id - prototype pollution protection", () => {
    test("should safely handle __proto__ as message_id", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
        messages: [
          { role: "user", content: "Test", message_id: "__proto__" },
          { role: "assistant", content: "Reply", message_id: "constructor" },
        ],
      };

      const state = chatReducer(initialState, applyChatEvent(snapshotEvent));
      const runtime = state.threads[chatId]!;

      // Messages should be stored correctly
      expect(runtime.thread.messages).toHaveLength(2);
      expect(runtime.thread.messages[0].content).toBe("Test");
      expect(runtime.thread.messages[1].content).toBe("Reply");

      // Index should work without polluting Object prototype
      expect(runtime.message_index_by_id).toBeDefined();
      const emptyObj = {};
      // Verify Object.prototype was not polluted
      expect(Object.getPrototypeOf(emptyObj)).toBe(Object.prototype);
      expect(emptyObj.constructor).toBe(Object);

      // Can update message with __proto__ id without crash
      const updateState = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "2",
          type: "message_updated",
          message_id: "__proto__",
          message: {
            role: "user",
            content: "Updated",
            message_id: "__proto__",
          },
        }),
      );
      expect(updateState.threads[chatId]!.thread.messages[0].content).toBe(
        "Updated",
      );
    });
  });

  describe("isStreaming flag transitions", () => {
    test("stream_finished should clear streaming flag", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
          state: "generating",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [{ role: "user", content: "Hello" }],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));
      expect(state.threads[chatId]!.streaming).toBe(true);

      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "2",
          type: "stream_started",
          message_id: "msg-1",
        }),
      );
      expect(state.threads[chatId]!.streaming).toBe(true);

      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "3",
          type: "stream_delta",
          message_id: "msg-1",
          ops: [{ op: "append_content", text: "content" }],
        }),
      );
      expect(state.threads[chatId]!.streaming).toBe(true);

      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "4",
          type: "stream_finished",
          message_id: "msg-1",
          finish_reason: "stop",
        }),
      );
      expect(state.threads[chatId]!.streaming).toBe(false);
      expect(state.threads[chatId]!.waiting_for_response).toBe(false);
    });

    test("stream_finished with tool_calls should keep waiting_for_response", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
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
          state: "generating",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [{ role: "user", content: "Hello" }],
      };

      let state = chatReducer(initialState, applyChatEvent(snapshotEvent));

      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "2",
          type: "stream_started",
          message_id: "msg-1",
        }),
      );

      state = chatReducer(
        state,
        applyChatEvent({
          chat_id: chatId,
          seq: "3",
          type: "stream_finished",
          message_id: "msg-1",
          finish_reason: "tool_calls",
        }),
      );

      const runtime = state.threads[chatId]!;
      expect(runtime.streaming).toBe(false);
      // tool_calls finish reason means tools are about to execute
      expect(runtime.session_state).toBe("executing_tools");
    });
  });
});

describe("Model Priority Order", () => {
  test("newChatAction should use empty string when no localStorage or previous model", () => {
    const emptyState = chatReducer(undefined, { type: "@@INIT" });
    const newState = chatReducer(emptyState, newChatAction(undefined));
    const chatId = newState.current_thread_id;

    expect(newState.threads[chatId]!.thread.model).toBe("");
  });

  test("newChatAction respects payload title", () => {
    const emptyState = chatReducer(undefined, { type: "@@INIT" });
    const newState = chatReducer(
      emptyState,
      newChatAction({ title: "Custom Title" }),
    );
    const chatId = newState.current_thread_id;

    expect(newState.threads[chatId]!.thread.title).toBe("Custom Title");
  });
});
