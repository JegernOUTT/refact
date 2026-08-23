import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type IndexingSettingsScope = "global" | "project";

export interface IndexingSettingsConfig {
  blocklist: string[];
  additional_indexing_dirs: string[];
}

export interface IndexingSettingsResponse {
  scope: IndexingSettingsScope;
  path: string;
  config: IndexingSettingsConfig;
  project_available: boolean;
}

export interface SaveIndexingSettingsRequest {
  scope: IndexingSettingsScope;
  config: IndexingSettingsConfig;
}

export const indexingSettingsApi = createApi({
  reducerPath: "indexingSettingsApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  tagTypes: ["IndexingSettings"],
  endpoints: (builder) => ({
    getIndexingSettings: builder.query<
      IndexingSettingsResponse,
      IndexingSettingsScope
    >({
      queryFn: async (scope, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/indexing-settings", {
            scope,
          }),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as IndexingSettingsResponse };
      },
      providesTags: (_result, _error, scope) => [
        { type: "IndexingSettings", id: scope },
      ],
    }),
    saveIndexingSettings: builder.mutation<
      IndexingSettingsResponse,
      SaveIndexingSettingsRequest
    >({
      queryFn: async ({ scope, config }, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/indexing-settings", {
            scope,
          }),
          method: "POST",
          body: config,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as IndexingSettingsResponse };
      },
      invalidatesTags: (_result, _error, { scope }) => [
        { type: "IndexingSettings", id: scope },
      ],
    }),
  }),
  refetchOnMountOrArgChange: true,
});

export const { useGetIndexingSettingsQuery, useSaveIndexingSettingsMutation } =
  indexingSettingsApi;
