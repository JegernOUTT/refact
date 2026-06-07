import { MutationActionCreatorResult, MutationDefinition, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta } from '@reduxjs/toolkit/query';
import { Integration } from "../services/refact/integrations";
export declare const useSaveIntegrationData: () => {
    saveIntegrationMutationTrigger: (filePath: string, values: Integration["integr_values"]) => MutationActionCreatorResult<MutationDefinition<{
        filePath: string;
        values: Integration["integr_values"];
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", unknown, "integrationsApi">>;
};
