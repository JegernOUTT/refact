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
} from "./types";
import type { ToolConfirmationPauseReason } from "../../../services/refact";
import { type ChatMessages } from "../../../services/refact/types";
import type { ChatEventEnvelope } from "../../../services/refact/chatSubscription";
import type { WorktreeMeta } from "../../../services/refact/worktrees";
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
import { selectConfig, selectApiKey } from "../../Config/configSlice";
import { hasUsableEngineEndpoint } from "../../../services/refact/apiUrl";
import { selectCurrentThreadId, selectMessagesById } from "./selectors";
import { push } from "../../Pages/pagesSlice";
import type {
  DiagnosticContext,
  BuddyConversationEntry,
  BuddyThreadMeta,
} from "../../Buddy/types";
import {
  buildBuddyInvestigationPrompt,
  buildBuddyInvestigationTitle,
  type BuddyInvestigationSource,
} from "../../Buddy/investigation";
import {
  createBuddyConversationRequest,
  fetchBuddyInvestigationContextRequest,
} from "../../../services/refact/buddy";

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
  if ("temperature" in thread) patch.temperature = thread.temperature;
  if ("frequency_penalty" in thread)
    patch.frequency_penalty = thread.frequency_penalty;
  if ("max_tokens" in thread) patch.max_tokens = thread.max_tokens;
  if (thread.context_tokens_cap !== undefined)
    patch.context_tokens_cap = thread.context_tokens_cap;
  if ("reasoning_effort" in thread)
    patch.reasoning_effort = thread.reasoning_effort;
  if ("thinking_budget" in thread)
    patch.thinking_budget = thread.thinking_budget;
  if ("parallel_tool_calls" in thread)
    patch.parallel_tool_calls = thread.parallel_tool_calls;
  if ("auto_enrichment_enabled" in thread)
    patch.auto_enrichment_enabled = thread.auto_enrichment_enabled;
  if ("auto_compact_enabled" in thread)
    patch.auto_compact_enabled = thread.auto_compact_enabled;
  Object.assign(patch, buildThreadScopePatch(thread));
  return patch;
}

function buildThreadScopePatch(thread: ChatThread): Record<string, unknown> {
  const patch: Record<string, unknown> = {};
  if (thread.task_meta) patch.task_meta = thread.task_meta;
  if (thread.worktree?.id) patch.worktree_id = thread.worktree.id;
  return patch;
}

export { buildThreadParamsPatch, buildThreadScopePatch };

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
  planner_chat_id?: string;
}

