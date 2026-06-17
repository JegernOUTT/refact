// Buddy queue imports — keep at top to avoid circular deps
import {
  enqueueRuntimeEvent,
  clearNowPlaying,
  setBuddySnapshot,
  dequeueRuntimeEvent,
} from "../features/Buddy/buddySlice";
import { registerBuddySpeechTtlListener } from "../features/Buddy/buddySpeechTtl";
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
  ideToolRequired,
  setIsWaitingForResponse,
  saveTitle,
  setBoostReasoning,
  setIncludeProjectInfo,
  setEnabledCheckpoints,
  setToolUse,
  setChatMode,
  setThreadMode,
  setChatModel,
  setAutoApproveEditingTools,
  setAutoApproveDangerousCommands,
  setIncreaseMaxTokens,
  setMaxNewTokens,
  setAreFollowUpsEnabled,
  setSystemPrompt,
  setReasoningEffort,
  setThinkingBudget,
  setTemperature,
  setMaxTokens,
  updateChatRuntimeFromSessionState,
  openBuddyChat,
  newBuddyChatAction,
  createChatWithId,
  buildThreadParamsPatch,
  buildThreadScopePatch,
  selectCurrentThreadId,
  hydratePersistedChatTabs,
  removeChatFromCache,
  closeThread,
  reorderOpenThreads,
} from "../features/Chat/Thread";
import { saveLastThreadParams } from "../utils/threadStorage";
import {
  getProjectStorageNamespace,
  isProjectStorageNamespaceTrusted,
  loadPersistedPaneLayout,
  savePersistedChatTabs,
  savePersistedPaneLayout,
  setProjectStorageNamespaceFromProjectInfo,
} from "../utils/chatUiPersistence";
import { integrationsApi } from "../services/refact/integrations";
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
import { reportBuddyFrontendError } from "../features/Buddy/reportBuddyFrontendError";
import { setBackendStatus } from "../features/Connection";
import { setThemeMode, updateConfig } from "../features/Config/configSlice";
import { setCurrentProjectInfo } from "../features/Chat/currentProject";
import { resetSidebarState } from "../features/Sidebar/sidebarSlice";
import { nextTip } from "../features/TipOfTheDay";
import { tasksApi } from "../services/refact/tasks";
import {
  closeTask,
  openTask,
  addPlannerChat,
  setTaskActiveChat,
  taskSseEventReceived,
} from "../features/Tasks/tasksSlice";
import {
  popBackTo,
  push,
  selectCurrentPage,
} from "../features/Pages/pagesSlice";
import { cronFireReceived } from "../features/Scheduler";
import {
  ideToolCallResponse,
  ideTaskDone,
  ideAskQuestions,
} from "../hooks/useEventBusForIDE";
import { upsertToolCallIntoHistory } from "../features/History/historySlice";
import { isToolMessage, modelsApi, providersApi } from "../services/refact";
import { sendChatCommand } from "../services/refact/chatCommands";
import { schedulerApi } from "../services/refact/schedulerApi";
import { taskDocumentsApi } from "../services/refact/taskDocumentsApi";
import { taskMemoriesApi } from "../services/refact/taskMemoriesApi";
import {
  getEngineEndpointIdentity,
  hasUsableEngineEndpoint,
} from "../services/refact/apiUrl";
import {
  addTabToFocusedPane,
  closePane,
  focusPane,
  hydratePaneLayout,
  moveTabToPane,
  reorderTabInPane,
  removeTabEverywhere,
  resizeSplit,
  selectFocusedActiveTabId,
  selectFocusedLeafId,
  setPaneActiveTab,
  splitPane,
} from "../features/ChatPanes/panesSlice";

const AUTH_ERROR_MESSAGE =
  "There is an issue with your API key. Check out your API Key or re-login";

export const listenerMiddleware = createListenerMiddleware();
const startListening = listenerMiddleware.startListening.withTypes<
  RootState,
  AppDispatch
>();

function syncProjectStorageNamespace(state: RootState): boolean {
  const previous = getProjectStorageNamespace();
  const wasTrusted = isProjectStorageNamespaceTrusted();
  const workspaceRoots = state.current_project.workspaceRoots;
  setProjectStorageNamespaceFromProjectInfo({
    workspaceRoots,
    projectName: workspaceRoots?.length
      ? state.current_project.name
      : undefined,
  });
  return (
    getProjectStorageNamespace() !== previous ||
    isProjectStorageNamespaceTrusted() !== wasTrusted
  );
}

