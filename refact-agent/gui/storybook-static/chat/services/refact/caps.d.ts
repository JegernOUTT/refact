import { reactHooksModuleName, UNINITIALIZED_VALUE } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, coreModuleName, ApiEndpointQuery, TSHelpersId, QueryStatus, TSHelpersOverride, QuerySubState, skipToken, SubscriptionOptions, QueryActionCreatorResult } from '@reduxjs/toolkit/query';
import { CodeChatModel, CodeCompletionModel, EmbeddingModel } from "./models";
export declare const capsApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    getCaps: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">;
}, "caps", never, typeof coreModuleName | typeof reactHooksModuleName>;
export declare const capsEndpoints: {
    getCaps: ApiEndpointQuery<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">, {
        getCaps: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">;
    }>;
} & {
    getCaps: {
        useQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
            status: QueryStatus.uninitialized;
            originalArgs?: undefined | undefined;
            data?: undefined | undefined;
            error?: undefined | undefined;
            requestId?: undefined | undefined;
            endpointName?: string | undefined;
            startedTimeStamp?: undefined | undefined;
            fulfilledTimeStamp?: undefined | undefined;
        } & {
            currentData?: CapsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
            currentData?: CapsResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
            currentData?: CapsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
            currentData?: CapsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
            currentData?: CapsResponse | undefined;
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
                currentData?: CapsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
                currentData?: CapsResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
                currentData?: CapsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
                currentData?: CapsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
                currentData?: CapsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }) | undefined) => [R][R extends any ? 0 : never] & {
            refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">>;
        };
        useLazyQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
            status: QueryStatus.uninitialized;
            originalArgs?: undefined | undefined;
            data?: undefined | undefined;
            error?: undefined | undefined;
            requestId?: undefined | undefined;
            endpointName?: string | undefined;
            startedTimeStamp?: undefined | undefined;
            fulfilledTimeStamp?: undefined | undefined;
        } & {
            currentData?: CapsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
            currentData?: CapsResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
            currentData?: CapsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
            currentData?: CapsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
            currentData?: CapsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(options?: (SubscriptionOptions & Omit<{
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
                currentData?: CapsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
                currentData?: CapsResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
                currentData?: CapsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
                currentData?: CapsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
                currentData?: CapsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: undefined, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">>, [R][R extends any ? 0 : never], {
            lastArg: undefined;
        }];
        useQuerySubscription: (arg: typeof skipToken | undefined, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: undefined, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">>, typeof UNINITIALIZED_VALUE | undefined];
        useQueryState: <R extends Record<string, any> = TSHelpersId<(Omit<{
            status: QueryStatus.uninitialized;
            originalArgs?: undefined | undefined;
            data?: undefined | undefined;
            error?: undefined | undefined;
            requestId?: undefined | undefined;
            endpointName?: string | undefined;
            startedTimeStamp?: undefined | undefined;
            fulfilledTimeStamp?: undefined | undefined;
        } & {
            currentData?: CapsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
            currentData?: CapsResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
            currentData?: CapsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
            currentData?: CapsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
            currentData?: CapsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: typeof skipToken | undefined, options?: {
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
                currentData?: CapsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
                currentData?: CapsResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
                currentData?: CapsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
                currentData?: CapsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
                currentData?: CapsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        } | undefined) => [R][R extends any ? 0 : never];
    };
};
export declare const useGetCapsQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: CapsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
    currentData?: CapsResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
    currentData?: CapsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
    currentData?: CapsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
    currentData?: CapsResponse | undefined;
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
        currentData?: CapsResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
        currentData?: CapsResponse | undefined;
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
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
        currentData?: CapsResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp">>) | ({
        isSuccess: true;
        isFetching: false;
        error: undefined;
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
        currentData?: CapsResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">> & {
        currentData?: CapsResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, CapsResponse, "caps">>;
};
export type CapCost = {
    prompt: number;
    generated: number;
    cache_read?: number;
    cache_creation?: number;
};
type CapsMetadata = {
    pricing?: Record<string, CapCost>;
    features?: string[];
};
export type CapsResponse = {
    caps_version: number;
    chat_default_model: string;
    chat_models: Record<string, CodeChatModel>;
    code_chat_default_system_prompt: string;
    completion_models: Record<string, CodeCompletionModel>;
    completion_default_model: string;
    code_completion_n_ctx: number;
    embedding_model?: EmbeddingModel;
    chat_model_2: string;
    task_planner_agent_model: string;
    chat_thinking_model: string;
    chat_light_model: string;
    chat_buddy_model: string;
    endpoint_chat_passthrough: string;
    endpoint_style: string;
    endpoint_template: string;
    running_models: string[];
    tokenizer_path_template: string;
    tokenizer_rewrite_path: Record<string, unknown>;
    metadata: CapsMetadata | null;
    customization: string;
};
export declare function isCapsResponse(json: unknown): json is CapsResponse;
type CapsErrorResponse = {
    detail: string;
};
export declare function isCapsErrorResponse(json: unknown): json is CapsErrorResponse;
export {};
