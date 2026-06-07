import { TSHelpersId, QueryStatus, TSHelpersOverride, QuerySubState, QueryDefinition, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryActionCreatorResult } from '@reduxjs/toolkit/query';
export declare function useGetMCPLogs(integrationPath: string): (TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: {
        logs: string[];
    } | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", {
    logs: string[];
}, "integrationsApi">> & {
    currentData?: {
        logs: string[];
    } | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", {
    logs: string[];
}, "integrationsApi">> & {
    currentData?: {
        logs: string[];
    } | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", {
    logs: string[];
}, "integrationsApi">> & {
    currentData?: {
        logs: string[];
    } | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", {
    logs: string[];
}, "integrationsApi">> & {
    currentData?: {
        logs: string[];
    } | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}) & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", {
        logs: string[];
    }, "integrationsApi">>;
};