function endpointConfigChanged(
  previous: RootState["config"],
  next: RootState["config"],
): boolean {
  return (
    getEngineEndpointIdentity(previous) !== getEngineEndpointIdentity(next) ||
    previous.host !== next.host ||
    previous.dev !== next.dev ||
    previous.engineServed !== next.engineServed ||
    previous.lspPort !== next.lspPort ||
    previous.lspUrl !== next.lspUrl ||
    previous.backendReady !== next.backendReady ||
    previous.connectionStatus !== next.connectionStatus
  );
}

function hasConfiguredEngineEndpoint(state: RootState): boolean {
  return hasUsableEngineEndpoint(state.config);
}

function persistOpenChatTabs(state: RootState): void {
  syncProjectStorageNamespace(state);
  savePersistedChatTabs({
    openThreadIds: state.chat.open_thread_ids,
    currentThreadId: state.chat.current_thread_id,
    tabs: state.chat.open_thread_ids.map((id) => {
      const runtime = state.chat.threads[id];
      const historyItem = state.history.chats[id] as
        | (typeof state.history.chats)[string]
        | undefined;
      return {
        id,
        title: runtime?.thread.title ?? historyItem?.title,
        mode: runtime?.thread.mode ?? historyItem?.mode,
        tool_use: runtime?.thread.tool_use,
        session_state: runtime?.session_state ?? historyItem?.session_state,
        is_buddy_chat: Boolean(runtime?.thread.buddy_meta?.is_buddy_chat),
        is_task_chat: Boolean(runtime?.thread.is_task_chat),
      };
    }),
  });
}

function persistPaneLayout(state: RootState): void {
  syncProjectStorageNamespace(state);
  savePersistedPaneLayout({
    root: state.panes.root,
    focusedLeafId: state.panes.focusedLeafId,
  });
}

function hydratePersistedChatUi(listenerApi: {
  dispatch: AppDispatch;
  getState: () => RootState;
}): void {
  listenerApi.dispatch(hydratePersistedChatTabs());
  if (isProjectStorageNamespaceTrusted()) {
    listenerApi.dispatch(hydratePaneLayout(loadPersistedPaneLayout()));
    switchToFocusedPaneTab(listenerApi);
  }
}

function isPaneRoutableChat(
  state: RootState,
  chatId: string | null | undefined,
): chatId is string {
  if (!chatId) return false;
  const runtime = state.chat.threads[chatId];
  if (!runtime) return false;
  return state.chat.open_thread_ids.includes(chatId);
}

function placeChatInFocusedPane(
  listenerApi: {
    dispatch: AppDispatch;
    getState: () => RootState;
  },
  chatId?: string,
): void {
  const state = listenerApi.getState();
  const targetChatId = chatId ?? state.chat.current_thread_id;
  if (!isPaneRoutableChat(state, targetChatId)) return;

  listenerApi.dispatch(addTabToFocusedPane(targetChatId));
  const focusedLeafId = selectFocusedLeafId(listenerApi.getState());
  listenerApi.dispatch(
    setPaneActiveTab({ leafId: focusedLeafId, tabId: targetChatId }),
  );
}

function switchToFocusedPaneTab(listenerApi: {
  dispatch: AppDispatch;
  getState: () => RootState;
}): void {
  const state = listenerApi.getState();
  const chatId = selectFocusedActiveTabId(state);
  if (!isPaneRoutableChat(state, chatId)) return;
  if (state.chat.current_thread_id === chatId) return;
  listenerApi.dispatch(switchToThread({ id: chatId, openTab: false }));
}

startListening({
  matcher: isAnyOf(
    newChatAction,
    restoreChat,
    newIntegrationChat,
    removeChatFromCache,
    closeThread,
    switchToThread,
    reorderOpenThreads,
    saveTitle,
    createChatWithId,
    updateChatRuntimeFromSessionState,
    openBuddyChat,
    newBuddyChatAction,
    applyChatEvent,
  ),
  effect: (action, listenerApi) => {
    if (
      applyChatEvent.match(action) &&
      action.payload.type !== "snapshot" &&
      action.payload.type !== "thread_updated" &&
      action.payload.type !== "runtime_updated" &&
      action.payload.type !== "stream_finished"
    ) {
      return;
    }

    persistOpenChatTabs(listenerApi.getState());
  },
});

