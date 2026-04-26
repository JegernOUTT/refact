import { createAction, createAsyncThunk } from "@reduxjs/toolkit";
import { v4 as uuidv4 } from "uuid";
import {
  type PayloadWithIdAndTitle,
  type ChatThread,
  type PayloadWithId,
  type ToolUse,
  type ImageFile,
  type TextFile,
  IntegrationMeta,
  LspChatMode,
  PayloadWithChatAndMessageId,
  PayloadWithChatAndBoolean,
  PayloadWithChatAndNumber,
} from "./types";
import type { ToolConfirmationPauseReason } from "../../../services/refact";
import { type ChatMessages } from "../../../services/refact/types";
import type { AppDispatch, RootState } from "../../../app/store";
import { type SystemPrompts } from "../../../services/refact/prompts";
import { ChatHistoryItem } from "../../History/historySlice";
import { ideToolCallResponse } from "../../../hooks/useEventBusForIDE";
import {
  trajectoriesApi,
  trajectoryDataToChatThread,
  isUserMessage,
} from "../../../services/refact";
import {
  sendUserMessage,
  sendChatCommand,
  type MessageContent,
  updateChatParams,
} from "../../../services/refact/chatCommands";
import { selectLspPort, selectApiKey } from "../../Config/configSlice";
import { selectCurrentThreadId } from "./selectors";
import { push } from "../../Pages/pagesSlice";

function buildThreadParamsPatch(
  thread: ChatThread,
  isNewChat: boolean,
): Record<string, unknown> {
  const patch: Record<string, unknown> = {};
  if (isNewChat) {
    if (thread.tool_use) patch.tool_use = thread.tool_use;
    if (thread.mode) patch.mode = thread.mode;
  }
  if (thread.model.trim()) patch.model = thread.model;
  if ("boost_reasoning" in thread)
    patch.boost_reasoning = thread.boost_reasoning;
  if ("include_project_info" in thread)
    patch.include_project_info = thread.include_project_info;
  if ("context_tokens_cap" in thread)
    patch.context_tokens_cap = thread.context_tokens_cap;
  if ("temperature" in thread) patch.temperature = thread.temperature;
  if ("frequency_penalty" in thread)
    patch.frequency_penalty = thread.frequency_penalty;
  if ("max_tokens" in thread) patch.max_tokens = thread.max_tokens;
  if ("reasoning_effort" in thread)
    patch.reasoning_effort = thread.reasoning_effort;
  if ("thinking_budget" in thread)
    patch.thinking_budget = thread.thinking_budget;
  if ("parallel_tool_calls" in thread)
    patch.parallel_tool_calls = thread.parallel_tool_calls;
  if ("auto_enrichment_enabled" in thread)
    patch.auto_enrichment_enabled = thread.auto_enrichment_enabled;
  return patch;
}

export { buildThreadParamsPatch };

function toMessageContent(
  content: import("../../../services/refact/types").UserMessage["content"],
): MessageContent {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  const out: (
    | { type: "text"; text: string }
    | { type: "image_url"; image_url: { url: string } }
  )[] = [];
  for (const item of content) {
    if ("type" in item && "text" in item && (item.type as string) === "text") {
      out.push({ type: "text", text: item.text });
    } else if ("type" in item && "image_url" in item) {
      out.push({ type: "image_url", image_url: item.image_url });
    } else if ("m_type" in item && "m_content" in item) {
      const { m_type, m_content } = item;
      if (m_type === "text") {
        out.push({ type: "text", text: String(m_content) });
      } else if (
        String(m_type).startsWith("image/") &&
        !String(m_type).includes("svg")
      ) {
        out.push({
          type: "image_url",
          image_url: { url: `data:${m_type};base64,${String(m_content)}` },
        });
      }
    }
  }
  return out.length ? out : "";
}

export const newChatAction = createAction<Partial<ChatThread> | undefined>(
  "chatThread/new",
);

export interface TaskMeta {
  task_id: string;
  role: string;
  agent_id?: string;
  card_id?: string;
}

