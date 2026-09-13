import {
  createContext,
  useContext,
  useEffect,
  useState,
  type RefObject,
} from "react";
import type { BuddyScenePose } from "./types";
import type { WorkItem } from "./operatorTypes";
import type { NativeWorkView } from "../../services/refact/buddyNativeWork";

/** Fixture-only approval boundary. No production provider enables this. */
export const BuddyPresencePreviewContext = createContext({
  enabled: false,
  lowPower: false,
  textOnly: false,
  reducedMotion: undefined as boolean | undefined,
  workState: "idle" as BuddyWorkState,
});

export function useBuddyPresencePreferences() {
  const input = useContext(BuddyPresencePreviewContext);
  return input.enabled
    ? input
    : {
        enabled: false,
        lowPower: false,
        textOnly: false,
        reducedMotion: undefined,
        workState: "idle" as BuddyWorkState,
      };
}

/** Correctness, not polish: hidden surfaces own no animation-frame chain. */
export function useBuddySurfaceVisible(ref: RefObject<Element>) {
  const [documentVisible, setDocumentVisible] = useState(
    () => !document.hidden,
  );
  const [surfaceVisible, setSurfaceVisible] = useState(true);
  useEffect(() => {
    const sync = () => setDocumentVisible(!document.hidden);
    document.addEventListener("visibilitychange", sync);
    const element = ref.current;
    const observer =
      typeof IntersectionObserver === "undefined"
        ? null
        : new IntersectionObserver(([entry]) =>
            setSurfaceVisible(entry.isIntersecting),
          );
    if (element) observer?.observe(element);
    return () => {
      observer?.disconnect();
      document.removeEventListener("visibilitychange", sync);
    };
  }, [ref]);
  return documentVisible && surfaceVisible;
}

/**
 * Shared, side-effect-free motion / work-state grammar (plan B34, B37).
 *
 * This module only describes motion; it never activates new avatar polish and
 * never changes an existing scene, palette, or cosmetic. Callers keep their own
 * rendering. Nothing here can grant authority or mutate work.
 */

export type BuddyWorkState =
  | "unavailable"
  | "resting"
  | "idle"
  | "observing"
  | "working"
  | "delivering"
  | "blocked"
  | "awaiting_decision";

export type BuddyMotionEmphasis = "still" | "calm" | "active" | "urgent";

export interface BuddyPresenceMotion {
  /** Work state this motion describes. */
  state: BuddyWorkState;
  /** Pose reused from the existing scene pose vocabulary. No new assets. */
  pose: BuddyScenePose;
  emphasis: BuddyMotionEmphasis;
  /** Cross-fade budget in ms; 0 when motion must be cut instantly. */
  transitionMs: number;
  /** A lower-priority state may not interrupt a higher-priority one. */
  priority: number;
  /**
   * True when this state owns the attention layer, so ambient/idle motion and
   * ambient speech must stay silent (no overlapping speech/control layers).
   */
  ownsAttention: boolean;
  /** Screen-reader sentence; never claims work that did not happen. */
  label: string;
}

const MOTION_GRAMMAR: Record<BuddyWorkState, BuddyPresenceMotion> = {
  unavailable: {
    state: "unavailable",
    pose: "idle",
    emphasis: "still",
    transitionMs: 0,
    priority: -1,
    ownsAttention: false,
    label: "Current work presence unavailable; no execution is inferred.",
  },
  resting: {
    state: "resting",
    pose: "sleepy",
    emphasis: "still",
    transitionMs: 420,
    priority: 0,
    ownsAttention: false,
    label: "Resting. Nothing is running.",
  },
  idle: {
    state: "idle",
    pose: "idle",
    emphasis: "calm",
    transitionMs: 360,
    priority: 1,
    ownsAttention: false,
    label: "Idle and available.",
  },
  observing: {
    state: "observing",
    pose: "look",
    emphasis: "calm",
    transitionMs: 300,
    priority: 2,
    ownsAttention: false,
    label: "Watching for changes. No work started.",
  },
  working: {
    state: "working",
    pose: "dig",
    emphasis: "active",
    transitionMs: 240,
    priority: 3,
    ownsAttention: true,
    label: "Working on the current item.",
  },
  delivering: {
    state: "delivering",
    pose: "carry",
    emphasis: "active",
    transitionMs: 200,
    priority: 4,
    ownsAttention: true,
    label: "Delivering a finished result.",
  },
  blocked: {
    state: "blocked",
    pose: "shield",
    emphasis: "urgent",
    transitionMs: 160,
    priority: 5,
    ownsAttention: true,
    label: "Blocked. Work stopped and is waiting on something.",
  },
  awaiting_decision: {
    state: "awaiting_decision",
    pose: "look",
    emphasis: "urgent",
    transitionMs: 0,
    priority: 6,
    ownsAttention: true,
    label: "Waiting for your decision.",
  },
};

