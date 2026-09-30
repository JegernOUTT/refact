import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import type { EngineApiConfig } from "./apiUrl";

import {
  InvalidOperatorContract,
  OPERATOR_SCHEMA_VERSION_V1,
  parseActionExecution,
  parseActionSpec,
  parseArtifactRevision,
  parseReadState,
  parseRunOutcome,
  parseWorkItem,
  type ActionExecution,
  type ActionSpec,
  type ArtifactRevision,
  type ProjectIdentity,
  type ReadState,
  type RunOutcome,
  type WorkItem,
  type WorkerIdentity,
} from "../../features/Buddy/operatorTypes";
import { buddyApi } from "./buddy";
import { buildApiUrlFromState, getEngineEndpointIdentity } from "./apiUrl";

type BuddyOperatorState = {
  config: EngineApiConfig & { apiKey?: string | null };
  current_project?: { workspaceRoots?: unknown; name?: unknown };
};

export function buddyOperatorIdentity(state: BuddyOperatorState): string {
  return JSON.stringify({
    endpoint: getEngineEndpointIdentity(state.config),
    roots: state.current_project?.workspaceRoots,
    project: state.current_project?.name,
  });
}

type WireEnvelope = {
  schema_version: unknown;
  project: unknown;
  data: unknown;
  next_cursor?: unknown;
};

export type OperatorPage<T> = {
  schema_version: typeof OPERATOR_SCHEMA_VERSION_V1;
  project: ProjectIdentity;
  data: T;
  next_cursor: number | null;
};

export type OperatorRecord<T> = {
  value: T;
  revision: number;
  change_seq: number;
};

export type OperatorStatus = {
  schema_version: 1;
  mode: "enabled" | "disabled" | "unavailable";
  features: Record<
    "storage" | "actions" | "supervision" | "improvement" | "editorial",
    boolean
  >;
  project: ProjectIdentity | null;
  worker: WorkerIdentity | null;
  unavailable_reason?: OperatorUnavailableReason;
};

export type OperatorUnavailableReason = {
  kind: string;
  detail: string;
  retry_in_seconds?: number;
};

function parseOperatorUnavailableReason(
  value: unknown,
): OperatorUnavailableReason {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Invalid operator unavailable reason");
  const reason = value as Record<string, unknown>;
  if (
    ![2, 3].includes(Object.keys(reason).length) ||
    Object.keys(reason).some(
      (key) => !["kind", "detail", "retry_in_seconds"].includes(key),
    ) ||
    typeof reason.kind !== "string" ||
    reason.kind.trim() === "" ||
    reason.kind.length > 64 ||
    typeof reason.detail !== "string" ||
    reason.detail.trim() === "" ||
    reason.detail.length > 2000 ||
    (reason.retry_in_seconds !== undefined &&
      (typeof reason.retry_in_seconds !== "number" ||
        !Number.isSafeInteger(reason.retry_in_seconds) ||
        reason.retry_in_seconds < 0))
  )
    throw new Error("Invalid operator unavailable reason");
  return {
    kind: reason.kind,
    detail: reason.detail,
    ...(reason.retry_in_seconds === undefined
      ? {}
      : { retry_in_seconds: reason.retry_in_seconds }),
  };
}

