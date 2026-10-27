import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type GitFileStatus =
  | "ADDED"
  | "MODIFIED"
  | "DELETED"
  | "RENAMED"
  | "TYPECHANGE";

export type GitFileChange = {
  relative_path: string;
  absolute_path: string;
  status: GitFileStatus;
};

export type GitChange = GitFileChange & {
  old_path: string | null;
  staged: boolean;
  unstaged: boolean;
};

export type GitUntrackedFile = {
  relative_path: string;
  absolute_path: string;
};

export type GitConflictSide = string | boolean | null;

export type GitConflictedFile = {
  relative_path: string;
  absolute_path: string;
  ours: GitConflictSide;
  theirs: GitConflictSide;
  ancestor: GitConflictSide;
};

export type GitRepoState =
  | "clean"
  | "merge"
  | "rebase"
  | "cherry_pick"
  | "revert"
  | "bisect"
  | "apply_mailbox"
  | "other";

export type GitStatusRoot = {
  root: string;
  branch: string | null;
  head_detached: boolean;
  ahead: number | null;
  behind: number | null;
  staged: GitFileChange[];
  unstaged: GitFileChange[];
  untracked_included: boolean;
  head_oid?: string | null;
  upstream?: string | null;
  repo_state?: GitRepoState;
  truncated?: boolean;
  changes?: GitChange[];
  untracked?: GitUntrackedFile[];
  conflicted?: GitConflictedFile[];
};

export type NormalizedGitStatus = {
  root: string;
  branch: string | null;
  headDetached: boolean;
  headOid: string | null;
  upstream: string | null;
  ahead: number | null;
  behind: number | null;
  repoState: GitRepoState;
  truncated: boolean;
  changes: GitChange[];
  untracked: GitUntrackedFile[];
  conflicted: GitConflictedFile[];
  extended: boolean;
};

function legacyChanges(status: GitStatusRoot): GitChange[] {
  const byPath = new Map<string, GitChange>();
  const add = (change: GitFileChange, staged: boolean) => {
    const known = byPath.get(change.relative_path);
    byPath.set(change.relative_path, {
      relative_path: change.relative_path,
      absolute_path: change.absolute_path,
      old_path: null,
      status: known?.status ?? change.status,
      staged: (known?.staged ?? false) || staged,
      unstaged: (known?.unstaged ?? false) || !staged,
    });
  };
  status.staged.forEach((change) => add(change, true));
  status.unstaged.forEach((change) => add(change, false));
  return [...byPath.values()];
}

export function normalizeGitStatus(status: GitStatusRoot): NormalizedGitStatus {
  const extended = Array.isArray(status.changes);
  return {
    root: status.root,
    branch: status.branch,
    headDetached: status.head_detached,
    headOid: status.head_oid ?? null,
    upstream: status.upstream ?? null,
    ahead: status.ahead,
    behind: status.behind,
    repoState: status.repo_state ?? "clean",
    truncated: status.truncated ?? false,
    changes: status.changes ?? legacyChanges(status),
    untracked: status.untracked ?? [],
    conflicted: status.conflicted ?? [],
    extended,
  };
}

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
  paths?: string[];
  staged?: boolean;
  against?: "head";
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

export type GitSkippedPath = {
  path: string;
  reason: string;
};

export type GitCommitRequest = {
  root: string;
  message: string;
  paths: string[];
  amend?: boolean;
};

export type GitCommitResponse = {
  oid: string;
  short_oid: string;
  summary: string;
  used_git_cli: boolean;
  hooks_ran: boolean;
  skipped: GitSkippedPath[];
};

export type GitRollbackRequest = {
  root: string;
  paths: string[];
  delete_added_files?: boolean;
  delete_untracked?: boolean;
};

export type GitRollbackResponse = {
  restored: number;
  unstaged: number;
  deleted: number;
  skipped: GitSkippedPath[];
};

export type GitFileRevision =
  | "HEAD"
  | "INDEX"
  | "WORKTREE"
  | (string & Record<never, never>);

export type GitFileRequest = {
  root: string;
  path: string;
  rev: GitFileRevision;
};

export type GitFileResponse = {
  root: string;
  path: string;
  rev: string;
  exists: boolean;
  binary: boolean;
  truncated: boolean;
  size: number;
  content: string;
};

