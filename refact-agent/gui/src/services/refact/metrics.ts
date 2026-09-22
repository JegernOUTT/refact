import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import { parseMetricsPage, type MetricsPage } from "./metricsTypes";

export type MetricsQuery = {
  endpoint: string;
  cursor?: string;
};

export const metricsApi = createApi({
  reducerPath: "metricsApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  keepUnusedDataFor: 0,
  endpoints: (builder) => ({
    getMetrics: builder.query<MetricsPage, MetricsQuery>({
      queryFn: async (query, api, _options, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            "/v1/metrics",
            {
              cursor: query.cursor,
            },
          ),
        });
        if (result.error) return { error: result.error };
        const data = parseMetricsPage(result.data);
        return data
          ? { data }
          : {
              error: {
                status: "CUSTOM_ERROR",
                error: "Invalid metrics snapshot",
              },
            };
      },
    }),
  }),
});

export const { useGetMetricsQuery } = metricsApi;