export function parseOperatorStatus(value: unknown): OperatorStatus {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Invalid operator status");
  const row = value as Record<string, unknown>;
  if (
    ![5, 6].includes(Object.keys(row).length) ||
    Object.keys(row).some(
      (key) =>
        ![
          "schema_version",
          "mode",
          "features",
          "project",
          "worker",
          "unavailable_reason",
        ].includes(key),
    ) ||
    row.schema_version !== 1 ||
    (row.mode !== "enabled" &&
      row.mode !== "disabled" &&
      row.mode !== "unavailable") ||
    !row.features ||
    typeof row.features !== "object" ||
    Array.isArray(row.features)
  )
    throw new Error("Unsupported operator status");
  const unavailableReason =
    row.unavailable_reason === undefined
      ? undefined
      : parseOperatorUnavailableReason(row.unavailable_reason);
  if (row.mode !== "unavailable" && unavailableReason !== undefined)
    throw new Error("Unexpected operator unavailable reason");
  const features = row.features as Record<string, unknown>;
  if (
    Object.keys(features).length !== 5 ||
    typeof features.storage !== "boolean" ||
    typeof features.actions !== "boolean" ||
    typeof features.supervision !== "boolean" ||
    typeof features.improvement !== "boolean" ||
    typeof features.editorial !== "boolean" ||
    (row.mode === "disabled" && features.storage) ||
    (row.mode === "enabled" &&
      (!features.storage || !row.project || !row.worker))
  )
    throw new Error("Unsupported operator status");
  const project = row.project === null ? null : parseProject(row.project);
  let worker: WorkerIdentity | null = null;
  if (row.worker !== null) {
    if (
      !row.worker ||
      typeof row.worker !== "object" ||
      Array.isArray(row.worker)
    )
      throw new Error("Invalid operator worker identity");
    const identity = row.worker as Record<string, unknown>;
    if (
      Object.keys(identity).length !== 3 ||
      typeof identity.worker_id !== "string" ||
      !identity.worker_id.trim() ||
      typeof identity.engine_instance_id !== "string" ||
      !identity.engine_instance_id.trim() ||
      typeof identity.epoch !== "number" ||
      !Number.isSafeInteger(identity.epoch) ||
      identity.epoch < 0
    )
      throw new Error("Invalid operator worker identity");
    worker = {
      worker_id: identity.worker_id,
      engine_instance_id: identity.engine_instance_id,
      epoch: identity.epoch,
    };
  }
  return {
    schema_version: 1,
    mode: row.mode,
    project,
    worker,
    features: {
      storage: features.storage,
      actions: features.actions,
      supervision: features.supervision,
      improvement: features.improvement,
      editorial: features.editorial,
    },
    ...(unavailableReason === undefined
      ? {}
      : { unavailable_reason: unavailableReason }),
  };
}

export type WorkListArgs = {
  project_id?: string;
  endpoint_identity?: string;
  cursor?: number;
  limit?: number;
  view?: "needs_you" | "working_on" | "delivered" | "watching";
  search?: string;
  attention?: "all" | "unread" | "snoozed" | "unsnoozed";
};
export type OperatorDetailArgs = {
  id: string;
  project_id?: string;
  endpoint_identity?: string;
};
export type ArtifactDetailArgs = OperatorDetailArgs & { revision?: number };

export type WorkSnooze = {
  schema_version: 1;
  project: ProjectIdentity;
  work_id: string;
  artifact_id: string;
  artifact_revision: number;
  until: string;
};

type WorkAttentionArgs = OperatorDetailArgs & {
  project_id: string;
  canonical_root: string;
};

function parseWorkSnooze(
  value: unknown,
  args: WorkAttentionArgs,
): WorkSnooze | null {
  if (value === null) return null;
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new InvalidOperatorContract("invalid work snooze");
  const row = value as Record<string, unknown>;
  const project = parseProject(row.project);
  if (
    Object.keys(row).length !== 6 ||
    row.schema_version !== 1 ||
    row.work_id !== args.id ||
    project.project_id !== args.project_id ||
    project.canonical_root !== args.canonical_root ||
    typeof row.artifact_id !== "string" ||
    row.artifact_id.trim() === "" ||
    typeof row.artifact_revision !== "number" ||
    !Number.isSafeInteger(row.artifact_revision) ||
    row.artifact_revision < 1 ||
    typeof row.until !== "string" ||
    !Number.isFinite(Date.parse(row.until))
  )
    throw new InvalidOperatorContract("invalid work snooze identity");
  return {
    schema_version: 1,
    project,
    work_id: args.id,
    artifact_id: row.artifact_id,
    artifact_revision: row.artifact_revision,
    until: row.until,
  };
}

export type DeskAction = {
  /** Origin is independent of the target resource; older evidence-only servers omit it. */
  work_id?: string;
  spec: ActionSpec;
  spec_hash: string;
  work_revision: number;
  artifact_id: string;
  artifact_revision: number;
  label: string;
  description: string;
  latest_execution: OperatorRecord<ActionExecution> | null;
};

export type DeskActionBatchEntry = {
  work_id: string;
  actions: DeskAction[];
  error: string | null;
};

export type DeskActionBatchArgs = {
  ids: string[];
  project_id: string;
  endpoint_identity: string;
};

export type DeskActionExecuteArgs = OperatorDetailArgs & {
  action_id: string;
  project_id: string;
  canonical_root: string;
  work_revision: number;
  artifact_id: string;
  artifact_revision: number;
  spec_hash: string;
  idempotency_key: string;
};

