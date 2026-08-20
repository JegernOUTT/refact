import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import type { ResolvedPrivacyZone } from "./privacy";

export const FILES_TREE_REQUEST_TIMEOUT_MS = 15_000;

export type FilesTreeEntry = {
  name: string;
  path: string;
  kind: "dir" | "file";
  size: number | null;
  ignored?: boolean;
  privacy_zone: ResolvedPrivacyZone;
};

export type FilesTreeResponse = {
  path: string;
  entries: FilesTreeEntry[];
  truncated: boolean;
};

export type ReadFileRequest = {
  path: string;
  chatId?: string;
  lineStart?: number;
  lineEnd?: number;
  revision?: string;
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

export type WriteFileRequest = {
  path: string;
  content: string;
  expectedMtimeMs?: number;
};

export type WriteFileResponse = {
  path: string;
  size: number;
  mtime_ms: number;
};

export const filesApi = createApi({
  reducerPath: "filesApi",
  tagTypes: ["File", "Tree"],
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
        const controller = new AbortController();
        const timeoutState = { expired: false };
        const abortRequest = () => controller.abort();
        api.signal.addEventListener("abort", abortRequest, { once: true });
        const timeoutId = setTimeout(() => {
          timeoutState.expired = true;
          abortRequest();
        }, FILES_TREE_REQUEST_TIMEOUT_MS);
        let result: Awaited<ReturnType<typeof baseQuery>>;
        try {
          result = await baseQuery({
            url: buildApiUrlFromState(state, "/v1/files/tree", { path }),
            credentials: "same-origin",
            redirect: "follow",
            signal: controller.signal,
          });
        } finally {
          clearTimeout(timeoutId);
          api.signal.removeEventListener("abort", abortRequest);
        }
        if (timeoutState.expired) {
          return {
            error: {
              status: "TIMEOUT_ERROR",
              error: `Workspace files request timed out after ${String(
                FILES_TREE_REQUEST_TIMEOUT_MS,
              )}ms`,
            },
          };
        }
        if (result.error) return { error: result.error };
        return { data: result.data as FilesTreeResponse };
      },
      providesTags: (_result, _error, path) => [{ type: "Tree", id: path }],
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
      providesTags: (_result, _error, request) => [
        { type: "File", id: request.path },
      ],
    }),
    writeFile: builder.mutation<WriteFileResponse, WriteFileRequest>({
      queryFn: async (request, api, _extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/files/write"),
          method: "POST",
          body: {
            path: request.path,
            content: request.content,
            expected_mtime_ms: request.expectedMtimeMs,
          },
          credentials: "same-origin",
          redirect: "follow",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as WriteFileResponse };
      },
      invalidatesTags: (_result, _error, request) => [
        { type: "File", id: request.path },
      ],
    }),
  }),
});

export const { useGetFilesTreeQuery, useReadFileQuery, useWriteFileMutation } =
  filesApi;
