import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
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
        const port = state.config.lspPort;
        const params = new URLSearchParams();
        if (args.from) params.set("from", args.from);
        if (args.to) params.set("to", args.to);
        const url = `http://127.0.0.1:${port}/v1/stats/llm/summary?${params.toString()}`;
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return { data: result.data as StatsSummary };
      },
    }),
    getStatsEvents: builder.query<StatsEventsResponse, StatsEventsParams>({
      queryFn: async (args, api, _extraOptions, baseQuery) => {
        const state = (api.getState as () => RootState)();
        const port = state.config.lspPort;
        const params = new URLSearchParams();
        if (args.from) params.set("from", args.from);
        if (args.to) params.set("to", args.to);
        if (args.limit !== undefined) params.set("limit", String(args.limit));
        if (args.offset !== undefined)
          params.set("offset", String(args.offset));
        if (args.model) params.set("model", args.model);
        if (args.provider) params.set("provider", args.provider);
        const url = `http://127.0.0.1:${port}/v1/stats/llm/events?${params.toString()}`;
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return { data: result.data as StatsEventsResponse };
      },
    }),
  }),
  refetchOnMountOrArgChange: true,
});

export const { useGetStatsSummaryQuery, useGetStatsEventsQuery } = statsApi;
