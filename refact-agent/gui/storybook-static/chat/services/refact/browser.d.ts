import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, MutationDefinition, coreModuleName } from '@reduxjs/toolkit/query';
export type BrowserStartRequest = {
    chat_id: string;
};
export type BrowserStartResponse = {
    runtime_id: string;
    status: "started" | "already_running";
};
export type BrowserStopRequest = {
    chat_id: string;
};
export type BrowserStopResponse = {
    status: "stopped";
};
export type BrowserScreenshotRequest = {
    chat_id: string;
    full_page: boolean;
};
export type BrowserScreenshotResponse = {
    mime: string;
    data: string;
    url: string;
    title: string;
};
export type BrowserContextRequest = {
    chat_id: string;
    max_bytes?: number;
    last_n_actions?: number;
    skip_cursor?: boolean;
};
export type BrowserContextResponse = {
    url: string;
    title: string;
    actions: unknown[];
    console: unknown[];
    network: unknown[];
    mutations: unknown[];
    total_bytes: number;
};
export type BrowserCurlRequest = {
    chat_id: string;
};
export type BrowserCurlResponse = {
    curl: string;
    url: string;
    method: string;
    status: number;
};
export type BrowserElementPickRequest = {
    chat_id: string;
};
export type BrowserElementPickResponse = {
    status: "picker_active";
};
export type BrowserElementPickResultRequest = {
    chat_id: string;
};
export type BrowserElementPickResultResponse = {
    status: "waiting";
} | {
    selector: string;
    innerText: string;
    bbox: {
        x: number;
        y: number;
        width: number;
        height: number;
    };
};
export type BrowserRecordAnimationRequest = {
    chat_id: string;
};
export type BrowserRecordAnimationResponse = {
    frames: {
        mime: string;
        data: string;
        timestamp: number;
    }[];
};
export type BrowserHandoffRequest = {
    from_chat_id: string;
    to_chat_id: string;
};
export type BrowserHandoffResponse = {
    runtime_id: string;
    status: string;
    from_chat_id: string;
    to_chat_id: string;
};
export type BrowserAnnotateStartRequest = {
    chat_id: string;
};
export type BrowserAnnotateStartResponse = {
    status: "started" | "already_active";
};
export type BrowserAnnotation = {
    index: number;
    type?: "element" | "rect";
    selector: string;
    innerText: string;
    caption?: string;
    bbox: {
        x: number;
        y: number;
        width: number;
        height: number;
    };
};
export type BrowserAnnotateResultRequest = {
    chat_id: string;
};
export type BrowserAnnotateResultResponse = {
    annotations: BrowserAnnotation[];
    active: boolean;
};
export type BrowserAnnotateClearRequest = {
    chat_id: string;
};
export type BrowserAnnotateClearResponse = {
    status: "cleared";
};
export type BrowserContextEstimateRequest = {
    chat_id: string;
    include_actions: boolean;
    include_console: boolean;
    include_network: boolean;
    include_mutations: boolean;
    include_screenshot: boolean;
    last_n_actions: number;
    last_n_console: number;
    last_n_network: number;
};
export type BrowserContextEstimateResponse = {
    estimated_bytes: number;
};
export type BrowserStatusRequest = {
    chat_id: string;
};
export type BrowserStatusResponse = {
    runtime_id: string | null;
    connected: boolean;
    active_tab?: string | null;
    url?: string;
    title?: string;
    tab_urls?: string[];
    tabs?: {
        tab_id: string;
        url: string;
        title: string;
    }[];
    idle_seconds?: number;
    idle_timeout?: number;
};
export type BrowserLocator = {
    by: string;
    value?: string;
    exact?: boolean;
    role?: string;
    name?: string;
    nth?: number;
    within?: string;
};
export type BrowserTabTarget = {
    type: "active";
} | {
    type: "id";
    id: string;
};
export type BrowserStep = {
    action: string;
    [key: string]: unknown;
};
export type BrowserActionRequest = {
    chat_id: string;
    session?: "shared_default";
    target?: BrowserTabTarget;
    steps: BrowserStep[];
};
export type BrowserExecutionStep = {
    step_index: number;
    ok: boolean;
    summary: string;
    error?: string | null;
    data?: Record<string, unknown> | null;
    field_kind?: string | null;
    fill_strategy?: string | null;
    verified?: boolean | null;
    retries: number;
};
export type BrowserActionResponse = {
    ok: boolean;
    steps: BrowserExecutionStep[];
    url?: string | null;
    title?: string | null;
};
export declare const browserApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    browserStart: MutationDefinition<BrowserStartRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserStartResponse, "browserApi">;
    browserStop: MutationDefinition<BrowserStopRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserStopResponse, "browserApi">;
    browserScreenshot: MutationDefinition<BrowserScreenshotRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserScreenshotResponse, "browserApi">;
    browserContext: MutationDefinition<BrowserContextRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserContextResponse, "browserApi">;
    browserCurl: MutationDefinition<BrowserCurlRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserCurlResponse, "browserApi">;
    browserElementPick: MutationDefinition<BrowserElementPickRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserElementPickResponse, "browserApi">;
    browserElementPickResult: MutationDefinition<BrowserElementPickResultRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserElementPickResultResponse, "browserApi">;
    browserRecordAnimation: MutationDefinition<BrowserRecordAnimationRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserRecordAnimationResponse, "browserApi">;
    browserHandoff: MutationDefinition<BrowserHandoffRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserHandoffResponse, "browserApi">;
    browserStatus: MutationDefinition<BrowserStatusRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserStatusResponse, "browserApi">;
    browserAnnotateStart: MutationDefinition<BrowserAnnotateStartRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserAnnotateStartResponse, "browserApi">;
    browserAnnotateResult: MutationDefinition<BrowserAnnotateResultRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserAnnotateResultResponse, "browserApi">;
    browserAnnotateClear: MutationDefinition<BrowserAnnotateClearRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserAnnotateClearResponse, "browserApi">;
    browserContextEstimate: MutationDefinition<BrowserContextEstimateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserContextEstimateResponse, "browserApi">;
    browserAction: MutationDefinition<BrowserActionRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "BROWSER", BrowserActionResponse, "browserApi">;
}, "browserApi", "BROWSER", typeof coreModuleName | typeof reactHooksModuleName>;
