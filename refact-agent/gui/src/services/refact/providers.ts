import { RootState } from "../../app/store";
import { hasProperty } from "../../utils";
import { isDetailMessage } from "./commands";
import { PROVIDERS_URL, PROVIDER_DEFAULTS_URL } from "./consts";
import { buildApiUrlFromState } from "./apiUrl";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { capsApi } from "./caps";

export type WireFormat =
  | "openai_chat_completions"
  | "openai_responses"
  | "anthropic_messages"
  | "refact";

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

// Available models from model discovery (lazy loaded)
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

export type ClaudeCodeScopedUsageWindow = {
  label: string;
  model_id?: string | null;
  window: ClaudeCodeUsageWindow;
};

export type ClaudeCodeExtraUsage = {
  is_enabled: boolean;
  used_credits?: number | null;
  monthly_limit?: number | null;
  utilization?: number | null;
  currency?: string | null;
  disabled_reason?: string | null;
};

export type ClaudeCodeUsageData = {
  five_hour?: ClaudeCodeUsageWindow | null;
  seven_day?: ClaudeCodeUsageWindow | null;
  seven_day_sonnet?: ClaudeCodeUsageWindow | null;
  seven_day_oauth_apps?: ClaudeCodeUsageWindow | null;
  seven_day_opus?: ClaudeCodeUsageWindow | null;
  seven_day_cowork?: ClaudeCodeUsageWindow | null;
  seven_day_omelette?: ClaudeCodeUsageWindow | null;
  scoped_windows?: ClaudeCodeScopedUsageWindow[] | null;
  extra_usage?: ClaudeCodeExtraUsage | null;
  cinder_cove?: unknown;
  iguana_necktie?: unknown;
  omelette_promotional?: unknown;
  tangelo?: unknown;
  raw_extra?: Record<string, unknown>;
};

export type ClaudeCodeUsageResponse = {
  data?: ClaudeCodeUsageData | null;
  error?: string | null;
};

export type OpenAICodexUsageWindow = {
  used_percent: number;
  reset_at?: string | null;
  reset_after_seconds?: number | null;
  limit_window_seconds?: number | null;
};

export type OpenAICodexRateLimit = {
  allowed?: boolean | null;
  limit_reached: boolean;
  primary_window?: OpenAICodexUsageWindow | null;
  secondary_window?: OpenAICodexUsageWindow | null;
};

export type OpenAICodexAdditionalRateLimit = {
  limit_name?: string | null;
  metered_feature?: string | null;
  rate_limit?: OpenAICodexRateLimit | null;
};

export type OpenAICodexResetCredits = {
  available_count?: number | null;
};

export type OpenAICodexCredits = {
  balance: number;
  unlimited: boolean;
  has_credits: boolean;
  granted?: number | null;
  used?: number | null;
  reset_at?: string | null;
  overage_limit_reached?: boolean | null;
  approx_cloud_messages?: number[] | null;
  approx_local_messages?: number[] | null;
};

export type OpenAICodexSpendControl = {
  individual_limit?: number | null;
  reached?: boolean | null;
};

export type OpenAICodexUsageData = {
  account_id?: string | null;
  user_id?: string | null;
  email?: string | null;
  plan_type?: string | null;
  rate_limit?: OpenAICodexRateLimit | null;
  additional_rate_limits?: OpenAICodexAdditionalRateLimit[] | null;
  code_review_rate_limit?: OpenAICodexRateLimit | null;
  rate_limit_reached_type?: string | null;
  rate_limit_reset_credits?: OpenAICodexResetCredits | null;
  credits?: OpenAICodexCredits | null;
  spend_control?: OpenAICodexSpendControl | null;
  promo?: unknown;
  referral_beacon?: unknown;
  raw_extra?: Record<string, unknown>;
};

export type OpenAICodexUsageResponse = {
  data?: OpenAICodexUsageData | null;
  error?: string | null;
};

export type OpenCodeUsageWindow = {
  used_percent: number;
  reset_at?: string | null;
  reset_after_seconds?: number | null;
  limit_window_seconds?: number | null;
  status?: string | null;
};

export type OpenCodeUsageData = {
  plan_type?: string | null;
  workspace_id?: string | null;
  balance?: number | null;
  rolling?: OpenCodeUsageWindow | null;
  weekly?: OpenCodeUsageWindow | null;
  monthly?: OpenCodeUsageWindow | null;
  raw_extra?: Record<string, unknown>;
};

