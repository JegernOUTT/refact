import { SerializedError } from '@reduxjs/toolkit';
import { reactHooksModuleName, UNINITIALIZED_VALUE } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, MutationDefinition, coreModuleName, ApiEndpointQuery, ApiEndpointMutation, TSHelpersId, QueryStatus, TSHelpersOverride, QuerySubState, skipToken, SubscriptionOptions, QueryActionCreatorResult, MutationActionCreatorResult, TSHelpersNoInfer } from '@reduxjs/toolkit/query';
export type WireFormat = "openai_chat_completions" | "openai_responses" | "anthropic_messages" | "refact";
export type ProviderModel = {
    id: string;
    base_name: string;
    enabled: boolean;
    n_ctx: number;
    supports_tools: boolean;
    supports_multimodality: boolean;
    reasoning_effort_options?: string[] | null;
    supports_thinking_budget?: boolean;
    supports_adaptive_thinking_budget?: boolean;
    supports_cache_control?: boolean;
    supports_agent: boolean;
    wire_format_override: WireFormat | null;
    endpoint_override: string | null;
    user_configured: boolean;
    removable: boolean;
};
export type ProviderRuntime = {
    name: string;
    base_provider: string;
    display_name: string;
    enabled: boolean;
    readonly: boolean;
    wire_format: WireFormat;
    chat_endpoint: string;
    completion_endpoint: string;
    embedding_endpoint: string;
    chat_models: ProviderModel[];
    completion_models: ProviderModel[];
    embedding_model: ProviderModel | null;
};
export type ProviderStatus = "not_configured" | "configured" | "active";
export type ProviderListItem = {
    name: string;
    base_provider: string;
    display_name: string;
    enabled: boolean;
    readonly: boolean;
    has_credentials: boolean;
    status: ProviderStatus;
    model_count: number;
};
export type ProviderListResponse = {
    providers: ProviderListItem[];
};
export type ProviderDetailResponse = {
    name: string;
    base_provider: string;
    display_name: string;
    enabled: boolean;
    readonly: boolean;
    has_credentials: boolean;
    selected_models_count: number;
    status: ProviderStatus;
    settings: Record<string, unknown>;
    runtime: ProviderRuntime | null;
};
export type ProviderSchemaResponse = {
    name: string;
    schema: string;
};
export type ProviderModelsResponse = {
    models: ProviderModel[];
};
export type AvailableModel = {
    id: string;
    display_name: string | null;
    n_ctx: number;
    supports_tools: boolean;
    supports_multimodality: boolean;
    reasoning_effort_options?: string[] | null;
    supports_thinking_budget?: boolean;
    supports_adaptive_thinking_budget?: boolean;
    supports_cache_control?: boolean;
    tokenizer: string | null;
    enabled: boolean;
    is_custom: boolean;
    pricing?: {
        prompt: number;
        generated: number;
        cache_read?: number;
        cache_creation?: number;
    } | null;
    available_providers?: string[];
    selected_provider?: string | null;
    max_output_tokens?: number | null;
    provider_variants?: {
        id: string;
        name?: string | null;
        tag?: string | null;
        context_length?: number | null;
        max_output_tokens?: number | null;
        pricing?: {
            prompt: number;
            generated: number;
            cache_read?: number;
            cache_creation?: number;
        } | null;
        latency_last_30m?: number | null;
        throughput_last_30m?: number | null;
        uptime_last_30m?: number | null;
        supported_parameters?: string[] | null;
    }[];
};
export type AvailableModelsResponse = {
    models: AvailableModel[];
    source: "model_caps" | "api" | "local" | "manual";
    error?: string | null;
};
export type ClaudeCodeUsageWindow = {
    percent_used: number;
    resets_at?: string | null;
};
export type ClaudeCodeExtraUsage = {
    is_enabled: boolean;
    used_credits: number;
    monthly_limit?: number | null;
    utilization?: number | null;
};
export type ClaudeCodeUsageData = {
    five_hour?: ClaudeCodeUsageWindow | null;
    seven_day?: ClaudeCodeUsageWindow | null;
    extra_usage?: ClaudeCodeExtraUsage | null;
};
export type ClaudeCodeUsageResponse = {
    data?: ClaudeCodeUsageData | null;
    error?: string | null;
};
export type OpenAICodexUsageWindow = {
    used_percent: number;
    reset_at?: string | null;
};
export type OpenAICodexRateLimit = {
    limit_reached: boolean;
    primary_window?: OpenAICodexUsageWindow | null;
    secondary_window?: OpenAICodexUsageWindow | null;
};
export type OpenAICodexCredits = {
    balance: number;
    unlimited: boolean;
    has_credits: boolean;
};
export type OpenAICodexUsageData = {
    plan_type?: string | null;
    rate_limit?: OpenAICodexRateLimit | null;
    code_review_rate_limit?: OpenAICodexRateLimit | null;
    credits?: OpenAICodexCredits | null;
};
export type OpenAICodexUsageResponse = {
    data?: OpenAICodexUsageData | null;
    error?: string | null;
};
export type OpenRouterAccountInfoResponse = {
    data: {
        key_name?: string | null;
        key_label?: string | null;
        limit?: number | null;
        usage?: number | null;
        remaining?: number | null;
        is_free_tier?: boolean | null;
        rate_limit?: unknown;
    };
};
export type OpenRouterHealthResponse = {
    ok: boolean;
    message?: string | null;
    data?: {
        key_name?: string | null;
        key_label?: string | null;
        rate_limit?: unknown;
    } | null;
};
export type OpenRouterModelEndpointsResponse = {
    provider_variants: NonNullable<AvailableModel["provider_variants"]>;
    available_providers: string[];
};
export type ProviderScopedQueryArg = {
    providerName?: string;
    useInstanceRoute?: boolean;
};
export type ProviderScopedQueryRequiredArg = {
    providerName: string;
    useInstanceRoute?: boolean;
};
export type ProviderIdentitySettings = Pick<ProviderDetailResponse, "base_provider" | "display_name">;
export type ModelPricing = NonNullable<AvailableModel["pricing"]>;
export type ModelToggleRequest = {
    model_id: string;
    enabled: boolean;
};
export type ModelProviderRequest = {
    model_id: string;
    selected_provider?: string | null;
};
export type CustomModelConfig = {
    n_ctx: number;
    supports_tools?: boolean;
    supports_multimodality?: boolean;
    reasoning_effort_options?: string[] | null;
    supports_thinking_budget?: boolean;
    supports_adaptive_thinking_budget?: boolean;
    supports_cache_control?: boolean;
    tokenizer?: string | null;
    pricing?: ModelPricing | null;
    max_output_tokens?: number | null;
};
export type AddCustomModelRequest = {
    id: string;
} & CustomModelConfig;
export type ModelTypeDefaults = {
    model?: string;
    max_new_tokens?: number;
    temperature?: number;
    top_p?: number;
    boost_reasoning?: boolean;
    reasoning_effort?: string;
    thinking_budget?: number;
};
export type ProviderDefaults = {
    chat: ModelTypeDefaults;
    chat_model_2: ModelTypeDefaults;
    task_planner_agent_model: ModelTypeDefaults;
    chat_light: ModelTypeDefaults;
    chat_thinking: ModelTypeDefaults;
    chat_buddy?: ModelTypeDefaults;
    completion_model?: string;
    embedding_model?: string;
};
export type ProviderDefaultsUpdateRequest = ProviderDefaults & {
    draft_id?: string;
};
export type OAuthStartMode = "callback" | "manual_code" | "device";
export type OAuthStartResponse = {
    session_id: string;
    authorize_url: string;
    user_code?: string;
    instructions?: string;
    poll_interval?: number;
    mode?: OAuthStartMode;
};
export type OAuthExchangeResponse = {
    success: boolean;
    auth_status: string;
    status?: string;
    poll_interval?: number;
};
export type ErrorLogInstance = {
    path: string;
    error_line: number;
    error_msg: string;
};
export type ConfiguredProvidersResponse = {
    providers: ProviderListItem[];
    error_log?: ErrorLogInstance[];
};
export type CreateProviderInstanceRequest = {
    base_provider: string;
    display_name: string;
    enabled?: false;
};
export declare function providerIdentitySettings(provider: ProviderIdentitySettings): ProviderIdentitySettings;
export declare const providersApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
    getProvider: QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
    getProviderSchema: QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
    getProviderModels: QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
    getAvailableModels: QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
    getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
        modelId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
    getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
    getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
    getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
    getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
    toggleModel: MutationDefinition<{
        providerName: string;
        modelId: string;
        enabled: boolean;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
        model_id: string;
        enabled: boolean;
    }, "providers">;
    setModelProvider: MutationDefinition<{
        providerName: string;
        modelId: string;
        selectedProvider?: string | null;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
        model_id: string;
        selected_provider?: string | null;
    }, "providers">;
    addCustomModel: MutationDefinition<{
        providerName: string;
        model: AddCustomModelRequest;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
        model_id: string;
    }, "providers">;
    removeCustomModel: MutationDefinition<{
        providerName: string;
        modelId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
        model_id: string;
    }, "providers">;
    updateProvider: MutationDefinition<{
        providerName: string;
        settings: Record<string, unknown>;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
    }, "providers">;
    oauthStart: MutationDefinition<{
        providerName: string;
        mode?: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
    oauthExchange: MutationDefinition<{
        providerName: string;
        session_id: string;
        code: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
    oauthLogout: MutationDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
    }, "providers">;
    deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
    }, "providers">;
    getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
    updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
    }, "providers">;
}, "providers", "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", typeof coreModuleName | typeof reactHooksModuleName>;
export declare function isProviderListResponse(data: unknown): data is ProviderListResponse;
export declare function isProviderDetailResponse(data: unknown): data is ProviderDetailResponse;
export declare const providersEndpoints: {
    getConfiguredProviders: ApiEndpointQuery<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    getProvider: ApiEndpointQuery<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    getProviderSchema: ApiEndpointQuery<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    getProviderModels: ApiEndpointQuery<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    getAvailableModels: ApiEndpointQuery<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    getOpenRouterModelEndpoints: ApiEndpointQuery<QueryDefinition<ProviderScopedQueryRequiredArg & {
        modelId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    getOpenRouterAccountInfo: ApiEndpointQuery<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    getOpenRouterHealth: ApiEndpointQuery<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    getClaudeCodeUsage: ApiEndpointQuery<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    getOpenAICodexUsage: ApiEndpointQuery<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    toggleModel: ApiEndpointMutation<MutationDefinition<{
        providerName: string;
        modelId: string;
        enabled: boolean;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
        model_id: string;
        enabled: boolean;
    }, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    setModelProvider: ApiEndpointMutation<MutationDefinition<{
        providerName: string;
        modelId: string;
        selectedProvider?: string | null;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
        model_id: string;
        selected_provider?: string | null;
    }, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    addCustomModel: ApiEndpointMutation<MutationDefinition<{
        providerName: string;
        model: AddCustomModelRequest;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
        model_id: string;
    }, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    removeCustomModel: ApiEndpointMutation<MutationDefinition<{
        providerName: string;
        modelId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
        model_id: string;
    }, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    updateProvider: ApiEndpointMutation<MutationDefinition<{
        providerName: string;
        settings: Record<string, unknown>;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
    }, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    oauthStart: ApiEndpointMutation<MutationDefinition<{
        providerName: string;
        mode?: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    oauthExchange: ApiEndpointMutation<MutationDefinition<{
        providerName: string;
        session_id: string;
        code: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    oauthLogout: ApiEndpointMutation<MutationDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
    }, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    deleteProvider: ApiEndpointMutation<MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
    }, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    getDefaults: ApiEndpointQuery<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
    updateDefaults: ApiEndpointMutation<MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
    }, "providers">, {
        getConfiguredProviders: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">;
        getProvider: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">;
        getProviderSchema: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">;
        getProviderModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">;
        getAvailableModels: QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">;
        getOpenRouterModelEndpoints: QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">;
        getOpenRouterAccountInfo: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">;
        getOpenRouterHealth: QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">;
        getClaudeCodeUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">;
        getOpenAICodexUsage: QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">;
        toggleModel: MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">;
        setModelProvider: MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">;
        addCustomModel: MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        removeCustomModel: MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">;
        updateProvider: MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        oauthStart: MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">;
        oauthExchange: MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">;
        oauthLogout: MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        deleteProvider: MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
        getDefaults: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">;
        updateDefaults: MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">;
    }>;
} & {
    getConfiguredProviders: {
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
            currentData?: ConfiguredProvidersResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
            currentData?: ConfiguredProvidersResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
            currentData?: ConfiguredProvidersResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
            currentData?: ConfiguredProvidersResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
            currentData?: ConfiguredProvidersResponse | undefined;
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
                currentData?: ConfiguredProvidersResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
                currentData?: ConfiguredProvidersResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
                currentData?: ConfiguredProvidersResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
                currentData?: ConfiguredProvidersResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
                currentData?: ConfiguredProvidersResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }) | undefined) => [R][R extends any ? 0 : never] & {
            refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">>;
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
            currentData?: ConfiguredProvidersResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
            currentData?: ConfiguredProvidersResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
            currentData?: ConfiguredProvidersResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
            currentData?: ConfiguredProvidersResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
            currentData?: ConfiguredProvidersResponse | undefined;
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
                currentData?: ConfiguredProvidersResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
                currentData?: ConfiguredProvidersResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
                currentData?: ConfiguredProvidersResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
                currentData?: ConfiguredProvidersResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
                currentData?: ConfiguredProvidersResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: undefined, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">>, [R][R extends any ? 0 : never], {
            lastArg: undefined;
        }];
        useQuerySubscription: (arg: typeof skipToken | undefined, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: undefined, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">>, typeof UNINITIALIZED_VALUE | undefined];
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
            currentData?: ConfiguredProvidersResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
            currentData?: ConfiguredProvidersResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
            currentData?: ConfiguredProvidersResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
            currentData?: ConfiguredProvidersResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
            currentData?: ConfiguredProvidersResponse | undefined;
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
                currentData?: ConfiguredProvidersResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
                currentData?: ConfiguredProvidersResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
                currentData?: ConfiguredProvidersResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
                currentData?: ConfiguredProvidersResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
                currentData?: ConfiguredProvidersResponse | undefined;
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
    getProvider: {
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
            currentData?: ProviderDetailResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
            currentData?: ProviderDetailResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
            currentData?: ProviderDetailResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
            currentData?: ProviderDetailResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
            currentData?: ProviderDetailResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: {
            providerName: string;
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
                currentData?: ProviderDetailResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
                currentData?: ProviderDetailResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
                currentData?: ProviderDetailResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
                currentData?: ProviderDetailResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
                currentData?: ProviderDetailResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">>;
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
            currentData?: ProviderDetailResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
            currentData?: ProviderDetailResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
            currentData?: ProviderDetailResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
            currentData?: ProviderDetailResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
            currentData?: ProviderDetailResponse | undefined;
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
                currentData?: ProviderDetailResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
                currentData?: ProviderDetailResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
                currentData?: ProviderDetailResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
                currentData?: ProviderDetailResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
                currentData?: ProviderDetailResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: {
            providerName: string;
        }, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">>, [R][R extends any ? 0 : never], {
            lastArg: {
                providerName: string;
            };
        }];
        useQuerySubscription: (arg: {
            providerName: string;
        } | typeof skipToken, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: {
            providerName: string;
        }, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">>, {
            providerName: string;
        } | typeof UNINITIALIZED_VALUE];
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
            currentData?: ProviderDetailResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
            currentData?: ProviderDetailResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
            currentData?: ProviderDetailResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
            currentData?: ProviderDetailResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
            currentData?: ProviderDetailResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: {
            providerName: string;
        } | typeof skipToken, options?: {
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
                currentData?: ProviderDetailResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
                currentData?: ProviderDetailResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
                currentData?: ProviderDetailResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
                currentData?: ProviderDetailResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
                currentData?: ProviderDetailResponse | undefined;
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
    getProviderSchema: {
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
            currentData?: ProviderSchemaResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
            currentData?: ProviderSchemaResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
            currentData?: ProviderSchemaResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
            currentData?: ProviderSchemaResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
            currentData?: ProviderSchemaResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: {
            providerName: string;
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
                currentData?: ProviderSchemaResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
                currentData?: ProviderSchemaResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
                currentData?: ProviderSchemaResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
                currentData?: ProviderSchemaResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
                currentData?: ProviderSchemaResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">>;
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
            currentData?: ProviderSchemaResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
            currentData?: ProviderSchemaResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
            currentData?: ProviderSchemaResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
            currentData?: ProviderSchemaResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
            currentData?: ProviderSchemaResponse | undefined;
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
                currentData?: ProviderSchemaResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
                currentData?: ProviderSchemaResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
                currentData?: ProviderSchemaResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
                currentData?: ProviderSchemaResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
                currentData?: ProviderSchemaResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: {
            providerName: string;
        }, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">>, [R][R extends any ? 0 : never], {
            lastArg: {
                providerName: string;
            };
        }];
        useQuerySubscription: (arg: {
            providerName: string;
        } | typeof skipToken, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: {
            providerName: string;
        }, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">>, {
            providerName: string;
        } | typeof UNINITIALIZED_VALUE];
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
            currentData?: ProviderSchemaResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
            currentData?: ProviderSchemaResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
            currentData?: ProviderSchemaResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
            currentData?: ProviderSchemaResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
            currentData?: ProviderSchemaResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: {
            providerName: string;
        } | typeof skipToken, options?: {
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
                currentData?: ProviderSchemaResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
                currentData?: ProviderSchemaResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
                currentData?: ProviderSchemaResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
                currentData?: ProviderSchemaResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
                currentData?: ProviderSchemaResponse | undefined;
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
    getProviderModels: {
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
            currentData?: ProviderModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
            currentData?: ProviderModelsResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
            currentData?: ProviderModelsResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
            currentData?: ProviderModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
            currentData?: ProviderModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: {
            providerName: string;
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
                currentData?: ProviderModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
                currentData?: ProviderModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
                currentData?: ProviderModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
                currentData?: ProviderModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
                currentData?: ProviderModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">>;
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
            currentData?: ProviderModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
            currentData?: ProviderModelsResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
            currentData?: ProviderModelsResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
            currentData?: ProviderModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
            currentData?: ProviderModelsResponse | undefined;
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
                currentData?: ProviderModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
                currentData?: ProviderModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
                currentData?: ProviderModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
                currentData?: ProviderModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
                currentData?: ProviderModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: {
            providerName: string;
        }, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">>, [R][R extends any ? 0 : never], {
            lastArg: {
                providerName: string;
            };
        }];
        useQuerySubscription: (arg: {
            providerName: string;
        } | typeof skipToken, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: {
            providerName: string;
        }, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">>, {
            providerName: string;
        } | typeof UNINITIALIZED_VALUE];
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
            currentData?: ProviderModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
            currentData?: ProviderModelsResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
            currentData?: ProviderModelsResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
            currentData?: ProviderModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
            currentData?: ProviderModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: {
            providerName: string;
        } | typeof skipToken, options?: {
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
                currentData?: ProviderModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
                currentData?: ProviderModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
                currentData?: ProviderModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
                currentData?: ProviderModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
                currentData?: ProviderModelsResponse | undefined;
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
    getAvailableModels: {
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
            currentData?: AvailableModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
            currentData?: AvailableModelsResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
            currentData?: AvailableModelsResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
            currentData?: AvailableModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
            currentData?: AvailableModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: {
            providerName: string;
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
                currentData?: AvailableModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
                currentData?: AvailableModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
                currentData?: AvailableModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
                currentData?: AvailableModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
                currentData?: AvailableModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">>;
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
            currentData?: AvailableModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
            currentData?: AvailableModelsResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
            currentData?: AvailableModelsResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
            currentData?: AvailableModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
            currentData?: AvailableModelsResponse | undefined;
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
                currentData?: AvailableModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
                currentData?: AvailableModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
                currentData?: AvailableModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
                currentData?: AvailableModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
                currentData?: AvailableModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: {
            providerName: string;
        }, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">>, [R][R extends any ? 0 : never], {
            lastArg: {
                providerName: string;
            };
        }];
        useQuerySubscription: (arg: {
            providerName: string;
        } | typeof skipToken, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: {
            providerName: string;
        }, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">>, {
            providerName: string;
        } | typeof UNINITIALIZED_VALUE];
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
            currentData?: AvailableModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
            currentData?: AvailableModelsResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
            currentData?: AvailableModelsResponse | undefined;
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
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
            currentData?: AvailableModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
            currentData?: AvailableModelsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: {
            providerName: string;
        } | typeof skipToken, options?: {
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
                currentData?: AvailableModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
                currentData?: AvailableModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
                currentData?: AvailableModelsResponse | undefined;
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
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
                currentData?: AvailableModelsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<{
                providerName: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
                currentData?: AvailableModelsResponse | undefined;
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
    getOpenRouterModelEndpoints: {
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
            currentData?: OpenRouterModelEndpointsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
            currentData?: OpenRouterModelEndpointsResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
            currentData?: OpenRouterModelEndpointsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
            currentData?: OpenRouterModelEndpointsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
            currentData?: OpenRouterModelEndpointsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: (ProviderScopedQueryRequiredArg & {
            modelId: string;
        }) | typeof skipToken, options?: (SubscriptionOptions & {
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
                currentData?: OpenRouterModelEndpointsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
                currentData?: OpenRouterModelEndpointsResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
                currentData?: OpenRouterModelEndpointsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
                currentData?: OpenRouterModelEndpointsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
                currentData?: OpenRouterModelEndpointsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }) | undefined) => [R][R extends any ? 0 : never] & {
            refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">>;
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
            currentData?: OpenRouterModelEndpointsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
            currentData?: OpenRouterModelEndpointsResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
            currentData?: OpenRouterModelEndpointsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
            currentData?: OpenRouterModelEndpointsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
            currentData?: OpenRouterModelEndpointsResponse | undefined;
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
                currentData?: OpenRouterModelEndpointsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
                currentData?: OpenRouterModelEndpointsResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
                currentData?: OpenRouterModelEndpointsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
                currentData?: OpenRouterModelEndpointsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
                currentData?: OpenRouterModelEndpointsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">>, [R][R extends any ? 0 : never], {
            lastArg: ProviderScopedQueryRequiredArg & {
                modelId: string;
            };
        }];
        useQuerySubscription: (arg: (ProviderScopedQueryRequiredArg & {
            modelId: string;
        }) | typeof skipToken, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">>, (ProviderScopedQueryRequiredArg & {
            modelId: string;
        }) | typeof UNINITIALIZED_VALUE];
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
            currentData?: OpenRouterModelEndpointsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
            currentData?: OpenRouterModelEndpointsResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
            currentData?: OpenRouterModelEndpointsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
            currentData?: OpenRouterModelEndpointsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
            currentData?: OpenRouterModelEndpointsResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: (ProviderScopedQueryRequiredArg & {
            modelId: string;
        }) | typeof skipToken, options?: {
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
                currentData?: OpenRouterModelEndpointsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
                currentData?: OpenRouterModelEndpointsResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
                currentData?: OpenRouterModelEndpointsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
                currentData?: OpenRouterModelEndpointsResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
                modelId: string;
            }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
                currentData?: OpenRouterModelEndpointsResponse | undefined;
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
    getOpenRouterAccountInfo: {
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
            currentData?: OpenRouterAccountInfoResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
            currentData?: OpenRouterAccountInfoResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
            currentData?: OpenRouterAccountInfoResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
            currentData?: OpenRouterAccountInfoResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
            currentData?: OpenRouterAccountInfoResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: ProviderScopedQueryArg | typeof skipToken | undefined, options?: (SubscriptionOptions & {
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
                currentData?: OpenRouterAccountInfoResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
                currentData?: OpenRouterAccountInfoResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
                currentData?: OpenRouterAccountInfoResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
                currentData?: OpenRouterAccountInfoResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
                currentData?: OpenRouterAccountInfoResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }) | undefined) => [R][R extends any ? 0 : never] & {
            refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">>;
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
            currentData?: OpenRouterAccountInfoResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
            currentData?: OpenRouterAccountInfoResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
            currentData?: OpenRouterAccountInfoResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
            currentData?: OpenRouterAccountInfoResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
            currentData?: OpenRouterAccountInfoResponse | undefined;
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
                currentData?: OpenRouterAccountInfoResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
                currentData?: OpenRouterAccountInfoResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
                currentData?: OpenRouterAccountInfoResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
                currentData?: OpenRouterAccountInfoResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
                currentData?: OpenRouterAccountInfoResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: ProviderScopedQueryArg | undefined, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">>, [R][R extends any ? 0 : never], {
            lastArg: ProviderScopedQueryArg | undefined;
        }];
        useQuerySubscription: (arg: ProviderScopedQueryArg | typeof skipToken | undefined, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: ProviderScopedQueryArg | undefined, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">>, ProviderScopedQueryArg | typeof UNINITIALIZED_VALUE | undefined];
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
            currentData?: OpenRouterAccountInfoResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
            currentData?: OpenRouterAccountInfoResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
            currentData?: OpenRouterAccountInfoResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
            currentData?: OpenRouterAccountInfoResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
            currentData?: OpenRouterAccountInfoResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: ProviderScopedQueryArg | typeof skipToken | undefined, options?: {
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
                currentData?: OpenRouterAccountInfoResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
                currentData?: OpenRouterAccountInfoResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
                currentData?: OpenRouterAccountInfoResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
                currentData?: OpenRouterAccountInfoResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
                currentData?: OpenRouterAccountInfoResponse | undefined;
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
    getOpenRouterHealth: {
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
            currentData?: OpenRouterHealthResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
            currentData?: OpenRouterHealthResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
            currentData?: OpenRouterHealthResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
            currentData?: OpenRouterHealthResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
            currentData?: OpenRouterHealthResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: ProviderScopedQueryArg | typeof skipToken | undefined, options?: (SubscriptionOptions & {
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
                currentData?: OpenRouterHealthResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
                currentData?: OpenRouterHealthResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
                currentData?: OpenRouterHealthResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
                currentData?: OpenRouterHealthResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
                currentData?: OpenRouterHealthResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }) | undefined) => [R][R extends any ? 0 : never] & {
            refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">>;
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
            currentData?: OpenRouterHealthResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
            currentData?: OpenRouterHealthResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
            currentData?: OpenRouterHealthResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
            currentData?: OpenRouterHealthResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
            currentData?: OpenRouterHealthResponse | undefined;
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
                currentData?: OpenRouterHealthResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
                currentData?: OpenRouterHealthResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
                currentData?: OpenRouterHealthResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
                currentData?: OpenRouterHealthResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
                currentData?: OpenRouterHealthResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: ProviderScopedQueryArg | undefined, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">>, [R][R extends any ? 0 : never], {
            lastArg: ProviderScopedQueryArg | undefined;
        }];
        useQuerySubscription: (arg: ProviderScopedQueryArg | typeof skipToken | undefined, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: ProviderScopedQueryArg | undefined, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">>, ProviderScopedQueryArg | typeof UNINITIALIZED_VALUE | undefined];
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
            currentData?: OpenRouterHealthResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
            currentData?: OpenRouterHealthResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
            currentData?: OpenRouterHealthResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
            currentData?: OpenRouterHealthResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
            currentData?: OpenRouterHealthResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: ProviderScopedQueryArg | typeof skipToken | undefined, options?: {
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
                currentData?: OpenRouterHealthResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
                currentData?: OpenRouterHealthResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
                currentData?: OpenRouterHealthResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
                currentData?: OpenRouterHealthResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
                currentData?: OpenRouterHealthResponse | undefined;
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
    getClaudeCodeUsage: {
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
            currentData?: ClaudeCodeUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
            currentData?: ClaudeCodeUsageResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
            currentData?: ClaudeCodeUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
            currentData?: ClaudeCodeUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
            currentData?: ClaudeCodeUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: ProviderScopedQueryRequiredArg | typeof skipToken, options?: (SubscriptionOptions & {
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
                currentData?: ClaudeCodeUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
                currentData?: ClaudeCodeUsageResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
                currentData?: ClaudeCodeUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
                currentData?: ClaudeCodeUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
                currentData?: ClaudeCodeUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }) | undefined) => [R][R extends any ? 0 : never] & {
            refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">>;
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
            currentData?: ClaudeCodeUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
            currentData?: ClaudeCodeUsageResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
            currentData?: ClaudeCodeUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
            currentData?: ClaudeCodeUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
            currentData?: ClaudeCodeUsageResponse | undefined;
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
                currentData?: ClaudeCodeUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
                currentData?: ClaudeCodeUsageResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
                currentData?: ClaudeCodeUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
                currentData?: ClaudeCodeUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
                currentData?: ClaudeCodeUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: ProviderScopedQueryRequiredArg, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">>, [R][R extends any ? 0 : never], {
            lastArg: ProviderScopedQueryRequiredArg;
        }];
        useQuerySubscription: (arg: ProviderScopedQueryRequiredArg | typeof skipToken, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: ProviderScopedQueryRequiredArg, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">>, ProviderScopedQueryRequiredArg | typeof UNINITIALIZED_VALUE];
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
            currentData?: ClaudeCodeUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
            currentData?: ClaudeCodeUsageResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
            currentData?: ClaudeCodeUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
            currentData?: ClaudeCodeUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
            currentData?: ClaudeCodeUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: ProviderScopedQueryRequiredArg | typeof skipToken, options?: {
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
                currentData?: ClaudeCodeUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
                currentData?: ClaudeCodeUsageResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
                currentData?: ClaudeCodeUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
                currentData?: ClaudeCodeUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
                currentData?: ClaudeCodeUsageResponse | undefined;
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
    getOpenAICodexUsage: {
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
            currentData?: OpenAICodexUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
            currentData?: OpenAICodexUsageResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
            currentData?: OpenAICodexUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
            currentData?: OpenAICodexUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
            currentData?: OpenAICodexUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: ProviderScopedQueryRequiredArg | typeof skipToken, options?: (SubscriptionOptions & {
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
                currentData?: OpenAICodexUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
                currentData?: OpenAICodexUsageResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
                currentData?: OpenAICodexUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
                currentData?: OpenAICodexUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
                currentData?: OpenAICodexUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }) | undefined) => [R][R extends any ? 0 : never] & {
            refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">>;
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
            currentData?: OpenAICodexUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
            currentData?: OpenAICodexUsageResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
            currentData?: OpenAICodexUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
            currentData?: OpenAICodexUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
            currentData?: OpenAICodexUsageResponse | undefined;
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
                currentData?: OpenAICodexUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
                currentData?: OpenAICodexUsageResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
                currentData?: OpenAICodexUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
                currentData?: OpenAICodexUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
                currentData?: OpenAICodexUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: ProviderScopedQueryRequiredArg, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">>, [R][R extends any ? 0 : never], {
            lastArg: ProviderScopedQueryRequiredArg;
        }];
        useQuerySubscription: (arg: ProviderScopedQueryRequiredArg | typeof skipToken, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: ProviderScopedQueryRequiredArg, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">>, ProviderScopedQueryRequiredArg | typeof UNINITIALIZED_VALUE];
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
            currentData?: OpenAICodexUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
            currentData?: OpenAICodexUsageResponse | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
            currentData?: OpenAICodexUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
            currentData?: OpenAICodexUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
            currentData?: OpenAICodexUsageResponse | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "error">>)>> & {
            status: QueryStatus;
        }>(arg: ProviderScopedQueryRequiredArg | typeof skipToken, options?: {
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
                currentData?: OpenAICodexUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
                currentData?: OpenAICodexUsageResponse | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
                currentData?: OpenAICodexUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
                currentData?: OpenAICodexUsageResponse | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
                currentData?: OpenAICodexUsageResponse | undefined;
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
    toggleModel: {
        useMutation: <R extends Record<string, any> = ({
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
                success: boolean;
                model_id: string;
                enabled: boolean;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
                model_id: string;
                enabled: boolean;
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
                success: boolean;
                model_id: string;
                enabled: boolean;
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
                success: boolean;
                model_id: string;
                enabled: boolean;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "error"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
                model_id: string;
                enabled: boolean;
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
                    success: boolean;
                    model_id: string;
                    enabled: boolean;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
                    model_id: string;
                    enabled: boolean;
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
                    success: boolean;
                    model_id: string;
                    enabled: boolean;
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
                    success: boolean;
                    model_id: string;
                    enabled: boolean;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "error"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
                    model_id: string;
                    enabled: boolean;
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
            providerName: string;
            modelId: string;
            enabled: boolean;
        }) => MutationActionCreatorResult<MutationDefinition<{
            providerName: string;
            modelId: string;
            enabled: boolean;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            enabled: boolean;
        }, "providers">>, TSHelpersNoInfer<R> & {
            originalArgs?: {
                providerName: string;
                modelId: string;
                enabled: boolean;
            } | undefined;
            reset: () => void;
        }];
    };
    setModelProvider: {
        useMutation: <R extends Record<string, any> = ({
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
                success: boolean;
                model_id: string;
                selected_provider?: string | null;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
                model_id: string;
                selected_provider?: string | null;
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
                success: boolean;
                model_id: string;
                selected_provider?: string | null;
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
                success: boolean;
                model_id: string;
                selected_provider?: string | null;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "error"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
                model_id: string;
                selected_provider?: string | null;
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
                    success: boolean;
                    model_id: string;
                    selected_provider?: string | null;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
                    model_id: string;
                    selected_provider?: string | null;
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
                    success: boolean;
                    model_id: string;
                    selected_provider?: string | null;
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
                    success: boolean;
                    model_id: string;
                    selected_provider?: string | null;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "error"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
                    model_id: string;
                    selected_provider?: string | null;
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
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }) => MutationActionCreatorResult<MutationDefinition<{
            providerName: string;
            modelId: string;
            selectedProvider?: string | null;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        }, "providers">>, TSHelpersNoInfer<R> & {
            originalArgs?: {
                providerName: string;
                modelId: string;
                selectedProvider?: string | null;
            } | undefined;
            reset: () => void;
        }];
    };
    addCustomModel: {
        useMutation: <R extends Record<string, any> = ({
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
                success: boolean;
                model_id: string;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
                model_id: string;
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
                success: boolean;
                model_id: string;
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
                success: boolean;
                model_id: string;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "error"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
                model_id: string;
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
                    success: boolean;
                    model_id: string;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
                    model_id: string;
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
                    success: boolean;
                    model_id: string;
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
                    success: boolean;
                    model_id: string;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "error"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
                    model_id: string;
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
            providerName: string;
            model: AddCustomModelRequest;
        }) => MutationActionCreatorResult<MutationDefinition<{
            providerName: string;
            model: AddCustomModelRequest;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">>, TSHelpersNoInfer<R> & {
            originalArgs?: {
                providerName: string;
                model: AddCustomModelRequest;
            } | undefined;
            reset: () => void;
        }];
    };
    removeCustomModel: {
        useMutation: <R extends Record<string, any> = ({
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
                success: boolean;
                model_id: string;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
                model_id: string;
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
                success: boolean;
                model_id: string;
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
                success: boolean;
                model_id: string;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "error"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
                model_id: string;
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
                    success: boolean;
                    model_id: string;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
                    model_id: string;
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
                    success: boolean;
                    model_id: string;
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
                    success: boolean;
                    model_id: string;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "error"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
                    model_id: string;
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
            providerName: string;
            modelId: string;
        }) => MutationActionCreatorResult<MutationDefinition<{
            providerName: string;
            modelId: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
            model_id: string;
        }, "providers">>, TSHelpersNoInfer<R> & {
            originalArgs?: {
                providerName: string;
                modelId: string;
            } | undefined;
            reset: () => void;
        }];
    };
    updateProvider: {
        useMutation: <R extends Record<string, any> = ({
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
                success: boolean;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
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
                success: boolean;
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
                success: boolean;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "error"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
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
                    success: boolean;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
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
                    success: boolean;
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
                    success: boolean;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "error"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
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
            providerName: string;
            settings: Record<string, unknown>;
        }) => MutationActionCreatorResult<MutationDefinition<{
            providerName: string;
            settings: Record<string, unknown>;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">>, TSHelpersNoInfer<R> & {
            originalArgs?: {
                providerName: string;
                settings: Record<string, unknown>;
            } | undefined;
            reset: () => void;
        }];
    };
    oauthStart: {
        useMutation: <R extends Record<string, any> = ({
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
            data?: OAuthStartResponse | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
            requestId: string;
            data?: OAuthStartResponse | undefined;
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
            data?: OAuthStartResponse | undefined;
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
            data?: OAuthStartResponse | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "error"> & Required<Pick<{
            requestId: string;
            data?: OAuthStartResponse | undefined;
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
                data?: OAuthStartResponse | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
                requestId: string;
                data?: OAuthStartResponse | undefined;
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
                data?: OAuthStartResponse | undefined;
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
                data?: OAuthStartResponse | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "error"> & Required<Pick<{
                requestId: string;
                data?: OAuthStartResponse | undefined;
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
            providerName: string;
            mode?: string;
        }) => MutationActionCreatorResult<MutationDefinition<{
            providerName: string;
            mode?: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">>, TSHelpersNoInfer<R> & {
            originalArgs?: {
                providerName: string;
                mode?: string;
            } | undefined;
            reset: () => void;
        }];
    };
    oauthExchange: {
        useMutation: <R extends Record<string, any> = ({
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
            data?: OAuthExchangeResponse | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
            requestId: string;
            data?: OAuthExchangeResponse | undefined;
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
            data?: OAuthExchangeResponse | undefined;
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
            data?: OAuthExchangeResponse | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "error"> & Required<Pick<{
            requestId: string;
            data?: OAuthExchangeResponse | undefined;
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
                data?: OAuthExchangeResponse | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
                requestId: string;
                data?: OAuthExchangeResponse | undefined;
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
                data?: OAuthExchangeResponse | undefined;
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
                data?: OAuthExchangeResponse | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "error"> & Required<Pick<{
                requestId: string;
                data?: OAuthExchangeResponse | undefined;
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
            providerName: string;
            session_id: string;
            code: string;
        }) => MutationActionCreatorResult<MutationDefinition<{
            providerName: string;
            session_id: string;
            code: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">>, TSHelpersNoInfer<R> & {
            originalArgs?: {
                providerName: string;
                session_id: string;
                code: string;
            } | undefined;
            reset: () => void;
        }];
    };
    oauthLogout: {
        useMutation: <R extends Record<string, any> = ({
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
                success: boolean;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
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
                success: boolean;
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
                success: boolean;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "error"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
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
                    success: boolean;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
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
                    success: boolean;
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
                    success: boolean;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "error"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
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
            providerName: string;
        }) => MutationActionCreatorResult<MutationDefinition<{
            providerName: string;
        }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">>, TSHelpersNoInfer<R> & {
            originalArgs?: {
                providerName: string;
            } | undefined;
            reset: () => void;
        }];
    };
    deleteProvider: {
        useMutation: <R extends Record<string, any> = ({
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
                success: boolean;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
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
                success: boolean;
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
                success: boolean;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "error"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
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
                    success: boolean;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
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
                    success: boolean;
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
                    success: boolean;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "error"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
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
        } | undefined) => readonly [(arg: string) => MutationActionCreatorResult<MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">>, TSHelpersNoInfer<R> & {
            originalArgs?: string | undefined;
            reset: () => void;
        }];
    };
    getDefaults: {
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
            currentData?: ProviderDefaults | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
            currentData?: ProviderDefaults | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
            currentData?: ProviderDefaults | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
            currentData?: ProviderDefaults | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
            currentData?: ProviderDefaults | undefined;
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
                currentData?: ProviderDefaults | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
                currentData?: ProviderDefaults | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
                currentData?: ProviderDefaults | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
                currentData?: ProviderDefaults | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
                currentData?: ProviderDefaults | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }) | undefined) => [R][R extends any ? 0 : never] & {
            refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">>;
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
            currentData?: ProviderDefaults | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
            currentData?: ProviderDefaults | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
            currentData?: ProviderDefaults | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
            currentData?: ProviderDefaults | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
            currentData?: ProviderDefaults | undefined;
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
                currentData?: ProviderDefaults | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
                currentData?: ProviderDefaults | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
                currentData?: ProviderDefaults | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
                currentData?: ProviderDefaults | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
                currentData?: ProviderDefaults | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "error">>)>> & {
                status: QueryStatus;
            }) => R) | undefined;
        }, "skip">) | undefined) => [(arg: undefined, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">>, [R][R extends any ? 0 : never], {
            lastArg: undefined;
        }];
        useQuerySubscription: (arg: typeof skipToken | undefined, options?: SubscriptionOptions & {
            skip?: boolean;
            refetchOnMountOrArgChange?: boolean | number;
        }) => {
            refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">>;
        };
        useLazyQuerySubscription: (options?: SubscriptionOptions) => readonly [(arg: undefined, preferCacheValue?: boolean) => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">>, typeof UNINITIALIZED_VALUE | undefined];
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
            currentData?: ProviderDefaults | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "isUninitialized"> & {
            isUninitialized: true;
        }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
            currentData?: ProviderDefaults | undefined;
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
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
            currentData?: ProviderDefaults | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp">>) | ({
            isSuccess: true;
            isFetching: false;
            error: undefined;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
            currentData?: ProviderDefaults | undefined;
            isUninitialized: false;
            isLoading: false;
            isFetching: false;
            isSuccess: false;
            isError: false;
        }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
            isError: true;
        } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
            currentData?: ProviderDefaults | undefined;
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
                currentData?: ProviderDefaults | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "isUninitialized"> & {
                isUninitialized: true;
            }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
                currentData?: ProviderDefaults | undefined;
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
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
                currentData?: ProviderDefaults | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp">>) | ({
                isSuccess: true;
                isFetching: false;
                error: undefined;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
                currentData?: ProviderDefaults | undefined;
                isUninitialized: false;
                isLoading: false;
                isFetching: false;
                isSuccess: false;
                isError: false;
            }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
                isError: true;
            } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
                currentData?: ProviderDefaults | undefined;
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
    updateDefaults: {
        useMutation: <R extends Record<string, any> = ({
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
                success: boolean;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
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
                success: boolean;
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
                success: boolean;
            } | undefined;
            error?: FetchBaseQueryError | SerializedError | undefined;
            endpointName: string;
            startedTimeStamp: number;
            fulfilledTimeStamp?: number;
        }, "error"> & Required<Pick<{
            requestId: string;
            data?: {
                success: boolean;
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
                    success: boolean;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
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
                    success: boolean;
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
                    success: boolean;
                } | undefined;
                error?: FetchBaseQueryError | SerializedError | undefined;
                endpointName: string;
                startedTimeStamp: number;
                fulfilledTimeStamp?: number;
            }, "error"> & Required<Pick<{
                requestId: string;
                data?: {
                    success: boolean;
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
        } | undefined) => readonly [(arg: ProviderDefaultsUpdateRequest) => MutationActionCreatorResult<MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
            success: boolean;
        }, "providers">>, TSHelpersNoInfer<R> & {
            originalArgs?: ProviderDefaultsUpdateRequest | undefined;
            reset: () => void;
        }];
    };
};
export declare const useGetConfiguredProvidersQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ConfiguredProvidersResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
    currentData?: ConfiguredProvidersResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
    currentData?: ConfiguredProvidersResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
    currentData?: ConfiguredProvidersResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
    currentData?: ConfiguredProvidersResponse | undefined;
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
        currentData?: ConfiguredProvidersResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
        currentData?: ConfiguredProvidersResponse | undefined;
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
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
        currentData?: ConfiguredProvidersResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp">>) | ({
        isSuccess: true;
        isFetching: false;
        error: undefined;
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
        currentData?: ConfiguredProvidersResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">> & {
        currentData?: ConfiguredProvidersResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ConfiguredProvidersResponse, "providers">>;
}, useGetProviderQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ProviderDetailResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
    currentData?: ProviderDetailResponse | undefined;
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
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
    currentData?: ProviderDetailResponse | undefined;
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
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
    currentData?: ProviderDetailResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
    currentData?: ProviderDetailResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: {
    providerName: string;
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
        currentData?: ProviderDetailResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
        currentData?: ProviderDetailResponse | undefined;
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
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
        currentData?: ProviderDetailResponse | undefined;
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
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
        currentData?: ProviderDetailResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">> & {
        currentData?: ProviderDetailResponse | undefined;
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
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDetailResponse, "providers">>;
}, useGetProviderSchemaQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ProviderSchemaResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
    currentData?: ProviderSchemaResponse | undefined;
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
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
    currentData?: ProviderSchemaResponse | undefined;
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
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
    currentData?: ProviderSchemaResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
    currentData?: ProviderSchemaResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: {
    providerName: string;
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
        currentData?: ProviderSchemaResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
        currentData?: ProviderSchemaResponse | undefined;
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
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
        currentData?: ProviderSchemaResponse | undefined;
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
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
        currentData?: ProviderSchemaResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">> & {
        currentData?: ProviderSchemaResponse | undefined;
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
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderSchemaResponse, "providers">>;
}, useGetProviderModelsQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ProviderModelsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
    currentData?: ProviderModelsResponse | undefined;
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
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
    currentData?: ProviderModelsResponse | undefined;
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
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
    currentData?: ProviderModelsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
    currentData?: ProviderModelsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: {
    providerName: string;
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
        currentData?: ProviderModelsResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
        currentData?: ProviderModelsResponse | undefined;
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
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
        currentData?: ProviderModelsResponse | undefined;
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
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
        currentData?: ProviderModelsResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">> & {
        currentData?: ProviderModelsResponse | undefined;
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
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderModelsResponse, "providers">>;
}, useGetAvailableModelsQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: AvailableModelsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
    currentData?: AvailableModelsResponse | undefined;
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
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
    currentData?: AvailableModelsResponse | undefined;
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
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
    currentData?: AvailableModelsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
    currentData?: AvailableModelsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: {
    providerName: string;
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
        currentData?: AvailableModelsResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
        currentData?: AvailableModelsResponse | undefined;
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
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
        currentData?: AvailableModelsResponse | undefined;
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
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
        currentData?: AvailableModelsResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<{
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">> & {
        currentData?: AvailableModelsResponse | undefined;
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
        providerName: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", AvailableModelsResponse, "providers">>;
}, useGetOpenRouterModelEndpointsQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: OpenRouterModelEndpointsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
    modelId: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
    currentData?: OpenRouterModelEndpointsResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
    modelId: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
    currentData?: OpenRouterModelEndpointsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
    modelId: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
    currentData?: OpenRouterModelEndpointsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
    modelId: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
    currentData?: OpenRouterModelEndpointsResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: (ProviderScopedQueryRequiredArg & {
    modelId: string;
}) | typeof skipToken, options?: (SubscriptionOptions & {
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
        currentData?: OpenRouterModelEndpointsResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
        modelId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
        currentData?: OpenRouterModelEndpointsResponse | undefined;
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
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
        modelId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
        currentData?: OpenRouterModelEndpointsResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp">>) | ({
        isSuccess: true;
        isFetching: false;
        error: undefined;
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
        modelId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
        currentData?: OpenRouterModelEndpointsResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg & {
        modelId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">> & {
        currentData?: OpenRouterModelEndpointsResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg & {
        modelId: string;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterModelEndpointsResponse, "providers">>;
}, useGetOpenRouterAccountInfoQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: OpenRouterAccountInfoResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
    currentData?: OpenRouterAccountInfoResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
    currentData?: OpenRouterAccountInfoResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
    currentData?: OpenRouterAccountInfoResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
    currentData?: OpenRouterAccountInfoResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: ProviderScopedQueryArg | typeof skipToken | undefined, options?: (SubscriptionOptions & {
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
        currentData?: OpenRouterAccountInfoResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
        currentData?: OpenRouterAccountInfoResponse | undefined;
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
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
        currentData?: OpenRouterAccountInfoResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp">>) | ({
        isSuccess: true;
        isFetching: false;
        error: undefined;
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
        currentData?: OpenRouterAccountInfoResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">> & {
        currentData?: OpenRouterAccountInfoResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterAccountInfoResponse, "providers">>;
}, useGetOpenRouterHealthQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: OpenRouterHealthResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
    currentData?: OpenRouterHealthResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
    currentData?: OpenRouterHealthResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
    currentData?: OpenRouterHealthResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
    currentData?: OpenRouterHealthResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: ProviderScopedQueryArg | typeof skipToken | undefined, options?: (SubscriptionOptions & {
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
        currentData?: OpenRouterHealthResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
        currentData?: OpenRouterHealthResponse | undefined;
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
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
        currentData?: OpenRouterHealthResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp">>) | ({
        isSuccess: true;
        isFetching: false;
        error: undefined;
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
        currentData?: OpenRouterHealthResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">> & {
        currentData?: OpenRouterHealthResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryArg | undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenRouterHealthResponse, "providers">>;
}, useGetClaudeCodeUsageQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ClaudeCodeUsageResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
    currentData?: ClaudeCodeUsageResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
    currentData?: ClaudeCodeUsageResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
    currentData?: ClaudeCodeUsageResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
    currentData?: ClaudeCodeUsageResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: ProviderScopedQueryRequiredArg | typeof skipToken, options?: (SubscriptionOptions & {
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
        currentData?: ClaudeCodeUsageResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
        currentData?: ClaudeCodeUsageResponse | undefined;
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
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
        currentData?: ClaudeCodeUsageResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp">>) | ({
        isSuccess: true;
        isFetching: false;
        error: undefined;
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
        currentData?: ClaudeCodeUsageResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">> & {
        currentData?: ClaudeCodeUsageResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ClaudeCodeUsageResponse, "providers">>;
}, useGetOpenAICodexUsageQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: OpenAICodexUsageResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
    currentData?: OpenAICodexUsageResponse | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
    currentData?: OpenAICodexUsageResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
    currentData?: OpenAICodexUsageResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
    currentData?: OpenAICodexUsageResponse | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "error">>)>> & {
    status: QueryStatus;
}>(arg: ProviderScopedQueryRequiredArg | typeof skipToken, options?: (SubscriptionOptions & {
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
        currentData?: OpenAICodexUsageResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
        currentData?: OpenAICodexUsageResponse | undefined;
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
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
        currentData?: OpenAICodexUsageResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp">>) | ({
        isSuccess: true;
        isFetching: false;
        error: undefined;
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
        currentData?: OpenAICodexUsageResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">> & {
        currentData?: OpenAICodexUsageResponse | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<ProviderScopedQueryRequiredArg, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OpenAICodexUsageResponse, "providers">>;
}, useToggleModelMutation: <R extends Record<string, any> = ({
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
        success: boolean;
        model_id: string;
        enabled: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
        model_id: string;
        enabled: boolean;
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
        success: boolean;
        model_id: string;
        enabled: boolean;
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
        success: boolean;
        model_id: string;
        enabled: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
        model_id: string;
        enabled: boolean;
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
            success: boolean;
            model_id: string;
            enabled: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
            model_id: string;
            enabled: boolean;
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
            success: boolean;
            model_id: string;
            enabled: boolean;
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
            success: boolean;
            model_id: string;
            enabled: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
            model_id: string;
            enabled: boolean;
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
    providerName: string;
    modelId: string;
    enabled: boolean;
}) => MutationActionCreatorResult<MutationDefinition<{
    providerName: string;
    modelId: string;
    enabled: boolean;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
    success: boolean;
    model_id: string;
    enabled: boolean;
}, "providers">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        providerName: string;
        modelId: string;
        enabled: boolean;
    } | undefined;
    reset: () => void;
}], useSetModelProviderMutation: <R extends Record<string, any> = ({
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
        success: boolean;
        model_id: string;
        selected_provider?: string | null;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
        model_id: string;
        selected_provider?: string | null;
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
        success: boolean;
        model_id: string;
        selected_provider?: string | null;
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
        success: boolean;
        model_id: string;
        selected_provider?: string | null;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
        model_id: string;
        selected_provider?: string | null;
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
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
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
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
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
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
            model_id: string;
            selected_provider?: string | null;
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
    providerName: string;
    modelId: string;
    selectedProvider?: string | null;
}) => MutationActionCreatorResult<MutationDefinition<{
    providerName: string;
    modelId: string;
    selectedProvider?: string | null;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
    success: boolean;
    model_id: string;
    selected_provider?: string | null;
}, "providers">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        providerName: string;
        modelId: string;
        selectedProvider?: string | null;
    } | undefined;
    reset: () => void;
}], useAddCustomModelMutation: <R extends Record<string, any> = ({
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
        success: boolean;
        model_id: string;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
        model_id: string;
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
        success: boolean;
        model_id: string;
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
        success: boolean;
        model_id: string;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
        model_id: string;
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
            success: boolean;
            model_id: string;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
            model_id: string;
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
            success: boolean;
            model_id: string;
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
            success: boolean;
            model_id: string;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
            model_id: string;
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
    providerName: string;
    model: AddCustomModelRequest;
}) => MutationActionCreatorResult<MutationDefinition<{
    providerName: string;
    model: AddCustomModelRequest;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
    success: boolean;
    model_id: string;
}, "providers">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        providerName: string;
        model: AddCustomModelRequest;
    } | undefined;
    reset: () => void;
}], useRemoveCustomModelMutation: <R extends Record<string, any> = ({
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
        success: boolean;
        model_id: string;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
        model_id: string;
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
        success: boolean;
        model_id: string;
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
        success: boolean;
        model_id: string;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
        model_id: string;
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
            success: boolean;
            model_id: string;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
            model_id: string;
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
            success: boolean;
            model_id: string;
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
            success: boolean;
            model_id: string;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
            model_id: string;
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
    providerName: string;
    modelId: string;
}) => MutationActionCreatorResult<MutationDefinition<{
    providerName: string;
    modelId: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
    success: boolean;
    model_id: string;
}, "providers">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        providerName: string;
        modelId: string;
    } | undefined;
    reset: () => void;
}], useUpdateProviderMutation: <R extends Record<string, any> = ({
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
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
        success: boolean;
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
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
            success: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
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
            success: boolean;
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
            success: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
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
    providerName: string;
    settings: Record<string, unknown>;
}) => MutationActionCreatorResult<MutationDefinition<{
    providerName: string;
    settings: Record<string, unknown>;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
    success: boolean;
}, "providers">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        providerName: string;
        settings: Record<string, unknown>;
    } | undefined;
    reset: () => void;
}], useDeleteProviderMutation: <R extends Record<string, any> = ({
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
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
        success: boolean;
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
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
            success: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
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
            success: boolean;
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
            success: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
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
} | undefined) => readonly [(arg: string) => MutationActionCreatorResult<MutationDefinition<string, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
    success: boolean;
}, "providers">>, TSHelpersNoInfer<R> & {
    originalArgs?: string | undefined;
    reset: () => void;
}], useGetDefaultsQuery: <R extends Record<string, any> = TSHelpersId<(Omit<{
    status: QueryStatus.uninitialized;
    originalArgs?: undefined | undefined;
    data?: undefined | undefined;
    error?: undefined | undefined;
    requestId?: undefined | undefined;
    endpointName?: string | undefined;
    startedTimeStamp?: undefined | undefined;
    fulfilledTimeStamp?: undefined | undefined;
} & {
    currentData?: ProviderDefaults | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "isUninitialized"> & {
    isUninitialized: true;
}) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
    currentData?: ProviderDefaults | undefined;
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
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
    currentData?: ProviderDefaults | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp">>) | ({
    isSuccess: true;
    isFetching: false;
    error: undefined;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
    currentData?: ProviderDefaults | undefined;
    isUninitialized: false;
    isLoading: false;
    isFetching: false;
    isSuccess: false;
    isError: false;
}, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
    isError: true;
} & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
    currentData?: ProviderDefaults | undefined;
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
        currentData?: ProviderDefaults | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "isUninitialized"> & {
        isUninitialized: true;
    }) | TSHelpersOverride<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
        currentData?: ProviderDefaults | undefined;
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
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
        currentData?: ProviderDefaults | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp">>) | ({
        isSuccess: true;
        isFetching: false;
        error: undefined;
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
        currentData?: ProviderDefaults | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "data" | "fulfilledTimeStamp" | "currentData">>) | ({
        isError: true;
    } & Required<Pick<QuerySubState<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">> & {
        currentData?: ProviderDefaults | undefined;
        isUninitialized: false;
        isLoading: false;
        isFetching: false;
        isSuccess: false;
        isError: false;
    }, "error">>)>> & {
        status: QueryStatus;
    }) => R) | undefined;
}) | undefined) => [R][R extends any ? 0 : never] & {
    refetch: () => QueryActionCreatorResult<QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", ProviderDefaults, "providers">>;
}, useUpdateDefaultsMutation: <R extends Record<string, any> = ({
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
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
        success: boolean;
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
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
            success: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
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
            success: boolean;
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
            success: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
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
} | undefined) => readonly [(arg: ProviderDefaultsUpdateRequest) => MutationActionCreatorResult<MutationDefinition<ProviderDefaultsUpdateRequest, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
    success: boolean;
}, "providers">>, TSHelpersNoInfer<R> & {
    originalArgs?: ProviderDefaultsUpdateRequest | undefined;
    reset: () => void;
}], useOauthStartMutation: <R extends Record<string, any> = ({
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
    data?: OAuthStartResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: OAuthStartResponse | undefined;
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
    data?: OAuthStartResponse | undefined;
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
    data?: OAuthStartResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: OAuthStartResponse | undefined;
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
        data?: OAuthStartResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: OAuthStartResponse | undefined;
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
        data?: OAuthStartResponse | undefined;
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
        data?: OAuthStartResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: OAuthStartResponse | undefined;
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
    providerName: string;
    mode?: string;
}) => MutationActionCreatorResult<MutationDefinition<{
    providerName: string;
    mode?: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthStartResponse, "providers">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        providerName: string;
        mode?: string;
    } | undefined;
    reset: () => void;
}], useOauthExchangeMutation: <R extends Record<string, any> = ({
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
    data?: OAuthExchangeResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: OAuthExchangeResponse | undefined;
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
    data?: OAuthExchangeResponse | undefined;
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
    data?: OAuthExchangeResponse | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: OAuthExchangeResponse | undefined;
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
        data?: OAuthExchangeResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: OAuthExchangeResponse | undefined;
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
        data?: OAuthExchangeResponse | undefined;
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
        data?: OAuthExchangeResponse | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: OAuthExchangeResponse | undefined;
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
    providerName: string;
    session_id: string;
    code: string;
}) => MutationActionCreatorResult<MutationDefinition<{
    providerName: string;
    session_id: string;
    code: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", OAuthExchangeResponse, "providers">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        providerName: string;
        session_id: string;
        code: string;
    } | undefined;
    reset: () => void;
}], useOauthLogoutMutation: <R extends Record<string, any> = ({
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
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "data" | "fulfilledTimeStamp"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
        success: boolean;
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
        success: boolean;
    } | undefined;
    error?: FetchBaseQueryError | SerializedError | undefined;
    endpointName: string;
    startedTimeStamp: number;
    fulfilledTimeStamp?: number;
}, "error"> & Required<Pick<{
    requestId: string;
    data?: {
        success: boolean;
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
            success: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "data" | "fulfilledTimeStamp"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
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
            success: boolean;
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
            success: boolean;
        } | undefined;
        error?: FetchBaseQueryError | SerializedError | undefined;
        endpointName: string;
        startedTimeStamp: number;
        fulfilledTimeStamp?: number;
    }, "error"> & Required<Pick<{
        requestId: string;
        data?: {
            success: boolean;
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
    providerName: string;
}) => MutationActionCreatorResult<MutationDefinition<{
    providerName: string;
}, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
    success: boolean;
}, "providers">>, TSHelpersNoInfer<R> & {
    originalArgs?: {
        providerName: string;
    } | undefined;
    reset: () => void;
}];
