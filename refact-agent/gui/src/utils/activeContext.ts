import {
  getReconstructedHistoryMetadata,
  isLegacyContextSummary,
  type ChatMessage,
  type ChatMessages,
  type ReconstructedHistoryMetadata,
} from "../services/refact/types";

/**
 * Unified context rebuild — GUI side.
 *
 * The stored transcript is always the full, untouched history: every original
 * message stays browseable in the archive. What the model actually sees (the
 * "active context") is derived here, from the newest `reconstructed_history`
 * report's `payload.messages` plus every message that came after that report.
 *
 * Rules encoded below, in priority order:
 *  - Only `reconstructed_history` reports are anchors. Deterministic/static
 *    reports (`chat_compression_report`) are never anchors, so an in-place
 *    static compression cannot silently truncate the active view.
 *  - The LATEST anchor wins; earlier anchors are already folded into it.
 *  - A malformed payload or an unknown `schema_version` BLOCKS. There is no
 *    fallback to the raw transcript: showing a plausible-but-wrong active
 *    context is worse than surfacing the problem.
 *  - A thread that only has legacy compression artifacts (LLM segment summaries
 *    but no reconstructed-history report) reports
 *    `legacy_rebuild_required` so the UI can ask for an explicit rebuild while
 *    still rendering the whole transcript.
 */
export type ActiveContextStatus =
  | "none"
  | "reconstructed"
  | "blocked"
  | "legacy_rebuild_required";

export type ActiveContextBlockedReason =
  | "unsupported_schema_version"
  | "malformed_payload";

export type ActiveContext = {
  status: ActiveContextStatus;
  /**
   * Messages the model is expected to see. Empty when the active view cannot be
   * trusted (`blocked` or `legacy_rebuild_required`).
   */
  active: ChatMessages;
  /**
   * The complete stored history, always intact and always safe to browse or
   * archive, regardless of status.
   */
  transcript: ChatMessages;
  /** Index in the source transcript of the anchor report, when there is one. */
  anchorIndex: number | null;
  /** Anchor report metadata, for disclosure rendering. */
  metadata: ReconstructedHistoryMetadata | null;
  /** Number of messages appended after the anchor (the "suffix"). */
  suffixCount: number;
  reason: ActiveContextBlockedReason | null;
};

const EMPTY_MESSAGES: ChatMessages = [];

function hasLegacySummaryArtifacts(messages: ChatMessages): boolean {
  return messages.some(isLegacyContextSummary);
}

/**
 * Finds the newest reconstructed-history report. Scans from the end so the
 * latest anchor wins, and stops at the first message that claims the kind —
 * including a malformed one, which must block rather than be skipped over in
 * favour of an older, stale anchor.
 */
function findAnchor(messages: ChatMessages): {
  index: number;
  parse: NonNullable<ReturnType<typeof getReconstructedHistoryMetadata>>;
} | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const parse = getReconstructedHistoryMetadata(messages[index]);
    if (parse) return { index, parse };
  }
  return null;
}

export function computeActiveContext(
  messages: ChatMessages | undefined,
): ActiveContext {
  const source = messages ?? EMPTY_MESSAGES;
  const anchor = findAnchor(source);

  if (!anchor) {
    const legacy = hasLegacySummaryArtifacts(source);
    return {
      // A legacy-summarized chat has NO trustworthy active context: the raw
      // transcript still contains the messages the old summary replaced, so
      // handing it back as "active" would silently double-count them. Callers
      // that want the transcript must read `transcript`, never `active`.
      status: legacy ? "legacy_rebuild_required" : "none",
      active: legacy ? EMPTY_MESSAGES : source,
      transcript: source,
      anchorIndex: null,
      metadata: null,
      suffixCount: 0,
      reason: null,
    };
  }

  if (!anchor.parse.ok) {
    return {
      status: "blocked",
      active: EMPTY_MESSAGES,
      transcript: source,
      anchorIndex: anchor.index,
      metadata: null,
      suffixCount: 0,
      reason: anchor.parse.reason,
    };
  }

  const suffix = source.slice(anchor.index + 1);
  const legacy = hasLegacySummaryArtifacts(suffix);
  const active = [...anchor.parse.metadata.payload.messages, ...suffix];
  const ids = active.map((message) => message.message_id).filter(Boolean);
  const duplicates = new Set(ids).size !== ids.length;
  return {
    status: legacy
      ? "legacy_rebuild_required"
      : duplicates
        ? "blocked"
        : "reconstructed",
    active: legacy || duplicates ? EMPTY_MESSAGES : active,
    transcript: source,
    anchorIndex: anchor.index,
    metadata: anchor.parse.metadata,
    suffixCount: suffix.length,
    reason: duplicates ? "malformed_payload" : null,
  };
}

/**
 * True when the server-side gate means normal generation must not start until
 * the user rebuilds the context. Blocked (untrustworthy new-kind report) and
 * legacy (no new-kind report at all) both qualify; the transcript stays visible
 * in either case.
 */
export function isGenerationGatedByContext(context: ActiveContext): boolean {
  return (
    context.status === "blocked" || context.status === "legacy_rebuild_required"
  );
}

export function describeActiveContextGate(
  context: ActiveContext,
): string | null {
  switch (context.status) {
    case "blocked":
      return context.reason === "unsupported_schema_version"
        ? "This chat's rebuilt context uses a newer format this client cannot read. Update the client, or rebuild the context."
        : "This chat's rebuilt context could not be read. Rebuild the context to continue.";
    case "legacy_rebuild_required":
      return "This chat was compressed with an older format. Rebuild the context to continue generating.";
    default:
      return null;
  }
}

/** Message ids present in the rebuilt payload, for de-duplicating rendering. */
export function activeContextPayloadMessageIds(
  context: ActiveContext,
): ReadonlySet<string> {
  const ids = new Set<string>();
  if (!context.metadata) return ids;
  for (const message of context.metadata.payload.messages) {
    const id = (message as ChatMessage & { message_id?: string }).message_id;
    if (typeof id === "string") ids.add(id);
  }
  return ids;
}
