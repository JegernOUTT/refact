import { z } from "zod";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

/*
 * Wire types mirror `refact-review-core/src/types.rs` exactly (serde
 * snake_case enums, internally tagged `EvidenceSource.kind` and
 * `Operation.op`). Keep both sides in the same change.
 */

export const REVIEW_SCHEMA_VERSION = 1;

export type ReviewOrigin = "local" | "imported" | "agent";
export type ReviewSide = "base" | "head";
export type AnchorStatus = "current" | "stalled";
export type AnnotationKind = "explanation" | "highlight" | "finding";
export type Severity = "info" | "low" | "medium" | "high" | "critical";
export type Confidence = "unknown" | "low" | "medium" | "high";
export type Assessment = "unassessed" | "supported" | "disputed" | "refuted";
export type Disposition =
  | "open"
  | "repair_proposed"
  | "fixed"
  | "risk_accepted"
  | "dismissed";
export type Applicability =
  | "current"
  | "needs_revalidation"
  | "historical"
  | "no_longer_applicable"
  | "unknown";
export type ActorKind = "user" | "agent" | "system" | "external";
export type MaterialKind = "imported" | "investigation" | "repair";
export type RunStatus =
  | "queued"
  | "running"
  | "cancel_requested"
  | "completed"
  | "partial"
  | "failed"
  | "cancelled"
  | "interrupted";

export type CapturePolicy = {
  staged: boolean;
  working: boolean;
  untracked: boolean;
};

export type BlobRef = { digest: string; size: number };

export type CandidateFile = {
  path: string;
  base: BlobRef | null;
  head: BlobRef | null;
  base_mode: number | null;
  head_mode: number | null;
};

export type Candidate = {
  id: string;
  base_oid: string;
  head_oid: string;
  content_digest: string;
  created_at_ms: number;
  repository: string;
  origin: ReviewOrigin;
  policy: CapturePolicy;
  /** Engine-described capture scope; empty for legacy candidates. */
  snapshot_scope?: string;
  files: CandidateFile[];
};

export type AnchorPosition = {
  candidate_id: string;
  path: string;
  side: ReviewSide;
  start_line: number;
  end_line: number;
  content_hash: string;
  context_before: string[];
  context_after: string[];
};

export type Anchor = {
  position: AnchorPosition;
  status: AnchorStatus;
  stalled_reason: string | null;
  history: AnchorPosition[];
};

export type RichBlock = {
  id: string;
  revision: number;
  content:
    | { kind: "markdown"; body: string }
    | {
        kind: "html";
        html: string;
        anchors?: Anchor[];
        static_fallback?: string;
      }
    | { kind: "code"; anchor: Anchor }
    | { kind: "diff"; candidate_id: string; path: string; anchors?: Anchor[] }
    | {
        kind: "sources";
        evidence_ids: string[];
        relation: "consulted" | "supports";
      }
    | { kind: "evidence"; evidence_ids: string[] };
};

export type DocumentTarget = {
  candidate_id: string;
  step_id: string;
  version: number;
  block_id?: string | null;
  block_revision?: number | null;
};

export type TourVersion = {
  html?: string;
  blocks?: RichBlock[];
  candidate_id?: string | null;
  version: number;
  title: string;
  body: string;
  anchors: Anchor[];
};

export type TourStep = {
  id: string;
  current: TourVersion;
  history: TourVersion[];
};

export type Annotation = {
  id: string;
  kind: AnnotationKind;
  title: string;
  body: string;
  anchors: Anchor[];
  severity: Severity;
  confidence: Confidence;
  assessment: Assessment;
  disposition: Disposition;
  applicability: Applicability;
  evidence_ids: string[];
  run_id: string | null;
};

export type Actor = { kind: ActorKind; id: string };

export type ReviewFileRecord = {
  path: string;
  zone: string;
  attribution: "declared" | "observed" | "heuristic";
};