export const sendIdeMessagesToCurrentChat = createAsyncThunk(
  "chatThread/sendIdeMessagesToCurrentChat",
  async (arg: { messages: ChatMessages; priority?: boolean }, api) => {
    const state = api.getState() as RootState;
    const chatId = selectCurrentThreadId(state);
    const port = selectLspPort(state);
    const apiKey = selectApiKey(state) ?? undefined;
    if (!chatId || !port) return;

    const runtime = state.chat.threads[chatId];
    if (!runtime) return;

    const isNewChat = runtime.thread.messages.length === 0;

    const patch = buildThreadParamsPatch(runtime.thread, isNewChat);
    if (Object.keys(patch).length > 0) {
      await sendChatCommand(chatId, port, apiKey, {
        type: "set_params",
        patch,
      });
    }

    for (const m of arg.messages) {
      if (!isUserMessage(m)) continue;
      const content = toMessageContent(m.content);
      const empty =
        typeof content === "string"
          ? content.trim().length === 0
          : content.length === 0;
      if (empty) continue;
      await sendUserMessage(chatId, content, port, apiKey, arg.priority);
    }
  },
);

export const createChatWithId = createAction<{
  id: string;
  title?: string;
  isTaskChat?: boolean;
  mode?: string;
  taskMeta?: TaskMeta;
  model?: string;
  parentId?: string;
  linkType?: string;
}>("chatThread/createWithId");

const SETUP_START_MESSAGES: Record<string, string> = {
  setup: "Start project setup for this repository.",
  setup_skills: "Help me set up project skills.",
  setup_agents_md: "Help me create or update AGENTS.md instructions.",
  setup_mcp: "Help me find and configure MCPs for this project.",
  setup_commands: "Help me define project commands.",
  setup_subagents: "Help me define project subagents.",
};

export const openChatInModeAndStart = createAsyncThunk<
  undefined,
  { mode: string; initialMessage?: string },
  { dispatch: AppDispatch; state: RootState }
>(
  "chatThread/openChatInModeAndStart",
  async ({ mode, initialMessage }, api) => {
    const chatId = uuidv4();
    api.dispatch(createChatWithId({ id: chatId, mode }));
    api.dispatch(push({ name: "chat" }));

    const state = api.getState();
    const port = selectLspPort(state);
    if (!port) return undefined;

    const apiKey = selectApiKey(state) ?? undefined;
    const startMessage =
      initialMessage ?? (SETUP_START_MESSAGES[mode] || "Start setup.");

    await updateChatParams(chatId, { mode }, port, apiKey);
    await sendUserMessage(chatId, startMessage, port, apiKey);
  },
);

export const newChatWithInitialMessages = createAsyncThunk(
  "chatThread/newChatWithInitialMessages",
  async (
    arg: { title?: string; messages: ChatMessages; priority?: boolean },
    api,
  ) => {
    api.dispatch(newChatAction({ title: arg.title }));
    const state = api.getState() as RootState;
    const chatId = state.chat.current_thread_id;
    const port = selectLspPort(state);
    const apiKey = selectApiKey(state) ?? undefined;
    if (!chatId || !port) return;

    const runtime = state.chat.threads[chatId];
    if (runtime && runtime.thread.messages.length === 0) {
      const patch = buildThreadParamsPatch(runtime.thread, true);
      if (Object.keys(patch).length > 0) {
        await sendChatCommand(chatId, port, apiKey, {
          type: "set_params",
          patch,
        });
      }
    }

    for (const m of arg.messages) {
      if (!isUserMessage(m)) continue;
      const content = toMessageContent(m.content);
      const empty =
        typeof content === "string"
          ? content.trim().length === 0
          : content.length === 0;
      if (empty) continue;
      await sendUserMessage(chatId, content, port, apiKey, arg.priority);
    }
  },
);

export const newIntegrationChat = createAction<{
  integration: IntegrationMeta;
  messages: ChatMessages;
  request_attempt_id: string;
}>("chatThread/newIntegrationChat");

export const setLastUserMessageId = createAction<PayloadWithChatAndMessageId>(
  "chatThread/setLastUserMessageId",
);

export const setIsNewChatSuggested = createAction<PayloadWithChatAndBoolean>(
  "chatThread/setIsNewChatSuggested",
);

export const setIsNewChatSuggestionRejected =
  createAction<PayloadWithChatAndBoolean>(
    "chatThread/setIsNewChatSuggestionRejected",
  );

export const backUpMessages = createAction<
  PayloadWithId & {
    messages: ChatThread["messages"];
  }