export const BUDDY_WORK_STATES: readonly BuddyWorkState[] = [
  "unavailable",
  "resting",
  "idle",
  "observing",
  "working",
  "delivering",
  "blocked",
  "awaiting_decision",
];

export function buddyPresenceMotion(
  state: BuddyWorkState,
): BuddyPresenceMotion {
  return MOTION_GRAMMAR[state];
}

/** Only native execution or saved work facts; a goal alone is not execution. */
export function buddyWorkStateFromFacts(
  work: readonly Pick<WorkItem, "state">[],
  native: readonly Pick<
    NativeWorkView,
    | "state"
    | "pending_questions"
    | "approval_count"
    | "browser_decision_pending"
  >[],
  pendingQuestion = false,
): BuddyWorkState {
  if (
    pendingQuestion ||
    work.some((w) => w.state === "needs_attention") ||
    native.some(
      (n) =>
        n.pending_questions > 0 ||
        n.approval_count > 0 ||
        n.browser_decision_pending ||
        n.state === "waiting_user_input",
    )
  )
    return "awaiting_decision";
  if (
    work.some((w) => w.state === "blocked") ||
    native.some((n) => ["error", "paused", "waiting_ide"].includes(n.state))
  )
    return "blocked";
  if (
    work.some((w) => w.state === "working") ||
    native.some((n) => ["generating", "executing_tools"].includes(n.state))
  )
    return "working";
  // A historical delivery is not a currently running delivery animation.
  if (work.some((w) => w.state === "watching")) return "observing";
  return "idle";
}

export type BuddyMotionTransitionKind =
  | "hold"
  | "blend"
  | "preempt"
  | "rejected";

export interface BuddyMotionTransition {
  kind: BuddyMotionTransitionKind;
  from: BuddyWorkState;
  to: BuddyWorkState;
  /** Effective cross-fade duration after reduced-motion/preemption rules. */
  durationMs: number;
  /** True when the outgoing motion must be cancelled rather than blended. */
  cancelsPrevious: boolean;
}

export interface BuddyMotionTransitionOptions {
  reducedMotion?: boolean;
  /** Set while a committed beat (e.g. a play phase) must not be cut short. */
  currentLocked?: boolean;
}

/**
 * Resolve one state change. Higher-priority states preempt (cancel) lower ones
 * immediately; equal or lower priority blends, and is rejected while the
 * current motion is locked.
 */
export function resolveBuddyMotionTransition(
  from: BuddyWorkState,
  to: BuddyWorkState,
  options: BuddyMotionTransitionOptions = {},
): BuddyMotionTransition {
  const current = buddyPresenceMotion(from);
  const next = buddyPresenceMotion(to);
  if (from === to) {
    return { kind: "hold", from, to, durationMs: 0, cancelsPrevious: false };
  }
  const preempts = next.priority > current.priority;
  if (!preempts && options.currentLocked === true) {
    return {
      kind: "rejected",
      from,
      to: from,
      durationMs: 0,
      cancelsPrevious: false,
    };
  }
  const durationMs =
    options.reducedMotion === true || preempts ? 0 : next.transitionMs;
  return {
    kind: preempts ? "preempt" : "blend",
    from,
    to,
    durationMs,
    cancelsPrevious: preempts,
  };
}

/**
 * Ambient/idle motion and ambient speech must yield whenever an attention-owning
 * state or an open speech/control layer is present.
 */
export function shouldSuppressAmbientMotion(args: {
  state: BuddyWorkState;
  speechVisible?: boolean;
  controlsVisible?: boolean;
}): boolean {
  if (buddyPresenceMotion(args.state).ownsAttention) return true;
  return args.speechVisible === true || args.controlsVisible === true;
}

/* ------------------------------------------------------------------ */
/* Animation loop policy (B37: hidden loops paused, reduced motion,    */
/* low power). One shared policy so every surface behaves the same.    */
/* ------------------------------------------------------------------ */

export const PRESENCE_INTERACTIVE_FRAME_MS = 0;
export const PRESENCE_AMBIENT_FRAME_MS = 1000 / 12;
export const PRESENCE_REDUCED_MOTION_FRAME_MS = 1000 / 4;
export const PRESENCE_LOW_POWER_FRAME_MS = 1000 / 6;

export type BuddyLoopStopReason =
  | "unmounted"
  | "document_hidden"
  | "surface_hidden"
  | "loop_disabled";

