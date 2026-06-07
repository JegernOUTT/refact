import { MutationActionCreatorResult, MutationDefinition, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta } from '@reduxjs/toolkit/query';
import { RestoreCheckpointsPayload, RestoreCheckpointsResponse, Checkpoint } from '../features/Checkpoints/types';
export declare const useRestoreCheckpoints: () => {
    restoreChangesFromCheckpoints: (checkpoints: Checkpoint[], chat_id: string, chat_mode?: string) => MutationActionCreatorResult<MutationDefinition<RestoreCheckpointsPayload, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "CHECKPOINTS", RestoreCheckpointsResponse, "checkpointsApi">>;
    isLoading: boolean;
};
