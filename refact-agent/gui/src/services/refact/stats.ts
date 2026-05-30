import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import type {
  StatsSummary,
  StatsEventsParams,
  StatsEventsResponse,
} from "../../features/StatsDashboard/types";

export const statsApi = createApi({
  reducerPath: "statsApi",
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
    getStatsSummary: builder.query<
      StatsSummary,
      { from?: string; to?: string }
    >({
      queryFn: async (args, api, _extraOptions, baseQuery) => {
        const state = (api.getState as () => RootState)();
        const url = buildApiUrlFromState(state, "/v1/stats/llm/summary", {
          from: args.from,
          to: args.to,
        });
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return { data: result.data as StatsSummary };
      },
    }),
    getStatsEvents: builder.query<StatsEventsResponse, StatsEventsParams>({
      queryFn: async (args, api, _extraOptions, baseQuery) => {
        const state = (api.getState as () => RootState)();
        const url = buildApiUrlFromState(state, "/v1/stats/llm/events", {
          from: args.from,
          to: args.to,
          limit: args.limit,
          offset: args.offset,
          model: args.model,
          provider: args.provider,
        });
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return { data: result.data as StatsEventsResponse };
      },
    }),
  }),
  refetchOnMountOrArgChange: true,
});

export const { useGetStatsSummaryQuery, useGetStatsEventsQuery } = statsApi;
