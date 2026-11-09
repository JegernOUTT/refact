import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { buildApiUrlFromState, type QueryParams } from "./apiUrl";

export type WorktreeLifecycleState =
  | "active"
  | "stale"
  | "deleted"
  | "missing"
  | "conflicted";

export type WorktreeMeta = {
  id: string;
  kind: string;
  root: string;
  source_workspace_root: string;
  repo_root: string;
  branch?: string | null;
  base_branch?: string | null;
  base_commit?: string | null;
  task_id?: string | null;
  card_id?: string | null;
  agent_id?: string | null;
  enforce: boolean;
  reference_count?: number;
  referencing_chat_ids?: string[];
  affected_chat_ids?: string[];
  lifecycle_state?: WorktreeLifecycleState;
  stale?: boolean;
  deleted?: boolean;
  status?: WorktreeStatus | null;
};

export type WorktreeReference = {
  kind: string;
  chat_id?: string | null;
  task_id?: string | null;
  card_id?: string | null;
  agent_id?: string | null;
};

export type WorktreeStatus = {
  path_exists: boolean;
  is_git_worktree: boolean;
  dirty: boolean;
  staged_count: number;
  unstaged_count: number;
  untracked_count: number;
  branch?: string | null;
  head_commit?: string | null;
  error?: string | null;
  lifecycle_state?: WorktreeLifecycleState;
  stale?: boolean;
  deleted?: boolean;
  conflicted?: boolean;
};

export type WorktreeRecordView = {
  meta: WorktreeMeta;
  created_at: string;
  updated_at: string;
  last_seen_at?: string | null;
  references: WorktreeReference[];
  reference_count: number;
  referencing_chat_ids?: string[];
  status: WorktreeStatus;
};

export type WorktreeListResponse = {
  project_hash: string;
  source_workspace_root: string;
  source_current_branch?: string | null;
  source_branches?: string[];
  worktrees: WorktreeRecordView[];
};

export type ListWorktreesRequest = {
  source_workspace_root?: string;
  include_branches?: boolean;
};

export type CreateWorktreeRequest = {
  source_workspace_root?: string;
  branch?: string;
  base_branch?: string;
  base_commit?: string;
  chat_id?: string;
  kind?: string;
  task_id?: string;
  card_id?: string;
  agent_id?: string;
};

export type CreateWorktreeResponse = {
  worktree: WorktreeRecordView;
  branch_was_created: boolean;
  dirty_source_warning: boolean;
  warnings: string[];
};

export type GetWorktreeRequest = {
  id: string;
  source_workspace_root?: string;
};

export type WorktreeDiffFile = {
  path: string;
  old_path?: string | null;
  status: string;
  source: string;
  additions?: number | null;
  deletions?: number | null;
};

export type WorktreeDiffStats = {
  committed_files: number;
  staged_files: number;
  unstaged_files: number;
  untracked_files: number;
  files_changed: number;
  additions?: number;
  deletions?: number;
};

export type WorktreeDiffResponse = {
  id: string;
  resolved_base_commit?: string | null;
  branch?: string | null;
  base_branch?: string | null;
  base_commit?: string | null;
  ahead?: number | null;
  behind?: number | null;
  status: WorktreeStatus;
  files: WorktreeDiffFile[];
  stats: WorktreeDiffStats;
  patch: string;
  patch_truncated: boolean;
  patch_shown_bytes?: number;
  patch_total_bytes?: number;
};

export type GetWorktreeDiffRequest = GetWorktreeRequest & {
  max_patch_bytes?: number;
};

export type WorktreeMergeStrategy = "merge" | "squash";

export type MergeWorktreeRequest = {
  id: string;
  source_workspace_root?: string;
  strategy?: WorktreeMergeStrategy;
  target_branch?: string;
  delete_after_merge?: boolean;
  include_uncommitted?: boolean;
  acknowledge_dirty_baseline?: boolean;
  commit_message?: string;
  generate_commit_message?: boolean;
};

export type WorktreeRemovalResult = {
  worktree_deleted: boolean;
  branch_deleted: boolean;
  registry_deleted: boolean;
  stale_path: boolean;
  warnings: string[];
};

export type WorktreeConflictState = {
  files: string[];
  aborted: boolean;
  merge_in_progress: boolean;
  instructions: string;
};

export type MergeWorktreeResponse = {
  id?: string;
  status?: string;
  merged?: boolean;
  strategy?: string;
  source_branch?: string;
  target_branch?: string;
  committed_uncommitted?: string | null;
  merge_commit?: string | null;
  cleanup?: WorktreeRemovalResult | null;
  conflict?: WorktreeConflictState | null;
  success?: boolean;
  message?: string;
  has_conflicts?: boolean;
  conflicted?: boolean;
  conflict_files?: string[];
  worktree?: WorktreeRecordView | null;
  deleted?: boolean;
  branch_deleted?: boolean;
  affected_references?: WorktreeReference[];
  affected_reference_count?: number;
  affected_chat_ids?: string[];
  warnings?: string[];
};

