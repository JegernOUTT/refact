import type { BuddyPetState, BuddyScenePose } from "./types";
import type { BuddySpeechStyle } from "./types";

export type BuddyPlaySessionKind = "fetch" | "firefly";

export type BuddyFetchPhase =
  | "armed"
  | "throwing"
  | "chasing"
  | "carrying"
  | "wiggling"
  | "done";

export interface BuddyFetchSession {
  kind: "fetch";
  phase: BuddyFetchPhase;
  throwCount: number;
  ballFromX: number;
  ballFromY: number;
  ballToX: number;
  ballToY: number;
  phaseStartedAtMs: number;
  armedAtMs: number;
  seed: number;
}

export type BuddyFireflyPhase = "armed" | "pouncing" | "resolved" | "done";

export interface BuddyFireflySession {
  kind: "firefly";
  phase: BuddyFireflyPhase;
  catches: number;
  caughtLast: boolean;
  targetX: number;
  targetY: number;
  phaseStartedAtMs: number;
  armedAtMs: number;
  seed: number;
}

export type BuddyPlaySession = BuddyFetchSession | BuddyFireflySession;

export const FETCH_MAX_THROWS = 3;
export const FIREFLY_MAX_CATCHES = 3;
export const PLAY_ARMED_TIMEOUT_MS = 45_000;

export const FETCH_PHASE_DURATIONS_MS: Record<BuddyFetchPhase, number> = {
  armed: PLAY_ARMED_TIMEOUT_MS,
  throwing: 950,
  chasing: 2_300,
  carrying: 2_700,
  wiggling: 1_900,
  done: 2_600,
};

export const FIREFLY_PHASE_DURATIONS_MS: Record<BuddyFireflyPhase, number> = {
  armed: PLAY_ARMED_TIMEOUT_MS,
  pouncing: 1_500,
  resolved: 1_600,
  done: 2_600,
};

const PLAY_MIN_X = 33;
const PLAY_MAX_X = 67;
const PLAY_MIN_Y = 70;
const PLAY_MAX_Y = 84;
const UINT_MAX = 4_294_967_295;

function finiteOr(value: number | null | undefined, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function clampRange(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, finiteOr(value, min)));
}

export function playSeededUnit(seed: number, salt: number): number {
  let value = (finiteOr(seed, 0) + Math.imul(salt + 1, 0x9e3779b9)) >>> 0;
  value ^= value >>> 16;
  value = Math.imul(value, 0x85ebca6b) >>> 0;
  value ^= value >>> 13;
  value = Math.imul(value, 0xc2b2ae35) >>> 0;
  value ^= value >>> 16;
  return (value >>> 0) / UINT_MAX;
}

export function createFetchSession(nowMs: number): BuddyFetchSession {
  const safeNow = finiteOr(nowMs, 0);
  return {
    kind: "fetch",
    phase: "armed",
    throwCount: 0,
    ballFromX: 50,
    ballFromY: 78,
    ballToX: 50,
    ballToY: 78,
    phaseStartedAtMs: safeNow,
    armedAtMs: safeNow,
    seed: Math.max(1, Math.floor(safeNow / 1000) >>> 0),
  };
}

export function throwFetchBall(
  session: BuddyFetchSession,
  targetX: number,
  targetY: number,
  buddyX: number,
  nowMs: number,
): BuddyFetchSession {
  if (session.phase !== "armed") return session;
  return {
    ...session,
    phase: "throwing",
    ballFromX: clampRange(buddyX, PLAY_MIN_X, PLAY_MAX_X),
    ballFromY: 74,
    ballToX: clampRange(targetX, PLAY_MIN_X, PLAY_MAX_X),
    ballToY: clampRange(targetY, PLAY_MIN_Y, PLAY_MAX_Y),
    phaseStartedAtMs: finiteOr(nowMs, 0),
  };
}

export function createFireflySession(nowMs: number): BuddyFireflySession {
  const safeNow = finiteOr(nowMs, 0);
  return {
    kind: "firefly",
    phase: "armed",
    catches: 0,
    caughtLast: false,
    targetX: 50,
    targetY: 80,
    phaseStartedAtMs: safeNow,
    armedAtMs: safeNow,
    seed: Math.max(1, Math.floor(safeNow / 1000) >>> 0),
  };
}