function parseDeskAction(
  value: unknown,
  projectId: string,
  workId: string,
): DeskAction {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new InvalidOperatorContract("invalid Desk action");
  }
  const v = value as Record<string, unknown>;
  if (
    ![8, 9].includes(Object.keys(v).length) ||
    Object.keys(v).some(
      (key) =>
        ![
          "spec",
          "spec_hash",
          "work_id",
          "work_revision",
          "artifact_id",
          "artifact_revision",
          "label",
          "description",
          "latest_execution",
        ].includes(key),
    ) ||
    ["spec_hash", "artifact_id", "label", "description"].some(
      (key) => typeof v[key] !== "string" || v[key].trim() === "",
    ) ||
    ["work_revision", "artifact_revision"].some(
      (key) =>
        typeof v[key] !== "number" ||
        !Number.isSafeInteger(v[key]) ||
        v[key] <= 0,
    )
  ) {
    throw new InvalidOperatorContract("invalid Desk action metadata");
  }
  const spec = parseActionSpec(v.spec);
  const latest =
    v.latest_execution === null
      ? null
      : parseRecord(v.latest_execution, parseActionExecution);
  if (
    spec.project.project_id !== projectId ||
    (v.work_id === undefined
      ? !["operator.saved_evidence", "operator.work_lifecycle"].includes(
          spec.adapter,
        ) || spec.resource.resource_id !== workId
      : v.work_id !== workId) ||
    (spec.adapter === "operator.saved_evidence" &&
      spec.resource.resource_id !== workId) ||
    (spec.adapter === "operator.work_lifecycle" &&
      spec.resource.resource_id !== workId) ||
    (latest &&
      (latest.value.spec_hash !== v.spec_hash ||
        latest.value.action_id !== spec.action_id ||
        latest.value.project.project_id !== projectId ||
        latest.value.project.canonical_root !== spec.project.canonical_root))
  ) {
    throw new InvalidOperatorContract("Desk action identity mismatch");
  }
  return {
    work_id: workId,
    spec,
    spec_hash: v.spec_hash as string,
    work_revision: v.work_revision as number,
    artifact_id: v.artifact_id as string,
    artifact_revision: v.artifact_revision as number,
    label: v.label as string,
    description: v.description as string,
    latest_execution: latest,
  };
}

function parseProject(value: unknown): ProjectIdentity {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new InvalidOperatorContract(
      "operator envelope project must be an object",
    );
  }
  const project = value as Record<string, unknown>;
  if (
    typeof project.project_id !== "string" ||
    project.project_id.trim() === "" ||
    typeof project.canonical_root !== "string" ||
    project.canonical_root.trim() === "" ||
    Object.keys(project).length !== 2
  ) {
    throw new InvalidOperatorContract("invalid operator envelope project");
  }
  return {
    project_id: project.project_id,
    canonical_root: project.canonical_root,
  };
}

function parseEnvelope<T>(
  value: unknown,
  parseData: (value: unknown) => T,
  expectedProjectId?: string,
): OperatorPage<T> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new InvalidOperatorContract("operator response must be an object");
  }
  const envelope = value as WireEnvelope;
  const keys = Object.keys(envelope);
  if (
    !keys.every((key) =>
      ["schema_version", "project", "data", "next_cursor"].includes(key),
    ) ||
    !keys.includes("schema_version") ||
    !keys.includes("project") ||
    !keys.includes("data") ||
    envelope.schema_version !== OPERATOR_SCHEMA_VERSION_V1
  ) {
    throw new InvalidOperatorContract("unsupported operator response envelope");
  }
  const project = parseProject(envelope.project);
  if (
    expectedProjectId !== undefined &&
    project.project_id !== expectedProjectId
  ) {
    throw new InvalidOperatorContract(
      "operator response project does not match request",
    );
  }
  const cursor = envelope.next_cursor;
  if (
    cursor !== undefined &&
    cursor !== null &&
    (typeof cursor !== "number" || !Number.isSafeInteger(cursor) || cursor < 0)
  ) {
    throw new InvalidOperatorContract("invalid operator response cursor");
  }
  const data = parseData(envelope.data);
  const records: unknown[] = Array.isArray(data) ? data : [data];
  for (const record of records) {
    if (typeof record !== "object" || record === null || !("value" in record))
      continue;
    const value: unknown = record.value;
    if (typeof value !== "object" || value === null || !("project" in value))
      continue;
    const itemProject = parseProject(value.project);
    if (
      itemProject.project_id !== project.project_id ||
      itemProject.canonical_root !== project.canonical_root
    ) {
      throw new InvalidOperatorContract(
        "operator record project does not match envelope",
      );
    }
  }
  return {
    schema_version: OPERATOR_SCHEMA_VERSION_V1,
    project,
    data,
    next_cursor: cursor ?? null,
  };
}

