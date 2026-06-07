import type { FetchArgs, FetchBaseQueryError } from "@reduxjs/toolkit/query";
import type { EngineApiConfig } from "./apiUrl";
type QueryState = {
    config: EngineApiConfig;
};
type InnerBaseQuery = (arg: string | FetchArgs) => Promise<{
    data: unknown;
    error?: undefined;
} | {
    error: FetchBaseQueryError;
    data?: undefined;
}>;
export declare function lspQueryFn<TArg, TResult>(buildRequest: (arg: TArg, state: QueryState) => string | FetchArgs): (arg: TArg, api: {
    getState: () => unknown;
}, _opts: object, baseQuery: InnerBaseQuery) => Promise<{
    error: FetchBaseQueryError;
    data?: undefined;
} | {
    data: TResult;
    error?: undefined;
}>;
export {};