export type ContributionContext = {
  candidate_id: string | null;
  run_id: string | null;
  applicability: Applicability;
  privacy: ReviewFileRecord[];
};

export type Contribution = {
  blocks?: RichBlock[];
  id: string;
  thread_id: string;
  actor: Actor;
  body: string;
  created_at_ms: number;
  chat_message_id: string | null;
  context?: ContributionContext | null;
};

export type ReviewThread = {
  target?: DocumentTarget | null;
  id: string;
  anchors: Anchor[];
  annotation_id: string | null;
  resolved: boolean;
  contributions: Contribution[];
};

export type Material = {
  id: string;
  kind: MaterialKind;
  candidate_id: string;
  agent_id: string | null;
  description: string;
  content_digest: string;
  input_digests: Record<string, string>;
  environment: string | null;
  worktree_id: string | null;
  worktree_root: string | null;
  artifacts: BlobRef[];
};

export type EvidenceSource =
  | { kind: "source"; path: string; anchor: Anchor | null }
  | { kind: "external"; url: string }
  | {
      kind: "executed";
      command: string[];
      workdir: string;
      exit_code: number | null;
      stdout: BlobRef | null;
      stderr: BlobRef | null;
    };

export type Evidence = {
  id: string;
  candidate_id: string;
  material_id: string | null;
  run_id: string | null;
  source: EvidenceSource;
  summary: string;
  applicability: Applicability;
  created_at_ms: number;
};

export type Run = {
  id: string;
  candidate_id: string;
  agent_id: string;
  purpose: string;
  model: string | null;
  domain: string | null;
  status: RunStatus;
  started_at_ms: number;
  finished_at_ms: number | null;
  error: string | null;
};

export type Activity = {
  id: string;
  at_ms: number;
  actor: Actor;
  action: string;
  candidate_id: string | null;
};

export type Review = {
  schema_version: number;
  id: string;
  owner_chat_id: string;
  revision: number;
  current_candidate_id: string | null;
  title: string;
  origin: ReviewOrigin;
  target_reference: string;
  candidates: Candidate[];
  tour: TourStep[];
  annotations: Annotation[];
  threads: ReviewThread[];
  evidence: Evidence[];
  runs: Run[];
  materials: Material[];
  activity: Activity[];
  remote_links?: RemoteLink[];
};

/** Ties one local comment or finding to its counterpart on the pull request. */
export type RemoteLink = {
  kind: "contribution" | "annotation";
  local_id: string;
  state: "imported" | "requested" | "published";
  source: string;
  remote_id?: string;
  remote_thread_id?: string;
  url?: string;
  author?: string;
  at_ms: number;
};

export type ReviewSyncReport = {
  available: boolean;
  reason?: string;
  pull_request?: string;
  imported: number;
  published: number;
  failed?: { id: string; reason: string }[];
};

