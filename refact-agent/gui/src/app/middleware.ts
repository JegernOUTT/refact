import type { RootState, AppDispatch } from "./store";
import {
  createListenerMiddleware,
  isAnyOf,
  isRejected,
} from "@reduxjs/toolkit";
import {
  newChatAction,
  restoreChat,
  newIntegrationChat,
  applyChatEvent,
  clearThreadPauseReasons,
  setThreadConfirmationStatus,
  setThreadPauseReasons,
  resetThreadImages,
  switchToThread,
  selectCurrentThreadId,
  ideToolRequired,
  saveTitle,
  setBoostReasoning,
  setIncludeProjectInfo,
  setContextTokensCap,
  setEnabledCheckpoints,
  setToolUse,
  setChatMode,
  setThreadMode,
  setChatModel,
  setAutoApproveEditingTools,
  setAutoApproveDangerousCommands,
  setIncreaseMaxTokens,
  setAreFollowUpsEnabled,
  setSystemPrompt,
  setReasoningEffort,
  setThinkingBudget,
  setTemperature,
  setMaxTokens,
  buildThreadParamsPatch,
} from "../features/Chat/Thread";
import { saveLastThreadParams } from "../utils/threadStorage";
import { statisticsApi } from "../services/refact/statistics";
import { integrationsApi } from "../services/refact/integrations";
import { dockerApi } from "../services/refact/docker";
import { capsApi, isCapsErrorResponse } from "../services/refact/caps";
import { promptsApi } from "../services/refact/prompts";
import { toolsApi } from "../services/refact/tools";
import { commandsApi, isDetailMessage } from "../services/refact/commands";
import { pathApi } from "../services/refact/path";
import { pingApi } from "../services/refact/ping";
import {
  clearError,
  setError,
  setIsAuthError,
} from "../features/Errors/errorsSlice";
import { setThemeMode, updateConfig } from "../features/Config/configSlice";
import { nextTip } from "../features/TipOfTheDay";
import { telemetryApi } from "../services/refact/telemetry";
import { tasksApi } from "../services/refact/tasks";
import { closeTask } from "../features/Tasks/tasksSlice";
import { closeThread } from "../features/Chat/Thread";
import { CONFIG_PATH_URL, FULL_PATH_URL } from "../services/refact/consts";
import {
  ideToolCallResponse,
  ideForceReloadProjectTreeFiles,
  ideTaskDone,
  ideAskQuestions,
} from "../hooks/useEventBusForIDE";
import { upsertToolCallIntoHistory } from "../features/History/historySlice";
import {
  isToolMessage,
  isDiffMessage,
  modelsApi,
  providersApi,
} from "../services/refact";
import { sendChatCommand } from "../services/refact/chatCommands";

const AUTH_ERROR_MESSAGE =
  "There is an issue with your API key. Check out your API Key or re-login";

export const listenerMiddleware = createListenerMiddleware();
const startListening = listenerMiddleware.startListening.withTypes<
  RootState,
  AppDispatch
>();

startListening({
  actionCreator: newChatAction,
  effect: async (_action, listenerApi) => {
    const state = listenerApi.getState();
    const chatId = state.chat.current_thread_id;

    [
      statisticsApi.util.resetApiState(),
      toolsApi.util.resetApiState(),
      commandsApi.util.resetApiState(),
    ].forEach((api) => listenerApi.dispatch(api));

    listenerApi.dispatch(resetThreadImages({ id: chatId }));
    listenerApi.dispatch(clearThreadPauseReasons({ id: chatId }));
    listenerApi.dispatch(
      setThreadConfirmationStatus({
        id: chatId,
        wasInteracted: false,
        confirmationStatus: true,
      }),
    );
    listenerApi.dispatch(clearError());

    // New chats are created client-side first; sync the initial params to backend
    // immediately so the first snapshot doesn't overwrite local defaults.
    const runtime = state.chat.threads[chatId];
    const port = state.config.lspPort;
    if (!runtime || !port || !chatId) return;

    try {
      const patch = buildThreadParamsPatch(runtime.thread, true);

      // If reasoning is enabled by defaults (new chat), ensure temperature is sent as null.
      // Otherwise backend may fall back to a numeric default (often 0), which is invalid
      // for reasoning-enabled providers.
      const isReasoningEnabled =
        Boolean(runtime.thread.boost_reasoning) ||
        runtime.thread.reasoning_effort != null ||
        runtime.thread.thinking_budget != null;
      if (isReasoningEnabled) {
        patch.temperature = null;
      }

      if (Object.keys(patch).length > 0) {
        await sendChatCommand(chatId, port, state.config.apiKey ?? undefined, {
          type: "set_params",
          patch,
        });
      }
    } catch {
      // Silently ignore - backend may not support this command
    }
  },
});

