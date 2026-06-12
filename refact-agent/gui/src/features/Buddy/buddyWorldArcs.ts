import type { BuddyScenePose } from "./types";
import type { BuddySpeechBeat } from "./buddySpeech";
import type { BuddyWorldPhase, BuddyWorldWeather } from "./buddyWorldModel";
import type { BuddyWorldMemos } from "./buddyWorldMemos";

export type BuddyWorldArcKind =
  | "morning_ritual"
  | "evening_lanterns"
  | "storm_story"
  | "first_snowflake"
  | "first_petal"
  | "first_firefly"
  | "first_red_leaf";

export interface BuddyArcStepDef {
  id: string;
  targetX: number;
  targetY: number;
  depthScale: number;
  pose: BuddyScenePose;
  durationMs: number;
  accentIntent: string | null;
  lanternLitCount?: number;
  beats: readonly BuddySpeechBeat[];
}

export interface BuddyWorldArcDef {
  kind: BuddyWorldArcKind;
  oncePerDay: boolean;
  steps: readonly BuddyArcStepDef[];
  finale?: { clear: BuddyArcStepDef; stormy: BuddyArcStepDef };
}

export interface BuddyArcRun {
  id: string;
  kind: BuddyWorldArcKind;
  stepIndex: number;
  finale: "clear" | "stormy" | null;
  startedAtMs: number;
  stepStartedAtMs: number;
}

export interface ChooseBuddyWorldArcArgs {
  previousPhase: BuddyWorldPhase | null;
  phase: BuddyWorldPhase;
  previousWeather: BuddyWorldWeather | null;
  weather: BuddyWorldWeather;
  layers: readonly string[];
  memos: BuddyWorldMemos;
  dayKey: string;
  year: number;
  busy: boolean;
}

const SEASON_FIRST_LAYERS = [
  ["first_snowflake", "season_snow"],
  ["first_petal", "season_petals"],
  ["first_firefly", "fireflies"],
  ["first_red_leaf", "season_leaves"],
] as const satisfies readonly (readonly [BuddyWorldArcKind, string])[];

export function seasonFirstMemoKey(
  kind: BuddyWorldArcKind,
  year: number,
): string {
  return `${kind}:${year}`;
}

