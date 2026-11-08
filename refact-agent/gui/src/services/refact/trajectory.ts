import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import {
  TRAJECTORY_TRANSFORM_PREVIEW_URL,
  TRAJECTORY_TRANSFORM_APPLY_URL,
  TRAJECTORY_LLM_COMPRESS_PREVIEW_URL,
  TRAJECTORY_LLM_COMPRESS_APPLY_URL,
  TRAJECTORY_MODE_TRANSITION_APPLY_URL,
} from "./consts";
import { buildApiUrlFromState, getEngineEndpointIdentity } from "./apiUrl";

export type TransformOptions = {
  dedup_and_compress_context?: boolean;
  drop_all_context?: boolean;
  compress_non_agentic_tools?: boolean;
  drop_all_memories?: boolean;
  drop_project_information?: boolean;
  strip_metering?: boolean;
};

export type LlmCompressOptions = {
  summary_model?: string;
  expected_trajectory_version?: number;
};

export type TransformStats = {
  before_message_count: number;
  after_message_count: number;
  before_approx_tokens: number;
  after_approx_tokens: number;
  context_messages_modified: number;
  tool_messages_modified: number;
};

export type TransformPreviewResponse = {
  stats: TransformStats;
  actions: string[];
};

export type TransformApplyResponse = {
  stats: TransformStats;
};

export type RefreshToolsResponse = {
  chat_id: string;
  refreshed: true;
  effective_next_turn: true;
};

export type LlmCompressPreviewResponse = {
  eligible: boolean;
  trajectory_version?: number | null;
  resolved_model?: string | null;
  context_window?: number | null;
  source_messages: number;
  approximate_source_tokens: number;
  reason?: string | null;
};

export type LlmCompressApplyResponse = {
  applied: boolean;
  resolved_model?: string;
  context_window?: number;
  source_messages: number;
  approximate_source_tokens: number;
  stats: TransformStats;
  reason?: string;
};

export type ModeTransitionApplyResponse = {
  new_chat_id: string;
  messages_count: number;
  root_chat_id?: string | null;
  initial_plan_document?: string | null;
  initial_plan_error?: string | null;
};

function buildPath(template: string, chatId: string): string {
  return template.replace("{chat_id}", encodeURIComponent(chatId));
}

export const trajectoryApi = createApi({
  reducerPath: "trajectoryApi",
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
    refreshTools: builder.mutation<
      RefreshToolsResponse,
      { chatId: string; expectedVersion?: number }
    >({
      async queryFn({ chatId, expectedVersion }, api, _opts, baseQuery) {
        const start = api.getState() as RootState;
        const identity = getEngineEndpointIdentity(start.config);
        const result = await baseQuery({
          url: buildApiUrlFromState(start, "/v1/trajectory/refresh-tools"),
          method: "POST",
          body: { chat_id: chatId, expected_version: expectedVersion },
        });
        if (result.error) return { error: result.error };
        const current = api.getState() as RootState;
        const data = result.data;
        if (
          identity !== getEngineEndpointIdentity(current.config) ||
          start.config.apiKey !== current.config.apiKey ||
          typeof data !== "object" ||
          data === null ||
          !("chat_id" in data) ||
          data.chat_id !== chatId ||
          !("refreshed" in data) ||
          data.refreshed !== true ||
          !("effective_next_turn" in data) ||
          data.effective_next_turn !== true
        ) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Tool refresh acknowledgement is stale or invalid",
            },
          };
        }
        return { data: result.data as RefreshToolsResponse };
      },
    }),
    previewTransform: builder.mutation<
      TransformPreviewResponse,
      { chatId: string; options: TransformOptions }
    >({
      async queryFn({ chatId, options }, api, _opts, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          buildPath(TRAJECTORY_TRANSFORM_PREVIEW_URL, chatId),
        );
        const result = await baseQuery({
          url,
          method: "POST",
          body: { options },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TransformPreviewResponse };
      },
    }),

    applyTransform: builder.mutation<
      TransformApplyResponse,
      { chatId: string; options: TransformOptions }
    >({
      async queryFn({ chatId, options }, api, _opts, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          buildPath(TRAJECTORY_TRANSFORM_APPLY_URL, chatId),
        );
        const result = await baseQuery({
          url,
          method: "POST",
          body: { options },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TransformApplyResponse };
      },
    }),

    previewLlmCompress: builder.mutation<
      LlmCompressPreviewResponse,
      { chatId: string; options: LlmCompressOptions }
    >({
      async queryFn({ chatId, options }, api, _opts, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          buildPath(TRAJECTORY_LLM_COMPRESS_PREVIEW_URL, chatId),
        );
        const result = await baseQuery({
          url,
          method: "POST",
          body: { options },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as LlmCompressPreviewResponse };
      },
    }),

    applyLlmCompress: builder.mutation<
      LlmCompressApplyResponse,
      { chatId: string; options: LlmCompressOptions }
    >({
      async queryFn({ chatId, options }, api, _opts, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          buildPath(TRAJECTORY_LLM_COMPRESS_APPLY_URL, chatId),
        );
        const result = await baseQuery({
          url,
          method: "POST",
          body: { options },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as LlmCompressApplyResponse };
      },
    }),

    applyModeTransition: builder.mutation<
      ModeTransitionApplyResponse,
      {
        chatId: string;
        targetMode: string;
        targetModeDescription?: string;
      }
    >({
      async queryFn(
        { chatId, targetMode, targetModeDescription },
        api,
        _opts,
        baseQuery,
      ) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          buildPath(TRAJECTORY_MODE_TRANSITION_APPLY_URL, chatId),
        );
        const result = await baseQuery({
          url,
          method: "POST",
          body: {
            target_mode: targetMode,
            target_mode_description: targetModeDescription ?? "",
          },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as ModeTransitionApplyResponse };
      },
    }),
  }),
});

export const {
  useRefreshToolsMutation,
  usePreviewTransformMutation,
  useApplyTransformMutation,
  usePreviewLlmCompressMutation,
  useApplyLlmCompressMutation,
  useApplyModeTransitionMutation,
} = trajectoryApi;
