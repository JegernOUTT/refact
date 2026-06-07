import { QueryActionCreatorResult, QueryDefinition, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta } from '@reduxjs/toolkit/query';
export declare const useDeleteIntegrationByPath: () => {
    deleteIntegrationTrigger: (arg: string, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", unknown, "integrationsApi">>;
};
