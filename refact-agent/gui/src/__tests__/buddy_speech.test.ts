import { describe, expect, it } from "vitest";
import {
  BUDDY_SPEECH_RECENT_LIMIT,
  BUDDY_WORLD_SPEECH_PRIORITY,
  DIRECTOR_SPEECH_BEATS,
  DIRECTOR_SPEECH_POOLS,
  SHOWCASE_SPEECH_POOLS,
  careMidBeatAtMs,
  createBuddySpeechMemory,
  pickBuddySpeechLine,
  resolveBuddyWorldSpeech,
  styleForBuddySpeechIntent,
} from "../features/Buddy/buddySpeech";
import { BUDDY_CARE_ACTIVITY_DEFS } from "../features/Buddy/buddyWorldCareActivities";
import type { BuddyWorldSpeechCandidate } from "../features/Buddy/buddySpeech";

const NAME = "Mochi";

function candidate(text: string): BuddyWorldSpeechCandidate {
  return { text, style: "say" };
}

describe("buddy speech picker", () => {
  it("never repeats a line while fresh options remain", () => {
    const memory = createBuddySpeechMemory();
    const lines = [() => "a", () => "b", () => "c"] as const;
    const seen = new Set<string>();
    for (let i = 0; i < 3; i += 1) {
      seen.add(pickBuddySpeechLine(memory, "pool", lines, NAME, () => 0));
    }
    expect(seen).toEqual(new Set(["a", "b", "c"]));
  });

  it("falls back to the least recently used line when all are recent", () => {
    const memory = createBuddySpeechMemory();
    const lines = [() => "a", () => "b"] as const;
    expect(pickBuddySpeechLine(memory, "p", lines, NAME, () => 0)).toBe("a");
    expect(pickBuddySpeechLine(memory, "p", lines, NAME, () => 0)).toBe("b");
    expect(pickBuddySpeechLine(memory, "p", lines, NAME, () => 0)).toBe("a");
    expect(pickBuddySpeechLine(memory, "p", lines, NAME, () => 0)).toBe("b");
  });

  it("keeps the memory ring bounded", () => {
    const memory = createBuddySpeechMemory();
    const lines = Array.from(
      { length: 30 },
      (_, index) => () => `line-${index}`,
    );
    for (let i = 0; i < 30; i += 1) {
      pickBuddySpeechLine(memory, "big", lines, NAME, () => 0);
    }
    expect(memory.recent.length).toBeLessThanOrEqual(BUDDY_SPEECH_RECENT_LIMIT);
  });

  it("tracks pools independently by pool key", () => {
    const memory = createBuddySpeechMemory();
    const lines = [() => "x", () => "y"] as const;
    expect(pickBuddySpeechLine(memory, "one", lines, NAME, () => 0)).toBe("x");
    expect(pickBuddySpeechLine(memory, "two", lines, NAME, () => 0)).toBe("x");
    expect(memory.recent).toEqual(["one:0", "two:0"]);
  });

  it("is deterministic with an injected random source", () => {
    const first = pickBuddySpeechLine(
      createBuddySpeechMemory(),
      "p",
      [() => "a", () => "b", () => "c"],
      NAME,
      () => 0.7,
    );
    const second = pickBuddySpeechLine(
      createBuddySpeechMemory(),
      "p",
      [() => "a", () => "b", () => "c"],
      NAME,
      () => 0.7,
    );
    expect(first).toBe(second);
    expect(first).toBe("c");
  });

  it("templates the buddy name into name-aware lines", () => {
    const memory = createBuddySpeechMemory();
    const line = pickBuddySpeechLine(
      memory,
      "named",
      [(name) => `${name} waves.`],
      NAME,
      () => 0,
    );
    expect(line).toBe("Mochi waves.");
  });

  it("returns an empty string for an empty pool", () => {
    expect(
      pickBuddySpeechLine(createBuddySpeechMemory(), "none", [], NAME),
    ).toBe("");
  });
});