export type WorktreeSyncResolutionChoice =
  | "working_tree"
  | "source"
  | "target"
  | "delete";

export type WorktreeSyncResolution = {
  path: string;
  choice: WorktreeSyncResolutionChoice;
};

export type WorktreeSyncRequest =
  | { action: "status"; target_branch?: string }
  | {
      action: "start";
      target_branch?: string;
      expected_source_head: string;
      expected_target_head: string;
      checkpoint?: boolean;
    }
  | {
      action: "resolve";
      operation_id: string;
      expected_revision: number;
      resolutions: WorktreeSyncResolution[];
    }
  | {
      action: "continue";
      operation_id: string;
      expected_revision: number;
      commit_message?: string;
    }
  | {
      action: "restore";
      archive_id?: string;
      operation_id: string;
      expected_revision: number;
    }
  | {
      action: "recover";
      expected_journal_digest: string;
    }
  | {
      action: "abort";
      operation_id: string;
      expected_revision: number;
      force?: boolean;
    };

export type WorktreeSyncStatus =
  | "idle"
  | "conflicts"
  | "ready"
  | "completed"
  | "aborted"
  | "recovery_required"
  | "target_advanced";

export type WorktreeSyncOperation = {
  operation_id: string;
  revision: number;
  source_head: string;
  target_branch: string;
  target_head: string;
  phase: string;
  checkpoint_commit?: string | null;
  recovery_path?: string | null;
  initiator?: string | null;
};

export type WorktreeSyncStage = {
  oid: string;
  mode: number;
};

export type WorktreeSyncConflict = {
  path: string;
  kind: string;
  base?: WorktreeSyncStage | null;
  source?: WorktreeSyncStage | null;
  target?: WorktreeSyncStage | null;
};

export type WorktreeWriterBlocker = {
  kind:
    | "agent_pending"
    | "execution_active"
    | "board_admitted"
    | "chat_commands_queued"
    | "chat_active";
  chat_id?: string;
  agent_id?: string;
  run_generation?: number;
  recovery_actions: (
    | "wait"
    | "inspect_agent"
    | "cancel_agent"
    | "stop_chat"
    | "user_required"
    | "retry_status"
  )[];
};

export type WorktreeSyncResponse = {
  id: string;
  status: WorktreeSyncStatus;
  source_head: string;
  target_branch?: string | null;
  target_head?: string | null;
  operation?: WorktreeSyncOperation | null;
  conflicts: WorktreeSyncConflict[];
  warnings: string[];
  instructions: string;
  recovery_journal_digest?: string | null;
  restore_archive_id?: string | null;
  recovery_archives?: { archive_id: string; path: string }[];
  writer_readiness?: {
    ready: boolean;
    blockers: WorktreeWriterBlocker[];
    truncated: boolean;
  };
};

export type GetWorktreeSyncStatusRequest = GetWorktreeRequest & {
  target_branch?: string;
};

export type SyncWorktreeRequest = {
  id: string;
  source_workspace_root?: string;
  request: WorktreeSyncRequest;
};

export type DeleteWorktreeRequest = {
  id: string;
  source_workspace_root?: string;
  delete_branch?: boolean;
  force_referenced?: boolean;
};

export type DeleteWorktreeResponse = {
  deleted: boolean;
  branch_deleted: boolean;
  stale_path: boolean;
  affected_references: WorktreeReference[];
  affected_reference_count: number;
  affected_chat_ids?: string[];
  warnings: string[];
};

export type OpenWorktreeRequest = GetWorktreeRequest;

export type OpenWorktreeResponse = {
  id: string;
  path: string;
  branch?: string | null;
  can_open_folder: boolean;
};

export type WorktreeInventorySummary = {
  total_registered: number;
  total_discovered: number;
  total: number;
  clean: number;
  dirty: number;
  unknown: number;
  stale: number;
  conflicted: number;
  shared: number;
  abandoned_clean: number;
  changed_files: number;
  additions: number;
  deletions: number;
  missing_registry_paths: number;
  unregistered_cache_dirs: number;
  merged_branches: number;
  newest_age_hours?: number | null;
  oldest_age_hours?: number | null;
  disk_usage_bytes?: number | null;
};

