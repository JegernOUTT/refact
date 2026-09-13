import {
  applyDecisionResolution,
  emptyDecisionAnnouncementState,
  outstandingDecisionCount,
  reconcileChatDecisions,
  reconcileDecisionSnapshot,
  scopeDecisionState,
  shouldAnnounceDecision,
  type ChatDecisionSnapshotInput,
  type DecisionAnnouncementState,
  type DecisionScope,
  type IdeAskQuestionsPayload,
  type IdeDecisionResolvedPayload,
  type NativeDecisionSnapshotInput,
} from "./hostDecisions";

/**
 * Process-wide announcement state for the host decision bridge.
 *
 * The web shell produces host notifications from two independent places: the
 * sidebar notification stream and the chat-event listener in the store
 * middleware. They observe the same native decision, so they have to share one
 * announcement state or every decision is announced twice.
 *
 * The state is deliberately not in Redux: it is host transport bookkeeping, not
 * application state, and it must keep working while no chat or tool card is
 * mounted.
 */
let state: DecisionAnnouncementState = emptyDecisionAnnouncementState();

const listeners = new Set<(outstanding: number) => void>();

function commit(next: DecisionAnnouncementState): void {
  const before = state.outstanding.length;
  state = next;
  const after = state.outstanding.length;
  if (before === after) return;
  for (const listener of listeners) listener(after);
}

export function hostDecisionState(): DecisionAnnouncementState {
  return state;
}

export function hostDecisionOutstanding(): number {
  return outstandingDecisionCount(state);
}

export function resetHostDecisions(): void {
  commit(emptyDecisionAnnouncementState());
}

export function subscribeHostDecisions(
  listener: (outstanding: number) => void,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Records an observed native decision and reports whether this host still owes
 * the user a notification for it. A repeat delivery, a reconnect replay, or an
 * already-resolved decision answers false.
 */
export function announceHostDecision(
  payload: IdeAskQuestionsPayload,
  scope: DecisionScope,
): boolean {
  const outcome = shouldAnnounceDecision(state, payload, scope);
  commit(outcome.next);
  return outcome.announce;
}

/** Retires one decision this host already knows about. */
export function resolveHostDecision(
  payload: IdeDecisionResolvedPayload,
): boolean {
  const outcome = applyDecisionResolution(state, payload);
  commit(outcome.next);
  return outcome.changed;
}

/**
 * Reconciles every tracked decision against a complete native listing. Returns
 * the decisions that stopped waiting, which the caller forwards to the host.
 */
export function reconcileHostDecisionSnapshot(
  snapshot: NativeDecisionSnapshotInput,
): { chatId: string; toolCallId: string }[] {
  const outcome = reconcileDecisionSnapshot(state, snapshot);
  commit(outcome.next);
  return outcome.resolved;
}

/** Reconciles one chat against its own authoritative native listing. */
export function reconcileHostChatDecisions(
  snapshot: ChatDecisionSnapshotInput,
): { chatId: string; toolCallId: string }[] {
  const outcome = reconcileChatDecisions(state, snapshot);
  commit(outcome.next);
  return outcome.resolved;
}

/** Drops announcements that belong to a different project or engine endpoint. */
export function scopeHostDecisions(scope: DecisionScope): void {
  commit(scopeDecisionState(state, scope));
}

/** Observe explicit native statuses only. Missing entries are never outcomes. */
export function observeHostQuestionStatuses(
  scope: DecisionScope,
  chatId: string,
  questions:
    | readonly {
        tool_call_id: string;
        asked_at_ms: number;
        status: string;
      }[]
    | undefined,
): IdeDecisionResolvedPayload[] {
  if (
    !questions ||
    !state.scope ||
    state.scope.projectRoot !== scope.projectRoot ||
    state.scope.endpointIdentity !== scope.endpointIdentity
  )
    return [];
  const resolved: IdeDecisionResolvedPayload[] = [];
  for (const question of questions) {
    if (question.status !== "answered" && question.status !== "cancelled")
      continue;
    const payload: IdeDecisionResolvedPayload = {
      chatId,
      toolCallId: question.tool_call_id,
      askedAtMs: question.asked_at_ms,
      resolution: question.status,
      origin: "native_snapshot",
      scope,
    };
    // Forward even unknown outcomes once: the host may have announced before
    // this webview was recreated. Tombstones suppress reconnect duplicates.
    const key = `${chatId}\u001f${question.tool_call_id}`;
    if (
      state.resolved.some(
        (entry) =>
          entry.key === key &&
          (entry.askedAtMs === undefined ||
            entry.askedAtMs >= question.asked_at_ms),
      )
    )
      continue;
    const outcome = applyDecisionResolution(state, payload);
    commit(outcome.next);
    if (outcome.reason !== "invalid_payload") resolved.push(payload);
  }
  return resolved;
}