export const BUDDY_WORLD_ARC_DEFS: Record<BuddyWorldArcKind, BuddyWorldArcDef> =
  {
    morning_ritual: {
      kind: "morning_ritual",
      oncePerDay: true,
      steps: [
        {
          id: "wake_stretch",
          targetX: 50,
          targetY: 76,
          depthScale: 1,
          pose: "bounce",
          durationMs: 5_200,
          accentIntent: null,
          beats: [
            {
              atMs: 400,
              style: "say",
              poolKey: "arc:morning:stretch",
              lines: [
                (name) =>
                  `${name} greets the morning with a full-body stretch.`,
                (name) => `${name} yawns so wide the meadow yawns back.`,
                (name) => `${name} boots up: paws, ears, appetite.`,
              ],
            },
          ],
        },
        {
          id: "water_garden",
          targetX: 41,
          targetY: 79,
          depthScale: 1,
          pose: "dig",
          durationMs: 7_200,
          accentIntent: "tend_garden",
          beats: [
            {
              atMs: 900,
              style: "say",
              poolKey: "arc:morning:water",
              lines: [
                (name) => `${name} waters the task sprouts, row by row.`,
                (name) =>
                  `${name} gives each sprout exactly one good-luck drop.`,
                (name) => `${name} hums while the garden drinks.`,
              ],
            },
          ],
        },
        {
          id: "check_mailbox",
          targetX: 35,
          targetY: 76,
          depthScale: 0.94,
          pose: "look",
          durationMs: 6_200,
          accentIntent: "check_mailbox",
          beats: [
            {
              atMs: 1_000,
              style: "say",
              poolKey: "arc:morning:mailbox",
              lines: [
                (name) => `${name} peeks into the mailbox. Routine patrol.`,
                (name) => `${name} checks for morning mail and stray acorns.`,
                (name) => `${name} taps the mailbox twice for luck.`,
              ],
            },
          ],
        },
        {
          id: "sun_salute",
          targetX: 47,
          targetY: 80,
          depthScale: 1.02,
          pose: "meditate",
          durationMs: 6_400,
          accentIntent: null,
          beats: [
            {
              atMs: 800,
              style: "sing",
              poolKey: "arc:morning:salute",
              lines: [
                () => "♪ good morning, sun. good morning, moss. ♪",
                () => "♪ small day, soft start ♪",
                () => "♪ hello light, hello leaves ♪",
              ],
            },
          ],
        },
      ],
    },
    evening_lanterns: {
      kind: "evening_lanterns",
      oncePerDay: true,
      steps: [
        {
          id: "lantern_one",
          targetX: 36,
          targetY: 79,
          depthScale: 0.98,
          pose: "look",
          durationMs: 5_600,
          accentIntent: null,
          lanternLitCount: 1,
          beats: [
            {
              atMs: 900,
              style: "say",
              poolKey: "arc:lanterns:one",
              lines: [
                (name) => `${name} lights the first lantern. Dusk approves.`,
                (name) => `One little flame up. ${name} nods at it.`,
                (name) => `${name} wakes the first lantern gently.`,
              ],
            },
          ],
        },
        {
          id: "lantern_two",
          targetX: 43,
          targetY: 81,
          depthScale: 1.02,
          pose: "look",
          durationMs: 5_600,
          accentIntent: null,
          lanternLitCount: 2,
          beats: [
            {
              atMs: 900,
              style: "whisper",
              poolKey: "arc:lanterns:two",
              lines: [
                () => "two… the path is warming up.",
                () => "second flame. steady now.",
                () => "halfway to a glowing road.",
              ],
            },
          ],
        },
        {
          id: "lantern_three",
          targetX: 50,
          targetY: 83,
          depthScale: 1.04,
          pose: "look",
          durationMs: 5_600,
          accentIntent: null,
          lanternLitCount: 3,
          beats: [
            {
              atMs: 900,
              style: "say",
              poolKey: "arc:lanterns:three",
              lines: [
                (name) =>
                  `${name} lights the last lantern. The path glows home.`,
                (name) => `All three lit. ${name} admires the little runway.`,
                (name) => `Third flame up. ${name} declares the evening open.`,
              ],
            },
          ],
        },
        {
          id: "campfire_rest",
          targetX: 58,
          targetY: 81,
          depthScale: 1.05,
          pose: "meditate",
          durationMs: 8_200,
          accentIntent: "warm_by_fire",
          lanternLitCount: 3,
          beats: [
            {
              atMs: 1_200,
              style: "think",
              poolKey: "arc:lanterns:rest",
              lines: [
                () => "Busy day. Good day.",
                () => "The fire crackles. The day settles.",
                () => "Lanterns lit, thoughts tucked in.",
              ],
            },
          ],
        },
      ],
    },
    storm_story: {
      kind: "storm_story",
      oncePerDay: true,
      steps: [
        {
          id: "storm_notice",
          targetX: 50,
          targetY: 76,
          depthScale: 1,
          pose: "look",
          durationMs: 4_600,
          accentIntent: null,
          beats: [
            {
              atMs: 200,
              style: "alert",
              poolKey: "arc:storm:notice",
              lines: [
                (name) => `${name} sniffs the wind. Storm incoming!`,
                (name) => `${name}'s ears flatten. Thunder on the way.`,
                (name) => `Storm front spotted. ${name} springs into action.`,
              ],
            },
          ],
        },
        {
          id: "secure_mailbox",
          targetX: 35,
          targetY: 76,
          depthScale: 0.94,
          pose: "carry",
          durationMs: 6_000,
          accentIntent: null,
          beats: [
            {
              atMs: 1_000,
              style: "whisper",
              poolKey: "arc:storm:secure",
              lines: [
                () => "flag down, lid tight. mail is safe.",
                () => "battening down the mailbox hatches.",
                () => "no letter left behind.",
              ],
            },
          ],
        },
        {
          id: "shelter",
          targetX: 33,
          targetY: 76,
          depthScale: 0.96,
          pose: "look",
          durationMs: 20_000,
          accentIntent: null,
          beats: [
            {
              atMs: 2_200,
              style: "whisper",
              poolKey: "arc:storm:shelter",
              lines: [
                () => "counting raindrops from under the awning…",
                () => "the sky is doing its big drum solo.",
                () => "dry paws, loud sky.",
              ],
            },
            {
              atMs: 11_500,
              style: "whisper",
              poolKey: "arc:storm:shelter_late",
              lines: [
                () => "…still rumbling. cocoa-weather, honestly.",
                () => "…the thunder is losing its voice.",
                () => "…one one-thousand, two one-thousand…",
              ],
            },
          ],
        },
      ],
      finale: {
        clear: {
          id: "rainbow_walk",
          targetX: 47,
          targetY: 80,
          depthScale: 1.02,
          pose: "dance",
          durationMs: 7_000,
          accentIntent: null,
          beats: [
            {
              atMs: 600,
              style: "excite",
              poolKey: "arc:storm:rainbow",
              lines: [
                (name) => `${name} struts out under the fresh rainbow!`,
                (name) => `Storm over! ${name} claims the puddle kingdom.`,
                (name) => `${name} takes a victory lap through clean air.`,
              ],
            },
          ],
        },
        stormy: {
          id: "quiet_end",
          targetX: 50,
          targetY: 76,
          depthScale: 1,
          pose: "look",
          durationMs: 5_000,
          accentIntent: null,
          beats: [
            {
              atMs: 700,
              style: "say",
              poolKey: "arc:storm:quiet",
              lines: [
                (name) => `${name} peeks out. Still grumbly up there.`,
                (name) => `${name} files the storm under "ongoing".`,
                (name) => `The sky isn't done. ${name} is patient.`,
              ],
            },
          ],
        },
      },
    },
    first_snowflake: {
      kind: "first_snowflake",
      oncePerDay: false,
      steps: [
        {
          id: "snowflake_chase",
          targetX: 47,
          targetY: 80,
          depthScale: 1.02,
          pose: "pounce",
          durationMs: 8_400,
          accentIntent: null,
          beats: [
            {
              atMs: 500,
              style: "excite",
              poolKey: "arc:first:snowflake",
              lines: [
                (name) => `FIRST SNOWFLAKE! ${name} must catch it!`,
                (name) => `${name} chases winter's very first flake.`,
                (name) => `Snow! The first one! ${name} is on it.`,
              ],
            },
            {
              atMs: 5_400,
              style: "whisper",
              poolKey: "arc:first:snowflake_end",
              lines: [
                () => "caught it on my nose. it's gone. worth it.",
                () => "cold little star. hello, winter.",
              ],
            },
          ],
        },
      ],
    },
    first_petal: {
      kind: "first_petal",
      oncePerDay: false,
      steps: [
        {
          id: "petal_catch",
          targetX: 47,
          targetY: 80,
          depthScale: 1.02,
          pose: "bounce",
          durationMs: 8_400,
          accentIntent: null,
          beats: [
            {
              atMs: 500,
              style: "excite",
              poolKey: "arc:first:petal",
              lines: [
                (name) => `First petal of spring! ${name} leaps for it!`,
                (name) => `${name} spots spring's opening petal. Catch it!`,
                (name) => `A petal! The season's first! ${name} jumps!`,
              ],
            },
            {
              atMs: 5_400,
              style: "whisper",
              poolKey: "arc:first:petal_end",
              lines: [
                () => "got it. soft as a hello.",
                () => "spring is officially open.",
              ],
            },
          ],
        },
      ],
    },
    first_firefly: {
      kind: "first_firefly",
      oncePerDay: false,
      steps: [
        {
          id: "firefly_follow",
          targetX: 47,
          targetY: 80,
          depthScale: 1.02,
          pose: "look",
          durationMs: 8_400,
          accentIntent: null,
          beats: [
            {
              atMs: 500,
              style: "excite",
              poolKey: "arc:first:firefly",
              lines: [
                (name) =>
                  `The summer's first firefly! ${name} follows the glow.`,
                (name) => `${name} trails the season's first little lantern.`,
                (name) => `First firefly spotted! ${name} tiptoes after it.`,
              ],
            },
            {
              atMs: 5_400,
              style: "whisper",
              poolKey: "arc:first:firefly_end",
              lines: [
                () => "it blinked goodnight. summer is here.",
                () => "tiny light, big season.",
              ],
            },
          ],
        },
      ],
    },
    first_red_leaf: {
      kind: "first_red_leaf",
      oncePerDay: false,
      steps: [
        {
          id: "red_leaf",
          targetX: 34,
          targetY: 78,
          depthScale: 0.98,
          pose: "look",
          durationMs: 8_400,
          accentIntent: null,
          beats: [
            {
              atMs: 500,
              style: "excite",
              poolKey: "arc:first:red_leaf",
              lines: [
                (name) => `The first red leaf! ${name} salutes autumn.`,
                (name) => `${name} catches autumn's opening leaf mid-spin.`,
                (name) => `One red leaf down. ${name} declares sweater season.`,
              ],
            },
            {
              atMs: 5_400,
              style: "whisper",
              poolKey: "arc:first:red_leaf_end",
              lines: [
                () => "crunchy. ceremonial. perfect.",
                () => "autumn signed its name in red.",
              ],
            },
          ],
        },
      ],
    },
  };

