import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, MutationDefinition, coreModuleName } from '@reduxjs/toolkit/query';
import { type EngineApiConfig } from "./apiUrl";
export declare const pingApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    ping: QueryDefinition<EngineApiConfig, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PING", string, "pingApi">;
    reset: MutationDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PING", null, "pingApi">;
}, "pingApi", "PING", typeof coreModuleName | typeof reactHooksModuleName>;
