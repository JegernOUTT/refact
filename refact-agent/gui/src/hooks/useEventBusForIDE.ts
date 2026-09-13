import { useCallback } from "react";
import { createAction } from "@reduxjs/toolkit";
import { usePostMessage } from "./usePostMessage";
import type { ChatThread } from "../features/Chat/Thread/types";
import { pathApi } from "../services/refact/path";

import { ToolEditResult } from "../services/refact";
import { TextDocToolCall } from "../components/Tools/types";

export const ideDiffPasteBackAction = createAction<{
  content: string;
  chatId?: string;
  toolCallId?: string;
}>("ide/diffPasteBack");

export const ideOpenSettingsAction = createAction("ide/openSettings");

export const ideNewFileAction = createAction<string>("ide/newFile");

export const ideOpenHotKeys = createAction("ide/openHotKeys");

export type OpenFilePayload = {
  file_path: string;
  line?: number;
};
export const ideOpenFile = createAction<OpenFilePayload>("ide/openFile");

export const ideOpenChatInNewTab = createAction<ChatThread>(
  "ide/openChatInNewTab",
);

export const ideOpenChatInBrowser = createAction("ide/openChatInBrowser");

export const ideOpenFolderInNewWindow = createAction<{ path: string }>(
  "ide/openFolderInNewWindow",
);

export const ideAnimateFileStart = createAction<string>(
  "ide/animateFile/start",
);

export const ideAnimateFileStop = createAction<string>("ide/animateFile/stop");

export const ideChatPageChange = createAction<string>("ide/chatPageChange");
export const ideEscapeKeyPressed = createAction<string>("ide/escapeKeyPressed");

export const ideIsChatStreaming = createAction<boolean>("ide/isChatStreaming");
export const ideIsChatReady = createAction<boolean>("ide/isChatReady");

export const ideSetCodeCompletionModel = createAction<string>(
  "ide/setCodeCompletionModel",
);

export const ideSetLoginMessage = createAction<string>(
  "ide/ideSetLoginMessage",
);

export const ideForceReloadFileByPath = createAction<string>(
  "ide/forceReloadFileByPath",
);

export const ideToolCall = createAction<{
  toolCall: TextDocToolCall;
  chatId: string;
  edit: ToolEditResult;
}>("ide/toolEdit");

export const ideToolCallResponse = createAction<{
  toolCallId: string;
  chatId: string;
  accepted: boolean | "indeterminate";
}>("ide/toolEditResponse");

export const ideForceReloadProjectTreeFiles = createAction(
  "ide/forceReloadProjectTreeFiles",
);

export const ideTaskDone = createAction<{
  chatId: string;
  toolCallId: string;
  summary: string;
  knowledgePath?: string;
}>("ide/taskDone");

/**
 * Host decision bridge actions (B21).
 *
 * The engine owns every decision: a native pending question is created,
 * answered, cancelled and resolved exclusively by the chat session
 * (`chat/queue.rs`). The IDE bridge is a *notification* channel only — it tells
 * a host that a native decision is waiting, and later that the same native
 * decision stopped waiting. It never carries an answer, a permission, a grant
 * or a credential, and a host may never synthesise one from it.
 *
 * The correlation/duplicate/stale/reconnect rules themselves live in
 * `events/hostDecisions.ts` so that the browser, VS Code and JetBrains bridges
 * all execute one implementation instead of three drifting copies.
 */
import type {
  IdeAskQuestionsPayload,
  IdeDecisionOrigin,
  IdeDecisionQuestion,
  IdeDecisionResolution,
  IdeDecisionResolvedPayload,
} from "../events/hostDecisions";

