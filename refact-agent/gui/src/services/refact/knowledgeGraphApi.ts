import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import type {
  KnowledgeGraphResponse,
  RelinkMemoriesResponse,
  SuccessResponse,
} from "./types";

export const knowledgeGraphApi = createApi({
  reducerPath: "knowledgeGraphApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  tagTypes: ["KnowledgeGraph", "Memory"],
  endpoints: (builder) => ({
    getKnowledgeGraph: builder.query<
      KnowledgeGraphResponse,
      { includeContent?: boolean } | undefined
    >({
      async queryFn(arg, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const includeContent = arg?.includeContent ?? false;
        const url = buildApiUrlFromState(state, "/v1/knowledge-graph", {
          include_content: includeContent ? 1 : 0,
        });

        const response = await baseQuery({ url });

        if (response.error) {
          return { error: response.error };
        }

        return { data: response.data as KnowledgeGraphResponse };
      },
      providesTags: ["KnowledgeGraph"],
    }),

    updateMemory: builder.mutation<
      SuccessResponse,
      {
        file_path: string;
        title?: string;
        content: string;
        tags: string[];
        kind: string;
        filenames: string[];
      }
    >({
      async queryFn(arg, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, "/v1/knowledge/update-memory");

        const response = await baseQuery({
          url,
          method: "POST",
          body: arg,
        });

        if (response.error) {
          return { error: response.error };
        }

        return { data: response.data as SuccessResponse };
      },
      invalidatesTags: ["KnowledgeGraph", "Memory"],
      async onQueryStarted(_arg, { dispatch, queryFulfilled }) {
        try {
          await queryFulfilled;
          dispatch(knowledgeGraphApi.util.invalidateTags(["KnowledgeGraph"]));
        } catch {
          // Error is handled by RTK Query
        }
      },
    }),

    deleteMemory: builder.mutation<
      SuccessResponse,
      {
        file_path: string;
        archive?: boolean;
      }
    >({
      async queryFn(arg, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, "/v1/knowledge/delete-memory");

        const response = await baseQuery({
          url,
          method: "POST",
          body: arg,
        });

        if (response.error) {
          return { error: response.error };
        }

        return { data: response.data as SuccessResponse };
      },
      invalidatesTags: ["KnowledgeGraph"],
    }),

    relinkMemories: builder.mutation<RelinkMemoriesResponse, undefined>({
      async queryFn(_arg, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          "/v1/knowledge/relink-memories",
        );

        const response = await baseQuery({ url, method: "POST" });

        if (response.error) {
          return { error: response.error };
        }

        return { data: response.data as RelinkMemoriesResponse };
      },
      invalidatesTags: ["KnowledgeGraph", "Memory"],
    }),
  }),
});

export const {
  useGetKnowledgeGraphQuery,
  useUpdateMemoryMutation,
  useDeleteMemoryMutation,
  useRelinkMemoriesMutation,
} = knowledgeGraphApi;
