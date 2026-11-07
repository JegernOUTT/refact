/**
 * Host-native decision bridge contract (B21 / plan §16.3).
 *
 * The native engine owns every pending decision. A host (VS Code, JetBrains,
 * browser shell) may only *announce* that a native decision is waiting and
 * offer to open the owning chat. Hosts never grant, deny, or otherwise resolve a
 * decision, and this module deliberately exposes no vocabulary that would let
 * them: see `HOST_DECISION_INTENTS`.
 *
 * The functions below are pure and deterministic so that every host can run the
 * same duplicate / stale / reconnect rules against the same golden corpus.
 */

export const DECISION_BRIDGE_CONTRACT_VERSION = 1;

/**
 * Everything a host is allowed to do with a native decision. Adding an
 * approval-like intent here would be a consent bypass, and is asserted against.
 */
export const HOST_DECISION_INTENTS = [
  "notify",
  "open_chat",
  "update_outstanding_badge",
] as const;
export type HostDecisionIntent = (typeof HOST_DECISION_INTENTS)[number];

/** Where the host learned about the decision. Telemetry only; never authority. */
export type IdeDecisionOrigin =
  | "native_notification_stream"
  | "native_chat_event"
  | "native_snapshot";

export const IDE_DECISION_ORIGINS: readonly IdeDecisionOrigin[] = [
  "native_notification_stream",
  "native_chat_event",
  "native_snapshot",
];

/** How a decision stopped being outstanding. All values are native outcomes. */
export type IdeDecisionResolution =
  | "answered"
  | "cancelled"
  | "resolved_elsewhere";

export type IdeDecisionQuestion = {
  id: string;
  type: string;
  text: string;
  options?: string[];
};

export type IdeAskQuestionsPayload = {
  chatId: string;
  toolCallId: string;
  questions: IdeDecisionQuestion[];
  /** Native incarnation timestamp; optional only for older engines. */
  askedAtMs?: number;
  scope?: DecisionScope;
  origin?: IdeDecisionOrigin;
};

export type IdeDecisionResolvedPayload = {
  chatId: string;
  toolCallId: string;
  resolution: IdeDecisionResolution;
  /** Native incarnation timestamp; a stale incarnation cannot retire a newer ask. */
  askedAtMs?: number;
  scope?: DecisionScope;
  origin?: IdeDecisionOrigin;
};

/** Identity scope: a decision only exists inside one project + engine endpoint. */
export type DecisionScope = {
  projectRoot: string;
  endpointIdentity: string;
};

export type OutstandingDecision = {
  chatId: string;
  toolCallId: string;
  digest: string;
  askedAtMs?: number;
  origin: IdeDecisionOrigin;
  /** Monotonic announcement order, used only for bounded eviction. */
  seq: number;
};

export type DecisionAnnouncementState = {
  version: number;
  scope: DecisionScope | null;
  outstanding: OutstandingDecision[];
  /** Bounded tombstones so a reconnect replay cannot re-announce. */
  resolved: { key: string; digest: string; askedAtMs?: number }[];
  seq: number;
};

export type AnnounceReason =
  | "new"
  | "duplicate"
  | "already_resolved"
  | "superseded_content"
  | "foreign_scope"
  | "invalid_payload";

export type DecisionAnnouncementDecision = {
  announce: boolean;
  reason: AnnounceReason;
  key: string | null;
  next: DecisionAnnouncementState;
};

export type DecisionResolutionOutcome = {
  changed: boolean;
  reason: "resolved" | "unknown_decision" | "invalid_payload";
  next: DecisionAnnouncementState;
};

export type NativeDecisionSnapshotInput = {
  scope: DecisionScope;
  /**
   * True only for a complete native listing. A failed, partial, or truncated
   * read must set this false: absence of data is not evidence of resolution.
   */
  authoritative: boolean;
  truncated?: boolean;
  pending: { chatId: string; toolCallId: string }[];
  /** Explicit native terminal records. Absence from pending never resolves. */
  resolved?: IdeDecisionResolvedPayload[];
};

export type SnapshotReconciliation = {
  resolved: IdeDecisionResolvedPayload[];
  reason: "reconciled" | "not_authoritative" | "foreign_scope";
  next: DecisionAnnouncementState;
};

export const MAX_OUTSTANDING_DECISIONS = 200;
export const MAX_RESOLVED_TOMBSTONES = 200;

const UNIT_SEPARATOR = "\u001f";

export function emptyDecisionAnnouncementState(): DecisionAnnouncementState {
  return {
    version: DECISION_BRIDGE_CONTRACT_VERSION,
    scope: null,
    outstanding: [],
    resolved: [],
    seq: 0,
  };
}