export function pounceFirefly(
  session: BuddyFireflySession,
  targetX: number,
  targetY: number,
  nowMs: number,
): BuddyFireflySession {
  if (session.phase !== "armed") return session;
  return {
    ...session,
    phase: "pouncing",
    targetX: clampRange(targetX, PLAY_MIN_X, PLAY_MAX_X),
    targetY: clampRange(targetY, PLAY_MIN_Y, PLAY_MAX_Y),
    phaseStartedAtMs: finiteOr(nowMs, 0),
  };
}

export function advanceBuddyPlaySession(
  session: BuddyPlaySession,
  nowMs: number,
): BuddyPlaySession | null {
  const safeNow = finiteOr(nowMs, 0);
  if (session.kind === "fetch") {
    const duration = FETCH_PHASE_DURATIONS_MS[session.phase];
    if (safeNow - session.phaseStartedAtMs < duration) return session;
    switch (session.phase) {
      case "armed":
        return null;
      case "throwing":
        return { ...session, phase: "chasing", phaseStartedAtMs: safeNow };
      case "chasing":
        return { ...session, phase: "carrying", phaseStartedAtMs: safeNow };
      case "carrying":
        return {
          ...session,
          phase: "wiggling",
          throwCount: session.throwCount + 1,
          phaseStartedAtMs: safeNow,
        };
      case "wiggling":
        if (session.throwCount >= FETCH_MAX_THROWS) {
          return { ...session, phase: "done", phaseStartedAtMs: safeNow };
        }
        return {
          ...session,
          phase: "armed",
          phaseStartedAtMs: safeNow,
          armedAtMs: safeNow,
        };
      case "done":
        return null;
    }
  }
  const duration = FIREFLY_PHASE_DURATIONS_MS[session.phase];
  if (safeNow - session.phaseStartedAtMs < duration) return session;
  switch (session.phase) {
    case "armed":
      return null;
    case "pouncing": {
      const caught =
        playSeededUnit(session.seed, session.catches * 7 + 1) < 0.62;
      return {
        ...session,
        phase: "resolved",
        caughtLast: caught,
        catches: session.catches + (caught ? 1 : 0),
        phaseStartedAtMs: safeNow,
      };
    }
    case "resolved":
      if (session.catches >= FIREFLY_MAX_CATCHES) {
        return { ...session, phase: "done", phaseStartedAtMs: safeNow };
      }
      return {
        ...session,
        phase: "armed",
        phaseStartedAtMs: safeNow,
        armedAtMs: safeNow,
      };
    case "done":
      return null;
  }
}

export interface BuddyPlayBodyTarget {
  x: number;
  y: number;
  pose: BuddyScenePose;
}

/* ------------------------------------------------------------------ */
/* B35: interruption / resume, real completion state, keyboard targets. */
/* Additive only — no existing phase timing or drawing behavior changes. */
/* ------------------------------------------------------------------ */

export interface BuddyPlaySuspension {
  session: BuddyPlaySession;
  /** Elapsed time inside the current phase, frozen at suspend time. */
  phaseElapsedMs: number;
  /** Elapsed time since the phase was armed, frozen at suspend time. */
  armedElapsedMs: number;
  suspendedAtMs: number;
  reason: BuddyPlayInterruptReason;
}

export type BuddyPlayInterruptReason =
  | "user_paused"
  | "hidden"
  | "unmounted"
  | "work_took_over";

/**
 * Freeze a running session. The session is preserved exactly; only the clock is
 * captured, so resuming never skips or replays a phase.
 */
export function suspendPlaySession(
  session: BuddyPlaySession,
  nowMs: number,
  reason: BuddyPlayInterruptReason,
): BuddyPlaySuspension {
  const safeNow = finiteOr(nowMs, 0);
  return {
    session,
    phaseElapsedMs: Math.max(0, safeNow - session.phaseStartedAtMs),
    armedElapsedMs: Math.max(0, safeNow - session.armedAtMs),
    suspendedAtMs: safeNow,
    reason,
  };
}

/** Restore a suspended session so the current phase continues where it stopped. */
export function resumePlaySession(
  suspension: BuddyPlaySuspension,
  nowMs: number,
): BuddyPlaySession {
  const safeNow = finiteOr(nowMs, 0);
  return {
    ...suspension.session,
    phaseStartedAtMs: safeNow - suspension.phaseElapsedMs,
    armedAtMs: safeNow - suspension.armedElapsedMs,
  };
}

