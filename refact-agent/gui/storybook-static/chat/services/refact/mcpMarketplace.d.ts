import { SerializedError } from '@reduxjs/toolkit';
import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, MutationDefinition, coreModuleName, TSHelpersId, QueryStatus, TSHelpersOverride, QuerySubState, skipToken, SubscriptionOptions, QueryActionCreatorResult, MutationActionCreatorResult, TSHelpersNoInfer } from '@reduxjs/toolkit/query';
export type MCPServer = {
    id: string;
    source_id: string;
    name: string;
    description: string;
    publisher: string;
    tags: string[];
    icon_url?: string;
    homepage?: string;
    transport: "stdio" | "http" | "sse";
    install_recipe: {
        command?: string;
        url?: string;
        env?: Record<string, string>;
        headers?: Record<string, string>;
    };
    confirmation_default: string[];
    verified?: boolean;
    use_count?: number;
};
export type MarketplaceSource = {
    id: string;
    label: string;
    type: "refact_index" | "smithery" | "official_mcp";
    enabled: boolean;
    removable: boolean;
    server_count?: number;
    status?: "ok" | "error" | "loading";
    error?: string;
    needs_api_key?: boolean;
    has_api_key?: boolean;
};
export type MarketplacePagination = {
    page: number;
    page_size: number;
    total: number;
};
export type MarketplaceResponse = {
    servers: MCPServer[];
    sources: MarketplaceSource[];
    pagination?: MarketplacePagination;
    source?: "remote" | "local" | "merged";
};
export type MarketplaceQueryParams = {
    source?: string;
    q?: string;
    page?: number;
    page_size?: number;
};
export type InstallRequest = {
    server_id: string;
    source_id?: string;
    config_overrides?: {
        env?: Record<string, string>;
        headers?: Record<string, string>;
    };
};
export type InstallResponse = {
    installed: boolean;
    config_path: string;
};
export type InstalledServer = {
    id: string;
    name: string;
    config_path: string;
};
export type InstalledResponse = {
    installed: InstalledServer[];
};
export type SaveSourceRequest = {
    id: string;
    label: string;
    type: "refact_index";
    url: string;
    enabled: boolean;
};
export type DeleteSourceRequest = {
    id: string;
};
export type ConfigureSourceRequest = {
    id: string;
    api_key?: string;
    enabled?: boolean;
};
export declare const mcpMarketplaceApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    getMarketplace: QueryDefinition<MarketplaceQueryParams | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", MarketplaceResponse, "mcpMarketplaceApi">;
    installServer: MutationDefinition<InstallRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstallResponse, "mcpMarketplaceApi">;
    getInstalledServers: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstalledResponse, "mcpMarketplaceApi">;
    getAutoName: MutationDefinition<AutoNameRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", AutoNameResponse, "mcpMarketplaceApi">;
    getMarketplaceSources: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
        sources: MarketplaceSource[];
    }, "mcpMarketplaceApi">;
    saveMarketplaceSource: MutationDefinition<SaveSourceRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
        ok: boolean;
    }, "mcpMarketplaceApi">;
    deleteMarketplaceSource: MutationDefinition<DeleteSourceRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
        ok: boolean;
    }, "mcpMarketplaceApi">;
    configureMarketplaceSource: MutationDefinition<ConfigureSourceRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
        ok: boolean;
    }, "mcpMarketplaceApi">;
}, "mcpMarketplaceApi", "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", typeof coreModuleName | typeof reactHooksModuleName>;
export type AutoNameRequest = {
    input: string;
};
export type AutoNameResponse = {
    suggested_name: string;
    transport: "stdio" | "http" | "sse";
    config_prefix: string;
};
export declare const useGetMarketplaceQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: MarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<MarketplaceQueryParams | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", MarketplaceResponse, "mcpMarketplaceApi">> & {
    currentData?: MarketplaceResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<MarketplaceQueryParams | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", MarketplaceResponse, "mcpMarketplaceApi">> & {
    currentData?: MarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<MarketplaceQueryParams | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", MarketplaceResponse, "mcpMarketplaceApi">> & {
    currentData?: MarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<MarketplaceQueryParams | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", MarketplaceResponse, "mcpMarketplaceApi">> & {
    currentData?: MarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: MarketplaceQueryParams | typeof skipToken | undefined, options?: (SubscriptionOptions & {
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
        currentData?: MarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<MarketplaceQueryParams | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", MarketplaceResponse, "mcpMarketplaceApi">> & {
        currentData?: MarketplaceResponse | undefined;
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
    } & Required<Pick<QuerySubState<QueryDefinition<MarketplaceQueryParams | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", MarketplaceResponse, "mcpMarketplaceApi">> & {
        currentData?: MarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp">>) | ({
        isSuccess: true;
        isFetching: false;
        error: undefined;
    } & Required<Pick<QuerySubState<QueryDefinition<MarketplaceQueryParams | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", MarketplaceResponse, "mcpMarketplaceApi">> & {
        currentData?: MarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<MarketplaceQueryParams | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", MarketplaceResponse, "mcpMarketplaceApi">> & {
        currentData?: MarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<MarketplaceQueryParams | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", MarketplaceResponse, "mcpMarketplaceApi">>;
}, useInstallServerMutation: <R extends Record<string, any> = ({
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
}) | ({
    status: QueryStatus.fulfilled;
} & Omit<{
    requestId: string;
    data?: InstallResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: InstallResponse | undefined;
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
}) | ({
    status: QueryStatus.pending;
} & {
    requestId: string;
    data?: InstallResponse | undefined;
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
}) | ({
    status: QueryStatus.rejected;
} & Omit<{
    requestId: string;
    data?: InstallResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: InstallResponse | undefined;
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
})>(options?: {
    selectFromResult?: ((state: ({
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
    }) | ({
        status: QueryStatus.fulfilled;
    } & Omit<{
        requestId: string;
        data?: InstallResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: InstallResponse | undefined;
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
    }) | ({
        status: QueryStatus.pending;
    } & {
        requestId: string;
        data?: InstallResponse | undefined;
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
    }) | ({
        status: QueryStatus.rejected;
    } & Omit<{
        requestId: string;
        data?: InstallResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: InstallResponse | undefined;
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
    })) => R) | undefined;
    fixedCacheKey?: string;
} | undefined) => readonly [(arg: InstallRequest) => MutationActionCreatorResult<MutationDefinition<InstallRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstallResponse, "mcpMarketplaceApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: InstallRequest | undefined;
    reset: () => void;
}], useGetInstalledServersQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: InstalledResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstalledResponse, "mcpMarketplaceApi">> & {
    currentData?: InstalledResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstalledResponse, "mcpMarketplaceApi">> & {
    currentData?: InstalledResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstalledResponse, "mcpMarketplaceApi">> & {
    currentData?: InstalledResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstalledResponse, "mcpMarketplaceApi">> & {
    currentData?: InstalledResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: typeof skipToken | undefined, options?: (SubscriptionOptions & {
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
        currentData?: InstalledResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstalledResponse, "mcpMarketplaceApi">> & {
        currentData?: InstalledResponse | undefined;
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
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstalledResponse, "mcpMarketplaceApi">> & {
        currentData?: InstalledResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp">>) | ({
        isSuccess: true;
        isFetching: false;
        error: undefined;
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstalledResponse, "mcpMarketplaceApi">> & {
        currentData?: InstalledResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstalledResponse, "mcpMarketplaceApi">> & {
        currentData?: InstalledResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", InstalledResponse, "mcpMarketplaceApi">>;
}, useGetAutoNameMutation: <R extends Record<string, any> = ({
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
}) | ({
    status: QueryStatus.fulfilled;
} & Omit<{
    requestId: string;
    data?: AutoNameResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: AutoNameResponse | undefined;
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
}) | ({
    status: QueryStatus.pending;
} & {
    requestId: string;
    data?: AutoNameResponse | undefined;
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
}) | ({
    status: QueryStatus.rejected;
} & Omit<{
    requestId: string;
    data?: AutoNameResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: AutoNameResponse | undefined;
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
})>(options?: {
    selectFromResult?: ((state: ({
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
    }) | ({
        status: QueryStatus.fulfilled;
    } & Omit<{
        requestId: string;
        data?: AutoNameResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: AutoNameResponse | undefined;
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
    }) | ({
        status: QueryStatus.pending;
    } & {
        requestId: string;
        data?: AutoNameResponse | undefined;
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
    }) | ({
        status: QueryStatus.rejected;
    } & Omit<{
        requestId: string;
        data?: AutoNameResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: AutoNameResponse | undefined;
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
    })) => R) | undefined;
    fixedCacheKey?: string;
} | undefined) => readonly [(arg: AutoNameRequest) => MutationActionCreatorResult<MutationDefinition<AutoNameRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", AutoNameResponse, "mcpMarketplaceApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: AutoNameRequest | undefined;
    reset: () => void;
}], useGetMarketplaceSourcesQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
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
        sources: MarketplaceSource[];
    } | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
    sources: MarketplaceSource[];
}, "mcpMarketplaceApi">> & {
    currentData?: {
        sources: MarketplaceSource[];
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
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
    sources: MarketplaceSource[];
}, "mcpMarketplaceApi">> & {
    currentData?: {
        sources: MarketplaceSource[];
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
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
    sources: MarketplaceSource[];
}, "mcpMarketplaceApi">> & {
    currentData?: {
        sources: MarketplaceSource[];
    } | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
    sources: MarketplaceSource[];
}, "mcpMarketplaceApi">> & {
    currentData?: {
        sources: MarketplaceSource[];
    } | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: typeof skipToken | undefined, options?: (SubscriptionOptions & {
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
        currentData?: {
            sources: MarketplaceSource[];
        } | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
        sources: MarketplaceSource[];
    }, "mcpMarketplaceApi">> & {
        currentData?: {
            sources: MarketplaceSource[];
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
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
        sources: MarketplaceSource[];
    }, "mcpMarketplaceApi">> & {
        currentData?: {
            sources: MarketplaceSource[];
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
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
        sources: MarketplaceSource[];
    }, "mcpMarketplaceApi">> & {
        currentData?: {
            sources: MarketplaceSource[];
        } | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
        sources: MarketplaceSource[];
    }, "mcpMarketplaceApi">> & {
        currentData?: {
            sources: MarketplaceSource[];
        } | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
        sources: MarketplaceSource[];
    }, "mcpMarketplaceApi">>;
}, useSaveMarketplaceSourceMutation: <R extends Record<string, any> = ({
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
}) | ({
    status: QueryStatus.fulfilled;
} & Omit<{
    requestId: string;
    data?: {
        ok: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        ok: boolean;
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
}) | ({
    status: QueryStatus.pending;
} & {
    requestId: string;
    data?: {
        ok: boolean;
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
}) | ({
    status: QueryStatus.rejected;
} & Omit<{
    requestId: string;
    data?: {
        ok: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        ok: boolean;
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
})>(options?: {
    selectFromResult?: ((state: ({
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
    }) | ({
        status: QueryStatus.fulfilled;
    } & Omit<{
        requestId: string;
        data?: {
            ok: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            ok: boolean;
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
    }) | ({
        status: QueryStatus.pending;
    } & {
        requestId: string;
        data?: {
            ok: boolean;
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
    }) | ({
        status: QueryStatus.rejected;
    } & Omit<{
        requestId: string;
        data?: {
            ok: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            ok: boolean;
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
    })) => R) | undefined;
    fixedCacheKey?: string;
} | undefined) => readonly [(arg: SaveSourceRequest) => MutationActionCreatorResult<MutationDefinition<SaveSourceRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
    ok: boolean;
}, "mcpMarketplaceApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: SaveSourceRequest | undefined;
    reset: () => void;
}], useDeleteMarketplaceSourceMutation: <R extends Record<string, any> = ({
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
}) | ({
    status: QueryStatus.fulfilled;
} & Omit<{
    requestId: string;
    data?: {
        ok: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        ok: boolean;
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
}) | ({
    status: QueryStatus.pending;
} & {
    requestId: string;
    data?: {
        ok: boolean;
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
}) | ({
    status: QueryStatus.rejected;
} & Omit<{
    requestId: string;
    data?: {
        ok: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        ok: boolean;
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
})>(options?: {
    selectFromResult?: ((state: ({
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
    }) | ({
        status: QueryStatus.fulfilled;
    } & Omit<{
        requestId: string;
        data?: {
            ok: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            ok: boolean;
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
    }) | ({
        status: QueryStatus.pending;
    } & {
        requestId: string;
        data?: {
            ok: boolean;
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
    }) | ({
        status: QueryStatus.rejected;
    } & Omit<{
        requestId: string;
        data?: {
            ok: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            ok: boolean;
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
    })) => R) | undefined;
    fixedCacheKey?: string;
} | undefined) => readonly [(arg: DeleteSourceRequest) => MutationActionCreatorResult<MutationDefinition<DeleteSourceRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
    ok: boolean;
}, "mcpMarketplaceApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: DeleteSourceRequest | undefined;
    reset: () => void;
}], useConfigureMarketplaceSourceMutation: <R extends Record<string, any> = ({
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
}) | ({
    status: QueryStatus.fulfilled;
} & Omit<{
    requestId: string;
    data?: {
        ok: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        ok: boolean;
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
}) | ({
    status: QueryStatus.pending;
} & {
    requestId: string;
    data?: {
        ok: boolean;
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
}) | ({
    status: QueryStatus.rejected;
} & Omit<{
    requestId: string;
    data?: {
        ok: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        ok: boolean;
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
})>(options?: {
    selectFromResult?: ((state: ({
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
    }) | ({
        status: QueryStatus.fulfilled;
    } & Omit<{
        requestId: string;
        data?: {
            ok: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            ok: boolean;
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
    }) | ({
        status: QueryStatus.pending;
    } & {
        requestId: string;
        data?: {
            ok: boolean;
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
    }) | ({
        status: QueryStatus.rejected;
    } & Omit<{
        requestId: string;
        data?: {
            ok: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            ok: boolean;
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
    })) => R) | undefined;
    fixedCacheKey?: string;
} | undefined) => readonly [(arg: ConfigureSourceRequest) => MutationActionCreatorResult<MutationDefinition<ConfigureSourceRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MarketplaceServers" | "InstalledServers" | "MarketplaceSources", {
    ok: boolean;
}, "mcpMarketplaceApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: ConfigureSourceRequest | undefined;
    reset: () => void;
}];
