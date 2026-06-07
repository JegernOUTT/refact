export type PersistedChatTab = {
    id: string;
    title?: string;
    mode?: string;
    tool_use?: "quick" | "explore" | "agent";
    session_state?: string;
    is_buddy_chat?: boolean;
};
export type PersistedChatTabsState = {
    openThreadIds: string[];
    currentThreadId: string;
    tabs: PersistedChatTab[];
};
export type PersistedActiveTab = {
    type: "dashboard";
} | {
    type: "chat";
    id: string;
} | {
    type: "task";
    taskId: string;
};
export type PersistedTaskActiveChat = {
    type: "planner";
    chatId: string;
} | {
    type: "agent";
    cardId: string;
    chatId: string;
} | null;
export interface PersistedPlannerInfo {
    id: string;
    title: string;
    createdAt: string;
    updatedAt: string;
    sessionState?: string;
    waitingForCardIds?: string[];
}
export interface PersistedOpenTask {
    id: string;
    name: string;
    plannerChats: PersistedPlannerInfo[];
    activeChat: PersistedTaskActiveChat;
}
export interface PersistedTasksUIState {
    openTasks: PersistedOpenTask[];
}
export type AskQuestionsDraftValue = string | string[];
export type AskQuestionsDraft = {
    answers: Record<string, AskQuestionsDraftValue>;
    additionalText: string;
    updatedAt: number;
};
export type TaskWorkspaceLayout = {
    chatExpanded: boolean;
    panelsExpanded: boolean;
    boardHeightPx: number;
};
export declare function getProjectStorageNamespace(): string | null;
export declare function isProjectStorageNamespaceTrusted(): boolean;
export declare function setProjectStorageNamespace(value: string | undefined): void;
export declare function setProjectStorageNamespaceFromProjectInfo(input: {
    workspaceRoots?: string[];
    projectName?: string;
    workspaceName?: string;
}): void;
export declare function loadPersistedChatTabs(): PersistedChatTabsState;
export declare function savePersistedChatTabs(input: PersistedChatTabsState): void;
export declare function loadPersistedActiveTab(): PersistedActiveTab | null;
export declare function savePersistedActiveTab(activeTab: PersistedActiveTab): void;
export declare function loadPersistedTasksUIState(): PersistedTasksUIState;
export declare function savePersistedTasksUIState(state: PersistedTasksUIState): void;
export declare function loadAskQuestionsDraft(toolCallId: string | undefined): AskQuestionsDraft | null;
export declare function saveAskQuestionsDraft(toolCallId: string | undefined, answers: Record<string, AskQuestionsDraftValue>, additionalText: string): void;
export declare function clearAskQuestionsDraft(toolCallId: string | undefined): void;
export declare function loadTaskWorkspaceLayout(taskId: string, defaults: TaskWorkspaceLayout): TaskWorkspaceLayout;
export declare function saveTaskWorkspaceLayout(taskId: string, layout: TaskWorkspaceLayout): void;
