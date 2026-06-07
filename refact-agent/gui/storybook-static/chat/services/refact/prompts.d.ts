import { reactHooksModuleName, UNINITIALIZED_VALUE } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, coreModuleName, ApiEndpointQuery, TSHelpersId, QueryStatus, TSHelpersOverride, QuerySubState, skipToken, SubscriptionOptions, QueryActionCreatorResult } from '@reduxjs/toolkit/query';
export declare const promptsApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    getPrompts: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">;
}, "prompts", never, typeof coreModuleName | typeof reactHooksModuleName>;
export declare const promptsEndpoints: {
    getPrompts: ApiEndpointQuery<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">, {
        getPrompts: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">;
    }>;
} & {
    getPrompts: {
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
            currentData?: SystemPrompts | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
            currentData?: SystemPrompts | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
            currentData?: SystemPrompts | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
            currentData?: SystemPrompts | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
            currentData?: SystemPrompts | undefined;
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
                currentData?: SystemPrompts | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
                currentData?: SystemPrompts | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
                currentData?: SystemPrompts | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
                currentData?: SystemPrompts | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
                currentData?: SystemPrompts | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }) | undefined) => [R][R extends any ? 0 : never] & {
            refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">>;
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
            currentData?: SystemPrompts | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
            currentData?: SystemPrompts | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
            currentData?: SystemPrompts | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
            currentData?: SystemPrompts | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
            currentData?: SystemPrompts | undefined;
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
                currentData?: SystemPrompts | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
                currentData?: SystemPrompts | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
                currentData?: SystemPrompts | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
                currentData?: SystemPrompts | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
                currentData?: SystemPrompts | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: undefined, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">>, [R][R extends any ? 0 : never], {
            lastArg: undefined;
        }];
        useQuerySubscription: (arg: typeof skipToken | undefined, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: undefined, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">>, typeof UNINITIALIZED_VALUE | undefined];
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
            currentData?: SystemPrompts | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
            currentData?: SystemPrompts | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
            currentData?: SystemPrompts | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
            currentData?: SystemPrompts | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
            currentData?: SystemPrompts | undefined;
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
                currentData?: SystemPrompts | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
                currentData?: SystemPrompts | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
                currentData?: SystemPrompts | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
                currentData?: SystemPrompts | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, never, SystemPrompts, "prompts">> & {
                currentData?: SystemPrompts | undefined;
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
export type SystemPrompt = {
    text: string;
    description: string;
};
export type SystemPrompts = Record<string, SystemPrompt>;
export declare function isSystemPrompts(json: unknown): json is SystemPrompts;
export type CustomPromptsResponse = {
    system_prompts: SystemPrompts;
    toolbox_commands: Record<string, unknown>;
};
export declare function isCustomPromptsResponse(json: unknown): json is CustomPromptsResponse;