export function decisionCorrelationKey(
  chatId: string,
  toolCallId: string,
): string {
  return `${chatId}${UNIT_SEPARATOR}${toolCallId}`;
}

/**
 * FNV-1a 32-bit over a canonical rendering of the question set. Deliberately
 * simple so the VS Code and JetBrains trackers can compute byte-identical
 * digests for the same native question set.
 */
export function decisionQuestionsDigest(
  questions: IdeDecisionQuestion[],
): string {
  const canonical = questions
    .map((question) =>
      [
        question.id,
        question.type,
        question.text,
        (question.options ?? []).join("\u0001"),
      ].join(UNIT_SEPARATOR),
    )
    .join("\u001e");
  let hash = 0x811c9dc5;
  for (let index = 0; index < canonical.length; index += 1) {
    hash ^= canonical.charCodeAt(index) & 0xff;
    hash = Math.imul(hash, 0x01000193) >>> 0;
    const high = canonical.charCodeAt(index) >>> 8;
    if (high !== 0) {
      hash ^= high;
      hash = Math.imul(hash, 0x01000193) >>> 0;
    }
  }
  return hash.toString(16).padStart(8, "0");
}

function nonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function validMetadata(payload: Record<string, unknown>): boolean {
  if (
    payload.askedAtMs !== undefined &&
    (typeof payload.askedAtMs !== "number" ||
      !Number.isSafeInteger(payload.askedAtMs) ||
      payload.askedAtMs < 0)
  )
    return false;
  if (
    payload.origin !== undefined &&
    !IDE_DECISION_ORIGINS.includes(payload.origin as IdeDecisionOrigin)
  )
    return false;
  if (payload.scope !== undefined) {
    if (!payload.scope || typeof payload.scope !== "object") return false;
    const scope = payload.scope as Record<string, unknown>;
    if (
      typeof scope.projectRoot !== "string" ||
      !nonEmptyString(scope.endpointIdentity)
    )
      return false;
  }
  return true;
}

export function isValidAskQuestionsPayload(
  value: unknown,
): value is IdeAskQuestionsPayload {
  if (!value || typeof value !== "object") return false;
  const payload = value as Record<string, unknown>;
  if (!validMetadata(payload)) return false;
  if (!nonEmptyString(payload.chatId)) return false;
  if (!nonEmptyString(payload.toolCallId)) return false;
  if (!Array.isArray(payload.questions) || payload.questions.length === 0) {
    return false;
  }
  if (
    payload.askedAtMs !== undefined &&
    (typeof payload.askedAtMs !== "number" ||
      !Number.isSafeInteger(payload.askedAtMs) ||
      payload.askedAtMs < 0)
  ) {
    return false;
  }
  if (
    payload.origin !== undefined &&
    !IDE_DECISION_ORIGINS.includes(payload.origin as IdeDecisionOrigin)
  ) {
    return false;
  }
  return payload.questions.every((question) => {
    if (!question || typeof question !== "object") return false;
    const item = question as Record<string, unknown>;
    if (!nonEmptyString(item.id)) return false;
    if (!nonEmptyString(item.type)) return false;
    if (typeof item.text !== "string") return false;
    if (item.options === undefined) return true;
    return (
      Array.isArray(item.options) &&
      item.options.every((option) => typeof option === "string")
    );
  });
}

export function isValidDecisionResolvedPayload(
  value: unknown,
): value is IdeDecisionResolvedPayload {
  if (!value || typeof value !== "object") return false;
  const payload = value as Record<string, unknown>;
  if (!validMetadata(payload)) return false;
  return (
    nonEmptyString(payload.chatId) &&
    nonEmptyString(payload.toolCallId) &&
    (payload.resolution === "answered" ||
      payload.resolution === "cancelled" ||
      payload.resolution === "resolved_elsewhere")
  );
}

function sameScope(left: DecisionScope | null, right: DecisionScope): boolean {
  return (
    left !== null &&
    left.projectRoot === right.projectRoot &&
    left.endpointIdentity === right.endpointIdentity
  );
}

/**
 * Rebinds the tracker to a project/engine endpoint. A different scope drops all
 * prior views instead of carrying foreign-root decisions across.
 */
export function scopeDecisionState(
  state: DecisionAnnouncementState,
  scope: DecisionScope,
): DecisionAnnouncementState {
  if (sameScope(state.scope, scope)) return state;
  if (state.scope === null) return { ...state, scope };
  return { ...emptyDecisionAnnouncementState(), scope };
}

