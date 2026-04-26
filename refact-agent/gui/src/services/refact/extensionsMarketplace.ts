import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { extensionsApi } from "./extensions";

export type MarketplaceKind = "skill" | "command" | "subagent";

export type MarketplaceItemParam = {
  name: string;
  label: string;
  default?: string;
  required: boolean;
};

export type MarketplaceSourceKind =
  | "builtin_embedded"
  | "builtin_github"
  | "user_github";

export type MarketplaceParserMode = "manifest" | "scan" | "overlay";

export type ExtensionMarketplaceSource = {
  id: string;
  label: string;
  description: string;
  enabled: boolean;
  builtin: boolean;
  removable: boolean;
  source_kind: MarketplaceSourceKind;
  repo_url?: string;
  supported_kinds: MarketplaceKind[];
  parser_mode: MarketplaceParserMode;
  last_sync_at?: string;
  error?: string;
  item_count?: number;
};

export type ExtensionMarketplaceItem = {
  id: string;
  name: string;
  description: string;
  tags: string[];
  publisher: string;
  homepage?: string;
  kind: MarketplaceKind;
  source_id: string;
  source_label: string;
  path: string;
  installed_scopes: string[];
  body_preview?: string;
  params?: MarketplaceItemParam[];
};

export type ExtensionMarketplaceResponse = {
  items: ExtensionMarketplaceItem[];
  sources: ExtensionMarketplaceSource[];
};

export type ExtensionMarketplaceInstallResponse = {
  installed: boolean;
  scope: "local" | "global";
  file_path: string;
  item_id: string;
  name: string;
};

