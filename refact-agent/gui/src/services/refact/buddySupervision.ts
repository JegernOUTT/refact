import { buddyApi } from "./buddy";
import { buildApiUrlFromState, type EngineApiConfig } from "./apiUrl";
import { buddyOperatorIdentity } from "./buddyOperator";
import {
  parseMandate,
  type Mandate,
  type ProjectIdentity,
  type WorkerIdentity,
  type NativeSourceIdentity,
} from "../../features/Buddy/operatorTypes";

export type SupervisionContext = {
  schema_version: 1;
  project: ProjectIdentity | null;
  worker: WorkerIdentity | null;
  features: Record<
    "storage" | "actions" | "supervision" | "improvement" | "editorial",
    boolean
  >;
  authenticated_owner: NativeSourceIdentity | null;
};
export type SupervisionArgs = {
  endpoint_identity: string;
  project: ProjectIdentity;
  worker: WorkerIdentity;
  id: string;
};
export type GoalIntake = {
  content: string;
  criteria: { id: string; text: string; verify_hint: string | null }[];
};
export type SteeringNote = { note_id: string; text: string };
export type SupervisionMode = "tool_free" | "read_only" | "engineering";
export type SupervisionScope =
  | { mode: "tool_free" }
  | { mode: "read_only" }
  | { mode: "engineering"; tools: string[]; target_files: string[] };
export type MandateRequest = {
  mandate: Mandate;
  max_workers: number;
  routine_choices: never[];
  worker_tools: string[];
  worker_target_files: string[];
  steering_notes: SteeringNote[];
};
export type GrantedScope = {
  worker_tools: string[];
  worker_target_files: string[];
  steering_notes: SteeringNote[];
};
export type ControlKind = "message" | "pause" | "resume" | "cancel";
export type ControlRecord = {
  control_id: string;
  kind: ControlKind;
  note_id: string | null;
  requested_by: NativeSourceIdentity;
  at_ms: number;
  outcome: string;
};
export type SupervisionLink = {
  mandate_id: string;
  chat_id: string;
  command_acknowledged: boolean;
  dispatch_attempted: boolean;
  revoked: boolean;
  escalation: string | null;
  delivery_run_id: string | null;
  pause_requested: boolean;
  control_history: ControlRecord[];
};
export type NativePause = {
  requested: boolean;
  quiescent: boolean;
  running_tool: string | null;
  running_tool_cancelable: boolean;
  queued_native_notes: number;
};
export type SupervisionStatus = {
  link: SupervisionLink;
  native_goal: { status: string } | null;
  native_worker: { status: string } | null;
  native_worker_goal: { status: string } | null;
  coverage: string;
  host_wake_supported: false;
  daemon_wake_supported: false;
  /** False when the native owner did not report granted operations: every
   * control is then unsupported, never silently enabled. */
  controls_supported: boolean;
  allowed_operations: string[];
  steering_notes: SteeringNote[];
  native_pause: NativePause | null;
};
/** Native read tools a `read_project` grant exposes; mirrors the engine
 * catalog exactly. Nothing else is selectable. */
export const SUPERVISED_READ_TOOLS = [
  "cat",
  "tree",
  "search_pattern",
  "glob",
] as const;
/** Native engineering edit tools an `engineer_project` grant may add; mirrors
 * the engine catalog exactly. No shell, MCP, browser or delegation tools. */
