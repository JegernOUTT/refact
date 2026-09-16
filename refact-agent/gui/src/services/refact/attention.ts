import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import { parseAttentionQueue, type AttentionQueue } from "./attentionTypes";

export type AttentionQuery = { endpoint: string };

export const attentionApi = createApi({
  reducerPath: "attentionApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  keepUnusedDataFor: 0,
  endpoints: (builder) => ({
    getAttentionQueue: builder.query<AttentionQueue, AttentionQuery>({
      queryFn: async (_query, api, _options, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            "/v1/attention",
          ),
        });
        if (result.error) return { error: result.error };
        const data = parseAttentionQueue(result.data);
        return data
          ? { data }
          : {
              error: {
                status: "CUSTOM_ERROR",
                error: "Invalid attention queue snapshot",
              },
            };
      },
    }),
  }),
});

export const { useGetAttentionQueueQuery } = attentionApi;