export function chooseBuddyWorldArc(
  args: ChooseBuddyWorldArcArgs,
): BuddyWorldArcKind | null {
  if (args.busy) return null;

  const stormArrived =
    args.previousWeather !== null &&
    args.previousWeather !== "storm" &&
    args.weather === "storm";
  if (stormArrived && args.memos.lastArcDates.storm_story !== args.dayKey) {
    return "storm_story";
  }

  if (args.weather === "storm") return null;

  const phaseChangedTo = (phase: BuddyWorldPhase): boolean =>
    args.previousPhase !== null &&
    args.previousPhase !== phase &&
    args.phase === phase;

  if (
    phaseChangedTo("morning") &&
    args.memos.lastArcDates.morning_ritual !== args.dayKey
  ) {
    return "morning_ritual";
  }
  if (
    phaseChangedTo("evening") &&
    args.memos.lastArcDates.evening_lanterns !== args.dayKey
  ) {
    return "evening_lanterns";
  }

  for (const [kind, layer] of SEASON_FIRST_LAYERS) {
    if (
      args.layers.includes(layer) &&
      !args.memos.seasonFirstsSeen.includes(seasonFirstMemoKey(kind, args.year))
    ) {
      return kind;
    }
  }

  return null;
}

export function createBuddyArcRun(
  kind: BuddyWorldArcKind,
  nowMs: number,
): BuddyArcRun {
  const safeNowMs = Number.isFinite(nowMs) ? nowMs : 0;
  return {
    id: `arc-${kind}-${Math.max(0, Math.floor(safeNowMs / 1000)).toString(36)}`,
    kind,
    stepIndex: 0,
    finale: null,
    startedAtMs: safeNowMs,
    stepStartedAtMs: safeNowMs,
  };
}

