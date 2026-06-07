import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, MutationDefinition, coreModuleName } from '@reduxjs/toolkit/query';
import { type ChatMessages } from ".";
export type SubscribeArgs = {
    quick_search?: string;
    limit?: number;
} | undefined;
export type MemAddRequest = {
    goal: string;
    payload: string;
    mem_type?: string;
    project?: string;
    origin?: string;
};
export declare function isAddMemoryRequest(obj: unknown): obj is MemAddRequest;
export type MemQuery = {
    goal: string;
    project?: string;
    top_n?: number;
};
export type MemUpdateUsedRequest = {
    memid: string;
    correct: number;
    relevant: number;
};
export type MemUpdateRequest = {
    memid: string;
    mem_type: string;
    goal: string;
    project: string;
    payload: string;
    origin: string;
};
export declare function isMemUpdateRequest(obj: unknown): obj is MemUpdateRequest;
export type CompressTrajectoryPost = {
    project: string;
    messages: ChatMessages;
};
export declare const knowledgeApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    compressMessages: MutationDefinition<CompressTrajectoryPost, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, {
        goal: string;
        trajectory: string;
    }, "knowledgeApi">;
}, "knowledgeApi", never, typeof coreModuleName | typeof reactHooksModuleName>;