export type WorktreeInspection = {
  id: string;
  source: string;
  root: string;
  branch?: string | null;
  base_branch?: string | null;
  base_commit?: string | null;
  status: WorktreeStatus;
  references: WorktreeReference[];
  reference_count: number;
  shared: boolean;
  stale: boolean;
  conflicted: boolean;
  changed_files: number;
  committed_files: number;
  staged_files: number;
  unstaged_files: number;
  untracked_files: number;
  additions: number;
  deletions: number;
  cleanup_candidate: boolean;
  cleanup_blockers: string[];
  disk_usage_bytes?: number | null;
  age_hours?: number | null;
  last_used_at?: string | null;
  branch_merged?: boolean | null;
  registry_missing: boolean;
  cache_dir_missing_from_registry: boolean;
  attached_chat_ids: string[];
  attached_task_ids: string[];
};

export type WorktreeInventory = {
  project_hash: string;
  source_workspace_root: string;
  generated_at: string;
  summary: WorktreeInventorySummary;
  worktrees: WorktreeInspection[];
  cleanup_candidates: string[];
};

export type WorktreeCleanupRequest = {
  ids: string[];
  source_workspace_root?: string;
  clean_only?: boolean;
  delete_branches?: boolean;
  allow_shared?: boolean;
  min_age_hours?: number;
};

export type WorktreeCleanupTarget = {
  id: string;
  root: string;
  branch?: string | null;
  shared: boolean;
  stale: boolean;
  changed_files: number;
  additions: number;
  deletions: number;
  delete_branch: boolean;
  references: WorktreeReference[];
  disk_usage_bytes?: number | null;
};

export type WorktreeCleanupSkipped = {
  id: string;
  root?: string | null;
  reason: string;
  details: string[];
};

export type WorktreeCleanupDeleted = {
  id: string;
  root: string;
  branch?: string | null;
  worktree_deleted: boolean;
  branch_deleted: boolean;
  registry_deleted: boolean;
  stale_path: boolean;
  warnings: string[];
};

export type WorktreeCleanupPlan = {
  generated_at: string;
  request: WorktreeCleanupRequest;
  candidates: WorktreeCleanupTarget[];
  skipped: WorktreeCleanupSkipped[];
};

export type WorktreeCleanupResult = {
  generated_at: string;
  request: WorktreeCleanupRequest;
  deleted: WorktreeCleanupDeleted[];
  skipped: WorktreeCleanupSkipped[];
  warnings: string[];
};

function buildWorktreeUrl(
  state: RootState,
  path: string,
  query?: QueryParams,
): string {
  return buildApiUrlFromState(state, `/v1${path}`, query);
}