export type BuddyPlayOutcome = "completed" | "interrupted" | "abandoned";

export interface BuddyPlayCompletion {
  kind: BuddyPlaySessionKind;
  outcome: BuddyPlayOutcome;
  /** Successful rounds actually played; never inflated by interruption. */
  rounds: number;
  /** Rounds the game asks for before it reports completion. */
  roundsTarget: number;
  /** Always false: play never grants rights or changes project work. */
  affectsWork: false;
  label: string;
}

function playRounds(session: BuddyPlaySession): number {
  return session.kind === "fetch" ? session.throwCount : session.catches;
}

function playRoundsTarget(kind: BuddyPlaySessionKind): number {
  return kind === "fetch" ? FETCH_MAX_THROWS : FIREFLY_MAX_CATCHES;
}

/**
 * Real completion state for a play session. `interrupted` means the user or the
 * host stopped a session that had progress; `abandoned` means it never started.
 */
export function playSessionCompletion(
  session: BuddyPlaySession,
  ended: "finished" | "interrupted",
): BuddyPlayCompletion {
  const rounds = playRounds(session);
  const roundsTarget = playRoundsTarget(session.kind);
  const completed = ended === "finished" && session.phase === "done";
  const outcome: BuddyPlayOutcome = completed
    ? "completed"
    : rounds > 0
      ? "interrupted"
      : "abandoned";
  const noun = session.kind === "fetch" ? "throws" : "catches";
  const label = completed
    ? `Play finished: ${rounds} of ${roundsTarget} ${noun}.`
    : outcome === "interrupted"
      ? `Play stopped early after ${rounds} of ${roundsTarget} ${noun}.`
      : "Play stopped before it started.";
  return {
    kind: session.kind,
    outcome,
    rounds,
    roundsTarget,
    affectsWork: false,
    label,
  };
}

export interface BuddyPlayKeyboardTarget {
  id: string;
  /** Single-key shortcut, also usable as a visible DOM button. */
  key: string;
  label: string;
  x: number;
  y: number;
}

const FETCH_KEYBOARD_TARGETS: readonly BuddyPlayKeyboardTarget[] = [
  { id: "left", key: "1", label: "Throw left", x: PLAY_MIN_X + 3, y: 78 },
  { id: "centre", key: "2", label: "Throw ahead", x: 50, y: PLAY_MAX_Y - 2 },
  { id: "right", key: "3", label: "Throw right", x: PLAY_MAX_X - 3, y: 78 },
];

const FIREFLY_KEYBOARD_TARGETS: readonly BuddyPlayKeyboardTarget[] = [
  {
    id: "low",
    key: "1",
    label: "Pounce on the low glow",
    x: 40,
    y: PLAY_MAX_Y - 1,
  },
  { id: "mid", key: "2", label: "Pounce on the middle glow", x: 50, y: 77 },
  {
    id: "high",
    key: "3",
    label: "Pounce on the high glow",
    x: 60,
    y: PLAY_MIN_Y + 1,
  },
];

/**
 * Discrete named targets so every pointer action in play has a keyboard and DOM
 * equivalent. Coordinates stay inside the same play bounds as pointer input.
 */
export function playKeyboardTargets(
  kind: BuddyPlaySessionKind,
): readonly BuddyPlayKeyboardTarget[] {
  return kind === "fetch" ? FETCH_KEYBOARD_TARGETS : FIREFLY_KEYBOARD_TARGETS;
}

export function playKeyboardTargetForKey(
  kind: BuddyPlaySessionKind,
  key: string,
): BuddyPlayKeyboardTarget | null {
  return playKeyboardTargets(kind).find((target) => target.key === key) ?? null;
}

/** Screen-reader sentence for the current session; states facts only. */
export function describePlaySessionForScreenReader(
  session: BuddyPlaySession | null,
  suspended: boolean,
): string {
  if (!session) return "No play session is running.";
  const rounds = playRounds(session);
  const target = playRoundsTarget(session.kind);
  const game = session.kind === "fetch" ? "Fetch" : "Firefly";
  if (suspended) {
    return `${game} paused at ${session.phase}, ${rounds} of ${target} rounds. Resume to continue.`;
  }
  if (session.phase === "armed") {
    return `${game} ready. Choose a target. ${rounds} of ${target} rounds done.`;
  }
  if (session.phase === "done") {
    return `${game} finished with ${rounds} of ${target} rounds.`;
  }
  return `${game} in progress: ${session.phase}. ${rounds} of ${target} rounds done.`;
}