const revisionSchema = z.number().int().nonnegative().safe();
const stackSourceSchema = z.object({
  kind: z.string(),
  reference: z.string().nullable(),
});
const stackScopeChoiceSchema = z.object({
  scope: z.enum(["single", "whole"]),
  selected_member_id: z.string(),
  confirmed: z.boolean(),
  explicit_request: z.boolean(),
});
const stackMemberSchema = z.object({
  id: z.string(),
  branch_ref: z.string(),
  pr_identity: z
    .object({
      host: z.string(),
      repository: z.string(),
      number: revisionSchema,
    })
    .nullable(),
  parent_member_id: z.string().nullable(),
  declared_base_ref: z.string(),
  resolved_base_oid: z.string().nullable(),
  head_oid: z.string().nullable(),
  review_id: z.string().nullable(),
  state: z.enum(["ready", "missing", "unauthorized", "ambiguous", "removed"]),
  reason: z.string().nullable(),
});
const stackManifestSchema = z.object({
  schema_version: revisionSchema,
  id: z.string(),
  owner_chat_id: z.string(),
  revision: revisionSchema,
  title: z.string(),
  repository: z.string(),
  source: stackSourceSchema,
  root_base_ref: z.string(),
  root_base_oid: z.string().nullable(),
  scope_choice: stackScopeChoiceSchema,
  members: z.array(stackMemberSchema),
});
export type StackSource = z.infer<typeof stackSourceSchema>;
export type StackScopeChoice = z.infer<typeof stackScopeChoiceSchema>;
export type StackScope = StackScopeChoice["scope"];
export type StackMember = z.infer<typeof stackMemberSchema>;
export type StackMemberState = StackMember["state"];
export type PrIdentity = NonNullable<StackMember["pr_identity"]>;
export type StackManifest = z.infer<typeof stackManifestSchema>;
const preparedStackSchema = z.object({
  status: z.literal("pending_scope_choice"),
  manifest: stackManifestSchema,
  question: z.object({
    id: z.string(),
    type: z.literal("single_select"),
    text: z.string(),
    options: z.array(z.string()),
  }),
  fingerprint: z.string(),
  acquisition: z.string(),
});
export type PreparedStack = z.infer<typeof preparedStackSchema>;
export type ReviewStackImportRequest = {
  chatId: string;
  manifest: StackManifest;
  scope_receipt_tool_call_id?: string;
  request_key: string;
};
export type ReviewStackMemberRequest = {
  chatId: string;
  memberId: string;
  stackRevision?: number;
};
export type ReviewStackCumulative = {
  comparison_id: string;
  stack_id: string;
  stack_revision: number;
  member_id: string;
  read_only: true;
  annotation_target: null;
  candidate: Candidate;
};
const reviewHandoffSchema = z.object({
  review_id: z.string(),
  revision: revisionSchema,
  stack_revision: revisionSchema.nullable(),
  markdown: z.string(),
  filename: z.string(),
  scope: z.string(),
});
export type ReviewHandoff = z.infer<typeof reviewHandoffSchema>;
export const isStackManifest = (value: unknown): value is StackManifest =>
  stackManifestSchema.safeParse(value).success;
export const isPreparedStack = (value: unknown): value is PreparedStack =>
  preparedStackSchema.safeParse(value).success;
export const isReviewHandoff = (value: unknown): value is ReviewHandoff =>
  reviewHandoffSchema.safeParse(value).success;

export type ReviewOperation =
  | { op: "rename"; title: string }
  | { op: "set_target_reference"; target_reference: string }
  | { op: "create_thread"; thread: ReviewThread; contribution: Contribution }
  | { op: "stage_candidate"; candidate: Candidate }
  | { op: "activate_candidate"; candidate_id: string }
  | { op: "put_tour_step"; step: TourStep }
  | { op: "put_annotation"; annotation: Annotation }
  | { op: "put_thread"; thread: ReviewThread }
  | { op: "add_contribution"; contribution: Contribution }
  | { op: "add_evidence"; evidence: Evidence }
  | { op: "put_run"; run: Run }
  | { op: "add_material"; material: Material };

/** Actor and time are normalized by the engine runtime; GUI values are placeholders. */
export type ReviewMutation = {
  expected_revision: number;
  request_key: string;
  actor: Actor;
  at_ms: number;
  operation: ReviewOperation;
};

export type ReviewDelivery = {
  id: string;
  review_id: string;
  contribution_id: string;
  owner_chat_id: string;
  delivered: boolean;
};

export type HistoricalObject =
  | { kind: "annotation"; value: Annotation }
  | { kind: "thread"; value: ReviewThread }
  | { kind: "tour_step"; value: TourStep };

export type ObjectRevision = {
  review_id: string;
  revision: number;
  actor: Actor;
  at_ms: number;
  object: HistoricalObject;
};

/** Raw `FileView`: `Vec<u8>` serializes as a JSON number array. */
export type ReviewFileViewWire = {
  path: string;
  base: number[] | null;
  head: number[] | null;
  diff: string;
};

export type ReviewFileSide =
  | { kind: "absent" }
  | { kind: "text"; text: string }
  | { kind: "binary"; size: number };

