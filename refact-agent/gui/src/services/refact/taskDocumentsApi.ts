import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";

export type TaskDocumentKind =
  | "plan"
  | "design"
  | "runbook"
  | "brief"
  | "postmortem"
  | "spec";

export interface TaskDocumentSummary {
  slug: string;
  name: string;
  kind: TaskDocumentKind;
  pinned: boolean;
  version: number;
  updated_at: string;
  created_at: string;
  author_role: string;
  relevant_cards: string[];
}

export interface TaskDocumentListResponse {
  task_id: string;
  documents: TaskDocumentSummary[];
}

export interface TaskDocumentDetail {
  slug: string;
  name: string;
  kind: TaskDocumentKind;
  pinned: boolean;
  version: number;
  content: string;
  created_at: string;
  updated_at: string;
  author_role: string;
  relevant_cards: string[];
}

export interface TaskDocumentHistoryEntry {
  version: number;
  updated_at: string;
  author_role: string;
  size_bytes: number;
}

export interface TaskDocumentHistoryResponse {
  task_id: string;
  slug: string;
  history: TaskDocumentHistoryEntry[];
}

export interface CreateTaskDocumentRequest {
  taskId: string;
  slug: string;
  name: string;
  kind: TaskDocumentKind;
  content: string;
  pinned?: boolean;
}

export interface UpdateTaskDocumentRequest {
  taskId: string;
  slug: string;
  content: string;
}

export interface DeleteTaskDocumentRequest {
  taskId: string;
  slug: string;
}

export interface DeleteTaskDocumentResponse {
  ok: boolean;
}

export interface PinTaskDocumentRequest {
  taskId: string;
  slug: string;
  pinned: boolean;
}

export interface AppendTaskDocumentRequest {
  taskId: string;
  slug: string;
  section: string;
}

export function isTaskDocumentDetail(value: unknown): value is TaskDocumentDetail {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.slug === "string" &&
    typeof v.name === "string" &&
    typeof v.kind === "string" &&
    typeof v.content === "string" &&
    typeof v.version === "number" &&
    typeof v.pinned === "boolean" &&
    typeof v.created_at === "string" &&
    typeof v.updated_at === "string"
  );
}

export function isTaskDocumentSummary(
  value: unknown,
): value is TaskDocumentSummary {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.slug === "string" &&
    typeof v.name === "string" &&
    typeof v.kind === "string" &&
    typeof v.pinned === "boolean" &&
    typeof v.version === "number" &&
    typeof v.updated_at === "string" &&
    typeof v.created_at === "string"
  );
}

export function isTaskDocumentHistoryResponse(
  value: unknown,
): value is TaskDocumentHistoryResponse {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.task_id === "string" &&
    typeof v.slug === "string" &&
    Array.isArray(v.history)
  );
}

export type TaskDocumentsTag = { type: "TaskDocuments"; id: string };

export const taskDocumentListTag = (taskId: string): TaskDocumentsTag => ({
  type: "TaskDocuments",
  id: taskId,
});

export const taskDocumentDetailTag = (
  taskId: string,
  slug: string,
): TaskDocumentsTag => ({
  type: "TaskDocuments",
  id: `${taskId}:${slug}:detail`,
});

export const taskDocumentHistoryTag = (
  taskId: string,
  slug: string,
): TaskDocumentsTag => ({
  type: "TaskDocuments",
  id: `${taskId}:${slug}:history`,
});

export const taskDocumentMutationInvalidation = {
  createTaskDocument: (taskId: string): TaskDocumentsTag[] => [
    taskDocumentListTag(taskId),
  ],
  updateTaskDocument: (taskId: string, slug: string): TaskDocumentsTag[] => [
    taskDocumentListTag(taskId),
    taskDocumentDetailTag(taskId, slug),
    taskDocumentHistoryTag(taskId, slug),
  ],
  pinTaskDocument: (taskId: string, slug: string): TaskDocumentsTag[] => [
    taskDocumentListTag(taskId),
    taskDocumentDetailTag(taskId, slug),
  ],
  deleteTaskDocument: (taskId: string, slug: string): TaskDocumentsTag[] => [
    taskDocumentListTag(taskId),
    taskDocumentDetailTag(taskId, slug),
    taskDocumentHistoryTag(taskId, slug),
  ],
  appendTaskDocument: (taskId: string, slug: string): TaskDocumentsTag[] => [
    taskDocumentListTag(taskId),
    taskDocumentDetailTag(taskId, slug),
    taskDocumentHistoryTag(taskId, slug),
  ],
};

