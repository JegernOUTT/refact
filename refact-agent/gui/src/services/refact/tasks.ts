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

export interface CardVerificationResult {
  command: string;
  exit_code?: number | null;
  passed?: boolean;
  output_tail?: string;
  outcome?:
    | "unknown"
    | "passed"
    | "command_failed"
    | "rejected"
    | "policy_denied"
    | "infrastructure_failed"
    | "no_commands";
  from_cache?: boolean;
  executed_at?: string | null;
}

export interface CardFinalReport {
  summary?: string;
  success?: boolean;
  files_changed?: string[];
  tests_added_or_updated?: string[];
  verification?: CardVerificationResult[];
  followup_cards?: {
    title: string;
    instructions: string;
    priority: string;
    target_files: string[];
  }[];
  risks?: string[];
  assumptions?: string[];
}

export interface CardVerifierReport {
  passed?: boolean;
  command_results?: CardVerificationResult[];
  concerns?: string[];
  recommendation?: string;
  classification?:
    | "unknown"
    | "passed"
    | "verification_failed"
    | "human_review"
    | "unavailable";
}

export interface CardClaimView {
  claimant_chat_id: string;
  claimed_at: string;
}

export interface CardReviewAttestation {
  reviewer_chat_id: string;
  reviewed_at: string;
  report: CardVerifierReport;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function taskErrorMessage(error: unknown): string {
  if (isRecord(error)) {
    if (typeof error.data === "string" && error.data.trim()) return error.data;
    if (isRecord(error.data)) {
      if (typeof error.data.error === "string") return error.data.error;
      if (typeof error.data.detail === "string") return error.data.detail;
    }
    if (typeof error.error === "string") return error.error;
    if (typeof error.message === "string") return error.message;
  }
  return "Request failed. Please try again.";
}

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((item) => typeof item === "string")
  );
}

function optionalField(
  value: Record<string, unknown>,
  key: string,
  validate: (field: unknown) => boolean,
): boolean {
  return value[key] === undefined || validate(value[key]);
}

function isVerificationResult(value: unknown): value is CardVerificationResult {
  return (
    isRecord(value) &&
    typeof value.command === "string" &&
    optionalField(value, "exit_code", (field) =>
      field === null ? true : Number.isInteger(field),
    ) &&
    optionalField(value, "passed", (field) => typeof field === "boolean") &&
    optionalField(value, "output_tail", (field) => typeof field === "string") &&
    optionalField(value, "outcome", (field) =>
      [
        "unknown",
        "passed",
        "command_failed",
        "rejected",
        "policy_denied",
        "infrastructure_failed",
        "no_commands",
      ].includes(String(field)),
    ) &&
    optionalField(value, "from_cache", (field) => typeof field === "boolean") &&
    optionalField(
      value,
      "executed_at",
      (field) => field === null || typeof field === "string",
    )
  );
}

function isVerifierReport(value: unknown): value is CardVerifierReport {
  return (
    isRecord(value) &&
    optionalField(value, "passed", (field) => typeof field === "boolean") &&
    optionalField(
      value,
      "command_results",
      (field) => Array.isArray(field) && field.every(isVerificationResult),
    ) &&
    optionalField(value, "concerns", isStringArray) &&
    optionalField(
      value,
      "recommendation",
      (field) => typeof field === "string",
    ) &&
    optionalField(value, "classification", (field) =>
      [
        "unknown",
        "passed",
        "verification_failed",
        "human_review",
        "unavailable",
      ].includes(String(field)),
    )
  );
}

function isFinalReport(value: unknown): value is CardFinalReport {
  return (
    isRecord(value) &&
    optionalField(value, "summary", (field) => typeof field === "string") &&
    optionalField(value, "success", (field) => typeof field === "boolean") &&
    ["files_changed", "tests_added_or_updated", "risks", "assumptions"].every(
      (key) => optionalField(value, key, isStringArray),
    ) &&
    optionalField(
      value,
      "verification",
      (field) => Array.isArray(field) && field.every(isVerificationResult),
    ) &&
    optionalField(
      value,
      "followup_cards",
      (field) =>
        Array.isArray(field) &&
        field.every(
          (entry: unknown) =>
            isRecord(entry) &&
            typeof entry.title === "string" &&
            typeof entry.instructions === "string" &&
            typeof entry.priority === "string" &&
            isStringArray(entry.target_files),
        ),
    )
  );
}