export type ReviewFileView = {
  path: string;
  base: ReviewFileSide;
  head: ReviewFileSide;
  diff: string;
};

export type ReviewAnchorRequest = {
  reviewId: string;
  candidate_id: string;
  path: string;
  side: ReviewSide;
  start_line: number;
  end_line: number;
};

export type ReviewCommentRequest = {
  target?: DocumentTarget;
  blocks?: RichBlock[];
  reviewId: string;
  expected_revision: number;
  request_key: string;
  thread_id?: string;
  annotation_id?: string;
  anchor?: Anchor;
  body: string;
  /** Also post the comment to the pull request. */
  publish?: boolean;
};

export type ReviewCancelRunRequest = {
  reviewId: string;
  runId: string;
  expected_revision: number;
  request_key: string;
};

/** Ephemeral selection context for the owning chat; not a review mutation. */
export type ReviewSelectionRequest = {
  reviewId: string;
  candidate_id: string;
  action: string;
  path: string | null;
  side: ReviewSide | null;
  start_line: number | null;
  end_line: number | null;
  object_id: string | null;
  request_key: string;
};

const GUI_ACTOR: Actor = { kind: "user", id: "gui" };

export function buildMutation(
  review: Pick<Review, "revision">,
  requestKey: string,
  operation: ReviewOperation,
): ReviewMutation {
  return {
    expected_revision: review.revision,
    request_key: requestKey,
    actor: GUI_ACTOR,
    at_ms: Date.now(),
    operation,
  };
}