startListening({
  actionCreator: restoreChat,
  effect: (_action, listenerApi) => {
    const state = listenerApi.getState();
    const chatId = state.chat.current_thread_id;

    [
      statisticsApi.util.resetApiState(),
      toolsApi.util.resetApiState(),
      commandsApi.util.resetApiState(),
    ].forEach((api) => listenerApi.dispatch(api));

    listenerApi.dispatch(resetThreadImages({ id: chatId }));
    listenerApi.dispatch(clearError());
  },
});

// TODO: think about better cache invalidation approach instead of listening for an action dispatching globally
startListening({
  matcher: isAnyOf((d: unknown): d is ReturnType<typeof newIntegrationChat> =>
    newIntegrationChat.match(d),
  ),
  effect: (_action, listenerApi) => {
    [integrationsApi.util.resetApiState()].forEach((api) =>
      listenerApi.dispatch(api),
    );
    listenerApi.dispatch(clearError());
  },
});

startListening({
  // TODO: figure out why this breaks the tests when it's not a function :/
  matcher: isAnyOf(isRejected),
  effect: (action, listenerApi) => {
    if (
      capsApi.endpoints.getCaps.matchRejected(action) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isCapsErrorResponse(action.payload?.data)
          ? action.payload.data.detail
          : `fetching caps from lsp`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }
    if (
      toolsApi.endpoints.getToolGroups.matchRejected(action) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(action.payload?.data)
          ? action.payload.data.detail
          : `fetching tool groups from lsp`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }
    if (
      toolsApi.endpoints.checkForConfirmation.matchRejected(action) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(action.payload?.data)
          ? action.payload.data.detail
          : `confirmation check from lsp`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }
    if (
      promptsApi.endpoints.getPrompts.matchRejected(action) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(action.payload?.data)
          ? action.payload.data.detail.split("\n").slice(0, 2).join("\n")
          : `fetching system prompts.`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }

    if (
      integrationsApi.endpoints.getAllIntegrations.matchRejected(action) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(action.payload?.data)
          ? action.payload.data.detail
          : `fetching integrations.`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }

    if (
      integrationsApi.endpoints.deleteIntegration.matchRejected(action) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(action.payload?.data)
          ? action.payload.data.detail
          : `deleting integrations.`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }

    if (
      integrationsApi.endpoints.getIntegrationByPath.matchRejected(action) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(action.payload?.data)
          ? action.payload.data.detail
          : `fetching integrations.`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }

    if (
      dockerApi.endpoints.getAllDockerContainers.matchRejected(action) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(action.payload?.data)
          ? action.payload.data.detail
          : `fetching docker containers.`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }

    if (
      dockerApi.endpoints.getDockerContainersByImage.matchRejected(action) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(action.payload?.data)
          ? action.payload.data.detail
          : `fetching docker containers.`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }

    if (
      dockerApi.endpoints.getDockerContainersByLabel.matchRejected(action) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(action.payload?.data)
          ? action.payload.data.detail
          : `fetching docker containers.`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }

    if (
      dockerApi.endpoints.executeActionForDockerContainer.matchRejected(
        action,
      ) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(action.payload?.data)
          ? action.payload.data.detail
          : `fetching docker containers.`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }

    if (
      pathApi.endpoints.getFullPath.matchRejected(action) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(action.payload?.data)
          ? action.payload.data.detail
          : `getting full path of file.`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }

    if (
      (providersApi.endpoints.updateProvider.matchRejected(action) ||
        providersApi.endpoints.getProvider.matchRejected(action) ||
        providersApi.endpoints.getConfiguredProviders.matchRejected(action)) &&
      typeof action.meta === "object" &&
      "condition" in action.meta &&
      !action.meta.condition
    ) {
      const payload = action.payload as
        | { status?: number; data?: unknown }
        | undefined;
      const errorStatus = payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(payload?.data)
          ? (payload.data as { detail: string }).detail
          : `provider update error.`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }
    if (
      modelsApi.endpoints.getModels.matchRejected(action) &&
      !action.meta.condition
    ) {
      const errorStatus = action.payload?.status;
      const isAuthError = errorStatus === 401;
      const message = isAuthError
        ? AUTH_ERROR_MESSAGE
        : isDetailMessage(action.payload?.data)
          ? action.payload.data.detail
          : `provider update error.`;

      listenerApi.dispatch(setError(message));
      listenerApi.dispatch(setIsAuthError(isAuthError));
    }
  },
});

