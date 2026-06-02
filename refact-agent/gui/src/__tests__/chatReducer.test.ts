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
  setTemperature,
  setMaxTokens,
  setChatModel,
  setMaxNewTokens,
} from "../features/Chat/Thread/actions";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";

function requireThread(state: Chat, threadId: string) {
  const runtime = state.threads[threadId];
  expect(runtime).toBeDefined();
  if (!runtime) throw new Error(`Expected thread ${threadId} to exist`);
  return runtime;
}

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
        background_agents: [],
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
        background_agents: [],
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

  describe("Snapshot params sync (stale-state regression)", () => {
    test("snapshot_with_temperature_absent_should_not_restore_stale_ui_temperature", () => {
      // User had temperature=0.9 set locally
      const withTemp = chatReducer(
        initialState,
        setTemperature({ chatId, value: 0.9 }),
      );
      expect(withTemp.threads[chatId]?.thread.temperature).toBe(0.9);

      // Backend sends snapshot WITHOUT temperature field (None in Rust → absent in JSON)
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
          model: "gpt-4o",
          mode: "agent",
          tool_use: "agent",
          boost_reasoning: false,
          include_project_info: true,
          checkpoints_enabled: false,
          context_tokens_cap: 8192,
          is_title_generated: false,
          // temperature intentionally absent — backend has None
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
        messages: [],
      };

      const afterSnapshot = chatReducer(
        withTemp,
        applyChatEvent(snapshotEvent),
      );
      // Should be undefined (backend authoritative), not the stale 0.9
      expect(afterSnapshot.threads[chatId]?.thread.temperature).toBeUndefined();
    });

    test("snapshot_with_max_tokens_absent_should_not_restore_stale_ui_max_tokens", () => {
      const withMaxTokens = chatReducer(
        initialState,
        setMaxTokens({ chatId, value: 2048 }),
      );
      expect(withMaxTokens.threads[chatId]?.thread.max_tokens).toBe(2048);

      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
          model: "gpt-4o",
          mode: "agent",
          tool_use: "agent",
          boost_reasoning: false,
          include_project_info: true,
          checkpoints_enabled: false,
          context_tokens_cap: 8192,
          is_title_generated: false,
          // max_tokens intentionally absent
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
        messages: [],
      };

      const afterSnapshot = chatReducer(
        withMaxTokens,
        applyChatEvent(snapshotEvent),
      );
      expect(afterSnapshot.threads[chatId]?.thread.max_tokens).toBeUndefined();
    });

    test("snapshot_with_temperature_present_should_apply_backend_value", () => {
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
          model: "gpt-4o",
          mode: "agent",
          tool_use: "agent",
          boost_reasoning: false,
          include_project_info: true,
          checkpoints_enabled: false,
          context_tokens_cap: 8192,
          is_title_generated: false,
          temperature: 0.7,
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
        messages: [],
      };

      const afterSnapshot = chatReducer(
        initialState,
        applyChatEvent(snapshotEvent),
      );
      expect(afterSnapshot.threads[chatId]?.thread.temperature).toBe(0.7);
    });
  });

  describe("Caps default model initialization", () => {
    test("caps_fulfilled_sets_default_model_when_thread_model_is_empty", () => {
      expect(initialState.threads[chatId]?.thread.model).toBe("");

      const capsPayload = {
        chat_default_model: "gpt-4o",
        chat_models: {
          "gpt-4o": { n_ctx: 128000 },
        },
      };

      // RTK Query matchFulfilled checks: meta.requestStatus === "fulfilled"
      // AND meta.arg.endpointName === "getCaps"
      const action = {
        type: "caps/executeQuery/fulfilled",
        payload: capsPayload,
        meta: {
          requestId: "test",
          requestStatus: "fulfilled" as const,
          arg: { endpointName: "getCaps" },
        },
      };

      const afterCaps = chatReducer(initialState, action);
      expect(afterCaps.threads[chatId]?.thread.model).toBe("gpt-4o");
    });

    test("caps_fulfilled_does_not_override_existing_model", () => {
      const withModel = chatReducer(
        initialState,
        createChatWithId({ id: "other", model: "claude-3-5-sonnet" }),
      );
      const otherChatId = "other";

      const capsPayload = {
        chat_default_model: "gpt-4o",
        chat_models: {
          "gpt-4o": { n_ctx: 128000 },
          "claude-3-5-sonnet": { n_ctx: 200000 },
        },
      };

      const action = {
        type: "caps/executeQuery/fulfilled",
        payload: capsPayload,
        meta: {
          requestId: "test",
          requestStatus: "fulfilled" as const,
          arg: { endpointName: "getCaps" },
        },
      };

      // Switch to 'other' chat so it becomes the current thread
      const withOtherCurrent = { ...withModel, current_thread_id: otherChatId };
      const afterCaps = chatReducer(withOtherCurrent, action);
      // claude-3-5-sonnet should be preserved, not overridden by gpt-4o
      expect(afterCaps.threads[otherChatId]?.thread.model).toBe(
        "claude-3-5-sonnet",
      );
    });

    test("caps_fulfilled_updates_auto_context_cap_after_model_switch", () => {
      const capsPayload = {
        chat_default_model: "old-model",
        chat_models: {
          "old-model": { n_ctx: 8192 },
          "new-model": { n_ctx: 128000 },
        },
      };

      const capsAction = {
        type: "caps/executeQuery/fulfilled",
        payload: capsPayload,
        meta: {
          requestId: "test",
          requestStatus: "fulfilled" as const,
          arg: { endpointName: "getCaps" },
        },
      };

      const initialized = chatReducer(initialState, capsAction);
      expect(initialized.threads[chatId]?.thread.context_tokens_cap).toBe(8192);

      const switchedModel = chatReducer(
        initialized,
        setChatModel({ model: "new-model" }),
      );
      const afterCaps = chatReducer(switchedModel, capsAction);

      expect(
        afterCaps.threads[chatId]?.thread.currentMaximumContextTokens,
      ).toBe(128000);
      expect(afterCaps.threads[chatId]?.thread.context_tokens_cap).toBe(128000);
    });

    test("caps_fulfilled_preserves_lower_explicit_context_cap_after_model_switch", () => {
      const capsPayload = {
        chat_default_model: "old-model",
        chat_models: {
          "old-model": { n_ctx: 8192 },
          "new-model": { n_ctx: 128000 },
        },
      };

      const capsAction = {
        type: "caps/executeQuery/fulfilled",
        payload: capsPayload,
        meta: {
          requestId: "test",
          requestStatus: "fulfilled" as const,
          arg: { endpointName: "getCaps" },
        },
      };

      const initialized = chatReducer(initialState, capsAction);
      const existingRuntime = requireThread(initialized, chatId);
      const withExplicitCap: Chat = {
        ...initialized,
        threads: {
          ...initialized.threads,
          [chatId]: {
            ...existingRuntime,
            thread: {
              ...existingRuntime.thread,
              context_tokens_cap: 4096,
            },
          },
        },
      };

      const switchedModel = chatReducer(
        withExplicitCap,
        setChatModel({ model: "new-model" }),
      );
      const afterCaps = chatReducer(switchedModel, capsAction);

      expect(
        afterCaps.threads[chatId]?.thread.currentMaximumContextTokens,
      ).toBe(128000);
      expect(afterCaps.threads[chatId]?.thread.context_tokens_cap).toBe(4096);
    });

    test("caps_fulfilled_preserves_explicit_cap_after_preview_updates_current_context", () => {
      const capsPayload = {
        chat_default_model: "old-model",
        chat_models: {
          "old-model": { n_ctx: 8192 },
          "new-model": { n_ctx: 128000 },
        },
      };

      const capsAction = {
        type: "caps/executeQuery/fulfilled",
        payload: capsPayload,
        meta: {
          requestId: "test",
          requestStatus: "fulfilled" as const,
          arg: { endpointName: "getCaps" },
        },
      };
      const previewAction = {
        type: "commands/executeQuery/fulfilled",
        payload: {
          messages: [],
          files: [],
          current_context: 1024,
          number_context: 4096,
        },
        meta: {
          requestId: "preview-test",
          requestStatus: "fulfilled" as const,
          arg: { endpointName: "getCommandPreview" },
        },
      };

      const initialized = chatReducer(initialState, capsAction);
      const existingRuntime = requireThread(initialized, chatId);
      const withExplicitCap: Chat = {
        ...initialized,
        threads: {
          ...initialized.threads,
          [chatId]: {
            ...existingRuntime,
            thread: {
              ...existingRuntime.thread,
              context_tokens_cap: 4096,
            },
          },
        },
      };
      const afterPreview = chatReducer(withExplicitCap, previewAction);
      expect(
        afterPreview.threads[chatId]?.thread.currentMaximumContextTokens,
      ).toBe(4096);
      expect(
        afterPreview.threads[chatId]?.thread.modelMaximumContextTokens,
      ).toBe(8192);

      const switchedModel = chatReducer(
        afterPreview,
        setChatModel({ model: "new-model" }),
      );
      const afterCaps = chatReducer(switchedModel, capsAction);

      expect(afterCaps.threads[chatId]?.thread.modelMaximumContextTokens).toBe(
        128000,
      );
      expect(afterCaps.threads[chatId]?.thread.context_tokens_cap).toBe(4096);
    });

    test("set_max_new_tokens_updates_auto_context_cap_after_model_switch", () => {
      const withOldModelLimit = chatReducer(
        initialState,
        setMaxNewTokens(8192),
      );
      const withNewModel = chatReducer(
        withOldModelLimit,
        setChatModel({ model: "new-model" }),
      );
      const afterLimitUpdate = chatReducer(
        withNewModel,
        setMaxNewTokens(128000),
      );

      expect(
        afterLimitUpdate.threads[chatId]?.thread.currentMaximumContextTokens,
      ).toBe(128000);
      expect(afterLimitUpdate.threads[chatId]?.thread.context_tokens_cap).toBe(
        128000,
      );
    });

    test("snapshot_preserves_model_context_limit_before_model_switch", () => {
      const capsPayload = {
        chat_default_model: "old-model",
        chat_models: {
          "old-model": { n_ctx: 8192 },
          "new-model": { n_ctx: 128000 },
        },
      };

      const capsAction = {
        type: "caps/executeQuery/fulfilled",
        payload: capsPayload,
        meta: {
          requestId: "test",
          requestStatus: "fulfilled" as const,
          arg: { endpointName: "getCaps" },
        },
      };
      const snapshotEvent: ChatEventEnvelope = {
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
          model: "old-model",
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
        background_agents: [],
        messages: [],
      };

      const initialized = chatReducer(initialState, capsAction);
      const afterSnapshot = chatReducer(
        initialized,
        applyChatEvent(snapshotEvent),
      );
      const switchedModel = chatReducer(
        afterSnapshot,
        setChatModel({ model: "new-model" }),
      );
      const afterCaps = chatReducer(switchedModel, capsAction);

      expect(
        afterSnapshot.threads[chatId]?.thread.modelMaximumContextTokens,
      ).toBe(8192);
      expect(afterCaps.threads[chatId]?.thread.modelMaximumContextTokens).toBe(
        128000,
      );
      expect(afterCaps.threads[chatId]?.thread.context_tokens_cap).toBe(128000);
    });

    test("model_switch_updates_auto_context_cap_without_existing_model_limit", () => {
      const existingRuntime = requireThread(initialState, chatId);
      const restoredState: Chat = {
        ...initialState,
        threads: {
          ...initialState.threads,
          [chatId]: {
            ...existingRuntime,
            thread: {
              ...existingRuntime.thread,
              model: "old-model",
              context_tokens_cap: 8192,
              currentMaximumContextTokens: undefined,
              modelMaximumContextTokens: undefined,
            },
          },
        },
      };

      const switchedModel = chatReducer(
        restoredState,
        setChatModel({
          model: "new-model",
          modelMaxContextTokens: 128000,
          previousModelMaxContextTokens: 8192,
        }),
      );

      expect(
        switchedModel.threads[chatId]?.thread.modelMaximumContextTokens,
      ).toBe(128000);
      expect(switchedModel.threads[chatId]?.thread.context_tokens_cap).toBe(
        128000,
      );
    });

    test("model_switch_updates_auto_context_cap_with_stale_previous_limit", () => {
      const withOldModelLimit = chatReducer(
        initialState,
        setMaxNewTokens(8192),
      );
      const firstSwitch = chatReducer(
        withOldModelLimit,
        setChatModel({
          model: "middle-model",
          modelMaxContextTokens: 128000,
          previousModelMaxContextTokens: 8192,
        }),
      );
      const secondSwitch = chatReducer(
        firstSwitch,
        setChatModel({
          model: "new-model",
          modelMaxContextTokens: 200000,
          previousModelMaxContextTokens: 8192,
        }),
      );

      expect(firstSwitch.threads[chatId]?.thread.context_tokens_cap).toBe(
        128000,
      );
      expect(secondSwitch.threads[chatId]?.thread.context_tokens_cap).toBe(
        200000,
      );
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
