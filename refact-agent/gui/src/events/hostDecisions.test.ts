import { describe, expect, it } from "vitest";
import {
  DECISION_BRIDGE_CONTRACT_VERSION,
  HOST_DECISION_INTENTS,
  HOST_DECISION_PARITY_CASES,
  MAX_OUTSTANDING_DECISIONS,
  MAX_RESOLVED_TOMBSTONES,
  applyDecisionResolution,
  decisionCorrelationKey,
  decisionQuestionsDigest,
  emptyDecisionAnnouncementState,
  isValidAskQuestionsPayload,
  isValidDecisionResolvedPayload,
  outstandingDecisionCount,
  reconcileDecisionSnapshot,
  scopeDecisionState,
  shouldAnnounceDecision,
  type DecisionScope,
  type IdeAskQuestionsPayload,
} from "./hostDecisions";

const scope: DecisionScope = {
  projectRoot: "/scratch/operator",
  endpointIdentity: "http://127.0.0.1:8123/project/scratch",
};

const otherScope: DecisionScope = {
  projectRoot: "/scratch/other",
  endpointIdentity: "http://127.0.0.1:8123/project/other",
};

const askA: IdeAskQuestionsPayload = {
  chatId: "chat-a",
  toolCallId: "tool-a",
  questions: [
    { id: "proceed", type: "yes_no", text: "Proceed with scratch check?" },
  ],
  origin: "native_notification_stream",
};