export const SUPERVISED_ENGINEERING_TOOLS = [
  "create_textdoc",
  "update_textdoc",
  "update_textdoc_regex",
  "update_textdoc_by_lines",
  "update_textdoc_anchored",
  "apply_patch",
] as const;
export const DEFAULT_ENGINEERING_TOOLS = ["apply_patch"] as const;
export const MAX_TARGET_FILES = 24;
export const MAX_STEERING_NOTES = 12;
export const MAX_CONTROL_HISTORY = 32;
const FORBIDDEN_SEGMENTS = [".git", ".refact", "target", "node_modules"];
const CONTROL_KINDS: ControlKind[] = ["message", "pause", "resume", "cancel"];
export function object(v: unknown): Record<string, unknown> {
  if (!v || typeof v !== "object" || Array.isArray(v))
    throw new Error("Invalid supervision object");
  return v as Record<string, unknown>;
}
function text(v: unknown): string {
  if (typeof v !== "string" || !v.trim())
    throw new Error("Invalid supervision text");
  return v;
}
function count(v: unknown): number {
  if (typeof v !== "number" || !Number.isSafeInteger(v) || v < 0)
    throw new Error("Invalid supervision count");
  return v;
}
function flag(v: unknown): boolean {
  if (typeof v !== "boolean") throw new Error("Invalid supervision flag");
  return v;
}
function bytes(v: string): number {
  return new TextEncoder().encode(v).length;
}
function parseSteeringNote(v: unknown): SteeringNote {
  const n = object(v);
  const note_id = text(n.note_id);
  const body = text(n.text);
  if (note_id.trim() !== note_id || bytes(note_id) > 64 || bytes(body) > 4000)
    throw new Error("Invalid steering note");
  return { note_id, text: body };
}
function parseSteeringNotes(v: unknown): SteeringNote[] {
  if (v === undefined) return [];
  if (!Array.isArray(v) || v.length > MAX_STEERING_NOTES)
    throw new Error("Invalid steering notes");
  const notes = v.map(parseSteeringNote);
  if (new Set(notes.map((n) => n.note_id)).size !== notes.length)
    throw new Error("Duplicate steering note");
  return notes;
}
function parseOperations(v: unknown): string[] {
  if (!Array.isArray(v)) throw new Error("Invalid supervision operations");
  const ops = v.map(text);
  if (new Set(ops).size !== ops.length)
    throw new Error("Duplicate supervision operation");
  return ops;
}
function parseNativePause(v: unknown): NativePause | null {
  if (v === undefined || v === null) return null;
  const p = object(v);
  return {
    requested: flag(p.requested),
    quiescent: flag(p.quiescent),
    running_tool: p.running_tool === null ? null : text(p.running_tool),
    running_tool_cancelable: flag(p.running_tool_cancelable),
    queued_native_notes: count(p.queued_native_notes),
  };
}
function parseControlHistory(v: unknown): ControlRecord[] {
  if (v === undefined) return [];
  if (!Array.isArray(v) || v.length > MAX_CONTROL_HISTORY)
    throw new Error("Invalid control history");
  return v.map((item) => {
    const c = object(item);
    const kind = text(c.kind);
    if (!CONTROL_KINDS.includes(kind as ControlKind))
      throw new Error("Unknown control kind");
    return {
      control_id: text(c.control_id),
      kind: kind as ControlKind,
      note_id: c.note_id === null ? null : text(c.note_id),
      requested_by: parseNativeSourceIdentity(c.requested_by),
      at_ms: count(c.at_ms),
      outcome: text(c.outcome),
    };
  });
}
function parseNativeSourceIdentity(v: unknown): NativeSourceIdentity {
  const source = object(v);
  if (source.source_kind !== "external")
    throw new Error("Unknown native control requester");
  return {
    source_kind: "external",
    source_id: text(source.source_id),
    source_version: text(source.source_version),
  };
}
function parseStringList(v: unknown): string[] {
  if (v === undefined) return [];
  if (!Array.isArray(v)) throw new Error("Invalid supervision list");
  return v.map(text);
}
export function parseProject(v: unknown): ProjectIdentity {
  const p = object(v);
  return {
    project_id: text(p.project_id),
    canonical_root: text(p.canonical_root),
  };
}
export function parseSupervisionContext(value: unknown): SupervisionContext {
  const v = object(value),
    f = object(v.features);
  if (v.schema_version !== 1)
    throw new Error("Unsupported supervision context");
  const w = v.worker === null ? null : object(v.worker);
  const owner =
    v.authenticated_owner === null
      ? null
      : parseNativeSourceIdentity(v.authenticated_owner);
  return {
    schema_version: 1,
    project: v.project === null ? null : parseProject(v.project),
    worker: w && {
      worker_id: text(w.worker_id),
      engine_instance_id: text(w.engine_instance_id),
      epoch: count(w.epoch),
    },
    authenticated_owner: owner,
    features: {
      storage: flag(f.storage),
      actions: flag(f.actions),
      supervision: flag(f.supervision),
      improvement: flag(f.improvement),
      editorial: flag(f.editorial),
    },
  };
}
function sameProject(value: unknown, expected: ProjectIdentity): void {
  const p = parseProject(value);
  if (
    p.project_id !== expected.project_id ||
    p.canonical_root !== expected.canonical_root
  )
    throw new Error("Supervision project changed");
}
function sameWorker(value: unknown, expected: WorkerIdentity): void {
  const worker = object(value);
  if (
    worker.worker_id !== expected.worker_id ||
    worker.engine_instance_id !== expected.engine_instance_id ||
    worker.epoch !== expected.epoch
  )
    throw new Error("Supervision worker changed");
}
export function parseSupervisionLink(
  value: unknown,
  args: SupervisionArgs,
): SupervisionLink {
  const v = object(value);
  sameProject(v.project, args.project);
  sameWorker(v.worker, args.worker);
  if (v.schema_version !== 1 || v.mandate_id !== args.id)
    throw new Error("Supervision identity changed");
  return {
    mandate_id: args.id,
    chat_id: text(v.chat_id),
    command_acknowledged: flag(v.command_acknowledged),
    dispatch_attempted: flag(v.dispatch_attempted),
    revoked: flag(v.revoked),
    escalation: v.escalation === null ? null : text(v.escalation),
    delivery_run_id:
      v.delivery_run_id === null ? null : text(v.delivery_run_id),
    pause_requested:
      v.pause_requested === undefined ? false : flag(v.pause_requested),
    control_history: parseControlHistory(v.control_history),
  };
}
export function parseSupervisionStatus(
  value: unknown,
  args: SupervisionArgs,
): SupervisionStatus {
  const v = object(value);
  sameProject(v.project, args.project);
  sameWorker(v.worker, args.worker);
  const native = (n: unknown) =>
    n === undefined || n === null ? null : { status: text(object(n).status) };
  if (v.host_wake_supported !== false || v.daemon_wake_supported !== false)
    throw new Error("Unsupported wake capability");
  const supported = v.allowed_operations !== undefined;
  return {
    link: parseSupervisionLink({ ...object(v.link), worker: v.worker }, args),
    native_goal: native(v.native_goal),
    native_worker: native(v.native_worker),
    native_worker_goal: native(v.native_worker_goal),
    coverage: text(v.coverage),
    host_wake_supported: false,
    daemon_wake_supported: false,
    controls_supported: supported,
    allowed_operations: supported ? parseOperations(v.allowed_operations) : [],
    steering_notes: parseSteeringNotes(v.steering_notes),
    native_pause: parseNativePause(v.native_pause),
  };
}
function mandateAck(
  value: unknown,
  args: SupervisionArgs,
  requested?: MandateRequest,
): { mandate: Mandate; scope: GrantedScope } {
  const record = object(value),
    v = object(record.value);
  count(record.revision);
  count(record.change_seq);
  sameProject(v.project, args.project);
  sameWorker(v.worker, args.worker);
  const mandate = parseMandate(v.mandate);
  sameProject(mandate.project, args.project);
  if (v.schema_version !== 1 || mandate.mandate_id !== args.id)
    throw new Error("Mandate identity changed");
  if (
    requested &&
    JSON.stringify(mandate) !== JSON.stringify(parseMandate(requested.mandate))
  )
    throw new Error("Native mandate changed from the requested grant");
  const scope = {
    worker_tools: parseStringList(v.worker_tools),
    worker_target_files: parseStringList(v.worker_target_files),
    steering_notes: parseSteeringNotes(v.steering_notes),
  };
  const operations = mandate.allowed_operations;
  const expectsEngineering = operations.includes("engineer_project");
  const expectsRead = operations.includes("read_project");
  const expectedTools: readonly string[] = expectsEngineering
    ? [...SUPERVISED_ENGINEERING_TOOLS]
    : [];
  if (
    scope.worker_tools.some((tool) => !expectedTools.includes(tool)) ||
    new Set(scope.worker_tools).size !== scope.worker_tools.length ||
    (!expectsEngineering && scope.worker_target_files.length) ||
    (expectsEngineering &&
      (!expectsRead ||
        !scope.worker_tools.length ||
        !mandate.allowed_authority_classes.includes("mutating") ||
        !scope.worker_target_files.length))
  )
    throw new Error("Native grant scope exceeds the bounded mandate");
  if (expectsEngineering)
    parseTargetFiles(scope.worker_target_files.join("\n"));
  if (
    requested &&
    (scope.worker_tools.join("\u0000") !==
      requested.worker_tools.join("\u0000") ||
      scope.worker_target_files.join("\u0000") !==
        requested.worker_target_files.join("\u0000") ||
      JSON.stringify(scope.steering_notes) !==
        JSON.stringify(requested.steering_notes))
  )
    throw new Error("Native grant scope changed from the requested mandate");
  return {
    mandate,
    scope,
  };
}
/** One relative path per line. Mirrors native rules (relative, non-escaping,
 * unique, 1..=24, <=512 bytes) and additionally refuses globs, dot segments
 * and protected directories so the UI never offers what native rejects. */
