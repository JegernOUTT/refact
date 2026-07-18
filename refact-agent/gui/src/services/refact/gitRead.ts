import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type GitFileStatus = "ADDED" | "MODIFIED" | "DELETED";

export type GitFileChange = {
  relative_path: string;
  absolute_path: string;
  status: GitFileStatus;
};

export type GitStatusRoot = {
  root: string;
  branch: string | null;
  head_detached: boolean;
  ahead: number | null;
  behind: number | null;
  staged: GitFileChange[];
  unstaged: GitFileChange[];
  untracked_included: boolean;
};

export type GitDiffRoot = {
  root: string;
  patch: string;
  truncated: boolean;
};

export type GitCommitLogEntry = {
  oid: string;
  short_oid: string;
  time_ms: number;
  author_name: string;
  author_email: string;
  message_first_line: string;
  message: string;
};

export type GitLogRoot = {
  root: string;
  commits: GitCommitLogEntry[];
};

export type GitBranch = {
  name: string;
  is_head: boolean;
  upstream: string | null;
};

export type GitBranchesRoot = {
  root: string;
  current: string | null;
  branches: GitBranch[];
};

export type GitRootsResponse<T> = {
  roots: T[];
};

export type GitDiffRequest = {
  root: string;
  path?: string;
  staged?: boolean;
};

export type GitLogRequest = {
  root: string;
  limit: number;
  skip: number;
};

export type GitPathsRequest = {
  root: string;
  paths: string[];
};

export type GitStageResponse = {
  staged: number;
  skipped: number;
};

export type GitUnstageResponse = {
  unstaged: number;
};

export type GitCommitChange = {
  relative_path: string;
  absolute_path: string;
  status: GitFileStatus;
};

export type GitCommitRequest = {
  commits: {
    project_path: string;
    commit_message: string;
    staged_changes: GitCommitChange[];
    unstaged_changes: GitCommitChange[];
  }[];
};

export type GitCommitResponse = {
  commits_applied: {
    project_path: string;
    project_name: string;
    commit_oid: string;
  }[];
  error_log: {
    error_message: string;
    project_path: string;
    project_name: string;
  }[];
};

export type GitCommitMutationRequest = {
  root: string;
  body: GitCommitRequest;
};

function gitUrl(
  state: RootState,
  path: string,
  query?: URLSearchParams,
): string {
  return buildApiUrlFromState(state, `/v1${path}`, query);
}

function rootTag(root: string) {
  return { type: "GitStatus" as const, id: root };
}

function diffTag(root: string) {
  return { type: "GitDiff" as const, id: root };
}

export const gitReadApi = createApi({
  reducerPath: "gitReadApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  tagTypes: ["GitStatus", "GitDiff", "GitLog", "GitBranches"],
  endpoints: (builder) => ({
    getGitStatus: builder.query<GitRootsResponse<GitStatusRoot>, string[]>({
      queryFn: async (_arg, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({ url: gitUrl(state, "/git/status") });
        if (result.error) return { error: result.error };
        return { data: result.data as GitRootsResponse<GitStatusRoot> };
      },
      serializeQueryArgs: ({ endpointName }) => endpointName,
      forceRefetch: ({ currentArg, previousArg }) =>
        currentArg?.join("\n") !== previousArg?.join("\n"),
      providesTags: (result) => [
        { type: "GitStatus", id: "LIST" },
        ...(result?.roots.map((root) => rootTag(root.root)) ?? []),
      ],
    }),
    getGitDiff: builder.query<GitRootsResponse<GitDiffRoot>, GitDiffRequest>({
      queryFn: async ({ root, path, staged }, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const query = new URLSearchParams({
          root,
          staged: String(staged ?? false),
        });
        if (path) query.set("path", path);
        const result = await baseQuery({
          url: gitUrl(state, "/git/diff", query),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitRootsResponse<GitDiffRoot> };
      },
      providesTags: (_result, _error, { root }) => [diffTag(root)],
    }),
    getGitLog: builder.query<GitRootsResponse<GitLogRoot>, GitLogRequest>({
      queryFn: async ({ root, limit, skip }, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(
            state,
            "/git/log",
            new URLSearchParams({
              root,
              limit: String(limit),
              skip: String(skip),
            }),
          ),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitRootsResponse<GitLogRoot> };
      },
      providesTags: (_result, _error, { root }) => [
        { type: "GitLog", id: root },
      ],
    }),
    getGitBranches: builder.query<GitRootsResponse<GitBranchesRoot>, string>({
      queryFn: async (root, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/branches", new URLSearchParams({ root })),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitRootsResponse<GitBranchesRoot> };
      },
      providesTags: (_result, _error, root) => [
        { type: "GitBranches", id: root },
      ],
    }),
    stageGitPaths: builder.mutation<GitStageResponse, GitPathsRequest>({
      queryFn: async (body, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/stage"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitStageResponse };
      },
      invalidatesTags: (_result, _error, { root }) => [
        rootTag(root),
        diffTag(root),
      ],
    }),
    unstageGitPaths: builder.mutation<GitUnstageResponse, GitPathsRequest>({
      queryFn: async (body, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/unstage"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitUnstageResponse };
      },
      invalidatesTags: (_result, _error, { root }) => [
        rootTag(root),
        diffTag(root),
      ],
    }),
    commitGitChanges: builder.mutation<
      GitCommitResponse,
      GitCommitMutationRequest
    >({
      queryFn: async ({ body }, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git-commit"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitCommitResponse };
      },
      invalidatesTags: (_result, _error, { root }) => [
        rootTag(root),
        diffTag(root),
        { type: "GitLog" as const, id: root },
        { type: "GitBranches" as const, id: root },
      ],
    }),
  }),
});

export const {
  useGetGitStatusQuery,
  useGetGitDiffQuery,
  useGetGitLogQuery,
  useGetGitBranchesQuery,
  useStageGitPathsMutation,
  useUnstageGitPathsMutation,
  useCommitGitChangesMutation,
} = gitReadApi;
