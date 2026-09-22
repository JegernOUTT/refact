export const OPERATOR_SCHEMA_VERSION_V1 = 1 as const;
type JsonRecord = Record<string, unknown>;

export type ProjectIdentity = { project_id: string; canonical_root: string };
export type WorkerIdentity = {
  worker_id: string;
  engine_instance_id: string;
  epoch: number;
};
export type NativeSourceKind =
  | "chat"
  | "goal"
  | "task"
  | "tool"
  | "question"
  | "approval"
  | "file"
  | "commit"
  | "check"
  | "external"
  | "workflow";
export type NativeSourceIdentity = {
  source_kind: NativeSourceKind;
  source_id: string;
  source_version: string;
};
export type ResourceRef = {
  resource_kind: string;
  resource_id: string;
  version: string;
};
export type WorkItemState =
  | "needs_attention"
  | "working"
  | "delivered"
  | "watching"
  | "blocked"
  | "cancelled";
export type WorkItemSeverity =
  | "unknown"
  | "low"
  | "normal"
  | "high"
  | "critical";
export type WorkItem = {
  schema_version: number;
  id: string;
  project: ProjectIdentity;
  subject: ResourceRef;
  kind: string;
  title: string;
  state: WorkItemState;
  native_sources: NativeSourceIdentity[];
  current_artifact_id: string | null;
  current_artifact_revision: number | null;
  last_material_change_at: string;
  severity?: WorkItemSeverity | null;
  problem_summary?: string | null;
  scope?: string | null;
};
export type RunStatus =
  | "no_change"
  | "partial"
  | "inconclusive"
  | "blocked"
  | "failed"
  | "cancelled"
  | "completed";
export type EvidenceRef = {
  evidence_id: string;
  source: NativeSourceIdentity;
  locator: string;
  observed_at: string;
  digest: string | null;
};
export type EffectState = "applied" | "skipped" | "failed" | "unknown";
export type ResourceEffect = {
  resource: ResourceRef;
  state: EffectState;
  evidence_refs: string[];
};
export type RunOutcome = {
  schema_version: number;
  run_id: string;
  work_id: string;
  parent_run_id: string | null;
  project: ProjectIdentity;
  worker: WorkerIdentity;
  origin: string;
  workflow: string | null;
  input_fingerprint: string;
  coverage: string;
  started_at: string;
  ended_at: string | null;
  status: RunStatus;
  evidence: EvidenceRef[];
  effects: ResourceEffect[];
  failure_class: string | null;
};
export type ArtifactProvenance = {
  source_run_ids: string[];
  evidence_refs: string[];
  contains_untrusted_source: boolean;
};
export type ArtifactRevision = {
  schema_version: number;
  artifact_id: string;
  revision: number;
  project: ProjectIdentity;
  source_run_ids: string[];
  title: string;
  conclusion: string;
  delta: string;
  body: string;
  uncertainty: string | null;
  proposed_action_ids: string[];
  evidence: EvidenceRef[];
  provenance: ArtifactProvenance;
};
export type ActionCapabilities = {
  can_preview: boolean;
  can_execute: boolean;
  can_verify: boolean;
  can_recover: boolean;
  can_cancel: boolean;
};
export type ActionAvailability =
  | "available"
  | "unavailable"
  | "stale"
  | "unsupported";
export type ConfirmationClass = "none" | "user_confirmation_required";
export type AuthorityClass =
  | "navigation"
  | "advisory"
  | "read_only"
  | "mutating"
  | "data_egress"
  | "credential_change"
  | "publication"
  | "deployment";
export type ActionSpec = {
  schema_version: number;
  action_id: string;
  kind: string;
  adapter: string;
  project: ProjectIdentity;
  resource: ResourceRef;
  expected_version: string;
  parameter_ref: string | null;
  availability: ActionAvailability;
  confirmation: ConfirmationClass;
  authority_class: AuthorityClass;
  capabilities: ActionCapabilities;
};
export type ActionExecutionState =
  | "prepared"
  | "awaiting_confirmation"
  | "claimed"
  | "running"
  | "reconciling"
  | "outcome_unknown"
  | "completed"
  | "partial"
  | "blocked"
  | "failed"
  | "cancelled";
