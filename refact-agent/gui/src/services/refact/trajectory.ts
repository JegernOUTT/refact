import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import {
  TRAJECTORY_TRANSFORM_PREVIEW_URL,
  TRAJECTORY_TRANSFORM_APPLY_URL,
  TRAJECTORY_HANDOFF_PREVIEW_URL,
  TRAJECTORY_HANDOFF_APPLY_URL,
} from "./consts";

export type TransformOptions = {
  dedup_and_compress_context?: boolean;
  drop_all_context?: boolean;
  compress_non_agentic_tools?: boolean;
};

export type HandoffOptions = {
  include_last_user_plus?: boolean;
  include_all_opened_context?: boolean;
  include_all_edited_context?: boolean;
  include_agentic_tools?: boolean;
  llm_summary_for_excluded?: boolean;
};

export type TransformPreviewResponse = {
  before_tokens: number;
  after_tokens: number;
  actions: string[];
  estimated_reduction_percent: number;
};

export type TransformApplyResponse = {
  success: boolean;
  new_token_count: number;
};

export type HandoffPreviewResponse = {
  new_chat_title: string;
  summary: string;
  key_files: string[];
  estimated_tokens: number;
};

export type HandoffApplyResponse = {
  success: boolean;
  new_chat_id: string;
};

function buildUrl(template: string, chatId: string, port: number): string {
  return `http://127.0.0.1:${port}${template.replace("{chat_id}", encodeURIComponent(chatId))}`;
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
    previewTransform: builder.mutation<
      TransformPreviewResponse,
      { chatId: string; options: TransformOptions }
    >({
      async queryFn({ chatId, options }, api, _opts, baseQuery) {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as number;
        const url = buildUrl(TRAJECTORY_TRANSFORM_PREVIEW_URL, chatId, port);
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
        const port = state.config.lspPort as number;
        const url = buildUrl(TRAJECTORY_TRANSFORM_APPLY_URL, chatId, port);
        const result = await baseQuery({
          url,
          method: "POST",
          body: { options },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TransformApplyResponse };
      },
    }),

    previewHandoff: builder.mutation<
      HandoffPreviewResponse,
      { chatId: string; options: HandoffOptions }
    >({
      async queryFn({ chatId, options }, api, _opts, baseQuery) {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as number;
        const url = buildUrl(TRAJECTORY_HANDOFF_PREVIEW_URL, chatId, port);
        const result = await baseQuery({
          url,
          method: "POST",
          body: { options },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as HandoffPreviewResponse };
      },
    }),

    applyHandoff: builder.mutation<
      HandoffApplyResponse,
      { chatId: string; options: HandoffOptions }
    >({
      async queryFn({ chatId, options }, api, _opts, baseQuery) {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as number;
        const url = buildUrl(TRAJECTORY_HANDOFF_APPLY_URL, chatId, port);
        const result = await baseQuery({
          url,
          method: "POST",
          body: { options },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as HandoffApplyResponse };
      },
    }),
  }),
});

export const {
  usePreviewTransformMutation,
  useApplyTransformMutation,
  usePreviewHandoffMutation,
  useApplyHandoffMutation,
} = trajectoryApi;
