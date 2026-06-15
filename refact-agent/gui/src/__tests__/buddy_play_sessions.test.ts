import { describe, expect, it } from "vitest";
import {
  BUDDY_REQUEST_PROMPT,
  FETCH_MAX_THROWS,
  FETCH_PHASE_DURATIONS_MS,
  FIREFLY_MAX_CATCHES,
  FIREFLY_PHASE_DURATIONS_MS,
  GIFT_LINES,
  PLAY_ARMED_TIMEOUT_MS,
  PLAY_SESSION_LINES,
  advanceBuddyPlaySession,
  createFetchSession,
  createFireflySession,
  fetchBallPositionAt,
  maybeCreateGiftMoment,
  playSessionBodyTarget,
  pounceFirefly,
  shouldOfferBuddyRequest,
  throwFetchBall,
  type BuddyFetchSession,
  type BuddyFireflySession,
  type BuddyPlaySession,
} from "../features/Buddy/buddyPlaySessions";
import type { BuddyPetState } from "../features/Buddy/types";

function advancePast(
  session: BuddyPlaySession,
  durations: Record<string, number>,
): BuddyPlaySession | null {
  return advanceBuddyPlaySession(
    session,
    session.phaseStartedAtMs + durations[session.phase] + 1,
  );
}

function makePet(overrides?: Partial<BuddyPetState["needs"]>): BuddyPetState {
  return {
    needs: {
      hunger: 50,
      energy: 50,
      hygiene: 50,
      boredom: 80,
      affection: 70,
      ...overrides,
    },
    condition: {
      sleeping: false,
      hungry: false,
      sleepy: false,
      dirty: false,
      bored: true,
      lonely: false,
    },
    evolution: {
      care_score: 0,
      neglect_score: 0,
      open_seconds: 0,
      last_evolved_at: null,
    },
  };
}

describe("fetch session", () => {
  it("walks throw → chase → carry → wiggle and re-arms", () => {
    let session: BuddyPlaySession | null = throwFetchBall(
      createFetchSession(0),
      60,
      80,
      50,
      1_000,
    );
    expect(session.phase).toBe("throwing");
    session = advancePast(session, FETCH_PHASE_DURATIONS_MS);
    expect(session?.phase).toBe("chasing");
    session = advancePast(
      session as BuddyFetchSession,
      FETCH_PHASE_DURATIONS_MS,
    );
    expect(session?.phase).toBe("carrying");
    session = advancePast(
      session as BuddyFetchSession,
      FETCH_PHASE_DURATIONS_MS,
    );
    expect(session?.phase).toBe("wiggling");
    expect((session as BuddyFetchSession).throwCount).toBe(1);
    session = advancePast(
      session as BuddyFetchSession,
      FETCH_PHASE_DURATIONS_MS,
    );
    expect(session?.phase).toBe("armed");
  });

  it("flops to done after the throw cap", () => {
    const session: BuddyFetchSession = {
      ...createFetchSession(0),
      phase: "wiggling",
      throwCount: FETCH_MAX_THROWS,
    };
    const advanced = advancePast(session, FETCH_PHASE_DURATIONS_MS);
    expect(advanced?.phase).toBe("done");
    expect(
      advancePast(advanced as BuddyFetchSession, FETCH_PHASE_DURATIONS_MS),
    ).toBeNull();
  });

  it("times out an unused armed session", () => {
    const session = createFetchSession(0);
    expect(
      advanceBuddyPlaySession(session, PLAY_ARMED_TIMEOUT_MS + 1),
    ).toBeNull();
  });

  it("clamps throws into the meadow band", () => {
    const session = throwFetchBall(createFetchSession(0), 5, 200, 200, 0);
    expect(session.ballToX).toBeGreaterThanOrEqual(33);
    expect(session.ballToX).toBeLessThanOrEqual(67);
    expect(session.ballToY).toBeGreaterThanOrEqual(70);
    expect(session.ballToY).toBeLessThanOrEqual(84);
    expect(session.ballFromX).toBeLessThanOrEqual(67);
  });

  it("ignores throws outside the armed phase", () => {
    const session: BuddyFetchSession = {
      ...createFetchSession(0),
      phase: "chasing",
    };
    expect(throwFetchBall(session, 60, 80, 50, 0)).toBe(session);
  });

  it("arcs the ball during flight and rests it while chased", () => {
    const thrown = throwFetchBall(createFetchSession(0), 64, 82, 40, 0);
    const midFlight = fetchBallPositionAt(
      thrown,
      FETCH_PHASE_DURATIONS_MS.throwing / 2,
    );
    expect(midFlight?.airborne).toBe(true);
    const linearMidY = (74 + 82) / 2;
    expect(midFlight && midFlight.y < linearMidY).toBe(true);

    const chasing: BuddyFetchSession = { ...thrown, phase: "chasing" };
    const resting = fetchBallPositionAt(chasing, 5_000);
    expect(resting).toEqual({ x: 64, y: 82, airborne: false });
    expect(fetchBallPositionAt({ ...thrown, phase: "armed" }, 0)).toBeNull();
  });
});

