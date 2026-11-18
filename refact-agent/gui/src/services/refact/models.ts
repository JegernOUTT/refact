import { RootState } from "../../app/store";
import { MODEL_DEFAULTS_URL, MODEL_URL, MODELS_URL } from "./consts";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { hasProperty } from "../../utils";
import { isDetailMessage } from "./commands";
import { buildApiUrlFromState } from "./apiUrl";
import type { ModelPricing } from "./providers";
import { capsApi } from "./caps";

export const modelsApi = createApi({
  reducerPath: "models",
  tagTypes: ["MODELS", "MODEL"],
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
    getModels: builder.query<ModelsResponse, GetModelsArgs>({
      providesTags: ["MODELS"],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, MODELS_URL);

        const result = await baseQuery({
          ...extraOptions,
          method: "GET",
          url,
          params: {
            "provider-name": args.providerName,
          },
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) {
          return { error: result.error };
        }
        if (!isModelsResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: "Invalid response from /v1/models",
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data };
      },
    }),
    getModel: builder.query<Model, GetModelArgs>({
      providesTags: ["MODEL"],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const { modelName, modelType, providerName } = args;

        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, MODEL_URL);

        const result = await baseQuery({
          ...extraOptions,
          method: "GET",
          url,
          params: {
            provider: providerName,
            model: modelName,
            type: modelType,
          },
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) {
          return { error: result.error };
        }
        if (!isModel(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: "Invalid response from /v1/model",
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data };
      },
    }),
    getModelDefaults: builder.query<Model, GetModelDefaultsArgs>({
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, MODEL_DEFAULTS_URL);

        const result = await baseQuery({
          ...extraOptions,
          method: "GET",
          url,
          params: {
            provider: args.providerName,
            type: args.modelType,
          },
        });

        if (result.error) {
          return { error: result.error };
        }

        if (!isModel(result.data)) {
          return {
            error: {
              error: "Invalid response from /v1/model-defaults",
              status: "CUSTOM_ERROR",
              data: result.data,
            },
          };
        }

        return { data: result.data };
      },
    }),
    updateModel: builder.mutation<unknown, UpdateModelRequestBody>({
      invalidatesTags: (_result, _error, args) => [
        { type: "MODEL", id: args.model.name },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, MODEL_URL);

        const result = await baseQuery({
          ...extraOptions,
          method: "POST",
          url,
          body: { ...args },
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }
        api.dispatch(capsApi.util.invalidateTags(["CAPS"]));

        // TODO: this doesn't really work, RTK Query gets FETCH_ERROR is request is failed and is dropping off actual response from the LSP :/
        if (isDetailMessage(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: "Invalid response from /v1/model",
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data };
      },
    }),
    deleteModel: builder.mutation<unknown, DeleteModelRequestBody>({
      invalidatesTags: (_result, _error, args) => [
        { type: "MODEL", id: args.model },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, MODEL_URL);

        const result = await baseQuery({
          ...extraOptions,
          method: "DELETE",
          url,
          params: { ...args },
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) {
          return { error: result.error };
        }
        api.dispatch(capsApi.util.invalidateTags(["CAPS"]));
        if (isDetailMessage(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: "Invalid response from /v1/model",
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data };
      },
    }),
  }),
  refetchOnMountOrArgChange: true,
});

export type SimplifiedModel = {
  name: string;
  enabled: boolean;
  removable: boolean;
  user_configured: boolean;
};

export type ModelsResponse = {
  chat_models: SimplifiedModel[];
};

export type ModelType = "chat";

export type GetModelArgs = {
  modelName: string;
  providerName: string;
  modelType: ModelType;
};

export type GetModelDefaultsArgs = Omit<GetModelArgs, "modelName">;

export type GetModelsArgs = {
  providerName: string;
};

export type UpdateModelRequestBody = {
  provider: string;
  model: Model;
  type: ModelType;
};

export type DeleteModelRequestBody = Omit<UpdateModelRequestBody, "model"> & {
  model: string;
};

export type CodeChatModel = {
  n_ctx: number;
  name: string;
  tokenizer: string;
  id: string;
  display_name?: string | null;

  supports_tools: boolean;
  supports_multimodality: boolean;
  supports_clicks: boolean;
  supports_agent: boolean;
  reasoning_effort_options?: string[] | null;
  supports_thinking_budget?: boolean;
  supports_adaptive_thinking_budget?: boolean;
  default_temperature: number | null;
  default_frequency_penalty?: number | null;
  default_max_tokens?: number | null;
  max_output_tokens?: number | null;
  pricing?: ModelPricing;

  enabled: boolean;

  type: "chat";
};

export function isModelsResponse(data: unknown): data is ModelsResponse {
  // Check if data is an object
  if (typeof data !== "object" || data === null) return false;

  return hasProperty(data, "chat_models") && Array.isArray(data.chat_models);
}

export type Model = CodeChatModel;

export function isCodeChatModel(data: unknown): data is CodeChatModel {
  if (!data || typeof data !== "object") return false;

  if (!("n_ctx" in data) || typeof data.n_ctx !== "number") return false;
  if (!("name" in data) || typeof data.name !== "string") return false;
  if (!("tokenizer" in data) || typeof data.tokenizer !== "string")
    return false;

  if (!("supports_tools" in data) || typeof data.supports_tools !== "boolean")
    return false;
  if (
    !("supports_multimodality" in data) ||
    typeof data.supports_multimodality !== "boolean"
  )
    return false;
  if (!("supports_clicks" in data) || typeof data.supports_clicks !== "boolean")
    return false;
  if (!("supports_agent" in data) || typeof data.supports_agent !== "boolean")
    return false;

  if (!("default_temperature" in data)) return false;
  if (
    data.default_temperature !== null &&
    typeof data.default_temperature !== "number"
  )
    return false;

  if (!("enabled" in data) || typeof data.enabled !== "boolean") return false;

  return true;
}

export function isModel(data: unknown): data is Model {
  return isCodeChatModel(data);
}
