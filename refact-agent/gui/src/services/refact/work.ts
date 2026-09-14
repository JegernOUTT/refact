import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import { parseWorkPage, type WorkPage, type WorkState } from "./workTypes";

export type WorkQuery = {
  endpoint: string;
  chatId?: string;
  states?: WorkState[];
  cursor?: string;
};

export const workApi = createApi({
  reducerPath: "workApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  keepUnusedDataFor: 0,
  endpoints: (builder) => ({
    getWork: builder.query<WorkPage, WorkQuery>({
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
          url: buildApiUrlFromState(api.getState() as RootState, "/v1/work", {
            chat_id: query.chatId,
            states: query.states?.join(","),
            cursor: query.cursor,
          }),
        });
        if (result.error) return { error: result.error };
        const data = parseWorkPage(result.data);
        return data
          ? { data }
          : {
              error: { status: "CUSTOM_ERROR", error: "Invalid Work snapshot" },
            };
      },
    }),
  }),
});

export const { useGetWorkQuery } = workApi;