export const extensionsMarketplaceApi = createApi({
  reducerPath: "extensionsMarketplaceApi",
  tagTypes: [
    "ExtensionMarketplaceSources",
    "SkillsMarketplace",
    "CommandsMarketplace",
    "SubagentsMarketplace",
  ],
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
    getExtensionMarketplaceSources: builder.query<
      { sources: ExtensionMarketplaceSource[] },
      undefined
    >({
      queryFn: async (_arg, api, _opts, baseQuery) => {
        const port = (api.getState() as RootState).config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/marketplace/sources`,
        });
        if (result.error) return { error: result.error };
        return {
          data: result.data as { sources: ExtensionMarketplaceSource[] },
        };
      },
      providesTags: ["ExtensionMarketplaceSources"],
    }),

    saveExtensionMarketplaceSource: builder.mutation<
      { ok: boolean; source: ExtensionMarketplaceSource },
      { url: string; enabled: boolean }
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const port = (api.getState() as RootState).config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/marketplace/sources`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return {
          data: result.data as {
            ok: boolean;
            source: ExtensionMarketplaceSource;
          },
        };
      },
      invalidatesTags: [
        "ExtensionMarketplaceSources",
        "SkillsMarketplace",
        "CommandsMarketplace",
        "SubagentsMarketplace",
      ],
    }),

    deleteExtensionMarketplaceSource: builder.mutation<
      { ok: boolean },
      { id: string }
    >({
      queryFn: async ({ id }, api, _opts, baseQuery) => {
        const port = (api.getState() as RootState).config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/marketplace/sources/${encodeURIComponent(
            id,
          )}`,
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { ok: boolean } };
      },
      invalidatesTags: [
        "ExtensionMarketplaceSources",
        "SkillsMarketplace",
        "CommandsMarketplace",
        "SubagentsMarketplace",
      ],
    }),

    configureExtensionMarketplaceSource: builder.mutation<
      { ok: boolean },
      { id: string; enabled?: boolean }
    >({
      queryFn: async ({ id, ...body }, api, _opts, baseQuery) => {
        const port = (api.getState() as RootState).config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/marketplace/sources/${encodeURIComponent(
            id,
          )}/configure`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { ok: boolean } };
      },
      invalidatesTags: [
        "ExtensionMarketplaceSources",
        "SkillsMarketplace",
        "CommandsMarketplace",
        "SubagentsMarketplace",
      ],
    }),

    getSubagentsMarketplace: builder.query<
      ExtensionMarketplaceResponse,
      { source?: string; q?: string } | undefined
    >({
      queryFn: async (params, api, _opts, baseQuery) => {
        const port = (api.getState() as RootState).config.lspPort;
        const search = new URLSearchParams();
        if (params?.source) search.set("source", params.source);
        if (params?.q) search.set("q", params.q);
        const qs = search.toString();
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/subagents/marketplace${
            qs ? `?${qs}` : ""
          }`,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as ExtensionMarketplaceResponse };
      },
      providesTags: ["SubagentsMarketplace"],
    }),

    getSkillsMarketplace: builder.query<
      ExtensionMarketplaceResponse,
      { source?: string; q?: string } | undefined
    >({
      queryFn: async (params, api, _opts, baseQuery) => {
        const port = (api.getState() as RootState).config.lspPort;
        const search = new URLSearchParams();
        if (params?.source) search.set("source", params.source);
        if (params?.q) search.set("q", params.q);
        const qs = search.toString();
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/skills/marketplace${
            qs ? `?${qs}` : ""
          }`,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as ExtensionMarketplaceResponse };
      },
      providesTags: ["SkillsMarketplace"],
    }),

    getCommandsMarketplace: builder.query<
      ExtensionMarketplaceResponse,
      { source?: string; q?: string } | undefined
    >({
      queryFn: async (params, api, _opts, baseQuery) => {
        const port = (api.getState() as RootState).config.lspPort;
        const search = new URLSearchParams();
        if (params?.source) search.set("source", params.source);
        if (params?.q) search.set("q", params.q);
        const qs = search.toString();
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/commands/marketplace${
            qs ? `?${qs}` : ""
          }`,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as ExtensionMarketplaceResponse };
      },
      providesTags: ["CommandsMarketplace"],
    }),

    refreshExtensionMarketplaceSource: builder.mutation<
      { ok: boolean },
      { id: string }
    >({
      queryFn: async ({ id }, api, _opts, baseQuery) => {
        const port = (api.getState() as RootState).config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/marketplace/sources/${encodeURIComponent(
            id,
          )}/refresh`,
          method: "POST",
          body: {},
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { ok: boolean } };
      },
      invalidatesTags: [
        "ExtensionMarketplaceSources",
        "SkillsMarketplace",
        "CommandsMarketplace",
        "SubagentsMarketplace",
      ],
    }),

    installMarketplaceSkill: builder.mutation<
      ExtensionMarketplaceInstallResponse,
      {
        source_id: string;
        item_id: string;
        scope: "local" | "global";
        overwrite?: boolean;
        params?: Record<string, string>;
      }
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const port = (api.getState() as RootState).config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/skills/marketplace/install`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as ExtensionMarketplaceInstallResponse };
      },
      invalidatesTags: ["SkillsMarketplace"],
      async onQueryStarted(_arg, { dispatch, queryFulfilled }) {
        await queryFulfilled;
        dispatch(extensionsApi.util.invalidateTags(["ExtRegistry"]));
      },
    }),

    installMarketplaceCommand: builder.mutation<
      ExtensionMarketplaceInstallResponse,
      {
        source_id: string;
        item_id: string;
        scope: "local" | "global";
        overwrite?: boolean;
        params?: Record<string, string>;
      }
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const port = (api.getState() as RootState).config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/commands/marketplace/install`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as ExtensionMarketplaceInstallResponse };
      },
      invalidatesTags: ["CommandsMarketplace"],
      async onQueryStarted(_arg, { dispatch, queryFulfilled }) {
        await queryFulfilled;
        dispatch(extensionsApi.util.invalidateTags(["ExtRegistry"]));
      },
    }),

    installMarketplaceSubagent: builder.mutation<
      ExtensionMarketplaceInstallResponse,
      {
        source_id: string;
        item_id: string;
        scope: "local" | "global";
        overwrite?: boolean;
        params?: Record<string, string>;
      }
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const port = (api.getState() as RootState).config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/subagents/marketplace/install`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as ExtensionMarketplaceInstallResponse };
      },
      invalidatesTags: ["SubagentsMarketplace", "ExtensionMarketplaceSources"],
    }),
  }),
});

export const {
  useGetExtensionMarketplaceSourcesQuery,
  useSaveExtensionMarketplaceSourceMutation,
  useDeleteExtensionMarketplaceSourceMutation,
  useConfigureExtensionMarketplaceSourceMutation,
  useRefreshExtensionMarketplaceSourceMutation,
  useGetSkillsMarketplaceQuery,
  useGetCommandsMarketplaceQuery,
  useGetSubagentsMarketplaceQuery,
  useInstallMarketplaceSkillMutation,
  useInstallMarketplaceCommandMutation,
  useInstallMarketplaceSubagentMutation,
} = extensionsMarketplaceApi;
