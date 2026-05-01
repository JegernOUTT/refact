import { describe, expect, it } from "vitest";
import {
  advanceBuddyShowcasePhase,
  BUDDY_SHOWCASE_PHASE_DURATIONS_MS,
  chooseBuddyShowcase,
  createBuddyShowcaseRun,
  type BuddyShowcaseTargetCandidate,
} from "../features/Buddy/buddyShowcase";
import type { BuddyPetState, BuddyRuntimeEvent } from "../features/Buddy/types";

const MEMORY_TARGET: BuddyShowcaseTargetCandidate = {
  id: "memory",
  x: 33,
  y: 52,
  label: "Memory fireflies",
  sprite: "memory_fireflies",
};

const OBSERVATORY_TARGET: BuddyShowcaseTargetCandidate = {
  id: "providers",
  x: 72,
  y: 67,
  label: "Model observatory",
  sprite: "observatory",
};

function makePet(sleeping = false): BuddyPetState {
  return {
    needs: {
      hunger: 80,
      energy: 80,
      hygiene: 80,
      boredom: 10,
      affection: 80,
    },
    condition: {
      sleeping,
      hungry: false,
      sleepy: false,
      dirty: false,
      bored: false,
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

function makeRuntimeEvent(
  overrides?: Partial<BuddyRuntimeEvent>,
): BuddyRuntimeEvent {
  return {
    id: "runtime-1",
    signal_type: "memory_extract",
    title: "Memory extracted",
    source: "test",
    status: "completed",
    priority: "normal",
    created_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

describe("buddy showcase director", () => {
  it("chooses and creates memory firefly night for memory runtime signals", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({ signal_type: "knowledge_update" }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 10_000,
      lastShowcaseKind: null,
    };

    expect(chooseBuddyShowcase(args)?.kind).toBe("memory_firefly_night");
    const run = createBuddyShowcaseRun(args);

    expect(run).toMatchObject({
      kind: "memory_firefly_night",
      phase: "travel",
      target: {
        id: "memory",
        label: "Memory fireflies",
      },
      pose: "meditate",
      startedAtMs: 10_000,
      phaseStartedAtMs: 10_000,
    });
  });

  it("chooses and creates stargazing constellation for generation and provider signals", () => {
    const generatingArgs = {
      targets: [OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "streaming",
        title: "Streaming answer",
        status: "streaming",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 20_000,
      lastShowcaseKind: null,
    };
    const providerArgs = {
      ...generatingArgs,
      nowPlaying: makeRuntimeEvent({
        signal_type: "error",
        title: "Provider quota warning",
        description: "The default model quota is low.",
        status: "failed",
      }),
    };

    expect(chooseBuddyShowcase(generatingArgs)?.kind).toBe(
      "stargazing_constellation",
    );
    expect(createBuddyShowcaseRun(generatingArgs)?.target.id).toBe("providers");
    expect(chooseBuddyShowcase(providerArgs)?.kind).toBe(
      "stargazing_constellation",
    );
  });

  it("active speech suppresses chooser and run creation", () => {
    const args = {
      targets: [MEMORY_TARGET],
      nowPlaying: makeRuntimeEvent(),
      activeSpeechVisible: true,
      pet: makePet(),
      nowMs: 30_000,
      lastShowcaseKind: null,
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("sleep and cooldown suppress chooser and run creation", () => {
    const sleepingArgs = {
      targets: [MEMORY_TARGET],
      nowPlaying: makeRuntimeEvent(),
      activeSpeechVisible: false,
      pet: makePet(true),
      nowMs: 35_000,
      lastShowcaseKind: null,
    };
    const cooldownArgs = {
      ...sleepingArgs,
      pet: makePet(false),
      cooldownUntilMs: 40_000,
    };

    expect(chooseBuddyShowcase(sleepingArgs)).toBeNull();
    expect(createBuddyShowcaseRun(sleepingArgs)).toBeNull();
    expect(chooseBuddyShowcase(cooldownArgs)).toBeNull();
    expect(createBuddyShowcaseRun(cooldownArgs)).toBeNull();
  });

  it("returns null when the required target is missing", () => {
    const args = {
      targets: [OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({ signal_type: "memory_extract" }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 40_000,
      lastShowcaseKind: null,
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("avoids immediate idle repeat unless a strong runtime trigger exists", () => {
    const idleArgs = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: null,
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 50_000,
      lastShowcaseKind: "memory_firefly_night" as const,
    };
    const strongArgs = {
      ...idleArgs,
      targets: [MEMORY_TARGET],
      nowPlaying: makeRuntimeEvent({ signal_type: "memory_extract" }),
    };

    expect(chooseBuddyShowcase(idleArgs)?.kind).toBe(
      "stargazing_constellation",
    );
    expect(chooseBuddyShowcase(strongArgs)?.kind).toBe("memory_firefly_night");
  });

  it("phase advancement reaches null after cooldown", () => {
    const run = createBuddyShowcaseRun({
      targets: [MEMORY_TARGET],
      nowPlaying: makeRuntimeEvent(),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 60_000,
      lastShowcaseKind: null,
    });
    expect(run).not.toBeNull();

    let current = run;
    let nowMs = 60_000;
    for (const phase of [
      "travel",
      "anticipate",
      "showcase",
      "react",
      "cooldown",
    ] as const) {
      expect(current?.phase).toBe(phase);
      nowMs += BUDDY_SHOWCASE_PHASE_DURATIONS_MS[phase];
      current = current
        ? advanceBuddyShowcasePhase({ run: current, nowMs })
        : null;
    }

    expect(current).toBeNull();
  });
});
