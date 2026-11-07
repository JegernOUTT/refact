import { describe, expect, it } from "vitest";
import {
  CONTINUITY_LONG_ABSENCE_MS,
  CONTINUITY_WELCOME_BACK_MS,
  PRESENCE_AMBIENT_FRAME_MS,
  PRESENCE_LOW_POWER_FRAME_MS,
  PRESENCE_REDUCED_MOTION_FRAME_MS,
  buddyPresenceMotion,
  buddyWorkStateFromFacts,
  resolveBuddyContinuity,
  resolveBuddyMotionTransition,
  resolveBuddyPresenceLoop,
  shouldDrawPresenceFrame,
  shouldRunPresenceLoop,
  shouldSuppressAmbientMotion,
} from "./buddyPresenceMotion";
import {
  advanceBuddyPlaySession,
  createFetchSession,
  createFireflySession,
  describePlaySessionForScreenReader,
  playKeyboardTargetForKey,
  playKeyboardTargets,
  playSessionCompletion,
  resumePlaySession,
  suspendPlaySession,
  throwFetchBall,
  FETCH_PHASE_DURATIONS_MS,
} from "./buddyPlaySessions";

describe("buddy presence motion grammar", () => {
  it("uses real native waiting states ahead of work and never treats history as execution", () => {
    const native = {
      state: "idle",
      pending_questions: 0,
      approval_count: 0,
      browser_decision_pending: false,
    };
    expect(buddyWorkStateFromFacts([{ state: "delivered" }], [native])).toBe(
      "idle",
    );
    expect(
      buddyWorkStateFromFacts([], [{ ...native, state: "generating" }]),
    ).toBe("working");
    expect(
      buddyWorkStateFromFacts(
        [{ state: "working" }],
        [{ ...native, approval_count: 1 }],
      ),
    ).toBe("awaiting_decision");
    expect(
      buddyWorkStateFromFacts([], [{ ...native, state: "waiting_ide" }]),
    ).toBe("blocked");
    expect(buddyWorkStateFromFacts([], [], true)).toBe("awaiting_decision");
  });
  it("orders priority so urgent states outrank ambient ones", () => {
    expect(buddyPresenceMotion("awaiting_decision").priority).toBeGreaterThan(
      buddyPresenceMotion("blocked").priority,
    );
    expect(buddyPresenceMotion("blocked").priority).toBeGreaterThan(
      buddyPresenceMotion("working").priority,
    );
    expect(buddyPresenceMotion("working").priority).toBeGreaterThan(
      buddyPresenceMotion("idle").priority,
    );
  });

  it("holds when the state does not change", () => {
    const transition = resolveBuddyMotionTransition("working", "working");
    expect(transition.kind).toBe("hold");
    expect(transition.durationMs).toBe(0);
    expect(transition.cancelsPrevious).toBe(false);
  });

  it("preempts and cancels lower-priority motion immediately", () => {
    const transition = resolveBuddyMotionTransition(
      "idle",
      "awaiting_decision",
    );
    expect(transition.kind).toBe("preempt");
    expect(transition.cancelsPrevious).toBe(true);
    expect(transition.durationMs).toBe(0);
  });

  it("blends downward transitions but drops the blend under reduced motion", () => {
    const blended = resolveBuddyMotionTransition("blocked", "idle");
    expect(blended.kind).toBe("blend");
    expect(blended.durationMs).toBe(buddyPresenceMotion("idle").transitionMs);

    const reduced = resolveBuddyMotionTransition("blocked", "idle", {
      reducedMotion: true,
    });
    expect(reduced.kind).toBe("blend");
    expect(reduced.durationMs).toBe(0);
  });

  it("rejects a non-preempting change while the current motion is locked", () => {
    const rejected = resolveBuddyMotionTransition("working", "idle", {
      currentLocked: true,
    });
    expect(rejected.kind).toBe("rejected");
    expect(rejected.to).toBe("working");

    const stillPreempts = resolveBuddyMotionTransition("working", "blocked", {
      currentLocked: true,
    });
    expect(stillPreempts.kind).toBe("preempt");
  });

  it("suppresses ambient motion for attention states and open speech layers", () => {
    expect(shouldSuppressAmbientMotion({ state: "awaiting_decision" })).toBe(
      true,
    );
    expect(shouldSuppressAmbientMotion({ state: "idle" })).toBe(false);
    expect(
      shouldSuppressAmbientMotion({ state: "idle", speechVisible: true }),
    ).toBe(true);
    expect(
      shouldSuppressAmbientMotion({ state: "idle", controlsVisible: true }),
    ).toBe(true);
  });
});

