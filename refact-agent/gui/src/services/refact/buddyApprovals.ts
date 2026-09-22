import { buddyApi } from "./buddy";
import { buildApiUrlFromState, type EngineApiConfig } from "./apiUrl";
import { buddyOperatorIdentity } from "./buddyOperator";

export interface NativeToolApproval {
  type: "confirmation";
  tool_call_id: string;
  tool_name: string;
  command: string;
  rule: string;
}
export interface NativeApprovalSession {
  chat_id: string;
  source_version: string;
  tools: NativeToolApproval[];
  browser_request_id: string | null;
  warnings: (
    | "native_storage_error"
    | "native_runtime_error"
    | "provider_usage_stale"
  )[];
}
export interface NativeApprovalsSnapshot {
  schema_version: 1;
  project_root: string;
  engine_identity: string;
  coverage: "loaded_native_sessions";
  truncated: boolean;
  warnings: ("native_provider_error" | "native_privacy_policy_error")[];
  sessions: NativeApprovalSession[];
}
export type ApprovalResponse =
  | { kind: "tool"; tool_call_id: string; accepted: boolean }
  | {
      kind: "browser";
      pending_message_id: string;
      include_actions: boolean;
      include_console: boolean;
      include_network: boolean;
      include_mutations: boolean;
      include_screenshot: boolean;
    };
export interface NativeApprovalRequest {
  endpoint_identity: string;
  chat_id: string;
  project_root: string;
  engine_identity: string;
  source_version: string;
  client_request_id: string;
  response: ApprovalResponse;
}
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Invalid approval object");
  return value as Record<string, unknown>;
}
function text(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}
export function parseNativeApprovals(value: unknown): NativeApprovalsSnapshot {
  const snapshot = object(value);
  if (
    snapshot.schema_version !== 1 ||
    !text(snapshot.project_root) ||
    !text(snapshot.engine_identity) ||
    snapshot.coverage !== "loaded_native_sessions" ||
    typeof snapshot.truncated !== "boolean" ||
    !Array.isArray(snapshot.warnings) ||
    !snapshot.warnings.every((warning) =>
      ["native_provider_error", "native_privacy_policy_error"].includes(
        String(warning),
      ),
    ) ||
    !Array.isArray(snapshot.sessions) ||
    snapshot.sessions.length > 200
  )
    throw new Error("Unsupported native approval snapshot");
  const ids = new Set<string>();
  const sessions = snapshot.sessions.map((value): NativeApprovalSession => {
    const session = object(value);
    if (
      !text(session.chat_id) ||
      ids.has(session.chat_id) ||
      !text(session.source_version) ||
      !/^[a-f0-9]{64}$/.test(session.source_version) ||
      !Array.isArray(session.tools) ||
      session.tools.length > 100 ||
      !(
        session.browser_request_id === null || text(session.browser_request_id)
      ) ||
      !Array.isArray(session.warnings) ||
      !session.warnings.every((w) =>
        [
          "native_storage_error",
          "native_runtime_error",
          "provider_usage_stale",
        ].includes(String(w)),
      )
    )
      throw new Error("Invalid native approval identity");
    ids.add(session.chat_id);
    const toolIds = new Set<string>();
    const tools = session.tools.map((value): NativeToolApproval => {
      const tool = object(value);
      if (
        tool.type !== "confirmation" ||
        !text(tool.tool_call_id) ||
        toolIds.has(tool.tool_call_id) ||
        !text(tool.tool_name) ||
        typeof tool.command !== "string" ||
        typeof tool.rule !== "string"
      )
        throw new Error("Unknown native approval type");
      toolIds.add(tool.tool_call_id);
      return {
        type: "confirmation",
        tool_call_id: tool.tool_call_id,
        tool_name: tool.tool_name,
        command: tool.command,
        rule: tool.rule,
      };
    });
    return {
      chat_id: session.chat_id,
      source_version: session.source_version,
      tools,
      browser_request_id: session.browser_request_id,
      warnings: session.warnings as NativeApprovalSession["warnings"],
    };
  });
  return {
    schema_version: 1,
    project_root: snapshot.project_root,
    engine_identity: snapshot.engine_identity,
    coverage: "loaded_native_sessions",
    truncated: snapshot.truncated,
    warnings: snapshot.warnings as NativeApprovalsSnapshot["warnings"],
    sessions,
  };
}
type State = { config: EngineApiConfig };
function failure(error: string) {
  return { error: { status: "CUSTOM_ERROR" as const, error } };
}
export const buddyApprovalsApi = buddyApi.injectEndpoints({
  endpoints: (builder) => ({
    getBuddyNativeApprovals: builder.query<
      NativeApprovalsSnapshot,
      { endpoint_identity: string }
    >({
      queryFn: async (args, api, _options, baseQuery) => {
        const state = api.getState() as State;
        if (buddyOperatorIdentity(state) !== args.endpoint_identity)
          return failure("Connection changed");
        const result = await baseQuery(
          buildApiUrlFromState(state, "/v1/buddy/approvals/current"),
        );
        if (
          buddyOperatorIdentity(api.getState() as State) !==
          args.endpoint_identity
        )
          return failure("Connection changed");
        if (result.error) return { error: result.error };
        try {
          return { data: parseNativeApprovals(result.data) };
        } catch {
          return failure("Invalid native approval snapshot");
        }
      },
    }),
    respondBuddyNativeApproval: builder.mutation<
      { queued: true },
      NativeApprovalRequest
    >({
      queryFn: async (args, api, _options, baseQuery) => {
        const state = api.getState() as State;
        if (buddyOperatorIdentity(state) !== args.endpoint_identity)
          return failure("Connection changed");
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/buddy/approvals/${encodeURIComponent(args.chat_id)}/response`,
          ),
          method: "POST",
          body: {
            project_root: args.project_root,
            engine_identity: args.engine_identity,
            source_version: args.source_version,
            client_request_id: args.client_request_id,
            response: args.response,
          },
        });
        if (
          buddyOperatorIdentity(api.getState() as State) !==
          args.endpoint_identity
        )
          return failure("Connection changed");
        if (result.error) return { error: result.error };
        return { data: { queued: true } };
      },
    }),
  }),
});
export const {
  useGetBuddyNativeApprovalsQuery,
  useRespondBuddyNativeApprovalMutation,
} = buddyApprovalsApi;