startListening({
  matcher: isAnyOf(
    providersApi.endpoints.updateProvider.matchFulfilled,
    providersApi.endpoints.oauthExchange.matchFulfilled,
  ),
  effect: (_action, listenerApi) => {
    listenerApi.dispatch(clearError());
    listenerApi.dispatch(capsApi.util.resetApiState());
    listenerApi.dispatch(modelsApi.util.resetApiState());
  },
});

startListening({
  actionCreator: updateConfig,
  effect: (_action, listenerApi) => {
    listenerApi.dispatch(pingApi.util.resetApiState());
  },
});

startListening({
  matcher: isAnyOf(restoreChat, newChatAction, updateConfig),
  effect: (action, listenerApi) => {
    const state = listenerApi.getState();
    const isUpdate = updateConfig.match(action);

    const host =
      isUpdate && action.payload.host ? action.payload.host : state.config.host;

    const completeManual = isUpdate
      ? action.payload.keyBindings?.completeManual
      : state.config.keyBindings?.completeManual;

    listenerApi.dispatch(
      nextTip({
        host,
        completeManual,
      }),
    );
  },
});

// Telemetry for path API
startListening({
  matcher: isAnyOf(
    pathApi.endpoints.getFullPath.matchFulfilled,
    pathApi.endpoints.getFullPath.matchRejected,
    pathApi.endpoints.customizationPath.matchFulfilled,
    pathApi.endpoints.customizationPath.matchRejected,
    pathApi.endpoints.privacyPath.matchFulfilled,
    pathApi.endpoints.privacyPath.matchRejected,
    pathApi.endpoints.integrationsPath.matchFulfilled,
    pathApi.endpoints.integrationsPath.matchRejected,
  ),
  effect: (action, listenerApi) => {
    if (pathApi.endpoints.getFullPath.matchFulfilled(action)) {
      const thunk = telemetryApi.endpoints.sendTelemetryNetEvent.initiate({
        url: FULL_PATH_URL,
        scope: "getFullPath",
        success: true,
        error_message: "",
      });
      void listenerApi.dispatch(thunk);
    }

    if (
      pathApi.endpoints.getFullPath.matchRejected(action) &&
      !action.meta.condition
    ) {
      const thunk = telemetryApi.endpoints.sendTelemetryNetEvent.initiate({
        url: FULL_PATH_URL,
        scope: "getFullPath",
        success: false,
        error_message: action.error.message ?? JSON.stringify(action.error),
      });
      void listenerApi.dispatch(thunk);
    }

    if (
      pathApi.endpoints.customizationPath.matchFulfilled(action) ||
      pathApi.endpoints.privacyPath.matchFulfilled(action) ||
      pathApi.endpoints.integrationsPath.matchFulfilled(action)
    ) {
      const thunk = telemetryApi.endpoints.sendTelemetryNetEvent.initiate({
        url: CONFIG_PATH_URL,
        scope: action.meta.arg.endpointName,
        success: true,
        error_message: "",
      });
      void listenerApi.dispatch(thunk);
    }

    if (
      (pathApi.endpoints.customizationPath.matchRejected(action) ||
        pathApi.endpoints.privacyPath.matchRejected(action) ||
        pathApi.endpoints.integrationsPath.matchRejected(action)) &&
      !action.meta.condition
    ) {
      const thunk = telemetryApi.endpoints.sendTelemetryNetEvent.initiate({
        url: CONFIG_PATH_URL,
        scope: action.meta.arg.endpointName,
        success: false,
        error_message: action.error.message ?? JSON.stringify(action.error),
      });
      void listenerApi.dispatch(thunk);
    }
  },
});

