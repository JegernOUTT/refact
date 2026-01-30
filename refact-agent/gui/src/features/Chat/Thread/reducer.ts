import { createReducer, Draft } from "@reduxjs/toolkit";
import {
  Chat,
  ChatThread,
  ChatThreadRuntime,
  IntegrationMeta,
  ToolUse,
  ChatModeId,
  isToolUse,
  normalizeLegacyMode,
} from "./types";
import { v4 as uuidv4 } from "uuid";
import { getLastThreadParams } from "../../../utils/threadStorage";
import {
  setToolUse,
  setThreadMode,
  enableSend,
  clearChatError,
  setChatModel,
  setSystemPrompt,
  newChatAction,
  createChatWithId,
  backUpMessages,
  removeChatFromCache,
  restoreChat,
  setPreventSend,
  saveTitle,
  newIntegrationChat,
  setSendImmediately,
  setChatMode,
  setIntegrationData,
  setIsWaitingForResponse,
  setMaxNewTokens,
  setAutomaticPatch,
  setLastUserMessageId,
  setEnabledCheckpoints,
  setBoostReasoning,
  fixBrokenToolMessages,
  setIsNewChatSuggested,
  setIsNewChatSuggestionRejected,
  upsertToolCall,
  setIncreaseMaxTokens,
  setAreFollowUpsEnabled,
  setIncludeProjectInfo,
  setContextTokensCap,
  closeThread,
  switchToThread,
  updateOpenThread,
  updateChatRuntimeFromSessionState,
  setThreadPauseReasons,
  clearThreadPauseReasons,
  setThreadConfirmationStatus,
  addThreadImage,
  removeThreadImageByIndex,
  resetThreadImages,
  applyChatEvent,
  requestSseRefresh,
  clearSseRefreshRequest,
} from "./actions";
import { applyDeltaOps } from "../../../services/refact/chatSubscription";
import {
  AssistantMessage,
  ChatMessages,
  commandsApi,
  isAssistantMessage,
  isDiffMessage,
  isToolCallMessage,
  isToolMessage,
  ToolCall,
  ToolConfirmationPauseReason,
  ToolMessage,
  validateToolCall,
  DiffChunk,
} from "../../../services/refact";
import { capsApi } from "../../../services/refact";

const createChatThread = (
  tool_use: ToolUse,
  integration?: IntegrationMeta | null,
  mode?: ChatModeId,
): ChatThread => {
  return {
    id: uuidv4(),
    messages: [],
    title: "",
    model: "",
    last_user_message_id: "",
    tool_use,
    integration,
    mode,
    new_chat_suggested: { wasSuggested: false },
    boost_reasoning: false,
    automatic_patch: false,
    increase_max_tokens: false,
    include_project_info: true,
    context_tokens_cap: undefined,
  };
};

const createThreadRuntime = (
  tool_use: ToolUse,
  integration?: IntegrationMeta | null,
  mode?: ChatModeId,
): ChatThreadRuntime => {
  return {
    thread: createChatThread(tool_use, integration, mode),
    streaming: false,
    waiting_for_response: false,
    prevent_send: false,
    error: null,
    queued_items: [],
    send_immediately: false,
    attached_images: [],
    confirmation: {
      pause: false,
      pause_reasons: [],
      status: {
        wasInteracted: false,
        confirmationStatus: true,
      },
    },
    snapshot_received: false,
  };
};

const getThreadMode = ({
  integration,
}: {
  integration?: IntegrationMeta | null;
}) => {
  if (integration) return "configurator";
  return "agent";
};

const normalizeMessage = (msg: ChatMessages[number]): ChatMessages[number] => {
  if (msg.role === "diff" && typeof msg.content === "string") {
    try {
      const parsed: unknown = JSON.parse(msg.content);
      if (Array.isArray(parsed)) {
        return {
          ...msg,
          content: parsed as DiffChunk[],
        } as ChatMessages[number];
      }
    } catch {
      // ignore
    }
  }
  return msg;
};

const createInitialState = (): Chat => {
  return {
    current_thread_id: "",
    open_thread_ids: [],
    threads: {},
    system_prompt: {},
    tool_use: "agent",
    checkpoints_enabled: true,
    follow_ups_enabled: undefined,
    sse_refresh_requested: null,
    stream_version: 0,
  };
};

const initialState = createInitialState();

