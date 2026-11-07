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

export type FilesSort = "name" | "type" | "size";
export type FilesDirection = "asc" | "desc";
export type FilesSearchEntry = FilesTreeEntry & {
  root: string;
  relative_path: string;
};
export type FilesSearchResponse = {
  entries: FilesSearchEntry[];
  truncated: boolean;
  index_state: "cold" | "building" | "ready" | "degraded";
  complete: boolean;
};
export type FilesSearchRequest = {
  q: string;
  root?: string;
  limit: number;
  sort: FilesSort;
  direction: FilesDirection;
  show_ignored: boolean;
  projectIdentity: string;
};

export type ReadFileRequest = {
  path: string;
  chatId?: string;
  lineStart?: number;
  lineEnd?: number;
  revision?: string;
  manual?: boolean;
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
  digest?: string;
  can_edit?: boolean;
  image_mime?: string | null;
  image_data?: string | null;
};

export type WriteFileRequest = {
  path: string;
  content: string;
  expectedMtimeMs?: number;
  expectedDigest?: string;
};

export type WriteFileResponse = {
  path: string;
  size: number;
  mtime_ms: number;
};

export const PREVIEW_IMAGE_MIME_TYPES = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
]);

export const MAX_PREVIEW_IMAGE_BYTES = 10 * 1024 * 1024;

const BASE64_PATTERN = /^[A-Za-z0-9+/]+={0,2}$/u;

export const imageDataUrl = (
  mime: string | null | undefined,
  data: string | null | undefined,
): string | null => {
  if (!mime || !data) return null;
  const normalizedMime = mime.trim().toLowerCase();
  if (!PREVIEW_IMAGE_MIME_TYPES.has(normalizedMime)) return null;
  if (data.length % 4 !== 0 || !BASE64_PATTERN.test(data)) return null;
  if ((data.length / 4) * 3 > MAX_PREVIEW_IMAGE_BYTES + 2) return null;
  return `data:${normalizedMime};base64,${data}`;
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
    searchFiles: builder.query<FilesSearchResponse, FilesSearchRequest>({
      queryFn: async (
        { projectIdentity: _identity, ...params },
        api,
        _options,
        baseQuery,
      ) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            "/v1/files/search",
            params,
          ),
          credentials: "same-origin",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as FilesSearchResponse };
      },
    }),
    getFilesTree: builder.query<
      FilesTreeResponse,
      string | { path: string; manual: true }
    >({
      queryFn: async (request, api, _extraOptions, baseQuery) => {
        const path = typeof request === "string" ? request : request.path;
        const manual = typeof request !== "string";
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
            url: buildApiUrlFromState(
              state,
              manual ? "/v1/files/browse" : "/v1/files/tree",
              manual ? undefined : { path },
            ),
            method: manual ? "POST" : "GET",
            body: manual ? { path } : undefined,
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
      providesTags: (_result, _error, request) => [
        {
          type: "Tree",
          id: typeof request === "string" ? request : request.path,
        },
      ],
    }),
    readFile: builder.query<ReadFileResponse, ReadFileRequest>({
      queryFn: async (request, api, _extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery(
          request.manual
            ? {
                url: buildApiUrlFromState(state, "/v1/files/open"),
                method: "POST",
                body: { path: request.path },
                credentials: "same-origin",
                redirect: "follow",
              }
            : {
                url: buildApiUrlFromState(state, "/v1/files/read", {
                  path: request.path,
                  line_start: request.lineStart,
                  line_end: request.lineEnd,
                }),
                credentials: "same-origin",
                redirect: "follow",
              },
        );
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
            expected_digest: request.expectedDigest,
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