describe("host decision bridge contract", () => {
  it("keeps the host vocabulary free of any consent capability", () => {
    expect(DECISION_BRIDGE_CONTRACT_VERSION).toBe(1);
    expect([...HOST_DECISION_INTENTS]).toEqual([
      "notify",
      "open_chat",
      "update_outstanding_badge",
    ]);
    const forbidden = /approve|approval|deny|allow|grant|consent|accept/i;
    for (const intent of HOST_DECISION_INTENTS) {
      expect(intent).not.toMatch(forbidden);
    }
  });

  it("pins golden digests so every host derives the same correlation identity", () => {
    for (const parityCase of HOST_DECISION_PARITY_CASES) {
      expect(decisionQuestionsDigest(parityCase.payload.questions)).toBe(
        parityCase.digest,
      );
    }
    const digests = HOST_DECISION_PARITY_CASES.map((c) => c.digest);
    expect(new Set(digests).size).toBe(digests.length);
    expect(decisionCorrelationKey("chat-a", "tool-a")).not.toBe(
      decisionCorrelationKey("chat-a\u001ftool", "a"),
    );
  });

  it("rejects malformed announcements instead of fabricating a decision", () => {
    expect(isValidAskQuestionsPayload(askA)).toBe(true);
    expect(isValidAskQuestionsPayload({ ...askA, questions: [] })).toBe(false);
    expect(isValidAskQuestionsPayload({ ...askA, chatId: "" })).toBe(false);
    expect(isValidAskQuestionsPayload({ ...askA, toolCallId: "" })).toBe(false);
    expect(isValidAskQuestionsPayload({ ...askA, askedAtMs: -1 })).toBe(false);
    expect(isValidAskQuestionsPayload({ ...askA, origin: "user" })).toBe(false);
    expect(
      isValidAskQuestionsPayload({
        ...askA,
        questions: [{ id: "q", type: "yes_no" }],
      }),
    ).toBe(false);
    expect(
      shouldAnnounceDecision(emptyDecisionAnnouncementState(), null),
    ).toMatchObject({ announce: false, reason: "invalid_payload" });

    expect(
      isValidDecisionResolvedPayload({
        chatId: "chat-a",
        toolCallId: "tool-a",
        resolution: "answered",
      }),
    ).toBe(true);
    expect(
      isValidDecisionResolvedPayload({
        chatId: "chat-a",
        toolCallId: "tool-a",
        resolution: "approved",
      }),
    ).toBe(false);
  });

  it("announces once and treats a repeated delivery as a duplicate", () => {
    const first = shouldAnnounceDecision(
      emptyDecisionAnnouncementState(),
      askA,
      scope,
    );
    expect(first).toMatchObject({ announce: true, reason: "new" });
    expect(outstandingDecisionCount(first.next)).toBe(1);

    const second = shouldAnnounceDecision(first.next, askA, scope);
    expect(second).toMatchObject({ announce: false, reason: "duplicate" });
    expect(outstandingDecisionCount(second.next)).toBe(1);

    // Same decision arriving through the other producer path.
    const viaChatEvent = shouldAnnounceDecision(
      second.next,
      { ...askA, origin: "native_chat_event" },
      scope,
    );
    expect(viaChatEvent).toMatchObject({
      announce: false,
      reason: "duplicate",
    });
    expect(outstandingDecisionCount(viaChatEvent.next)).toBe(1);
  });

  it("does not re-announce after a reconnect replays a resolved decision", () => {
    const announced = shouldAnnounceDecision(
      emptyDecisionAnnouncementState(),
      askA,
      scope,
    );
    const resolved = applyDecisionResolution(announced.next, {
      chatId: "chat-a",
      toolCallId: "tool-a",
      resolution: "answered",
    });
    expect(resolved).toMatchObject({ changed: true, reason: "resolved" });
    expect(outstandingDecisionCount(resolved.next)).toBe(0);

    const replay = shouldAnnounceDecision(resolved.next, askA, scope);
    expect(replay).toMatchObject({
      announce: false,
      reason: "already_resolved",
    });
    expect(outstandingDecisionCount(replay.next)).toBe(0);
  });

  it("re-announces only when the native question content actually changed", () => {
    const announced = shouldAnnounceDecision(
      emptyDecisionAnnouncementState(),
      askA,
      scope,
    );
    const superseded = shouldAnnounceDecision(
      announced.next,
      {
        ...askA,
        questions: [
          { id: "proceed", type: "yes_no", text: "Proceed with a full check?" },
        ],
      },
      scope,
    );
    expect(superseded).toMatchObject({
      announce: false,
      reason: "superseded_content",
    });
    expect(outstandingDecisionCount(superseded.next)).toBe(1);
    expect(superseded.next.outstanding[0].digest).not.toBe(
      announced.next.outstanding[0].digest,
    );
  });

  it("ignores unknown legacy resolutions without an incarnation timestamp", () => {
    const outcome = applyDecisionResolution(emptyDecisionAnnouncementState(), {
      chatId: "chat-z",
      toolCallId: "tool-z",
      resolution: "cancelled",
    });
    expect(outcome).toMatchObject({
      changed: false,
      reason: "unknown_decision",
    });
    const replay = shouldAnnounceDecision(
      outcome.next,
      { ...askA, chatId: "chat-z", toolCallId: "tool-z" },
      scope,
    );
    // Unknown legacy outcomes cannot establish an indefinite tombstone.
    expect(replay).toMatchObject({ announce: true, reason: "new" });
  });

  it("resolves outstanding decisions only from a complete native listing", () => {
    const announced = shouldAnnounceDecision(
      emptyDecisionAnnouncementState(),
      askA,
      scope,
    );

    const unavailable = reconcileDecisionSnapshot(announced.next, {
      scope,
      authoritative: false,
      pending: [],
    });
    expect(unavailable).toMatchObject({
      reason: "not_authoritative",
      resolved: [],
    });
    expect(outstandingDecisionCount(unavailable.next)).toBe(1);

    const truncated = reconcileDecisionSnapshot(announced.next, {
      scope,
      authoritative: true,
      truncated: true,
      pending: [],
    });
    expect(truncated).toMatchObject({
      reason: "not_authoritative",
      resolved: [],
    });
    expect(outstandingDecisionCount(truncated.next)).toBe(1);

    const stillPending = reconcileDecisionSnapshot(announced.next, {
      scope,
      authoritative: true,
      pending: [{ chatId: "chat-a", toolCallId: "tool-a" }],
    });
    expect(stillPending.resolved).toEqual([]);
    expect(outstandingDecisionCount(stillPending.next)).toBe(1);

    const omitted = reconcileDecisionSnapshot(announced.next, {
      scope,
      authoritative: true,
      pending: [],
    });
    expect(omitted.resolved).toEqual([]);
    expect(outstandingDecisionCount(omitted.next)).toBe(1);
    const answeredElsewhere = reconcileDecisionSnapshot(announced.next, {
      scope,
      authoritative: true,
      pending: [],
      resolved: [
        { chatId: "chat-a", toolCallId: "tool-a", resolution: "answered" },
      ],
    });
    expect(answeredElsewhere).toMatchObject({ reason: "reconciled" });
    expect(answeredElsewhere.resolved).toEqual([
      { chatId: "chat-a", toolCallId: "tool-a", resolution: "answered" },
    ]);
    expect(outstandingDecisionCount(answeredElsewhere.next)).toBe(0);
    expect(
      shouldAnnounceDecision(answeredElsewhere.next, askA, scope),
    ).toMatchObject({ announce: false, reason: "already_resolved" });
  });

  it("never carries decisions across a project or engine endpoint change", () => {
    const announced = shouldAnnounceDecision(
      emptyDecisionAnnouncementState(),
      askA,
      scope,
    );

    const foreign = shouldAnnounceDecision(announced.next, askA, otherScope);
    expect(foreign).toMatchObject({ announce: false, reason: "foreign_scope" });
    expect(outstandingDecisionCount(foreign.next)).toBe(1);

    const rescoped = scopeDecisionState(announced.next, {
      projectRoot: scope.projectRoot,
      endpointIdentity: "http://127.0.0.1:8123/project/scratch-2",
    });
    expect(outstandingDecisionCount(rescoped)).toBe(0);
    expect(rescoped.resolved).toEqual([]);

    const foreignSnapshot = reconcileDecisionSnapshot(announced.next, {
      scope: otherScope,
      authoritative: true,
      pending: [],
    });
    expect(foreignSnapshot).toMatchObject({
      reason: "foreign_scope",
      resolved: [],
    });
    expect(outstandingDecisionCount(foreignSnapshot.next)).toBe(1);
  });

  it("bounds outstanding and tombstone growth", () => {
    let state = scopeDecisionState(emptyDecisionAnnouncementState(), scope);
    const total = MAX_OUTSTANDING_DECISIONS + 25;
    for (let index = 0; index < total; index += 1) {
      state = shouldAnnounceDecision(
        state,
        { ...askA, toolCallId: `tool-${index}` },
        scope,
      ).next;
    }
    expect(outstandingDecisionCount(state)).toBe(MAX_OUTSTANDING_DECISIONS);

    for (let index = 0; index < total; index += 1) {
      state = applyDecisionResolution(state, {
        chatId: "chat-a",
        toolCallId: `tool-${index}`,
        resolution: "answered",
      }).next;
    }
    expect(outstandingDecisionCount(state)).toBe(0);
    expect(state.resolved.length).toBeLessThanOrEqual(MAX_RESOLVED_TOMBSTONES);
  });
});
