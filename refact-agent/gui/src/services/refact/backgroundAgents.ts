import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import type { BackgroundAgentSummary } from "./types";

export type CancelBackgroundAgentRequest = {
  agentId: string;
  subtree?: boolean;
  reason?: string;
};

export type MessageBackgroundAgentRequest = {
  agentId: string;
  text: string;
};

export const backgroundAgentsApi = createApi({
  reducerPath: "backgroundAgentsApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  tagTypes: ["BackgroundAgents"],
  endpoints: (builder) => ({
    getBackgroundAgents: builder.query<BackgroundAgentSummary[], string>({
      queryFn: async (chatId, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/background-agents", {
            chat_id: chatId,
          }),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BackgroundAgentSummary[] };
      },
      providesTags: (_result, _error, chatId) => [
        { type: "BackgroundAgents", id: chatId },
      ],
    }),
    cancelBackgroundAgent: builder.mutation<unknown, CancelBackgroundAgentRequest>({
      queryFn: async ({ agentId, subtree = true, reason }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/background-agents/${encodeURIComponent(agentId)}/cancel`,
          ),
          method: "POST",
          body: { subtree, ...(reason === undefined ? {} : { reason }) },
        });
        if (result.error) return { error: result.error };
        return { data: result.data };
      },
      invalidatesTags: ["BackgroundAgents"],
    }),
    messageBackgroundAgent: builder.mutation<
      unknown,
      MessageBackgroundAgentRequest
    >({
      queryFn: async ({ agentId, text }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/background-agents/${encodeURIComponent(agentId)}/message`,
          ),
          method: "POST",
          body: { text },
        });
        if (result.error) return { error: result.error };
        return { data: result.data };
      },
      invalidatesTags: ["BackgroundAgents"],
    }),
  }),
});

export const {
  useGetBackgroundAgentsQuery,
  useCancelBackgroundAgentMutation,
  useMessageBackgroundAgentMutation,
} = backgroundAgentsApi;
