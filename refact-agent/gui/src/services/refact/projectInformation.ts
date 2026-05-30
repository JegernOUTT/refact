import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type FileOverride = {
  enabled?: boolean;
  max_chars?: number;
};

export type SectionConfig = {
  enabled: boolean;
  max_chars?: number;
  max_items?: number;
  max_chars_per_item?: number;
  max_depth?: number;
  overrides?: Record<string, FileOverride>;
};

export type ProjectInformationConfig = {
  schema_version: number;
  enabled: boolean;
  defaults: {
    max_chars_per_item: number;
    max_items_per_section: number;
  };
  sections: {
    system_info: SectionConfig;
    environment_instructions: SectionConfig;
    detected_environments: SectionConfig;
    git_info: SectionConfig;
    project_tree: SectionConfig;
    instruction_files: SectionConfig;
    project_configs: SectionConfig;
    memories: SectionConfig;
  };
};

export type ProjectInfoBlock = {
  id: string;
  section: string;
  title: string;
  path: string | null;
  content: string;
  truncated: boolean;
  enabled: boolean;
  char_count: number;
  original_char_count?: number;
};

export type ProjectInformationPreviewResponse = {
  blocks: ProjectInfoBlock[];
  warnings: string[];
};

const DEFAULT_CONFIG: ProjectInformationConfig = {
  schema_version: 1,
  enabled: true,
  defaults: {
    max_chars_per_item: 8000,
    max_items_per_section: 50,
  },
  sections: {
    system_info: { enabled: true },
    environment_instructions: { enabled: true, max_chars: 6000 },
    detected_environments: { enabled: true, max_items: 50 },
    git_info: { enabled: true, max_chars: 6000 },
    project_tree: { enabled: true, max_depth: 4, max_chars: 16000 },
    instruction_files: {
      enabled: true,
      max_items: 20,
      max_chars_per_item: 8000,
    },
    project_configs: { enabled: true, max_items: 30, max_chars_per_item: 4000 },
    memories: { enabled: true, max_items: 30, max_chars_per_item: 2000 },
  },
};

export { DEFAULT_CONFIG as defaultProjectInformationConfig };

export const projectInformationApi = createApi({
  reducerPath: "projectInformationApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const state = getState() as RootState;
      const apiKey = state.config.apiKey;
      if (apiKey) {
        headers.set("Authorization", `Bearer ${apiKey}`);
      }
      return headers;
    },
  }),
  tagTypes: ["ProjectInformation"],
  endpoints: (builder) => ({
    getProjectInformation: builder.query<ProjectInformationConfig, undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/project-information"),
        });
        if (result.error) {
          return { error: result.error };
        }
        return {
          data:
            (result.data as ProjectInformationConfig | null) ?? DEFAULT_CONFIG,
        };
      },
      providesTags: ["ProjectInformation"],
    }),
    saveProjectInformation: builder.mutation<
      undefined,
      ProjectInformationConfig
    >({
      queryFn: async (config, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/project-information"),
          method: "POST",
          body: config,
        });
        if (result.error) {
          return { error: result.error };
        }
        return { data: undefined };
      },
      invalidatesTags: ["ProjectInformation"],
    }),
    getProjectInformationPreview: builder.mutation<
      ProjectInformationPreviewResponse,
      ProjectInformationConfig
    >({
      queryFn: async (config, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/project-information/preview"),
          method: "POST",
          body: config,
        });
        if (result.error) {
          return { error: result.error };
        }
        return { data: result.data as ProjectInformationPreviewResponse };
      },
    }),
  }),
});

export const {
  useGetProjectInformationQuery,
  useSaveProjectInformationMutation,
  useGetProjectInformationPreviewMutation,
} = projectInformationApi;
