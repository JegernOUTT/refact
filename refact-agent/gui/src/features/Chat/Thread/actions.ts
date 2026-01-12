import { createAction, createAsyncThunk } from "@reduxjs/toolkit";
import {
  type PayloadWithIdAndTitle,
  type ChatThread,
  type PayloadWithId,
  type ToolUse,
  type ImageFile,
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
  type MessageContent,
} from "../../../services/refact/chatCommands";
import { selectLspPort, selectApiKey } from "../../Config/configSlice";

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
      } else if (String(m_type).startsWith("image/")) {
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

export const createChatWithId = createAction<{
  id: string;
  title?: string;
  isTaskChat?: boolean;
  mode?: string;
  taskMeta?: TaskMeta;
  model?: string;
}>("chatThread/createWithId");

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

export const setEnabledCheckpoints = createAction<boolean>(
  "chat/setEnabledCheckpoints",
);

export const setBoostReasoning = createAction<PayloadWithChatAndBoolean>(
  "chatThread/setBoostReasoning",
);

export const setAutomaticPatch = createAction<PayloadWithChatAndBoolean>(
  "chatThread/setAutomaticPatch",
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

export const clearSseRefreshRequest = createAction(
  "chatThread/clearSseRefreshRequest",
);
