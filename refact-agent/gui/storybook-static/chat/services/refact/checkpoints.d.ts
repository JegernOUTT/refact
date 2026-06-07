import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, MutationDefinition, coreModuleName } from '@reduxjs/toolkit/query';
import { PreviewCheckpointsPayload, PreviewCheckpointsResponse, RestoreCheckpointsPayload, RestoreCheckpointsResponse } from "../../features/Checkpoints/types";
export declare const checkpointsApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    previewCheckpoints: MutationDefinition<PreviewCheckpointsPayload, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "CHECKPOINTS", PreviewCheckpointsResponse, "checkpointsApi">;
    restoreCheckpoints: MutationDefinition<RestoreCheckpointsPayload, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "CHECKPOINTS", RestoreCheckpointsResponse, "checkpointsApi">;
}, "checkpointsApi", "CHECKPOINTS", typeof coreModuleName | typeof reactHooksModuleName>;