describe("presence loop policy", () => {
  it("stops the loop when the document is hidden", () => {
    const policy = resolveBuddyPresenceLoop({
      mounted: true,
      documentHidden: true,
    });
    expect(policy.running).toBe(false);
    expect(policy.stopReason).toBe("document_hidden");
    expect(shouldDrawPresenceFrame(policy, 1_000, 0)).toBe(false);
  });

  it("stops the loop when unmounted, surface hidden, or explicitly disabled", () => {
    expect(
      resolveBuddyPresenceLoop({ mounted: false, documentHidden: false })
        .stopReason,
    ).toBe("unmounted");
    expect(
      resolveBuddyPresenceLoop({
        mounted: true,
        documentHidden: false,
        surfaceVisible: false,
      }).stopReason,
    ).toBe("surface_hidden");
    expect(
      resolveBuddyPresenceLoop({
        mounted: true,
        documentHidden: false,
        enabled: false,
      }).stopReason,
    ).toBe("loop_disabled");
  });

  it("keeps interactive frames unthrottled and ambient frames throttled", () => {
    const interactive = resolveBuddyPresenceLoop({
      mounted: true,
      documentHidden: false,
      interactive: true,
    });
    expect(interactive.frameIntervalMs).toBe(0);
    expect(shouldDrawPresenceFrame(interactive, 1, 0)).toBe(true);

    const ambient = resolveBuddyPresenceLoop({
      mounted: true,
      documentHidden: false,
    });
    expect(ambient.frameIntervalMs).toBe(PRESENCE_AMBIENT_FRAME_MS);
    expect(shouldDrawPresenceFrame(ambient, 10, 0)).toBe(false);
    expect(shouldDrawPresenceFrame(ambient, PRESENCE_AMBIENT_FRAME_MS, 0)).toBe(
      true,
    );
  });

  it("preserves a caller-owned ambient interval instead of overriding it", () => {
    const policy = resolveBuddyPresenceLoop({
      mounted: true,
      documentHidden: false,
      ambientFrameIntervalMs: 33,
    });
    expect(policy.frameIntervalMs).toBe(33);
    expect(shouldDrawPresenceFrame(policy, 32, 0)).toBe(false);
    expect(shouldDrawPresenceFrame(policy, 33, 0)).toBe(true);
  });

  it("slows but does not stop the loop for reduced motion and low power", () => {
    const reduced = resolveBuddyPresenceLoop({
      mounted: true,
      documentHidden: false,
      reducedMotion: true,
      interactive: true,
    });
    expect(reduced.running).toBe(true);
    expect(reduced.frameIntervalMs).toBe(PRESENCE_REDUCED_MOTION_FRAME_MS);

    const lowPower = resolveBuddyPresenceLoop({
      mounted: true,
      documentHidden: false,
      lowPower: true,
      interactive: true,
    });
    expect(lowPower.running).toBe(true);
    expect(lowPower.frameIntervalMs).toBe(PRESENCE_LOW_POWER_FRAME_MS);
  });

  it("exposes a boolean helper matching the policy", () => {
    expect(
      shouldRunPresenceLoop({ mounted: true, documentHidden: false }),
    ).toBe(true);
    expect(shouldRunPresenceLoop({ mounted: true, documentHidden: true })).toBe(
      false,
    );
  });
});

describe("non-punitive continuity", () => {
  it("never applies a penalty regardless of absence length", () => {
    for (const awayMs of [
      0,
      CONTINUITY_WELCOME_BACK_MS,
      CONTINUITY_LONG_ABSENCE_MS * 4,
    ]) {
      const continuity = resolveBuddyContinuity({
        lastSeenAtMs: 1_000_000,
        nowMs: 1_000_000 + awayMs,
      });
      expect(continuity.penaltyApplied).toBe(false);
      expect(continuity.awayMs).toBe(awayMs);
    }
  });

  it("classifies short, medium and long absences", () => {
    const base = 5_000_000;
    expect(
      resolveBuddyContinuity({ lastSeenAtMs: base, nowMs: base + 60_000 })
        .greeting,
    ).toBe("none");
    expect(
      resolveBuddyContinuity({
        lastSeenAtMs: base,
        nowMs: base + CONTINUITY_WELCOME_BACK_MS,
      }).greeting,
    ).toBe("welcome_back");
    expect(
      resolveBuddyContinuity({
        lastSeenAtMs: base,
        nowMs: base + CONTINUITY_LONG_ABSENCE_MS,
      }).greeting,
    ).toBe("long_absence");
  });

  it("clamps negative or non-finite clocks to zero", () => {
    expect(
      resolveBuddyContinuity({ lastSeenAtMs: 10_000, nowMs: 1_000 }).awayMs,
    ).toBe(0);
    expect(
      resolveBuddyContinuity({ lastSeenAtMs: Number.NaN, nowMs: Number.NaN })
        .awayMs,
    ).toBe(0);
  });
});

