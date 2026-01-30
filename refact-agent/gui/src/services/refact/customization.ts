import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";

export interface ConfigItem {
  id: string;
  kind: string;
  title: string;
  file_path: string;
  specific: boolean;
}

export interface ErrorItem {
  file_path: string;
  error: string;
}

export interface RegistryResponse {
  modes: ConfigItem[];
  subagents: ConfigItem[];
  toolbox_commands: ConfigItem[];
  code_lens: ConfigItem[];
  errors: ErrorItem[];
}

export interface ConfigDetailResponse {
  config: Record<string, unknown>;
  file_path: string;
  raw_yaml: string;
}

export interface SaveConfigResponse {
  ok: boolean;
  file_path: string;
  errors: ErrorItem[];
}

export interface DeleteConfigResponse {
  ok: boolean;
  errors: ErrorItem[];
}

export type ConfigKind = "modes" | "subagents" | "toolbox_commands" | "code_lens";

export const customizationApi = createApi({
  reducerPath: "customizationApi",
  tagTypes: ["Registry", "Config"],
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
    getRegistry: builder.query<RegistryResponse, void>({
      queryFn: async (_arg, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/customization/registry`,
        });
        if (result.error) {
          return { error: { status: result.error.status as number, data: String(result.error.data) } };
        }
        return { data: result.data as RegistryResponse };
      },
      providesTags: ["Registry"],
    }),

    getConfig: builder.query<ConfigDetailResponse, { kind: ConfigKind; id: string }>({
      queryFn: async ({ kind, id }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/customization/${kind}/${id}`,
        });
        if (result.error) {
          return { error: { status: result.error.status as number, data: String(result.error.data) } };
        }
        return { data: result.data as ConfigDetailResponse };
      },
      providesTags: (_result, _error, { kind, id }) => [{ type: "Config", id: `${kind}/${id}` }],
    }),

    saveConfig: builder.mutation<SaveConfigResponse, { kind: ConfigKind; id: string; config: Record<string, unknown> }>({
      queryFn: async ({ kind, id, config }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/customization/${kind}/${id}`,
          method: "PUT",
          body: { config },
        });
        if (result.error) {
          return { error: { status: result.error.status as number, data: String(result.error.data) } };
        }
        return { data: result.data as SaveConfigResponse };
      },
      invalidatesTags: (_result, _error, { kind, id }) => ["Registry", { type: "Config", id: `${kind}/${id}` }],
    }),

    createConfig: builder.mutation<SaveConfigResponse, { kind: ConfigKind; id: string; config: Record<string, unknown> }>({
      queryFn: async ({ kind, id, config }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/customization/${kind}`,
          method: "POST",
          body: { id, config },
        });
        if (result.error) {
          return { error: { status: result.error.status as number, data: String(result.error.data) } };
        }
        return { data: result.data as SaveConfigResponse };
      },
      invalidatesTags: ["Registry"],
    }),

    deleteConfig: builder.mutation<DeleteConfigResponse, { kind: ConfigKind; id: string }>({
      queryFn: async ({ kind, id }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/customization/${kind}/${id}`,
          method: "DELETE",
        });
        if (result.error) {
          return { error: { status: result.error.status as number, data: String(result.error.data) } };
        }
        return { data: result.data as DeleteConfigResponse };
      },
      invalidatesTags: (_result, _error, { kind, id }) => ["Registry", { type: "Config", id: `${kind}/${id}` }],
    }),
  }),
});

export const {
  useGetRegistryQuery,
  useGetConfigQuery,
  useSaveConfigMutation,
  useCreateConfigMutation,
  useDeleteConfigMutation,
} = customizationApi;