export type ActionExecution = {
  schema_version: number;
  execution_id: string;
  project: ProjectIdentity;
  worker: WorkerIdentity;
  idempotency_key: string;
  action_id: string;
  spec_hash: string;
  requester: NativeSourceIdentity;
  decision: DecisionReference | null;
  status: ActionExecutionState;
  started_at: string | null;
  ended_at: string | null;
  effects: ResourceEffect[];
  verification_evidence: string[];
  recovery: RecoveryRef | null;
};
export type RecoveryRef = {
  recovery_id: string;
  action_id: string;
  expected_post_version: string;
  available: boolean;
};
export type DecisionReference = {
  project: ProjectIdentity;
  worker: WorkerIdentity;
  native: NativeSourceIdentity;
};
export type DecisionOption = { option_id: string; label: string };
export type DecisionState =
  | "pending"
  | "acknowledged"
  | "resolved"
  | "expired"
  | "superseded"
  | "unsupported";
export type DecisionProjection = {
  schema_version: number;
  decision: DecisionReference;
  title: string;
  impact: string;
  options: DecisionOption[];
  recommended_option_id: string | null;
  status: DecisionState;
  response_authority: AuthorityClass;
  expires_at: string | null;
  revalidation_required: boolean;
};
export type BudgetLimit = {
  max_tokens: number | null;
  max_turns: number | null;
  max_cost_micros: number | null;
  max_duration_seconds: number | null;
};
export type Mandate = {
  schema_version: number;
  mandate_id: string;
  project: ProjectIdentity;
  owner: NativeSourceIdentity;
  owner_epoch: number;
  resource_scopes: ResourceRef[];
  allowed_operations: string[];
  allowed_authority_classes: AuthorityClass[];
  model_constraints: string[];
  budget_limit: BudgetLimit | null;
  expires_at: string | null;
  revoked_at: string | null;
};
export type DeliveryState =
  | "pending"
  | "delivered"
  | "seen"
  | "read"
  | "snoozed"
  | "muted";
export type Delivery = {
  schema_version: number;
  delivery_id: string;
  project: ProjectIdentity;
  work_id: string;
  artifact_id: string;
  artifact_revision: number;
  surface: string;
  client_id: string;
  state: DeliveryState;
  delivered_at: string | null;
  seen_at: string | null;
  attention_category: string;
};
export type ReadState = {
  schema_version: number;
  project: ProjectIdentity;
  work_id: string;
  artifact_id: string;
  last_read_revision: number;
  read_at: string;
  snoozed_until: string | null;
  muted: boolean;
};
export type ImprovementCandidate = {
  schema_version: number;
  candidate_id: string;
  project: ProjectIdentity;
  target: string;
  worktree_ref: string | null;
  config_draft_ref: string | null;
  patch_digest: string | null;
  build_identity: string | null;
  evaluation_spec_version: string;
  proof_refs: string[];
  activation_decision: DecisionReference | null;
  previous_known_good_identity: string | null;
};

export class UnsupportedOperatorSchemaVersion extends Error {
  constructor(readonly version: number) {
    super(`unsupported operator schema version: ${version}`);
  }
}
export class InvalidOperatorContract extends Error {}

const sourceKinds = [
  "chat",
  "goal",
  "task",
  "tool",
  "question",
  "approval",
  "file",
  "commit",
  "check",
  "external",
  "workflow",
] as const;
const authorityClasses = [
  "navigation",
  "advisory",
  "read_only",
  "mutating",
  "data_egress",
  "credential_change",
  "publication",
  "deployment",
] as const;
const states = {
  work: [
    "needs_attention",
    "working",
    "delivered",
    "watching",
    "blocked",
    "cancelled",
  ],
  workSeverity: ["unknown", "low", "normal", "high", "critical"],
  run: [
    "no_change",
    "partial",
    "inconclusive",
    "blocked",
    "failed",
    "cancelled",
    "completed",
  ],
  effect: ["applied", "skipped", "failed", "unknown"],
  availability: ["available", "unavailable", "stale", "unsupported"],
  confirmation: ["none", "user_confirmation_required"],
  execution: [
    "prepared",
    "awaiting_confirmation",
    "claimed",
    "running",
    "reconciling",
    "outcome_unknown",
    "completed",
    "partial",
    "blocked",
    "failed",
    "cancelled",
  ],
  decision: [
    "pending",
    "acknowledged",
    "resolved",
    "expired",
    "superseded",
    "unsupported",
  ],
  delivery: ["pending", "delivered", "seen", "read", "snoozed", "muted"],
} as const;