export function parseTargetFiles(input: string): string[] {
  const rows = input
    .split("\n")
    .map((row) => row.trim())
    .filter(Boolean);
  if (!rows.length || rows.length > MAX_TARGET_FILES)
    throw new Error(
      `List 1–${MAX_TARGET_FILES} exact repository-relative files, one per line`,
    );
  for (const row of rows) {
    const parts = row.split("/");
    if (
      bytes(row) > 512 ||
      row.startsWith("/") ||
      /[\\:*?[\]{}]/.test(row) ||
      [...row].some(
        (char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127,
      ) ||
      parts.some((p) => !p || p === "." || p === "..") ||
      parts.some((p) => FORBIDDEN_SEGMENTS.includes(p))
    )
      throw new Error(
        `Target "${row}" must be an exact relative file path without globs, traversal or .git/.refact/target/node_modules segments`,
      );
  }
  if (new Set(rows).size !== rows.length)
    throw new Error("Target files must be unique");
  return rows;
}
/** Exact engineering tool selection, catalog-ordered, non-empty, no unknowns. */
export function selectEngineeringTools(selected: readonly string[]): string[] {
  const unique = new Set(selected);
  for (const tool of unique)
    if (!(SUPERVISED_ENGINEERING_TOOLS as readonly string[]).includes(tool))
      throw new Error(
        `Tool "${tool}" is outside the native engineering catalog`,
      );
  const tools = SUPERVISED_ENGINEERING_TOOLS.filter((t) => unique.has(t));
  if (!tools.length)
    throw new Error("Select at least one native engineering edit tool");
  return tools;
}
/** One pre-authorized note per line, stable IDs N1..N12, bounded like native. */
export function buildSteeringNotes(input: string): SteeringNote[] {
  const rows = input
    .split("\n")
    .map((row) => row.trim())
    .filter(Boolean);
  if (rows.length > MAX_STEERING_NOTES || rows.some((r) => bytes(r) > 4000))
    throw new Error(
      `Provide up to ${MAX_STEERING_NOTES} steering notes, each up to 4,000 bytes`,
    );
  return rows.map((text, index) => ({ note_id: `N${index + 1}`, text }));
}
export function describeScope(
  scope: SupervisionScope,
  steering: SteeringNote[] | null,
): string {
  const steer =
    steering === null
      ? "no native pause/resume/steering controls"
      : `native pause/resume controls and ${steering.length} pre-written steering note(s) sent by ID only`;
  if (scope.mode === "tool_free")
    return `Tool-free, read-only analysis: no tools, no edits; ${steer}.`;
  if (scope.mode === "read_only")
    return `Read-only project access; native read tools ${SUPERVISED_READ_TOOLS.join(
      ", ",
    )} are implicit in read_project and not independently granted; no edits; ${steer}.`;
  return `Mutating engineering: read_project implicitly exposes ${SUPERVISED_READ_TOOLS.join(
    ", ",
  )}; explicit edit tools ${scope.tools.join(", ")}; edits limited to ${
    scope.target_files.length
  } exact file(s): ${scope.target_files.join(
    ", ",
  )}; no shell, MCP, permission changes or auto-merge; ${steer}.`;
}
export function buildSupervisionGrant(
  context: SupervisionContext,
  id: string,
  model: string,
  turns: number,
  tokens: number,
  minutes: number,
  costCents: number | null,
  expires: string,
  scope: SupervisionScope = { mode: "tool_free" },
  steering: SteeringNote[] | null = null,
): MandateRequest {
  if (!["tool_free", "read_only", "engineering"].includes(scope.mode))
    throw new Error("Unknown supervision scope");
  if (
    !context.project ||
    !context.worker ||
    !context.authenticated_owner ||
    !context.features.supervision ||
    !context.features.storage
  )
    throw new Error("Supervision is not available");
  if (
    !id.trim() ||
    !model.trim() ||
    !Number.isInteger(turns) ||
    turns < 1 ||
    turns > 100 ||
    !Number.isInteger(tokens) ||
    tokens < 1 ||
    tokens > 1_000_000 ||
    !Number.isInteger(minutes) ||
    minutes < 1 ||
    minutes > 1440 ||
    (costCents !== null &&
      (!Number.isSafeInteger(costCents) ||
        costCents < 1 ||
        !Number.isSafeInteger(costCents * 10_000)))
  )
    throw new Error(
      "Use finite turns, tokens, whole minutes and optional whole cents within the displayed limits",
    );
  const expiry = Date.parse(expires);
  const now = Date.now();
  if (!Number.isFinite(expiry) || expiry <= now || expiry - now > 86_400_000)
    throw new Error("Expiry must be in the future, within 24 hours");
  const allowed_operations = ["create_goal", "spawn_worker"];
  const allowed_authority_classes: Mandate["allowed_authority_classes"] = [
    "read_only",
  ];
  let worker_tools: string[] = [];
  let worker_target_files: string[] = [];
  if (scope.mode === "read_only") {
    allowed_operations.push("read_project");
  } else if (scope.mode === "engineering") {
    worker_tools = selectEngineeringTools(scope.tools);
    worker_target_files = parseTargetFiles(scope.target_files.join("\n"));
    if (worker_target_files.length !== scope.target_files.length)
      throw new Error("Target files must be exact and unique");
    allowed_operations.push("read_project", "engineer_project");
    allowed_authority_classes.push("mutating");
  }
  const steering_notes =
    steering === null
      ? []
      : parseSteeringNotes(steering.map((n) => ({ ...n })));
  if (steering !== null) allowed_operations.push("steer_worker");
  return {
    max_workers: 1,
    routine_choices: [],
    worker_tools,
    worker_target_files,
    steering_notes,
    mandate: {
      schema_version: 1,
      mandate_id: id,
      project: context.project,
      owner: context.authenticated_owner,
      owner_epoch: 1,
      resource_scopes: [
        {
          resource_kind: "project",
          resource_id: context.project.project_id,
          version: "1",
        },
      ],
      allowed_operations,
      allowed_authority_classes,
      model_constraints: [model.trim()],
      budget_limit: {
        max_turns: turns,
        max_tokens: tokens,
        max_duration_seconds: minutes * 60,
        max_cost_micros: costCents === null ? null : costCents * 10_000,
      },
      expires_at: new Date(expiry).toISOString(),
      revoked_at: null,
    },
  };
}
export function buildControl(
  kind: ControlKind,
  control_id: string,
  note_id?: string,
): { control_id: string; note_id?: string } {
  if (!control_id || control_id.trim() !== control_id || bytes(control_id) > 64)
    throw new Error("Invalid control identity");
  if (kind === "message") {
    if (!note_id || note_id.trim() !== note_id || bytes(note_id) > 64)
      throw new Error("Select a pre-authorized steering note");
    return { control_id, note_id };
  }
  if (note_id !== undefined) throw new Error("Only messages carry a note");
  return { control_id };
}
export function buildGoalIntake(content: string, criteria: string): GoalIntake {
  const rows = criteria
    .split("\n")
    .map((row) => row.trim())
    .filter(Boolean);
  if (
    !content.trim() ||
    new TextEncoder().encode(content).length > 16_000 ||
    !rows.length ||
    rows.length > 12 ||
    rows.some((row) => new TextEncoder().encode(row).length > 2000)
  )
    throw new Error(
      "Provide a goal (up to 16,000 bytes) and 1–12 criteria, each up to 2,000 bytes",
    );
  return {
    content,
    criteria: rows.map((text, index) => ({
      id: `C${index + 1}`,
      text,
      verify_hint: null,
    })),
  };
}
function assertIdentity(state: unknown, identity: string) {
  if (
    buddyOperatorIdentity(
      state as Parameters<typeof buddyOperatorIdentity>[0],
    ) !== identity
  )
    throw new Error("Project connection changed");
}
export function supervisionHeaders(
  args: SupervisionArgs,
): Record<string, string> {
  return {
    "X-Refact-Project-Id": args.project.project_id,
    "X-Refact-Worker-Id": args.worker.worker_id,
    "X-Refact-Engine-Instance": args.worker.engine_instance_id,
    "X-Refact-Worker-Epoch": String(args.worker.epoch),
  };
}
const path = (id: string) =>
  `/v1/buddy/supervision/mandates/${encodeURIComponent(id)}`;
export const buddySupervisionApi = buddyApi.injectEndpoints({
  endpoints: (builder) => ({
    getBuddySupervisionContext: builder.query<
      SupervisionContext,
      { endpoint_identity: string }
    >({
      queryFn: async (args, api, _options, baseQuery) => {
        try {
          assertIdentity(api.getState(), args.endpoint_identity);
          const result = await baseQuery(
            buildApiUrlFromState(
              api.getState() as { config: EngineApiConfig },
              "/v1/buddy/supervision/context",
            ),
          );
          assertIdentity(api.getState(), args.endpoint_identity);
          if (result.error) return { error: result.error };
          return { data: parseSupervisionContext(result.data) };
        } catch {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Supervision context unconfirmed",
            },
          };
        }
      },
    }),
    getBuddySupervisionStatus: builder.query<
      SupervisionStatus,
      SupervisionArgs
    >({
      queryFn: async (args, api, _options, baseQuery) => {
        try {
          assertIdentity(api.getState(), args.endpoint_identity);
          const result = await baseQuery({
            url: buildApiUrlFromState(
              api.getState() as { config: EngineApiConfig },
              `${path(args.id)}/status`,
            ),
            headers: supervisionHeaders(args),
          });
          assertIdentity(api.getState(), args.endpoint_identity);
          if (result.error) return { error: result.error };
          return { data: parseSupervisionStatus(result.data, args) };
        } catch {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Native supervision status unconfirmed",
            },
          };
        }
      },
    }),
    changeBuddySupervision: builder.mutation<
      { mandate?: Mandate; scope?: GrantedScope; link?: SupervisionLink },
      SupervisionArgs &
        (
          | { operation: "create"; request: MandateRequest }
          | { operation: "intake"; request: GoalIntake }
          | {
              operation: "message";
              request: { control_id: string; note_id: string };
            }
          | {
              operation: "pause" | "resume" | "cancel";
              request: { control_id: string };
            }
          | { operation: "revoke" | "spawn" | "reconcile" }
        )
    >({
      queryFn: async (args, api, _options, baseQuery) => {
        try {
          assertIdentity(api.getState(), args.endpoint_identity);
          if (!args.id.trim()) throw new Error("Missing mandate identity");
          if (args.operation === "create") {
            const m = parseMandate(args.request.mandate);
            sameProject(m.project, args.project);
            if (m.mandate_id !== args.id) throw new Error("Mandate mismatch");
          }
          if (args.operation === "message")
            buildControl(
              "message",
              args.request.control_id,
              args.request.note_id,
            );
          else if (
            args.operation === "pause" ||
            args.operation === "resume" ||
            args.operation === "cancel"
          )
            buildControl(args.operation, args.request.control_id);
          const result = await baseQuery({
            url: buildApiUrlFromState(
              api.getState() as { config: EngineApiConfig },
              args.operation === "create"
                ? "/v1/buddy/supervision/mandates"
                : `${path(args.id)}/${args.operation}`,
            ),
            method: "POST",
            headers: supervisionHeaders(args),
            ...("request" in args ? { body: args.request } : {}),
          });
          assertIdentity(api.getState(), args.endpoint_identity);
          if (result.error) return { error: result.error };
          return {
            data:
              args.operation === "create" ||
              (args.operation === "revoke" && "value" in object(result.data))
                ? mandateAck(
                    result.data,
                    args,
                    args.operation === "create" ? args.request : undefined,
                  )
                : { link: parseSupervisionLink(result.data, args) },
          };
        } catch {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error:
                "Supervision request unconfirmed; do not replay automatically",
            },
          };
        }
      },
    }),
  }),
});
export const {
  useGetBuddySupervisionContextQuery,
  useGetBuddySupervisionStatusQuery,
  useChangeBuddySupervisionMutation,
} = buddySupervisionApi;