export function playSessionBodyTarget(
  session: BuddyPlaySession | null,
): BuddyPlayBodyTarget | null {
  if (!session) return null;
  if (session.kind === "fetch") {
    switch (session.phase) {
      case "chasing":
        return { x: session.ballToX, y: session.ballToY, pose: "pounce" };
      case "carrying":
        return { x: 50, y: 78, pose: "carry" };
      case "wiggling":
        return { x: 50, y: 78, pose: "bounce" };
      case "done":
        return { x: 50, y: 78, pose: "sleepy" };
      case "armed":
      case "throwing":
        return null;
    }
  }
  switch (session.phase) {
    case "pouncing":
      return { x: session.targetX, y: session.targetY, pose: "pounce" };
    case "resolved":
      return {
        x: session.targetX,
        y: session.targetY,
        pose: session.caughtLast ? "cheer" : "look",
      };
    case "done":
      return { x: 50, y: 78, pose: "dance" };
    case "armed":
      return null;
  }
}

export interface BuddyPlayBallState {
  x: number;
  y: number;
  airborne: boolean;
}

export function fetchBallPositionAt(
  session: BuddyFetchSession,
  nowMs: number,
): BuddyPlayBallState | null {
  const safeNow = finiteOr(nowMs, 0);
  if (session.phase === "throwing") {
    const duration = Math.max(1, FETCH_PHASE_DURATIONS_MS.throwing);
    const t = clampRange((safeNow - session.phaseStartedAtMs) / duration, 0, 1);
    const x = session.ballFromX + (session.ballToX - session.ballFromX) * t;
    const baseY = session.ballFromY + (session.ballToY - session.ballFromY) * t;
    const arcLift = Math.sin(t * Math.PI) * 9;
    return { x, y: baseY - arcLift, airborne: true };
  }
  if (session.phase === "chasing") {
    return { x: session.ballToX, y: session.ballToY, airborne: false };
  }
  if (session.phase === "wiggling") {
    return { x: 52.5, y: 79, airborne: false };
  }
  return null;
}

export interface BuddyGiftMoment {
  item: "acorn" | "fish" | "leaf" | "flower" | "spark";
  startedAtMs: number;
}

export const GIFT_MOMENT_MS = 6_000;
export const GIFT_CHANCE = 0.35;

const GIFT_BY_INTENT: Partial<Record<string, BuddyGiftMoment["item"]>> = {
  fish_at_pond: "fish",
  gather_acorns: "acorn",
  collect_leaves: "leaf",
  smell_flowers: "flower",
  catch_fireflies: "spark",
};

export function maybeCreateGiftMoment(
  intentKind: string | null,
  seed: number,
  nowMs: number,
): BuddyGiftMoment | null {
  if (!intentKind) return null;
  const item = GIFT_BY_INTENT[intentKind];
  if (!item) return null;
  if (playSeededUnit(seed, 13) >= GIFT_CHANCE) return null;
  return { item, startedAtMs: finiteOr(nowMs, 0) };
}

export interface BuddyRequestArgs {
  pet: BuddyPetState | undefined;
  nowMs: number;
  lastOfferAtMs: number;
  offersThisSession: number;
  busy: boolean;
}

export const REQUEST_COOLDOWN_MS = 10 * 60_000;
export const REQUEST_MAX_PER_SESSION = 2;

export function shouldOfferBuddyRequest(args: BuddyRequestArgs): boolean {
  if (args.busy) return false;
  if (!args.pet) return false;
  if (args.offersThisSession >= REQUEST_MAX_PER_SESSION) return false;
  if (args.nowMs - args.lastOfferAtMs < REQUEST_COOLDOWN_MS) return false;
  return args.pet.needs.boredom > 60 || args.pet.needs.affection < 40;
}

export interface BuddyPlayLine {
  style: BuddySpeechStyle;
  poolKey: string;
  lines: readonly ((name: string) => string)[];
}