const getRuntime = (
  state: Draft<Chat>,
  chatId: string,
): Draft<ChatThreadRuntime> | null => {
  return state.threads[chatId] ?? null;
};

const getCurrentRuntime = (
  state: Draft<Chat>,
): Draft<ChatThreadRuntime> | null => {
  return getRuntime(state, state.current_thread_id);
};

export const chatReducer = createReducer(initialState, (builder) => {
  builder.addCase(setToolUse, (state, action) => {
    state.tool_use = action.payload;
  });

  builder.addCase(setThreadMode, (state, action) => {
    const rt = getRuntime(state, action.payload.chatId);
    if (rt && rt.thread.messages.length === 0) {
      rt.thread.mode = action.payload.mode;
    }
  });

  builder.addCase(setPreventSend, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (rt) rt.prevent_send = true;
  });

  builder.addCase(enableSend, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (rt) rt.prevent_send = false;
  });

  builder.addCase(setAreFollowUpsEnabled, (state, action) => {
    state.follow_ups_enabled = action.payload;
  });

  builder.addCase(clearChatError, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (rt) rt.error = null;
  });

  builder.addCase(setChatModel, (state, action) => {
    const rt = getCurrentRuntime(state);
    if (rt) rt.thread.model = action.payload;
  });

  builder.addCase(setSystemPrompt, (state, action) => {
    state.system_prompt = action.payload;
  });

  builder.addCase(newChatAction, (state, action) => {
    const currentRt = getCurrentRuntime(state);
    const lastParams = getLastThreadParams();

    const mode = getThreadMode({});
    const newRuntime = createThreadRuntime(state.tool_use, null, mode);

    newRuntime.thread.model = lastParams.model ?? currentRt?.thread.model ?? "";
    newRuntime.thread.boost_reasoning =
      lastParams.boost_reasoning ?? currentRt?.thread.boost_reasoning ?? false;
    newRuntime.thread.automatic_patch = false;
    newRuntime.thread.increase_max_tokens =
      lastParams.increase_max_tokens ??
      currentRt?.thread.increase_max_tokens ??
      false;
    newRuntime.thread.include_project_info =
      lastParams.include_project_info ??
      currentRt?.thread.include_project_info ??
      true;
    newRuntime.thread.context_tokens_cap =
      lastParams.context_tokens_cap ?? currentRt?.thread.context_tokens_cap;

    if (action.payload?.title) {
      newRuntime.thread.title = action.payload.title;
    }

    const newId = newRuntime.thread.id;
    state.threads[newId] = newRuntime;
    state.open_thread_ids.push(newId);
    state.current_thread_id = newId;
  });

  builder.addCase(createChatWithId, (state, action) => {
    const { id, title, isTaskChat, mode, taskMeta, model } = action.payload;
    const existingRt = state.threads[id];

    if (existingRt) {
      if (isTaskChat) {
        existingRt.thread.is_task_chat = true;
        state.open_thread_ids = state.open_thread_ids.filter(
          (tid) => tid !== id,
        );
      }
      if (title && !existingRt.thread.title) {
        existingRt.thread.title = title;
      }
      if (mode) {
        existingRt.thread.mode = normalizeLegacyMode(mode);
      }
      if (taskMeta) {
        existingRt.thread.task_meta = taskMeta;
      }
      if (model && !existingRt.thread.model) {
        existingRt.thread.model = model;
      }
      state.current_thread_id = id;
      return;
    }

    const currentRt = getCurrentRuntime(state);
    const lastParams = getLastThreadParams();

    const effectiveMode = mode ?? getThreadMode({});
    const newRuntime = createThreadRuntime("agent", null, effectiveMode);

    newRuntime.thread.id = id;
    newRuntime.thread.model =
      model ?? lastParams.model ?? currentRt?.thread.model ?? "";
    newRuntime.thread.boost_reasoning =
      lastParams.boost_reasoning ?? currentRt?.thread.boost_reasoning ?? false;
    newRuntime.thread.automatic_patch = false;
    newRuntime.thread.increase_max_tokens =
      lastParams.increase_max_tokens ??
      currentRt?.thread.increase_max_tokens ??
      false;
    newRuntime.thread.include_project_info =
      lastParams.include_project_info ??
      currentRt?.thread.include_project_info ??
      true;
    newRuntime.thread.context_tokens_cap =
      lastParams.context_tokens_cap ?? currentRt?.thread.context_tokens_cap;

    if (title) {
      newRuntime.thread.title = title;
    }
    if (isTaskChat) {
      newRuntime.thread.is_task_chat = true;
    }
    if (taskMeta) {
      newRuntime.thread.task_meta = taskMeta;
    }

    state.threads[id] = newRuntime;
    if (!isTaskChat) {
      state.open_thread_ids.push(id);
    }
    state.current_thread_id = id;
  });

  builder.addCase(backUpMessages, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (rt) {
      rt.error = null;
      rt.thread.messages = action.payload.messages;
    }
  });

  builder.addCase(setAutomaticPatch, (state, action) => {
    const rt = getRuntime(state, action.payload.chatId);
    if (rt) rt.thread.automatic_patch = action.payload.value;
  });

  builder.addCase(setIsNewChatSuggested, (state, action) => {
    const rt = getRuntime(state, action.payload.chatId);
    if (rt)
      rt.thread.new_chat_suggested = { wasSuggested: action.payload.value };
  });

  builder.addCase(setIsNewChatSuggestionRejected, (state, action) => {
    const rt = getRuntime(state, action.payload.chatId);
    if (rt) {
      rt.prevent_send = false;
      rt.thread.new_chat_suggested = {
        ...rt.thread.new_chat_suggested,
        wasRejectedByUser: action.payload.value,
      };
    }
  });

  builder.addCase(setEnabledCheckpoints, (state, action) => {
    state.checkpoints_enabled = action.payload;
  });

  builder.addCase(setBoostReasoning, (state, action) => {
    const rt = getRuntime(state, action.payload.chatId);
    if (rt) rt.thread.boost_reasoning = action.payload.value;
  });

  builder.addCase(setLastUserMessageId, (state, action) => {
    const rt = getRuntime(state, action.payload.chatId);
    if (rt) rt.thread.last_user_message_id = action.payload.messageId;
  });

  builder.addCase(removeChatFromCache, (state, action) => {
    const id = action.payload.id;
    const rt = state.threads[id];
    if (rt && !rt.streaming && !rt.confirmation.pause) {
      const { [id]: _, ...rest } = state.threads;
      state.threads = rest;
      state.open_thread_ids = state.open_thread_ids.filter((tid) => tid !== id);
    }
  });

  builder.addCase(closeThread, (state, action) => {
    const id = action.payload.id;
    const force = action.payload.force ?? false;
    state.open_thread_ids = state.open_thread_ids.filter((tid) => tid !== id);
    const rt = state.threads[id];
    if (
      rt &&
      (force ||
        (!rt.streaming && !rt.waiting_for_response && !rt.confirmation.pause))
    ) {
      const { [id]: _, ...rest } = state.threads;
      state.threads = rest;
    }
    if (state.current_thread_id === id) {
      state.current_thread_id = state.open_thread_ids[0] ?? "";
    }
  });

  builder.addCase(restoreChat, (state, action) => {
    const existingRt = getRuntime(state, action.payload.id);
    if (existingRt) {
      if (!state.open_thread_ids.includes(action.payload.id)) {
        state.open_thread_ids.push(action.payload.id);
      }
      state.current_thread_id = action.payload.id;
      // Don't reset snapshot_received - thread was already hydrated
      return;
    }

    const mode = normalizeLegacyMode(action.payload.mode);
    const newRuntime: ChatThreadRuntime = {
      thread: {
        id: action.payload.id,
        messages: [],
        model: action.payload.model,
        title: action.payload.title,
        tool_use: action.payload.tool_use ?? state.tool_use,
        mode,
        new_chat_suggested: { wasSuggested: false },
      },
      streaming: false,
      waiting_for_response: false,
      prevent_send: false,
      error: null,
      queued_items: [],
      send_immediately: false,
      attached_images: [],
      confirmation: {
        pause: false,
        pause_reasons: [],
        status: {
          wasInteracted: false,
          confirmationStatus: true,
        },
      },
      snapshot_received: false,
    };

    state.threads[action.payload.id] = newRuntime;
    if (!state.open_thread_ids.includes(action.payload.id)) {
      state.open_thread_ids.push(action.payload.id);
    }
    state.current_thread_id = action.payload.id;
  });

  builder.addCase(switchToThread, (state, action) => {
    const { id, openTab } = action.payload;
    const existingRt = getRuntime(state, id);

    if (!existingRt) {
      console.warn(`[switchToThread] No runtime for ${id}`);
    }

    if (existingRt) {
      const shouldOpenTab =
        openTab !== false && !existingRt.thread.is_task_chat;
      if (shouldOpenTab && !state.open_thread_ids.includes(id)) {
        state.open_thread_ids.push(id);
      }
      state.current_thread_id = id;
    }
  });

  builder.addCase(updateOpenThread, (state, action) => {
    const existingRt = getRuntime(state, action.payload.id);
    if (!existingRt) return;

    const incomingTitle = action.payload.thread.title;
    const incomingGenerated = action.payload.thread.isTitleGenerated;

    if (incomingTitle) {
      if (incomingGenerated === true) {
        if (!existingRt.thread.isTitleGenerated) {
          existingRt.thread.title = incomingTitle;
          existingRt.thread.isTitleGenerated = true;
        }
      } else if (incomingGenerated === false) {
        existingRt.thread.title = incomingTitle;
        existingRt.thread.isTitleGenerated = false;
      }
    }

    const isCurrentThread = action.payload.id === state.current_thread_id;
    if (
      !existingRt.streaming &&
      !existingRt.waiting_for_response &&
      !existingRt.error &&
      !isCurrentThread
    ) {
      const {
        title: _title,
        isTitleGenerated: _isTitleGenerated,
        messages: _messages,
        ...otherFields
      } = action.payload.thread;
      existingRt.thread = {
        ...existingRt.thread,
        ...otherFields,
      };
    }
  });

  builder.addCase(updateChatRuntimeFromSessionState, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (!rt) return;

    const sessionState = action.payload.session_state;
    rt.streaming = sessionState === "generating";
    rt.waiting_for_response =
      sessionState === "generating" ||
      sessionState === "executing_tools" ||
      sessionState === "waiting_ide";
    rt.prevent_send = false;

    if (sessionState === "paused") {
      rt.confirmation.pause = true;
      if (rt.confirmation.pause_reasons.length === 0) {
        state.sse_refresh_requested = action.payload.id;
      }
    } else if (sessionState === "idle" || sessionState === "error") {
      rt.confirmation.pause = false;
      rt.confirmation.pause_reasons = [];
    }

    if (sessionState === "error") {
      rt.error = action.payload.error ?? "An error occurred";
    } else if (sessionState === "idle") {
      rt.error = null;
    }
  });

  builder.addCase(saveTitle, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (rt) {
      rt.thread.title = action.payload.title;
      rt.thread.isTitleGenerated = action.payload.isTitleGenerated;
    }
  });

  builder.addCase(newIntegrationChat, (state, action) => {
    const currentRt = getCurrentRuntime(state);
    const newRuntime = createThreadRuntime(
      "agent",
      action.payload.integration,
      "configurator",
    );
    newRuntime.thread.last_user_message_id = action.payload.request_attempt_id;
    newRuntime.thread.messages = action.payload.messages;
    if (currentRt) {
      newRuntime.thread.model = currentRt.thread.model;
    }

    const newId = newRuntime.thread.id;
    state.threads[newId] = newRuntime;
    state.open_thread_ids.push(newId);
    state.current_thread_id = newId;
  });

  builder.addCase(setSendImmediately, (state, action) => {
    const rt = getCurrentRuntime(state);
    if (rt) rt.send_immediately = action.payload;
  });

  builder.addCase(setChatMode, (state, action) => {
    const rt = getCurrentRuntime(state);
    if (rt) rt.thread.mode = action.payload;
  });

  builder.addCase(setIntegrationData, (state, action) => {
    const rt = getCurrentRuntime(state);
    if (rt) rt.thread.integration = action.payload;
  });

  builder.addCase(setIsWaitingForResponse, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (rt) rt.waiting_for_response = action.payload.value;
  });

  builder.addCase(setMaxNewTokens, (state, action) => {
    const rt = getCurrentRuntime(state);
    if (rt) {
      rt.thread.currentMaximumContextTokens = action.payload;
      if (
        rt.thread.context_tokens_cap === undefined ||
        rt.thread.context_tokens_cap > action.payload
      ) {
        rt.thread.context_tokens_cap = action.payload;
      }
    }
  });

  builder.addCase(fixBrokenToolMessages, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (!rt || rt.thread.messages.length === 0) return;
    const lastMessage = rt.thread.messages[rt.thread.messages.length - 1];
    if (!isToolCallMessage(lastMessage)) return;
    if (lastMessage.tool_calls.every(validateToolCall)) return;
    const validToolCalls = lastMessage.tool_calls.filter(validateToolCall);
    const messages = rt.thread.messages.slice(0, -1);
    const newMessage = { ...lastMessage, tool_calls: validToolCalls };
    rt.thread.messages = [...messages, newMessage];
  });

  builder.addCase(upsertToolCall, (state, action) => {
    const rt = getRuntime(state, action.payload.chatId);
    if (rt) {
      maybeAppendToolCallResultFromIdeToMessages(
        rt.thread.messages,
        action.payload.toolCallId,
        action.payload.accepted,
        action.payload.replaceOnly,
      );
    }
  });

  builder.addCase(setIncreaseMaxTokens, (state, action) => {
    const rt = getCurrentRuntime(state);
    if (rt) rt.thread.increase_max_tokens = action.payload;
  });

  builder.addCase(setIncludeProjectInfo, (state, action) => {
    const rt = getRuntime(state, action.payload.chatId);
    if (rt) rt.thread.include_project_info = action.payload.value;
  });

  builder.addCase(setContextTokensCap, (state, action) => {
    const rt = getRuntime(state, action.payload.chatId);
    if (rt) rt.thread.context_tokens_cap = action.payload.value;
  });

  builder.addCase(setThreadPauseReasons, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (rt) {
      rt.confirmation.pause = true;
      rt.confirmation.pause_reasons = action.payload.pauseReasons;
      rt.confirmation.status.wasInteracted = false;
      rt.confirmation.status.confirmationStatus = false;
      rt.streaming = false;
      rt.waiting_for_response = false;
    }
  });

  builder.addCase(clearThreadPauseReasons, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (rt) {
      rt.confirmation.pause = false;
      rt.confirmation.pause_reasons = [];
    }
  });

  builder.addCase(setThreadConfirmationStatus, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (rt) {
      rt.confirmation.status.wasInteracted = action.payload.wasInteracted;
      rt.confirmation.status.confirmationStatus =
        action.payload.confirmationStatus;
    }
  });

  builder.addCase(addThreadImage, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (rt && rt.attached_images.length < 5) {
      rt.attached_images.push(action.payload.image);
    }
  });

  builder.addCase(removeThreadImageByIndex, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (rt) {
      rt.attached_images = rt.attached_images.filter(
        (_, index) => index !== action.payload.index,
      );
    }
  });

  builder.addCase(resetThreadImages, (state, action) => {
    const rt = getRuntime(state, action.payload.id);
    if (rt) {
      rt.attached_images = [];
    }
  });

  builder.addCase(applyChatEvent, (state, action) => {
    const { chat_id, ...event } = action.payload;

    const rt = getRuntime(state, chat_id);

    switch (event.type) {
      case "snapshot": {
        const existingRuntime = rt;
        const existing = existingRuntime?.thread;
        const snapshotMessages = (event.messages as ChatMessages).map(
          normalizeMessage,
        );

        const backendModel = event.thread.model.trim();
        const backendToolUse = event.thread.tool_use;
        const backendMode = event.thread.mode;

        const snapshotTaskMeta = event.thread.task_meta ?? existing?.task_meta;
        const isTaskChat =
          Boolean(existing?.is_task_chat) || Boolean(snapshotTaskMeta?.task_id);

        const snapshotTitle = event.thread.title;
        const existingTitle = existingRuntime?.thread.title;
        const snapshotTitleGenerated = event.thread.is_title_generated;
        const existingTitleGenerated =
          existingRuntime?.thread.isTitleGenerated === true;
        const useSnapshotTitle =
          !existingTitle ||
          existingTitle === "New Chat" ||
          (snapshotTitleGenerated && !existingTitleGenerated);

        const thread: ChatThread = {
          id: event.thread.id,
          messages: snapshotMessages,
          model: backendModel || (existing?.model ?? ""),
          title: useSnapshotTitle ? snapshotTitle : existingTitle,
          tool_use: isToolUse(backendToolUse)
            ? backendToolUse
            : existing?.tool_use && isToolUse(existing.tool_use)
              ? existing.tool_use
              : "agent",
          mode: normalizeLegacyMode(backendMode || existing?.mode),
          boost_reasoning: event.thread.boost_reasoning,
          context_tokens_cap:
            event.thread.context_tokens_cap ?? existing?.context_tokens_cap,
          include_project_info: event.thread.include_project_info,
          checkpoints_enabled: event.thread.checkpoints_enabled,
          isTitleGenerated:
            existingRuntime?.thread.isTitleGenerated ??
            event.thread.is_title_generated,
          automatic_patch:
            event.thread.automatic_patch ?? existing?.automatic_patch ?? false,
          increase_max_tokens: existing?.increase_max_tokens ?? false,
          new_chat_suggested: { wasSuggested: false },
          is_task_chat: isTaskChat,
          task_meta: snapshotTaskMeta,
        };

        const snapshotState = event.runtime.state as string;
        const snapshotStreaming = snapshotState === "generating";
        const snapshotWaiting =
          snapshotState === "generating" ||
          snapshotState === "executing_tools" ||
          snapshotState === "waiting_ide";

        const newRt: ChatThreadRuntime = {
          thread,
          streaming: snapshotStreaming,
          waiting_for_response: snapshotWaiting,
          prevent_send: false,
          error: event.runtime.error ?? null,
          queued_items: event.runtime
            .queued_items as ChatThreadRuntime["queued_items"],
          send_immediately: existingRuntime?.send_immediately ?? false,
          attached_images: existingRuntime?.attached_images ?? [],
          confirmation: {
            pause: event.runtime.paused,
            pause_reasons: event.runtime
              .pause_reasons as ToolConfirmationPauseReason[],
            status: existingRuntime?.confirmation.status ?? {
              wasInteracted: false,
              confirmationStatus: true,
            },
          },
          snapshot_received: true,
        };

        state.threads[chat_id] = newRt;

        if (!isTaskChat && !state.open_thread_ids.includes(chat_id)) {
          state.open_thread_ids.push(chat_id);
        }
        if (!state.current_thread_id) {
          state.current_thread_id = chat_id;
        }
        break;
      }

      case "thread_updated": {
        if (!rt) break;
        const { type: _, ...params } = event;
        if ("model" in params && typeof params.model === "string")
          rt.thread.model = params.model;
        if ("mode" in params && typeof params.mode === "string") {
          rt.thread.mode = normalizeLegacyMode(params.mode);
        }
        if (
          "boost_reasoning" in params &&
          typeof params.boost_reasoning === "boolean"
        )
          rt.thread.boost_reasoning = params.boost_reasoning;
        if ("tool_use" in params && typeof params.tool_use === "string") {
          rt.thread.tool_use = isToolUse(params.tool_use)
            ? params.tool_use
            : rt.thread.tool_use;
        }
        if ("context_tokens_cap" in params) {
          rt.thread.context_tokens_cap =
            params.context_tokens_cap == null
              ? undefined
              : (params.context_tokens_cap as number);
        }
        if (
          "include_project_info" in params &&
          typeof params.include_project_info === "boolean"
        )
          rt.thread.include_project_info = params.include_project_info;
        if (
          "checkpoints_enabled" in params &&
          typeof params.checkpoints_enabled === "boolean"
        )
          rt.thread.checkpoints_enabled = params.checkpoints_enabled;
        if (
          "automatic_patch" in params &&
          typeof params.automatic_patch === "boolean"
        )
          rt.thread.automatic_patch = params.automatic_patch;
        if ("task_meta" in params && params.task_meta != null) {
          rt.thread.task_meta = params.task_meta as ChatThread["task_meta"];
          rt.thread.is_task_chat = true;
          state.open_thread_ids = state.open_thread_ids.filter(
            (id) => id !== chat_id,
          );
        }
        break;
      }

      case "message_added": {
        if (!rt) break;
        const msg = normalizeMessage(event.message);
        const messageId = "message_id" in msg ? msg.message_id : null;
        if (messageId) {
          const existingIdx = rt.thread.messages.findIndex(
            (m) => "message_id" in m && m.message_id === messageId,
          );
          if (existingIdx >= 0) {
            const existing = rt.thread.messages[existingIdx];
            if (isAssistantMessage(existing) && isAssistantMessage(msg)) {
              const merged: AssistantMessage = {
                ...msg,
                tool_calls: msg.tool_calls ?? existing.tool_calls,
                reasoning_content:
                  msg.reasoning_content ?? existing.reasoning_content,
                thinking_blocks:
                  msg.thinking_blocks ?? existing.thinking_blocks,
                citations: msg.citations ?? existing.citations,
                usage: msg.usage ?? existing.usage,
                finish_reason: msg.finish_reason ?? existing.finish_reason,
              };
              rt.thread.messages[existingIdx] = merged;
            } else {
              rt.thread.messages[existingIdx] = msg;
            }
            break;
          }
        }
        const clampedIndex = Math.min(event.index, rt.thread.messages.length);
        rt.thread.messages.splice(clampedIndex, 0, msg);
        break;
      }

      case "message_updated": {
        if (!rt) break;
        const idx = rt.thread.messages.findIndex(
          (m) => "message_id" in m && m.message_id === event.message_id,
        );
        if (idx >= 0) {
          rt.thread.messages[idx] = normalizeMessage(event.message);
        }
        break;
      }

      case "message_removed": {
        if (!rt) break;
        rt.thread.messages = rt.thread.messages.filter(
          (m) => !("message_id" in m) || m.message_id !== event.message_id,
        );
        break;
      }

      case "messages_truncated": {
        if (!rt) break;
        const clampedIndex = Math.min(
          event.from_index,
          rt.thread.messages.length,
        );
        rt.thread.messages = rt.thread.messages.slice(0, clampedIndex);
        break;
      }

      case "stream_started": {
        if (!rt) break;
        rt.streaming = true;
        rt.waiting_for_response = true;
        rt.thread.messages.push({
          role: "assistant",
          content: "",
          message_id: event.message_id,
        } as ChatMessages[number]);
        break;
      }

      case "stream_delta": {
        if (!rt) break;
        const msgIdx = rt.thread.messages.findIndex(
          (m) => "message_id" in m && m.message_id === event.message_id,
        );
        if (msgIdx >= 0) {
          const msg = rt.thread.messages[msgIdx];
          rt.thread.messages[msgIdx] = applyDeltaOps(
            msg as Parameters<typeof applyDeltaOps>[0],
            event.ops,
          );
          state.stream_version = (state.stream_version + 1) % 1_000_000;
        }
        break;
      }

      case "stream_finished": {
        if (!rt) break;
        rt.streaming = false;
        if (
          event.finish_reason === "stop" ||
          event.finish_reason === "length" ||
          event.finish_reason === "abort" ||
          event.finish_reason === "error"
        ) {
          rt.waiting_for_response = false;
        }
        const msgIdx = rt.thread.messages.findIndex(
          (m) => "message_id" in m && m.message_id === event.message_id,
        );
        if (msgIdx >= 0 && isAssistantMessage(rt.thread.messages[msgIdx])) {
          const msg = rt.thread.messages[msgIdx] as AssistantMessage;
          if (event.finish_reason && !msg.finish_reason) {
            msg.finish_reason =
              event.finish_reason as AssistantMessage["finish_reason"];
          }
        }
        break;
      }

      case "pause_required": {
        if (!rt) break;
        rt.streaming = false;
        rt.waiting_for_response = false;
        rt.confirmation.pause = true;
        rt.confirmation.pause_reasons =
          event.reasons as ToolConfirmationPauseReason[];
        rt.confirmation.status.wasInteracted = false;
        rt.confirmation.status.confirmationStatus = false;
        break;
      }

      case "pause_cleared": {
        if (!rt) break;
        rt.confirmation.pause = false;
        rt.confirmation.pause_reasons = [];
        rt.confirmation.status.wasInteracted = false;
        rt.confirmation.status.confirmationStatus = true;
        break;
      }

      case "ide_tool_required": {
        break;
      }

      case "subchat_update": {
        if (!rt) break;
        for (const msg of rt.thread.messages) {
          if (!isAssistantMessage(msg) || !msg.tool_calls) continue;
          const tc = msg.tool_calls.find((t) => t.id === event.tool_call_id);
          if (tc) {
            if (event.subchat_id === "") {
              tc.subchat = undefined;
              tc.subchat_log = [];
              tc.attached_files = [];
            } else {
              tc.subchat = event.subchat_id;
              const isToolNotification = event.subchat_id.includes("/tool:");
              if (!isToolNotification) {
                const prev = tc.subchat_log ?? [];
                if (prev[prev.length - 1] !== event.subchat_id) {
                  tc.subchat_log = [...prev, event.subchat_id].slice(-50);
                }
              }
            }
            if (event.attached_files && event.attached_files.length > 0) {
              tc.attached_files = [
                ...(tc.attached_files ?? []),
                ...event.attached_files.filter(
                  (f) => !tc.attached_files?.includes(f),
                ),
              ];
            }
            break;
          }
        }
        break;
      }

      case "ack":
        break;

      case "queue_updated": {
        if (!rt) break;
        rt.queued_items =
          event.queued_items as ChatThreadRuntime["queued_items"];
        break;
      }
    }
  });

  builder.addCase(requestSseRefresh, (state, action) => {
    state.sse_refresh_requested = action.payload.chatId;
  });

  builder.addCase(clearSseRefreshRequest, (state) => {
    state.sse_refresh_requested = null;
  });

  builder.addMatcher(
    capsApi.endpoints.getCaps.matchFulfilled,
    (state, action) => {
      const defaultModel = action.payload.chat_default_model;
      const rt = getCurrentRuntime(state);
      if (!rt) return;

      const model = rt.thread.model || defaultModel;
      if (!(model in action.payload.chat_models)) return;

      const currentModelMaximumContextTokens =
        action.payload.chat_models[model].n_ctx;

      rt.thread.currentMaximumContextTokens = currentModelMaximumContextTokens;

      if (
        rt.thread.context_tokens_cap === undefined ||
        rt.thread.context_tokens_cap > currentModelMaximumContextTokens
      ) {
        rt.thread.context_tokens_cap = currentModelMaximumContextTokens;
      }
    },
  );

  builder.addMatcher(
    commandsApi.endpoints.getCommandPreview.matchFulfilled,
    (state, action) => {
      const rt = getCurrentRuntime(state);
      if (rt) {
        rt.thread.currentMaximumContextTokens = action.payload.number_context;
        rt.thread.currentMessageContextTokens = action.payload.current_context;
      }
    },
  );
});

