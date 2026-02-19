import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import {
  TRAJECTORY_TRANSFORM_PREVIEW_URL,
  TRAJECTORY_TRANSFORM_APPLY_URL,
  TRAJECTORY_HANDOFF_PREVIEW_URL,
  TRAJECTORY_HANDOFF_APPLY_URL,
  TRAJECTORY_MODE_TRANSITION_APPLY_URL,
} from "./consts";

export type TransformOptions = {
  dedup_and_compress_context?: boolean;
  drop_all_context?: boolean;
  compress_non_agentic_tools?: boolean;
  drop_all_memories?: boolean;
  drop_project_information?: boolean;
};

export type HandoffOptions = {
  include_last_user_plus?: boolean;
  include_all_opened_context?: boolean;
  include_all_edited_context?: boolean;
  include_agentic_tools?: boolean;
  llm_summary_for_excluded?: boolean;
  include_all_user_assistant_only?: boolean;
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

export type HandoffPreviewResponse = {
  stats: TransformStats;
  actions: string[];
  llm_summary?: string | null;
};

export type HandoffApplyResponse = {
  new_chat_id: string;
  stats: TransformStats;
  browser_runtime_id?: string | null;
};

export type ModeTransitionApplyResponse = {
  new_chat_id: string;
  messages_count: number;
};

function buildUrl(template: string, chatId: string, port: number): string {
  return `http://127.0.0.1:${port}${template.replace(
    "{chat_id}",
    encodeURIComponent(chatId),
  )}`;
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
        const port = state.config.lspPort;
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
        const port = state.config.lspPort;
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
        const port = state.config.lspPort;
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
        const port = state.config.lspPort;
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
        const port = state.config.lspPort;
        const url = buildUrl(
          TRAJECTORY_MODE_TRANSITION_APPLY_URL,
          chatId,
          port,
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
  usePreviewTransformMutation,
  useApplyTransformMutation,
  usePreviewHandoffMutation,
  useApplyHandoffMutation,
  useApplyModeTransitionMutation,
} = trajectoryApi;