export function currentBuddyArcStep(run: BuddyArcRun): BuddyArcStepDef | null {
  const def = BUDDY_WORLD_ARC_DEFS[run.kind];
  if (run.finale !== null && def.finale) {
    return run.stepIndex >= def.steps.length ? def.finale[run.finale] : null;
  }
  return def.steps[run.stepIndex] ?? null;
}

export function advanceBuddyArcRun(
  run: BuddyArcRun,
  nowMs: number,
  weather: BuddyWorldWeather,
): BuddyArcRun | null {
  const step = currentBuddyArcStep(run);
  if (!step) return null;
  if (nowMs - run.stepStartedAtMs < step.durationMs) return run;

  const def = BUDDY_WORLD_ARC_DEFS[run.kind];
  if (run.finale !== null) return null;
  if (run.stepIndex < def.steps.length - 1) {
    return {
      ...run,
      stepIndex: run.stepIndex + 1,
      stepStartedAtMs: nowMs,
    };
  }
  if (def.finale) {
    return {
      ...run,
      stepIndex: def.steps.length,
      finale: weather === "storm" || weather === "rain" ? "stormy" : "clear",
      stepStartedAtMs: nowMs,
    };
  }
  return null;
}

export function buddyArcLanternLitCount(
  run: BuddyArcRun | null,
): number | null {
  if (!run) return null;
  const step = currentBuddyArcStep(run);
  return step?.lanternLitCount ?? null;
}
