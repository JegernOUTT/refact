import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";

export interface MarketplaceEntry {
  name: string;
  source: string;
  cached_at: string | null;
}

export interface MarketplacesResponse {
  marketplaces: MarketplaceEntry[];
}

export interface PluginEntry {
  id: string;
  name: string;
  description: string;
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
      queryFn: async (_arg, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/plugins/marketplaces`,
        });
        if (result.error) {
          return {
            error: {
              status: result.error.status as number,
              data: String(result.error.data),
            },
          };
        }
        return { data: result.data as MarketplacesResponse };
      },
      providesTags: ["Marketplaces"],
    }),

    addMarketplace: builder.mutation<undefined, { source: string }>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/plugins/marketplaces`,
          method: "POST",
          body,
        });
        if (result.error) {
          return {
            error: {
              status: result.error.status as number,
              data: String(result.error.data),
            },
          };
        }
        return { data: undefined };
      },
      invalidatesTags: ["Marketplaces"],
    }),

    deleteMarketplace: builder.mutation<undefined, string>({
      queryFn: async (name, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/plugins/marketplaces/${name}`,
          method: "DELETE",
        });
        if (result.error) {
          return {
            error: {
              status: result.error.status as number,
              data: String(result.error.data),
            },
          };
        }
        return { data: undefined };
      },
      invalidatesTags: ["Marketplaces"],
    }),

    getMarketplacePlugins: builder.query<PluginListResponse, string>({
      queryFn: async (name, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/plugins/marketplace/${name}/plugins`,
        });
        if (result.error) {
          return {
            error: {
              status: result.error.status as number,
              data: String(result.error.data),
            },
          };
        }
        return { data: result.data as PluginListResponse };
      },
    }),

    installPlugin: builder.mutation<
      undefined,
      { plugin: string; marketplace: string }
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/plugins/install`,
          method: "POST",
          body,
        });
        if (result.error) {
          return {
            error: {
              status: result.error.status as number,
              data: String(result.error.data),
            },
          };
        }
        return { data: undefined };
      },
      invalidatesTags: ["InstalledPlugins", "Marketplaces"],
    }),

    getInstalled: builder.query<InstalledPluginsResponse, undefined>({
      queryFn: async (_arg, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/plugins/installed`,
        });
        if (result.error) {
          return {
            error: {
              status: result.error.status as number,
              data: String(result.error.data),
            },
          };
        }
        return { data: result.data as InstalledPluginsResponse };
      },
      providesTags: ["InstalledPlugins"],
    }),

    uninstallPlugin: builder.mutation<undefined, string>({
      queryFn: async (name, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/plugins/installed/${name}`,
          method: "DELETE",
        });
        if (result.error) {
          return {
            error: {
              status: result.error.status as number,
              data: String(result.error.data),
            },
          };
        }
        return { data: undefined };
      },
      invalidatesTags: ["InstalledPlugins"],
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