export type GitApiErrorCode =
  | "bad_request"
  | "not_found"
  | "git_error"
  | "conflict"
  | "nothing_to_commit"
  | "hook_failed"
  | "git_unavailable"
  | "timeout";

export type GitApiError = {
  code: GitApiErrorCode | (string & Record<never, never>);
  error: string;
};

export type GenerateCommitMessageRequest = {
  diff: string;
  text?: string;
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

function fileTag(root: string) {
  return { type: "GitFile" as const, id: root };
}

function worktreeTags(root: string) {
  return [rootTag(root), diffTag(root), fileTag(root)];
}

export function gitDiffQuery({
  root,
  path,
  paths,
  staged,
  against,
}: GitDiffRequest): URLSearchParams {
  const query = new URLSearchParams({ root });
  if (against) query.set("against", against);
  else query.set("staged", String(staged ?? false));
  if (path) query.append("path", path);
  for (const item of paths ?? []) query.append("path", item);
  return query;
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
  tagTypes: ["GitStatus", "GitDiff", "GitLog", "GitBranches", "GitFile"],
  endpoints: (builder) => ({
    getGitStatus: builder.query<GitRootsResponse<GitStatusRoot>, string[]>({
      queryFn: async (roots, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const query =
          roots.length === 1
            ? new URLSearchParams({ root: roots[0] })
            : undefined;
        const result = await baseQuery({
          url: gitUrl(state, "/git/status", query),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitRootsResponse<GitStatusRoot> };
      },
      providesTags: (result, _error, roots) => [
        { type: "GitStatus", id: "LIST" },
        ...[
          ...new Set([
            ...roots,
            ...(result?.roots.map((root) => root.root) ?? []),
          ]),
        ].map(rootTag),
      ],
    }),
    getGitDiff: builder.query<GitRootsResponse<GitDiffRoot>, GitDiffRequest>({
      queryFn: async (request, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/diff", gitDiffQuery(request)),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitRootsResponse<GitDiffRoot> };
      },
      providesTags: (_result, _error, { root }) => [diffTag(root)],
    }),
    getGitFile: builder.query<GitFileResponse, GitFileRequest>({
      queryFn: async ({ root, path, rev }, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(
            state,
            "/git/file",
            new URLSearchParams({ root, path, rev }),
          ),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitFileResponse };
      },
      providesTags: (_result, _error, { root }) => [fileTag(root)],
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
      invalidatesTags: (_result, _error, { root }) => worktreeTags(root),
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
      invalidatesTags: (_result, _error, { root }) => worktreeTags(root),
    }),
    commitGitPaths: builder.mutation<GitCommitResponse, GitCommitRequest>({
      queryFn: async (body, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/commit"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitCommitResponse };
      },
      invalidatesTags: (_result, _error, { root }) => [
        ...worktreeTags(root),
        { type: "GitLog" as const, id: root },
        { type: "GitBranches" as const, id: root },
      ],
    }),
    rollbackGitPaths: builder.mutation<GitRollbackResponse, GitRollbackRequest>(
      {
        queryFn: async (body, api, _options, baseQuery) => {
          const state = api.getState() as RootState;
          const result = await baseQuery({
            url: gitUrl(state, "/git/rollback"),
            method: "POST",
            body,
          });
          if (result.error) return { error: result.error };
          return { data: result.data as GitRollbackResponse };
        },
        invalidatesTags: (_result, _error, { root }) => worktreeTags(root),
      },
    ),
    generateCommitMessage: builder.mutation<
      string,
      GenerateCommitMessageRequest
    >({
      queryFn: async (body, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/commit-message-from-diff"),
          method: "POST",
          body,
          responseHandler: "text",
        });
        if (result.error) return { error: result.error };
        return { data: String(result.data ?? "") };
      },
    }),
  }),
});

export const {
  useGetGitStatusQuery,
  useGetGitDiffQuery,
  useLazyGetGitDiffQuery,
  useGetGitFileQuery,
  useGetGitLogQuery,
  useLazyGetGitLogQuery,
  useGetGitBranchesQuery,
  useStageGitPathsMutation,
  useUnstageGitPathsMutation,
  useCommitGitPathsMutation,
  useRollbackGitPathsMutation,
  useGenerateCommitMessageMutation,
} = gitReadApi;
