import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { setRagStatus } from "../../features/Knowledge/knowledgeSlice";
import { buildApiUrlFromState, hasUsableEngineEndpoint } from "./apiUrl";
import type { CodeGraphStatus, RagStatus } from "./types";
import { isCodeGraphStatus, isRagStatus } from "./types";

export const ragStatusApi = createApi({
  reducerPath: "ragStatusApi",
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
    getRagStatus: builder.query<RagStatus, undefined>({
      queryFn: async (_arg, api, _extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        if (!hasUsableEngineEndpoint(state.config)) {
          return {
            error: { status: 500, data: "Missing engine endpoint in config" },
          };
        }
        const url = buildApiUrlFromState(state, "/v1/rag-status");
        const response = await baseQuery({ url });
        if (response.error) {
          return { error: response.error };
        }
        if (!isRagStatus(response.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: `Invalid response from ${url}`,
              data: response.data,
            },
          };
        }
        return { data: response.data };
      },
      async onQueryStarted(_arg, { dispatch, queryFulfilled }) {
        try {
          const { data } = await queryFulfilled;
          dispatch(setRagStatus(data));
        } catch {
          return;
        }
      },
    }),
    getCodeGraphStatus: builder.query<CodeGraphStatus, undefined>({
      queryFn: async (_arg, api, _extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        if (!hasUsableEngineEndpoint(state.config)) {
          return {
            error: { status: 500, data: "Missing engine endpoint in config" },
          };
        }
        const url = buildApiUrlFromState(state, "/v1/codegraph-status");
        const response = await baseQuery({ url });
        if (response.error) {
          return { error: response.error };
        }
        if (!isCodeGraphStatus(response.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: `Invalid response from ${url}`,
              data: response.data,
            },
          };
        }
        return { data: response.data };
      },
    }),
  }),
});

export const { useGetRagStatusQuery, useGetCodeGraphStatusQuery } =
  ragStatusApi;
