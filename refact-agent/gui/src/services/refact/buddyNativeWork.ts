import { buddyApi } from "./buddy";
import { buildApiUrlFromState, type EngineApiConfig } from "./apiUrl";

export type NativeGoalView = {
  version: number;
  content: string;
  active: boolean;
  status: string;
  turns_used: number;
  tokens_used: number;
  max_turns: number | null;
  max_tokens: number | null;
  latest_verdict: string | null;
  gaps: string[];
  verification_blocked: boolean;
  truncated: boolean;
};
export type NativeAgentView = {
  agent_id: string;
  title: string;
  status: string;
  model: string;
  progress: string | null;
};
export type NativeWorkView = {
  chat_id: string;
  title: string;
  model: string;
  state: string;
  persistence_pending: boolean;
  goal: NativeGoalView | null;
  agents: NativeAgentView[];
  agents_truncated: boolean;
  pending_questions: number;
  approval_count: number;
  browser_decision_pending: boolean;
  error: string | null;
};
export type NativeWorkSnapshot = {
  schema_version: 1;
  project_root: string;
  coverage: "loaded_native_sessions";
  next_cursor: string | null;
  sessions: NativeWorkView[];
};

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Invalid native work");
  return value as Record<string, unknown>;
}
function text(value: unknown): string {
  if (typeof value !== "string") throw new Error("Invalid native work text");
  return value;
}
function flag(value: unknown): boolean {
  if (typeof value !== "boolean") throw new Error("Invalid native work flag");
  return value;
}
function count(value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0)
    throw new Error("Invalid native work count");
  return value;
}
function nullable<T>(value: unknown, parse: (value: unknown) => T): T | null {
  return value === null ? null : parse(value);
}
function list<T>(
  value: unknown,
  limit: number,
  parse: (value: unknown) => T,
): T[] {
  if (!Array.isArray(value) || value.length > limit)
    throw new Error("Invalid native work list");
  return value.map(parse);
}
function goal(value: unknown): NativeGoalView {
  const v = object(value);
  const status = text(v.status);
  if (
    ![
      "active",
      "verifying",
      "paused",
      "completed",
      "stopped",
      "budget_exhausted",
      "no_progress",
      "transferred",
    ].includes(status)
  )
    throw new Error("Unknown native goal state");
  return {
    version: count(v.version),
    content: text(v.content),
    active: flag(v.active),
    status,
    turns_used: count(v.turns_used),
    tokens_used: count(v.tokens_used),
    max_turns: nullable(v.max_turns, count),
    max_tokens: nullable(v.max_tokens, count),
    latest_verdict: nullable(v.latest_verdict, text),
    gaps: list(v.gaps, 10, text),
    verification_blocked: flag(v.verification_blocked),
    truncated: flag(v.truncated),
  };
}
function agent(value: unknown): NativeAgentView {
  const v = object(value);
  return {
    agent_id: text(v.agent_id),
    title: text(v.title),
    status: text(v.status),
    model: text(v.model),
    progress: nullable(v.progress, text),
  };
}
function session(value: unknown): NativeWorkView {
  const v = object(value);
  const state = text(v.state);
  if (
    ![
      "idle",
      "generating",
      "executing_tools",
      "paused",
      "waiting_ide",
      "waiting_user_input",
      "completed",
      "error",
    ].includes(state)
  )
    throw new Error("Unknown native session state");
  return {
    chat_id: text(v.chat_id),
    title: text(v.title),
    model: text(v.model),
    state,
    persistence_pending: flag(v.persistence_pending),
    goal: nullable(v.goal, goal),
    agents: list(v.agents, 20, agent),
    agents_truncated: flag(v.agents_truncated),
    pending_questions: count(v.pending_questions),
    approval_count: count(v.approval_count),
    browser_decision_pending: flag(v.browser_decision_pending),
    error: nullable(v.error, text),
  };
}
export function parseNativeWork(value: unknown): NativeWorkSnapshot {
  const v = object(value);
  if (v.schema_version !== 1 || v.coverage !== "loaded_native_sessions")
    throw new Error("Unsupported native work snapshot");
  const sessions = list(v.sessions, 20, session);
  if (
    new Set(sessions.map((row) => row.chat_id)).size !== sessions.length ||
    sessions.some((row) => !row.chat_id.trim())
  )
    throw new Error("Invalid native work identity");
  const root = text(v.project_root);
  if (!root.trim()) throw new Error("Missing native project");
  return {
    schema_version: 1,
    project_root: root,
    coverage: "loaded_native_sessions",
    next_cursor: nullable(v.next_cursor, text),
    sessions,
  };
}

export const buddyNativeWorkApi = buddyApi.injectEndpoints({
  endpoints: (builder) => ({
    getBuddyNativeWork: builder.query<
      NativeWorkSnapshot,
      { endpoint_identity: string; after?: string }
    >({
      queryFn: async (args, api, _options, baseQuery) => {
        const result = await baseQuery(
          buildApiUrlFromState(
            api.getState() as { config: EngineApiConfig },
            "/v1/buddy/native-work",
            { after: args.after },
          ),
        );
        if (result.error) return { error: result.error };
        try {
          return { data: parseNativeWork(result.data) };
        } catch {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Invalid native work snapshot",
            },
          };
        }
      },
    }),
  }),
});
export const { useGetBuddyNativeWorkQuery } = buddyNativeWorkApi;
