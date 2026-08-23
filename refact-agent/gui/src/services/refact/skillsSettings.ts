import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type SkillsAutoTrigger = "index_only" | "inject_full" | "off";

export interface SkillsSettingsResponse {
  auto_trigger: SkillsAutoTrigger;
}

export const skillsSettingsApi = createApi({
  reducerPath: "skillsSettingsApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  tagTypes: ["SkillsSettings"],
  endpoints: (builder) => ({
    getSkillsSettings: builder.query<SkillsSettingsResponse, undefined>({
      queryFn: async (_arg, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/skills/settings"),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as SkillsSettingsResponse };
      },
      providesTags: ["SkillsSettings"],
    }),
    saveSkillsSettings: builder.mutation<
      SkillsSettingsResponse,
      SkillsSettingsResponse
    >({
      queryFn: async (settings, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/skills/settings"),
          method: "POST",
          body: settings,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as SkillsSettingsResponse };
      },
      invalidatesTags: ["SkillsSettings"],
    }),
  }),
  refetchOnMountOrArgChange: true,
});

export const { useGetSkillsSettingsQuery, useSaveSkillsSettingsMutation } =
  skillsSettingsApi;
