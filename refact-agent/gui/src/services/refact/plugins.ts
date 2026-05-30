import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { extensionsApi } from "./extensions";
import { lspQueryFn } from "./queryHelpers";

export interface MarketplaceEntry {
  name: string;
  source: string;
  added_at: string | null;
}

export interface MarketplacesResponse {
  marketplaces: MarketplaceEntry[];
}

export interface PluginEntry {
  name: string;
  description: string;
  version?: string;
  tags?: string[];
  marketplace: string;
}

export interface PluginListResponse {
  plugins: PluginEntry[];
}

export interface InstalledPluginEntry {
  name: string;
  install_dir: string;
  installed_at: string;
}

export interface InstalledPluginsResponse {
  installed: InstalledPluginEntry[];
}

export const pluginsApi = createApi({
  reducerPath: "pluginsApi",
  tagTypes: ["Marketplaces", "InstalledPlugins"],
  baseQuery: fetchBaseQuery({
    baseUrl: "/",
    prepareHeaders: (headers, { getState }) => {
      const state = getState() as RootState;
      const token = state.config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  endpoints: (builder) => ({
    getMarketplaces: builder.query<MarketplacesResponse, undefined>({
      queryFn: lspQueryFn<undefined, MarketplacesResponse>(
        () => "/v1/plugins/marketplaces",
      ),
      providesTags: ["Marketplaces"],
    }),

    addMarketplace: builder.mutation<undefined, { source: string }>({
      queryFn: lspQueryFn<{ source: string }, undefined>((body) => ({
        url: "/v1/plugins/marketplaces",
        method: "POST",
        body,
      })),
      invalidatesTags: ["Marketplaces"],
    }),

    deleteMarketplace: builder.mutation<undefined, string>({
      queryFn: lspQueryFn<string, undefined>((name) => ({
        url: `/v1/plugins/marketplaces/${name}`,
        method: "DELETE",
      })),
      invalidatesTags: ["Marketplaces"],
    }),

    getMarketplacePlugins: builder.query<PluginListResponse, string>({
      queryFn: lspQueryFn<string, PluginListResponse>(
        (name) => `/v1/plugins/marketplace/${name}/plugins`,
      ),
    }),

    installPlugin: builder.mutation<
      undefined,
      { plugin: string; marketplace: string }
    >({
      queryFn: lspQueryFn<{ plugin: string; marketplace: string }, undefined>(
        (body) => ({
          url: "/v1/plugins/install",
          method: "POST",
          body,
        }),
      ),
      invalidatesTags: ["InstalledPlugins", "Marketplaces"],
      onQueryStarted: async (_arg, { dispatch, queryFulfilled }) => {
        await queryFulfilled;
        dispatch(extensionsApi.util.invalidateTags(["ExtRegistry"]));
      },
    }),

    getInstalled: builder.query<InstalledPluginsResponse, undefined>({
      queryFn: lspQueryFn<undefined, InstalledPluginsResponse>(
        () => "/v1/plugins/installed",
      ),
      providesTags: ["InstalledPlugins"],
    }),

    uninstallPlugin: builder.mutation<undefined, string>({
      queryFn: lspQueryFn<string, undefined>((name) => ({
        url: `/v1/plugins/installed/${name}`,
        method: "DELETE",
      })),
      invalidatesTags: ["InstalledPlugins"],
      onQueryStarted: async (_arg, { dispatch, queryFulfilled }) => {
        await queryFulfilled;
        dispatch(extensionsApi.util.invalidateTags(["ExtRegistry"]));
      },
    }),
  }),
});

export const {
  useGetMarketplacesQuery,
  useAddMarketplaceMutation,
  useDeleteMarketplaceMutation,
  useGetMarketplacePluginsQuery,
  useInstallPluginMutation,
  useGetInstalledQuery,
  useUninstallPluginMutation,
} = pluginsApi;