export {
  DECISION_BRIDGE_CONTRACT_VERSION,
  HOST_DECISION_INTENTS,
  HOST_DECISION_PARITY_CASES,
  MAX_OUTSTANDING_DECISIONS,
  MAX_RESOLVED_TOMBSTONES,
  applyDecisionResolution,
  decisionCorrelationKey,
  decisionQuestionsDigest,
  emptyDecisionAnnouncementState,
  isValidAskQuestionsPayload,
  isValidDecisionResolvedPayload,
  outstandingDecisionCount,
  reconcileDecisionSnapshot,
  scopeDecisionState,
  shouldAnnounceDecision,
} from "../events/hostDecisions";
export type {
  DecisionAnnouncementDecision,
  DecisionAnnouncementState,
  DecisionResolutionOutcome,
  DecisionScope,
  HostDecisionIntent,
  IdeAskQuestionsPayload,
  IdeDecisionOrigin,
  IdeDecisionQuestion,
  IdeDecisionResolution,
  IdeDecisionResolvedPayload,
  NativeDecisionSnapshotInput,
  OutstandingDecision,
  SnapshotReconciliation,
} from "../events/hostDecisions";

export const ideAskQuestions =
  createAction<IdeAskQuestionsPayload>("ide/askQuestions");

/**
 * A native decision that was announced to the host is no longer waiting.
 *
 * Emitted when the authoritative native question leaves `pending` — including
 * when it was answered from a different client (other IDE, browser, TUI) or
 * cancelled natively. Hosts use it to retire their own notification state; it
 * grants nothing and answers nothing.
 */
export const ideDecisionResolved = createAction<IdeDecisionResolvedPayload>(
  "ide/decisionResolved",
);

export const ideSwitchToThread = createAction<{
  chatId: string;
}>("ide/switchToThread");

export const ideRequestLogs = createAction<{ limit: number }>(
  "ide/requestLogs",
);

