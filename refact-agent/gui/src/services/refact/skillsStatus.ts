import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export interface SkillsStatusResponse {
  skills_available: number;
  skills_included: string[];
  skills_enabled: boolean;
  active_skill: string | null;
}

export const skillsStatusApi = createApi({
  reducerPath: "skillsStatusApi",
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
    getSkillsStatus: builder.query<SkillsStatusResponse, string>({
      queryFn: async (chatId, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/chats/${chatId}/skills-status`,
          ),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as SkillsStatusResponse };
      },
    }),
  }),
});

export const { useGetSkillsStatusQuery } = skillsStatusApi;
