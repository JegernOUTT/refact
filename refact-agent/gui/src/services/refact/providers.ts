import { RootState } from "../../app/store";
import { hasProperty } from "../../utils";
import { isDetailMessage } from "./commands";
import { PROVIDERS_URL, PROVIDER_DEFAULTS_URL } from "./consts";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

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
  supports_agent: boolean;
  wire_format_override: WireFormat | null;
  endpoint_override: string | null;
  user_configured: boolean;
  removable: boolean;
};

export type ProviderRuntime = {
  name: string;
  display_name: string;
  enabled: boolean;
  readonly: boolean;
  wire_format: WireFormat;
  chat_endpoint: string;
  completion_endpoint: string;
  embedding_endpoint: string;
  support_metadata: boolean;
  chat_models: ProviderModel[];
  completion_models: ProviderModel[];
  embedding_model: ProviderModel | null;
};

export type ProviderStatus = "not_configured" | "configured" | "active";

export type ProviderListItem = {
  name: string;
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
  tokenizer?: string | null;
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
  chat_light: ModelTypeDefaults;
  chat_thinking: ModelTypeDefaults;
  completion_model?: string;
  embedding_model?: string;
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
      queryFn: async (_args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}`;

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
        if (!isProviderListResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: "Invalid response from /v1/providers",
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: { providers: result.data.providers, error_log: [] } };
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}`;

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

        if (!isProviderDetailResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data };
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}/schema`;

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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}/models`;

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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}/available-models`;

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
      { providerName: string; modelId: string }
    >({
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}/models/${encodeURIComponent(args.modelId)}/endpoints`;

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

    getOpenRouterAccountInfo: builder.query<OpenRouterAccountInfoResponse, undefined>({
      queryFn: async (_args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}/v1/openrouter/account-info`;

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

    getOpenRouterHealth: builder.query<OpenRouterHealthResponse, undefined>({
      queryFn: async (_args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}/v1/openrouter/health`;

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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}/models/toggle`;

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
      { providerName: string; modelId: string; selectedProvider?: string | null }
    >({
      invalidatesTags: (_result, _error, { providerName }) => [
        { type: "AVAILABLE_MODELS", id: providerName },
        { type: "PROVIDER", id: providerName },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}/models/provider`;

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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}/custom-models`;

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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}/custom-models/remove`;

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
        { type: "PROVIDERS", id: "LIST" },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}`;

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

        return { data: { success: true } };
      },
    }),

    oauthStart: builder.mutation<
      { session_id: string; authorize_url: string },
      { providerName: string; mode?: string }
    >({
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}/oauth/start`;

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
        const data = result.data as {
          session_id: string;
          authorize_url: string;
        };
        return { data };
      },
    }),

    oauthExchange: builder.mutation<
      { success: boolean; auth_status: string },
      { providerName: string; session_id: string; code: string }
    >({
      invalidatesTags: (_result, _error, { providerName }) => [
        { type: "PROVIDER", id: providerName },
        { type: "PROVIDERS", id: "LIST" },
        { type: "AVAILABLE_MODELS", id: providerName },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}/oauth/exchange`;

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
        const data = result.data as {
          success: boolean;
          auth_status: string;
        };
        return { data };
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${args.providerName}/oauth/logout`;

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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDERS_URL}/${providerName}`;

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

        return { data: { success: true } };
      },
    }),

    getDefaults: builder.query<ProviderDefaults, undefined>({
      providesTags: ["DEFAULTS"],
      queryFn: async (_args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDER_DEFAULTS_URL}`;

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

    updateDefaults: builder.mutation<{ success: boolean }, ProviderDefaults>({
      invalidatesTags: ["DEFAULTS"],
      queryFn: async (defaults, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${PROVIDER_DEFAULTS_URL}`;

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

        return { data: { success: true } };
      },
    }),
  }),
  refetchOnMountOrArgChange: true,
});

function isProviderListResponse(data: unknown): data is ProviderListResponse {
  if (typeof data !== "object" || data === null) return false;
  if (!hasProperty(data, "providers")) return false;
  if (!Array.isArray(data.providers)) return false;

  for (const provider of data.providers) {
    if (!isProviderListItem(provider)) return false;
  }

  return true;
}

function isProviderListItem(data: unknown): data is ProviderListItem {
  if (typeof data !== "object" || data === null) return false;
  if (!hasProperty(data, "name") || typeof data.name !== "string") return false;
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
  // has_credentials and status are optional for backward compat
  return true;
}

function isProviderDetailResponse(
  data: unknown,
): data is ProviderDetailResponse {
  if (typeof data !== "object" || data === null) return false;
  if (!hasProperty(data, "name") || typeof data.name !== "string") return false;
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
  // runtime can be null
  return true;
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

function isModelTypeDefaults(data: unknown): data is ModelTypeDefaults {
  if (typeof data !== "object" || data === null) return false;
  return true;
}

function isProviderDefaults(data: unknown): data is ProviderDefaults {
  if (typeof data !== "object" || data === null) return false;
  const obj = data as Record<string, unknown>;
  if (hasProperty(obj, "chat") && !isModelTypeDefaults(obj.chat)) return false;
  if (hasProperty(obj, "chat_light") && !isModelTypeDefaults(obj.chat_light))
    return false;
  if (
    hasProperty(obj, "chat_thinking") &&
    !isModelTypeDefaults(obj.chat_thinking)
  )
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
