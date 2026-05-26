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
  base_branch?: string;
  base_commit?: string;
  default_agent_model?: string;
  planner_session_state?:
    | "idle"
    | "generating"
    | "executing_tools"
    | "paused"
    | "waiting_ide"
    | "waiting_user_input"
    | "completed"
    | "error";
}

export interface BoardColumn {
  id: string;
  title: string;
}

export interface StatusUpdate {
  timestamp: string;
  message: string;
}

export interface CardComment {
  id: string;
  author_role: "planner" | "agents" | "user" | "system" | "http";
  author_id: string | null;
  timestamp: string;
  body: string;
  reply_to: string | null;
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
  last_heartbeat_at?: string | null;
  agent_branch?: string;
  agent_worktree?: string;
  agent_worktree_name?: string;
  target_files: string[];
  comments?: CardComment[];
}

export interface CreateTaskRequest {
  name: string;
  target_files?: string[];
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

export interface TrajectoryInfo {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  session_state?: string;
  waiting_for_card_ids?: string[];
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
  tagTypes: ["Tasks", "Board", "TaskTrajectories"],
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

    createTask: builder.mutation<TaskMeta, CreateTaskRequest>({
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
      providesTags: (_result, _error, taskId) => [
        { type: "Tasks", id: taskId },
      ],
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

    updateTaskStatus: builder.mutation<
      TaskMeta,
      { taskId: string; status: TaskMeta["status"] }
    >({
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
      invalidatesTags: (_result, _error, { taskId }) => [
        { type: "Tasks", id: taskId },
        "Tasks",
      ],
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
      providesTags: (_result, _error, taskId) => [
        { type: "Board", id: taskId },
      ],
    }),

    patchBoard: builder.mutation<
      TaskBoard,
      { taskId: string; board: Partial<TaskBoard> }
    >({
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
      invalidatesTags: (_result, _error, { taskId }) => [
        { type: "Board", id: taskId },
      ],
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
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/planner-instructions`,
        });
        if (result.error) return { error: result.error };
        const payload = result.data as { content?: unknown };
        return {
          data: typeof payload.content === "string" ? payload.content : "",
        };
      },
    }),

    setOrchestratorInstructions: builder.mutation<
      { saved: boolean },
      { taskId: string; content: string }
    >({
      queryFn: async ({ taskId, content }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/planner-instructions`,
          method: "PUT",
          body: { content },
        });
        if (result.error) return { error: result.error };
        return { data: { saved: true } };
      },
    }),

    listTaskTrajectories: builder.query<
      TrajectoryInfo[],
      { taskId: string; role: string }
    >({
      queryFn: async ({ taskId, role }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/trajectories/${role}`,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TrajectoryInfo[] };
      },
      providesTags: (_result, _error, { taskId, role }) => [
        { type: "TaskTrajectories", id: `${taskId}/${role}` },
      ],
    }),

    createPlannerChat: builder.mutation<{ chat_id: string }, string>({
      queryFn: async (taskId, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/planner-chats`,
          method: "POST",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { chat_id: string } };
      },
      invalidatesTags: (_result, _error, taskId) => [
        { type: "TaskTrajectories", id: `${taskId}/planner` },
      ],
    }),

    deletePlannerChat: builder.mutation<
      { deleted: boolean },
      { taskId: string; chatId: string }
    >({
      queryFn: async ({ taskId, chatId }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/planner-chats/${chatId}`,
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { deleted: boolean } };
      },
      invalidatesTags: (_result, _error, { taskId }) => [
        { type: "TaskTrajectories", id: `${taskId}/planner` },
        { type: "Tasks", id: taskId },
        "Tasks",
      ],
    }),

    createPlannerChatFromTransition: builder.mutation<
      { new_chat_id: string; messages_count: number },
      {
        taskId: string;
        sourceChatId: string;
        targetModeDescription?: string;
      }
    >({
      queryFn: async (
        { taskId, sourceChatId, targetModeDescription },
        api,
        _opts,
        baseQuery,
      ) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/planner-chats/from-transition`,
          method: "POST",
          body: {
            source_chat_id: sourceChatId,
            target_mode_description: targetModeDescription ?? "",
          },
        });
        if (result.error) return { error: result.error };
        return {
          data: result.data as { new_chat_id: string; messages_count: number },
        };
      },
      invalidatesTags: (_result, _error, { taskId }) => [
        { type: "TaskTrajectories", id: `${taskId}/planner` },
      ],
    }),

    addCardComment: builder.mutation<
      TaskBoard,
      {
        taskId: string;
        cardId: string;
        body: string;
        authorRole: "user";
        replyTo?: string;
      }
    >({
      queryFn: async (
        { taskId, cardId, body, authorRole, replyTo },
        api,
        _opts,
        baseQuery,
      ) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/board`,
          method: "POST",
          body: {
            rev: 0,
            patches: [
              {
                type: "AddComment",
                card_id: cardId,
                body,
                author_role: authorRole,
                author_id: null,
                reply_to: replyTo ?? null,
              },
            ],
          },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskBoard };
      },
      invalidatesTags: (_result, _error, { taskId }) => [
        { type: "Board", id: taskId },
      ],
    }),

    updateTaskMeta: builder.mutation<
      TaskMeta,
      {
        taskId: string;
        name?: string;
        baseBranch?: string;
        baseCommit?: string;
        defaultAgentModel?: string;
      }
    >({
      queryFn: async (
        { taskId, name, baseBranch, baseCommit, defaultAgentModel },
        api,
        _opts,
        baseQuery,
      ) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const body: Record<string, string> = {};
        if (name !== undefined) body.name = name;
        if (baseBranch !== undefined) body.base_branch = baseBranch;
        if (baseCommit !== undefined) body.base_commit = baseCommit;
        if (defaultAgentModel !== undefined)
          body.default_agent_model = defaultAgentModel;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/tasks/${taskId}/meta`,
          method: "PATCH",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskMeta };
      },
      invalidatesTags: (_result, _error, { taskId }) => [
        { type: "Tasks", id: taskId },
      ],
    }),
  }),
});

export const {
  useListTasksQuery,
  useCreateTaskMutation,
  useGetTaskQuery,
  useDeleteTaskMutation,
  useUpdateTaskStatusMutation,
  useUpdateTaskMetaMutation,
  useGetBoardQuery,
  usePatchBoardMutation,
  useGetReadyCardsQuery,
  useGetOrchestratorInstructionsQuery,
  useSetOrchestratorInstructionsMutation,
  useListTaskTrajectoriesQuery,
  useCreatePlannerChatMutation,
  useDeletePlannerChatMutation,
  useCreatePlannerChatFromTransitionMutation,
  useAddCardCommentMutation,
} = tasksApi;
