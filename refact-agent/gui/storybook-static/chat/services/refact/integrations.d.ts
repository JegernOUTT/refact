import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, MutationDefinition, coreModuleName } from '@reduxjs/toolkit/query';
import { LspChatMessage } from "./chat";
export declare const integrationsApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    getAllIntegrations: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", IntegrationWithIconResponse, "integrationsApi">;
    getMCPLogsByPath: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", MCPLogsResponse, "integrationsApi">;
    getIntegrationByPath: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", Integration, "integrationsApi">;
    saveIntegration: MutationDefinition<{
        filePath: string;
        values: Integration["integr_values"];
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", unknown, "integrationsApi">;
    deleteIntegration: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", unknown, "integrationsApi">;
    mcpOauthStart: MutationDefinition<{
        config_path: string;
        scopes?: string[];
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", MCPOAuthStartResponse, "integrationsApi">;
    mcpOauthExchange: MutationDefinition<{
        session_id: string;
        code: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", {
        success: boolean;
    }, "integrationsApi">;
    mcpOauthLogout: MutationDefinition<{
        config_path: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", {
        success: boolean;
    }, "integrationsApi">;
    mcpOauthCancel: MutationDefinition<{
        session_id: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", {
        cancelled: boolean;
    }, "integrationsApi">;
    mcpOauthStatus: QueryDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", MCPOAuthStatusResponse, "integrationsApi">;
}, "integrationsApi", "INTEGRATIONS" | "INTEGRATION" | "MCP_OAUTH", typeof coreModuleName | typeof reactHooksModuleName>;
export type IntegrationPrimitive = string | number | boolean | null;
export declare function isPrimitive(json: unknown): json is IntegrationPrimitive;
export type ToolConfirmation = {
    ask_user: string[];
    deny: string[];
};
export type SchemaToolConfirmation = {
    ask_user_default: string[];
    deny_default: string[];
    not_applicable?: boolean;
};
export type MCPArgs = string[];
export type MCPEnvs = Record<string, string>;
export type IntegrationFieldValue = IntegrationPrimitive | Record<string, boolean> | Record<string, unknown> | MCPEnvs | MCPArgs | ToolParameterEntity[] | ToolConfirmation;
export type Integration = {
    project_path: string;
    integr_name: string;
    integr_config_path: string;
    integr_schema: IntegrationSchema;
    integr_values: Record<string, IntegrationFieldValue> | null;
    error_log: YamlError[];
};
type MCPLogsResponse = {
    logs: string[];
};
type IntegrationSchema = {
    description?: string;
    fields: Record<string, IntegrationField<NonNullable<IntegrationPrimitive>>>;
    available: Record<string, boolean>;
    confirmation: SchemaToolConfirmation;
    smartlinks?: SmartLink[];
};
export type IntegrationField<T extends IntegrationPrimitive> = {
    f_type: T;
    f_desc?: string;
    f_placeholder?: T;
    f_default?: T | Record<string, IntegrationPrimitive>;
    f_label?: string;
    f_extra?: boolean | Record<string, unknown>;
    smartlinks?: SmartLink[];
};
export type SmartLink = {
    sl_label: string;
    sl_chat?: LspChatMessage[];
    sl_goto?: string;
    sl_enable_only_with_tool?: boolean;
};
export type IntegrationWithIconRecord = {
    project_path: string;
    integr_name: string;
    icon_path: string;
    integr_config_path: string;
    integr_config_exists: boolean;
    on_your_laptop: boolean;
    when_isolated: boolean;
    wasOpenedThroughChat?: boolean;
};
export type IntegrationWithIconRecordAndAddress = IntegrationWithIconRecord & {
    shouldIntermediatePageShowUp?: boolean;
    commandName?: string;
};
export type NotConfiguredIntegrationWithIconRecord = {
    project_path: string[];
    integr_name: string;
    icon_path: string;
    integr_config_path: string[];
    integr_config_exists: false;
    on_your_laptop: boolean;
    when_isolated: boolean;
    commandName?: string;
    wasOpenedThroughChat?: boolean;
};
export type GroupedIntegrationWithIconRecord = {
    project_path: string[];
    integr_name: string;
    integr_config_path: string[];
    integr_config_exists: boolean;
    on_your_laptop: boolean;
    when_isolated: boolean;
};
export declare function areIntegrationsNotConfigured(json: GroupedIntegrationWithIconRecord): json is NotConfiguredIntegrationWithIconRecord;
export declare function isNotConfiguredIntegrationWithIconRecord(json: unknown): json is NotConfiguredIntegrationWithIconRecord;
type YamlError = {
    integr_config_path: string;
    error_line: number;
    error_msg: string;
};
export type IntegrationWithIconResponse = {
    integrations: IntegrationWithIconRecord[];
    error_log: YamlError[];
};
export declare function isIntegrationWithIconResponse(json: unknown): json is IntegrationWithIconResponse;
export type ToolParameterEntity = {
    name: string;
    description: string;
    type?: string;
};
export declare function areToolParameters(json: unknown): json is ToolParameterEntity[];
export declare function areToolConfirmation(json: unknown): json is ToolConfirmation;
export type MCPOAuthStartResponse = {
    session_id: string;
    authorize_url: string;
};
export type MCPOAuthStatusResponse = {
    auth_type: string;
    authenticated: boolean;
    expires_at: number;
    scopes: string[];
};
export {};
