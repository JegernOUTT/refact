import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";

export interface SkillEntry {
  name: string;
  description?: string;
  author?: string;
  repo?: string;
  stars?: number;
  forks?: number;
  updated?: string;
  marketplace?: boolean;
}

export interface SkillsSearchData {
  skills: SkillEntry[];
  total?: number;
  page?: number;
}

export interface SkillsRateLimit {
  daily_limit: number;
  daily_remaining: number;
}

export interface SkillsSearchResponse {
  data: SkillsSearchData;
  ratelimit?: SkillsRateLimit;
}

export interface SearchParams {
  q: string;
  page?: number;
  limit?: number;
  sort_by?: "stars" | "recent";
  apiKey: string;
}

export interface AiSearchParams {
  q: string;
  apiKey: string;
}

export const skillsmpApi = createApi({
  reducerPath: "skillsmpApi",
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
    searchSkills: builder.query<SkillsSearchResponse, SearchParams>({
      queryFn: async (params, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const searchParams = new URLSearchParams();
        searchParams.set("q", params.q);
        if (params.page !== undefined) searchParams.set("page", String(params.page));
        if (params.limit !== undefined) searchParams.set("limit", String(params.limit));
        if (params.sort_by) searchParams.set("sort_by", params.sort_by);
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/skillsmp/search?${searchParams.toString()}`,
          headers: { "X-SkillsMP-Api-Key": params.apiKey },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as SkillsSearchResponse };
      },
    }),

    aiSearchSkills: builder.query<SkillsSearchResponse, AiSearchParams>({
      queryFn: async (params, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }
        const searchParams = new URLSearchParams();
        searchParams.set("q", params.q);
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/skillsmp/ai-search?${searchParams.toString()}`,
          headers: { "X-SkillsMP-Api-Key": params.apiKey },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as SkillsSearchResponse };
      },
    }),
  }),
});

export const { useSearchSkillsQuery, useAiSearchSkillsQuery } = skillsmpApi;