startListening({
  matcher: isAnyOf(
    splitPane,
    setPaneActiveTab,
    focusPane,
    closePane,
    moveTabToPane,
    reorderTabInPane,
    addTabToFocusedPane,
    removeTabEverywhere,
    resizeSplit,
    hydratePaneLayout,
    newChatAction,
    restoreChat,
    newIntegrationChat,
    removeChatFromCache,
    closeThread,
    switchToThread,
    reorderOpenThreads,
    saveTitle,
    createChatWithId,
    updateChatRuntimeFromSessionState,
    openBuddyChat,
    newBuddyChatAction,
    applyChatEvent,
  ),
  effect: (action, listenerApi) => {
    if (
      applyChatEvent.match(action) &&
      action.payload.type !== "snapshot" &&
      action.payload.type !== "thread_updated" &&
      action.payload.type !== "runtime_updated" &&
      action.payload.type !== "stream_finished"
    ) {
      return;
    }

    persistPaneLayout(listenerApi.getState());
  },
});

startListening({
  matcher: isAnyOf(closeThread, removeChatFromCache),
  effect: (action, listenerApi) => {
    if (!closeThread.match(action) && !removeChatFromCache.match(action))
      return;
    listenerApi.dispatch(removeTabEverywhere(action.payload.id));
    switchToFocusedPaneTab(listenerApi);
  },
});

startListening({
  matcher: isAnyOf(newChatAction, restoreChat, newIntegrationChat),
  effect: (_action, listenerApi) => {
    placeChatInFocusedPane(listenerApi);
  },
});

startListening({
  actionCreator: createChatWithId,
  effect: (action, listenerApi) => {
    placeChatInFocusedPane(listenerApi, action.payload.id);
  },
});

startListening({
  actionCreator: switchToThread,
  effect: (action, listenerApi) => {
    placeChatInFocusedPane(listenerApi, action.payload.id);
  },
});

startListening({
  actionCreator: setPaneActiveTab,
  effect: (action, listenerApi) => {
    const state = listenerApi.getState();
    if (selectFocusedLeafId(state) !== action.payload.leafId) {
      listenerApi.dispatch(focusPane(action.payload.leafId));
    }

    const nextState = listenerApi.getState();
    if (nextState.chat.current_thread_id === action.payload.tabId) return;
    if (!isPaneRoutableChat(nextState, action.payload.tabId)) return;
    listenerApi.dispatch(
      switchToThread({ id: action.payload.tabId, openTab: false }),
    );
  },
});

startListening({
  actionCreator: focusPane,
  effect: (_action, listenerApi) => {
    switchToFocusedPaneTab(listenerApi);
  },
});

startListening({
  actionCreator: setError,
  effect: (action, listenerApi) => {
    const state = listenerApi.getState();
    const chatId = state.chat.current_thread_id || undefined;
    void reportBuddyFrontendError({
      source: "ui_error_state",
      error: action.payload,
      sourceFile: "frontend/ui_error_state",
      toolName: "ui_error_state",
      chatId,
    });
  },
});

