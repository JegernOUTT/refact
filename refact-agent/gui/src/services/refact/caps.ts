import { RootState } from "../../app/store";
import { CAPS_URL } from "./consts";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import {
  CodeChatModel,
  CodeCompletionModel,
  EmbeddingModel,
  isCodeChatModel,
  isCodeCompletionModel,
  isEmbeddingModel,
} from "./models";
import { buildApiUrlFromState } from "./apiUrl";

export const capsApi = createApi({
  reducerPath: "caps",
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
    getCaps: builder.query<CapsResponse, undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, CAPS_URL);

        const result = await baseQuery({
          url,
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) {
          return { error: result.error };
        }
        if (!isCapsResponse(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: "Invalid response from caps",
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

export const capsEndpoints = capsApi.endpoints;

// Export the generated RTK Query hook
export const { useGetCapsQuery } = capsApi;

export type CapCost = {
  prompt: number;
  generated: number;
  cache_read?: number;
  cache_creation?: number;
};

function isCapCost(json: unknown): json is CapCost {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (!("prompt" in json)) return false;
  if (typeof json.prompt !== "number") return false;
  if (!("generated" in json)) return false;
  if (typeof json.generated !== "number") return false;
  return true;
}
type CapsMetadata = {
  pricing?: Record<string, CapCost>;
  features?: string[];
};

function isCapsMetadata(json: unknown): json is CapsMetadata {
  if (json === null) return true;
  if (typeof json !== "object") return false;
  if ("pricing" in json && json.pricing) {
    return Object.values(json.pricing).every(isCapCost);
  }
  return true;
}

export type CapsResponse = {
  caps_version: number;
  chat_default_model: string;
  chat_models: Record<string, CodeChatModel>;
  code_chat_default_system_prompt: string;
  completion_models: Record<string, CodeCompletionModel>;
  completion_default_model: string;
  code_completion_n_ctx: number;
  // The engine wire caps expose a single authoritative embedding model
  // (VecDB indexes one embedding dimension at a time). `embedding_models`
  // (plural) is accepted for forward/back-compat but is NOT populated by the
  // current backend; consumers should treat `embedding_model` as the source of
  // truth and only fall back to the plural map when present.
  embedding_model?: EmbeddingModel;
  embedding_models?: Record<string, EmbeddingModel>;
  chat_model_2: string;
  task_planner_agent_model: string;
  chat_thinking_model: string;
  chat_light_model: string;
  chat_buddy_model: string;

  endpoint_chat_passthrough: string;
  endpoint_style: string;
  endpoint_template: string;
  running_models: string[];
  tokenizer_path_template: string;
  tokenizer_rewrite_path: Record<string, unknown>;
  metadata: CapsMetadata | null;
  customization: string;
};

export function isCapsResponse(json: unknown): json is CapsResponse {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (!("caps_version" in json) || typeof json.caps_version !== "number")
    return false;
  if (!("metadata" in json)) return false;
  if (!isCapsMetadata(json.metadata)) return false;
  if (!("chat_default_model" in json)) return false;
  if (typeof json.chat_default_model !== "string") return false;
  if (!("completion_default_model" in json)) return false;
  if (typeof json.completion_default_model !== "string") return false;
  if (!("completion_models" in json)) return false;
  if (!isModelRecord(json.completion_models, isCodeCompletionModel))
    return false;
  if (!isOptionalStringField(json, "chat_model_2")) return false;
  if (!isOptionalStringField(json, "task_planner_agent_model")) return false;
  if (!("chat_models" in json)) return false;
  if (!isModelRecord(json.chat_models, isCodeChatModel)) return false;
  if ("embedding_model" in json && !isEmbeddingModel(json.embedding_model))
    return false;
  if (
    "embedding_models" in json &&
    !isModelRecord(json.embedding_models, isEmbeddingModel)
  )
    return false;
  if (!("embedding_model" in json) && !("embedding_models" in json))
    return false;
  return true;
}

function isModelRecord<T>(
  value: unknown,
  modelGuard: (model: unknown) => model is T,
): value is Record<string, T> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  return Object.values(value).every(modelGuard);
}

function isOptionalStringField(
  json: Record<string, unknown>,
  field: string,
): boolean {
  return !(field in json) || typeof json[field] === "string";
}

type CapsErrorResponse = {
  detail: string;
};

export function isCapsErrorResponse(json: unknown): json is CapsErrorResponse {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (!("detail" in json)) return false;
  if (typeof json.detail !== "string") return false;
  return true;
}
