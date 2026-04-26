import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { lspQueryFn } from "./queryHelpers";

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
  has_project_root: boolean;
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
      queryFn: lspQueryFn<undefined, ExtRegistryResponse>(
        (_arg, port) => `http://127.0.0.1:${port}/v1/ext/registry`,
      ),
      providesTags: ["ExtRegistry"],
    }),

    getSkill: builder.query<SkillDetail, { name: string; scope?: string }>({
      queryFn: lspQueryFn<{ name: string; scope?: string }, SkillDetail>(
        ({ name, scope }, port) =>
          `http://127.0.0.1:${port}/v1/ext/skills/${name}${
            scope ? `?scope=${scope}` : ""
          }`,
      ),
      providesTags: (_result, _error, { name }) => [
        { type: "Skill", id: name },
      ],
    }),

    saveSkill: builder.mutation<
      undefined,
      { name: string; scope?: string; body: Record<string, unknown> }
    >({
      queryFn: lspQueryFn<
        { name: string; scope?: string; body: Record<string, unknown> },
        undefined
      >(({ name, scope, body }, port) => ({
        url: `http://127.0.0.1:${port}/v1/ext/skills/${name}${
          scope ? `?scope=${scope}` : ""
        }`,
        method: "PUT",
        body,
      })),
      invalidatesTags: (_result, _error, { name }) => [
        "ExtRegistry",
        { type: "Skill" as const, id: name },
      ],
    }),

    createSkill: builder.mutation<
      undefined,
      { name: string; scope: string; description: string; body: string }
    >({
      queryFn: lspQueryFn<
        { name: string; scope: string; description: string; body: string },
        undefined
      >((body, port) => ({
        url: `http://127.0.0.1:${port}/v1/ext/skills`,
        method: "POST",
        body,
      })),
      invalidatesTags: ["ExtRegistry"],
    }),

    deleteSkill: builder.mutation<undefined, { name: string; scope?: string }>({
      queryFn: lspQueryFn<{ name: string; scope?: string }, undefined>(
        ({ name, scope }, port) => ({
          url: `http://127.0.0.1:${port}/v1/ext/skills/${name}${
            scope ? `?scope=${scope}` : ""
          }`,
          method: "DELETE",
        }),
      ),
      invalidatesTags: ["ExtRegistry"],
    }),

    getCommand: builder.query<CommandDetail, { name: string; scope?: string }>({
      queryFn: lspQueryFn<{ name: string; scope?: string }, CommandDetail>(
        ({ name, scope }, port) =>
          `http://127.0.0.1:${port}/v1/ext/commands/${name}${
            scope ? `?scope=${scope}` : ""
          }`,
      ),
      providesTags: (_result, _error, { name }) => [
        { type: "Command", id: name },
      ],
    }),

    saveCommand: builder.mutation<
      undefined,
      { name: string; scope?: string; body: Record<string, unknown> }
    >({
      queryFn: lspQueryFn<
        { name: string; scope?: string; body: Record<string, unknown> },
        undefined
      >(({ name, scope, body }, port) => ({
        url: `http://127.0.0.1:${port}/v1/ext/commands/${name}${
          scope ? `?scope=${scope}` : ""
        }`,
        method: "PUT",
        body,
      })),
      invalidatesTags: (_result, _error, { name }) => [
        "ExtRegistry",
        { type: "Command" as const, id: name },
      ],
    }),

    createCommand: builder.mutation<undefined, Record<string, unknown>>({
      queryFn: lspQueryFn<Record<string, unknown>, undefined>((body, port) => ({
        url: `http://127.0.0.1:${port}/v1/ext/commands`,
        method: "POST",
        body,
      })),
      invalidatesTags: ["ExtRegistry"],
    }),

    deleteCommand: builder.mutation<
      undefined,
      { name: string; scope?: string }
    >({
      queryFn: lspQueryFn<{ name: string; scope?: string }, undefined>(
        ({ name, scope }, port) => ({
          url: `http://127.0.0.1:${port}/v1/ext/commands/${name}${
            scope ? `?scope=${scope}` : ""
          }`,
          method: "DELETE",
        }),
      ),
      invalidatesTags: ["ExtRegistry"],
    }),

    getHooks: builder.query<HooksDetail, { scope?: string }>({
      queryFn: lspQueryFn<{ scope?: string }, HooksDetail>(
        ({ scope }, port) =>
          `http://127.0.0.1:${port}/v1/ext/hooks${
            scope ? `?scope=${scope}` : ""
          }`,
      ),
      providesTags: ["Hooks"],
    }),

    saveHooks: builder.mutation<
      undefined,
      { scope?: string; body: Record<string, unknown> }
    >({
      queryFn: lspQueryFn<
        { scope?: string; body: Record<string, unknown> },
        undefined
      >(({ scope, body }, port) => ({
        url: `http://127.0.0.1:${port}/v1/ext/hooks${
          scope ? `?scope=${scope}` : ""
        }`,
        method: "PUT",
        body,
      })),
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
