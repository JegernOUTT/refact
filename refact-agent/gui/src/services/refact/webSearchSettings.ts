import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type SearchCategory =
  | "general"
  | "news"
  | "code"
  | "docs"
  | "papers"
  | "packages";
export type ProviderTier = "general" | "fallback" | "specialist";
export type ProviderStatus =
  | "ok"
  | "empty"
  | "error"
  | "timeout"
  | "skipped"
  | "not_configured"
  | "disabled"
  | "not_applicable";
export type ProviderConfig = { enabled: boolean; api_key?: string };
export type WebSearchConfig = {
  schema_version: 1;
  deadline_ms: number;
  provider_timeout_ms: number;
  prefer_builtin_over_native: boolean;
  searxng_url: string;
  providers: Partial<Record<string, ProviderConfig>>;
};
export type WebSearchConfigPatch = Partial<
  Omit<WebSearchConfig, "providers">
> & {
  providers?: Partial<Record<string, Partial<ProviderConfig>>>;
};
export type WebSearchProvider = {
  id: string;
  display_name: string;
  tier: ProviderTier;
  category: SearchCategory | null;
  key: "none" | "optional" | "required";
  key_env_vars: string[];
  signup_url: string | null;
  description: string;
  default_enabled: boolean;
  credential_status: "stored" | "env" | "missing" | "not_applicable";
  active: boolean;
};
export type WebSearchSettingsResponse = {
  config: WebSearchConfig;
  providers: WebSearchProvider[];
};
export type ProviderReport = {
  provider: string;
  status: ProviderStatus;
  results: number;
  latency_ms: number;
  detail?: string | null;
  top: { title: string; url: string }[];
};
export type TestWebSearchRequest = { query?: string; providers?: string[] };
export type TestWebSearchResponse = {
  query: string;
  reports: ProviderReport[];
};

export const webSearchSettingsApi = createApi({
  reducerPath: "webSearchSettingsApi",
  tagTypes: ["WebSearchSettings"],
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  endpoints: (builder) => ({
    getWebSearchSettings: builder.query<WebSearchSettingsResponse, undefined>({
      queryFn: async (_arg, api, _extra, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            "/v1/web-search-settings",
          ),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as WebSearchSettingsResponse };
      },
      providesTags: ["WebSearchSettings"],
    }),
    updateWebSearchSettings: builder.mutation<
      WebSearchSettingsResponse,
      { config: WebSearchConfigPatch }
    >({
      queryFn: async (body, api, _extra, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            "/v1/web-search-settings",
          ),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        const data = result.data as WebSearchSettingsResponse;
        api.dispatch(
          webSearchSettingsApi.util.updateQueryData(
            "getWebSearchSettings",
            undefined,
            () => data,
          ),
        );
        return { data };
      },
      invalidatesTags: (_result, error) => (error ? [] : ["WebSearchSettings"]),
    }),
    testWebSearchProviders: builder.mutation<
      TestWebSearchResponse,
      TestWebSearchRequest
    >({
      queryFn: async (body, api, _extra, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            "/v1/web-search-settings/test",
          ),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TestWebSearchResponse };
      },
    }),
  }),
});
export const {
  useGetWebSearchSettingsQuery,
  useUpdateWebSearchSettingsMutation,
  useTestWebSearchProvidersMutation,
} = webSearchSettingsApi;