export const useEventsBusForIDE = () => {
  const postMessage = usePostMessage();
  // const canPaste = useAppSelector((state) => state.active_file.can_paste);

  const startFileAnimation = useCallback(
    (fileName: string) => {
      const action = ideAnimateFileStart(fileName);
      postMessage(action);
    },
    [postMessage],
  );

  const stopFileAnimation = useCallback(
    (fileName: string) => {
      const action = ideAnimateFileStop(fileName);
      postMessage(action);
    },
    [postMessage],
  );

  const diffPasteBack = useCallback(
    (content: string, chatId?: string, toolCallId?: string) => {
      const action = ideDiffPasteBackAction({ content, chatId, toolCallId });
      postMessage(action);
    },
    [postMessage],
  );

  const openSettings = useCallback(() => {
    const action = ideOpenSettingsAction();
    postMessage(action);
  }, [postMessage]);

  const newFile = useCallback(
    (content: string) => {
      const action = ideNewFileAction(content);
      postMessage(action);
    },
    [postMessage],
  );

  const openHotKeys = useCallback(() => {
    const action = ideOpenHotKeys();
    postMessage(action);
  }, [postMessage]);

  const openFile = useCallback(
    (file: OpenFilePayload) => {
      const action = ideOpenFile(file);
      postMessage(action);
    },
    [postMessage],
  );

  const [getFullPath, _] = pathApi.useLazyGetFullPathQuery();

  const queryPathThenOpenFile = useCallback(
    async (file: OpenFilePayload) => {
      const res = await getFullPath(file.file_path).unwrap();
      const file_name = res ?? file.file_path;
      const action = ideOpenFile({ file_path: file_name, line: file.line });
      postMessage(action);
    },
    [getFullPath, postMessage],
  );

  const openChatInNewTab = useCallback(
    (thread: ChatThread) => {
      const action = ideOpenChatInNewTab(thread);
      postMessage(action);
    },
    [postMessage],
  );

  const openChatInBrowser = useCallback(() => {
    const action = ideOpenChatInBrowser();
    postMessage(action);
  }, [postMessage]);

  const openFolderInNewWindow = useCallback(
    (path: string) => {
      const action = ideOpenFolderInNewWindow({ path });
      postMessage(action);
    },
    [postMessage],
  );

  const chatPageChange = useCallback(
    (page: string) => {
      const action = ideChatPageChange(page);
      postMessage(action);
    },
    [postMessage],
  );

  const escapeKeyPressed = useCallback(
    (mode: string) => {
      const action = ideEscapeKeyPressed(mode);
      postMessage(action);
    },
    [postMessage],
  );

  const setIsChatStreaming = useCallback(
    (state: boolean) => {
      const action = ideIsChatStreaming(state);
      postMessage(action);
    },
    [postMessage],
  );

  const setIsChatReady = useCallback(
    (state: boolean) => {
      const action = ideIsChatReady(state);
      postMessage(action);
    },
    [postMessage],
  );

  const setForceReloadFileByPath = useCallback(
    (path: string) => {
      const action = ideForceReloadFileByPath(path);
      postMessage(action);
    },
    [postMessage],
  );

  const setCodeCompletionModel = useCallback(
    (model: string) => {
      const action = ideSetCodeCompletionModel(model);
      postMessage(action);
    },
    [postMessage],
  );

  const setLoginMessage = useCallback(
    (message: string) => {
      const action = ideSetLoginMessage(message);
      postMessage(action);
    },
    [postMessage],
  );

  const [getCustomizationPath] = pathApi.useLazyCustomizationPathQuery();
  const [getIntegrationsPath] = pathApi.useLazyIntegrationsPathQuery();
  const [getPrivacyPath] = pathApi.useLazyPrivacyPathQuery();

  // Creating a generic function to trigger different queries from RTK Query (to avoid duplicative code)
  const openFileFromPathQuery = useCallback(
    async (
      getPathQuery: (arg: undefined) => {
        unwrap: () => Promise<string | undefined>;
      },
    ) => {
      const res = await getPathQuery(undefined).unwrap();

      if (res) {
        const action = ideOpenFile({ file_path: res });
        postMessage(action);
      }
    },
    [postMessage],
  );

  const openCustomizationFile = () =>
    openFileFromPathQuery(getCustomizationPath);

  const openPrivacyFile = () => openFileFromPathQuery(getPrivacyPath);
  const openIntegrationsFile = () => openFileFromPathQuery(getIntegrationsPath);

  const sendToolCallToIde = useCallback(
    (toolCall: TextDocToolCall, edit: ToolEditResult, chatId: string) => {
      const action = ideToolCall({ toolCall, edit, chatId });
      postMessage(action);
    },
    [postMessage],
  );

  const notifyTaskDone = useCallback(
    (
      chatId: string,
      toolCallId: string,
      summary: string,
      knowledgePath?: string,
    ) => {
      const action = ideTaskDone({
        chatId,
        toolCallId,
        summary,
        knowledgePath,
      });
      postMessage(action);
    },
    [postMessage],
  );

  const notifyAskQuestions = useCallback(
    (
      chatId: string,
      toolCallId: string,
      questions: IdeDecisionQuestion[],
      meta?: { askedAtMs?: number; origin?: IdeDecisionOrigin },
    ) => {
      const action = ideAskQuestions({
        chatId,
        toolCallId,
        questions,
        ...(meta?.askedAtMs !== undefined ? { askedAtMs: meta.askedAtMs } : {}),
        ...(meta?.origin !== undefined ? { origin: meta.origin } : {}),
      });
      postMessage(action);
    },
    [postMessage],
  );

  const notifyDecisionResolved = useCallback(
    (chatId: string, toolCallId: string, resolution: IdeDecisionResolution) => {
      const action = ideDecisionResolved({ chatId, toolCallId, resolution });
      postMessage(action);
    },
    [postMessage],
  );

  const queryIdeLogs = useCallback(
    (limit: number) => {
      const action = ideRequestLogs({ limit });
      postMessage(action);
    },
    [postMessage],
  );

  return {
    diffPasteBack,
    openSettings,
    newFile,
    openHotKeys,
    openFile,
    openChatInNewTab,
    openChatInBrowser,
    openFolderInNewWindow,
    queryPathThenOpenFile,
    openCustomizationFile,
    openPrivacyFile,
    openIntegrationsFile,
    stopFileAnimation,
    startFileAnimation,
    chatPageChange,
    escapeKeyPressed,
    setIsChatStreaming,
    setIsChatReady,
    setForceReloadFileByPath,
    sendToolCallToIde,
    setCodeCompletionModel,
    setLoginMessage,
    notifyTaskDone,
    notifyAskQuestions,
    notifyDecisionResolved,
    queryIdeLogs,
  };
};
