import type { BuddyScenePose } from "./types";
import type {
  BuddyWorldLayer,
  BuddyWorldPhase,
  BuddyWorldSeason,
  BuddyWorldWeather,
} from "./buddyWorldModel";

export type BuddyCompanionKind = "shiro" | "soot" | "kuro";

export type BuddyShiroMode =
  | "follow"
  | "mimic"
  | "wander"
  | "watch"
  | "nap_pile"
  | "hide";

export type BuddyCompanionPose =
  | "idle"
  | "sit"
  | "sleep"
  | "pounce"
  | "peek"
  | "perch"
  | "flee";

export interface BuddyWorldCompanion {
  id: string;
  kind: BuddyCompanionKind;
  fromX: number;
  fromY: number;
  toX: number;
  toY: number;
  moveStartMs: number;
  moveDurationMs: number;
  scale: number;
  facing: 1 | -1;
  pose: BuddyCompanionPose;
  seed: number;
}

export interface ShiroState {
  mode: BuddyShiroMode;
  fromX: number;
  fromY: number;
  toX: number;
  toY: number;
  moveStartMs: number;
  moveDurationMs: number;
  mimicPose: BuddyScenePose | null;
  watchSide: 1 | -1;
}

export interface ShiroContext {
  buddyX: number;
  buddyY: number;
  buddyPose: BuddyScenePose;
  longActionActive: boolean;
  sleeping: boolean;
  storm: boolean;
  nowMs: number;
  random?: () => number;
}

export const SHIRO_TICK_MS = 2_500;
export const SHIRO_SCALE = 0.38;
export const SHIRO_HIDE_SPOT = { x: 29, y: 77 } as const;

const SHIRO_MIN_X = 6;
const SHIRO_MAX_X = 94;
const SHIRO_MIN_Y = 70;
const SHIRO_MAX_Y = 86;
const UINT_MAX = 4_294_967_295;

function finiteOr(value: number | null | undefined, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function clampRange(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, finiteOr(value, min)));
}