export interface BuddyPresenceLoopInput {
  /** False once the owning component unmounted or its element went away. */
  mounted: boolean;
  /** `document.hidden` for the owning document. */
  documentHidden: boolean;
  /** False when the surface itself is offscreen/collapsed (e.g. observer). */
  surfaceVisible?: boolean;
  /** Explicit disable switch, used by default-disabled previews. */
  enabled?: boolean;
  reducedMotion?: boolean;
  lowPower?: boolean;
  /** True while a user-driven interaction needs full frame rate. */
  interactive?: boolean;
  /**
   * Ambient draw interval owned by the calling surface. Existing surfaces pass
   * their current constant so this policy never changes their scheduling.
   */
  ambientFrameIntervalMs?: number;
}

export interface BuddyPresenceLoopPolicy {
  running: boolean;
  /** Minimum ms between drawn frames; 0 means every animation frame. */
  frameIntervalMs: number;
  stopReason: BuddyLoopStopReason | null;
}

export function resolveBuddyPresenceLoop(
  input: BuddyPresenceLoopInput,
): BuddyPresenceLoopPolicy {
  const stopped = (
    stopReason: BuddyLoopStopReason,
  ): BuddyPresenceLoopPolicy => ({
    running: false,
    frameIntervalMs: Number.POSITIVE_INFINITY,
    stopReason,
  });
  if (!input.mounted) return stopped("unmounted");
  if (input.enabled === false) return stopped("loop_disabled");
  if (input.documentHidden) return stopped("document_hidden");
  if (input.surfaceVisible === false) return stopped("surface_hidden");
  const ambientFrameIntervalMs =
    typeof input.ambientFrameIntervalMs === "number" &&
    Number.isFinite(input.ambientFrameIntervalMs) &&
    input.ambientFrameIntervalMs >= 0
      ? input.ambientFrameIntervalMs
      : PRESENCE_AMBIENT_FRAME_MS;
  if (input.reducedMotion === true) {
    return {
      running: true,
      frameIntervalMs: Math.max(
        PRESENCE_REDUCED_MOTION_FRAME_MS,
        ambientFrameIntervalMs,
      ),
      stopReason: null,
    };
  }
  if (input.lowPower === true) {
    return {
      running: true,
      frameIntervalMs: Math.max(
        PRESENCE_LOW_POWER_FRAME_MS,
        ambientFrameIntervalMs,
      ),
      stopReason: null,
    };
  }
  return {
    running: true,
    frameIntervalMs:
      input.interactive === true
        ? PRESENCE_INTERACTIVE_FRAME_MS
        : ambientFrameIntervalMs,
    stopReason: null,
  };
}

/** Convenience wrapper for the existing canvas loops. */
export function shouldRunPresenceLoop(input: BuddyPresenceLoopInput): boolean {
  return resolveBuddyPresenceLoop(input).running;
}

/**
 * True when a frame at `timestampMs` should actually be drawn.
 * Callers keep requesting animation frames; only drawing is throttled.
 */
export function shouldDrawPresenceFrame(
  policy: BuddyPresenceLoopPolicy,
  timestampMs: number,
  lastDrawMs: number,
): boolean {
  if (!policy.running) return false;
  if (policy.frameIntervalMs <= 0) return true;
  return timestampMs - lastDrawMs >= policy.frameIntervalMs;
}

/* ------------------------------------------------------------------ */
/* Non-punitive continuity (B36).                                      */
/* ------------------------------------------------------------------ */

export type BuddyReturnGreeting = "none" | "welcome_back" | "long_absence";

export interface BuddyContinuityInput {
  lastSeenAtMs: number;
  nowMs: number;
}

export interface BuddyContinuity {
  awayMs: number;
  greeting: BuddyReturnGreeting;
  /** Always false: absence never costs progress, mood, or stage. */
  penaltyApplied: false;
  label: string;
}

export const CONTINUITY_WELCOME_BACK_MS = 6 * 60 * 60 * 1000;
export const CONTINUITY_LONG_ABSENCE_MS = 7 * 24 * 60 * 60 * 1000;

export function resolveBuddyContinuity(
  input: BuddyContinuityInput,
): BuddyContinuity {
  const awayMs = Math.max(
    0,
    (Number.isFinite(input.nowMs) ? input.nowMs : 0) -
      (Number.isFinite(input.lastSeenAtMs) ? input.lastSeenAtMs : 0),
  );
  if (awayMs >= CONTINUITY_LONG_ABSENCE_MS) {
    return {
      awayMs,
      greeting: "long_absence",
      penaltyApplied: false,
      label: "Everything is exactly where you left it.",
    };
  }
  if (awayMs >= CONTINUITY_WELCOME_BACK_MS) {
    return {
      awayMs,
      greeting: "welcome_back",
      penaltyApplied: false,
      label: "Welcome back. Nothing was lost while you were away.",
    };
  }
  return {
    awayMs,
    greeting: "none",
    penaltyApplied: false,
    label: "Continuing where you left off.",
  };
}