describe("firefly session", () => {
  it("accumulates seeded catches up to the cap", () => {
    let session: BuddyPlaySession | null = createFireflySession(7);
    let guard = 0;
    while (session && session.phase !== "done" && guard < 30) {
      guard += 1;
      if (session.phase === "armed") {
        session = pounceFirefly(
          session as BuddyFireflySession,
          50,
          80,
          session.phaseStartedAtMs + 100,
        );
      } else {
        session = advancePast(session, FIREFLY_PHASE_DURATIONS_MS);
      }
    }
    expect(session?.phase).toBe("done");
    expect((session as BuddyFireflySession).catches).toBe(FIREFLY_MAX_CATCHES);
  });

  it("resolves a pounce deterministically per seed", () => {
    const pounced = pounceFirefly(createFireflySession(7), 50, 80, 100);
    const first = advancePast(pounced, FIREFLY_PHASE_DURATIONS_MS);
    const second = advancePast(pounced, FIREFLY_PHASE_DURATIONS_MS);
    expect(first).toEqual(second);
    expect(first?.phase).toBe("resolved");
  });
});

describe("play body targets", () => {
  it("keeps the buddy attentive while armed and chasing afterwards", () => {
    const armed = createFetchSession(0);
    expect(playSessionBodyTarget(armed)).toBeNull();
    const thrown = throwFetchBall(armed, 60, 80, 50, 0);
    expect(playSessionBodyTarget(thrown)).toBeNull();
    const chasing: BuddyFetchSession = { ...thrown, phase: "chasing" };
    expect(playSessionBodyTarget(chasing)).toEqual({
      x: 60,
      y: 80,
      pose: "pounce",
    });
    const pounce = pounceFirefly(createFireflySession(0), 44, 78, 0);
    expect(playSessionBodyTarget(pounce)).toEqual({
      x: 44,
      y: 78,
      pose: "pounce",
    });
    expect(playSessionBodyTarget(null)).toBeNull();
  });
});

describe("gift moments", () => {
  it("only springs from foraging payoffs", () => {
    expect(maybeCreateGiftMoment("wander_curiously", 1, 0)).toBeNull();
    expect(maybeCreateGiftMoment(null, 1, 0)).toBeNull();
  });

  it("is seeded and typed per intent", () => {
    let gifted = 0;
    for (let seed = 0; seed < 60; seed += 1) {
      const gift = maybeCreateGiftMoment("gather_acorns", seed, 123);
      if (gift) {
        gifted += 1;
        expect(gift.item).toBe("acorn");
        expect(gift.startedAtMs).toBe(123);
      }
    }
    expect(gifted).toBeGreaterThan(5);
    expect(gifted).toBeLessThan(40);
  });

  it("covers every gift item with lines", () => {
    for (const pool of Object.values(GIFT_LINES)) {
      expect(pool.lines.length).toBeGreaterThanOrEqual(2);
    }
  });
});

describe("buddy requests", () => {
  const base = {
    nowMs: 20 * 60_000,
    lastOfferAtMs: 0,
    offersThisSession: 0,
    busy: false,
  };

  it("offers when the pet is bored or lonely", () => {
    expect(
      shouldOfferBuddyRequest({ ...base, pet: makePet({ boredom: 80 }) }),
    ).toBe(true);
    expect(
      shouldOfferBuddyRequest({
        ...base,
        pet: makePet({ boredom: 10, affection: 20 }),
      }),
    ).toBe(true);
    expect(
      shouldOfferBuddyRequest({
        ...base,
        pet: makePet({ boredom: 10, affection: 90 }),
      }),
    ).toBe(false);
  });

  it("respects busy, cooldown, and the session cap", () => {
    const pet = makePet({ boredom: 90 });
    expect(shouldOfferBuddyRequest({ ...base, pet, busy: true })).toBe(false);
    expect(
      shouldOfferBuddyRequest({
        ...base,
        pet,
        lastOfferAtMs: base.nowMs - 60_000,
      }),
    ).toBe(false);
    expect(
      shouldOfferBuddyRequest({ ...base, pet, offersThisSession: 2 }),
    ).toBe(false);
    expect(shouldOfferBuddyRequest({ ...base, pet: undefined })).toBe(false);
  });

  it("templates request prompts with the buddy name", () => {
    for (const line of BUDDY_REQUEST_PROMPT.lines) {
      expect(line("Mochi")).toContain("Mochi");
    }
  });
});

describe("play session lines", () => {
  it("covers every fetch and firefly phase voice", () => {
    const keys = [
      "fetch:armed",
      "fetch:throwing",
      "fetch:chasing",
      "fetch:carrying",
      "fetch:wiggling",
      "fetch:done",
      "firefly:armed",
      "firefly:pouncing",
      "firefly:caught",
      "firefly:missed",
      "firefly:done",
    ];
    for (const key of keys) {
      expect(PLAY_SESSION_LINES[key].lines.length).toBeGreaterThanOrEqual(3);
    }
  });
});
