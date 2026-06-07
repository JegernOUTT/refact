import { SerializedError } from '@reduxjs/toolkit';
import { MutationActionCreatorResult, MutationDefinition, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryStatus } from '@reduxjs/toolkit/query';
import { ToolGroupUpdate } from '../services/refact';
export declare function useUpdateToolGroupsMutation(): {
    mutationTrigger: (arg: ToolGroupUpdate[]) => MutationActionCreatorResult<MutationDefinition<ToolGroupUpdate[], BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TOOL_GROUPS", {
        success: true;
    }, "tools">>;
    mutationResult: ({
        requestId?: undefined;
        status: QueryStatus.uninitialized;
        data?: undefined;
        error?: undefined;
        endpointName?: string;
        startedTimeStamp?: undefined;
        fulfilledTimeStamp?: undefined;
    } & {
        status: QueryStatus.uninitialized;
        isUninitialized: true;
        isLoading: false;
        isSuccess: false;
        isError: false;
    } & {
        originalArgs?: ToolGroupUpdate[] | undefined;
        reset: () => void;
    }) | ({
        status: QueryStatus.fulfilled;
    } & Omit<{
        requestId: string;
        data?: {
            success: true;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            success: true;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp">> & {
        error: undefined;
    } & {
        status: QueryStatus.fulfilled;
        isUninitialized: false;
        isLoading: false;
        isSuccess: true;
        isError: false;
    } & {
        originalArgs?: ToolGroupUpdate[] | undefined;
        reset: () => void;
    }) | ({
        status: QueryStatus.pending;
    } & {
        requestId: string;
        data?: {
            success: true;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    } & {
        data?: undefined;
    } & {
        status: QueryStatus.pending;
        isUninitialized: false;
        isLoading: true;
        isSuccess: false;
        isError: false;
    } & {
        originalArgs?: ToolGroupUpdate[] | undefined;
        reset: () => void;
    }) | ({
        status: QueryStatus.rejected;
    } & Omit<{
        requestId: string;
        data?: {
            success: true;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            success: true;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error">> & {
        status: QueryStatus.rejected;
        isUninitialized: false;
        isLoading: false;
        isSuccess: false;
        isError: true;
    } & {
        originalArgs?: ToolGroupUpdate[] | undefined;
        reset: () => void;
    });
};
