import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";

export interface SkillRegistryItem {
  name: string;
  description: string;
  source: string;
  source_label: string;
  scope: "global" | "local" | "plugin";
  read_only: boolean;
  file_path: string;
}

export interface CommandRegistryItem {
  name: string;
  description: string;
  source: string;
  source_label: string;
  scope: "global" | "local" | "plugin";
  read_only: boolean;
  file_path: string;
}

export interface HookRegistryItem {
  event: string;
  command: string;
  source: string;
  source_label: string;
  scope: "global" | "local" | "plugin";
  read_only: boolean;
}

export interface ExtRegistryResponse {
  skills: SkillRegistryItem[];
  slash_commands: CommandRegistryItem[];
  hooks: HookRegistryItem[];
}

export interface SkillDetail {
  name: string;
  description: string;
  user_invocable: boolean;
  disable_model_invocation: boolean;
  allowed_tools: string[];
  model: string | null;
  context: string | null;
  agent: string | null;
  argument_hint: string;
  body: string;
  raw_content: string;
  source: string;
  file_path: string;
}

export interface CommandDetail {
  name: string;
  description: string;
  argument_hint: string;
  allowed_tools: string[];
  model: string | null;
  body: string;
  raw_content: string;
  source: string;
  file_path: string;
}

export interface HooksDetail {
  hooks: HookEntry[];
  raw_yaml: string;
  file_path: string;
}

export interface HookEntry {
  event: string;
  command: string;
  matcher?: string;
  timeout?: number;
}

export const extensionsApi = createApi({
  reducerPath: "extensionsApi",
  tagTypes: ["ExtRegistry", "Skill", "Command", "Hooks"],
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
    getExtRegistry: builder.query<ExtRegistryResponse, undefined>({
      queryFn: async (_arg, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/registry`,
        });
        if (result.error) {
          return {
            error: {
              status: result.error.status as number,
              data: String(result.error.data),
            },
          };
        }
        return { data: result.data as ExtRegistryResponse };
      },
      providesTags: ["ExtRegistry"],
    }),

    getSkill: builder.query<SkillDetail, { name: string; scope?: string }>({
      queryFn: async ({ name, scope }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const scopeParam = scope ? `?scope=${scope}` : "";
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/skills/${name}${scopeParam}`,
        });
        if (result.error) {
          return {
            error: {
              status: result.error.status as number,
              data: String(result.error.data),
            },
          };
        }
        return { data: result.data as SkillDetail };
      },
      providesTags: (_result, _error, { name }) => [{ type: "Skill", id: name }],
    }),

    saveSkill: builder.mutation<
      undefined,
      { name: string; scope?: string; body: Record<string, unknown> }
    >({
      queryFn: async ({ name, scope, body }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const scopeParam = scope ? `?scope=${scope}` : "";
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/skills/${name}${scopeParam}`,
          method: "PUT",
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
      invalidatesTags: (_result, _error, { name }) => [
        "ExtRegistry",
        { type: "Skill", id: name },
      ],
    }),

    createSkill: builder.mutation<
      undefined,
      { name: string; scope: string; description: string; body: string }
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/skills`,
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
      invalidatesTags: ["ExtRegistry"],
    }),

    deleteSkill: builder.mutation<undefined, { name: string; scope?: string }>({
      queryFn: async ({ name, scope }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const scopeParam = scope ? `?scope=${scope}` : "";
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/skills/${name}${scopeParam}`,
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
      invalidatesTags: ["ExtRegistry"],
    }),

    getCommand: builder.query<CommandDetail, { name: string; scope?: string }>({
      queryFn: async ({ name, scope }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const scopeParam = scope ? `?scope=${scope}` : "";
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/commands/${name}${scopeParam}`,
        });
        if (result.error) {
          return {
            error: {
              status: result.error.status as number,
              data: String(result.error.data),
            },
          };
        }
        return { data: result.data as CommandDetail };
      },
      providesTags: (_result, _error, { name }) => [
        { type: "Command", id: name },
      ],
    }),

    saveCommand: builder.mutation<
      undefined,
      { name: string; scope?: string; body: Record<string, unknown> }
    >({
      queryFn: async ({ name, scope, body }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const scopeParam = scope ? `?scope=${scope}` : "";
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/commands/${name}${scopeParam}`,
          method: "PUT",
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
      invalidatesTags: (_result, _error, { name }) => [
        "ExtRegistry",
        { type: "Command", id: name },
      ],
    }),

    createCommand: builder.mutation<undefined, Record<string, unknown>>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/commands`,
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
      invalidatesTags: ["ExtRegistry"],
    }),

    deleteCommand: builder.mutation<undefined, { name: string; scope?: string }>({
      queryFn: async ({ name, scope }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const scopeParam = scope ? `?scope=${scope}` : "";
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/commands/${name}${scopeParam}`,
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
      invalidatesTags: ["ExtRegistry"],
    }),

    getHooks: builder.query<HooksDetail, { scope?: string }>({
      queryFn: async ({ scope }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const scopeParam = scope ? `?scope=${scope}` : "";
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/hooks${scopeParam}`,
        });
        if (result.error) {
          return {
            error: {
              status: result.error.status as number,
              data: String(result.error.data),
            },
          };
        }
        return { data: result.data as HooksDetail };
      },
      providesTags: ["Hooks"],
    }),

    saveHooks: builder.mutation<
      undefined,
      { scope?: string; body: Record<string, unknown> }
    >({
      queryFn: async ({ scope, body }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const scopeParam = scope ? `?scope=${scope}` : "";
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/ext/hooks${scopeParam}`,
          method: "PUT",
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
      invalidatesTags: ["Hooks", "ExtRegistry"],
    }),
  }),
});

export const {
  useGetExtRegistryQuery,
  useGetSkillQuery,
  useSaveSkillMutation,
  useCreateSkillMutation,
  useDeleteSkillMutation,
  useGetCommandQuery,
  useSaveCommandMutation,
  useCreateCommandMutation,
  useDeleteCommandMutation,
  useGetHooksQuery,
  useSaveHooksMutation,
} = extensionsApi;
