import { SerializedError } from '@reduxjs/toolkit';
import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, MutationDefinition, coreModuleName, TSHelpersId, QueryStatus, TSHelpersOverride, QuerySubState, skipToken, SubscriptionOptions, QueryActionCreatorResult, MutationActionCreatorResult, TSHelpersNoInfer } from '@reduxjs/toolkit/query';
export type MarketplaceKind = "skill" | "command" | "subagent";
export type MarketplaceItemParam = {
    name: string;
    label: string;
    default?: string;
    required: boolean;
};
export type MarketplaceSourceKind = "builtin_embedded" | "builtin_github" | "user_github";
export type MarketplaceParserMode = "manifest" | "scan" | "overlay";
export type ExtensionMarketplaceSource = {
    id: string;
    label: string;
    description: string;
    enabled: boolean;
    builtin: boolean;
    removable: boolean;
    source_kind: MarketplaceSourceKind;
    repo_url?: string;
    supported_kinds: MarketplaceKind[];
    parser_mode: MarketplaceParserMode;
    last_sync_at?: string;
    error?: string;
    item_count?: number;
};
export type ExtensionMarketplaceItem = {
    id: string;
    name: string;
    description: string;
    tags: string[];
    publisher: string;
    homepage?: string;
    kind: MarketplaceKind;
    source_id: string;
    source_label: string;
    path: string;
    installed_scopes: string[];
    body_preview?: string;
    params?: MarketplaceItemParam[];
};
export type ExtensionMarketplaceResponse = {
    items: ExtensionMarketplaceItem[];
    sources: ExtensionMarketplaceSource[];
};
export type ExtensionMarketplaceInstallResponse = {
    installed: boolean;
    scope: "local" | "global";
    file_path: string;
    item_id: string;
    name: string;
};
export declare const extensionsMarketplaceApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    getExtensionMarketplaceSources: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
        sources: ExtensionMarketplaceSource[];
    }, "extensionsMarketplaceApi">;
    saveExtensionMarketplaceSource: MutationDefinition<{
        url: string;
        enabled: boolean;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
        ok: boolean;
        source: ExtensionMarketplaceSource;
    }, "extensionsMarketplaceApi">;
    deleteExtensionMarketplaceSource: MutationDefinition<{
        id: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
        ok: boolean;
    }, "extensionsMarketplaceApi">;
    configureExtensionMarketplaceSource: MutationDefinition<{
        id: string;
        enabled?: boolean;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
        ok: boolean;
    }, "extensionsMarketplaceApi">;
    getSubagentsMarketplace: QueryDefinition<{
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">;
    getSkillsMarketplace: QueryDefinition<{
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">;
    getCommandsMarketplace: QueryDefinition<{
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">;
    refreshExtensionMarketplaceSource: MutationDefinition<{
        id: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
        ok: boolean;
    }, "extensionsMarketplaceApi">;
    installMarketplaceSkill: MutationDefinition<{
        source_id: string;
        item_id: string;
        scope: "local" | "global";
        overwrite?: boolean;
        params?: Record<string, string>;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceInstallResponse, "extensionsMarketplaceApi">;
    installMarketplaceCommand: MutationDefinition<{
        source_id: string;
        item_id: string;
        scope: "local" | "global";
        overwrite?: boolean;
        params?: Record<string, string>;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceInstallResponse, "extensionsMarketplaceApi">;
    installMarketplaceSubagent: MutationDefinition<{
        source_id: string;
        item_id: string;
        scope: "local" | "global";
        overwrite?: boolean;
        params?: Record<string, string>;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceInstallResponse, "extensionsMarketplaceApi">;
}, "extensionsMarketplaceApi", "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", typeof coreModuleName | typeof reactHooksModuleName>;
export declare const useGetExtensionMarketplaceSourcesQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
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
        sources: ExtensionMarketplaceSource[];
    } | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
    sources: ExtensionMarketplaceSource[];
}, "extensionsMarketplaceApi">> & {
    currentData?: {
        sources: ExtensionMarketplaceSource[];
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
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
    sources: ExtensionMarketplaceSource[];
}, "extensionsMarketplaceApi">> & {
    currentData?: {
        sources: ExtensionMarketplaceSource[];
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
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
    sources: ExtensionMarketplaceSource[];
}, "extensionsMarketplaceApi">> & {
    currentData?: {
        sources: ExtensionMarketplaceSource[];
    } | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
    sources: ExtensionMarketplaceSource[];
}, "extensionsMarketplaceApi">> & {
    currentData?: {
        sources: ExtensionMarketplaceSource[];
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
            sources: ExtensionMarketplaceSource[];
        } | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
        sources: ExtensionMarketplaceSource[];
    }, "extensionsMarketplaceApi">> & {
        currentData?: {
            sources: ExtensionMarketplaceSource[];
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
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
        sources: ExtensionMarketplaceSource[];
    }, "extensionsMarketplaceApi">> & {
        currentData?: {
            sources: ExtensionMarketplaceSource[];
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
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
        sources: ExtensionMarketplaceSource[];
    }, "extensionsMarketplaceApi">> & {
        currentData?: {
            sources: ExtensionMarketplaceSource[];
        } | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
        sources: ExtensionMarketplaceSource[];
    }, "extensionsMarketplaceApi">> & {
        currentData?: {
            sources: ExtensionMarketplaceSource[];
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
    refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
        sources: ExtensionMarketplaceSource[];
    }, "extensionsMarketplaceApi">>;
}, useSaveExtensionMarketplaceSourceMutation: <R extends Record<string, any> = ({
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
        source: ExtensionMarketplaceSource;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        ok: boolean;
        source: ExtensionMarketplaceSource;
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
        source: ExtensionMarketplaceSource;
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
        source: ExtensionMarketplaceSource;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        ok: boolean;
        source: ExtensionMarketplaceSource;
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
            source: ExtensionMarketplaceSource;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            ok: boolean;
            source: ExtensionMarketplaceSource;
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
            source: ExtensionMarketplaceSource;
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
            source: ExtensionMarketplaceSource;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            ok: boolean;
            source: ExtensionMarketplaceSource;
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
} | undefined) => readonly [(arg: {
    url: string;
    enabled: boolean;
}) => MutationActionCreatorResult<MutationDefinition<{
    url: string;
    enabled: boolean;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
    ok: boolean;
    source: ExtensionMarketplaceSource;
}, "extensionsMarketplaceApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        url: string;
        enabled: boolean;
    } | undefined;
    reset: () => void;
}], useDeleteExtensionMarketplaceSourceMutation: <R extends Record<string, any> = ({
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
} | undefined) => readonly [(arg: {
    id: string;
}) => MutationActionCreatorResult<MutationDefinition<{
    id: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
    ok: boolean;
}, "extensionsMarketplaceApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        id: string;
    } | undefined;
    reset: () => void;
}], useConfigureExtensionMarketplaceSourceMutation: <R extends Record<string, any> = ({
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
} | undefined) => readonly [(arg: {
    id: string;
    enabled?: boolean;
}) => MutationActionCreatorResult<MutationDefinition<{
    id: string;
    enabled?: boolean;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
    ok: boolean;
}, "extensionsMarketplaceApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        id: string;
        enabled?: boolean;
    } | undefined;
    reset: () => void;
}], useRefreshExtensionMarketplaceSourceMutation: <R extends Record<string, any> = ({
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
} | undefined) => readonly [(arg: {
    id: string;
}) => MutationActionCreatorResult<MutationDefinition<{
    id: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", {
    ok: boolean;
}, "extensionsMarketplaceApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        id: string;
    } | undefined;
    reset: () => void;
}], useGetSkillsMarketplaceQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ExtensionMarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    source?: string;
    q?: string;
} | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
    currentData?: ExtensionMarketplaceResponse | undefined;
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
    source?: string;
    q?: string;
} | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
    currentData?: ExtensionMarketplaceResponse | undefined;
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
    source?: string;
    q?: string;
} | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
    currentData?: ExtensionMarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    source?: string;
    q?: string;
} | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
    currentData?: ExtensionMarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: {
    source?: string;
    q?: string;
} | typeof skipToken | undefined, options?: (SubscriptionOptions & {
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
        currentData?: ExtensionMarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
        currentData?: ExtensionMarketplaceResponse | undefined;
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
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
        currentData?: ExtensionMarketplaceResponse | undefined;
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
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
        currentData?: ExtensionMarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<{
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
        currentData?: ExtensionMarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<{
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">>;
}, useGetCommandsMarketplaceQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ExtensionMarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    source?: string;
    q?: string;
} | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
    currentData?: ExtensionMarketplaceResponse | undefined;
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
    source?: string;
    q?: string;
} | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
    currentData?: ExtensionMarketplaceResponse | undefined;
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
    source?: string;
    q?: string;
} | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
    currentData?: ExtensionMarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    source?: string;
    q?: string;
} | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
    currentData?: ExtensionMarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: {
    source?: string;
    q?: string;
} | typeof skipToken | undefined, options?: (SubscriptionOptions & {
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
        currentData?: ExtensionMarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
        currentData?: ExtensionMarketplaceResponse | undefined;
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
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
        currentData?: ExtensionMarketplaceResponse | undefined;
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
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
        currentData?: ExtensionMarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<{
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
        currentData?: ExtensionMarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<{
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">>;
}, useGetSubagentsMarketplaceQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ExtensionMarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    source?: string;
    q?: string;
} | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
    currentData?: ExtensionMarketplaceResponse | undefined;
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
    source?: string;
    q?: string;
} | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
    currentData?: ExtensionMarketplaceResponse | undefined;
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
    source?: string;
    q?: string;
} | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
    currentData?: ExtensionMarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    source?: string;
    q?: string;
} | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
    currentData?: ExtensionMarketplaceResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: {
    source?: string;
    q?: string;
} | typeof skipToken | undefined, options?: (SubscriptionOptions & {
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
        currentData?: ExtensionMarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
        currentData?: ExtensionMarketplaceResponse | undefined;
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
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
        currentData?: ExtensionMarketplaceResponse | undefined;
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
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
        currentData?: ExtensionMarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<{
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">> & {
        currentData?: ExtensionMarketplaceResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<{
        source?: string;
        q?: string;
    } | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceResponse, "extensionsMarketplaceApi">>;
}, useInstallMarketplaceSkillMutation: <R extends Record<string, any> = ({
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
    data?: ExtensionMarketplaceInstallResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: ExtensionMarketplaceInstallResponse | undefined;
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
    data?: ExtensionMarketplaceInstallResponse | undefined;
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
    data?: ExtensionMarketplaceInstallResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: ExtensionMarketplaceInstallResponse | undefined;
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
        data?: ExtensionMarketplaceInstallResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: ExtensionMarketplaceInstallResponse | undefined;
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
        data?: ExtensionMarketplaceInstallResponse | undefined;
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
        data?: ExtensionMarketplaceInstallResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: ExtensionMarketplaceInstallResponse | undefined;
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
} | undefined) => readonly [(arg: {
    source_id: string;
    item_id: string;
    scope: "local" | "global";
    overwrite?: boolean;
    params?: Record<string, string>;
}) => MutationActionCreatorResult<MutationDefinition<{
    source_id: string;
    item_id: string;
    scope: "local" | "global";
    overwrite?: boolean;
    params?: Record<string, string>;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceInstallResponse, "extensionsMarketplaceApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        source_id: string;
        item_id: string;
        scope: "local" | "global";
        overwrite?: boolean;
        params?: Record<string, string>;
    } | undefined;
    reset: () => void;
}], useInstallMarketplaceCommandMutation: <R extends Record<string, any> = ({
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
    data?: ExtensionMarketplaceInstallResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: ExtensionMarketplaceInstallResponse | undefined;
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
    data?: ExtensionMarketplaceInstallResponse | undefined;
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
    data?: ExtensionMarketplaceInstallResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: ExtensionMarketplaceInstallResponse | undefined;
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
        data?: ExtensionMarketplaceInstallResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: ExtensionMarketplaceInstallResponse | undefined;
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
        data?: ExtensionMarketplaceInstallResponse | undefined;
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
        data?: ExtensionMarketplaceInstallResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: ExtensionMarketplaceInstallResponse | undefined;
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
} | undefined) => readonly [(arg: {
    source_id: string;
    item_id: string;
    scope: "local" | "global";
    overwrite?: boolean;
    params?: Record<string, string>;
}) => MutationActionCreatorResult<MutationDefinition<{
    source_id: string;
    item_id: string;
    scope: "local" | "global";
    overwrite?: boolean;
    params?: Record<string, string>;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceInstallResponse, "extensionsMarketplaceApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        source_id: string;
        item_id: string;
        scope: "local" | "global";
        overwrite?: boolean;
        params?: Record<string, string>;
    } | undefined;
    reset: () => void;
}], useInstallMarketplaceSubagentMutation: <R extends Record<string, any> = ({
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
    data?: ExtensionMarketplaceInstallResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: ExtensionMarketplaceInstallResponse | undefined;
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
    data?: ExtensionMarketplaceInstallResponse | undefined;
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
    data?: ExtensionMarketplaceInstallResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: ExtensionMarketplaceInstallResponse | undefined;
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
        data?: ExtensionMarketplaceInstallResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: ExtensionMarketplaceInstallResponse | undefined;
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
        data?: ExtensionMarketplaceInstallResponse | undefined;
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
        data?: ExtensionMarketplaceInstallResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: ExtensionMarketplaceInstallResponse | undefined;
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
} | undefined) => readonly [(arg: {
    source_id: string;
    item_id: string;
    scope: "local" | "global";
    overwrite?: boolean;
    params?: Record<string, string>;
}) => MutationActionCreatorResult<MutationDefinition<{
    source_id: string;
    item_id: string;
    scope: "local" | "global";
    overwrite?: boolean;
    params?: Record<string, string>;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "ExtensionMarketplaceSources" | "SkillsMarketplace" | "CommandsMarketplace" | "SubagentsMarketplace", ExtensionMarketplaceInstallResponse, "extensionsMarketplaceApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        source_id: string;
        item_id: string;
        scope: "local" | "global";
        overwrite?: boolean;
        params?: Record<string, string>;
    } | undefined;
    reset: () => void;
}];
