import type {
  BaseQueryApi,
  FetchArgs,
  FetchBaseQueryError,
} from "@reduxjs/toolkit/query";
import { buddyApi } from "./buddy";
import { buddyOperatorIdentity } from "./buddyOperator";
import { buildApiUrlFromState, type EngineApiConfig } from "./apiUrl";
import {
  parseActionSpec,
  parseActionExecution,
  type ActionSpec,
  type ActionExecution,
} from "../../features/Buddy/operatorTypes";

export type ResourceContext = { endpoint_identity: string; spec: ActionSpec };
export type ResourcePreview = {
  resource_id: string;
  expected_version: string;
  proposed_version: string;
  reload_needed: boolean;
  diff: string | null;
};
export type NativeRights = [string, unknown] | null;
export type PreparedResource = {
  spec: ActionSpec;
  spec_hash: string;
  bound_parameter_ref: string | null;
};
export type ResourceGrant = { grant_id: string; expires_at: string };
export type GrantArgs = ResourceContext & {
  spec_hash: string;
  native_rights: NativeRights;
  recovery_execution_id: string | null;
};
export type ExecutionIdentity = ResourceContext & {
  execution_id: string;
  spec_hash: string;
  idempotency_key: string;
  recovery_execution_id?: string;
  recovery_id?: string;
};
export type ExecuteArgs = ExecutionIdentity & { grant_id: string };
export type ResourceResult = {
  disposition: "Finished" | "Duplicate";
  execution: ActionExecution;
};
type State = {
  config: EngineApiConfig & { apiKey?: string | null };
  current_project?: { workspaceRoots?: unknown; name?: unknown };
};
type QueryResult = { data?: unknown; error?: FetchBaseQueryError };
type Query = (
  args: string | FetchArgs,
) => QueryResult | PromiseLike<QueryResult>;

