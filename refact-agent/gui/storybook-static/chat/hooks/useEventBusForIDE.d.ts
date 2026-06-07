import { ActionCreatorWithPayload, ActionCreatorWithoutPayload } from '@reduxjs/toolkit';
import type { ChatThread } from "../features/Chat/Thread/types";
import { ToolEditResult } from "../services/refact";
import { TextDocToolCall } from "../components/Tools/types";
export declare const ideDiffPasteBackAction: ActionCreatorWithPayload<{
    content: string;
    chatId?: string;
    toolCallId?: string;
}, string>;
export declare const ideOpenSettingsAction: ActionCreatorWithoutPayload<"ide/openSettings">;
export declare const ideNewFileAction: ActionCreatorWithPayload<string, string>;
export declare const ideOpenHotKeys: ActionCreatorWithoutPayload<"ide/openHotKeys">;
export type OpenFilePayload = {
    file_path: string;
    line?: number;
};
export declare const ideOpenFile: ActionCreatorWithPayload<OpenFilePayload, string>;
export declare const ideOpenChatInNewTab: ActionCreatorWithPayload<ChatThread, string>;
export declare const ideOpenChatInBrowser: ActionCreatorWithoutPayload<"ide/openChatInBrowser">;
export declare const ideOpenFolderInNewWindow: ActionCreatorWithPayload<{
    path: string;
}, string>;
export declare const ideAnimateFileStart: ActionCreatorWithPayload<string, string>;
export declare const ideAnimateFileStop: ActionCreatorWithPayload<string, string>;
export declare const ideChatPageChange: ActionCreatorWithPayload<string, string>;
export declare const ideEscapeKeyPressed: ActionCreatorWithPayload<string, string>;
export declare const ideIsChatStreaming: ActionCreatorWithPayload<boolean, string>;
export declare const ideIsChatReady: ActionCreatorWithPayload<boolean, string>;
export declare const ideSetCodeCompletionModel: ActionCreatorWithPayload<string, string>;
export declare const ideSetLoginMessage: ActionCreatorWithPayload<string, string>;
export declare const ideForceReloadFileByPath: ActionCreatorWithPayload<string, string>;
export declare const ideToolCall: ActionCreatorWithPayload<{
    toolCall: TextDocToolCall;
    chatId: string;
    edit: ToolEditResult;
}, string>;
export declare const ideToolCallResponse: ActionCreatorWithPayload<{
    toolCallId: string;
    chatId: string;
    accepted: boolean | "indeterminate";
}, string>;
export declare const ideForceReloadProjectTreeFiles: ActionCreatorWithoutPayload<"ide/forceReloadProjectTreeFiles">;
export declare const ideTaskDone: ActionCreatorWithPayload<{
    chatId: string;
    toolCallId: string;
    summary: string;
    knowledgePath?: string;
}, string>;
export declare const ideAskQuestions: ActionCreatorWithPayload<{
    chatId: string;
    toolCallId: string;
    questions: {
        id: string;
        type: string;
        text: string;
        options?: string[];
    }[];
}, string>;
export declare const ideSwitchToThread: ActionCreatorWithPayload<{
    chatId: string;
}, string>;
export declare const useEventsBusForIDE: () => {
    diffPasteBack: (content: string, chatId?: string, toolCallId?: string) => void;
    openSettings: () => void;
    newFile: (content: string) => void;
    openHotKeys: () => void;
    openFile: (file: OpenFilePayload) => void;
    openChatInNewTab: (thread: ChatThread) => void;
    openChatInBrowser: () => void;
    openFolderInNewWindow: (path: string) => void;
    queryPathThenOpenFile: (file: OpenFilePayload) => Promise<void>;
    openCustomizationFile: () => Promise<void>;
    openPrivacyFile: () => Promise<void>;
    openIntegrationsFile: () => Promise<void>;
    stopFileAnimation: (fileName: string) => void;
    startFileAnimation: (fileName: string) => void;
    chatPageChange: (page: string) => void;
    escapeKeyPressed: (mode: string) => void;
    setIsChatStreaming: (state: boolean) => void;
    setIsChatReady: (state: boolean) => void;
    setForceReloadFileByPath: (path: string) => void;
    sendToolCallToIde: (toolCall: TextDocToolCall, edit: ToolEditResult, chatId: string) => void;
    setCodeCompletionModel: (model: string) => void;
    setLoginMessage: (message: string) => void;
    notifyTaskDone: (chatId: string, toolCallId: string, summary: string, knowledgePath?: string) => void;
    notifyAskQuestions: (chatId: string, toolCallId: string, questions: {
        id: string;
        type: string;
        text: string;
        options?: string[];
    }[]) => void;
};
