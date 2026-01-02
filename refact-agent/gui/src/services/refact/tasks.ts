import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";

export interface TaskMeta {
  id: string;
  name: string;
  status: "planning" | "active" | "paused" | "completed" | "abandoned";
  created_at: string;
  updated_at: string;
  cards_total: number;
  cards_done: number;
  cards_failed: number;
  agents_active: number;
}

export interface BoardColumn {
  id: string;
  title: string;
}

export interface StatusUpdate {
  timestamp: string;
  message: string;
}

export interface BoardCard {
  id: string;
  title: string;
  column: string;
  priority: string;
  depends_on: string[];
  instructions: string;
  assignee: string | null;
  agent_chat_id: string | null;
  status_updates: StatusUpdate[];
  final_report: string | null;
  created_at: string;
  started_at: string | null;
  completed_at: string | null;
}

export interface TaskBoard {
  schema_version: number;
  rev: number;
  columns: BoardColumn[];
  cards: BoardCard[];
}

export interface ReadyCardsResult {
  ready: string[];
  blocked: string[];
  in_progress: string[];
  completed: string[];
  failed: string[];
}

export const tasksApi = createApi({
  reducerPath: "tasksApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  tagTypes: ["Tasks", "Board"],
  endpoints: (builder) => ({
    listTasks: builder.query<TaskMeta[], undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks`,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskMeta[] };
      },
      providesTags: ["Tasks"],
    }),

    createTask: builder.mutation<TaskMeta, { name: string }>({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks`,
          method: "POST",
          body: args,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskMeta };
      },
      invalidatesTags: ["Tasks"],
    }),

    getTask: builder.query<TaskMeta, string>({
      queryFn: async (taskId, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}`,
        });
        if (result.error) return { error: result.error };
        const response = result.data as { meta: TaskMeta };
        return { data: response.meta };
      },
      providesTags: (_result, _error, taskId) => [{ type: "Tasks", id: taskId }],
    }),

    deleteTask: builder.mutation<{ deleted: boolean }, string>({
      queryFn: async (taskId, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}`,
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: { deleted: true } };
      },
      invalidatesTags: ["Tasks"],
    }),

    updateTaskStatus: builder.mutation<TaskMeta, { taskId: string; status: TaskMeta["status"] }>({
      queryFn: async ({ taskId, status }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/status`,
          method: "POST",
          body: { status },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskMeta };
      },
      invalidatesTags: (_result, _error, { taskId }) => [{ type: "Tasks", id: taskId }, "Tasks"],
    }),

    getBoard: builder.query<TaskBoard, string>({
      queryFn: async (taskId, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/board`,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskBoard };
      },
      providesTags: (_result, _error, taskId) => [{ type: "Board", id: taskId }],
    }),

    patchBoard: builder.mutation<TaskBoard, { taskId: string; board: Partial<TaskBoard> }>({
      queryFn: async ({ taskId, board }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/board`,
          method: "POST",
          body: board,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskBoard };
      },
      invalidatesTags: (_result, _error, { taskId }) => [{ type: "Board", id: taskId }],
    }),

    getReadyCards: builder.query<ReadyCardsResult, string>({
      queryFn: async (taskId, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/board/ready`,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as ReadyCardsResult };
      },
    }),

    getOrchestratorInstructions: builder.query<string, string>({
      queryFn: async (taskId, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/orchestrator-instructions`,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as string };
      },
    }),

    setOrchestratorInstructions: builder.mutation<{ saved: boolean }, { taskId: string; content: string }>({
      queryFn: async ({ taskId, content }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/orchestrator-instructions`,
          method: "PUT",
          body: content,
          headers: { "Content-Type": "text/plain" },
        });
        if (result.error) return { error: result.error };
        return { data: { saved: true } };
      },
    }),

    listTaskTrajectories: builder.query<string[], { taskId: string; role: string }>({
      queryFn: async ({ taskId, role }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/trajectories/${role}`,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as string[] };
      },
    }),
  }),
});

export const {
  useListTasksQuery,
  useCreateTaskMutation,
  useGetTaskQuery,
  useDeleteTaskMutation,
  useUpdateTaskStatusMutation,
  useGetBoardQuery,
  usePatchBoardMutation,
  useGetReadyCardsQuery,
  useGetOrchestratorInstructionsQuery,
  useSetOrchestratorInstructionsMutation,
  useListTaskTrajectoriesQuery,
} = tasksApi;
