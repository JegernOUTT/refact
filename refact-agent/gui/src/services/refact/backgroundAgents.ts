import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import {
  isValidBackgroundAgent,
  normalizeBackgroundAgentSummary,
} from "./chatSubscription";
import type { BackgroundAgentSummary } from "./types";

export type CancelBackgroundAgentRequest = {
  agentId: string;
  subtree?: boolean;
};

export type MessageBackgroundAgentRequest = {
  agentId: string;
  text: string;
};

// TODO: Migrate this API when a shared refact API slice provides this base query.
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
        if (!Array.isArray(result.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Invalid response from background agents",
              data: result.data,
            },
          };
        }
        return {
          data: result.data
            .filter(isValidBackgroundAgent)
            .map(normalizeBackgroundAgentSummary),
        };
      },
      providesTags: (_result, _error, chatId) => [
        { type: "BackgroundAgents", id: chatId },
      ],
    }),
    cancelBackgroundAgent: builder.mutation<
      unknown,
      CancelBackgroundAgentRequest
    >({
      queryFn: async ({ agentId, subtree = true }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/background-agents/${encodeURIComponent(agentId)}/cancel`,
          ),
          method: "POST",
          body: { subtree },
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