describe("buddy world speech resolver", () => {
  const empty = {
    backend: null,
    care: null,
    session: null,
    arc: null,
    showcase: null,
    director: null,
    reaction: null,
  };

  it("resolves the full ladder in priority order", () => {
    const all = {
      backend: candidate("backend"),
      care: candidate("care"),
      session: candidate("session"),
      arc: candidate("arc"),
      showcase: candidate("showcase"),
      director: candidate("director"),
      reaction: candidate("reaction"),
    };
    expect(resolveBuddyWorldSpeech(all).source).toBe("active");
    expect(resolveBuddyWorldSpeech({ ...all, backend: null }).source).toBe(
      "care",
    );
    expect(
      resolveBuddyWorldSpeech({ ...all, backend: null, care: null }).source,
    ).toBe("session");
    expect(
      resolveBuddyWorldSpeech({
        ...all,
        backend: null,
        care: null,
        session: null,
      }).source,
    ).toBe("arc");
    expect(
      resolveBuddyWorldSpeech({
        ...all,
        backend: null,
        care: null,
        session: null,
        arc: null,
      }).source,
    ).toBe("showcase");
    expect(
      resolveBuddyWorldSpeech({
        ...all,
        backend: null,
        care: null,
        session: null,
        arc: null,
        showcase: null,
      }).source,
    ).toBe("director");
    expect(
      resolveBuddyWorldSpeech({ ...empty, reaction: candidate("r") }).source,
    ).toBe("reaction");
  });

  it("returns none with a null text when every slot is empty", () => {
    const resolution = resolveBuddyWorldSpeech(empty);
    expect(resolution.source).toBe("none");
    expect(resolution.text).toBeNull();
    expect(resolution.style).toBe("say");
  });

  it("skips empty-text candidates", () => {
    const resolution = resolveBuddyWorldSpeech({
      ...empty,
      care: candidate(""),
      director: candidate("fallback"),
    });
    expect(resolution.source).toBe("director");
  });

  it("carries the winning candidate style", () => {
    const resolution = resolveBuddyWorldSpeech({
      ...empty,
      showcase: { text: "zzz", style: "think" },
    });
    expect(resolution.style).toBe("think");
  });

  it("documents the priority ladder attribute", () => {
    expect(BUDDY_WORLD_SPEECH_PRIORITY).toBe(
      "backend-care-session-arc-showcase-director-local",
    );
  });
});

describe("speech intent styles", () => {
  it("maps warning-like intents to alert", () => {
    expect(styleForBuddySpeechIntent("warning")).toBe("alert");
    expect(styleForBuddySpeechIntent("quota_alert")).toBe("alert");
    expect(styleForBuddySpeechIntent("Critical issue")).toBe("alert");
  });

  it("maps celebratory intents to excite", () => {
    expect(styleForBuddySpeechIntent("celebration")).toBe("excite");
    expect(styleForBuddySpeechIntent("success")).toBe("excite");
  });

  it("maps reflective intents to think", () => {
    expect(styleForBuddySpeechIntent("reflection")).toBe("think");
    expect(styleForBuddySpeechIntent("daydream")).toBe("think");
  });

  it("defaults to say", () => {
    expect(styleForBuddySpeechIntent(undefined)).toBe("say");
    expect(styleForBuddySpeechIntent("")).toBe("say");
    expect(styleForBuddySpeechIntent("suggestion")).toBe("say");
  });
});

describe("speech pools", () => {
  it("gives every director pool at least three variants", () => {
    for (const pool of Object.values(DIRECTOR_SPEECH_POOLS)) {
      expect(pool.lines.length).toBeGreaterThanOrEqual(3);
      for (const line of pool.lines) {
        expect(line(NAME).length).toBeGreaterThan(0);
      }
    }
  });

  it("gives every showcase pool at least three name-templated variants", () => {
    for (const pool of Object.values(SHOWCASE_SPEECH_POOLS)) {
      expect(pool.lines.length).toBeGreaterThanOrEqual(3);
      for (const line of pool.lines) {
        expect(line(NAME)).toContain(NAME);
      }
    }
  });

  it("keeps director beat tables sorted with positive offsets", () => {
    for (const beats of Object.values(DIRECTOR_SPEECH_BEATS)) {
      expect(beats.length).toBeGreaterThan(0);
      let previousAtMs = 0;
      for (const beat of beats) {
        expect(beat.atMs).toBeGreaterThan(previousAtMs);
        previousAtMs = beat.atMs;
        expect(beat.lines.length).toBeGreaterThan(0);
        expect(beat.poolKey.length).toBeGreaterThan(0);
      }
    }
  });

  it("places the care mid beat inside the perform window", () => {
    for (const def of Object.values(BUDDY_CARE_ACTIVITY_DEFS)) {
      const midAtMs = careMidBeatAtMs(3_800, def.performMs);
      expect(midAtMs).toBeGreaterThan(3_800);
      expect(midAtMs).toBeLessThan(3_800 + def.performMs);
    }
  });
});