export function maybeAppendToolCallResultFromIdeToMessages(
  messages: Draft<ChatMessages>,
  toolCallId: string,
  accepted: boolean | "indeterminate",
  replaceOnly = false,
) {
  const hasDiff = messages.find(
    (d) => isDiffMessage(d) && d.tool_call_id === toolCallId,
  );
  if (hasDiff) return;

  const maybeToolResult = messages.find(
    (d) => isToolMessage(d) && d.tool_call_id === toolCallId,
  );

  const toolCalls = messages.reduce<ToolCall[]>((acc, message) => {
    if (!isAssistantMessage(message)) return acc;
    if (!message.tool_calls) return acc;
    return acc.concat(message.tool_calls);
  }, []);

  const maybeToolCall = toolCalls.find(
    (toolCall) => toolCall.id === toolCallId,
  );

  const message = messageForToolCall(accepted, maybeToolCall);

  if (replaceOnly && !maybeToolResult) return;

  if (
    maybeToolResult &&
    isToolMessage(maybeToolResult) &&
    typeof maybeToolResult.content === "string"
  ) {
    maybeToolResult.content = message;
    return;
  } else if (
    maybeToolResult &&
    isToolMessage(maybeToolResult) &&
    Array.isArray(maybeToolResult.content)
  ) {
    maybeToolResult.content.push({
      m_type: "text",
      m_content: message,
    });
    return;
  }

  const assistantMessageIndex = messages.findIndex((message) => {
    if (!isAssistantMessage(message)) return false;
    return message.tool_calls?.find((toolCall) => toolCall.id === toolCallId);
  });

  if (assistantMessageIndex === -1) return;
  const toolMessage: ToolMessage = {
    role: "tool",
    tool_call_id: toolCallId,
    content: message,
    tool_failed: false,
  };

  messages.splice(assistantMessageIndex + 1, 0, toolMessage);
}

function messageForToolCall(
  accepted: boolean | "indeterminate",
  toolCall?: ToolCall,
) {
  if (accepted === false && toolCall?.function.name) {
    return `Whoops the user didn't like the command ${toolCall.function.name}. Stop and ask for correction from the user.`;
  }
  if (accepted === false) return "The user rejected the changes.";
  if (accepted === true) return "The user accepted the changes.";
  return "The user may have made modifications to changes.";
}