function record(value: unknown): JsonRecord {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new InvalidOperatorContract("expected object");
  return value as JsonRecord;
}
function text(value: unknown, identity = false): string {
  if (typeof value !== "string" || (identity && value.trim() === ""))
    throw new InvalidOperatorContract("expected non-empty string");
  return value;
}
function nullableText(value: unknown): string | null {
  return value === undefined || value === null ? null : text(value);
}
function boolean(value: unknown): boolean {
  if (typeof value !== "boolean")
    throw new InvalidOperatorContract("expected boolean");
  return value;
}
function integer(value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0)
    throw new InvalidOperatorContract("expected non-negative safe integer");
  return value;
}
function u32(value: unknown): number {
  const parsed = integer(value);
  if (parsed > 0xffffffff)
    throw new InvalidOperatorContract("integer exceeds u32 range");
  return parsed;
}
function nullableInteger(value: unknown): number | null {
  return value === undefined || value === null ? null : integer(value);
}
function nullableU32(value: unknown): number | null {
  return value === undefined || value === null ? null : u32(value);
}
function nullableEnumValue<T extends string>(
  value: unknown,
  values: readonly T[],
): T | null {
  return value === undefined || value === null
    ? null
    : enumValue(value, values);
}
function exact<T>(raw: JsonRecord, parsed: T): T {
  const parsedRecord = parsed as JsonRecord;
  for (const key of Object.keys(raw)) {
    if (!(key in parsedRecord))
      throw new InvalidOperatorContract(`unknown field: ${key}`);
    const input = raw[key];
    const output = parsedRecord[key];
    if (
      input &&
      typeof input === "object" &&
      output &&
      typeof output === "object"
    ) {
      if (Array.isArray(input) !== Array.isArray(output))
        throw new InvalidOperatorContract("invalid field shape");
      if (Array.isArray(input) && Array.isArray(output)) {
        input.forEach((item, index) => {
          if (item && typeof item === "object" && !Array.isArray(item))
            exact(record(item), output[index]);
        });
      } else if (!Array.isArray(input) && !Array.isArray(output)) {
        exact(record(input), output as JsonRecord);
      }
    }
  }
  return parsed;
}
function list<T>(value: unknown, parser: (item: unknown) => T): T[] {
  if (!Array.isArray(value))
    throw new InvalidOperatorContract("expected array");
  return value.map(parser);
}
function enumValue<T extends readonly string[]>(
  value: unknown,
  values: T,
): T[number] {
  if (typeof value !== "string" || !values.includes(value))
    throw new InvalidOperatorContract("invalid enum value");
  return value as T[number];
}
function version(value: unknown): number {
  const parsed = integer(value);
  if (parsed !== OPERATOR_SCHEMA_VERSION_V1)
    throw new UnsupportedOperatorSchemaVersion(parsed);
  return parsed;
}
function entity(value: unknown): [JsonRecord, number] {
  const parsed = record(value);
  return [parsed, version(parsed.schema_version)];
}
function project(value: unknown): ProjectIdentity {
  const v = record(value);
  return {
    project_id: text(v.project_id, true),
    canonical_root: text(v.canonical_root, true),
  };
}
function worker(value: unknown): WorkerIdentity {
  const v = record(value);
  return {
    worker_id: text(v.worker_id, true),
    engine_instance_id: text(v.engine_instance_id, true),
    epoch: integer(v.epoch),
  };
}
function native(value: unknown): NativeSourceIdentity {
  const v = record(value);
  return {
    source_kind: enumValue(v.source_kind, sourceKinds),
    source_id: text(v.source_id, true),
    source_version: text(v.source_version, true),
  };
}
function resource(value: unknown): ResourceRef {
  const v = record(value);
  return {
    resource_kind: text(v.resource_kind, true),
    resource_id: text(v.resource_id, true),
    version: text(v.version, true),
  };
}
function decisionReference(value: unknown): DecisionReference {
  const v = record(value);
  return {
    project: project(v.project),
    worker: worker(v.worker),
    native: native(v.native),
  };
}
function evidence(value: unknown): EvidenceRef {
  const v = record(value);
  return {
    evidence_id: text(v.evidence_id, true),
    source: native(v.source),
    locator: text(v.locator, true),
    observed_at: text(v.observed_at, true),
    digest: nullableText(v.digest),
  };
}
function effect(value: unknown): ResourceEffect {
  const v = record(value);
  return {
    resource: resource(v.resource),
    state: enumValue(v.state, states.effect),
    evidence_refs: list(v.evidence_refs, (x) => text(x, true)),
  };
}
function capabilities(value: unknown): ActionCapabilities {
  const v = record(value);
  return {
    can_preview: boolean(v.can_preview),
    can_execute: boolean(v.can_execute),
    can_verify: boolean(v.can_verify),
    can_recover: boolean(v.can_recover),
    can_cancel: boolean(v.can_cancel),
  };
}