function rememberResolved(
  state: DecisionAnnouncementState,
  key: string,
  digest: string,
  askedAtMs?: number,
): { key: string; digest: string; askedAtMs?: number }[] {
  const existing = state.resolved.find((entry) => entry.key === key);
  if (
    existing &&
    (existing.askedAtMs === undefined ||
      (askedAtMs !== undefined && askedAtMs < existing.askedAtMs))
  )
    return state.resolved;
  const kept = state.resolved.filter((entry) => entry.key !== key);
  kept.push({ key, digest, askedAtMs });
  return kept.slice(-MAX_RESOLVED_TOMBSTONES);
}

export function shouldAnnounceDecision(
  state: DecisionAnnouncementState,
  payload: unknown,
  scope?: DecisionScope,
): DecisionAnnouncementDecision {
  if (!isValidAskQuestionsPayload(payload)) {
    return {
      announce: false,
      reason: "invalid_payload",
      key: null,
      next: state,
    };
  }
  scope = scope ?? payload.scope;
  if (payload.scope && scope && !sameScope(payload.scope, scope)) {
    return { announce: false, reason: "foreign_scope", key: null, next: state };
  }
  let current = state;
  if (scope) {
    if (state.scope !== null && !sameScope(state.scope, scope)) {
      // A decision from another project/engine must not surface here at all.
      return {
        announce: false,
        reason: "foreign_scope",
        key: null,
        next: state,
      };
    }
    current = scopeDecisionState(state, scope);
  }

  const key = decisionCorrelationKey(payload.chatId, payload.toolCallId);
  const digest = decisionQuestionsDigest(payload.questions);
  const origin = payload.origin ?? "native_notification_stream";

  const known = current.outstanding.find(
    (entry) => decisionCorrelationKey(entry.chatId, entry.toolCallId) === key,
  );
  if (known) {
    if (
      known.askedAtMs !== undefined &&
      (payload.askedAtMs === undefined || payload.askedAtMs < known.askedAtMs)
    ) {
      return { announce: false, reason: "duplicate", key, next: current };
    }
    if (known.digest === digest && known.askedAtMs === payload.askedAtMs) {
      return { announce: false, reason: "duplicate", key, next: current };
    }
    // Same native tool call, different question content: the earlier view is
    // stale. Replace it without announcing a second outstanding decision.
    const outstanding = current.outstanding.map((entry) =>
      decisionCorrelationKey(entry.chatId, entry.toolCallId) === key
        ? { ...entry, digest, origin, askedAtMs: payload.askedAtMs }
        : entry,
    );
    const newerIncarnation =
      known.askedAtMs !== undefined &&
      payload.askedAtMs !== undefined &&
      payload.askedAtMs > known.askedAtMs;
    return {
      announce: newerIncarnation,
      reason: newerIncarnation ? "new" : "superseded_content",
      key,
      next: { ...current, outstanding },
    };
  }

  const tombstone = current.resolved.find((entry) => entry.key === key);
  if (
    tombstone &&
    (tombstone.askedAtMs === undefined ||
      payload.askedAtMs === undefined ||
      payload.askedAtMs <= tombstone.askedAtMs)
  ) {
    // Reconnect replay of an already-resolved decision.
    return { announce: false, reason: "already_resolved", key, next: current };
  }

  const seq = current.seq + 1;
  const outstanding = [
    ...current.outstanding,
    {
      chatId: payload.chatId,
      toolCallId: payload.toolCallId,
      digest,
      askedAtMs: payload.askedAtMs,
      origin,
      seq,
    },
  ].slice(-MAX_OUTSTANDING_DECISIONS);

  return {
    announce: true,
    reason: "new",
    key,
    next: {
      ...current,
      seq,
      outstanding,
      resolved: current.resolved.filter((entry) => entry.key !== key),
    },
  };
}

export function applyDecisionResolution(
  state: DecisionAnnouncementState,
  payload: unknown,
): DecisionResolutionOutcome {
  if (!isValidDecisionResolvedPayload(payload)) {
    return { changed: false, reason: "invalid_payload", next: state };
  }
  if (payload.scope && !sameScope(state.scope, payload.scope)) {
    return { changed: false, reason: "invalid_payload", next: state };
  }
  const key = decisionCorrelationKey(payload.chatId, payload.toolCallId);
  const known = state.outstanding.find(
    (entry) => decisionCorrelationKey(entry.chatId, entry.toolCallId) === key,
  );
  if (known?.askedAtMs !== undefined && payload.askedAtMs !== known.askedAtMs) {
    return { changed: false, reason: "invalid_payload", next: state };
  }
  if (!known && payload.askedAtMs === undefined) {
    return { changed: false, reason: "unknown_decision", next: state };
  }
  if (!known) {
    // Resolution for something never announced here (answered in another
    // client before this host connected): still tombstone it so a later replay
    // cannot raise a decision the engine already closed.
    return {
      changed: false,
      reason: "unknown_decision",
      next: {
        ...state,
        resolved: rememberResolved(state, key, "", payload.askedAtMs),
      },
    };
  }
  return {
    changed: true,
    reason: "resolved",
    next: {
      ...state,
      outstanding: state.outstanding.filter(
        (entry) =>
          decisionCorrelationKey(entry.chatId, entry.toolCallId) !== key,
      ),
      resolved: rememberResolved(state, key, known.digest, known.askedAtMs),
    },
  };
}

