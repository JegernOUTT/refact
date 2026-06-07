import { SerializedError } from '@reduxjs/toolkit';
import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, MutationDefinition, coreModuleName, TSHelpersId, QueryStatus, TSHelpersOverride, QuerySubState, skipToken, SubscriptionOptions, QueryActionCreatorResult, MutationActionCreatorResult, TSHelpersNoInfer } from '@reduxjs/toolkit/query';
export type TaskDocumentKind = "plan" | "design" | "runbook" | "brief" | "postmortem" | "spec";
export interface TaskDocumentSummary {
    slug: string;
    name: string;
    kind: TaskDocumentKind;
    pinned: boolean;
    version: number;
    updated_at: string;
    created_at: string;
    author_role: string;
    relevant_cards: string[];
}
export interface TaskDocumentListResponse {
    task_id: string;
    documents: TaskDocumentSummary[];
}
export interface TaskDocumentDetail {
    slug: string;
    name: string;
    kind: TaskDocumentKind;
    pinned: boolean;
    version: number;
    content: string;
    created_at: string;
    updated_at: string;
    author_role: string;
    relevant_cards: string[];
}
export interface TaskDocumentHistoryEntry {
    version: number;
    updated_at: string;
    author_role: string;
    size_bytes: number;
}
export interface TaskDocumentHistoryResponse {
    task_id: string;
    slug: string;
    history: TaskDocumentHistoryEntry[];
}
export interface CreateTaskDocumentRequest {
    taskId: string;
    slug: string;
    name: string;
    kind: TaskDocumentKind;
    content: string;
    pinned?: boolean;
}
export interface UpdateTaskDocumentRequest {
    taskId: string;
    slug: string;
    content: string;
}
export interface DeleteTaskDocumentRequest {
    taskId: string;
    slug: string;
}
export interface DeleteTaskDocumentResponse {
    ok: boolean;
}
export interface PinTaskDocumentRequest {
    taskId: string;
    slug: string;
    pinned: boolean;
}
export interface AppendTaskDocumentRequest {
    taskId: string;
    slug: string;
    section: string;
}
export declare function isTaskDocumentDetail(value: unknown): value is TaskDocumentDetail;
export declare function isTaskDocumentSummary(value: unknown): value is TaskDocumentSummary;
export declare function isTaskDocumentHistoryResponse(value: unknown): value is TaskDocumentHistoryResponse;
export type TaskDocumentsTag = {
    type: "TaskDocuments";
    id: string;
};
export declare const taskDocumentListTag: (taskId: string) => TaskDocumentsTag;
export declare const taskDocumentDetailTag: (taskId: string, slug: string) => TaskDocumentsTag;
export declare const taskDocumentHistoryTag: (taskId: string, slug: string) => TaskDocumentsTag;
export declare const taskDocumentMutationInvalidation: {
    createTaskDocument: (taskId: string) => TaskDocumentsTag[];
    updateTaskDocument: (taskId: string, slug: string) => TaskDocumentsTag[];
    pinTaskDocument: (taskId: string, slug: string) => TaskDocumentsTag[];
    deleteTaskDocument: (taskId: string, slug: string) => TaskDocumentsTag[];
    appendTaskDocument: (taskId: string, slug: string) => TaskDocumentsTag[];
};
export declare const taskDocumentsApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    listTaskDocuments: QueryDefinition<{
        taskId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentListResponse, "taskDocumentsApi">;
    getTaskDocument: QueryDefinition<{
        taskId: string;
        slug: string;
        version?: number;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">;
    createTaskDocument: MutationDefinition<CreateTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">;
    updateTaskDocument: MutationDefinition<UpdateTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">;
    appendTaskDocument: MutationDefinition<AppendTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">;
    deleteTaskDocument: MutationDefinition<DeleteTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", DeleteTaskDocumentResponse, "taskDocumentsApi">;
    pinTaskDocument: MutationDefinition<PinTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">;
    getTaskDocumentHistory: QueryDefinition<{
        taskId: string;
        slug: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentHistoryResponse, "taskDocumentsApi">;
}, "taskDocumentsApi", "TaskDocuments", typeof coreModuleName | typeof reactHooksModuleName>;
export declare const useListTaskDocumentsQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: TaskDocumentListResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    taskId: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentListResponse, "taskDocumentsApi">> & {
    currentData?: TaskDocumentListResponse | undefined;
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
    taskId: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentListResponse, "taskDocumentsApi">> & {
    currentData?: TaskDocumentListResponse | undefined;
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
    taskId: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentListResponse, "taskDocumentsApi">> & {
    currentData?: TaskDocumentListResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    taskId: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentListResponse, "taskDocumentsApi">> & {
    currentData?: TaskDocumentListResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: {
    taskId: string;
} | typeof skipToken, options?: (SubscriptionOptions & {
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
        currentData?: TaskDocumentListResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
        taskId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentListResponse, "taskDocumentsApi">> & {
        currentData?: TaskDocumentListResponse | undefined;
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
        taskId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentListResponse, "taskDocumentsApi">> & {
        currentData?: TaskDocumentListResponse | undefined;
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
        taskId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentListResponse, "taskDocumentsApi">> & {
        currentData?: TaskDocumentListResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<{
        taskId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentListResponse, "taskDocumentsApi">> & {
        currentData?: TaskDocumentListResponse | undefined;
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
        taskId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentListResponse, "taskDocumentsApi">>;
}, useGetTaskDocumentQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: TaskDocumentDetail | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    taskId: string;
    slug: string;
    version?: number;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">> & {
    currentData?: TaskDocumentDetail | undefined;
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
    taskId: string;
    slug: string;
    version?: number;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">> & {
    currentData?: TaskDocumentDetail | undefined;
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
    taskId: string;
    slug: string;
    version?: number;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">> & {
    currentData?: TaskDocumentDetail | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    taskId: string;
    slug: string;
    version?: number;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">> & {
    currentData?: TaskDocumentDetail | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: {
    taskId: string;
    slug: string;
    version?: number;
} | typeof skipToken, options?: (SubscriptionOptions & {
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
        currentData?: TaskDocumentDetail | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
        taskId: string;
        slug: string;
        version?: number;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">> & {
        currentData?: TaskDocumentDetail | undefined;
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
        taskId: string;
        slug: string;
        version?: number;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">> & {
        currentData?: TaskDocumentDetail | undefined;
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
        taskId: string;
        slug: string;
        version?: number;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">> & {
        currentData?: TaskDocumentDetail | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<{
        taskId: string;
        slug: string;
        version?: number;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">> & {
        currentData?: TaskDocumentDetail | undefined;
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
        taskId: string;
        slug: string;
        version?: number;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">>;
}, useCreateTaskDocumentMutation: <R extends Record<string, any> = ({
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
    data?: TaskDocumentDetail | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: TaskDocumentDetail | undefined;
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
    data?: TaskDocumentDetail | undefined;
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
    data?: TaskDocumentDetail | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: TaskDocumentDetail | undefined;
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
        data?: TaskDocumentDetail | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: TaskDocumentDetail | undefined;
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
        data?: TaskDocumentDetail | undefined;
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
        data?: TaskDocumentDetail | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: TaskDocumentDetail | undefined;
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
} | undefined) => readonly [(arg: CreateTaskDocumentRequest) => MutationActionCreatorResult<MutationDefinition<CreateTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: CreateTaskDocumentRequest | undefined;
    reset: () => void;
}], useUpdateTaskDocumentMutation: <R extends Record<string, any> = ({
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
    data?: TaskDocumentDetail | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: TaskDocumentDetail | undefined;
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
    data?: TaskDocumentDetail | undefined;
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
    data?: TaskDocumentDetail | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: TaskDocumentDetail | undefined;
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
        data?: TaskDocumentDetail | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: TaskDocumentDetail | undefined;
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
        data?: TaskDocumentDetail | undefined;
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
        data?: TaskDocumentDetail | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: TaskDocumentDetail | undefined;
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
} | undefined) => readonly [(arg: UpdateTaskDocumentRequest) => MutationActionCreatorResult<MutationDefinition<UpdateTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: UpdateTaskDocumentRequest | undefined;
    reset: () => void;
}], useAppendTaskDocumentMutation: <R extends Record<string, any> = ({
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
    data?: TaskDocumentDetail | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: TaskDocumentDetail | undefined;
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
    data?: TaskDocumentDetail | undefined;
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
    data?: TaskDocumentDetail | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: TaskDocumentDetail | undefined;
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
        data?: TaskDocumentDetail | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: TaskDocumentDetail | undefined;
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
        data?: TaskDocumentDetail | undefined;
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
        data?: TaskDocumentDetail | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: TaskDocumentDetail | undefined;
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
} | undefined) => readonly [(arg: AppendTaskDocumentRequest) => MutationActionCreatorResult<MutationDefinition<AppendTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: AppendTaskDocumentRequest | undefined;
    reset: () => void;
}], useDeleteTaskDocumentMutation: <R extends Record<string, any> = ({
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
    data?: DeleteTaskDocumentResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: DeleteTaskDocumentResponse | undefined;
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
    data?: DeleteTaskDocumentResponse | undefined;
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
    data?: DeleteTaskDocumentResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: DeleteTaskDocumentResponse | undefined;
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
        data?: DeleteTaskDocumentResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: DeleteTaskDocumentResponse | undefined;
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
        data?: DeleteTaskDocumentResponse | undefined;
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
        data?: DeleteTaskDocumentResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: DeleteTaskDocumentResponse | undefined;
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
} | undefined) => readonly [(arg: DeleteTaskDocumentRequest) => MutationActionCreatorResult<MutationDefinition<DeleteTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", DeleteTaskDocumentResponse, "taskDocumentsApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: DeleteTaskDocumentRequest | undefined;
    reset: () => void;
}], usePinTaskDocumentMutation: <R extends Record<string, any> = ({
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
    data?: TaskDocumentDetail | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: TaskDocumentDetail | undefined;
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
    data?: TaskDocumentDetail | undefined;
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
    data?: TaskDocumentDetail | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: TaskDocumentDetail | undefined;
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
        data?: TaskDocumentDetail | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: TaskDocumentDetail | undefined;
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
        data?: TaskDocumentDetail | undefined;
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
        data?: TaskDocumentDetail | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: TaskDocumentDetail | undefined;
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
} | undefined) => readonly [(arg: PinTaskDocumentRequest) => MutationActionCreatorResult<MutationDefinition<PinTaskDocumentRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentDetail, "taskDocumentsApi">>, TSHelpersNoInfer<R> & {
    originalArgs?: PinTaskDocumentRequest | undefined;
    reset: () => void;
}], useGetTaskDocumentHistoryQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: TaskDocumentHistoryResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    taskId: string;
    slug: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentHistoryResponse, "taskDocumentsApi">> & {
    currentData?: TaskDocumentHistoryResponse | undefined;
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
    taskId: string;
    slug: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentHistoryResponse, "taskDocumentsApi">> & {
    currentData?: TaskDocumentHistoryResponse | undefined;
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
    taskId: string;
    slug: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentHistoryResponse, "taskDocumentsApi">> & {
    currentData?: TaskDocumentHistoryResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    taskId: string;
    slug: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentHistoryResponse, "taskDocumentsApi">> & {
    currentData?: TaskDocumentHistoryResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: {
    taskId: string;
    slug: string;
} | typeof skipToken, options?: (SubscriptionOptions & {
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
        currentData?: TaskDocumentHistoryResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
        taskId: string;
        slug: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentHistoryResponse, "taskDocumentsApi">> & {
        currentData?: TaskDocumentHistoryResponse | undefined;
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
        taskId: string;
        slug: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentHistoryResponse, "taskDocumentsApi">> & {
        currentData?: TaskDocumentHistoryResponse | undefined;
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
        taskId: string;
        slug: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentHistoryResponse, "taskDocumentsApi">> & {
        currentData?: TaskDocumentHistoryResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<{
        taskId: string;
        slug: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentHistoryResponse, "taskDocumentsApi">> & {
        currentData?: TaskDocumentHistoryResponse | undefined;
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
        taskId: string;
        slug: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "TaskDocuments", TaskDocumentHistoryResponse, "taskDocumentsApi">>;
};
