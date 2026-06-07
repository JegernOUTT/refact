import { AssistantMessage, ChatContextFile, ChatMessages, DiffChunk, DiffMessage, ErrorMessage, SummarizationMessage, UserMessage } from "../../services/refact";
type DisplayItemAssistant = {
    type: "assistant";
    key: string;
    index: number;
    messageIndex: number;
    message: AssistantMessage;
    contextFilesByToolId: Record<string, ChatContextFile[]>;
    diffsByToolId: Record<string, DiffChunk[]>;
    isStreaming: boolean;
};
type DisplayItemUser = {
    type: "user";
    key: string;
    index: number;
    messageIndex: number;
    message: UserMessage;
    isLastUser: boolean;
};
type DisplayItemContextFiles = {
    type: "context_files";
    key: string;
    messageIndex: number;
    files: ChatContextFile[];
    toolCallId?: string;
    rawExtra?: unknown;
};
type DisplayItemDiffGroup = {
    type: "diff_group";
    key: string;
    messageIndex: number;
    diffs: DiffMessage[];
};
type DisplayItemSystem = {
    type: "system";
    key: string;
    messageIndex: number;
    content: string;
};
type DisplayItemPlainText = {
    type: "plain_text";
    key: string;
    messageIndex: number;
    content: string;
};
type DisplayItemError = {
    type: "error";
    key: string;
    messageIndex: number;
    errors: ErrorMessage[];
};
type DisplayItemSkillActivated = {
    type: "skill_activated";
    key: string;
    messageIndex: number;
    name: string;
    body: string;
    allowedTools: string[];
    modelOverride: string | null;
};
type DisplayItemSkillReport = {
    type: "skill_report";
    key: string;
    messageIndex: number;
    skillName: string;
    report: string;
};
type DisplayItemSummarization = {
    type: "summarization";
    key: string;
    messageIndex: number;
    message: SummarizationMessage;
};
export type DisplayItem = DisplayItemAssistant | DisplayItemUser | DisplayItemContextFiles | DisplayItemDiffGroup | DisplayItemSystem | DisplayItemPlainText | DisplayItemError | DisplayItemSkillActivated | DisplayItemSkillReport | DisplayItemSummarization;
export declare function buildDisplayItems(messages: ChatMessages, isStreaming: boolean): DisplayItem[];
export declare function tryIncrementalDisplayItemsUpdate(previousMessages: ChatMessages | null, nextMessages: ChatMessages, previousItems: DisplayItem[] | null, isStreaming: boolean): DisplayItem[] | null;
export {};