startListening({
  actionCreator: ideToolCallResponse,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const chatId = action.payload.chatId;
    const { toolCallId, accepted } = action.payload;

    listenerApi.dispatch(upsertToolCallIntoHistory(action.payload));

    const port = state.config.lspPort;
    if (!port) return;

    const apiKey = state.config.apiKey;
    const content =
      accepted === true
        ? "The user accepted the changes."
        : accepted === false
          ? "The user rejected the changes."
          : "The user applied the changes with modifications.";

    try {
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "ide_tool_result",
        tool_call_id: toolCallId,
        content,
        tool_failed: accepted === false,
      });
    } catch {
      // Silently ignore - backend may not support this command
    }
  },
});

startListening({
  matcher: isAnyOf(updateConfig.match, setThemeMode.match),
  effect: (_action, listenerApi) => {
    const appearance = listenerApi.getState().config.themeProps.appearance;
    if (appearance === "light" && document.body.className !== "vscode-light") {
      document.body.className = "vscode-light";
    } else if (
      appearance === "dark" &&
      document.body.className !== "vscode-dark"
    ) {
      document.body.className = "vscode-dark";
    }
  },
});

startListening({
  actionCreator: setThreadPauseReasons,
  effect: (action, listenerApi) => {
    const state = listenerApi.getState();
    const currentThreadId = selectCurrentThreadId(state);
    const threadIdNeedingConfirmation = action.payload.id;

    if (threadIdNeedingConfirmation !== currentThreadId) {
      listenerApi.dispatch(switchToThread({ id: threadIdNeedingConfirmation }));
    }
  },
});

startListening({
  actionCreator: saveTitle,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = action.payload.id;
    const title = action.payload.title;
    const isTitleGenerated = action.payload.isTitleGenerated;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { title, is_title_generated: isTitleGenerated },
      });
    } catch {
      // Silently ignore - backend may not support this command
    }
  },
});

startListening({
  actionCreator: applyChatEvent,
  effect: (action, listenerApi) => {
    const state = listenerApi.getState();
    if (state.config.host !== "jetbrains") return;
    if (!window.postIntellijMessage) return;

    const event = action.payload;
    if (event.type === "message_added") {
      const msg = event.message;
      if (isToolMessage(msg) || isDiffMessage(msg)) {
        window.postIntellijMessage(ideForceReloadProjectTreeFiles());
      }
    }
  },
});

startListening({
  actionCreator: applyChatEvent,
  effect: (action, listenerApi) => {
    const event = action.payload;
    if (event.type === "ide_tool_required") {
      listenerApi.dispatch(
        ideToolRequired({
          chatId: event.chat_id,
          toolCallId: event.tool_call_id,
          toolName: event.tool_name,
          args: event.args,
        }),
      );
    }
  },
});

// Type definitions for tool message content
interface TaskDoneContent {
  type: "task_done";
  summary?: string;
  knowledge_path?: string;
}

interface AskQuestionsContent {
  type: "ask_questions";
  questions: { id: string; type: string; text: string; options?: string[] }[];
}

type ToolMessageContent =
  | TaskDoneContent
  | AskQuestionsContent
  | { type: string };

function isTaskDoneContent(
  content: ToolMessageContent,
): content is TaskDoneContent {
  return content.type === "task_done";
}

function isAskQuestionsContent(
  content: ToolMessageContent,
): content is AskQuestionsContent {
  return (
    content.type === "ask_questions" &&
    "questions" in content &&
    Array.isArray(content.questions)
  );
}

let cachedPostMessage: ((message: Record<string, unknown>) => void) | null =
  null;

function getPostMessageForHost(): (message: Record<string, unknown>) => void {
  if (cachedPostMessage) return cachedPostMessage;
  if (window.acquireVsCodeApi) {
    cachedPostMessage = window.acquireVsCodeApi().postMessage;
  } else if (window.postIntellijMessage) {
    cachedPostMessage = window.postIntellijMessage;
  } else {
    cachedPostMessage = (msg) => window.postMessage(msg, "*");
  }
  return cachedPostMessage;
}

function isIdeHost(): boolean {
  return !!(window.acquireVsCodeApi ?? window.postIntellijMessage);
}

function safeParseJson(str: string): unknown {
  try {
    return JSON.parse(str);
  } catch {
    return undefined;
  }
}

