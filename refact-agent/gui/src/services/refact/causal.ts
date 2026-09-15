import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import { parseCausalPage, type CausalPage } from "./causalTypes";

export type CausalQuery = {
  endpoint: string;
  root: string;
  chatId?: string;
  cursor?: string;
};

export const causalApi = createApi({
  reducerPath: "causalApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  keepUnusedDataFor: 0,
  endpoints: (builder) => ({
    getCausalTrace: builder.query<CausalPage, CausalQuery>({
      merge: (current, incoming) => {
        if (
          current.snapshot_id === incoming.snapshot_id &&
          current.seq > incoming.seq
        )
          return;
        return incoming;
      },
      queryFn: async (query, api, _options, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            "/v1/work/causal",
            {
              root: query.root,
              chat_id: query.chatId,
              cursor: query.cursor,
            },
          ),
        });
        if (result.error) return { error: result.error };
        const data = parseCausalPage(result.data);
        return data
          ? { data }
          : {
              error: {
                status: "CUSTOM_ERROR",
                error: "Invalid causal trace snapshot",
              },
            };
      },
    }),
  }),
});

export const { useGetCausalTraceQuery } = causalApi;