function object(value: unknown, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Invalid resource response object");
  const row = value as Record<string, unknown>;
  if (
    Object.keys(row).length !== keys.length ||
    keys.some((key) => !(key in row))
  )
    throw new Error("Unknown or missing resource response fields");
  return row;
}
function text(value: unknown): string {
  if (typeof value !== "string" || !value.trim())
    throw new Error("Invalid resource identity");
  return value;
}
function equal(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (!a || !b || typeof a !== "object" || typeof b !== "object") return false;
  const left = a as Record<string, unknown>;
  const right = b as Record<string, unknown>;
  return (
    Object.keys(left).length === Object.keys(right).length &&
    Object.keys(left).every(
      (key) => key in right && equal(left[key], right[key]),
    )
  );
}
export function parseNativeRights(value: unknown): NativeRights {
  if (value === null) return null;
  if (!Array.isArray(value) || value.length !== 2)
    throw new Error("Invalid native rights tuple");
  return [text(value[0]), value[1] as unknown];
}
export function parseResourcePreview(
  value: unknown,
  spec: ActionSpec,
): ResourcePreview {
  const row = object(value, [
    "resource_id",
    "expected_version",
    "proposed_version",
    "reload_needed",
    "diff",
  ]);
  if (
    row.resource_id !== spec.resource.resource_id ||
    row.expected_version !== spec.expected_version ||
    typeof row.reload_needed !== "boolean" ||
    (row.diff !== null && typeof row.diff !== "string")
  )
    throw new Error("Stale or mismatched resource preview");
  return {
    resource_id: text(row.resource_id),
    expected_version: text(row.expected_version),
    proposed_version: text(row.proposed_version),
    reload_needed: row.reload_needed,
    diff: row.diff,
  };
}
function parsePrepared(value: unknown, spec: ActionSpec): PreparedResource {
  const row = object(value, ["spec", "spec_hash", "bound_parameter_ref"]);
  const parsed = parseActionSpec(row.spec);
  if (!equal(parsed, spec) || row.bound_parameter_ref !== spec.parameter_ref)
    throw new Error("Prepared action changed");
  return {
    spec: parsed,
    spec_hash: text(row.spec_hash),
    bound_parameter_ref: spec.parameter_ref,
  };
}
function parseGrant(value: unknown, args: GrantArgs): ResourceGrant {
  const row = object(value, [
    "native_rights",
    "spec_hash",
    "requester",
    "decision",
    "expires_at",
    "revoked",
    "recovery_execution_id",
  ]);
  const requester = object(row.requester, [
    "source_kind",
    "source_id",
    "source_version",
  ]);
  if (requester.source_kind !== "external")
    throw new Error("Invalid grant requester");
  text(requester.source_id);
  text(requester.source_version);
  const decision = object(row.decision, ["project", "worker", "native"]);
  const worker = object(decision.worker, [
    "worker_id",
    "engine_instance_id",
    "epoch",
  ]);
  text(worker.worker_id);
  text(worker.engine_instance_id);
  if (
    typeof worker.epoch !== "number" ||
    !Number.isSafeInteger(worker.epoch) ||
    worker.epoch < 0
  )
    throw new Error("Invalid grant worker");
  const native = object(decision.native, [
    "source_kind",
    "source_id",
    "source_version",
  ]);
  if (
    !equal(decision.project, args.spec.project) ||
    native.source_kind !== "external" ||
    native.source_version !== "resource-grant-v1" ||
    row.spec_hash !== args.spec_hash ||
    !equal(parseNativeRights(row.native_rights), args.native_rights) ||
    row.recovery_execution_id !== args.recovery_execution_id ||
    row.revoked !== false ||
    typeof row.expires_at !== "string" ||
    !Number.isFinite(Date.parse(row.expires_at)) ||
    Date.parse(row.expires_at) <= Date.now()
  )
    throw new Error("Invalid or expired exact grant");
  return { grant_id: text(native.source_id), expires_at: row.expires_at };
}
function parseExecution(
  value: unknown,
  args: ExecutionIdentity,
): ActionExecution {
  const execution = parseActionExecution(value);
  const actionId =
    args.recovery_execution_id && args.recovery_id
      ? `recovery:${args.recovery_execution_id}:${args.recovery_id}`
      : args.spec.action_id;
  if (
    execution.execution_id !== args.execution_id ||
    execution.idempotency_key !== args.idempotency_key ||
    execution.action_id !== actionId ||
    (!args.recovery_execution_id && execution.spec_hash !== args.spec_hash) ||
    !equal(execution.project, args.spec.project)
  )
    throw new Error("Execution identity mismatch");
  return execution;
}
function parseExecutionRecord(
  value: unknown,
  args: ExecuteArgs,
): ActionExecution {
  const row = object(value, [
    "execution",
    "effects",
    "verification_evidence",
    "recovery",
  ]);
  const execution = parseExecution(row.execution, args);
  const details = parseActionExecution({
    ...execution,
    effects: row.effects,
    verification_evidence: row.verification_evidence,
    recovery: row.recovery,
  });
  if (
    execution.idempotency_key !== args.idempotency_key ||
    !equal(details, execution)
  )
    throw new Error("Execution receipt mismatch");
  return execution;
}
export function parseResourceResult(
  value: unknown,
  args: ExecuteArgs,
): ResourceResult {
  if (!value || typeof value !== "object")
    throw new Error("Invalid execution result");
  if ("Rejected" in value) {
    const row = object(value, ["Rejected"]);
    throw new Error(`Action rejected: ${text(row.Rejected)}`);
  }
  const disposition = "Duplicate" in value ? "Duplicate" : "Finished";
  const row = object(value, [disposition]);
  return {
    disposition,
    execution: parseExecutionRecord(row[disposition], args),
  };
}
async function request<T>(
  args: ResourceContext,
  api: BaseQueryApi,
  query: Query,
  path: string,
  body: unknown,
  parse: (value: unknown) => T,
) {
  try {
    const state = api.getState() as State;
    if (buddyOperatorIdentity(state) !== args.endpoint_identity)
      throw new Error("Project connection changed; reload action");
    parseActionSpec(args.spec);
    const result = await query({
      url: buildApiUrlFromState(
        state,
        path,
        body === undefined
          ? { project_id: args.spec.project.project_id }
          : undefined,
      ),
      method: body === undefined ? "GET" : "POST",
      ...(body === undefined ? {} : { body }),
    });
    if (result.error) return { error: result.error };
    if (
      buddyOperatorIdentity(api.getState() as State) !== args.endpoint_identity
    )
      throw new Error("Project connection changed during request");
    const hasCursor =
      !!result.data &&
      typeof result.data === "object" &&
      "next_cursor" in result.data;
    const envelope = object(result.data, [
      "schema_version",
      "project",
      "data",
      ...(hasCursor ? ["next_cursor"] : []),
    ]);
    if (
      envelope.schema_version !== 1 ||
      (hasCursor && envelope.next_cursor !== null) ||
      !equal(envelope.project, args.spec.project)
    )
      throw new Error("Resource response project mismatch");
    return { data: parse(envelope.data) };
  } catch (error) {
    return {
      error: {
        status: "CUSTOM_ERROR" as const,
        error:
          error instanceof Error ? error.message : "Invalid resource response",
      },
    };
  }
}
export const buddyResourceActionsApi = buddyApi.injectEndpoints({
  endpoints: (builder) => ({
    previewBuddyResource: builder.mutation<ResourcePreview, ResourceContext>({
      queryFn: (args, api, _options, query) =>
        request(
          args,
          api,
          query,
          "/v1/buddy/actions/preview",
          { spec: args.spec },
          (value) => parseResourcePreview(value, args.spec),
        ),
    }),
    prepareBuddyResource: builder.mutation<PreparedResource, ResourceContext>({
      queryFn: (args, api, _options, query) =>
        request(
          args,
          api,
          query,
          "/v1/buddy/actions/prepare",
          { spec: args.spec },
          (value) => parsePrepared(value, args.spec),
        ),
    }),
    rightsBuddyResource: builder.mutation<NativeRights, ResourceContext>({
      queryFn: (args, api, _options, query) =>
        request(
          args,
          api,
          query,
          "/v1/buddy/actions/rights",
          { spec: args.spec },
          parseNativeRights,
        ),
    }),
    grantBuddyResource: builder.mutation<ResourceGrant, GrantArgs>({
      queryFn: (args, api, _options, query) =>
        request(
          args,
          api,
          query,
          "/v1/buddy/actions/grant",
          {
            project_id: args.spec.project.project_id,
            spec_hash: args.spec_hash,
            native_rights: args.native_rights,
            recovery_execution_id: args.recovery_execution_id,
            expires_in_seconds: 300,
          },
          (value) => parseGrant(value, args),
        ),
    }),
    executeBuddyResource: builder.mutation<ResourceResult, ExecuteArgs>({
      queryFn: (args, api, _options, query) =>
        request(
          args,
          api,
          query,
          args.recovery_execution_id
            ? `/v1/buddy/executions/${encodeURIComponent(
                args.recovery_execution_id,
              )}/recover`
            : "/v1/buddy/actions/execute",
          {
            project_id: args.spec.project.project_id,
            spec_hash: args.spec_hash,
            grant_id: args.grant_id,
            execution_id: args.execution_id,
            idempotency_key: args.idempotency_key,
          },
          (value) =>
            args.recovery_execution_id
              ? {
                  disposition: "Finished",
                  execution: parseExecutionRecord(value, args),
                }
              : parseResourceResult(value, args),
        ),
    }),
    statusBuddyResource: builder.query<ActionExecution, ExecutionIdentity>({
      queryFn: (args, api, _options, query) =>
        request(
          args,
          api,
          query,
          `/v1/buddy/executions/${encodeURIComponent(args.execution_id)}`,
          undefined,
          (value) => {
            const row = object(value, ["value", "revision", "change_seq"]);
            if (
              [row.revision, row.change_seq].some(
                (v) =>
                  typeof v !== "number" || !Number.isSafeInteger(v) || v < 0,
              )
            )
              throw new Error("Invalid saved execution revision");
            return parseExecution(row.value, args);
          },
        ),
    }),
  }),
});
export const {
  usePreviewBuddyResourceMutation,
  usePrepareBuddyResourceMutation,
  useRightsBuddyResourceMutation,
  useGrantBuddyResourceMutation,
  useExecuteBuddyResourceMutation,
  useLazyStatusBuddyResourceQuery,
} = buddyResourceActionsApi;