startListening({
  actionCreator: applyChatEvent,
  effect: (action) => {
    if (!isIdeHost()) return;

    const event = action.payload;
    if (event.type !== "message_added") return;

    const msg = event.message;
    if (!isToolMessage(msg)) return;
    if (typeof msg.content !== "string") return;

    const parsed = safeParseJson(msg.content);
    if (!parsed || typeof parsed !== "object") return;

    const content = parsed as ToolMessageContent;
    const chatId = event.chat_id;
    const toolCallId = msg.tool_call_id;
    const postToIde = getPostMessageForHost();

    if (isTaskDoneContent(content)) {
      postToIde(
        ideTaskDone({
          chatId,
          toolCallId,
          summary: content.summary ?? "Task completed",
          knowledgePath: content.knowledge_path,
        }),
      );
    } else if (isAskQuestionsContent(content)) {
      postToIde(
        ideAskQuestions({
          chatId,
          toolCallId,
          questions: content.questions,
        }),
      );
    }
  },
});

// Sync thread params to backend when changed via Redux actions
startListening({
  actionCreator: setBoostReasoning,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { boost_reasoning: action.payload.value },
      });

      // When reasoning is enabled, temperature must be unset.
      // This avoids provider-side validation errors.
      if (action.payload.value) {
        await sendChatCommand(chatId, port, apiKey ?? undefined, {
          type: "set_params",
          patch: { temperature: null },
        });
      }
    } catch {
      // Silently ignore - backend may not support this command
    }
  },
});

startListening({
  actionCreator: setReasoningEffort,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { reasoning_effort: action.payload.value },
      });

      // Any explicit reasoning effort implies reasoning mode: unset temperature.
      if (action.payload.value != null) {
        await sendChatCommand(chatId, port, apiKey ?? undefined, {
          type: "set_params",
          patch: { temperature: null },
        });
      }
    } catch {
      // Silently ignore
    }
  },
});

startListening({
  actionCreator: setThinkingBudget,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { thinking_budget: action.payload.value },
      });

      // Any explicit thinking budget implies reasoning mode: unset temperature.
      if (action.payload.value != null) {
        await sendChatCommand(chatId, port, apiKey ?? undefined, {
          type: "set_params",
          patch: { temperature: null },
        });
      }
    } catch {
      // Silently ignore errors - user will see them via SSE events
    }
  },
});

startListening({
  actionCreator: setTemperature,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { temperature: action.payload.value },
      });
    } catch {
      // Silently ignore errors - user will see them via SSE events
    }
  },
});

startListening({
  actionCreator: setMaxTokens,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { max_tokens: action.payload.value },
      });
    } catch {
      // Silently ignore
    }
  },
});

startListening({
  actionCreator: setAutoApproveEditingTools,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { auto_approve_editing_tools: action.payload.value },
      });
    } catch {
      /* ignore */
    }
  },
});

startListening({
  actionCreator: setAutoApproveDangerousCommands,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { auto_approve_dangerous_commands: action.payload.value },
      });
    } catch {
      /* ignore */
    }
  },
});

startListening({
  actionCreator: setIncludeProjectInfo,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { include_project_info: action.payload.value },
      });
    } catch {
      // Silently ignore - backend may not support this command
    }
  },
});

startListening({
  actionCreator: setContextTokensCap,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { context_tokens_cap: action.payload.value },
      });
    } catch {
      // Silently ignore - backend may not support this command
    }
  },
});

startListening({
  actionCreator: setEnabledCheckpoints,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = state.chat.current_thread_id;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { checkpoints_enabled: action.payload },
      });
    } catch {
      // Silently ignore - backend may not support this command
    }
  },
});

startListening({
  actionCreator: setToolUse,
  effect: async (_action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = state.chat.current_thread_id;
    const runtime = state.chat.threads[chatId];

    if (!port || !chatId || !runtime) return;
    if (runtime.thread.messages.length > 0) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: {
          tool_use: runtime.thread.tool_use,
          mode: runtime.thread.mode,
        },
      });
    } catch {
      // Silently ignore - backend may not support this command
    }
  },
});

startListening({
  actionCreator: setChatMode,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = state.chat.current_thread_id;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { mode: action.payload },
      });
    } catch {
      // Silently ignore - backend may not support this command
    }
  },
});

startListening({
  actionCreator: setThreadMode,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;
    const runtime = state.chat.threads[chatId];

    if (!port || !chatId || !runtime) return;
    if (runtime.thread.messages.length > 0) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { mode: action.payload.mode },
      });
    } catch {
      // Silently ignore - backend may not support this command
    }
  },
});