export function companionSeedFromText(text: string): number {
  let hash = 2166136261;
  for (let index = 0; index < text.length; index += 1) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

export function companionSeededUnit(seed: number, salt: number): number {
  let value = (finiteOr(seed, 0) + Math.imul(salt + 1, 0x9e3779b9)) >>> 0;
  value ^= value >>> 16;
  value = Math.imul(value, 0x85ebca6b) >>> 0;
  value ^= value >>> 13;
  value = Math.imul(value, 0xc2b2ae35) >>> 0;
  value ^= value >>> 16;
  return (value >>> 0) / UINT_MAX;
}

export function companionPositionAt(
  companion: Pick<
    BuddyWorldCompanion,
    "fromX" | "fromY" | "toX" | "toY" | "moveStartMs" | "moveDurationMs"
  >,
  nowMs: number,
): { x: number; y: number } {
  const duration = Math.max(1, finiteOr(companion.moveDurationMs, 1));
  const rawProgress =
    (finiteOr(nowMs, 0) - finiteOr(companion.moveStartMs, 0)) / duration;
  const t = Math.max(0, Math.min(1, finiteOr(rawProgress, 1)));
  const eased = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
  const fromX = finiteOr(companion.fromX, 50);
  const fromY = finiteOr(companion.fromY, 80);
  return {
    x: fromX + (finiteOr(companion.toX, fromX) - fromX) * eased,
    y: fromY + (finiteOr(companion.toY, fromY) - fromY) * eased,
  };
}

export function createShiroState(
  nowMs: number,
  buddyX: number,
  buddyY: number,
): ShiroState {
  const startX = clampRange(buddyX - 9, SHIRO_MIN_X, SHIRO_MAX_X);
  const startY = clampRange(buddyY + 1, SHIRO_MIN_Y, SHIRO_MAX_Y);
  return {
    mode: "follow",
    fromX: startX,
    fromY: startY,
    toX: startX,
    toY: startY,
    moveStartMs: finiteOr(nowMs, 0),
    moveDurationMs: 1,
    mimicPose: null,
    watchSide: -1,
  };
}

function moveDurationFor(distance: number): number {
  return clampRange(Math.abs(distance) * 95, 800, 2_600);
}

export function stepShiroState(
  state: ShiroState,
  context: ShiroContext,
): ShiroState {
  const random = context.random ?? Math.random;
  const position = companionPositionAt(state, context.nowMs);
  const buddyX = clampRange(context.buddyX, SHIRO_MIN_X, SHIRO_MAX_X);
  const buddyY = clampRange(context.buddyY, SHIRO_MIN_Y, SHIRO_MAX_Y);

  let mode: BuddyShiroMode;
  if (context.storm) mode = "hide";
  else if (context.sleeping) mode = "nap_pile";
  else if (context.longActionActive) mode = "watch";
  else {
    const roll = random();
    mode = roll < 0.4 ? "follow" : roll < 0.65 ? "mimic" : "wander";
  }

  const watchSide: 1 | -1 =
    mode === "watch" && state.mode === "watch"
      ? state.watchSide
      : position.x <= buddyX
        ? -1
        : 1;

  let targetX = position.x;
  let targetY = position.y;
  switch (mode) {
    case "hide":
      targetX = SHIRO_HIDE_SPOT.x;
      targetY = SHIRO_HIDE_SPOT.y;
      break;
    case "nap_pile":
      targetX = buddyX + 5;
      targetY = buddyY + 1.5;
      break;
    case "watch":
      targetX = buddyX + watchSide * 7;
      targetY = buddyY + 1;
      break;
    case "follow":
      targetX = buddyX + (random() < 0.5 ? -1 : 1) * (5.5 + random() * 2.5);
      targetY = buddyY + random() * 2;
      break;
    case "mimic":
      targetX = buddyX + (position.x <= buddyX ? -6 : 6);
      targetY = buddyY + 1;
      break;
    case "wander":
      targetX = 12 + random() * 76;
      targetY = 74 + random() * 10;
      break;
  }

  targetX = clampRange(targetX, SHIRO_MIN_X, SHIRO_MAX_X);
  targetY = clampRange(targetY, SHIRO_MIN_Y, SHIRO_MAX_Y);

  return {
    mode,
    fromX: position.x,
    fromY: position.y,
    toX: targetX,
    toY: targetY,
    moveStartMs: context.nowMs,
    moveDurationMs: moveDurationFor(targetX - position.x),
    mimicPose: mode === "mimic" ? context.buddyPose : null,
    watchSide,
  };
}

function shiroPoseFor(state: ShiroState): BuddyCompanionPose {
  switch (state.mode) {
    case "nap_pile":
      return "sleep";
    case "hide":
      return "peek";
    case "watch":
      return "sit";
    case "mimic":
      switch (state.mimicPose) {
        case "sleepy":
          return "sleep";
        case "pounce":
        case "dig":
          return "pounce";
        case "look":
        case "stargaze":
        case "meditate":
          return "sit";
        default:
          return "idle";
      }
    default:
      return "idle";
  }
}

export function shiroCompanion(state: ShiroState): BuddyWorldCompanion {
  return {
    id: "shiro",
    kind: "shiro",
    fromX: state.fromX,
    fromY: state.fromY,
    toX: state.toX,
    toY: state.toY,
    moveStartMs: state.moveStartMs,
    moveDurationMs: state.moveDurationMs,
    scale: SHIRO_SCALE,
    facing: state.toX >= state.fromX ? 1 : -1,
    pose: shiroPoseFor(state),
    seed: 71,
  };
}

export interface SootColonyArgs {
  phase: BuddyWorldPhase;
  weather: BuddyWorldWeather;
  layers: readonly BuddyWorldLayer[];
  buddyX: number;
  dayKey: string;
}

const SOOT_EVENING_SPOTS = [
  { id: "soot-eaves", x: 12, y: 63.5 },
  { id: "soot-roots", x: 29.5, y: 80 },
  { id: "soot-fire", x: 61, y: 82.5 },
] as const;

export function sootCompanions(args: SootColonyArgs): BuddyWorldCompanion[] {
  if (args.weather === "rain" || args.weather === "storm") return [];
  const daySeed = companionSeedFromText(args.dayKey);

  if (args.phase === "evening" || args.phase === "night") {
    const hasCampfire = args.layers.includes("campfire");
    return SOOT_EVENING_SPOTS.map((spot, index) => {
      const x = spot.id === "soot-fire" && !hasCampfire ? spot.x + 9 : spot.x;
      const scattered = Math.abs(finiteOr(args.buddyX, 50) - x) < 7;
      const offset = scattered ? (x <= args.buddyX ? -9 : 9) : 0;
      return {
        id: spot.id,
        kind: "soot" as const,
        fromX: x,
        fromY: spot.y,
        toX: clampRange(x + offset, 4, 96),
        toY: spot.y + (scattered ? 1 : 0),
        moveStartMs: 0,
        moveDurationMs: 1,
        scale: 1,
        facing: offset >= 0 ? 1 : -1,
        pose: scattered ? "flee" : "idle",
        seed: daySeed + index * 17,
      } satisfies BuddyWorldCompanion;
    });
  }

  if (args.phase === "day" && companionSeededUnit(daySeed, 7) < 0.2) {
    return [
      {
        id: "soot-lurker",
        kind: "soot",
        fromX: 30,
        fromY: 79,
        toX: 30,
        toY: 79,
        moveStartMs: 0,
        moveDurationMs: 1,
        scale: 0.8,
        facing: 1,
        pose: "peek",
        seed: daySeed + 91,
      },
    ];
  }

  return [];
}

export type KuroMode = "away" | "perch" | "flee";

export interface KuroState {
  mode: KuroMode;
  sinceMs: number;
}

export const KURO_PERCH = { x: 31, y: 61.5 } as const;
const KURO_FLEE_MS = 1_400;

export function kuroDayActive(
  dayKey: string,
  season: BuddyWorldSeason,
): boolean {
  if (season !== "autumn") return false;
  return companionSeededUnit(companionSeedFromText(dayKey), 3) < 0.28;
}

export interface KuroStepArgs {
  active: boolean;
  gatherActive: boolean;
  buddyX: number;
  nowMs: number;
}

export function stepKuroState(
  state: KuroState,
  args: KuroStepArgs,
): { state: KuroState; fledNow: boolean } {
  if (!args.active) {
    return {
      state:
        state.mode === "away" ? state : { mode: "away", sinceMs: args.nowMs },
      fledNow: false,
    };
  }
  switch (state.mode) {
    case "away":
      if (args.gatherActive) {
        return {
          state: { mode: "perch", sinceMs: args.nowMs },
          fledNow: false,
        };
      }
      return { state, fledNow: false };
    case "perch":
      if (Math.abs(finiteOr(args.buddyX, 50) - KURO_PERCH.x) < 6) {
        return { state: { mode: "flee", sinceMs: args.nowMs }, fledNow: true };
      }
      if (!args.gatherActive) {
        return { state: { mode: "away", sinceMs: args.nowMs }, fledNow: false };
      }
      return { state, fledNow: false };
    case "flee":
      if (args.nowMs - state.sinceMs > KURO_FLEE_MS) {
        return { state: { mode: "away", sinceMs: args.nowMs }, fledNow: false };
      }
      return { state, fledNow: false };
  }
}

export function kuroCompanion(state: KuroState): BuddyWorldCompanion | null {
  if (state.mode === "away") return null;
  const fleeing = state.mode === "flee";
  return {
    id: "kuro",
    kind: "kuro",
    fromX: KURO_PERCH.x,
    fromY: KURO_PERCH.y,
    toX: fleeing ? -8 : KURO_PERCH.x,
    toY: fleeing ? KURO_PERCH.y - 14 : KURO_PERCH.y,
    moveStartMs: state.sinceMs,
    moveDurationMs: fleeing ? KURO_FLEE_MS : 1,
    scale: 1,
    facing: fleeing ? -1 : 1,
    pose: fleeing ? "flee" : "perch",
    seed: 137,
  };
}

export const SHIRO_INTRO_LINES: readonly ((name: string) => string)[] = [
  (name) =>
    `A rustle in the ferns… a tiny white shadow followed ${name} home. Meet Shiro.`,
  (name) =>
    `Something small and white has been copying ${name} all day. Shiro has moved in.`,
  (name) =>
    `${name} has a tiny apprentice now. Shiro waddles in from the ferns.`,
];

export const KURO_FLEE_LINES: readonly ((name: string) => string)[] = [
  () => "KURO. We TALKED about this.",
  (name) => `${name} watches Kuro escape with exactly one acorn. Audacity.`,
  () => "The crow tax strikes again. One acorn, gone.",
];