>("chatThread/backUpMessages");

export const setChatModel = createAction<string>("chatThread/setChatModel");
export const getSelectedChatModel = (state: RootState) => {
  const runtime = state.chat.threads[state.chat.current_thread_id] as
    | { thread: { model: string } }
    | undefined;
  return runtime?.thread.model ?? "";
};

export const setSystemPrompt = createAction<SystemPrompts>(
  "chatThread/setSystemPrompt",
);

export const removeChatFromCache = createAction<PayloadWithId>(
  "chatThread/removeChatFromCache",
);

export const restoreChat = createAction<ChatHistoryItem>(
  "chatThread/restoreChat",
);

export const updateOpenThread = createAction<{
  id: string;
  thread: Partial<ChatThread>;
}>("chatThread/updateOpenThread");

export const updateChatRuntimeFromSessionState = createAction<{
  id: string;
  session_state:
    | "idle"
    | "generating"
    | "executing_tools"
    | "paused"
    | "waiting_ide"
    | "waiting_user_input"
    | "completed"
    | "error";
  error?: string;
}>("chatThread/updateChatRuntimeFromSessionState");

export const switchToThread = createAction<
  PayloadWithId & { openTab?: boolean }
>("chatThread/switchToThread");

export const closeThread = createAction<PayloadWithId & { force?: boolean }>(
  "chatThread/closeThread",
);

export const setThreadPauseReasons = createAction<{
  id: string;
  pauseReasons: ToolConfirmationPauseReason[];
}>("chatThread/setPauseReasons");

export const clearThreadPauseReasons = createAction<PayloadWithId>(
  "chatThread/clearPauseReasons",
);

export const setThreadConfirmationStatus = createAction<{
  id: string;
  wasInteracted: boolean;
  confirmationStatus: boolean;
}>("chatThread/setConfirmationStatus");

export const addThreadImage = createAction<{ id: string; image: ImageFile }>(
  "chatThread/addImage",
);

export const removeThreadImageByIndex = createAction<{
  id: string;
  index: number;
}>("chatThread/removeImageByIndex");

export const resetThreadImages = createAction<PayloadWithId>(
  "chatThread/resetImages",
);

export const addThreadTextFile = createAction<{ id: string; file: TextFile }>(
  "chatThread/addTextFile",
);

export const removeThreadTextFileByIndex = createAction<{
  id: string;
  index: number;
}>("chatThread/removeTextFileByIndex");

export const resetThreadTextFiles = createAction<PayloadWithId>(
  "chatThread/resetTextFiles",
);

export const clearChatError = createAction<PayloadWithId>(
  "chatThread/clearError",
);

export const enableSend = createAction<PayloadWithId>("chatThread/enableSend");
export const setPreventSend = createAction<PayloadWithId>(
  "chatThread/preventSend",
);
export const setAreFollowUpsEnabled = createAction<boolean>(
  "chat/setAreFollowUpsEnabled",
);

export const setToolUse = createAction<ToolUse>("chatThread/setToolUse");

export const setThreadMode = createAction<{
  chatId: string;
  mode: string;
  threadDefaults?: {
    include_project_info?: boolean;
    checkpoints_enabled?: boolean;
    auto_approve_editing_tools?: boolean;
    auto_approve_dangerous_commands?: boolean;
  };
}>("chatThread/setThreadMode");

export const setEnabledCheckpoints = createAction<boolean>(
  "chat/setEnabledCheckpoints",
);

export const setBoostReasoning = createAction<PayloadWithChatAndBoolean>(
  "chatThread/setBoostReasoning",
);

export const setAutoApproveEditingTools =
  createAction<PayloadWithChatAndBoolean>(
    "chatThread/setAutoApproveEditingTools",
  );

export const setAutoApproveDangerousCommands =
  createAction<PayloadWithChatAndBoolean>(
    "chatThread/setAutoApproveDangerousCommands",
  );

export const saveTitle = createAction<PayloadWithIdAndTitle>(
  "chatThread/saveTitle",
);

export const setSendImmediately = createAction<boolean>(
  "chatThread/setSendImmediately",
);

export const setChatMode = createAction<LspChatMode>("chatThread/setChatMode");

export const setIntegrationData = createAction<Partial<IntegrationMeta> | null>(
  "chatThread/setIntegrationData",
);

