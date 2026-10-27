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

export type GitRefKind = "head" | "local" | "remote" | "tag";

export type GitRef = {
  name: string;
  kind: GitRefKind;
};

export type GitCommitLogEntry = {
  oid: string;
  short_oid: string;
  time_ms: number;
  author_name: string;
  author_email: string;
  message_first_line: string;
  message: string;
  parents?: string[];
  refs?: GitRef[];
};

export type GitLogRoot = {
  root: string;
  commits: GitCommitLogEntry[];
  head_oid?: string | null;
  scan_truncated?: boolean;
};

export type GitBranch = {
  name: string;
  is_head: boolean;
  upstream: string | null;
  tip_oid?: string | null;
  ahead?: number | null;
  behind?: number | null;
  upstream_gone?: boolean;
};

export type GitRemoteBranch = {
  name: string;
  remote: string;
  short_name: string;
  tip_oid: string | null;
};

export type GitTag = {
  name: string;
  target_oid: string | null;
};

export type GitBranchesRoot = {
  root: string;
  current: string | null;
  branches: GitBranch[];
  head_detached?: boolean;
  head_oid?: string | null;
  remotes?: string[];
  remote_branches?: GitRemoteBranch[];
  tags?: GitTag[];
};

export type GitCommitFile = {
  path: string;
  old_path: string | null;
  status: GitFileStatus;
  additions: number;
  deletions: number;
  binary: boolean;
};

export type GitCommitDetails = {
  root: string;
  oid: string;
  short_oid: string;
  parents: string[];
  author_name: string;
  author_email: string;
  author_time_ms: number;
  committer_name: string;
  committer_email: string;
  committer_time_ms: number;
  message: string;
  refs: GitRef[];
  branches: string[];
  /** Set when the engine stopped listing containing branches early. */
  branches_truncated?: boolean;
  files: GitCommitFile[];
  files_truncated: boolean;
};

