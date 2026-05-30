import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type MCPServer = {
  id: string;
  source_id: string;
  name: string;
  description: string;
  publisher: string;
  tags: string[];
  icon_url?: string;
  homepage?: string;
  transport: "stdio" | "http" | "sse";
  install_recipe: {
    command?: string;
    url?: string;
    env?: Record<string, string>;
    headers?: Record<string, string>;
  };
  confirmation_default: string[];
  verified?: boolean;
  use_count?: number;
};

export type MarketplaceSource = {
  id: string;
  label: string;
  type: "refact_index" | "smithery" | "official_mcp";
  enabled: boolean;
  removable: boolean;
  server_count?: number;
  status?: "ok" | "error" | "loading";
  error?: string;
  needs_api_key?: boolean;
  has_api_key?: boolean;
};

export type MarketplacePagination = {
  page: number;
  page_size: number;
  total: number;
};

export type MarketplaceResponse = {
  servers: MCPServer[];
  sources: MarketplaceSource[];
  pagination?: MarketplacePagination;
  source?: "remote" | "local" | "merged";
};

export type MarketplaceQueryParams = {
  source?: string;
  q?: string;
  page?: number;
  page_size?: number;
};

export type InstallRequest = {
  server_id: string;
  source_id?: string;
  config_overrides?: {
    env?: Record<string, string>;
    headers?: Record<string, string>;
  };
};

export type InstallResponse = {
  installed: boolean;
  config_path: string;
};

export type InstalledServer = {
  id: string;
  name: string;
  config_path: string;
};

export type InstalledResponse = {
  installed: InstalledServer[];
};

export type SaveSourceRequest = {
  id: string;
  label: string;
  type: "refact_index";
  url: string;
  enabled: boolean;
};

export type DeleteSourceRequest = {
  id: string;
};

export type ConfigureSourceRequest = {
  id: string;
  api_key?: string;
  enabled?: boolean;
};

export const mcpMarketplaceApi = createApi({
  reducerPath: "mcpMarketplaceApi",
  tagTypes: ["MarketplaceServers", "InstalledServers", "MarketplaceSources"],
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
    getMarketplace: builder.query<
      MarketplaceResponse,
      MarketplaceQueryParams | undefined
    >({
      queryFn: async (params, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/mcp/marketplace", params),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as MarketplaceResponse };
      },
      providesTags: ["MarketplaceServers"],
    }),

    installServer: builder.mutation<InstallResponse, InstallRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/mcp/marketplace/install"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as InstallResponse };
      },
      invalidatesTags: ["InstalledServers"],
    }),

    getInstalledServers: builder.query<InstalledResponse, undefined>({
      queryFn: async (_arg, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/mcp/marketplace/installed"),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as InstalledResponse };
      },
      providesTags: ["InstalledServers"],
    }),

    getAutoName: builder.mutation<AutoNameResponse, AutoNameRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/mcp/auto-name"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as AutoNameResponse };
      },
    }),

    getMarketplaceSources: builder.query<
      { sources: MarketplaceSource[] },
      undefined
    >({
      queryFn: async (_arg, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/mcp/marketplace/sources"),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { sources: MarketplaceSource[] } };
      },
      providesTags: ["MarketplaceSources"],
    }),

    saveMarketplaceSource: builder.mutation<{ ok: boolean }, SaveSourceRequest>(
      {
        queryFn: async (body, api, _opts, baseQuery) => {
          const state = api.getState() as RootState;
          const result = await baseQuery({
            url: buildApiUrlFromState(state, "/v1/mcp/marketplace/sources"),
            method: "POST",
            body,
          });
          if (result.error) return { error: result.error };
          return { data: result.data as { ok: boolean } };
        },
        invalidatesTags: ["MarketplaceSources", "MarketplaceServers"],
      },
    ),

    deleteMarketplaceSource: builder.mutation<
      { ok: boolean },
      DeleteSourceRequest
    >({
      queryFn: async ({ id }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/mcp/marketplace/sources/${encodeURIComponent(id)}`,
          ),
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { ok: boolean } };
      },
      invalidatesTags: ["MarketplaceSources", "MarketplaceServers"],
    }),

    configureMarketplaceSource: builder.mutation<
      { ok: boolean },
      ConfigureSourceRequest
    >({
      queryFn: async ({ id, ...body }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/mcp/marketplace/sources/${encodeURIComponent(id)}/configure`,
          ),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { ok: boolean } };
      },
      invalidatesTags: ["MarketplaceSources", "MarketplaceServers"],
    }),
  }),
});

export type AutoNameRequest = {
  input: string;
};

export type AutoNameResponse = {
  suggested_name: string;
  transport: "stdio" | "http" | "sse";
  config_prefix: string;
};

export const {
  useGetMarketplaceQuery,
  useInstallServerMutation,
  useGetInstalledServersQuery,
  useGetAutoNameMutation,
  useGetMarketplaceSourcesQuery,
  useSaveMarketplaceSourceMutation,
  useDeleteMarketplaceSourceMutation,
  useConfigureMarketplaceSourceMutation,
} = mcpMarketplaceApi;