describe("play suspension, resume and completion", () => {
  it("resumes a suspended phase at the same elapsed time", () => {
    const started = createFetchSession(1_000);
    const thrown = throwFetchBall(started, 60, 78, 50, 1_000);
    const suspended = suspendPlaySession(thrown, 1_400, "user_paused");
    expect(suspended.phaseElapsedMs).toBe(400);

    const resumed = resumePlaySession(suspended, 90_000);
    expect(resumed.phase).toBe("throwing");
    expect(90_000 - resumed.phaseStartedAtMs).toBe(400);

    // The frozen phase does not advance across the pause…
    expect(advanceBuddyPlaySession(resumed, 90_100)?.phase).toBe("throwing");
    // …and completes only after the real remaining duration elapses.
    const remaining = FETCH_PHASE_DURATIONS_MS.throwing - 400;
    expect(advanceBuddyPlaySession(resumed, 90_000 + remaining)?.phase).toBe(
      "chasing",
    );
  });

  it("keeps armed timeout accounting across a suspension", () => {
    const armed = createFetchSession(2_000);
    const suspended = suspendPlaySession(armed, 12_000, "hidden");
    expect(suspended.armedElapsedMs).toBe(10_000);
    const resumed = resumePlaySession(suspended, 500_000);
    expect(500_000 - resumed.armedAtMs).toBe(10_000);
  });

  it("reports real completion outcomes and never affects work", () => {
    const fetchDone = {
      ...createFetchSession(0),
      phase: "done" as const,
      throwCount: 3,
    };
    const finished = playSessionCompletion(fetchDone, "finished");
    expect(finished.outcome).toBe("completed");
    expect(finished.rounds).toBe(3);
    expect(finished.affectsWork).toBe(false);

    const partial = playSessionCompletion(
      { ...createFetchSession(0), phase: "wiggling" as const, throwCount: 1 },
      "interrupted",
    );
    expect(partial.outcome).toBe("interrupted");
    expect(partial.rounds).toBe(1);

    const untouched = playSessionCompletion(
      createFireflySession(0),
      "interrupted",
    );
    expect(untouched.outcome).toBe("abandoned");
    expect(untouched.rounds).toBe(0);
  });

  it("does not report completion for an interrupted session that reached done", () => {
    const doneish = {
      ...createFireflySession(0),
      phase: "done" as const,
      catches: 3,
    };
    expect(playSessionCompletion(doneish, "interrupted").outcome).toBe(
      "interrupted",
    );
  });
});

describe("play keyboard targets", () => {
  it("offers a keyboard equivalent for every pointer target inside play bounds", () => {
    for (const kind of ["fetch", "firefly"] as const) {
      const targets = playKeyboardTargets(kind);
      expect(targets.length).toBeGreaterThanOrEqual(3);
      const keys = new Set(targets.map((target) => target.key));
      expect(keys.size).toBe(targets.length);
      for (const target of targets) {
        expect(target.x).toBeGreaterThanOrEqual(33);
        expect(target.x).toBeLessThanOrEqual(67);
        expect(target.y).toBeGreaterThanOrEqual(70);
        expect(target.y).toBeLessThanOrEqual(84);
        expect(target.label.length).toBeGreaterThan(0);
      }
    }
  });

  it("resolves keys to targets and ignores unknown keys", () => {
    expect(playKeyboardTargetForKey("fetch", "2")?.id).toBe("centre");
    expect(playKeyboardTargetForKey("firefly", "3")?.id).toBe("high");
    expect(playKeyboardTargetForKey("fetch", "z")).toBeNull();
  });

  it("describes session state for screen readers without inventing progress", () => {
    expect(describePlaySessionForScreenReader(null, false)).toContain(
      "No play session",
    );
    const armed = createFetchSession(0);
    expect(describePlaySessionForScreenReader(armed, false)).toContain(
      "0 of 3",
    );
    expect(describePlaySessionForScreenReader(armed, true)).toContain("paused");
  });
});
