import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type TaskMemoryKind =
  | "decision"
  | "spec"
  | "finding"
  | "gotcha"
  | "risk"
  | "handoff"
  | "progress"
  | "postmortem"
  | "brief"
  | "freeform";

export type TaskMemoryStatus = "active" | "archived" | "superseded";

export interface TaskMemoryEntry {
  filename: string;
  created_at: string;
  created_at_known: boolean;
  title: string;
  content: string;
  tags: string[];
  kind: TaskMemoryKind;
  namespace: string;
  pinned: boolean;
  status: TaskMemoryStatus;
  role?: string | null;
  agent_id?: string | null;
  card_id?: string | null;
  supersedes?: string | null;
}

export interface TaskMemoryWarning {
  filename: string;
  error: string;
}

export interface TaskMemoriesResponse {
  task_id: string;
  since: string;
  new_count: number;
  memories: TaskMemoryEntry[];
  warnings: TaskMemoryWarning[];
}

export interface TaskMemoriesQuery {
  taskId: string;
  since?: string;
  kind?: string;
  namespace?: string;
  search?: string;
}

export interface TaskMemoryFacetsResponse {
  task_id: string;
  namespaces: string[];
  tags: string[];
  kinds: string[];
  total_count: number;
  pinned_count: number;
}

export function isTaskMemoryEntry(value: unknown): value is TaskMemoryEntry {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.filename === "string" &&
    typeof v.created_at === "string" &&
    typeof v.title === "string" &&
    typeof v.content === "string" &&
    Array.isArray(v.tags) &&
    v.tags.every((t) => typeof t === "string") &&
    typeof v.kind === "string" &&
    typeof v.namespace === "string" &&
    typeof v.pinned === "boolean"
  );
}

export function isTaskMemoriesResponse(
  value: unknown,
): value is TaskMemoriesResponse {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.task_id === "string" &&
    typeof v.since === "string" &&
    typeof v.new_count === "number" &&
    Array.isArray(v.memories) &&
    Array.isArray(v.warnings)
  );
}

export function isTaskMemoryFacetsResponse(
  value: unknown,
): value is TaskMemoryFacetsResponse {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.task_id === "string" &&
    Array.isArray(v.namespaces) &&
    Array.isArray(v.tags) &&
    Array.isArray(v.kinds) &&
    typeof v.total_count === "number" &&
    typeof v.pinned_count === "number"
  );
}

export interface PinTaskMemoryRequest {
  taskId: string;
  filename: string;
  pinned: boolean;
}

export interface PinTaskMemoryResponse {
  ok: boolean;
  filename: string;
  pinned: boolean;
  changed: boolean;
}

export interface ArchiveTaskMemoryRequest {
  taskId: string;
  filename: string;
}

export interface ArchiveTaskMemoryResponse {
  ok: boolean;
  filename: string;
  archived_filename: string;
}

export interface TriageTaskMemoriesRequest {
  taskId: string;
  cursor?: string;
}

export interface TriageTaskMemoriesResponse {
  ok: boolean;
  cursor: string;
}

function buildTaskMemoriesUrl(
  state: RootState,
  query: TaskMemoriesQuery,
): string {
  const params = new URLSearchParams();
  if (query.since) params.set("since", query.since);
  if (query.kind && query.kind !== "all") params.set("kind", query.kind);
  if (query.namespace && query.namespace !== "all") {
    params.set("namespace", query.namespace);
  }
  if (query.search) params.set("search", query.search);
  const taskId = encodeURIComponent(query.taskId);
  return buildApiUrlFromState(state, `/v1/task/${taskId}/memories`, params);
}