export const PLAY_SESSION_LINES: Record<string, BuddyPlayLine> = {
  "fetch:armed": {
    style: "excite",
    poolKey: "play:fetch:armed",
    lines: [
      () => "Throw it anywhere! I'm READY.",
      () => "Ball! Ball ball ball. Click the meadow!",
      () => "Wind-up accepted. Pick a spot!",
    ],
  },
  "fetch:throwing": {
    style: "say",
    poolKey: "play:fetch:throwing",
    lines: [
      () => "Eyes on the ball—",
      () => "Incoming arc detected—",
      () => "Trajectory locked—",
    ],
  },
  "fetch:chasing": {
    style: "excite",
    poolKey: "play:fetch:chasing",
    lines: [() => "ON IT.", () => "MINE.", () => "Intercepting!!"],
  },
  "fetch:carrying": {
    style: "say",
    poolKey: "play:fetch:carrying",
    lines: [
      () => "Got it got it got it.",
      () => "Returning the prize.",
      () => "Special delivery, slightly drooled on.",
    ],
  },
  "fetch:wiggling": {
    style: "excite",
    poolKey: "play:fetch:wiggling",
    lines: [
      () => "Again! The ball demands it.",
      () => "One more throw. The ball agrees.",
      () => "Re-arm the ball! Please!",
    ],
  },
  "fetch:done": {
    style: "say",
    poolKey: "play:fetch:done",
    lines: [
      () => "Final score: legs tired, heart full.",
      () => "Fetch complete. Flopping now.",
      () => "The ball rests. So do I.",
    ],
  },
  "firefly:armed": {
    style: "whisper",
    poolKey: "play:firefly:armed",
    lines: [
      () => "point me at a glow… any glow…",
      () => "pick a sparkle. i'll do the sneaking.",
      () => "lantern hunt is live. click one!",
    ],
  },
  "firefly:pouncing": {
    style: "whisper",
    poolKey: "play:firefly:pouncing",
    lines: [
      () => "sneak… sneak… POUNCE.",
      () => "soft paws… soft paws…",
      () => "almost… aaaalmost…",
    ],
  },
  "firefly:caught": {
    style: "excite",
    poolKey: "play:firefly:caught",
    lines: [
      () => "Caught one! It's blinking thanks.",
      () => "Lantern aboard! Gently, gently.",
      () => "One more glow for the crew!",
    ],
  },
  "firefly:missed": {
    style: "whisper",
    poolKey: "play:firefly:missed",
    lines: [
      () => "it juked me. respect.",
      () => "missed. the glow giggled.",
      () => "evasive sparkle. noted.",
    ],
  },
  "firefly:done": {
    style: "say",
    poolKey: "play:firefly:done",
    lines: [
      () => "Fly free, lanterns. Shift's over.",
      () => "Release ceremony complete. Goodnight, glows.",
      () => "Three lanterns, zero harm, full heart.",
    ],
  },
};

export const GIFT_LINES: Record<BuddyGiftMoment["item"], BuddyPlayLine> = {
  acorn: {
    style: "say",
    poolKey: "gift:acorn",
    lines: [
      () => "For you. It's the roundest one.",
      () => "A premium acorn. Hand-picked. Paw-picked.",
    ],
  },
  fish: {
    style: "say",
    poolKey: "gift:fish",
    lines: [
      () => "Brick says you should have this one.",
      () => "A fish for you! It already waved goodbye.",
    ],
  },
  leaf: {
    style: "say",
    poolKey: "gift:leaf",
    lines: [
      () => "The crunchiest leaf of the pile. Yours.",
      () => "This leaf matched your vibe. Take it.",
    ],
  },
  flower: {
    style: "say",
    poolKey: "gift:flower",
    lines: [
      () => "A flower! It smells like good mornings.",
      () => "Best petal-to-stem ratio in the meadow. Yours.",
    ],
  },
  spark: {
    style: "whisper",
    poolKey: "gift:spark",
    lines: [
      () => "one little glow stayed to say hi to you.",
      () => "shh. this sparkle wanted to meet you.",
    ],
  },
};

export const BUDDY_REQUEST_PROMPT: BuddyPlayLine = {
  style: "say",
  poolKey: "request:fetch",
  lines: [
    (name) => `${name} drops a ball at your feet. Play fetch?`,
    (name) => `${name} looks up with maximum eyes. One game?`,
    (name) => `${name} nudges the ball toward you. Quick round?`,
  ],
};
