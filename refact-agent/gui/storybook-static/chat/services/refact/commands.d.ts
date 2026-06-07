import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, coreModuleName } from '@reduxjs/toolkit/query';
import { LspChatMessage } from "./chat";
import type { ChatContextFile, ChatMeta } from "./types";
export type CompletionArgs = {
    query: string;
    cursor: number;
    top_n?: number;
};
export declare const commandsApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    getCommandCompletion: QueryDefinition<CompletionArgs, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CommandCompletionResponse, "commands">;
    getCommandPreview: QueryDefinition<CommandPreviewRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CommandPreviewResponse & {
        files: (ChatContextFile | string)[];
    }, "commands">;
}, "commands", never, typeof coreModuleName | typeof reactHooksModuleName>;
export type CompletionDetail = {
    description?: string;
    argument_hint?: string;
    source?: string;
    kind?: string;
};
export type CommandCompletionResponse = {
    completions: string[];
    completion_details?: Record<string, CompletionDetail>;
    replace: [number, number];
    is_cmd_executable: boolean;
};
export declare function isCommandCompletionResponse(json: unknown): json is CommandCompletionResponse;
export type DetailMessage = {
    detail: string;
};
export type DetailMessageWithErrorType = DetailMessage & {
    errorType: "CHAT" | "GLOBAL";
};
export declare function isDetailMessage(json: unknown): json is DetailMessage;
export declare function isDetailMessageWithErrorType(json: unknown): json is DetailMessageWithErrorType;
export type CommandPreviewContent = {
    role: "plain_text";
    content: string;
} | {
    role: "context_file";
    content: ChatContextFile[] | string;
};
export type CommandPreviewRequest = {
    messages: LspChatMessage[];
    meta: ChatMeta;
    model: string;
};
export type CommandPreviewResponse = {
    messages: CommandPreviewContent[];
    current_context: number;
    number_context: number;
};
export declare function isCommandPreviewResponse(json: unknown): json is CommandPreviewResponse;
