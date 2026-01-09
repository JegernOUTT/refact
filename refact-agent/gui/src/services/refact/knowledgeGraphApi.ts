import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import type {
  KnowledgeGraphResponse,
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
    getKnowledgeGraph: builder.query<KnowledgeGraphResponse, void>({
      async queryFn(_arg, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}/v1/knowledge-graph`;

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
        file_path: string; // path to .md file
        title?: string;
        content: string;
        tags: string[];
        kind: string;
        filenames: string[];
      }
    >({
      async queryFn(arg, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}/v1/knowledge/update-memory`;

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
        // Optimistic update: refetch graph after success
        try {
          await queryFulfilled;
          dispatch(knowledgeGraphApi.util.invalidateTags(["KnowledgeGraph"]));
        } catch (err) {
          console.error("Failed to update memory", err);
        }
      },
    }),

    deleteMemory: builder.mutation<
      SuccessResponse,
      {
        file_path: string;
        archive?: boolean; // true = move to archive, false = permanent delete
      }
    >({
      async queryFn(arg, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}/v1/knowledge/delete-memory`;

        const response = await baseQuery({
          url,
          method: "DELETE",
          body: arg,
        });

        if (response.error) {
          return { error: response.error };
        }

        return { data: response.data as SuccessResponse };
      },
      invalidatesTags: ["KnowledgeGraph"],
    }),
  }),
});

export const {
  useGetKnowledgeGraphQuery,
  useUpdateMemoryMutation,
  useDeleteMemoryMutation,
} = knowledgeGraphApi;
