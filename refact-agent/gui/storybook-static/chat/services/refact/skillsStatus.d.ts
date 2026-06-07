import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, coreModuleName, TSHelpersId, QueryStatus, TSHelpersOverride, QuerySubState, skipToken, SubscriptionOptions, QueryActionCreatorResult } from '@reduxjs/toolkit/query';
export interface SkillsStatusResponse {
    skills_available: number;
    skills_included: string[];
    skills_enabled: boolean;
    active_skill: string | null;
}
export declare const skillsStatusApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    getSkillsStatus: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SkillsStatusResponse, "skillsStatusApi">;
}, "skillsStatusApi", never, typeof coreModuleName | typeof reactHooksModuleName>;
export declare const useGetSkillsStatusQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: SkillsStatusResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SkillsStatusResponse, "skillsStatusApi">> & {
    currentData?: SkillsStatusResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, {
    isLoading: true;
    isFetching: boolean;
    data: undefined;
} | ({
    isSuccess: true;
    isFetching: true;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SkillsStatusResponse, "skillsStatusApi">> & {
    currentData?: SkillsStatusResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SkillsStatusResponse, "skillsStatusApi">> & {
    currentData?: SkillsStatusResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SkillsStatusResponse, "skillsStatusApi">> & {
    currentData?: SkillsStatusResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: string | typeof skipToken, options?: (SubscriptionOptions & {
    skip?: boolean;
    refetchOnMountOrArgChange?: boolean | number;
} & {
    skip?: boolean;
    selectFromResult?: ((state: TSHelpersId<(Omit<{
        status: QueryStatus.uninitialized;
        originalArgs?: undefined | undefined;
        data?: undefined | undefined;
        error?: undefined | undefined;
        requestId?: undefined | undefined;
        endpointName?: string | undefined;
        startedTimeStamp?: undefined | undefined;
        fulfilledTimeStamp?: undefined | undefined;
    } & {
        currentData?: SkillsStatusResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SkillsStatusResponse, "skillsStatusApi">> & {
        currentData?: SkillsStatusResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, {
        isLoading: true;
        isFetching: boolean;
        data: undefined;
    } | ({
        isSuccess: true;
        isFetching: true;
        error: undefined;
    } & Required<Pick<QuerySubState<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SkillsStatusResponse, "skillsStatusApi">> & {
        currentData?: SkillsStatusResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp">>) | ({
        isSuccess: true;
        isFetching: false;
        error: undefined;
    } & Required<Pick<QuerySubState<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SkillsStatusResponse, "skillsStatusApi">> & {
        currentData?: SkillsStatusResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SkillsStatusResponse, "skillsStatusApi">> & {
        currentData?: SkillsStatusResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SkillsStatusResponse, "skillsStatusApi">>;
};