export type GitCommitDetailsRequest = {
  root: string;
  oid: string;
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

export type GitLogScope = "head" | "all";

export type GitLogRequest = {
  root: string;
  limit: number;
  skip: number;
  scope?: GitLogScope;
  ref?: string;
  text?: string;
  author?: string;
  path?: string;
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
  /** Repo-relative paths restored to HEAD. */
  restored: string[];
  /** Repo-relative added paths removed from the index. */
  unstaged: string[];
  /** Repo-relative paths deleted from disk. */
  deleted: string[];
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
  /**
   * Cache key only (never sent): the HEAD oid the HEAD/INDEX sides were read
   * against, so moving HEAD outside the app refetches them.
   */
  headOid?: string | null;
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

export type GitCheckoutKind = "local" | "remote" | "tag" | "revision";

export type GitCheckoutRequest = {
  root: string;
  name: string;
  kind: GitCheckoutKind;
};

export type GitCheckoutResponse = {
  branch: string | null;
  head_oid: string | null;
  detached: boolean;
};

export type GitCreateBranchRequest = {
  root: string;
  name: string;
  start_point?: string;
  checkout?: boolean;
};

export type GitCreateBranchResponse = {
  name: string;
  tip_oid: string | null;
  checked_out: boolean;
};

export type GitRenameBranchRequest = {
  root: string;
  old_name: string;
  new_name: string;
};

export type GitRenameBranchResponse = {
  name: string;
};

export type GitDeleteBranchRequest = {
  root: string;
  name: string;
  force?: boolean;
};

export type GitDeleteBranchResponse = {
  deleted: string | boolean;
};

export type GitFetchRequest = {
  root: string;
  remote?: string;
  prune?: boolean;
};

export type GitPullRequest = {
  root: string;
};

export type GitPushRequest = {
  root: string;
  remote?: string;
  branch?: string;
  set_upstream?: boolean;
  force_with_lease?: boolean;
};

export type GitRemoteResponse = {
  ok: boolean;
  summary: string;
  output: string;
};

export type GitApiErrorCode =
  | "bad_request"
  | "not_found"
  | "git_error"
  | "conflict"
  | "nothing_to_commit"
  | "hook_failed"
  | "git_unavailable"
  | "timeout"
  | "checkout_conflict"
  | "already_exists"
  | "current_branch"
  | "not_merged"
  | "no_upstream"
  | "upstream_mismatch"
  | "not_fast_forward"
  | "rejected"
  | "auth_failed"
  | "remote_error";

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

function repositoryTags(root: string) {
  return [
    ...worktreeTags(root),
    { type: "GitLog" as const, id: root },
    { type: "GitBranches" as const, id: root },
    { type: "GitCommit" as const, id: root },
  ];
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

export function gitLogQuery({
  root,
  limit,
  skip,
  scope,
  ref,
  text,
  author,
  path,
}: GitLogRequest): URLSearchParams {
  const query = new URLSearchParams({
    root,
    limit: String(limit),
    skip: String(skip),
  });
  const optional: [string, string | undefined][] = [
    ["scope", scope],
    ["ref", ref],
    ["text", text],
    ["author", author],
    ["path", path],
  ];
  for (const [key, value] of optional) {
    if (value) query.set(key, value);
  }
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
  tagTypes: [
    "GitStatus",
    "GitDiff",
    "GitLog",
    "GitBranches",
    "GitCommit",
    "GitFile",
  ],
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
      queryFn: async (request, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/log", gitLogQuery(request)),
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
    getGitCommitDetails: builder.query<
      GitCommitDetails,
      GitCommitDetailsRequest
    >({
      queryFn: async ({ root, oid }, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/commit", new URLSearchParams({ root, oid })),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitCommitDetails };
      },
      providesTags: (_result, _error, { root }) => [
        { type: "GitCommit", id: root },
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
      invalidatesTags: (_result, _error, { root }) => repositoryTags(root),
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
    checkoutGitRef: builder.mutation<GitCheckoutResponse, GitCheckoutRequest>({
      queryFn: async (body, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/branch/checkout"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitCheckoutResponse };
      },
      invalidatesTags: (_result, _error, { root }) => repositoryTags(root),
    }),
    createGitBranch: builder.mutation<
      GitCreateBranchResponse,
      GitCreateBranchRequest
    >({
      queryFn: async (body, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/branch/create"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitCreateBranchResponse };
      },
      invalidatesTags: (_result, _error, { root }) => repositoryTags(root),
    }),
    renameGitBranch: builder.mutation<
      GitRenameBranchResponse,
      GitRenameBranchRequest
    >({
      queryFn: async (body, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/branch/rename"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitRenameBranchResponse };
      },
      invalidatesTags: (_result, _error, { root }) => repositoryTags(root),
    }),
    deleteGitBranch: builder.mutation<
      GitDeleteBranchResponse,
      GitDeleteBranchRequest
    >({
      queryFn: async (body, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/branch/delete"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitDeleteBranchResponse };
      },
      invalidatesTags: (_result, _error, { root }) => repositoryTags(root),
    }),
    fetchGitRemote: builder.mutation<GitRemoteResponse, GitFetchRequest>({
      queryFn: async (body, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/fetch"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitRemoteResponse };
      },
      invalidatesTags: (_result, _error, { root }) => repositoryTags(root),
    }),
    pullGitRemote: builder.mutation<GitRemoteResponse, GitPullRequest>({
      queryFn: async (body, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/pull"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitRemoteResponse };
      },
      invalidatesTags: (_result, _error, { root }) => repositoryTags(root),
    }),
    pushGitRemote: builder.mutation<GitRemoteResponse, GitPushRequest>({
      queryFn: async (body, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: gitUrl(state, "/git/push"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GitRemoteResponse };
      },
      invalidatesTags: (_result, _error, { root }) => repositoryTags(root),
    }),
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
  useGetGitCommitDetailsQuery,
  useStageGitPathsMutation,
  useUnstageGitPathsMutation,
  useCommitGitPathsMutation,
  useRollbackGitPathsMutation,
  useGenerateCommitMessageMutation,
  useCheckoutGitRefMutation,
  useCreateGitBranchMutation,
  useRenameGitBranchMutation,
  useDeleteGitBranchMutation,
  useFetchGitRemoteMutation,
  usePullGitRemoteMutation,
  usePushGitRemoteMutation,
} = gitReadApi;