startListening({
  actionCreator: setChatModel,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const port = state.config.lspPort;
    const apiKey = state.config.apiKey;
    const chatId = state.chat.current_thread_id;

    if (!port || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, port, apiKey ?? undefined, {
        type: "set_params",
        patch: { model: action.payload },
      });
    } catch {
      /* ignore */
    }
  },
});

startListening({
  matcher: isAnyOf(
    setChatModel,
    setBoostReasoning,
    setReasoningEffort,
    setThinkingBudget,
    setTemperature,
    setMaxTokens,
    setIncreaseMaxTokens,
    setIncludeProjectInfo,
    setContextTokensCap,
    setEnabledCheckpoints,
    setAreFollowUpsEnabled,
    setChatMode,
    setThreadMode,
    setSystemPrompt,
  ),
  effect: (_action, listenerApi) => {
    const state = listenerApi.getState();
    const chatId = setThreadMode.match(_action)
      ? _action.payload.chatId
      : state.chat.current_thread_id;
    const runtime = state.chat.threads[chatId];
    if (!runtime) return;

    const isUnstartedChat = runtime.thread.messages.length === 0;
    const shouldPersistForNewChats =
      isUnstartedChat ||
      setBoostReasoning.match(_action) ||
      setReasoningEffort.match(_action) ||
      setThinkingBudget.match(_action);
    if (!shouldPersistForNewChats) return;

    // Persist the updated param(s) as defaults for *new* chats.
    // IMPORTANT: For started chats, we only persist reasoning-related toggles
    // (boost_reasoning / reasoning_effort / thinking_budget), keeping other
    // sampling params “sticky” only before the first message.
    const mode = runtime.thread.mode;
    const patch: Parameters<typeof saveLastThreadParams>[0] = { mode };

    if (isUnstartedChat) {
      patch.model = runtime.thread.model;
      patch.temperature = runtime.thread.temperature ?? undefined;

      patch.max_tokens = runtime.thread.max_tokens ?? undefined;
      patch.increase_max_tokens = runtime.thread.increase_max_tokens;
      patch.include_project_info = runtime.thread.include_project_info;
      patch.context_tokens_cap = runtime.thread.context_tokens_cap;
      patch.system_prompt = state.chat.system_prompt;
      patch.checkpoints_enabled = state.chat.checkpoints_enabled;
      patch.follow_ups_enabled = state.chat.follow_ups_enabled;
    }

    if (setBoostReasoning.match(_action)) {
      patch.boost_reasoning = runtime.thread.boost_reasoning;
      // preserve temperature reset as part of “reasoning defaults”

      patch.temperature = runtime.thread.temperature ?? undefined;
    }
    if (setReasoningEffort.match(_action)) {

      patch.reasoning_effort = runtime.thread.reasoning_effort ?? undefined;

      patch.temperature = runtime.thread.temperature ?? undefined;
    }
    if (setThinkingBudget.match(_action)) {

      patch.thinking_budget = runtime.thread.thinking_budget ?? undefined;

      patch.temperature = runtime.thread.temperature ?? undefined;
    }

    // Still persist model changes after start (matches current UX).
    if (setChatModel.match(_action)) {
      patch.model = runtime.thread.model;
    }

    saveLastThreadParams(patch);
  },
});

// Thread params (model, temperature, etc.) are now sent synchronously
// before the user_message in each submit code path (actions.ts, useChatActions.ts),
// eliminating the race condition where this async listener could fire
// after the user_message had already triggered generation.

startListening({
  matcher: tasksApi.endpoints.deleteTask.matchFulfilled,
  effect: (action, listenerApi) => {
    const taskId = action.meta.arg.originalArgs;
    const state = listenerApi.getState();
    const threads = state.chat.threads as Record<
      string,
      | {
          thread: {
            task_meta?: { task_id: string };
            is_task_chat?: boolean;
            id: string;
          };
        }
      | undefined
    >;

    for (const [threadId, runtime] of Object.entries(threads)) {
      if (!runtime) continue;
      const thread = runtime.thread;
      if (
        thread.task_meta?.task_id === taskId ||
        (thread.is_task_chat && thread.id.includes(taskId))
      ) {
        listenerApi.dispatch(closeThread({ id: threadId, force: true }));
      }
    }

    listenerApi.dispatch(closeTask(taskId));
  },
});
