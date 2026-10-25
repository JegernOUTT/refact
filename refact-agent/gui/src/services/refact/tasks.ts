import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import type { GoalSnapshot } from "./types";

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

export interface OutcomeRequirement {
  fragment: string;
  requires_evidence: boolean;
}

export type OutcomeFragmentReview =
  | "pending"
  | "approved"
  | "rejected"
  | "contradicted";

export interface OutcomeFragment {
  name: string;
  content: string;
  provenance: {
    source_run: string | null;
    evidence_spec_ids: string[];
  };
  review: OutcomeFragmentReview;
  reviewer_chat_id: string | null;
  reviewed_at: string | null;
  review_note: string | null;
}

export interface CardOutcome {
  required: OutcomeRequirement[];
  fragments: OutcomeFragment[];
  finalized: { finalized_by: string; finalized_at: string } | null;
}

export interface CardAttention {
  reason: string;
  kind: string;
  since: string;
  count: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((item) => typeof item === "string")
  );
}

function isOutcomeRequirement(value: unknown): value is OutcomeRequirement {
  return (
    isRecord(value) &&
    typeof value.fragment === "string" &&
    typeof value.requires_evidence === "boolean"
  );
}

function isOutcomeFragment(value: unknown): value is OutcomeFragment {
  if (!isRecord(value) || !isRecord(value.provenance)) return false;
  return (
    typeof value.name === "string" &&
    typeof value.content === "string" &&
    (typeof value.provenance.source_run === "string" ||
      value.provenance.source_run === null) &&
    isStringArray(value.provenance.evidence_spec_ids) &&
    (value.review === "pending" ||
      value.review === "approved" ||
      value.review === "rejected" ||
      value.review === "contradicted") &&
    (typeof value.reviewer_chat_id === "string" ||
      value.reviewer_chat_id === null) &&
    (typeof value.reviewed_at === "string" || value.reviewed_at === null) &&
    (typeof value.review_note === "string" || value.review_note === null)
  );
}

function isCardOutcome(value: unknown): value is CardOutcome {
  if (!isRecord(value)) return false;
  const finalized = value.finalized;
  return (
    Array.isArray(value.required) &&
    value.required.every(isOutcomeRequirement) &&
    Array.isArray(value.fragments) &&
    value.fragments.every(isOutcomeFragment) &&
    (finalized === null ||
      (isRecord(finalized) &&
        typeof finalized.finalized_by === "string" &&
        typeof finalized.finalized_at === "string"))
  );
}

export function normalizeCardOutcomes(
  value: unknown,
): Record<string, CardOutcome> | undefined {
  if (value === undefined) return undefined;
  if (!isRecord(value)) return undefined;

  return Object.fromEntries(
    Object.entries(value).filter((entry): entry is [string, CardOutcome] =>
      isCardOutcome(entry[1]),
    ),
  );
}

export function normalizeCardAttention(
  value: unknown,
): CardAttention | undefined {
  if (
    !isRecord(value) ||
    typeof value.reason !== "string" ||
    typeof value.kind !== "string" ||
    typeof value.since !== "string" ||
    typeof value.count !== "number" ||
    !Number.isInteger(value.count) ||
    value.count < 0
  ) {
    return undefined;
  }

  return {
    reason: value.reason,
    kind: value.kind,
    since: value.since,
    count: value.count,
  };
}

export function normalizeTaskBoard(value: unknown): TaskBoard | null {
  if (!isRecord(value) || !Array.isArray(value.cards)) return null;
  return {
    ...(value as unknown as TaskBoard),
    cards: value.cards.filter(isBoardCard).map((card) => ({
      ...card,
      attention: normalizeCardAttention(card.attention),
      paused_at:
        typeof card.paused_at === "string" ? card.paused_at : undefined,
      paused_by:
        typeof card.paused_by === "string" ? card.paused_by : undefined,
    })),
    card_outcomes: normalizeCardOutcomes(value.card_outcomes),
  };
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
  attention?: CardAttention;
  paused_at?: string;
  paused_by?: string;
  agent_branch?: string;
  agent_worktree?: string;
  agent_worktree_name?: string;
  base_branch?: string;
  base_commit?: string;
  target_files: string[];
  comments?: CardComment[];
}

