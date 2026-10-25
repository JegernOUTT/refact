import { buddyApi } from "./buddy";
import { buildApiUrlFromState, type EngineApiConfig } from "./apiUrl";
import type {
  PendingQuestionSet,
  PendingQuestionAnswer,
  QuestionResponse,
} from "./types";
import { buddyOperatorIdentity } from "./buddyOperator";

export interface NativeQuestionView {
  project_root: string;
  chat_id: string;
  question: PendingQuestionSet;
}
export interface NativeDecisionSnapshot {
  schema_version: 1;
  project_root: string;
  coverage: "loaded_native_sessions";
  truncated: boolean;
  questions: NativeQuestionView[];
}

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Invalid decision object");
  return value as Record<string, unknown>;
}
function nonempty(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}
export function parseNativeQuestion(value: unknown): NativeQuestionView {
  const row = object(value);
  const q = object(row.question);
  if (
    !nonempty(row.project_root) ||
    !nonempty(row.chat_id) ||
    !nonempty(q.tool_call_id) ||
    typeof q.asked_at_ms !== "number" ||
    !Number.isSafeInteger(q.asked_at_ms) ||
    q.asked_at_ms < 0 ||
    !["pending", "answered", "cancelled"].includes(String(q.status)) ||
    !Array.isArray(q.questions) ||
    q.questions.length === 0 ||
    q.questions.length > 50
  )
    throw new Error("Unsupported native decision");
  const ids = new Set<string>();
  const questions = q.questions.map((value) => {
    const question = object(value);
    if (
      !nonempty(question.id) ||
      ids.has(question.id) ||
      !nonempty(question.text) ||
      !["yes_no", "single_select", "multi_select", "free_text"].includes(
        String(question.type),
      ) ||
      (question.options !== undefined &&
        (!Array.isArray(question.options) ||
          !question.options.every(nonempty))) ||
      (["single_select", "multi_select"].includes(String(question.type)) &&
        (!Array.isArray(question.options) || !question.options.length))
    )
      throw new Error("Invalid native question");
    ids.add(question.id);
    return {
      id: question.id,
      text: question.text,
      type: question.type as PendingQuestionSet["questions"][number]["type"],
      options: question.options,
    };
  });
  return {
    project_root: row.project_root,
    chat_id: row.chat_id,
    question: {
      tool_call_id: q.tool_call_id,
      asked_at_ms: q.asked_at_ms,
      status: q.status as PendingQuestionSet["status"],
      questions,
      response_protocol_version:
        q.response_protocol_version === 2 ? 2 : undefined,
    },
  };
}
export function parseNativeDecisionSnapshot(
  value: unknown,
): NativeDecisionSnapshot {
  const v = object(value);
  if (
    v.schema_version !== 1 ||
    v.coverage !== "loaded_native_sessions" ||
    !nonempty(v.project_root) ||
    typeof v.truncated !== "boolean" ||
    !Array.isArray(v.questions) ||
    v.questions.length > 100
  )
    throw new Error("Unsupported native decision snapshot");
  const questions = v.questions.map(parseNativeQuestion);
  if (questions.some((question) => question.project_root !== v.project_root))
    throw new Error("Native project mismatch");
  const identities = new Set(
    questions.map((question) =>
      JSON.stringify([
        question.chat_id,
        question.question.tool_call_id,
        question.question.asked_at_ms,
      ]),
    ),
  );
  if (identities.size !== questions.length)
    throw new Error("Duplicate native decision identity");
  return {
    schema_version: 1,
    project_root: v.project_root,
    coverage: "loaded_native_sessions",
    truncated: v.truncated,
    questions,
  };
}

type BuddyDecisionsState = { config: EngineApiConfig };

function staleEndpoint(): { error: { status: "CUSTOM_ERROR"; error: string } } {
  return {
    error: {
      status: "CUSTOM_ERROR",
      error: "Project connection changed; reload native questions",
    },
  };
}

export const buddyDecisionsApi = buddyApi.injectEndpoints({
  endpoints: (builder) => ({
    answerBuddyNativeQuestion: builder.mutation<
      { queued: true },
      {
        chat_id: string;
        tool_call_id: string;
        project_root: string;
        asked_at_ms: number;
        client_request_id: string;
        endpoint_identity: string;
        answers?: PendingQuestionAnswer[];
        response?: QuestionResponse;
      }
    >({
      queryFn: async (args, api, _options, baseQuery) => {
        const state = api.getState() as BuddyDecisionsState;
        if (buddyOperatorIdentity(state) !== args.endpoint_identity)
          return staleEndpoint();
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/buddy/decisions/questions/${encodeURIComponent(
              args.chat_id,
            )}/${encodeURIComponent(args.tool_call_id)}`,
          ),
          method: "POST",
          body: {
            project_root: args.project_root,
            asked_at_ms: args.asked_at_ms,
            client_request_id: args.client_request_id,
            answers: args.answers,
            response: args.response,
          },
        });
        if (
          buddyOperatorIdentity(api.getState() as BuddyDecisionsState) !==
          args.endpoint_identity
        )
          return staleEndpoint();
        if (result.error) return { error: result.error };
        return { data: { queued: true } };
      },
    }),
    getBuddyNativeDecisions: builder.query<
      NativeDecisionSnapshot,
      { endpoint_identity: string }
    >({
      queryFn: async (args, api, _options, baseQuery) => {
        const state = api.getState() as BuddyDecisionsState;
        if (buddyOperatorIdentity(state) !== args.endpoint_identity)
          return staleEndpoint();
        const result = await baseQuery(
          buildApiUrlFromState(state, "/v1/buddy/decisions/current"),
        );
        if (
          buddyOperatorIdentity(api.getState() as BuddyDecisionsState) !==
          args.endpoint_identity
        )
          return staleEndpoint();
        if (result.error) return { error: result.error };
        try {
          return { data: parseNativeDecisionSnapshot(result.data) };
        } catch {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Invalid native decision snapshot",
            },
          };
        }
      },
    }),
    getBuddyNativeQuestion: builder.query<
      NativeQuestionView,
      {
        project_root: string;
        chat_id: string;
        tool_call_id: string;
        endpoint_identity: string;
      }
    >({
      queryFn: async (args, api, _options, baseQuery) => {
        const state = api.getState() as BuddyDecisionsState;
        if (buddyOperatorIdentity(state) !== args.endpoint_identity)
          return staleEndpoint();
        const result = await baseQuery(
          buildApiUrlFromState(
            state,
            `/v1/buddy/decisions/questions/${encodeURIComponent(
              args.chat_id,
            )}/${encodeURIComponent(args.tool_call_id)}`,
            { project_root: args.project_root },
          ),
        );
        if (
          buddyOperatorIdentity(api.getState() as BuddyDecisionsState) !==
          args.endpoint_identity
        )
          return staleEndpoint();
        if (result.error) return { error: result.error };
        try {
          const data = parseNativeQuestion(result.data);
          if (
            data.project_root !== args.project_root ||
            data.chat_id !== args.chat_id ||
            data.question.tool_call_id !== args.tool_call_id
          )
            throw new Error("Identity mismatch");
          return { data };
        } catch {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Invalid native decision identity",
            },
          };
        }
      },
    }),
  }),
});
export const {
  useGetBuddyNativeDecisionsQuery,
  useGetBuddyNativeQuestionQuery,
  useAnswerBuddyNativeQuestionMutation,
} = buddyDecisionsApi;