export function parseWorkItem(value: unknown): WorkItem {
  const [v, schema_version] = entity(value);
  return exact(v, {
    schema_version,
    id: text(v.id, true),
    project: project(v.project),
    subject: resource(v.subject),
    kind: text(v.kind, true),
    title: text(v.title, true),
    state: enumValue(v.state, states.work),
    native_sources: list(v.native_sources, native),
    current_artifact_id: nullableText(v.current_artifact_id),
    current_artifact_revision: nullableU32(v.current_artifact_revision),
    last_material_change_at: text(v.last_material_change_at, true),
    severity: nullableEnumValue(v.severity, states.workSeverity),
    problem_summary: nullableText(v.problem_summary),
    scope: nullableText(v.scope),
  });
}
export function parseRunOutcome(value: unknown): RunOutcome {
  const [v, schema_version] = entity(value);
  return exact(v, {
    schema_version,
    run_id: text(v.run_id, true),
    work_id: text(v.work_id, true),
    parent_run_id: nullableText(v.parent_run_id),
    project: project(v.project),
    worker: worker(v.worker),
    origin: text(v.origin, true),
    workflow: nullableText(v.workflow),
    input_fingerprint: text(v.input_fingerprint, true),
    coverage: text(v.coverage, true),
    started_at: text(v.started_at, true),
    ended_at: nullableText(v.ended_at),
    status: enumValue(v.status, states.run),
    evidence: list(v.evidence, evidence),
    effects: list(v.effects, effect),
    failure_class: nullableText(v.failure_class),
  });
}
export function parseArtifactRevision(value: unknown): ArtifactRevision {
  const [v, schema_version] = entity(value);
  const p = record(v.provenance);
  return exact(v, {
    schema_version,
    artifact_id: text(v.artifact_id, true),
    revision: u32(v.revision),
    project: project(v.project),
    source_run_ids: list(v.source_run_ids, (x) => text(x, true)),
    title: text(v.title, true),
    conclusion: text(v.conclusion, true),
    delta: text(v.delta),
    body: text(v.body, true),
    uncertainty: nullableText(v.uncertainty),
    proposed_action_ids: list(v.proposed_action_ids, (x) => text(x, true)),
    evidence: list(v.evidence, evidence),
    provenance: {
      source_run_ids: list(p.source_run_ids, (x) => text(x, true)),
      evidence_refs: list(p.evidence_refs, (x) => text(x, true)),
      contains_untrusted_source: boolean(p.contains_untrusted_source),
    },
  });
}
export function parseActionSpec(value: unknown): ActionSpec {
  const [v, schema_version] = entity(value);
  return exact(v, {
    schema_version,
    action_id: text(v.action_id, true),
    kind: text(v.kind, true),
    adapter: text(v.adapter, true),
    project: project(v.project),
    resource: resource(v.resource),
    expected_version: text(v.expected_version, true),
    parameter_ref: nullableText(v.parameter_ref),
    availability: enumValue(v.availability, states.availability),
    confirmation: enumValue(v.confirmation, states.confirmation),
    authority_class: enumValue(v.authority_class, authorityClasses),
    capabilities: capabilities(v.capabilities),
  });
}
export function parseActionExecution(value: unknown): ActionExecution {
  const [v, schema_version] = entity(value);
  const recovery =
    v.recovery === undefined || v.recovery === null ? null : record(v.recovery);
  return exact(v, {
    schema_version,
    execution_id: text(v.execution_id, true),
    project: project(v.project),
    worker: worker(v.worker),
    idempotency_key: text(v.idempotency_key, true),
    action_id: text(v.action_id, true),
    spec_hash: text(v.spec_hash, true),
    requester: native(v.requester),
    decision:
      v.decision === undefined || v.decision === null
        ? null
        : decisionReference(v.decision),
    status: enumValue(v.status, states.execution),
    started_at: nullableText(v.started_at),
    ended_at: nullableText(v.ended_at),
    effects: list(v.effects, effect),
    verification_evidence: list(v.verification_evidence, (x) => text(x, true)),
    recovery:
      recovery === null
        ? null
        : {
            recovery_id: text(recovery.recovery_id, true),
            action_id: text(recovery.action_id, true),
            expected_post_version: text(recovery.expected_post_version, true),
            available: boolean(recovery.available),
          },
  });
}
export function parseDecisionProjection(value: unknown): DecisionProjection {
  const [v, schema_version] = entity(value);
  return exact(v, {
    schema_version,
    decision: decisionReference(v.decision),
    title: text(v.title, true),
    impact: text(v.impact, true),
    options: list(v.options, (x) => {
      const o = record(x);
      return { option_id: text(o.option_id, true), label: text(o.label, true) };
    }),
    recommended_option_id: nullableText(v.recommended_option_id),
    status: enumValue(v.status, states.decision),
    response_authority: enumValue(v.response_authority, authorityClasses),
    expires_at: nullableText(v.expires_at),
    revalidation_required: boolean(v.revalidation_required),
  });
}
export function parseMandate(value: unknown): Mandate {
  const [v, schema_version] = entity(value);
  const budget =
    v.budget_limit === undefined || v.budget_limit === null
      ? null
      : record(v.budget_limit);
  return exact(v, {
    schema_version,
    mandate_id: text(v.mandate_id, true),
    project: project(v.project),
    owner: native(v.owner),
    owner_epoch: integer(v.owner_epoch),
    resource_scopes: list(v.resource_scopes, resource),
    allowed_operations: list(v.allowed_operations, (x) => text(x, true)),
    allowed_authority_classes: list(v.allowed_authority_classes, (x) =>
      enumValue(x, authorityClasses),
    ),
    model_constraints: list(v.model_constraints, (x) => text(x, true)),
    budget_limit:
      budget === null
        ? null
        : {
            max_tokens: nullableInteger(budget.max_tokens),
            max_turns: nullableU32(budget.max_turns),
            max_cost_micros: nullableInteger(budget.max_cost_micros),
            max_duration_seconds: nullableInteger(budget.max_duration_seconds),
          },
    expires_at: nullableText(v.expires_at),
    revoked_at: nullableText(v.revoked_at),
  });
}
export function parseDelivery(value: unknown): Delivery {
  const [v, schema_version] = entity(value);
  return exact(v, {
    schema_version,
    delivery_id: text(v.delivery_id, true),
    project: project(v.project),
    work_id: text(v.work_id, true),
    artifact_id: text(v.artifact_id, true),
    artifact_revision: u32(v.artifact_revision),
    surface: text(v.surface, true),
    client_id: text(v.client_id, true),
    state: enumValue(v.state, states.delivery),
    delivered_at: nullableText(v.delivered_at),
    seen_at: nullableText(v.seen_at),
    attention_category: text(v.attention_category, true),
  });
}
export function parseReadState(value: unknown): ReadState {
  const [v, schema_version] = entity(value);
  return exact(v, {
    schema_version,
    project: project(v.project),
    work_id: text(v.work_id, true),
    artifact_id: text(v.artifact_id, true),
    last_read_revision: u32(v.last_read_revision),
    read_at: text(v.read_at, true),
    snoozed_until: nullableText(v.snoozed_until),
    muted: boolean(v.muted),
  });
}
export function parseImprovementCandidate(
  value: unknown,
): ImprovementCandidate {
  const [v, schema_version] = entity(value);
  return exact(v, {
    schema_version,
    candidate_id: text(v.candidate_id, true),
    project: project(v.project),
    target: text(v.target, true),
    worktree_ref: nullableText(v.worktree_ref),
    config_draft_ref: nullableText(v.config_draft_ref),
    patch_digest: nullableText(v.patch_digest),
    build_identity: nullableText(v.build_identity),
    evaluation_spec_version: text(v.evaluation_spec_version, true),
    proof_refs: list(v.proof_refs, (x) => text(x, true)),
    activation_decision:
      v.activation_decision === undefined || v.activation_decision === null
        ? null
        : decisionReference(v.activation_decision),
    previous_known_good_identity: nullableText(v.previous_known_good_identity),
  });
}

export type OperatorContractFixture = {
  schema_version: number;
  work_item: WorkItem;
  run_outcome: RunOutcome;
  artifact_revision: ArtifactRevision;
  action_spec: ActionSpec;
  action_execution: ActionExecution;
  decision_projection: DecisionProjection;
  mandate: Mandate;
  delivery: Delivery;
  read_state: ReadState;
  improvement_candidate: ImprovementCandidate;
};

export function parseOperatorContractFixture(
  value: unknown,
): OperatorContractFixture {
  const v = record(value);
  return exact(v, {
    schema_version: version(v.schema_version),
    work_item: parseWorkItem(v.work_item),
    run_outcome: parseRunOutcome(v.run_outcome),
    artifact_revision: parseArtifactRevision(v.artifact_revision),
    action_spec: parseActionSpec(v.action_spec),
    action_execution: parseActionExecution(v.action_execution),
    decision_projection: parseDecisionProjection(v.decision_projection),
    mandate: parseMandate(v.mandate),
    delivery: parseDelivery(v.delivery),
    read_state: parseReadState(v.read_state),
    improvement_candidate: parseImprovementCandidate(v.improvement_candidate),
  });
}

export function validateOperatorContractV1(value: unknown): void {
  parseOperatorContractFixture(value);
}