export const setIsWaitingForResponse = createAction<{
  id: string;
  value: boolean;
}>("chatThread/setIsWaiting");

export const setMaxNewTokens = createAction<number>(
  "chatThread/setMaxNewTokens",
);

export const fixBrokenToolMessages = createAction<PayloadWithId>(
  "chatThread/fixBrokenToolMessages",
);

export const upsertToolCall = createAction<
  Parameters<typeof ideToolCallResponse>[0] & { replaceOnly?: boolean }
>("chatThread/upsertToolCall");

export const setIncreaseMaxTokens = createAction<boolean>(
  "chatThread/setIncreaseMaxTokens",
);

export const setIncludeProjectInfo = createAction<PayloadWithChatAndBoolean>(
  "chatThread/setIncludeProjectInfo",
);

export const setContextTokensCap = createAction<PayloadWithChatAndNumber>(
  "chatThread/setContextTokensCap",
);

export const setReasoningEffort = createAction<{
  chatId: string;
  value:
    | "none"
    | "minimal"
    | "low"
    | "medium"
    | "high"
    | "xhigh"
    | "max"
    | null;
}>("chatThread/setReasoningEffort");

export const setThinkingBudget = createAction<{
  chatId: string;
  value: number | null;
}>("chatThread/setThinkingBudget");

export const setTemperature = createAction<{
  chatId: string;
  value: number | null;
}>("chatThread/setTemperature");

export const setFrequencyPenalty = createAction<{
  chatId: string;
  value: number | null;
}>("chatThread/setFrequencyPenalty");

export const setMaxTokens = createAction<{
  chatId: string;
  value: number | null;
}>("chatThread/setMaxTokens");

export const setParallelToolCalls = createAction<{
  chatId: string;
  value: boolean | null;
}>("chatThread/setParallelToolCalls");

export const restoreChatFromBackend = createAsyncThunk<
  undefined,
  { id: string; fallback: ChatHistoryItem },
  { dispatch: AppDispatch; state: RootState }
>("chatThread/restoreChatFromBackend", async ({ id, fallback }, thunkApi) => {
  try {
    const result = await thunkApi
      .dispatch(
        trajectoriesApi.endpoints.getTrajectory.initiate(id, {
          forceRefetch: true,
        }),
      )
      .unwrap();

    const thread = trajectoryDataToChatThread(result);
    const historyItem: ChatHistoryItem = {
      ...thread,
      createdAt: result.created_at,
      updatedAt: result.updated_at,
      title: result.title,
      isTitleGenerated: result.isTitleGenerated,
    };

    thunkApi.dispatch(restoreChat(historyItem));
  } catch {
    thunkApi.dispatch(restoreChat(fallback));
  }
  return undefined;
});

import type { ChatEventEnvelope } from "../../../services/refact/chatSubscription";

export const applyChatEvent = createAction<ChatEventEnvelope>(
  "chatThread/applyChatEvent",
);

export type IdeToolRequiredPayload = {
  chatId: string;
  toolCallId: string;
  toolName: string;
  args: unknown;
};

export const ideToolRequired = createAction<IdeToolRequiredPayload>(
  "chatThread/ideToolRequired",
);

export const requestSseRefresh = createAction<{ chatId: string }>(
  "chatThread/requestSseRefresh",
);

export const setAutoEnrichmentEnabled = createAction<PayloadWithChatAndBoolean>(
  "chatThread/setAutoEnrichmentEnabled",
);

export const markMemoryEnrichmentUserTouched = createAction<{ chatId: string }>(
  "chatThread/markMemoryEnrichmentUserTouched",
);

export const setManualPreviewItems = createAction<{
  chatId: string;
  items: import("./types").ManualPreviewItem[];
}>("chatThread/setManualPreviewItems");

export const removeManualPreviewItem = createAction<{
  chatId: string;
  index: number;
}>("chatThread/removeManualPreviewItem");

export const clearManualPreviewItems = createAction<{ chatId: string }>(
  "chatThread/clearManualPreviewItems",
);

export const clearSseRefreshRequest = createAction(
  "chatThread/clearSseRefreshRequest",
);

export const setTaskWidgetExpanded = createAction<{
  id: string;
  expanded: boolean;
}>("chatThread/setTaskWidgetExpanded");