function normalizeRecord<T>(
  value: unknown,
  parse: (entry: unknown) => T | undefined,
): Record<string, T> | undefined {
  if (!isRecord(value)) return undefined;
  return Object.fromEntries(
    Object.entries(value).flatMap(([key, entry]) => {
      const parsed = parse(entry);
      return parsed === undefined ? [] : [[key, parsed]];
    }),
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
      final_report_structured: isFinalReport(card.final_report_structured)
        ? card.final_report_structured
        : undefined,
      verifier_report: isVerifierReport(card.verifier_report)
        ? card.verifier_report
        : undefined,
    })),
    owner_chat_id:
      typeof value.owner_chat_id === "string" && value.owner_chat_id.trim()
        ? value.owner_chat_id
        : undefined,
    reviewer_chat_ids: isStringArray(value.reviewer_chat_ids)
      ? value.reviewer_chat_ids
      : undefined,
    review_policy:
      value.review_policy === "owner_allowed" ||
      value.review_policy === "independent_required"
        ? value.review_policy
        : undefined,
    card_claims: normalizeRecord(value.card_claims, (entry) =>
      isRecord(entry) &&
      typeof entry.claimant_chat_id === "string" &&
      typeof entry.claimed_at === "string"
        ? {
            claimant_chat_id: entry.claimant_chat_id,
            claimed_at: entry.claimed_at,
          }
        : undefined,
    ),
    review_attestations: normalizeRecord(value.review_attestations, (entry) =>
      isRecord(entry) &&
      typeof entry.reviewer_chat_id === "string" &&
      typeof entry.reviewed_at === "string" &&
      isVerifierReport(entry.report)
        ? {
            reviewer_chat_id: entry.reviewer_chat_id,
            reviewed_at: entry.reviewed_at,
            report: entry.report,
          }
        : undefined,
    ),
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
  final_report_structured?: CardFinalReport;
  verifier_report?: CardVerifierReport;
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
  owner_chat_id?: string;
  reviewer_chat_ids?: string[];
  review_policy?: "owner_allowed" | "independent_required";
  card_claims?: Record<string, CardClaimView>;
  review_attestations?: Record<string, CardReviewAttestation>;
  card_outcomes?: Record<string, CardOutcome>;
}

export type BoardPatch =
  | {
      action: "create_card";
      id: string;
      title: string;
      priority?: string;
      depends_on?: string[];
      instructions?: string;
      target_files?: string[];
    }
  | {
      action: "update_card";
      id: string;
      title?: string;
      priority?: string;
      depends_on?: string[];
      instructions?: string;
      target_files?: string[];
    }
  | { action: "move_card"; id: string; column: string }
  | {
      action: "assign_agent";
      card_id: string;
      agent_id: string;
      agent_chat_id: string;
    }
  | { action: "add_status_update"; card_id: string; message: string }
  | {
      action: "add_comment";
      card_id: string;
      body: string;
      author_role: string;
      author_id?: string | null;
      reply_to?: string | null;
    }
  | { action: "set_final_report"; card_id: string; report: string }
  | { action: "delete_card"; card_id: string; force?: boolean };

export type UpdateBoardRequest = { rev: number } & BoardPatch;

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
      { taskId: string; patch: UpdateBoardRequest }
    >({
      queryFn: async ({ taskId, patch }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/tasks/${encodeURIComponent(taskId)}/board`,
          ),
          method: "POST",
          body: patch,
          responseHandler: "content-type",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TaskBoard };
      },
      invalidatesTags: (result, _error, { taskId }) =>
        result
          ? [
              { type: "Board", id: taskId },
              { type: "Tasks", id: taskId },
              "Tasks",
            ]
          : [],
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
          ),
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { deleted: boolean } };
      },
      invalidatesTags: (result, _error, { taskId }) =>
        result?.deleted
          ? [
              { type: "TaskTrajectories", id: `${taskId}/planner` },
              { type: "Board", id: taskId },
              { type: "Tasks", id: taskId },
              { type: "TaskGoal", id: taskId },
              "Tasks",
            ]
          : [],
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
