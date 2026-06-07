import { MutationActionCreatorResult, MutationDefinition, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta } from '@reduxjs/toolkit/query';
import { PreviewCheckpointsPayload, PreviewCheckpointsResponse, Checkpoint } from '../features/Checkpoints/types';
export declare const usePreviewCheckpoints: () => {
    previewChangesFromCheckpoints: (checkpoints: Checkpoint[], chat_id: string, chat_mode?: string) => MutationActionCreatorResult<MutationDefinition<PreviewCheckpointsPayload, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "CHECKPOINTS", PreviewCheckpointsResponse, "checkpointsApi">>;
    isLoading: boolean;
};