export function isBoardCard(value: unknown): value is BoardCard {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.id === "string" &&
    typeof v.title === "string" &&
    typeof v.column === "string" &&
    typeof v.priority === "string" &&
    Array.isArray(v.depends_on) &&
    typeof v.instructions === "string"
  );
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
  card_outcomes?: Record<string, CardOutcome>;
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
  mode?: string;
  parent_id?: string;
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
  tagTypes: ["Tasks", "Board", "TaskTrajectories", "TaskGoal"],
  endpoints: (builder) => ({
    listTasks: builder.query<TaskMeta[], undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/tasks"),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskMeta[] };
      },
      providesTags: ["Tasks"],
    }),

    createTask: builder.mutation<TaskMeta, CreateTaskRequest>({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/tasks"),
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
        const result = await baseQuery({
          url: buildApiUrlFromState(state, `/v1/tasks/${taskId}`),
        });
        if (result.error) return { error: result.error };
        const response = result.data as { meta: TaskMeta };
        return { data: response.meta };
      },
      providesTags: (_result, _error, taskId) => [
        { type: "Tasks", id: taskId },
      ],
    }),

    getTaskGoal: builder.query<GoalSnapshot | null, string>({
      queryFn: async (taskId, api, _opts, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            `/v1/tasks/${encodeURIComponent(taskId)}/goal`,
            { include_history: "true" },
          ),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as GoalSnapshot | null };
      },
      providesTags: (_result, _error, taskId) => [
        { type: "TaskGoal", id: taskId },
      ],
    }),

    deleteTask: builder.mutation<{ deleted: boolean }, string>({
      queryFn: async (taskId, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, `/v1/tasks/${taskId}`),
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: { deleted: true } };
      },
      invalidatesTags: (_result, _error, taskId) => [
        "Tasks",
        { type: "Tasks", id: taskId },
        { type: "Board", id: taskId },
        { type: "TaskGoal", id: taskId },
        { type: "TaskTrajectories", id: `${taskId}/planner` },
        { type: "TaskTrajectories", id: `${taskId}/agents` },
      ],
    }),

    updateTaskStatus: builder.mutation<
      TaskMeta,
      { taskId: string; status: TaskMeta["status"] }
    >({
      queryFn: async ({ taskId, status }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, `/v1/tasks/${taskId}/status`),
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
        const result = await baseQuery({
          url: buildApiUrlFromState(state, `/v1/tasks/${taskId}/board`),
        });
        if (result.error) return { error: result.error };
        const raw = result.data;
        const board = normalizeTaskBoard(raw);
        if (!board) {
          return {
            error: {
              status: "CUSTOM_ERROR" as const,
              error: `Invalid TaskBoard shape: ${JSON.stringify(raw).slice(
                0,
                200,
              )}`,
            },
          };
        }
        return { data: board };
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
        const result = await baseQuery({
          url: buildApiUrlFromState(state, `/v1/tasks/${taskId}/board`),
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
        const result = await baseQuery({
          url: buildApiUrlFromState(state, `/v1/tasks/${taskId}/board/ready`),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as ReadyCardsResult };
      },
    }),

    getOrchestratorInstructions: builder.query<string, string>({
      queryFn: async (taskId, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/tasks/${taskId}/planner-instructions`,
          ),
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
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/tasks/${taskId}/planner-instructions`,
          ),
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
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/tasks/${taskId}/trajectories/${role}`,
          ),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TrajectoryInfo[] };
      },
      providesTags: (_result, _error, { taskId, role }) => [
        { type: "TaskTrajectories", id: `${taskId}/${role}` },
      ],
    }),

    createPlannerChat: builder.mutation<
      { chat_id: string; mode?: string },
      { taskId: string; mode?: string }
    >({
      queryFn: async ({ taskId, mode }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, `/v1/tasks/${taskId}/planner-chats`),
          method: "POST",
          body: mode ? { mode } : {},
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { chat_id: string; mode?: string } };
      },
      invalidatesTags: (_result, _error, { taskId }) => [
        { type: "TaskTrajectories", id: `${taskId}/planner` },
        { type: "TaskGoal", id: taskId },
      ],
    }),

    deletePlannerChat: builder.mutation<
      { deleted: boolean },
      { taskId: string; chatId: string }
    >({
      queryFn: async ({ taskId, chatId }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/tasks/${taskId}/planner-chats/${chatId}`,
            { force: "true" },
          ),
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { deleted: boolean } };
      },
      invalidatesTags: (_result, _error, { taskId }) => [
        { type: "TaskTrajectories", id: `${taskId}/planner` },
        { type: "Board", id: taskId },
        { type: "Tasks", id: taskId },
        "Tasks",
      ],
    }),

    createPlannerChatFromTransition: builder.mutation<
      {
        new_chat_id: string;
        messages_count: number;
        root_chat_id?: string | null;
        initial_plan_document?: string | null;
        initial_plan_error?: string | null;
      },
      {
        taskId: string;
        sourceChatId: string;
        targetModeDescription?: string;
        targetMode?: string;
      }
    >({
      queryFn: async (
        { taskId, sourceChatId, targetModeDescription, targetMode },
        api,
        _opts,
        baseQuery,
      ) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/tasks/${taskId}/planner-chats/from-transition`,
          ),
          method: "POST",
          body: {
            source_chat_id: sourceChatId,
            target_mode_description: targetModeDescription ?? "",
            target_mode: targetMode ?? "task_planner",
          },
        });
        if (result.error) return { error: result.error };
        return {
          data: result.data as {
            new_chat_id: string;
            messages_count: number;
            root_chat_id?: string | null;
            initial_plan_document?: string | null;
            initial_plan_error?: string | null;
          },
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
        const requestBody: {
          body: string;
          author_role: "user";
          reply_to?: string;
        } = {
          body,
          author_role: authorRole,
        };
        if (replyTo !== undefined) {
          requestBody.reply_to = replyTo;
        }
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/tasks/${taskId}/cards/${cardId}/comments`,
          ),
          method: "POST",
          body: requestBody,
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
      }
    >({
      queryFn: async (
        { taskId, name, baseBranch, baseCommit },
        api,
        _opts,
        baseQuery,
      ) => {
        const state = api.getState() as RootState;
        const body: Record<string, string> = {};
        if (name !== undefined) body.name = name;
        if (baseBranch !== undefined) body.base_branch = baseBranch;
        if (baseCommit !== undefined) body.base_commit = baseCommit;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, `/v1/tasks/${taskId}/meta`),
          method: "PATCH",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskMeta };
      },
      invalidatesTags: (_result, _error, { taskId }) => [
        { type: "Tasks", id: taskId },
        "Tasks",
      ],
    }),
  }),
});

export const {
  useListTasksQuery,
  useCreateTaskMutation,
  useGetTaskQuery,
  useGetTaskGoalQuery,
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