export const taskMemoriesApi = createApi({
  reducerPath: "taskMemoriesApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  tagTypes: ["TaskMemories"],
  endpoints: (builder) => ({
    listTaskMemories: builder.query<TaskMemoriesResponse, TaskMemoriesQuery>({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildTaskMemoriesUrl(state, args),
        });
        if (result.error) return { error: result.error };
        if (!isTaskMemoriesResponse(result.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR" as const,
              error: `Invalid TaskMemoriesResponse shape: ${JSON.stringify(
                result.data,
              ).slice(0, 200)}`,
            },
          };
        }
        return { data: result.data };
      },
      providesTags: (_result, _error, { taskId }) => [
        { type: "TaskMemories", id: taskId },
      ],
    }),

    getTaskMemoryFacets: builder.query<
      TaskMemoryFacetsResponse,
      { taskId: string }
    >({
      queryFn: async ({ taskId }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/task/${encodeURIComponent(taskId)}/memories/facets`,
          ),
        });
        if (result.error) return { error: result.error };
        if (!isTaskMemoryFacetsResponse(result.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR" as const,
              error: `Invalid TaskMemoryFacetsResponse shape: ${JSON.stringify(
                result.data,
              ).slice(0, 200)}`,
            },
          };
        }
        return { data: result.data };
      },
      providesTags: (_result, _error, { taskId }) => [
        { type: "TaskMemories", id: `${taskId}:facets` },
      ],
    }),

    pinTaskMemory: builder.mutation<
      PinTaskMemoryResponse,
      PinTaskMemoryRequest
    >({
      queryFn: async ({ taskId, filename, pinned }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/task/${encodeURIComponent(taskId)}/memories/${encodeURIComponent(
              filename,
            )}/pin`,
          ),
          method: "POST",
          body: { pinned },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as PinTaskMemoryResponse };
      },
      invalidatesTags: (_result, _error, { taskId }) => [
        { type: "TaskMemories", id: taskId },
        { type: "TaskMemories", id: `${taskId}:facets` },
      ],
      async onQueryStarted(
        { taskId, filename, pinned },
        { dispatch, queryFulfilled, getState },
      ) {
        const invalidated = taskMemoriesApi.util.selectInvalidatedBy(
          getState() as RootState,
          [{ type: "TaskMemories", id: taskId }],
        );
        const patches = invalidated
          .filter((entry) => entry.endpointName === "listTaskMemories")
          .map((entry) =>
            dispatch(
              taskMemoriesApi.util.updateQueryData(
                "listTaskMemories",
                entry.originalArgs as TaskMemoriesQuery,
                (draft) => {
                  const memory = draft.memories.find(
                    (m) => m.filename === filename,
                  );
                  if (memory) memory.pinned = pinned;
                },
              ),
            ),
          );
        try {
          await queryFulfilled;
        } catch {
          patches.forEach((p) => p.undo());
        }
      },
    }),

    archiveTaskMemory: builder.mutation<
      ArchiveTaskMemoryResponse,
      ArchiveTaskMemoryRequest
    >({
      queryFn: async ({ taskId, filename }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/task/${encodeURIComponent(taskId)}/memories/${encodeURIComponent(
              filename,
            )}/archive`,
          ),
          method: "POST",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as ArchiveTaskMemoryResponse };
      },
      invalidatesTags: (_result, _error, { taskId }) => [
        { type: "TaskMemories", id: taskId },
        { type: "TaskMemories", id: `${taskId}:facets` },
      ],
      async onQueryStarted(
        { taskId, filename },
        { dispatch, queryFulfilled, getState },
      ) {
        const invalidated = taskMemoriesApi.util.selectInvalidatedBy(
          getState() as RootState,
          [{ type: "TaskMemories", id: taskId }],
        );
        const patches = invalidated
          .filter((entry) => entry.endpointName === "listTaskMemories")
          .map((entry) =>
            dispatch(
              taskMemoriesApi.util.updateQueryData(
                "listTaskMemories",
                entry.originalArgs as TaskMemoriesQuery,
                (draft) => {
                  const idx = draft.memories.findIndex(
                    (m) => m.filename === filename,
                  );
                  if (idx !== -1) draft.memories.splice(idx, 1);
                },
              ),
            ),
          );
        try {
          await queryFulfilled;
        } catch {
          patches.forEach((p) => p.undo());
        }
      },
    }),

    triageTaskMemories: builder.mutation<
      TriageTaskMemoriesResponse,
      TriageTaskMemoriesRequest
    >({
      queryFn: async ({ taskId, cursor }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/task/${encodeURIComponent(taskId)}/memories/triage-done`,
          ),
          method: "POST",
          body: { cursor },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TriageTaskMemoriesResponse };
      },
      invalidatesTags: (_result, _error, { taskId }) => [
        { type: "TaskMemories", id: taskId },
        { type: "TaskMemories", id: `${taskId}:facets` },
      ],
    }),
  }),
});

export const {
  useListTaskMemoriesQuery,
  useGetTaskMemoryFacetsQuery,
  usePinTaskMemoryMutation,
  useArchiveTaskMemoryMutation,
  useTriageTaskMemoriesMutation,
} = taskMemoriesApi;