startListening({
  actionCreator: newChatAction,
  effect: async (_action, listenerApi) => {
    const state = listenerApi.getState();
    const chatId = state.chat.current_thread_id;

    [toolsApi.util.resetApiState(), commandsApi.util.resetApiState()].forEach(
      (api) => listenerApi.dispatch(api),
    );

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
    if (!runtime || !hasConfiguredEngineEndpoint(state) || !chatId) return;

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
        await sendChatCommand(
          chatId,
          state.config,
          state.config.apiKey ?? undefined,
          {
            type: "set_params",
            patch,
          },
        );
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

    [toolsApi.util.resetApiState(), commandsApi.util.resetApiState()].forEach(
      (api) => listenerApi.dispatch(api),
    );

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
  actionCreator: setBackendStatus,
  effect: (action, listenerApi) => {
    const previousStatus =
      listenerApi.getOriginalState().connection.backendStatus;
    if (action.payload.status === "online") {
      if (previousStatus === "online") return;
      listenerApi.dispatch(clearError());
      listenerApi.dispatch(capsApi.util.resetApiState());
      listenerApi.dispatch(modelsApi.util.resetApiState());
      listenerApi.dispatch(providersApi.util.resetApiState());
      return;
    }
    if (previousStatus !== "online") return;
    listenerApi.dispatch(capsApi.util.resetApiState());
    listenerApi.dispatch(modelsApi.util.resetApiState());
    listenerApi.dispatch(providersApi.util.resetApiState());
  },
});

startListening({
  actionCreator: updateConfig,
  effect: (_action, listenerApi) => {
    listenerApi.dispatch(pingApi.util.resetApiState());
    const namespaceChanged = syncProjectStorageNamespace(
      listenerApi.getState(),
    );
    if (namespaceChanged) {
      hydratePersistedChatUi(listenerApi);
    }

    const previousConfig = listenerApi.getOriginalState().config;
    const nextConfig = listenerApi.getState().config;
    if (endpointConfigChanged(previousConfig, nextConfig)) {
      listenerApi.dispatch(capsApi.util.resetApiState());
      listenerApi.dispatch(providersApi.util.resetApiState());
      listenerApi.dispatch(modelsApi.util.resetApiState());
      listenerApi.dispatch(resetSidebarState({ lspPort: nextConfig.lspPort }));
    }
  },
});

startListening({
  actionCreator: setCurrentProjectInfo,
  effect: (_action, listenerApi) => {
    const namespaceChanged = syncProjectStorageNamespace(
      listenerApi.getState(),
    );
    if (namespaceChanged) {
      hydratePersistedChatUi(listenerApi);
    }
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

startListening({
  actionCreator: ideToolCallResponse,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const chatId = action.payload.chatId;
    const { toolCallId, accepted } = action.payload;

    listenerApi.dispatch(upsertToolCallIntoHistory(action.payload));

    if (!hasConfiguredEngineEndpoint(state)) return;

    const apiKey = state.config.apiKey;
    const content =
      accepted === true
        ? "The user accepted the changes."
        : accepted === false
          ? "The user rejected the changes."
          : "The user applied the changes with modifications.";

    try {
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
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
    } else if (appearance === "inherit") {
      // Drop classes written for previous explicit choices so "inherit"
      // resolves from the host/system signal instead of a stale class.
      document.body.classList.remove("vscode-light", "vscode-dark");
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
    const apiKey = state.config.apiKey;
    const chatId = action.payload.id;
    const title = action.payload.title;
    const isTitleGenerated = action.payload.isTitleGenerated;

    if (!hasConfiguredEngineEndpoint(state) || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
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

startListening({
  actionCreator: applyChatEvent,
  effect: (action, listenerApi) => {
    const event = action.payload;
    if (event.type !== "message_added") return;
    const message = event.message;
    if (message.role !== "event" || message.subkind !== "cron_fire") return;
    listenerApi.dispatch(cronFireReceived(Date.now()));
    listenerApi.dispatch(schedulerApi.util.invalidateTags(["CronTasks"]));
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

interface HandoffToModeContent {
  type: "handoff_to_mode";
  new_chat_id: string;
  target_mode?: string;
  reason?: string;
  messages_count?: number;
}

type ToolMessageContent =
  | TaskDoneContent
  | AskQuestionsContent
  | HandoffToModeContent
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

function isHandoffToModeContent(
  content: ToolMessageContent,
): content is HandoffToModeContent {
  if (content.type !== "handoff_to_mode" || !("new_chat_id" in content)) {
    return false;
  }
  const id = content.new_chat_id;
  return typeof id === "string" && id.length > 0;
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

const MAX_PROCESSED_HANDOFFS = 1024;
const processedHandoffIds = new Set<string>();
const inFlightHandoffs = new Set<string>();

export function resetHandoffSets(): void {
  processedHandoffIds.clear();
  inFlightHandoffs.clear();
}

export function handoffSetsSize(): { processed: number; inFlight: number } {
  return {
    processed: processedHandoffIds.size,
    inFlight: inFlightHandoffs.size,
  };
}

startListening({
  actionCreator: applyChatEvent,
  effect: async (action, listenerApi) => {
    const event = action.payload;
    if (event.type !== "message_added") return;

    const msg = event.message;
    if (!isToolMessage(msg)) return;
    if (typeof msg.content !== "string") return;

    const toolCallId = msg.tool_call_id;
    const key = `${event.chat_id}:${toolCallId}`;
    if (processedHandoffIds.has(key) || inFlightHandoffs.has(key)) return;
    inFlightHandoffs.add(key);

    const parsed = safeParseJson(msg.content);
    if (!parsed || typeof parsed !== "object") {
      inFlightHandoffs.delete(key);
      return;
    }

    const content = parsed as ToolMessageContent;
    if (!isHandoffToModeContent(content)) {
      inFlightHandoffs.delete(key);
      return;
    }

    const { new_chat_id } = content;
    const state = listenerApi.getState();

    const apiKey = state.config.apiKey;

    // Preserve task/browser metadata from the source thread so the new chat
    // inherits the correct context (planner, task agent, browser session, etc.)
    const sourceRuntime = state.chat.threads[event.chat_id];
    const isTaskChat = sourceRuntime?.thread.is_task_chat ?? false;
    const taskMeta = sourceRuntime?.thread.task_meta;
    const worktree = sourceRuntime?.thread.worktree;

    listenerApi.dispatch(
      createChatWithId({
        id: new_chat_id,
        isTaskChat,
        taskMeta,
        parentId: event.chat_id,
        linkType: "handoff",
        worktree,
        mode:
          isTaskChat && taskMeta?.role === "planner"
            ? "TASK_PLANNER"
            : content.target_mode,
      }),
    );
    // Ensure the tab is open and switched to (handles both new and cached threads)
    listenerApi.dispatch(switchToThread({ id: new_chat_id }));
    // Optimistically mark as waiting so the UI shows a generating state
    // immediately before the first SSE event arrives
    listenerApi.dispatch(
      setIsWaitingForResponse({ id: new_chat_id, value: true }),
    );

    if (isTaskChat && taskMeta?.role === "planner" && taskMeta.task_id) {
      const taskId = taskMeta.task_id;
      const now = new Date().toISOString();
      // Ensure the task shell exists in tasksUI before registering the planner.
      // openTask is a safe upsert: it only updates the name when a real name is
      // provided and skips the update when the task already exists with a name.
      const taskListResult =
        tasksApi.endpoints.listTasks.select(undefined)(state);
      const taskName =
        taskListResult.data?.find((t) => t.id === taskId)?.name ?? "";
      listenerApi.dispatch(openTask({ id: taskId, name: taskName }));
      listenerApi.dispatch(
        addPlannerChat({
          taskId,
          planner: {
            id: new_chat_id,
            title: "",
            createdAt: now,
            updatedAt: now,
          },
        }),
      );
      listenerApi.dispatch(
        setTaskActiveChat({
          taskId,
          activeChat: { type: "planner", chatId: new_chat_id },
        }),
      );
      const currentPage = selectCurrentPage(listenerApi.getState());
      if (
        currentPage?.name !== "task workspace" ||
        currentPage.taskId !== taskId
      ) {
        listenerApi.dispatch(push({ name: "task workspace", taskId }));
      }
    } else {
      listenerApi.dispatch(push({ name: "chat" }));
    }

    if (hasConfiguredEngineEndpoint(state)) {
      const { regenerate } = await import("../services/refact/chatCommands");
      try {
        await regenerate(new_chat_id, state.config, apiKey ?? undefined);
        if (processedHandoffIds.size >= MAX_PROCESSED_HANDOFFS) {
          const arr = Array.from(processedHandoffIds);
          arr
            .slice(0, MAX_PROCESSED_HANDOFFS / 2)
            .forEach((k) => processedHandoffIds.delete(k));
        }
        processedHandoffIds.add(key);
      } catch {
        listenerApi.dispatch(
          setIsWaitingForResponse({ id: new_chat_id, value: false }),
        );
      } finally {
        inFlightHandoffs.delete(key);
      }
    } else {
      if (processedHandoffIds.size >= MAX_PROCESSED_HANDOFFS) {
        const arr = Array.from(processedHandoffIds);
        arr
          .slice(0, MAX_PROCESSED_HANDOFFS / 2)
          .forEach((k) => processedHandoffIds.delete(k));
      }
      processedHandoffIds.add(key);
      inFlightHandoffs.delete(key);
    }
  },
});

// Sync thread params to backend when changed via Redux actions
startListening({
  actionCreator: setBoostReasoning,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!hasConfiguredEngineEndpoint(state) || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
        type: "set_params",
        patch: { boost_reasoning: action.payload.value },
      });

      // When reasoning is enabled, temperature must be unset.
      // This avoids provider-side validation errors.
      if (action.payload.value) {
        await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
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
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!hasConfiguredEngineEndpoint(state) || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
        type: "set_params",
        patch: { reasoning_effort: action.payload.value },
      });

      // Any explicit reasoning effort implies reasoning mode: unset temperature.
      if (action.payload.value != null) {
        await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
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
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!hasConfiguredEngineEndpoint(state) || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
        type: "set_params",
        patch: { thinking_budget: action.payload.value },
      });

      // Any explicit thinking budget implies reasoning mode: unset temperature.
      if (action.payload.value != null) {
        await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
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
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!hasConfiguredEngineEndpoint(state) || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
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
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!hasConfiguredEngineEndpoint(state) || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
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
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!hasConfiguredEngineEndpoint(state) || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
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
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!hasConfiguredEngineEndpoint(state) || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
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
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;

    if (!hasConfiguredEngineEndpoint(state) || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
        type: "set_params",
        patch: { include_project_info: action.payload.value },
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
    const apiKey = state.config.apiKey;
    const chatId =
      typeof action.payload === "boolean"
        ? state.chat.current_thread_id
        : action.payload.chatId;
    const value =
      typeof action.payload === "boolean"
        ? action.payload
        : action.payload.value;

    if (!hasConfiguredEngineEndpoint(state) || !chatId) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
        type: "set_params",
        patch: { checkpoints_enabled: value },
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
    const apiKey = state.config.apiKey;
    const chatId = state.chat.current_thread_id;
    const runtime = state.chat.threads[chatId];

    if (!hasConfiguredEngineEndpoint(state) || !chatId || !runtime) return;
    if (runtime.thread.messages.length > 0) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
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
    const apiKey = state.config.apiKey;
    const chatId =
      typeof action.payload === "string"
        ? state.chat.current_thread_id
        : action.payload.chatId ?? state.chat.current_thread_id;
    const mode =
      typeof action.payload === "string" ? action.payload : action.payload.mode;
    const runtime = chatId ? state.chat.threads[chatId] : undefined;

    if (!hasConfiguredEngineEndpoint(state) || !chatId || !runtime) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
        type: "set_params",
        patch: {
          mode,
          ...buildThreadScopePatch(runtime.thread),
        },
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
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId;
    const runtime = state.chat.threads[chatId];

    if (!hasConfiguredEngineEndpoint(state) || !chatId || !runtime) return;
    if (runtime.thread.messages.length > 0) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
        type: "set_params",
        patch: {
          mode: action.payload.mode,
          ...buildThreadScopePatch(runtime.thread),
        },
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
    const apiKey = state.config.apiKey;
    const chatId = action.payload.chatId ?? state.chat.current_thread_id;
    const runtime = chatId ? state.chat.threads[chatId] : undefined;

    if (!hasConfiguredEngineEndpoint(state) || !chatId || !runtime) return;

    const patch: Record<string, unknown> = {
      model: action.payload.model,
    };
    if (action.payload.modelMaxContextTokens !== undefined) {
      patch.context_tokens_cap = runtime.thread.context_tokens_cap ?? null;
    }

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
        type: "set_params",
        patch,
      });
    } catch {
      /* ignore */
    }
  },
});

startListening({
  actionCreator: setMaxNewTokens,
  effect: async (action, listenerApi) => {
    const state = listenerApi.getState();
    const apiKey = state.config.apiKey;
    const chatId =
      typeof action.payload === "number"
        ? state.chat.current_thread_id
        : action.payload.chatId ?? state.chat.current_thread_id;
    const runtime = chatId ? state.chat.threads[chatId] : undefined;

    if (!hasConfiguredEngineEndpoint(state) || !chatId || !runtime) return;

    const previousCap =
      listenerApi.getOriginalState().chat.threads[chatId]?.thread
        .context_tokens_cap;
    if (previousCap === runtime.thread.context_tokens_cap) return;

    try {
      const { sendChatCommand } = await import(
        "../services/refact/chatCommands"
      );
      await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
        type: "set_params",
        patch: {
          context_tokens_cap: runtime.thread.context_tokens_cap ?? null,
        },
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
      : setChatModel.match(_action)
        ? _action.payload.chatId ?? state.chat.current_thread_id
        : setChatMode.match(_action)
          ? typeof _action.payload === "string"
            ? state.chat.current_thread_id
            : _action.payload.chatId ?? state.chat.current_thread_id
          : setEnabledCheckpoints.match(_action) &&
              typeof _action.payload !== "boolean"
            ? _action.payload.chatId
            : setIncreaseMaxTokens.match(_action) &&
                typeof _action.payload !== "boolean"
              ? _action.payload.chatId
              : setMaxNewTokens.match(_action) &&
                  typeof _action.payload !== "number"
                ? _action.payload.chatId ?? state.chat.current_thread_id
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
      patch.max_tokens = runtime.thread.max_tokens ?? undefined;
      patch.increase_max_tokens = runtime.thread.increase_max_tokens;
      patch.include_project_info = runtime.thread.include_project_info;
      patch.system_prompt = state.chat.system_prompt;
      patch.checkpoints_enabled =
        runtime.thread.checkpoints_enabled ?? state.chat.checkpoints_enabled;
      patch.follow_ups_enabled = state.chat.follow_ups_enabled;
    }

    if (setBoostReasoning.match(_action)) {
      patch.boost_reasoning = runtime.thread.boost_reasoning;
    }
    if (setReasoningEffort.match(_action)) {
      patch.reasoning_effort = runtime.thread.reasoning_effort ?? undefined;
    }
    if (setThinkingBudget.match(_action)) {
      patch.thinking_budget = runtime.thread.thinking_budget ?? undefined;
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

function clearDeletedTaskState(
  taskId: string,
  listenerApi: {
    dispatch: AppDispatch;
    getState: () => RootState;
  },
): void {
  const state = listenerApi.getState();
  const threads = state.chat.threads as Record<
    string,
    | {
        thread: {
          task_meta?: { task_id: string };
        };
      }
    | undefined
  >;

  for (const [threadId, runtime] of Object.entries(threads)) {
    if (!runtime) continue;
    if (runtime.thread.task_meta?.task_id === taskId) {
      listenerApi.dispatch(closeThread({ id: threadId, force: true }));
    }
  }

  listenerApi.dispatch(closeTask(taskId));

  const currentPage = selectCurrentPage(listenerApi.getState());
  if (currentPage?.name === "task workspace" && currentPage.taskId === taskId) {
    listenerApi.dispatch(popBackTo({ name: "history" }));
  }
}

function invalidateDeletedTaskCaches(
  taskId: string,
  listenerApi: {
    dispatch: AppDispatch;
  },
): void {
  listenerApi.dispatch(
    tasksApi.util.invalidateTags([
      "Tasks",
      { type: "Tasks", id: taskId },
      { type: "Board", id: taskId },
      { type: "TaskTrajectories", id: `${taskId}/planner` },
      { type: "TaskTrajectories", id: `${taskId}/agents` },
    ]),
  );
  listenerApi.dispatch(
    taskDocumentsApi.util.invalidateTags([
      { type: "TaskDocuments", id: taskId },
    ]),
  );
  listenerApi.dispatch(
    taskMemoriesApi.util.invalidateTags([
      { type: "TaskMemories", id: taskId },
      { type: "TaskMemories", id: `${taskId}:facets` },
    ]),
  );
}

startListening({
  matcher: tasksApi.endpoints.deleteTask.matchFulfilled,
  effect: (action, listenerApi) => {
    const taskId = action.meta.arg.originalArgs;
    invalidateDeletedTaskCaches(taskId, listenerApi);
    clearDeletedTaskState(taskId, listenerApi);
  },
});

startListening({
  actionCreator: taskSseEventReceived,
  effect: (action, listenerApi) => {
    const event = action.payload;
    switch (event.type) {
      case "snapshot":
        void listenerApi.dispatch(
          tasksApi.util.upsertQueryData("listTasks", undefined, event.tasks),
        );
        break;
      case "task_created": {
        const result = listenerApi.dispatch(
          tasksApi.util.updateQueryData("listTasks", undefined, (draft) => {
            if (!draft.some((t) => t.id === event.task_id)) {
              draft.unshift(event.meta);
            }
          }),
        );
        if (result.patches.length === 0) {
          void listenerApi.dispatch(
            tasksApi.util.upsertQueryData("listTasks", undefined, [event.meta]),
          );
        }
        break;
      }
      case "task_updated": {
        const result = listenerApi.dispatch(
          tasksApi.util.updateQueryData("listTasks", undefined, (draft) => {
            const index = draft.findIndex((t) => t.id === event.task_id);
            if (index >= 0) {
              draft[index] = event.meta;
            } else {
              draft.unshift(event.meta);
            }
            draft.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
          }),
        );
        if (result.patches.length === 0) {
          void listenerApi.dispatch(
            tasksApi.util.upsertQueryData("listTasks", undefined, [event.meta]),
          );
        }
        void listenerApi.dispatch(
          tasksApi.util.updateQueryData(
            "getTask",
            event.task_id,
            () => event.meta,
          ),
        );
        break;
      }
      case "task_deleted":
        void listenerApi.dispatch(
          tasksApi.util.updateQueryData("listTasks", undefined, (draft) => {
            const index = draft.findIndex((t) => t.id === event.task_id);
            if (index >= 0) {
              draft.splice(index, 1);
            }
          }),
        );
        invalidateDeletedTaskCaches(event.task_id, listenerApi);
        clearDeletedTaskState(event.task_id, listenerApi);
        break;
      case "board_changed":
        void listenerApi.dispatch(
          tasksApi.util.updateQueryData(
            "getBoard",
            event.task_id,
            () => event.board,
          ),
        );
        // Planner trajectory metadata (waiting_for_card_ids, session state) is
        // not included in the board payload, so invalidate to keep it fresh.
        void listenerApi.dispatch(
          tasksApi.util.invalidateTags([
            { type: "TaskTrajectories", id: `${event.task_id}/planner` },
          ]),
        );
        break;
      case "task_comments_changed":
        void listenerApi.dispatch(
          tasksApi.util.invalidateTags([{ type: "Board", id: event.task_id }]),
        );
        break;
      case "task_document_changed":
        void listenerApi.dispatch(
          taskDocumentsApi.util.invalidateTags(
            event.slug
              ? [
                  { type: "TaskDocuments", id: event.task_id },
                  {
                    type: "TaskDocuments",
                    id: `${event.task_id}:${event.slug}:detail`,
                  },
                  {
                    type: "TaskDocuments",
                    id: `${event.task_id}:${event.slug}:history`,
                  },
                ]
              : [{ type: "TaskDocuments", id: event.task_id }],
          ),
        );
        break;
      case "task_memories_changed":
        void listenerApi.dispatch(
          taskMemoriesApi.util.invalidateTags([
            { type: "TaskMemories", id: event.task_id },
            { type: "TaskMemories", id: `${event.task_id}:facets` },
          ]),
        );
        break;
    }
  },
});

// ─── Buddy runtime-queue manager ───────────────────────────────────────────
// Dequeue exactly once (in Redux middleware) so multiple mounted useBuddyState
// instances never race to dispatch dequeueRuntimeEvent simultaneously.
listenerMiddleware.startListening({
  matcher: isAnyOf(enqueueRuntimeEvent, clearNowPlaying, setBuddySnapshot),
  effect: (_action, { getState, dispatch }) => {
    const buddy = (getState() as RootState).buddy;
    if (buddy.nowPlaying === null && buddy.runtimeQueue.length > 0) {
      dispatch(dequeueRuntimeEvent());
    }
  },
});

// ─── Buddy speech TTL ──────────────────────────────────────────────────────
// The engine emits BuddySpeechItem with `persistent: bool` and `ttl_seconds`,
// but nothing clears non-persistent speeches client-side, so they hang in the
// cloud until the user dismisses them or another speech overwrites them.
// `registerBuddySpeechTtlListener` honors the TTL — see `buddySpeechTtl.ts`.
registerBuddySpeechTtlListener(listenerMiddleware);