export type OpenCodeUsageResponse = {
  data?: OpenCodeUsageData | null;
  error?: string | null;
};

export type OpenAICodexResetRedeemData = {
  code: string;
  windows_reset?: number | null;
};

export type OpenAICodexResetRedeemResponse = {
  data?: OpenAICodexResetRedeemData | null;
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

export type ProviderIdentitySettings = Pick<
  ProviderDetailResponse,
  "base_provider" | "display_name"
>;

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

function providerBaseName(provider: { name: string; base_provider?: string }) {
  const baseProvider = provider.base_provider?.trim();
  return baseProvider === "" || baseProvider === undefined
    ? provider.name
    : baseProvider;
}

function providerScopedPath(
  singletonPath: string,
  defaultProviderName: string,
  args: ProviderScopedQueryArg | undefined,
  suffix: string,
) {
  if (!args?.useInstanceRoute) return singletonPath;
  const providerName = args.providerName?.trim();
  if (!providerName || providerName === defaultProviderName)
    return singletonPath;
  return `${PROVIDERS_URL}/${encodeURIComponent(providerName)}${suffix}`;
}

export function providerIdentitySettings(
  provider: ProviderIdentitySettings,
): ProviderIdentitySettings {
  return {
    base_provider: provider.base_provider,
    display_name: provider.display_name,
  };
}

export const providersApi = createApi({
  reducerPath: "providers",
  tagTypes: [
    "PROVIDERS",
    "PROVIDER",
    "PROVIDER_SCHEMA",
    "PROVIDER_MODELS",
    "AVAILABLE_MODELS",
    "DEFAULTS",
  ],
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  endpoints: (builder) => ({
    getConfiguredProviders: builder.query<
      ConfiguredProvidersResponse,
      undefined
    >({
      queryFn: async (_arg, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, PROVIDERS_URL);

        const result = await baseQuery({
          ...extraOptions,
          method: "GET",
          url,
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) {
          return { error: result.error };
        }
        if (!isProviderListResponseWire(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: "Invalid response from /v1/providers",
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return {
          data: {
            providers: normalizeProviderListResponse(result.data).providers,
            error_log: [],
          },
        };
      },
      providesTags: [{ type: "PROVIDERS", id: "LIST" }],
    }),

    getProvider: builder.query<
      ProviderDetailResponse,
      { providerName: string }
    >({
      providesTags: (_result, _error, { providerName }) => [
        { type: "PROVIDER", id: providerName },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${args.providerName}`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "GET",
          url,
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        if (!isProviderDetailResponseWire(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return {
          data: normalizeProviderDetailResponse(result.data),
        };
      },
    }),

    getProviderSchema: builder.query<
      ProviderSchemaResponse,
      { providerName: string }
    >({
      providesTags: (_result, _error, { providerName }) => [
        { type: "PROVIDER_SCHEMA", id: providerName },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${args.providerName}/schema`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "GET",
          url,
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        if (!isProviderSchemaResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}/schema`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data };
      },
    }),

    getProviderModels: builder.query<
      ProviderModelsResponse,
      { providerName: string }
    >({
      providesTags: (_result, _error, { providerName }) => [
        { type: "PROVIDER_MODELS", id: providerName },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${args.providerName}/models`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "GET",
          url,
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        if (!isProviderModelsResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}/models`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data };
      },
    }),

    // Get all available models for a provider (discovered + custom)
    getAvailableModels: builder.query<
      AvailableModelsResponse,
      { providerName: string }
    >({
      providesTags: (_result, _error, { providerName }) => [
        { type: "AVAILABLE_MODELS", id: providerName },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${args.providerName}/available-models`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "GET",
          url,
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        if (!isAvailableModelsResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}/available-models`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data };
      },
    }),

    getOpenRouterModelEndpoints: builder.query<
      OpenRouterModelEndpointsResponse,
      ProviderScopedQueryRequiredArg & { modelId: string }
    >({
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const providerName =
          args.useInstanceRoute === true ? args.providerName : "openrouter";
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${providerName}/models/${encodeURIComponent(
            args.modelId,
          )}/endpoints`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "GET",
          url,
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        if (!isOpenRouterModelEndpointsResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}/models/${args.modelId}/endpoints`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data };
      },
    }),

    getOpenRouterAccountInfo: builder.query<
      OpenRouterAccountInfoResponse,
      ProviderScopedQueryArg | undefined
    >({
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const path = providerScopedPath(
          "/v1/openrouter/account-info",
          "openrouter",
          args,
          "/account-info",
        );
        const url = buildApiUrlFromState(state, path);

        const result = await baseQuery({
          ...extraOptions,
          method: "GET",
          url,
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        return { data: result.data as OpenRouterAccountInfoResponse };
      },
    }),

    getOpenRouterHealth: builder.query<
      OpenRouterHealthResponse,
      ProviderScopedQueryArg | undefined
    >({
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const path = providerScopedPath(
          "/v1/openrouter/health",
          "openrouter",
          args,
          "/health",
        );
        const url = buildApiUrlFromState(state, path);

        const result = await baseQuery({
          ...extraOptions,
          method: "GET",
          url,
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        return { data: result.data as OpenRouterHealthResponse };
      },
    }),

    getClaudeCodeUsage: builder.query<
      ClaudeCodeUsageResponse,
      ProviderScopedQueryRequiredArg
    >({
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${encodeURIComponent(args.providerName)}/usage`,
        );

        const controller = new AbortController();
        const timeoutId = setTimeout(() => controller.abort(), 10_000);
        let result: Awaited<ReturnType<typeof baseQuery>>;
        try {
          result = await baseQuery({
            ...extraOptions,
            method: "GET",
            url,
            credentials: "same-origin",
            redirect: "follow",
            signal: controller.signal,
          });
        } finally {
          clearTimeout(timeoutId);
        }

        if (result.error) {
          return { error: result.error };
        }

        if (!isUsageResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}/usage`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data };
      },
    }),

    getOpenAICodexUsage: builder.query<
      OpenAICodexUsageResponse,
      ProviderScopedQueryRequiredArg
    >({
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${encodeURIComponent(args.providerName)}/usage`,
        );

        const controller = new AbortController();
        const timeoutId = setTimeout(() => controller.abort(), 10_000);
        let result: Awaited<ReturnType<typeof baseQuery>>;
        try {
          result = await baseQuery({
            ...extraOptions,
            method: "GET",
            url,
            credentials: "same-origin",
            redirect: "follow",
            signal: controller.signal,
          });
        } finally {
          clearTimeout(timeoutId);
        }

        if (result.error) {
          return { error: result.error };
        }

        if (!isUsageResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}/usage`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data as OpenAICodexUsageResponse };
      },
    }),

    getOpenCodeUsage: builder.query<
      OpenCodeUsageResponse,
      ProviderScopedQueryRequiredArg
    >({
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${encodeURIComponent(args.providerName)}/usage`,
        );

        const controller = new AbortController();
        const timeoutId = setTimeout(() => controller.abort(), 10_000);
        let result: Awaited<ReturnType<typeof baseQuery>>;
        try {
          result = await baseQuery({
            ...extraOptions,
            method: "GET",
            url,
            credentials: "same-origin",
            redirect: "follow",
            signal: controller.signal,
          });
        } finally {
          clearTimeout(timeoutId);
        }

        if (result.error) {
          return { error: result.error };
        }

        if (!isUsageResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}/usage`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data as OpenCodeUsageResponse };
      },
    }),

    redeemOpenAICodexResetCredit: builder.mutation<
      OpenAICodexResetRedeemResponse,
      ProviderScopedQueryRequiredArg & { redeemRequestId: string }
    >({
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${encodeURIComponent(
            args.providerName,
          )}/usage/redeem`,
        );

        const controller = new AbortController();
        const timeoutId = setTimeout(() => controller.abort(), 15_000);
        let result: Awaited<ReturnType<typeof baseQuery>>;
        try {
          result = await baseQuery({
            ...extraOptions,
            method: "POST",
            url,
            body: { redeem_request_id: args.redeemRequestId },
            credentials: "same-origin",
            redirect: "follow",
            signal: controller.signal,
          });
        } finally {
          clearTimeout(timeoutId);
        }

        if (result.error) {
          return { error: result.error };
        }

        if (!isUsageResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}/usage/redeem`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data as OpenAICodexResetRedeemResponse };
      },
    }),

    // Toggle model enabled/disabled
    toggleModel: builder.mutation<
      { success: boolean; model_id: string; enabled: boolean },
      { providerName: string; modelId: string; enabled: boolean }
    >({
      invalidatesTags: (_result, _error, { providerName }) => [
        { type: "AVAILABLE_MODELS", id: providerName },
        { type: "PROVIDER", id: providerName },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${args.providerName}/models/toggle`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "POST",
          url,
          body: { model_id: args.modelId, enabled: args.enabled },
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        const data = result.data as
          | { success?: boolean; detail?: string }
          | undefined;
        if (data?.success === false) {
          return {
            meta: result.meta,
            error: {
              error: data.detail ?? "Failed to toggle model",
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        api.dispatch(capsApi.util.resetApiState());

        return {
          data: {
            success: true,
            model_id: args.modelId,
            enabled: args.enabled,
          },
        };
      },
    }),

    setModelProvider: builder.mutation<
      {
        success: boolean;
        model_id: string;
        selected_provider?: string | null;
      },
      {
        providerName: string;
        modelId: string;
        selectedProvider?: string | null;
      }
    >({
      invalidatesTags: (_result, _error, { providerName }) => [
        { type: "AVAILABLE_MODELS", id: providerName },
        { type: "PROVIDER", id: providerName },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${args.providerName}/models/provider`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "POST",
          url,
          body: {
            model_id: args.modelId,
            selected_provider: args.selectedProvider ?? null,
          },
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        api.dispatch(capsApi.util.resetApiState());

        return {
          data: {
            success: true,
            model_id: args.modelId,
            selected_provider: args.selectedProvider ?? null,
          },
        };
      },
    }),

    // Add custom model
    addCustomModel: builder.mutation<
      { success: boolean; model_id: string },
      { providerName: string; model: AddCustomModelRequest }
    >({
      invalidatesTags: (_result, _error, { providerName }) => [
        { type: "AVAILABLE_MODELS", id: providerName },
        { type: "PROVIDER", id: providerName },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${args.providerName}/custom-models`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "POST",
          url,
          body: args.model,
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        const data = result.data as
          | { success?: boolean; detail?: string }
          | undefined;
        if (data?.success === false) {
          return {
            meta: result.meta,
            error: {
              error: data.detail ?? "Failed to add custom model",
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        api.dispatch(capsApi.util.resetApiState());

        return { data: { success: true, model_id: args.model.id } };
      },
    }),

    // Remove custom model
    removeCustomModel: builder.mutation<
      { success: boolean; model_id: string },
      { providerName: string; modelId: string }
    >({
      invalidatesTags: (_result, _error, { providerName }) => [
        { type: "AVAILABLE_MODELS", id: providerName },
        { type: "PROVIDER", id: providerName },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${args.providerName}/custom-models/remove`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "POST",
          url,
          body: { model_id: args.modelId },
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        const data = result.data as
          | { success?: boolean; detail?: string }
          | undefined;
        if (data?.success === false) {
          return {
            meta: result.meta,
            error: {
              error: data.detail ?? "Failed to remove custom model",
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        api.dispatch(capsApi.util.resetApiState());

        return { data: { success: true, model_id: args.modelId } };
      },
    }),

    updateProvider: builder.mutation<
      { success: boolean },
      { providerName: string; settings: Record<string, unknown> }
    >({
      invalidatesTags: (_result, _error, { providerName }) => [
        { type: "PROVIDER", id: providerName },
        { type: "PROVIDER_MODELS", id: providerName },
        { type: "AVAILABLE_MODELS", id: providerName },
        { type: "PROVIDERS", id: "LIST" },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${args.providerName}`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "POST",
          url,
          body: args.settings,
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) {
          return { error: result.error };
        }
        if (isDetailMessage(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Failed to update provider ${args.providerName}`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        api.dispatch(capsApi.util.resetApiState());

        return { data: { success: true } };
      },
    }),

    oauthStart: builder.mutation<
      OAuthStartResponse,
      { providerName: string; mode?: string }
    >({
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${args.providerName}/oauth/start`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "POST",
          url,
          body: { mode: args.mode ?? "max" },
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) {
          return { error: result.error };
        }
        if (!isOAuthStartResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}/oauth/start`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }
        return { data: result.data };
      },
    }),

    oauthExchange: builder.mutation<
      OAuthExchangeResponse,
      { providerName: string; session_id: string; code: string }
    >({
      invalidatesTags: (result, _error, { providerName }) =>
        result?.success
          ? [
              { type: "PROVIDER", id: providerName },
              { type: "PROVIDERS", id: "LIST" },
              { type: "AVAILABLE_MODELS", id: providerName },
            ]
          : [],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${args.providerName}/oauth/exchange`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "POST",
          url,
          body: { session_id: args.session_id, code: args.code },
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) {
          return { error: result.error };
        }
        if (!isOAuthExchangeResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}/oauth/exchange`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }
        if (result.data.success) {
          api.dispatch(capsApi.util.resetApiState());
        }
        return { data: result.data };
      },
    }),

    oauthLogout: builder.mutation<
      { success: boolean },
      { providerName: string }
    >({
      invalidatesTags: (_result, _error, { providerName }) => [
        { type: "PROVIDER", id: providerName },
        { type: "PROVIDERS", id: "LIST" },
        { type: "AVAILABLE_MODELS", id: providerName },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${args.providerName}/oauth/logout`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "POST",
          url,
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) {
          return { error: result.error };
        }
        api.dispatch(capsApi.util.resetApiState());
        return { data: { success: true } };
      },
    }),

    deleteProvider: builder.mutation<{ success: boolean }, string>({
      invalidatesTags: (_result, _error, providerName) => [
        { type: "PROVIDER", id: providerName },
        { type: "PROVIDER_MODELS", id: providerName },
        { type: "PROVIDERS", id: "LIST" },
      ],
      queryFn: async (providerName, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${providerName}`,
        );

        const result = await baseQuery({
          ...extraOptions,
          method: "DELETE",
          url,
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) {
          return { error: result.error };
        }
        if (isDetailMessage(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Failed to delete provider ${providerName}`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        api.dispatch(capsApi.util.resetApiState());

        return { data: { success: true } };
      },
    }),

    getDefaults: builder.query<ProviderDefaults, undefined>({
      providesTags: ["DEFAULTS"],
      queryFn: async (_args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, PROVIDER_DEFAULTS_URL);

        const result = await baseQuery({
          ...extraOptions,
          method: "GET",
          url,
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        if (!isProviderDefaults(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: "Invalid response from /v1/defaults",
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data };
      },
    }),

    updateDefaults: builder.mutation<
      { success: boolean },
      ProviderDefaultsUpdateRequest
    >({
      invalidatesTags: ["DEFAULTS"],
      queryFn: async (defaults, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, PROVIDER_DEFAULTS_URL);

        const result = await baseQuery({
          ...extraOptions,
          method: "POST",
          url,
          body: defaults,
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        api.dispatch(capsApi.util.resetApiState());

        return { data: { success: true } };
      },
    }),
  }),
  refetchOnMountOrArgChange: true,
});

type ProviderListItemWire = Omit<ProviderListItem, "base_provider"> & {
  base_provider?: string;
};

type ProviderListResponseWire = {
  providers: ProviderListItemWire[];
};

type ProviderRuntimeWire = Omit<ProviderRuntime, "base_provider"> & {
  base_provider?: string;
};

type ProviderDetailResponseWire = Omit<
  ProviderDetailResponse,
  "base_provider" | "runtime"
> & {
  base_provider?: string;
  runtime?: ProviderRuntimeWire | null;
};

function isProviderListResponseWire(
  data: unknown,
): data is ProviderListResponseWire {
  if (typeof data !== "object" || data === null) return false;
  if (!hasProperty(data, "providers")) return false;
  if (!Array.isArray(data.providers)) return false;

  for (const provider of data.providers) {
    if (!isProviderListItem(provider)) return false;
  }

  return true;
}

function isOptionalBaseProviderField(data: object): boolean {
  return (
    !hasProperty(data, "base_provider") ||
    typeof data.base_provider === "string"
  );
}

function isProviderListItem(data: unknown): data is ProviderListItemWire {
  if (typeof data !== "object" || data === null) return false;
  if (!hasProperty(data, "name") || typeof data.name !== "string") return false;
  if (!isOptionalBaseProviderField(data)) return false;
  if (
    !hasProperty(data, "display_name") ||
    typeof data.display_name !== "string"
  )
    return false;
  if (!hasProperty(data, "enabled") || typeof data.enabled !== "boolean")
    return false;
  if (!hasProperty(data, "readonly") || typeof data.readonly !== "boolean")
    return false;
  if (!hasProperty(data, "model_count") || typeof data.model_count !== "number")
    return false;
  if (!hasProperty(data, "has_credentials")) return false;
  if (typeof data.has_credentials !== "boolean") return false;
  if (!hasProperty(data, "status")) return false;
  if (
    data.status !== "not_configured" &&
    data.status !== "configured" &&
    data.status !== "active"
  )
    return false;
  return true;
}

function normalizeProviderListItem(
  provider: ProviderListItemWire,
): ProviderListItem {
  return {
    ...provider,
    base_provider: providerBaseName(provider),
  };
}

function normalizeProviderListResponse(
  response: ProviderListResponseWire,
): ProviderListResponse {
  return {
    providers: response.providers.map(normalizeProviderListItem),
  };
}

function isProviderDetailResponseWire(
  data: unknown,
): data is ProviderDetailResponseWire {
  if (typeof data !== "object" || data === null) return false;
  if (!hasProperty(data, "name") || typeof data.name !== "string") return false;
  if (!isOptionalBaseProviderField(data)) return false;
  if (
    !hasProperty(data, "display_name") ||
    typeof data.display_name !== "string"
  )
    return false;
  if (!hasProperty(data, "enabled") || typeof data.enabled !== "boolean")
    return false;
  if (!hasProperty(data, "readonly") || typeof data.readonly !== "boolean")
    return false;
  if (!hasProperty(data, "settings")) return false;
  if (hasProperty(data, "runtime") && !isProviderRuntime(data.runtime)) {
    return false;
  }
  return true;
}

function isProviderRuntime(data: unknown): data is ProviderRuntimeWire | null {
  if (data === null || data === undefined) return true;
  if (typeof data !== "object") return false;
  if (!hasProperty(data, "name") || typeof data.name !== "string") return false;
  if (!isOptionalBaseProviderField(data)) return false;
  if (
    !hasProperty(data, "display_name") ||
    typeof data.display_name !== "string"
  )
    return false;
  return true;
}

function normalizeProviderRuntime(
  runtime: ProviderDetailResponseWire["runtime"],
  provider: Pick<ProviderDetailResponse, "base_provider">,
): ProviderRuntime | null {
  if (!runtime) return null;
  return {
    ...runtime,
    base_provider: providerBaseName({
      name: runtime.name,
      base_provider: runtime.base_provider ?? provider.base_provider,
    }),
  };
}

function normalizeProviderDetailResponse(
  provider: ProviderDetailResponseWire,
): ProviderDetailResponse {
  const base_provider = providerBaseName(provider);
  return {
    ...provider,
    base_provider,
    runtime: normalizeProviderRuntime(provider.runtime, { base_provider }),
  };
}

export function isProviderListResponse(
  data: unknown,
): data is ProviderListResponse {
  if (!isProviderListResponseWire(data)) return false;
  return data.providers.every((provider) => Boolean(provider.base_provider));
}

export function isProviderDetailResponse(
  data: unknown,
): data is ProviderDetailResponse {
  return isProviderDetailResponseWire(data) && Boolean(data.base_provider);
}

function isProviderSchemaResponse(
  data: unknown,
): data is ProviderSchemaResponse {
  if (typeof data !== "object" || data === null) return false;
  if (!hasProperty(data, "name") || typeof data.name !== "string") return false;
  if (!hasProperty(data, "schema") || typeof data.schema !== "string")
    return false;
  return true;
}

function isProviderModelsResponse(
  data: unknown,
): data is ProviderModelsResponse {
  if (typeof data !== "object" || data === null) return false;
  if (!hasProperty(data, "models")) return false;
  if (!Array.isArray(data.models)) return false;
  return true;
}

function isAvailableModelsResponse(
  data: unknown,
): data is AvailableModelsResponse {
  if (typeof data !== "object" || data === null) return false;
  if (!hasProperty(data, "models")) return false;
  if (!Array.isArray(data.models)) return false;
  if (!hasProperty(data, "source")) return false;
  return true;
}

function isOpenRouterModelEndpointsResponse(
  data: unknown,
): data is OpenRouterModelEndpointsResponse {
  if (typeof data !== "object" || data === null) return false;
  if (!hasProperty(data, "provider_variants")) return false;
  if (!hasProperty(data, "available_providers")) return false;
  if (!Array.isArray(data.available_providers)) return false;
  return true;
}

function isUsageResponse(data: unknown): data is ClaudeCodeUsageResponse {
  if (typeof data !== "object" || data === null) return false;
  // Must have at least one of `data` or `error` key
  return hasProperty(data, "data") || hasProperty(data, "error");
}

function isOAuthStartMode(data: unknown): data is OAuthStartMode {
  return data === "callback" || data === "manual_code" || data === "device";
}

function isOptionalStringField(data: object, key: string): boolean {
  return !hasProperty(data, key) || typeof data[key] === "string";
}

function isOptionalNumberField(data: object, key: string): boolean {
  return (
    !hasProperty(data, key) ||
    (typeof data[key] === "number" && Number.isFinite(data[key]))
  );
}

function isOAuthStartResponse(data: unknown): data is OAuthStartResponse {
  if (typeof data !== "object" || data === null) return false;
  if (!hasProperty(data, "session_id") || typeof data.session_id !== "string")
    return false;
  if (
    !hasProperty(data, "authorize_url") ||
    typeof data.authorize_url !== "string"
  )
    return false;
  if (!isOptionalStringField(data, "user_code")) return false;
  if (!isOptionalStringField(data, "instructions")) return false;
  if (!isOptionalNumberField(data, "poll_interval")) return false;
  if (hasProperty(data, "mode") && !isOAuthStartMode(data.mode)) return false;
  return true;
}

function isOAuthExchangeResponse(data: unknown): data is OAuthExchangeResponse {
  if (typeof data !== "object" || data === null) return false;
  if (!hasProperty(data, "success") || typeof data.success !== "boolean")
    return false;
  if (!hasProperty(data, "auth_status") || typeof data.auth_status !== "string")
    return false;
  if (!isOptionalStringField(data, "status")) return false;
  if (!isOptionalNumberField(data, "poll_interval")) return false;
  return true;
}

function isModelTypeDefaults(data: unknown): data is ModelTypeDefaults {
  if (typeof data !== "object" || data === null) return false;
  return true;
}

function isProviderDefaults(data: unknown): data is ProviderDefaults {
  if (typeof data !== "object" || data === null) return false;
  const obj = data as Record<string, unknown>;
  if (hasProperty(obj, "chat") && !isModelTypeDefaults(obj.chat)) return false;
  if (
    hasProperty(obj, "chat_model_2") &&
    !isModelTypeDefaults(obj.chat_model_2)
  )
    return false;
  if (
    hasProperty(obj, "task_planner_agent_model") &&
    !isModelTypeDefaults(obj.task_planner_agent_model)
  )
    return false;
  if (hasProperty(obj, "chat_light") && !isModelTypeDefaults(obj.chat_light))
    return false;
  if (
    hasProperty(obj, "chat_thinking") &&
    !isModelTypeDefaults(obj.chat_thinking)
  )
    return false;
  if (hasProperty(obj, "chat_buddy") && !isModelTypeDefaults(obj.chat_buddy))
    return false;
  if (hasProperty(obj, "detail")) return false;
  return true;
}

export const providersEndpoints = providersApi.endpoints;

export const {
  useGetConfiguredProvidersQuery,
  useGetProviderQuery,
  useGetProviderSchemaQuery,
  useGetProviderModelsQuery,
  useGetAvailableModelsQuery,
  useGetOpenRouterModelEndpointsQuery,
  useGetOpenRouterAccountInfoQuery,
  useGetOpenRouterHealthQuery,
  useGetClaudeCodeUsageQuery,
  useGetOpenAICodexUsageQuery,
  useGetOpenCodeUsageQuery,
  useRedeemOpenAICodexResetCreditMutation,
  useToggleModelMutation,
  useSetModelProviderMutation,
  useAddCustomModelMutation,
  useRemoveCustomModelMutation,
  useUpdateProviderMutation,
  useDeleteProviderMutation,
  useGetDefaultsQuery,
  useUpdateDefaultsMutation,
  useOauthStartMutation,
  useOauthExchangeMutation,
  useOauthLogoutMutation,
} = providersApi;