/**
 * Reconciles against an explicit, complete native listing. Any error, partial
 * read, or truncated page must arrive with `authoritative: false`, in which case
 * nothing is resolved: missing data never implies a decision went away.
 */
export function reconcileDecisionSnapshot(
  state: DecisionAnnouncementState,
  snapshot: NativeDecisionSnapshotInput,
): SnapshotReconciliation {
  if (state.scope !== null && !sameScope(state.scope, snapshot.scope)) {
    return {
      resolved: [],
      reason: "foreign_scope",
      next: state,
    };
  }
  const scoped = scopeDecisionState(state, snapshot.scope);
  if (!snapshot.authoritative || snapshot.truncated === true) {
    return { resolved: [], reason: "not_authoritative", next: scoped };
  }
  let next = scoped;
  const resolved: IdeDecisionResolvedPayload[] = [];
  for (const payload of snapshot.resolved ?? []) {
    const outcome = applyDecisionResolution(next, {
      ...payload,
      scope: snapshot.scope,
    });
    next = outcome.next;
    if (outcome.changed) resolved.push(payload);
  }
  return { resolved, reason: "reconciled", next };
}

export type ChatDecisionSnapshotInput = {
  scope: DecisionScope;
  chatId: string;
  /**
   * True only when the native chat reported its complete pending-question set
   * (a `snapshot` envelope). A dropped connection, an error frame, or an
   * incremental delta must pass false: silence is not resolution.
   */
  authoritative: boolean;
  /** Tool call ids still pending in that native chat. */
  pending: string[];
  resolved?: IdeDecisionResolvedPayload[];
};

/** Reconciles explicit native terminal records for one chat, never omissions. */
export function reconcileChatDecisions(
  state: DecisionAnnouncementState,
  snapshot: ChatDecisionSnapshotInput,
): SnapshotReconciliation {
  if (state.scope !== null && !sameScope(state.scope, snapshot.scope)) {
    return {
      resolved: [],
      reason: "foreign_scope",
      next: state,
    };
  }
  const scoped = scopeDecisionState(state, snapshot.scope);
  if (!snapshot.authoritative) {
    return { resolved: [], reason: "not_authoritative", next: scoped };
  }
  return reconcileDecisionSnapshot(scoped, {
    scope: snapshot.scope,
    authoritative: true,
    pending: [],
    resolved: snapshot.resolved?.filter(
      (entry) => entry.chatId === snapshot.chatId,
    ),
  });
}

export function outstandingDecisionCount(
  state: DecisionAnnouncementState,
): number {
  return state.outstanding.length;
}

/**
 * Golden parity corpus. The VS Code and JetBrains trackers assert the same
 * digests and outcomes against literal copies of these wire cases, which is
 * contract parity — not a substitute for running the actual IDE hosts.
 */
export const HOST_DECISION_PARITY_CASES: {
  name: string;
  payload: IdeAskQuestionsPayload;
  digest: string;
}[] = [
  {
    name: "single yes_no",
    payload: {
      chatId: "chat-a",
      toolCallId: "tool-a",
      questions: [
        { id: "proceed", type: "yes_no", text: "Proceed with scratch check?" },
      ],
    },
    digest: "beee8ea4",
  },
  {
    name: "single_select with options",
    payload: {
      chatId: "chat-a",
      toolCallId: "tool-b",
      questions: [
        {
          id: "pond",
          type: "single_select",
          text: "Pick a pond",
          options: ["north", "south"],
        },
      ],
    },
    digest: "539fd1f6",
  },
  {
    name: "option order is significant",
    payload: {
      chatId: "chat-a",
      toolCallId: "tool-c",
      questions: [
        {
          id: "pond",
          type: "single_select",
          text: "Pick a pond",
          options: ["south", "north"],
        },
      ],
    },
    digest: "12c5c376",
  },
  {
    name: "non-ascii free text",
    payload: {
      chatId: "chat-b",
      toolCallId: "tool-d",
      questions: [
        { id: "note", type: "free_text", text: "Отчёт: продолжить? ✅" },
      ],
    },
    digest: "26382507",
  },
];
