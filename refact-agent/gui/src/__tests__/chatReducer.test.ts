import { expect, test, describe, beforeEach } from "vitest";
import { chatReducer } from "../features/Chat/Thread/reducer";
import type { Chat } from "../features/Chat/Thread/types";
import {
  newChatAction,
  createChatWithId,
  closeThread,
  switchToThread,
  addThreadImage,
  removeThreadImageByIndex,
  applyChatEvent,
} from "../features/Chat/Thread/actions";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";

describe("Chat Thread Reducer - Core Functionality", () => {
  let initialState: Chat;
  let chatId: string;

  beforeEach(() => {
    const emptyState = chatReducer(undefined, { type: "@@INIT" });
    initialState = chatReducer(emptyState, newChatAction(undefined));
    chatId = initialState.current_thread_id;
  });

  describe("Chat Thread Creation", () => {
    test("should_create_new_chat_with_initial_state", () => {
      expect(initialState.open_thread_ids).toHaveLength(1);
      expect(initialState.current_thread_id).toBe(
        initialState.open_thread_ids[0],
      );
      expect(initialState.threads[chatId]?.thread.messages).toHaveLength(0);
    });

    test("should_preserve_last_used_parameters", () => {
      const customTitle = "Test Chat Title";
      const state = chatReducer(
        initialState,
        newChatAction({ title: customTitle }),
      );
      const newChatId = state.current_thread_id;

      expect(state.threads[newChatId]?.thread.title).toBe(customTitle);
      expect(state.open_thread_ids).toHaveLength(2);
    });
  });

  describe("Task Chat Handling", () => {
    test("should_not_add_task_chat_to_open_tabs", () => {
      const taskChatId = "task-chat-123";
      const state = chatReducer(
        initialState,
        createChatWithId({
          id: taskChatId,
          isTaskChat: true,
          title: "Task Chat",
        }),
      );

      expect(state.open_thread_ids).not.toContain(taskChatId);
      expect(state.threads[taskChatId]).toBeDefined();
      expect(state.threads[taskChatId]?.thread.is_task_chat).toBe(true);
    });

    test("should_preserve_is_task_chat_flag_on_snapshot", () => {
      const taskChatId = "task-chat-456";
      const state = chatReducer(
        initialState,
        createChatWithId({
          id: taskChatId,
          isTaskChat: true,
          title: "Task Chat",
        }),
      );

      expect(state.threads[taskChatId]?.thread.is_task_chat).toBe(true);
      expect(state.open_thread_ids).not.toContain(taskChatId);
    });
  });

  describe("Thread Lifecycle", () => {
    test("should_switch_threads_and_reset_snapshot_received", () => {
      const state1 = chatReducer(initialState, newChatAction(undefined));
      const chat1Id = initialState.current_thread_id;
      const chat2Id = state1.current_thread_id;

      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chat2Id,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chat2Id,
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

      let state = chatReducer(state1, applyChatEvent(snapshotEvent));
      expect(state.threads[chat2Id]?.snapshot_received).toBe(true);

      state = chatReducer(state, switchToThread({ id: chat1Id }));

      expect(state.current_thread_id).toBe(chat1Id);
      expect(state.threads[chat1Id]?.snapshot_received).toBe(false);
    });

    test("should_close_thread_when_not_streaming", () => {
      const state1 = chatReducer(initialState, newChatAction(undefined));
      const chat1Id = initialState.current_thread_id;
      const chat2Id = state1.current_thread_id;

      const state = chatReducer(state1, closeThread({ id: chat2Id }));

      expect(state.open_thread_ids).not.toContain(chat2Id);
      expect(state.threads[chat2Id]).toBeUndefined();
      expect(state.current_thread_id).toBe(chat1Id);
    });

    test("should_keep_thread_in_memory_when_streaming", () => {
      const state1 = chatReducer(initialState, newChatAction(undefined));
      const chat2Id = state1.current_thread_id;

      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chat2Id,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chat2Id,
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

      let state = chatReducer(state1, applyChatEvent(snapshotEvent));
      expect(state.threads[chat2Id]?.streaming).toBe(true);

      state = chatReducer(state, closeThread({ id: chat2Id }));

      expect(state.open_thread_ids).not.toContain(chat2Id);
      expect(state.threads[chat2Id]).toBeDefined();
      expect(state.threads[chat2Id]?.streaming).toBe(true);
    });
  });

  describe("Image Attachment", () => {
    test("should_add_image_up_to_limit", () => {
      let state = initialState;

      for (let i = 0; i < 5; i++) {
        state = chatReducer(
          state,
          addThreadImage({
            id: chatId,
            image: {
              name: `image${i}.png`,
              content: `data:image/png;base64,${i}`,
              type: "image/png",
            },
          }),
        );
      }

      expect(state.threads[chatId]?.attached_images).toHaveLength(5);

      state = chatReducer(
        state,
        addThreadImage({
          id: chatId,
          image: {
            name: "image5.png",
            content: "data:image/png;base64,5",
            type: "image/png",
          },
        }),
      );

      expect(state.threads[chatId]?.attached_images).toHaveLength(5);
    });

    test("should_remove_image_by_index", () => {
      let state = initialState;

      state = chatReducer(
        state,
        addThreadImage({
          id: chatId,
          image: {
            name: "image1.png",
            content: "data:image/png;base64,1",
            type: "image/png",
          },
        }),
      );

      state = chatReducer(
        state,
        addThreadImage({
          id: chatId,
          image: {
            name: "image2.png",
            content: "data:image/png;base64,2",
            type: "image/png",
          },
        }),
      );

      expect(state.threads[chatId]?.attached_images).toHaveLength(2);

      state = chatReducer(
        state,
        removeThreadImageByIndex({
          id: chatId,
          index: 0,
        }),
      );

      expect(state.threads[chatId]?.attached_images).toHaveLength(1);
      expect(state.threads[chatId]?.attached_images[0]?.name).toBe(
        "image2.png",
      );
    });

    test("should_handle_image_removal_edge_cases", () => {
      let state = initialState;

      state = chatReducer(
        state,
        removeThreadImageByIndex({
          id: chatId,
          index: 0,
        }),
      );

      expect(state.threads[chatId]?.attached_images).toHaveLength(0);

      state = chatReducer(
        state,
        addThreadImage({
          id: chatId,
          image: {
            name: "image1.png",
            content: "data:image/png;base64,1",
            type: "image/png",
          },
        }),
      );

      state = chatReducer(
        state,
        removeThreadImageByIndex({
          id: chatId,
          index: 999,
        }),
      );

      expect(state.threads[chatId]?.attached_images).toHaveLength(1);
    });
  });

  describe("Edge Cases", () => {
    test("should_handle_operations_on_nonexistent_thread_gracefully", () => {
      const state = chatReducer(
        initialState,
        closeThread({ id: "nonexistent-id" }),
      );

      expect(state.threads["nonexistent-id"]).toBeUndefined();
      expect(state.current_thread_id).toBe(chatId);
    });

    test("should_maintain_state_consistency_with_concurrent_operations", () => {
      const state1 = chatReducer(initialState, newChatAction(undefined));
      const chat1Id = initialState.current_thread_id;
      const chat2Id = state1.current_thread_id;

      let state = state1;
      state = chatReducer(state, switchToThread({ id: chat1Id }));
      expect(state.current_thread_id).toBe(chat1Id);

      state = chatReducer(state, closeThread({ id: chat2Id }));
      expect(state.current_thread_id).toBe(chat1Id);
      expect(state.open_thread_ids).toContain(chat1Id);
      expect(state.open_thread_ids).not.toContain(chat2Id);
    });
  });
});
