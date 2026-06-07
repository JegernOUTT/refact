import { SerializedError } from '@reduxjs/toolkit';
import { ConfiguredProvidersResponse, ProviderDetailResponse, ProviderSchemaResponse, ProviderModelsResponse, ProviderDefaults, ProviderDefaultsUpdateRequest } from '../services/refact';
import { TSHelpersId, QueryStatus, TSHelpersOverride, QuerySubState, QueryDefinition, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryActionCreatorResult, MutationActionCreatorResult, MutationDefinition } from '@reduxjs/toolkit/query';
export declare function useGetConfiguredProvidersQuery(): (TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ConfiguredProvidersResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
    currentData?: ConfiguredProvidersResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
    currentData?: ConfiguredProvidersResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
    currentData?: ConfiguredProvidersResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
    currentData?: ConfiguredProvidersResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}) & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">>;
};
export declare function useGetProviderQuery({ providerName, }: {
    providerName: string;
}): (TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ProviderDetailResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
    currentData?: ProviderDetailResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
    currentData?: ProviderDetailResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
    currentData?: ProviderDetailResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
    currentData?: ProviderDetailResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}) & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">>;
};
export declare function useGetProviderSchemaQuery({ providerName, }: {
    providerName: string;
}): (TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ProviderSchemaResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
    currentData?: ProviderSchemaResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
    currentData?: ProviderSchemaResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
    currentData?: ProviderSchemaResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
    currentData?: ProviderSchemaResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}) & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">>;
};
export declare function useGetProviderModelsQuery({ providerName, }: {
    providerName: string;
}): (TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ProviderModelsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
    currentData?: ProviderModelsResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
    currentData?: ProviderModelsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
    currentData?: ProviderModelsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
    currentData?: ProviderModelsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}) & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">>;
};
export declare function useUpdateProviderMutation(): readonly [(arg: {
    providerName: string;
    settings: Record<string, unknown>;
}) => MutationActionCreatorResult<MutationDefinition<{
    providerName: string;
    settings: Record<string, unknown>;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
    success: boolean;
}, "providers">>, ({
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
    originalArgs?: {
        providerName: string;
        settings: Record<string, unknown>;
    } | undefined;
    reset: () => void;
}) | ({
    status: QueryStatus.fulfilled;
} & Omit<{
    requestId: string;
    data?: {
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
    originalArgs?: {
        providerName: string;
        settings: Record<string, unknown>;
    } | undefined;
    reset: () => void;
}) | ({
    status: QueryStatus.pending;
} & {
    requestId: string;
    data?: {
        success: boolean;
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
    originalArgs?: {
        providerName: string;
        settings: Record<string, unknown>;
    } | undefined;
    reset: () => void;
}) | ({
    status: QueryStatus.rejected;
} & Omit<{
    requestId: string;
    data?: {
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
    originalArgs?: {
        providerName: string;
        settings: Record<string, unknown>;
    } | undefined;
    reset: () => void;
})];
export declare function useDeleteProviderMutation(): readonly [(arg: string) => MutationActionCreatorResult<MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
    success: boolean;
}, "providers">>, ({
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
    originalArgs?: string | undefined;
    reset: () => void;
}) | ({
    status: QueryStatus.fulfilled;
} & Omit<{
    requestId: string;
    data?: {
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
    originalArgs?: string | undefined;
    reset: () => void;
}) | ({
    status: QueryStatus.pending;
} & {
    requestId: string;
    data?: {
        success: boolean;
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
    originalArgs?: string | undefined;
    reset: () => void;
}) | ({
    status: QueryStatus.rejected;
} & Omit<{
    requestId: string;
    data?: {
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
    originalArgs?: string | undefined;
    reset: () => void;
})];
export declare function useGetDefaultsQuery(): (TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ProviderDefaults | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
    currentData?: ProviderDefaults | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
    currentData?: ProviderDefaults | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
    currentData?: ProviderDefaults | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
    currentData?: ProviderDefaults | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}) & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">>;
};
export declare function useUpdateDefaultsMutation(): readonly [(arg: ProviderDefaultsUpdateRequest) => MutationActionCreatorResult<MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
    success: boolean;
}, "providers">>, ({
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
    originalArgs?: ProviderDefaultsUpdateRequest | undefined;
    reset: () => void;
}) | ({
    status: QueryStatus.fulfilled;
} & Omit<{
    requestId: string;
    data?: {
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
    originalArgs?: ProviderDefaultsUpdateRequest | undefined;
    reset: () => void;
}) | ({
    status: QueryStatus.pending;
} & {
    requestId: string;
    data?: {
        success: boolean;
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
    originalArgs?: ProviderDefaultsUpdateRequest | undefined;
    reset: () => void;
}) | ({
    status: QueryStatus.rejected;
} & Omit<{
    requestId: string;
    data?: {
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
    originalArgs?: ProviderDefaultsUpdateRequest | undefined;
    reset: () => void;
})];