export const sendIdeMessagesToCurrentChat = createAsyncThunk(
  "chatThread/sendIdeMessagesToCurrentChat",
  async (
    arg: { chatId?: string; messages: ChatMessages; priority?: boolean },
    api,
  ) => {
    const state = api.getState() as RootState;
    const chatId = arg.chatId ?? selectCurrentThreadId(state);
    const connection = selectConfig(state);
    const apiKey = selectApiKey(state) ?? undefined;
    if (!chatId) return;

    const runtime = state.chat.threads[chatId];
    if (!runtime) return;

    const isNewChat = runtime.thread.messages.length === 0;

    const patch = buildThreadParamsPatch(runtime.thread, isNewChat);
    if (Object.keys(patch).length > 0) {
      await sendChatCommand(chatId, connection, apiKey, {
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
      await sendUserMessage(chatId, content, connection, apiKey, arg.priority);
    }
  },
);

export const createChatWithId = createAction<{
  id: string;
  title?: string;
  isTaskChat?: boolean;
  openTab?: boolean;
  mode?: string;
  taskMeta?: TaskMeta;
  model?: string;
  parentId?: string;
  linkType?: string;
  rootChatId?: string;
  worktree?: WorktreeMeta | null;
}>("chatThread/createWithId");

const SETUP_START_MESSAGES: Record<string, string> = {
  setup: "Start project setup for this repository.",
  setup_skills: "Help me set up project skills.",
  setup_agents_md: "Help me create or update AGENTS.md instructions.",
  setup_mcp: "Help me find and configure MCPs for this project.",
  setup_commands: "Help me define project commands.",
  setup_subagents: "Help me define project subagents.",
  setup_modes: "Help me create a custom chat mode for this project.",
  setup_hooks: "Help me set up lifecycle hooks for this project.",
  setup_knowledge: "Help me seed durable project knowledge.",
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
    const connection = selectConfig(state);
    if (!hasUsableEngineEndpoint(connection)) return undefined;

    const apiKey = selectApiKey(state) ?? undefined;
    const startMessage =
      initialMessage ?? (SETUP_START_MESSAGES[mode] || "Start setup.");

    await updateChatParams(chatId, { mode }, connection, apiKey);
    await sendUserMessage(chatId, startMessage, connection, apiKey);
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
    const connection = selectConfig(state);
    const apiKey = selectApiKey(state) ?? undefined;
    if (!chatId) return;

    const runtime = state.chat.threads[chatId];
    if (runtime && runtime.thread.messages.length === 0) {
      const patch = buildThreadParamsPatch(runtime.thread, true);
      if (Object.keys(patch).length > 0) {
        await sendChatCommand(chatId, connection, apiKey, {
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
      await sendUserMessage(chatId, content, connection, apiKey, arg.priority);
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

export type SetChatModelPayload = {
  chatId?: string;
  model: string;
  modelMaxContextTokens?: number;
  previousModelMaxContextTokens?: number;
};

export const setChatModel = createAction<SetChatModelPayload>(
  "chatThread/setChatModel",
);
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

export const reorderOpenThreads = createAction<{
  sourceId: string;
  targetId: string;
}>("chatThread/reorderOpenThreads");

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

export const setThreadWorktree = createAction<{
  chatId: string;
  worktree: WorktreeMeta | null;
}>("chatThread/setThreadWorktree");

export const setEnabledCheckpoints = createAction<
  boolean | PayloadWithChatAndBoolean
>("chat/setEnabledCheckpoints");

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

export const setSendImmediately = createAction<
  boolean | PayloadWithChatAndBoolean
>("chatThread/setSendImmediately");

export const setChatMode = createAction<
  LspChatMode | { chatId?: string; mode: LspChatMode }
>("chatThread/setChatMode");

export const setIntegrationData = createAction<
  | (Partial<IntegrationMeta> & { chatId?: string })
  | { chatId?: string; value: Partial<IntegrationMeta> | null }
  | null
>("chatThread/setIntegrationData");

export const setIsWaitingForResponse = createAction<{
  id: string;
  value: boolean;
}>("chatThread/setIsWaiting");

export const markThreadSseError = createAction<{
  id: string;
  error: string;
}>("chatThread/markSseError");

export const setMaxNewTokens = createAction<
  number | { chatId?: string; value: number }
>("chatThread/setMaxNewTokens");

export const fixBrokenToolMessages = createAction<PayloadWithId>(
  "chatThread/fixBrokenToolMessages",
);

export const upsertToolCall = createAction<
  Parameters<typeof ideToolCallResponse>[0] & { replaceOnly?: boolean }
>("chatThread/upsertToolCall");

export const setIncreaseMaxTokens = createAction<
  boolean | PayloadWithChatAndBoolean
>("chatThread/setIncreaseMaxTokens");

export const setIncludeProjectInfo = createAction<PayloadWithChatAndBoolean>(
  "chatThread/setIncludeProjectInfo",
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
          subscribe: false,
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

export const hydratePersistedChatTabs = createAction(
  "chatThread/hydratePersistedChatTabs",
);

export const requestSseRefresh = createAction<{ chatId: string }>(
  "chatThread/requestSseRefresh",
);

export const setAutoEnrichmentEnabled = createAction<PayloadWithChatAndBoolean>(
  "chatThread/setAutoEnrichmentEnabled",
);

export const setAutoCompactEnabled = createAction<PayloadWithChatAndBoolean>(
  "chatThread/setAutoCompactEnabled",
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

export const setTaskGoalExpanded = createAction<{
  id: string;
  expanded: boolean;
}>("chatThread/setTaskGoalExpanded");

export const openBuddyChat = createAction<{ chat_id: string; title?: string }>(
  "chat/openBuddyChat",
);

export const newBuddyChatAction = createAction<{ chat_id: string }>(
  "chat/newBuddyChat",
);

type OpenableBuddyChatKind = "chat" | "setup" | "workflow";

function normalizeOpenableBuddyChatKind(
  kind: BuddyConversationEntry["kind"],
): OpenableBuddyChatKind | null {
  if (kind === "chat" || kind === "setup" || kind === "workflow") return kind;
  return null;
}

function describeError(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  return "Unknown error";
}

export const openExistingBuddyChat = createAsyncThunk<
  undefined,
  BuddyConversationEntry,
  { dispatch: AppDispatch; state: RootState }
>("chat/openExistingBuddyChat", async (entry, thunkApi) => {
  const openableKind = normalizeOpenableBuddyChatKind(entry.kind);
  if (!openableKind) {
    throw new Error(`Buddy conversation kind ${entry.kind} cannot be opened`);
  }

  const buddyMeta: BuddyThreadMeta = {
    is_buddy_chat: true,
    buddy_chat_kind: openableKind,
    workflow_id: openableKind === "workflow" ? entry.id : null,
  };

  const fallback: ChatHistoryItem = {
    id: entry.id,
    title: entry.title || "Untitled",
    model: "",
    mode: "buddy",
    tool_use: "agent",
    messages: [],
    boost_reasoning: false,
    context_tokens_cap: undefined,
    include_project_info: true,
    increase_max_tokens: false,
    last_user_message_id: "",
    buddy_meta: buddyMeta,
    createdAt: entry.created_at,
    updatedAt: entry.updated_at,
  };
  let hydrated = false;

  try {
    const result = await thunkApi
      .dispatch(
        trajectoriesApi.endpoints.getTrajectory.initiate(entry.id, {
          forceRefetch: true,
          subscribe: false,
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
      buddy_meta: buddyMeta,
    };
    thunkApi.dispatch(restoreChat(historyItem));
    hydrated = true;
  } catch (error) {
    const message = `Buddy could not load saved messages for this chat. ${describeError(
      error,
    )}`;
    thunkApi.dispatch(
      restoreChat({
        ...fallback,
        messages: [
          {
            role: "assistant",
            content: message,
            message_id: `buddy-hydration-failed-${entry.id}`,
            finish_reason: "error",
          },
        ],
        session_state: "error",
      }),
    );
    thunkApi.dispatch(
      updateChatRuntimeFromSessionState({
        id: entry.id,
        session_state: "error",
        error: message,
      }),
    );
  }

  if (hydrated) {
    thunkApi.dispatch(requestSseRefresh({ chatId: entry.id }));
  }
  thunkApi.dispatch(push({ name: "chat" }));

  return undefined;
});

export const startBuddyInvestigation = createAsyncThunk<
  { chat_id: string; title: string } | undefined,
  {
    triggerText: string;
    triggerSource: BuddyInvestigationSource;
    sourceChatId?: string;
    diagnostic?: DiagnosticContext | null;
  },
  { dispatch: AppDispatch; state: RootState }
>(
  "chat/startBuddyInvestigation",
  async ({ triggerText, triggerSource, sourceChatId, diagnostic }, api) => {
    const state = api.getState();
    const connection = selectConfig(state);
    if (!hasUsableEngineEndpoint(connection)) return undefined;

    const apiKey = selectApiKey(state) ?? undefined;
    const messages = sourceChatId
      ? selectMessagesById(state, sourceChatId)
      : [];
    const title = buildBuddyInvestigationTitle(triggerText);

    const [meta, context] = await Promise.all([
      createBuddyConversationRequest(connection, apiKey, { title }),
      fetchBuddyInvestigationContextRequest(connection, apiKey, {
        error: triggerText,
        source_file: diagnostic?.source_file ?? undefined,
        tool_name: diagnostic?.tool_name ?? undefined,
        chat_id: sourceChatId ?? diagnostic?.chat_id ?? undefined,
        diagnostic_id: diagnostic?.diagnostic_id,
        collected_at: diagnostic?.collected_at ?? undefined,
      }).catch(() => ({
        logs: "Investigation logs were unavailable.",
        internal_context: "Internal setup/config context was unavailable.",
        repo_owner: "smallcloudai",
        repo_name: "refact",
      })),
    ]);

    api.dispatch(newBuddyChatAction({ chat_id: meta.chat_id }));
    api.dispatch(openBuddyChat({ chat_id: meta.chat_id, title: meta.title }));
    api.dispatch(push({ name: "chat" }));

    try {
      await updateChatParams(
        meta.chat_id,
        {
          title: meta.title,
          mode: "buddy",
          buddy_meta: {
            is_buddy_chat: true,
            buddy_chat_kind: "investigation",
            workflow_id: null,
          },
          include_project_info: true,
        },
        connection,
        apiKey,
      );

      const prompt = buildBuddyInvestigationPrompt({
        triggerSource,
        triggerText,
        sourceChatId,
        messages,
        diagnostic,
        logs: context.logs,
        internalContext: context.internal_context,
        repoOwner: context.repo_owner,
        repoName: context.repo_name,
      });

      await sendUserMessage(
        meta.chat_id,
        prompt,
        connection,
        apiKey,
        undefined,
        undefined,
        true,
      );
    } catch (error) {
      api.dispatch(
        updateChatRuntimeFromSessionState({
          id: meta.chat_id,
          session_state: "error",
          error: `Buddy investigation setup failed. ${describeError(error)}`,
        }),
      );
      throw error;
    }

    return { chat_id: meta.chat_id, title: meta.title };
  },
);