/** Strict UTF-8 decode; never lossy. Non-UTF-8 content is reported as binary. */
export function decodeFileSide(bytes: number[] | null): ReviewFileSide {
  if (bytes === null) return { kind: "absent" };
  try {
    const text = new TextDecoder("utf-8", { fatal: true }).decode(
      Uint8Array.from(bytes),
    );
    if (text.includes("\u0000")) return { kind: "binary", size: bytes.length };
    return { kind: "text", text };
  } catch {
    return { kind: "binary", size: bytes.length };
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function isReview(value: unknown): value is Review {
  if (!isRecord(value)) return false;
  return (
    typeof value.id === "string" &&
    typeof value.owner_chat_id === "string" &&
    typeof value.revision === "number" &&
    typeof value.schema_version === "number" &&
    Array.isArray(value.candidates) &&
    Array.isArray(value.tour) &&
    Array.isArray(value.annotations) &&
    Array.isArray(value.threads) &&
    Array.isArray(value.evidence) &&
    Array.isArray(value.runs) &&
    Array.isArray(value.materials) &&
    Array.isArray(value.activity)
  );
}

function isSyncReport(value: unknown): value is ReviewSyncReport {
  return (
    isRecord(value) &&
    typeof value.available === "boolean" &&
    typeof value.imported === "number" &&
    typeof value.published === "number"
  );
}

function isByteArray(value: unknown): value is number[] | null {
  return (
    value === null ||
    (Array.isArray(value) &&
      value.every(
        (b) =>
          typeof b === "number" && Number.isInteger(b) && b >= 0 && b <= 255,
      ))
  );
}

export function isFileViewWire(value: unknown): value is ReviewFileViewWire {
  return (
    isRecord(value) &&
    typeof value.path === "string" &&
    typeof value.diff === "string" &&
    isByteArray(value.base) &&
    isByteArray(value.head)
  );
}

export function isAnchor(value: unknown): value is Anchor {
  return (
    isRecord(value) &&
    isRecord(value.position) &&
    (value.status === "current" || value.status === "stalled") &&
    Array.isArray(value.history)
  );
}

/** Extract a human-readable message from an engine `{detail}` error. */
export function reviewErrorMessage(error: unknown): string {
  if (!isRecord(error)) return "Request failed";
  const data = error.data;
  if (isRecord(data) && typeof data.detail === "string") return data.detail;
  if (typeof error.error === "string") return error.error;
  if (typeof error.message === "string") return error.message;
  return "Request failed";
}

export function isReviewConflict(error: unknown): boolean {
  return isRecord(error) && error.status === 409;
}

function shapeError(what: string, _raw: unknown): FetchBaseQueryError {
  // Never echo untrusted response bodies (which may contain private material).
  return {
    status: "CUSTOM_ERROR",
    error: `Invalid ${what} response`,
    data: {
      code: "invalid_review_response",
      detail: `Invalid ${what} response`,
    },
  };
}

const blobRefSchema = z.object({ digest: z.string(), size: revisionSchema });
const candidateSchema = z.object({
  id: z.string(),
  base_oid: z.string(),
  head_oid: z.string(),
  content_digest: z.string(),
  created_at_ms: z.number().finite(),
  repository: z.string(),
  origin: z.enum(["local", "imported", "agent"]),
  policy: z.object({
    staged: z.boolean(),
    working: z.boolean(),
    untracked: z.boolean(),
  }),
  snapshot_scope: z.string().optional(),
  files: z.array(
    z.object({
      path: z.string(),
      base: blobRefSchema.nullable(),
      head: blobRefSchema.nullable(),
      base_mode: revisionSchema.nullable(),
      head_mode: revisionSchema.nullable(),
    }),
  ),
});
const cumulativeSchema = z.object({
  comparison_id: z.string(),
  stack_id: z.string(),
  stack_revision: revisionSchema,
  member_id: z.string(),
  read_only: z.literal(true),
  annotation_target: z.null(),
  candidate: candidateSchema,
});
export const isReviewStackCumulative = (
  value: unknown,
): value is ReviewStackCumulative => cumulativeSchema.safeParse(value).success;
export const reviewStackChatTag = (chatId: string): ReviewTag => ({
  type: "Review",
  id: `stack:${chatId}`,
});
const stackPath = (chatId: string, suffix = "") =>
  `/v1/review-stacks/by-chat/${encodeURIComponent(chatId)}${suffix}`;

export type ReviewTag = { type: "Review"; id: string };
export const reviewChatTag = (chatId: string): ReviewTag => ({
  type: "Review",
  id: `chat:${chatId}`,
});
export const reviewIdTag = (reviewId: string): ReviewTag => ({
  type: "Review",
  id: reviewId,
});

const reviewPath = (reviewId: string, suffix = "") =>
  `/v1/reviews/${encodeURIComponent(reviewId)}${suffix}`;

export const reviewApi = createApi({
  reducerPath: "reviewApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  tagTypes: ["Review"],
  endpoints: (builder) => ({
    getReviewStackByChat: builder.query<
      StackManifest | null,
      { chatId: string }
    >({
      queryFn: async ({ chatId }, api, _opts, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            stackPath(chatId),
          ),
        });
        if (result.error) return { error: result.error };
        if (result.data === null) return { data: null };
        if (!isStackManifest(result.data))
          return { error: shapeError("StackManifest", result.data) };
        return { data: result.data };
      },
      providesTags: (_result, _error, { chatId }) => [
        reviewStackChatTag(chatId),
      ],
    }),
    prepareReviewStack: builder.mutation<
      PreparedStack,
      { chatId: string; manifest: StackManifest }
    >({
      queryFn: async ({ chatId, manifest }, api, _opts, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            stackPath(chatId, "/prepare"),
          ),
          method: "POST",
          body: manifest,
        });
        if (result.error) return { error: result.error };
        if (!isPreparedStack(result.data))
          return { error: shapeError("PreparedStack", result.data) };
        return { data: result.data };
      },
    }),
    importReviewStack: builder.mutation<
      StackManifest,
      ReviewStackImportRequest
    >({
      queryFn: async ({ chatId, ...body }, api, _opts, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            stackPath(chatId, "/import"),
          ),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        if (!isStackManifest(result.data))
          return { error: shapeError("StackManifest", result.data) };
        return { data: result.data };
      },
      invalidatesTags: (_result, _error, { chatId }) => [
        reviewStackChatTag(chatId),
      ],
    }),
    getReviewStackHistory: builder.query<StackManifest[], { chatId: string }>({
      queryFn: async ({ chatId }, api, _opts, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            stackPath(chatId, "/history"),
          ),
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (!Array.isArray(raw) || !raw.every(isStackManifest))
          return { error: shapeError("StackManifest[]", raw) };
        return { data: raw };
      },
      providesTags: (_result, _error, { chatId }) => [
        reviewStackChatTag(chatId),
      ],
    }),
    /** Browse only: this GET never activates a member or changes execution context. */
    getReviewStackMember: builder.query<
      Review | null,
      ReviewStackMemberRequest
    >({
      queryFn: async ({ chatId, memberId }, api, _opts, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            stackPath(chatId, `/members/${encodeURIComponent(memberId)}`),
          ),
        });
        if (result.error) return { error: result.error };
        if (result.data === null) return { data: null };
        if (!isReview(result.data))
          return { error: shapeError("Review", result.data) };
        return { data: result.data };
      },
      providesTags: (result, _error, { chatId }) => [
        reviewStackChatTag(chatId),
        ...(result ? [reviewIdTag(result.id)] : []),
      ],
    }),
    getReviewStackCumulative: builder.query<
      ReviewStackCumulative,
      ReviewStackMemberRequest
    >({
      queryFn: async (
        { chatId, memberId, stackRevision },
        api,
        _opts,
        baseQuery,
      ) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            stackPath(chatId, `/cumulative/${encodeURIComponent(memberId)}`),
            stackRevision === undefined
              ? {}
              : { expected_revision: String(stackRevision) },
          ),
        });
        if (result.error) return { error: result.error };
        if (!isReviewStackCumulative(result.data))
          return { error: shapeError("StackCumulative", result.data) };
        return { data: result.data };
      },
      providesTags: (_result, _error, { chatId }) => [
        reviewStackChatTag(chatId),
      ],
    }),
    /** Explicit local ref refresh only; never polls Graphite or changes execution. */
    refreshReviewStack: builder.mutation<
      StackManifest,
      { chatId: string; expected_revision: number; request_key: string }
    >({
      queryFn: async ({ chatId, ...body }, api, _opts, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            stackPath(chatId, "/refresh"),
          ),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        if (!isStackManifest(result.data))
          return { error: shapeError("StackManifest", result.data) };
        return { data: result.data };
      },
      invalidatesTags: (_result, _error, { chatId }) => [
        reviewStackChatTag(chatId),
      ],
    }),
    getReviewStackCumulativeFile: builder.query<
      ReviewFileView,
      ReviewStackMemberRequest & { path: string }
    >({
      queryFn: async (
        { chatId, memberId, path, stackRevision },
        api,
        _opts,
        baseQuery,
      ) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            stackPath(
              chatId,
              `/cumulative/${encodeURIComponent(memberId)}/files`,
            ),
            {
              path,
              ...(stackRevision === undefined
                ? {}
                : { expected_revision: String(stackRevision) }),
            },
          ),
        });
        if (result.error) return { error: result.error };
        if (!isFileViewWire(result.data))
          return { error: shapeError("FileView", result.data) };
        return {
          data: {
            path: result.data.path,
            base: decodeFileSide(result.data.base),
            head: decodeFileSide(result.data.head),
            diff: result.data.diff,
          },
        };
      },
      providesTags: (_result, _error, { chatId }) => [
        reviewStackChatTag(chatId),
      ],
    }),
    getReviewHandoff: builder.query<ReviewHandoff, { reviewId: string }>({
      queryFn: async ({ reviewId }, api, _opts, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            reviewPath(reviewId, "/handoff"),
          ),
        });
        if (result.error) return { error: result.error };
        if (!isReviewHandoff(result.data))
          return { error: shapeError("ReviewHandoff", result.data) };
        return { data: result.data };
      },
      keepUnusedDataFor: 0,
    }),
    getReviewByChat: builder.query<Review | null, { chatId: string }>({
      queryFn: async ({ chatId }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/reviews/by-chat/${encodeURIComponent(chatId)}`,
          ),
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (raw === null || raw === undefined) return { data: null };
        if (!isReview(raw)) return { error: shapeError("Review", raw) };
        return { data: raw };
      },
      providesTags: (result, _error, { chatId }) =>
        result
          ? [reviewChatTag(chatId), reviewIdTag(result.id)]
          : [reviewChatTag(chatId)],
    }),

    getReview: builder.query<Review, { reviewId: string }>({
      queryFn: async ({ reviewId }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, reviewPath(reviewId)),
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (!isReview(raw)) return { error: shapeError("Review", raw) };
        return { data: raw };
      },
      providesTags: (_result, _error, { reviewId }) => [reviewIdTag(reviewId)],
    }),

    /** Immutable candidate content; never reads the live worktree. */
    getReviewFile: builder.query<
      ReviewFileView,
      { reviewId: string; candidateId: string; path: string }
    >({
      queryFn: async (
        { reviewId, candidateId, path },
        api,
        _opts,
        baseQuery,
      ) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, reviewPath(reviewId, "/files"), {
            candidate_id: candidateId,
            path,
          }),
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (!isFileViewWire(raw)) return { error: shapeError("FileView", raw) };
        return {
          data: {
            path: raw.path,
            base: decodeFileSide(raw.base),
            head: decodeFileSide(raw.head),
            diff: raw.diff,
          },
        };
      },
      // Candidate content is content-addressed and immutable.
      keepUnusedDataFor: 300,
    }),

    getReviewDeliveries: builder.query<ReviewDelivery[], { reviewId: string }>({
      queryFn: async ({ reviewId }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            reviewPath(reviewId, "/delivery-status"),
          ),
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (!Array.isArray(raw)) {
          return { error: shapeError("ReviewDelivery[]", raw) };
        }
        return { data: raw as ReviewDelivery[] };
      },
      providesTags: (_result, _error, { reviewId }) => [reviewIdTag(reviewId)],
    }),

    getReviewHistory: builder.query<ObjectRevision[], { reviewId: string }>({
      queryFn: async ({ reviewId }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, reviewPath(reviewId, "/history")),
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (!Array.isArray(raw)) {
          return { error: shapeError("ObjectRevision[]", raw) };
        }
        return { data: raw as ObjectRevision[] };
      },
      providesTags: (_result, _error, { reviewId }) => [reviewIdTag(reviewId)],
    }),

    /** Referenced evidence/material artifact bytes, decoded strictly. */
    getReviewBlob: builder.query<
      ReviewFileSide,
      { reviewId: string; digest: string }
    >({
      queryFn: async ({ reviewId, digest }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            reviewPath(reviewId, `/blobs/${encodeURIComponent(digest)}`),
          ),
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (!isByteArray(raw) || raw === null) {
          return { error: shapeError("blob", raw) };
        }
        return { data: decodeFileSide(raw) };
      },
      keepUnusedDataFor: 120,
    }),

    resolveAnchor: builder.mutation<Anchor, ReviewAnchorRequest>({
      queryFn: async ({ reviewId, ...body }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, reviewPath(reviewId, "/anchors")),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (!isAnchor(raw)) return { error: shapeError("Anchor", raw) };
        return { data: raw };
      },
    }),

    mutateReview: builder.mutation<
      Review,
      { reviewId: string; mutation: ReviewMutation }
    >({
      queryFn: async ({ reviewId, mutation }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, reviewPath(reviewId, "/mutations")),
          method: "POST",
          body: mutation,
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (!isReview(raw)) return { error: shapeError("Review", raw) };
        return { data: raw };
      },
      invalidatesTags: (_result, _error, { reviewId }) => [
        reviewIdTag(reviewId),
      ],
    }),

    addReviewComment: builder.mutation<Review, ReviewCommentRequest>({
      queryFn: async ({ reviewId, ...body }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, reviewPath(reviewId, "/comments")),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (!isReview(raw)) return { error: shapeError("Review", raw) };
        return { data: raw };
      },
      invalidatesTags: (_result, _error, { reviewId }) => [
        reviewIdTag(reviewId),
      ],
    }),

    /** Every file of one captured comparison; the polled review lists only changed and referenced ones. */
    getReviewCandidateFiles: builder.query<
      CandidateFile[],
      { reviewId: string; candidateId: string }
    >({
      queryFn: async ({ reviewId, candidateId }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            reviewPath(
              reviewId,
              `/candidates/${encodeURIComponent(candidateId)}/files`,
            ),
          ),
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (
          !Array.isArray(raw) ||
          !raw.every((file) => isRecord(file) && typeof file.path === "string")
        )
          return { error: shapeError("files", raw) };
        return { data: raw as CandidateFile[] };
      },
    }),

    sendReviewItemToPr: builder.mutation<
      Review,
      { reviewId: string; id: string }
    >({
      queryFn: async ({ reviewId, id }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, reviewPath(reviewId, "/outbox")),
          method: "POST",
          body: { id },
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (!isReview(raw)) return { error: shapeError("Review", raw) };
        return { data: raw };
      },
      invalidatesTags: (_result, _error, { reviewId }) => [
        reviewIdTag(reviewId),
      ],
    }),

    syncReviewComments: builder.mutation<
      ReviewSyncReport,
      { reviewId: string; quiet: boolean }
    >({
      queryFn: async ({ reviewId, quiet }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, reviewPath(reviewId, "/pr-sync")),
          method: "POST",
          body: { quiet },
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (!isSyncReport(raw)) return { error: shapeError("sync", raw) };
        return { data: raw };
      },
      invalidatesTags: (result, _error, { reviewId }) =>
        result && result.imported + result.published > 0
          ? [reviewIdTag(reviewId)]
          : [],
    }),

    reportReviewSelection: builder.mutation<null, ReviewSelectionRequest>({
      queryFn: async ({ reviewId, ...body }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, reviewPath(reviewId, "/activity")),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: null };
      },
    }),

    cancelReviewRun: builder.mutation<Review, ReviewCancelRunRequest>({
      queryFn: async ({ reviewId, runId, ...body }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            reviewPath(reviewId, `/runs/${encodeURIComponent(runId)}/cancel`),
          ),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        const raw: unknown = result.data;
        if (!isReview(raw)) return { error: shapeError("Review", raw) };
        return { data: raw };
      },
      invalidatesTags: (_result, _error, { reviewId }) => [
        reviewIdTag(reviewId),
      ],
    }),
  }),
});

/** Action for SSE/tool-result driven refresh of the review owned by a chat. */
export const invalidateReviewForChat = (chatId: string) =>
  reviewApi.util.invalidateTags([
    reviewChatTag(chatId),
    reviewStackChatTag(chatId),
  ]);

export const {
  useGetReviewStackByChatQuery,
  useGetReviewStackMemberQuery,
  useGetReviewStackHistoryQuery,
  useGetReviewStackCumulativeQuery,
  useGetReviewStackCumulativeFileQuery,
  usePrepareReviewStackMutation,
  useImportReviewStackMutation,
  useRefreshReviewStackMutation,
  useLazyGetReviewHandoffQuery,
  useGetReviewByChatQuery,
  useGetReviewQuery,
  useGetReviewFileQuery,
  useGetReviewDeliveriesQuery,
  useGetReviewHistoryQuery,
  useGetReviewBlobQuery,
  useResolveAnchorMutation,
  useMutateReviewMutation,
  useAddReviewCommentMutation,
  useGetReviewCandidateFilesQuery,
  useSendReviewItemToPrMutation,
  useSyncReviewCommentsMutation,
  useCancelReviewRunMutation,
  useReportReviewSelectionMutation,
} = reviewApi;
