import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type FilesTreeEntry = {
  name: string;
  path: string;
  kind: "dir" | "file";
  size: number | null;
};

export type FilesTreeResponse = {
  path: string;
  entries: FilesTreeEntry[];
  truncated: boolean;
};

export type ReadFileRequest = {
  path: string;
  lineStart?: number;
  lineEnd?: number;
};

export type ReadFileResponse = {
  path: string;
  content: string;
  language: string | null;
  size: number;
  truncated: boolean;
  line_start: number | null;
  line_end: number | null;
  mtime_ms: number;
  binary?: boolean;
};

export const filesApi = createApi({
  reducerPath: "filesApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  endpoints: (builder) => ({
    getFilesTree: builder.query<FilesTreeResponse, string>({
      queryFn: async (path, api, _extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/files/tree", { path }),
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as FilesTreeResponse };
      },
    }),
    readFile: builder.query<ReadFileResponse, ReadFileRequest>({
      queryFn: async (request, api, _extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/files/read", {
            path: request.path,
            line_start: request.lineStart,
            line_end: request.lineEnd,
          }),
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as ReadFileResponse };
      },
    }),
  }),
});

export const { useGetFilesTreeQuery, useReadFileQuery } = filesApi;