export const taskDocumentsApi = createApi({
  reducerPath: "taskDocumentsApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  tagTypes: ["TaskDocuments"],
  endpoints: (builder) => ({
    listTaskDocuments: builder.query<
      TaskDocumentListResponse,
      { taskId: string }
    >({
      queryFn: async ({ taskId }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: `http://127.0.0.1:${
            state.config.lspPort
          }/v1/task/${encodeURIComponent(taskId)}/documents`,
        });
        if (result.error) return { error: result.error };
        const raw = result.data as { task_id?: unknown; documents?: unknown[] };
        if (
          typeof raw !== "object" ||
          raw === null ||
          !Array.isArray(raw.documents)
        ) {
          return {
            error: {
              status: "CUSTOM_ERROR" as const,
              error: `Invalid TaskDocumentListResponse shape: ${JSON.stringify(raw).slice(0, 200)}`,
            },
          };
        }
        return { data: result.data as TaskDocumentListResponse };
      },
      providesTags: (_result, _error, { taskId }) => [
        taskDocumentListTag(taskId),
      ],
    }),

    getTaskDocument: builder.query<
      TaskDocumentDetail,
      { taskId: string; slug: string; version?: number }
    >({
      queryFn: async ({ taskId, slug, version }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const params =
          version !== undefined ? `?version=${String(version)}` : "";
        const result = await baseQuery({
          url: `http://127.0.0.1:${
            state.config.lspPort
          }/v1/task/${encodeURIComponent(
            taskId,
          )}/documents/${encodeURIComponent(slug)}${params}`,
        });
        if (result.error) return { error: result.error };
        if (!isTaskDocumentDetail(result.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR" as const,
              error: `Invalid TaskDocumentDetail shape: ${JSON.stringify(result.data).slice(0, 200)}`,
            },
          };
        }
        return { data: result.data };
      },
      providesTags: (_result, _error, { taskId, slug }) => [
        taskDocumentDetailTag(taskId, slug),
      ],
    }),

    createTaskDocument: builder.mutation<
      TaskDocumentDetail,
      CreateTaskDocumentRequest
    >({
      queryFn: async (
        { taskId, slug, name, kind, content, pinned },
        api,
        _opts,
        baseQuery,
      ) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: `http://127.0.0.1:${
            state.config.lspPort
          }/v1/task/${encodeURIComponent(taskId)}/documents`,
          method: "POST",
          body: { slug, name, kind, content, pinned },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskDocumentDetail };
      },
      invalidatesTags: (_result, _error, { taskId }) =>
        taskDocumentMutationInvalidation.createTaskDocument(taskId),
    }),

    updateTaskDocument: builder.mutation<
      TaskDocumentDetail,
      UpdateTaskDocumentRequest
    >({
      queryFn: async ({ taskId, slug, content }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: `http://127.0.0.1:${
            state.config.lspPort
          }/v1/task/${encodeURIComponent(
            taskId,
          )}/documents/${encodeURIComponent(slug)}`,
          method: "PUT",
          body: { content },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskDocumentDetail };
      },
      invalidatesTags: (_result, _error, { taskId, slug }) =>
        taskDocumentMutationInvalidation.updateTaskDocument(taskId, slug),
    }),

    appendTaskDocument: builder.mutation<
      TaskDocumentDetail,
      AppendTaskDocumentRequest
    >({
      queryFn: async ({ taskId, slug, section }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: `http://127.0.0.1:${
            state.config.lspPort
          }/v1/task/${encodeURIComponent(
            taskId,
          )}/documents/${encodeURIComponent(slug)}/append`,
          method: "POST",
          body: { section },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskDocumentDetail };
      },
      invalidatesTags: (_result, _error, { taskId, slug }) =>
        taskDocumentMutationInvalidation.appendTaskDocument(taskId, slug),
    }),

    deleteTaskDocument: builder.mutation<
      DeleteTaskDocumentResponse,
      DeleteTaskDocumentRequest
    >({
      queryFn: async ({ taskId, slug }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: `http://127.0.0.1:${
            state.config.lspPort
          }/v1/task/${encodeURIComponent(
            taskId,
          )}/documents/${encodeURIComponent(slug)}`,
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: { ok: true } };
      },
      invalidatesTags: (_result, _error, { taskId, slug }) =>
        taskDocumentMutationInvalidation.deleteTaskDocument(taskId, slug),
    }),

    pinTaskDocument: builder.mutation<
      TaskDocumentDetail,
      PinTaskDocumentRequest
    >({
      queryFn: async ({ taskId, slug, pinned }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: `http://127.0.0.1:${
            state.config.lspPort
          }/v1/task/${encodeURIComponent(
            taskId,
          )}/documents/${encodeURIComponent(slug)}/pin`,
          method: "POST",
          body: { pinned },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskDocumentDetail };
      },
      invalidatesTags: (_result, _error, { taskId, slug }) =>
        taskDocumentMutationInvalidation.pinTaskDocument(taskId, slug),
      async onQueryStarted(
        { taskId, slug, pinned },
        { dispatch, queryFulfilled },
      ) {
        const listPatch = dispatch(
          taskDocumentsApi.util.updateQueryData(
            "listTaskDocuments",
            { taskId },
            (draft) => {
              const document = draft.documents.find((doc) => doc.slug === slug);
              if (document) {
                document.pinned = pinned;
              }
            },
          ),
        );
        const detailPatch = dispatch(
          taskDocumentsApi.util.updateQueryData(
            "getTaskDocument",
            { taskId, slug },
            (draft) => {
              draft.pinned = pinned;
            },
          ),
        );
        try {
          await queryFulfilled;
        } catch {
          listPatch.undo();
          detailPatch.undo();
        }
      },
    }),

    getTaskDocumentHistory: builder.query<
      TaskDocumentHistoryResponse,
      { taskId: string; slug: string }
    >({
      queryFn: async ({ taskId, slug }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: `http://127.0.0.1:${
            state.config.lspPort
          }/v1/task/${encodeURIComponent(
            taskId,
          )}/documents/${encodeURIComponent(slug)}/history`,
        });
        if (result.error) return { error: result.error };
        if (!isTaskDocumentHistoryResponse(result.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR" as const,
              error: `Invalid TaskDocumentHistoryResponse shape: ${JSON.stringify(result.data).slice(0, 200)}`,
            },
          };
        }
        return { data: result.data };
      },
      providesTags: (_result, _error, { taskId, slug }) => [
        taskDocumentHistoryTag(taskId, slug),
      ],
    }),
  }),
});

export const {
  useListTaskDocumentsQuery,
  useGetTaskDocumentQuery,
  useCreateTaskDocumentMutation,
  useUpdateTaskDocumentMutation,
  useAppendTaskDocumentMutation,
  useDeleteTaskDocumentMutation,
  usePinTaskDocumentMutation,
  useGetTaskDocumentHistoryQuery,
} = taskDocumentsApi;