function invalidResponse(error: unknown): { error: FetchBaseQueryError } {
  return {
    error: {
      status: "PARSING_ERROR",
      originalStatus: 200,
      data: "invalid operator response",
      error: error instanceof Error ? error.message : String(error),
    },
  };
}

function pathWithId(prefix: string, id: string): string {
  return `${prefix}/${encodeURIComponent(id)}`;
}

function parseRecord<T>(
  value: unknown,
  parseValue: (value: unknown) => T,
): OperatorRecord<T> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new InvalidOperatorContract("operator record must be an object");
  }
  const record = value as Record<string, unknown>;
  if (
    Object.keys(record).length !== 3 ||
    !("value" in record) ||
    typeof record.revision !== "number" ||
    !Number.isSafeInteger(record.revision) ||
    record.revision < 0 ||
    typeof record.change_seq !== "number" ||
    !Number.isSafeInteger(record.change_seq) ||
    record.change_seq < 0
  ) {
    throw new InvalidOperatorContract("invalid operator record metadata");
  }
  return {
    value: parseValue(record.value),
    revision: record.revision,
    change_seq: record.change_seq,
  };
}

export const buddyOperatorApi = buddyApi.injectEndpoints({
  endpoints: (builder) => ({
    getBuddyOperatorStatus: builder.query<OperatorStatus, string>({
      queryFn: async (identity, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        if (identity !== buddyOperatorIdentity(state))
          return invalidResponse(new Error("Operator connection changed"));
        const result = await baseQuery(
          buildApiUrlFromState(state, "/v1/buddy/operator/status"),
        );
        if (result.error) return { error: result.error };
        try {
          if (
            identity !==
            buddyOperatorIdentity(api.getState() as BuddyOperatorState)
          )
            throw new Error("Operator connection changed");
          return { data: parseOperatorStatus(result.data) };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    reopenBuddyOperator: builder.mutation<OperatorStatus, string>({
      queryFn: async (identity, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        if (identity !== buddyOperatorIdentity(state))
          return invalidResponse(new Error("Operator connection changed"));
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/buddy/operator/reopen"),
          method: "POST",
        });
        if (result.error) return { error: result.error };
        try {
          if (
            identity !==
            buddyOperatorIdentity(api.getState() as BuddyOperatorState)
          )
            throw new Error("Operator connection changed");
          return { data: parseOperatorStatus(result.data) };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    getBuddyLegacyDraftWork: builder.query<
      OperatorPage<OperatorRecord<WorkItem>>,
      OperatorDetailArgs
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        if (args.endpoint_identity !== buddyOperatorIdentity(state))
          return invalidResponse(new Error("Operator connection changed"));
        const result = await baseQuery(
          buildApiUrlFromState(
            state,
            `${pathWithId("/v1/buddy/operator/legacy-drafts", args.id)}/work`,
            { project_id: args.project_id },
          ),
        );
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (value) => parseRecord(value, parseWorkItem),
            args.project_id,
          );
          if (
            data.data.value.subject.resource_kind !== "legacy_draft" ||
            data.data.value.subject.resource_id !== args.id
          )
            throw new Error("Legacy proposal identity mismatch");
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    getBuddyWorkOutcomes: builder.query<
      OperatorPage<OperatorRecord<RunOutcome>[]>,
      OperatorDetailArgs & { cursor?: number }
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const result = await baseQuery(
          buildApiUrlFromState(
            api.getState() as BuddyOperatorState,
            `${pathWithId("/v1/buddy/work", args.id)}/outcomes`,
            { project_id: args.project_id, cursor: args.cursor, limit: 20 },
          ),
        );
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (value) => {
              if (!Array.isArray(value))
                throw new InvalidOperatorContract("invalid work run history");
              return value.map((row) => parseRecord(row, parseRunOutcome));
            },
            args.project_id,
          );
          if (data.data.some((row) => row.value.work_id !== args.id))
            throw new InvalidOperatorContract(
              "work run history identity mismatch",
            );
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    getBuddyWorkSnooze: builder.query<
      OperatorPage<WorkSnooze | null>,
      WorkAttentionArgs
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const result = await baseQuery(
          buildApiUrlFromState(
            api.getState() as BuddyOperatorState,
            `${pathWithId("/v1/buddy/work", args.id)}/snooze`,
            { project_id: args.project_id },
          ),
        );
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (value) => parseWorkSnooze(value, args),
            args.project_id,
          );
          if (data.project.canonical_root !== args.canonical_root)
            throw new InvalidOperatorContract("work snooze project mismatch");
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    snoozeBuddyWork: builder.mutation<
      OperatorPage<WorkSnooze | null>,
      WorkAttentionArgs & {
        artifact_id: string;
        artifact_revision: number;
        minutes: number | null;
      }
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        if (args.endpoint_identity !== buddyOperatorIdentity(state))
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Project connection changed; reload the report.",
            },
          };
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `${pathWithId("/v1/buddy/work", args.id)}/snooze`,
          ),
          method: "POST",
          body: {
            project_id: args.project_id,
            canonical_root: args.canonical_root,
            artifact_id: args.artifact_id,
            artifact_revision: args.artifact_revision,
            minutes: args.minutes,
          },
        });
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (value) => parseWorkSnooze(value, args),
            args.project_id,
          );
          if (
            data.project.canonical_root !== args.canonical_root ||
            (args.minutes === null ? data.data !== null : data.data === null) ||
            (data.data &&
              (data.data.artifact_id !== args.artifact_id ||
                data.data.artifact_revision !== args.artifact_revision))
          )
            throw new InvalidOperatorContract(
              "work snooze acknowledgment mismatch",
            );
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    getBuddyRunOutcome: builder.query<
      OperatorPage<OperatorRecord<RunOutcome>>,
      OperatorDetailArgs
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const result = await baseQuery(
          buildApiUrlFromState(
            api.getState() as BuddyOperatorState,
            pathWithId("/v1/buddy/outcomes", args.id),
            { project_id: args.project_id },
          ),
        );
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (value) => parseRecord(value, parseRunOutcome),
            args.project_id,
          );
          if (data.data.value.run_id !== args.id)
            throw new InvalidOperatorContract("run identity mismatch");
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    getBuddyArtifactHistory: builder.query<
      OperatorPage<OperatorRecord<ArtifactRevision>[]>,
      OperatorDetailArgs & { cursor?: number }
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const result = await baseQuery(
          buildApiUrlFromState(
            api.getState() as BuddyOperatorState,
            `${pathWithId("/v1/buddy/artifacts", args.id)}/revisions`,
            { project_id: args.project_id, cursor: args.cursor, limit: 20 },
          ),
        );
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (value) => {
              if (!Array.isArray(value))
                throw new InvalidOperatorContract("invalid report history");
              return value.map((record) =>
                parseRecord(record, parseArtifactRevision),
              );
            },
            args.project_id,
          );
          if (data.data.some((record) => record.value.artifact_id !== args.id))
            throw new InvalidOperatorContract(
              "artifact history identity mismatch",
            );
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    getBuddyWorkReadState: builder.query<
      OperatorPage<ReadState | null>,
      OperatorDetailArgs & { project_id: string; canonical_root: string }
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        const result = await baseQuery(
          buildApiUrlFromState(
            state,
            `${pathWithId("/v1/buddy/work", args.id)}/read`,
            { project_id: args.project_id },
          ),
        );
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (value) => (value === null ? null : parseReadState(value)),
            args.project_id,
          );
          if (
            data.project.canonical_root !== args.canonical_root ||
            (data.data &&
              (data.data.work_id !== args.id ||
                data.data.project.project_id !== args.project_id ||
                data.data.project.canonical_root !== args.canonical_root))
          )
            throw new InvalidOperatorContract("read-state identity mismatch");
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    markBuddyWorkRead: builder.mutation<
      OperatorPage<ReadState>,
      OperatorDetailArgs & {
        project_id: string;
        canonical_root: string;
        artifact_id: string;
        artifact_revision: number;
      }
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        if (args.endpoint_identity !== buddyOperatorIdentity(state)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Project connection changed; reload the report.",
            },
          };
        }
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `${pathWithId("/v1/buddy/work", args.id)}/read`,
          ),
          method: "POST",
          body: {
            project_id: args.project_id,
            artifact_id: args.artifact_id,
            artifact_revision: args.artifact_revision,
          },
        });
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            parseReadState,
            args.project_id,
          );
          if (
            data.project.canonical_root !== args.canonical_root ||
            data.data.project.project_id !== args.project_id ||
            data.data.project.canonical_root !== args.canonical_root ||
            data.data.work_id !== args.id ||
            data.data.artifact_id !== args.artifact_id ||
            data.data.last_read_revision !== args.artifact_revision
          )
            throw new InvalidOperatorContract("read-state identity mismatch");
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    getBuddyDeskActions: builder.query<
      OperatorPage<DeskAction[]>,
      OperatorDetailArgs & { project_id: string }
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        const result = await baseQuery(
          buildApiUrlFromState(
            state,
            `${pathWithId("/v1/buddy/work", args.id)}/actions`,
            { project_id: args.project_id },
          ),
        );
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (data) => {
              if (!Array.isArray(data))
                throw new InvalidOperatorContract("invalid Desk actions list");
              return data.map((value) =>
                parseDeskAction(value, args.project_id, args.id),
              );
            },
            args.project_id,
          );
          if (
            data.data.some(
              (action) =>
                action.spec.project.canonical_root !==
                data.project.canonical_root,
            )
          ) {
            throw new InvalidOperatorContract("Desk action project mismatch");
          }
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    getBuddyDeskActionsBatch: builder.query<
      OperatorPage<DeskActionBatchEntry[]>,
      DeskActionBatchArgs
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        const result = await baseQuery(
          buildApiUrlFromState(state, "/v1/buddy/work/actions", {
            project_id: args.project_id,
            ids: args.ids.join(","),
          }),
        );
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (value) => {
              if (!Array.isArray(value))
                throw new InvalidOperatorContract("invalid Desk actions batch");
              const seen = new Set<string>();
              return value.map((entry) => {
                if (!entry || typeof entry !== "object" || Array.isArray(entry))
                  throw new InvalidOperatorContract(
                    "invalid Desk actions entry",
                  );
                const row = entry as Record<string, unknown>;
                if (
                  Object.keys(row).length !== 3 ||
                  Object.keys(row).some(
                    (key) => !["work_id", "actions", "error"].includes(key),
                  ) ||
                  typeof row.work_id !== "string" ||
                  !args.ids.includes(row.work_id) ||
                  seen.has(row.work_id) ||
                  !Array.isArray(row.actions) ||
                  (row.error !== null && typeof row.error !== "string")
                )
                  throw new InvalidOperatorContract(
                    "invalid Desk actions entry",
                  );
                const workId = row.work_id;
                const actions = row.actions;
                const entryError = row.error;
                seen.add(workId);
                return {
                  work_id: workId,
                  actions: actions.map((action) =>
                    parseDeskAction(action, args.project_id, workId),
                  ),
                  error: entryError,
                };
              });
            },
            args.project_id,
          );
          if (
            data.data.some((entry) =>
              entry.actions.some(
                (action) =>
                  action.spec.project.canonical_root !==
                  data.project.canonical_root,
              ),
            )
          )
            throw new InvalidOperatorContract("Desk action project mismatch");
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
      serializeQueryArgs: ({ endpointName, queryArgs }) =>
        !Array.isArray(queryArgs.ids)
          ? endpointName
          : `${endpointName}:${JSON.stringify({
              endpoint_identity: queryArgs.endpoint_identity,
              project_id: queryArgs.project_id,
              ids: [...queryArgs.ids].sort(),
            })}`,
      providesTags: ["BuddyDeskActions"],
    }),
    executeBuddyDeskAction: builder.mutation<
      OperatorPage<OperatorRecord<ActionExecution>>,
      DeskActionExecuteArgs
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        if (
          args.endpoint_identity !== undefined &&
          args.endpoint_identity !== buddyOperatorIdentity(state)
        ) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Project connection changed; reload the action.",
            },
          };
        }
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `${pathWithId(
              "/v1/buddy/work",
              args.id,
            )}/actions/${encodeURIComponent(args.action_id)}/execute`,
          ),
          method: "POST",
          body: {
            project_id: args.project_id,
            work_revision: args.work_revision,
            artifact_id: args.artifact_id,
            artifact_revision: args.artifact_revision,
            spec_hash: args.spec_hash,
            idempotency_key: args.idempotency_key,
          },
        });
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (value) => parseRecord(value, parseActionExecution),
            args.project_id,
          );
          if (
            data.data.value.action_id !== args.action_id ||
            data.project.canonical_root !== args.canonical_root ||
            data.data.value.spec_hash !== args.spec_hash ||
            data.data.value.idempotency_key !== args.idempotency_key
          ) {
            throw new InvalidOperatorContract(
              "Desk execution identity mismatch",
            );
          }
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
      invalidatesTags: ["BuddySnapshot", "BuddyDeskActions"],
    }),
    getBuddyOperatorWork: builder.query<
      OperatorPage<OperatorRecord<WorkItem>[]>,
      WorkListArgs | undefined
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        const result = await baseQuery(
          buildApiUrlFromState(state, "/v1/buddy/work", {
            project_id: args?.project_id,
            cursor: args?.cursor,
            limit: args?.limit,
            view: args?.view,
            search: args?.search,
            attention: args?.attention,
          }),
        );
        if (result.error) return { error: result.error };
        try {
          return {
            data: parseEnvelope(
              result.data,
              (data) => {
                if (!Array.isArray(data)) {
                  throw new InvalidOperatorContract(
                    "operator work data must be an array",
                  );
                }
                return data.map((item) => parseRecord(item, parseWorkItem));
              },
              args?.project_id,
            ),
          };
        } catch (error) {
          return invalidResponse(error);
        }
      },
      providesTags: ["BuddySnapshot"],
    }),
    getBuddyOperatorWorkItem: builder.query<
      OperatorPage<OperatorRecord<WorkItem>>,
      OperatorDetailArgs
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        const result = await baseQuery(
          buildApiUrlFromState(state, pathWithId("/v1/buddy/work", args.id), {
            project_id: args.project_id,
          }),
        );
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (data) => parseRecord(data, parseWorkItem),
            args.project_id,
          );
          if (data.data.value.id !== args.id)
            throw new InvalidOperatorContract("work identity mismatch");
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    getBuddyOperatorArtifact: builder.query<
      OperatorPage<OperatorRecord<ArtifactRevision>>,
      ArtifactDetailArgs
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        const result = await baseQuery(
          buildApiUrlFromState(
            state,
            pathWithId("/v1/buddy/artifacts", args.id),
            { project_id: args.project_id, revision: args.revision },
          ),
        );
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (data) => parseRecord(data, parseArtifactRevision),
            args.project_id,
          );
          if (
            data.data.value.artifact_id !== args.id ||
            (args.revision !== undefined &&
              data.data.value.revision !== args.revision)
          )
            throw new InvalidOperatorContract("artifact identity mismatch");
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
    getBuddyOperatorExecution: builder.query<
      OperatorPage<OperatorRecord<ActionExecution>>,
      OperatorDetailArgs
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyOperatorState;
        const result = await baseQuery(
          buildApiUrlFromState(
            state,
            pathWithId("/v1/buddy/executions", args.id),
            { project_id: args.project_id },
          ),
        );
        if (result.error) return { error: result.error };
        try {
          const data = parseEnvelope(
            result.data,
            (data) => parseRecord(data, parseActionExecution),
            args.project_id,
          );
          if (data.data.value.execution_id !== args.id)
            throw new InvalidOperatorContract("execution identity mismatch");
          return { data };
        } catch (error) {
          return invalidResponse(error);
        }
      },
    }),
  }),
});

export const {
  useGetBuddyLegacyDraftWorkQuery,
  useGetBuddyOperatorStatusQuery,
  useReopenBuddyOperatorMutation,
  useGetBuddyWorkOutcomesQuery,
  useGetBuddyWorkSnoozeQuery,
  useSnoozeBuddyWorkMutation,
  useGetBuddyRunOutcomeQuery,
  useGetBuddyArtifactHistoryQuery,
  useGetBuddyWorkReadStateQuery,
  useMarkBuddyWorkReadMutation,
  useGetBuddyDeskActionsQuery,
  useGetBuddyDeskActionsBatchQuery,
  useExecuteBuddyDeskActionMutation,
  useGetBuddyOperatorWorkQuery,
  useGetBuddyOperatorWorkItemQuery,
  useGetBuddyOperatorArtifactQuery,
  useGetBuddyOperatorExecutionQuery,
} = buddyOperatorApi;