export const worktreesApi = createApi({
  reducerPath: "worktreesApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  tagTypes: ["Worktrees"],
  endpoints: (builder) => ({
    listWorktrees: builder.query<
      WorktreeListResponse,
      ListWorktreesRequest | undefined
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildWorktreeUrl(state, "/worktrees", {
            source_workspace_root: args?.source_workspace_root,
            include_branches: args?.include_branches === true,
          }),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as WorktreeListResponse };
      },
      serializeQueryArgs: ({ endpointName, queryArgs }) =>
        `${endpointName}:${JSON.stringify({
          source_workspace_root: queryArgs?.source_workspace_root ?? null,
          include_branches: queryArgs?.include_branches === true,
        })}`,
      providesTags: (result) =>
        result
          ? [
              { type: "Worktrees", id: "LIST" },
              ...result.worktrees.map((worktree) => ({
                type: "Worktrees" as const,
                id: worktree.meta.id,
              })),
            ]
          : [{ type: "Worktrees", id: "LIST" }],
    }),

    getWorktreesSummary: builder.query<
      WorktreeInventory,
      { source_workspace_root?: string } | undefined
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildWorktreeUrl(state, "/worktrees/summary", {
            source_workspace_root: args?.source_workspace_root,
          }),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as WorktreeInventory };
      },
      providesTags: [{ type: "Worktrees", id: "SUMMARY" }],
    }),
    cleanupWorktreesDryRun: builder.mutation<
      WorktreeCleanupPlan,
      WorktreeCleanupRequest
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildWorktreeUrl(state, "/worktrees/cleanup-dry-run"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as WorktreeCleanupPlan };
      },
    }),
    cleanupWorktrees: builder.mutation<
      WorktreeCleanupResult,
      WorktreeCleanupRequest
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildWorktreeUrl(state, "/worktrees/cleanup"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as WorktreeCleanupResult };
      },
      invalidatesTags: [
        { type: "Worktrees", id: "LIST" },
        { type: "Worktrees", id: "SUMMARY" },
      ],
    }),
    createWorktree: builder.mutation<
      CreateWorktreeResponse,
      CreateWorktreeRequest
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildWorktreeUrl(state, "/worktrees"),
          method: "POST",
          body: args,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as CreateWorktreeResponse };
      },
      invalidatesTags: ["Worktrees"],
    }),
    getWorktree: builder.query<WorktreeRecordView, GetWorktreeRequest>({
      queryFn: async ({ id, source_workspace_root }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildWorktreeUrl(state, `/worktrees/${encodeURIComponent(id)}`, {
            source_workspace_root,
          }),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as WorktreeRecordView };
      },
      providesTags: (_result, _error, { id }) => [{ type: "Worktrees", id }],
    }),
    getWorktreeDiff: builder.query<
      WorktreeDiffResponse,
      GetWorktreeDiffRequest
    >({
      queryFn: async (
        { id, source_workspace_root, max_patch_bytes },
        api,
        _opts,
        baseQuery,
      ) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildWorktreeUrl(
            state,
            `/worktrees/${encodeURIComponent(id)}/diff`,
            {
              source_workspace_root,
              max_patch_bytes,
            },
          ),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as WorktreeDiffResponse };
      },
      providesTags: (_result, _error, { id }) => [{ type: "Worktrees", id }],
    }),
    mergeWorktree: builder.mutation<
      MergeWorktreeResponse,
      MergeWorktreeRequest
    >({
      queryFn: async (
        { id, source_workspace_root, ...body },
        api,
        _opts,
        baseQuery,
      ) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildWorktreeUrl(
            state,
            `/worktrees/${encodeURIComponent(id)}/merge`,
            {
              source_workspace_root,
            },
          ),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as MergeWorktreeResponse };
      },
      invalidatesTags: (_result, _error, { id }) => [
        { type: "Worktrees", id },
        { type: "Worktrees", id: "LIST" },
        { type: "Worktrees", id: "SUMMARY" },
      ],
    }),
    getWorktreeSyncStatus: builder.query<
      WorktreeSyncResponse,
      GetWorktreeSyncStatusRequest
    >({
      queryFn: async (
        { id, source_workspace_root, target_branch },
        api,
        _opts,
        baseQuery,
      ) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildWorktreeUrl(
            state,
            `/worktrees/${encodeURIComponent(id)}/sync`,
            { source_workspace_root },
          ),
          method: "POST",
          body: {
            action: "status",
            target_branch,
          } satisfies WorktreeSyncRequest,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as WorktreeSyncResponse };
      },
      providesTags: (_result, _error, { id }) => [
        { type: "Worktrees", id: `SYNC:${id}` },
      ],
    }),
    syncWorktree: builder.mutation<WorktreeSyncResponse, SyncWorktreeRequest>({
      queryFn: async (
        { id, source_workspace_root, request },
        api,
        _opts,
        baseQuery,
      ) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildWorktreeUrl(
            state,
            `/worktrees/${encodeURIComponent(id)}/sync`,
            { source_workspace_root },
          ),
          method: "POST",
          body: request,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as WorktreeSyncResponse };
      },
      invalidatesTags: (_result, _error, { id }) => [
        { type: "Worktrees", id },
        { type: "Worktrees", id: `SYNC:${id}` },
        { type: "Worktrees", id: "LIST" },
        { type: "Worktrees", id: "SUMMARY" },
      ],
    }),
    deleteWorktree: builder.mutation<
      DeleteWorktreeResponse,
      DeleteWorktreeRequest
    >({
      queryFn: async (
        { id, source_workspace_root, delete_branch, force_referenced },
        api,
        _opts,
        baseQuery,
      ) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildWorktreeUrl(state, `/worktrees/${encodeURIComponent(id)}`, {
            source_workspace_root,
            delete_branch,
            force_referenced,
          }),
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as DeleteWorktreeResponse };
      },
      invalidatesTags: (_result, _error, { id }) => [
        { type: "Worktrees", id },
        { type: "Worktrees", id: "LIST" },
        { type: "Worktrees", id: "SUMMARY" },
      ],
    }),
    openWorktree: builder.mutation<OpenWorktreeResponse, OpenWorktreeRequest>({
      queryFn: async ({ id, source_workspace_root }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildWorktreeUrl(
            state,
            `/worktrees/${encodeURIComponent(id)}/open`,
            {
              source_workspace_root,
            },
          ),
          method: "POST",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as OpenWorktreeResponse };
      },
    }),
  }),
});

export const {
  useListWorktreesQuery,
  useGetWorktreesSummaryQuery,
  useCleanupWorktreesDryRunMutation,
  useCleanupWorktreesMutation,
  useCreateWorktreeMutation,
  useGetWorktreeQuery,
  useGetWorktreeDiffQuery,
  useMergeWorktreeMutation,
  useGetWorktreeSyncStatusQuery,
  useSyncWorktreeMutation,
  useDeleteWorktreeMutation,
  useOpenWorktreeMutation,
} = worktreesApi;
