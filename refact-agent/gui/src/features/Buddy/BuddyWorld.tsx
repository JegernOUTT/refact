import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import classNames from "classnames";
import { BuddyCharacter } from "./BuddyCharacter";
import type {
  BuddyCareAction,
  BuddyControl,
  BuddyEvent,
  BuddyPage,
  BuddyPetState,
  BuddyPulse,
  BuddyQuest,
  BuddyRuntimeEvent,
  BuddyScenePose,
  BuddySemanticState,
  BuddyShowcaseKind,
  BuddyShowcaseRun,
  BubblePosition,
  Palette,
  Stage,
} from "./types";
import {
  buildBuddyWorldState,
  type BuddyWorldState,
  type BuddyWorldTone,
} from "./buddyWorldModel";
import {
  advanceBuddyShowcasePhase,
  BUDDY_SHOWCASE_IDLE_COOLDOWN_MS,
  BUDDY_SHOWCASE_INITIAL_GRACE_MS,
  BUDDY_SHOWCASE_PHASE_DURATIONS_MS,
  BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS,
  createBuddyShowcaseRun,
  hasBuddyShowcaseRuntimeTrigger,
  type BuddyShowcaseTargetCandidate,
} from "./buddyShowcase";
import { drawShowcaseEvent } from "./buddyShowcaseDraw";
import { drawBuddyWorld } from "./buddyWorldDraw";
import {
  chooseBuddyWorldIntent,
  type BuddyWorldIntent,
  type BuddyWorldIntentKind,
} from "./buddyWorldDirector";
import styles from "./BuddyWorld.module.css";

interface BuddyWorldProps {
  palette: Palette;
  stage: Stage;
  state: BuddySemanticState;
  pulse: BuddyPulse | null | undefined;
  pet: BuddyPetState | undefined;
  nowPlaying: BuddyRuntimeEvent | null;
  activeQuest: BuddyQuest | null;
  activeSpeech: {
    text: string;
    controls: BuddyControl[];
    chat_id?: string;
  } | null;
  setupNeeded: boolean;
  compact?: boolean;
  homeDoorDisabled?: boolean;
  onCanvasEvent: (event: BuddyEvent) => void;
  onCare: (action: BuddyCareAction, toy?: string) => void;
  onOpenPage: (page: BuddyPage) => void;
  onRunMode: (mode: string) => void;
  onDismissSetup: () => void;
  onSpeechControl: (control: BuddyControl) => void;
  now?: Date;
}

const TONE_CLASS: Record<BuddyWorldTone, string> = {
  good: styles.toneGood,
  neutral: styles.toneNeutral,
  warning: styles.toneWarning,
  danger: styles.toneDanger,
};

const SETUP_MODE_ACTIONS = [
  { mode: "setup", label: "Warm up" },
  { mode: "setup_mcp", label: "Link MCP" },
  { mode: "setup_skills", label: "Teach skills" },
] as const;

const HOME_HOTSPOT = { x: 8.5, y: 67 } as const;
const BUDDY_CENTER_X = 50;
const BUDDY_MIN_X = 33;
const BUDDY_MAX_X = 67;
const MAX_RUNTIME_SHOWCASE_EVENT_IDS = 16;
const DIRECTOR_INTENT_TICK_MS = 2_000;
const DIRECTOR_MIN_INTENT_HOLD_MS = 7_000;
const DIRECTOR_CHARM_SPEECH_COOLDOWN_MS = 12_000;
const DIRECTOR_ACTIONABLE_SPEECH_COOLDOWN_MS = 20_000;
const DIRECTOR_REPEAT_KIND_COOLDOWN_MS = 30_000;
const MAX_RECENT_DIRECTOR_INTENTS = 12;

const RANDOM_IDLE_REACTIONS = [
  "Buddy does a tiny spin.",
  "Buddy watches the garden for a moment.",
  "Buddy checks the breeze and grins.",
  "Buddy makes a small happy bounce.",
  "Buddy pauses to inspect a sparkle.",
] as const;

const RANDOM_POSES = [
  "idle",
  "spin",
  "bounce",
  "look",
] as const satisfies readonly BuddyScenePose[];

type BuddyRandomPose = (typeof RANDOM_POSES)[number];

interface BuddyWaypoint {
  id: string;
  x: number;
  y: number;
  label: string;
  reaction: string;
}

interface RecentDirectorIntent {
  kind: BuddyWorldIntentKind;
  untilMs: number;
}

function clampBuddySceneX(x: number): number {
  return Math.max(BUDDY_MIN_X, Math.min(BUDDY_MAX_X, x));
}

function bubblePositionForSceneX(x: number): BubblePosition {
  if (x < 42) return "right";
  if (x > 58) return "left";
  return "top";
}

function buildBuddyShowcaseTargets(
  world: BuddyWorldState,
): BuddyShowcaseTargetCandidate[] {
  return world.objects.map((item) => ({
    id: item.id,
    x: item.x,
    y: item.y,
    label: item.label,
    sprite: item.sprite,
  }));
}

function buildBuddyWaypoints(world: BuddyWorldState): BuddyWaypoint[] {
  return [
    {
      id: "center",
      x: BUDDY_CENTER_X,
      y: 76,
      label: "clearing",
      reaction: "Buddy wanders back to the clearing.",
    },
    {
      id: "home",
      x: HOME_HOTSPOT.x,
      y: HOME_HOTSPOT.y,
      label: "home",
      reaction: "Buddy checks the front door lights.",
    },
    {
      id: "celestial",
      x: world.celestialX,
      y: world.celestialY,
      label: world.celestialLabel,
      reaction: `Buddy tracks the ${world.celestialLabel.toLowerCase()}.`,
    },
    ...world.objects.map((item) => ({
      id: item.id,
      x: item.x,
      y: item.y,
      label: item.label,
      reaction: `Buddy inspects ${item.label.toLowerCase()}.`,
    })),
    {
      id: "weather",
      x: world.weatherX,
      y: world.weatherY,
      label: world.weatherLabel,
      reaction: `Buddy watches ${world.weatherLabel.toLowerCase()}.`,
    },
  ];
}

function pickNextWaypointIndex(
  waypoints: BuddyWaypoint[],
  currentIndex: number,
): number {
  if (waypoints.length <= 1) return 0;

  const roll = Math.random();
  if (roll < 0.24) return 0;

  let nextIndex = currentIndex;
  while (nextIndex === currentIndex) {
    nextIndex = Math.floor(Math.random() * waypoints.length);
  }
  return nextIndex;
}

function randomIdleReaction(): string {
  return RANDOM_IDLE_REACTIONS[
    Math.floor(Math.random() * RANDOM_IDLE_REACTIONS.length)
  ];
}

function directorSpeechCooldownMs(intent: BuddyWorldIntent): number {
  return intent.speechKind === "actionable"
    ? DIRECTOR_ACTIONABLE_SPEECH_COOLDOWN_MS
    : DIRECTOR_CHARM_SPEECH_COOLDOWN_MS;
}

function activeRecentIntentKinds(
  recentIntents: RecentDirectorIntent[],
  nowMs: number,
): BuddyWorldIntentKind[] {
  return recentIntents
    .filter((intent) => intent.untilMs > nowMs)
    .map((intent) => intent.kind);
}

function rememberRecentIntentKind(
  recentIntents: RecentDirectorIntent[],
  kind: BuddyWorldIntentKind,
  nowMs: number,
): RecentDirectorIntent[] {
  return [
    { kind, untilMs: nowMs + DIRECTOR_REPEAT_KIND_COOLDOWN_MS },
    ...recentIntents.filter(
      (intent) => intent.kind !== kind && intent.untilMs > nowMs,
    ),
  ].slice(0, MAX_RECENT_DIRECTOR_INTENTS);
}

function prefersReducedMotion(): boolean {
  if (typeof window === "undefined") return false;
  if (typeof window.matchMedia !== "function") return false;
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

function resolveBuddyWorldSpeechOverride(args: {
  activeSpeechText: string | null;
  showcaseActive: boolean;
  showcaseSpeech: string | null;
  directorSpeech: string | null;
  reaction: string | null;
}): string | null {
  if (args.activeSpeechText !== null) return args.activeSpeechText;
  if (args.showcaseActive) return args.showcaseSpeech;
  if (args.directorSpeech !== null) return args.directorSpeech;
  return args.reaction;
}

export const BuddyWorld: React.FC<BuddyWorldProps> = ({
  palette,
  stage,
  state,
  pulse,
  pet,
  nowPlaying,
  activeQuest,
  activeSpeech,
  setupNeeded,
  compact = false,
  homeDoorDisabled = false,
  onCanvasEvent,
  onCare,
  onOpenPage,
  onRunMode,
  onDismissSetup,
  onSpeechControl,
  now,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [currentTime, setCurrentTime] = useState(() => now ?? new Date());
  const [reaction, setReaction] = useState<string | null>(null);
  const [activeWaypointIndex, setActiveWaypointIndex] = useState(0);
  const [lastWaypoint, setLastWaypoint] = useState<BuddyWaypoint | null>(null);
  const [randomPose, setRandomPose] = useState<BuddyRandomPose>("idle");
  const [showcaseRun, setShowcaseRun] = useState<BuddyShowcaseRun | null>(null);
  const [lastShowcaseKind, setLastShowcaseKind] =
    useState<BuddyShowcaseKind | null>(null);
  const [runtimeShowcaseEventIds, setRuntimeShowcaseEventIds] = useState<
    string[]
  >([]);
  const [idleTick, setIdleTick] = useState(0);
  const [idleGraceUntilMs] = useState(
    () => Date.now() + BUDDY_SHOWCASE_INITIAL_GRACE_MS,
  );
  const [nextIdleShowcaseAtMs, setNextIdleShowcaseAtMs] = useState(0);
  const [nextRuntimeShowcaseAtMs, setNextRuntimeShowcaseAtMs] = useState(0);
  const [directorIntent, setDirectorIntent] = useState<BuddyWorldIntent | null>(
    null,
  );
  const [directorIntentStartedAtMs, setDirectorIntentStartedAtMs] = useState(0);
  const [nextDirectorSpeechAtMs, setNextDirectorSpeechAtMs] = useState(0);
  const [recentDirectorIntents, setRecentDirectorIntents] = useState<
    RecentDirectorIntent[]
  >([]);
  const [reducedMotion, setReducedMotion] = useState(prefersReducedMotion);

  useEffect(() => {
    if (typeof window === "undefined") return;
    if (typeof window.matchMedia !== "function") {
      setReducedMotion(false);
      return;
    }

    const media = window.matchMedia("(prefers-reduced-motion: reduce)") as {
      matches: boolean;
      addEventListener?: (type: "change", listener: () => void) => void;
      removeEventListener?: (type: "change", listener: () => void) => void;
      addListener?: (listener: () => void) => void;
      removeListener?: (listener: () => void) => void;
    };
    const updateReducedMotion = () => setReducedMotion(media.matches);
    updateReducedMotion();
    if (typeof media.addEventListener === "function") {
      media.addEventListener("change", updateReducedMotion);
      return () => {
        if (typeof media.removeEventListener === "function") {
          media.removeEventListener("change", updateReducedMotion);
        }
      };
    }
    if (typeof media.addListener === "function") {
      media.addListener(updateReducedMotion);
      return () => {
        if (typeof media.removeListener === "function") {
          media.removeListener(updateReducedMotion);
        }
      };
    }
  }, []);

  useEffect(() => {
    if (now) {
      setCurrentTime(now);
      return;
    }
    const timer = window.setInterval(() => setCurrentTime(new Date()), 60_000);
    return () => window.clearInterval(timer);
  }, [now]);

  useEffect(() => {
    if (now) return;
    const lastSignalTime = state.activity?.lastSignalTime;
    if (
      typeof lastSignalTime !== "number" ||
      !Number.isFinite(lastSignalTime) ||
      lastSignalTime <= 0
    ) {
      return;
    }
    setCurrentTime(new Date());
  }, [now, state.activity?.lastSignalTime]);

  useEffect(() => {
    if (!reaction) return;
    const timer = window.setTimeout(() => setReaction(null), 5000);
    return () => window.clearTimeout(timer);
  }, [reaction]);

  useEffect(() => {
    if (randomPose === "idle") return;
    const timer = window.setTimeout(() => setRandomPose("idle"), 2600);
    return () => window.clearTimeout(timer);
  }, [randomPose]);

  const world = useMemo(
    () =>
      buildBuddyWorldState({
        now: currentTime,
        pulse,
        pet,
        nowPlaying,
        activeQuest,
        semanticState: state,
      }),
    [activeQuest, currentTime, nowPlaying, pet, pulse, state],
  );
  const waypoints = useMemo(() => buildBuddyWaypoints(world), [world]);
  const showcaseTargets = useMemo(
    () => buildBuddyShowcaseTargets(world),
    [world],
  );
  const activeWaypoint = waypoints[activeWaypointIndex % waypoints.length];
  const effectiveDirectorIntent =
    activeSpeech || showcaseRun ? null : directorIntent;
  const characterSceneX = clampBuddySceneX(
    showcaseRun
      ? showcaseRun.target.x
      : effectiveDirectorIntent
        ? effectiveDirectorIntent.targetX
        : activeWaypoint.x,
  );
  const characterSceneY = showcaseRun
    ? showcaseRun.target.y
    : effectiveDirectorIntent?.targetY;
  const characterDepthScale = showcaseRun
    ? 1
    : effectiveDirectorIntent?.depthScale;
  const bubblePosition = bubblePositionForSceneX(characterSceneX);

  useEffect(() => {
    setActiveWaypointIndex(0);
    setLastWaypoint(null);
  }, [world.headline]);

  useEffect(() => {
    if (showcaseRun) {
      setDirectorIntent(null);
    }
  }, [showcaseRun]);

  const startShowcase = useCallback(
    (strongRuntimeTrigger: boolean) => {
      if (showcaseRun) return false;
      const nowMs = Date.now();
      const run = createBuddyShowcaseRun({
        targets: showcaseTargets,
        nowPlaying,
        activeSpeechVisible: Boolean(activeSpeech) || Boolean(reaction),
        pet,
        nowMs,
        idleCooldownUntilMs: nextIdleShowcaseAtMs,
        runtimeCooldownUntilMs: nextRuntimeShowcaseAtMs,
        idleGraceUntilMs,
        lastShowcaseKind,
        runtimeShowcaseEventIds,
        strongRuntimeTrigger,
        world: {
          phase: world.phase,
          weather: world.weather,
        },
        pulse,
      });
      if (!run) return false;
      setShowcaseRun(run);
      setLastWaypoint(null);
      setLastShowcaseKind(run.kind);
      if (strongRuntimeTrigger && nowPlaying?.id) {
        setRuntimeShowcaseEventIds((eventIds) =>
          [
            nowPlaying.id,
            ...eventIds.filter((eventId) => eventId !== nowPlaying.id),
          ].slice(0, MAX_RUNTIME_SHOWCASE_EVENT_IDS),
        );
      }
      if (strongRuntimeTrigger) {
        setNextRuntimeShowcaseAtMs(nowMs + BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS);
      } else {
        setNextIdleShowcaseAtMs(nowMs + BUDDY_SHOWCASE_IDLE_COOLDOWN_MS);
      }
      return true;
    },
    [
      activeSpeech,
      idleGraceUntilMs,
      lastShowcaseKind,
      runtimeShowcaseEventIds,
      nextIdleShowcaseAtMs,
      nextRuntimeShowcaseAtMs,
      nowPlaying,
      pet,
      pulse,
      reaction,
      showcaseRun,
      showcaseTargets,
      world.phase,
      world.weather,
    ],
  );

  useEffect(() => {
    if (activeSpeech ?? reaction ?? showcaseRun ?? directorIntent) return;
    const delay = 4200 + Math.random() * 7200;
    const timer = window.setTimeout(() => {
      const roll = Math.random();
      if (roll < 0.18 && startShowcase(false)) return;

      if (roll < 0.34) {
        setRandomPose(
          RANDOM_POSES[Math.floor(Math.random() * RANDOM_POSES.length)],
        );
        setReaction(randomIdleReaction());
      } else if (roll < 0.46) {
        setLastWaypoint(null);
      } else {
        setLastWaypoint(null);
        setActiveWaypointIndex((index) =>
          pickNextWaypointIndex(waypoints, index),
        );
      }
      setIdleTick((tick) => tick + 1);
    }, delay);
    return () => window.clearTimeout(timer);
  }, [
    activeSpeech,
    directorIntent,
    idleTick,
    reaction,
    showcaseRun,
    startShowcase,
    waypoints,
  ]);

  useEffect(() => {
    if (activeSpeech ?? reaction ?? showcaseRun ?? directorIntent) return;
    if (lastWaypoint?.id === activeWaypoint.id) return;
    const timer = window.setTimeout(() => {
      setLastWaypoint(activeWaypoint);
      if (Math.random() < 0.72) {
        setReaction(activeWaypoint.reaction);
      }
    }, 2200);
    return () => window.clearTimeout(timer);
  }, [
    activeSpeech,
    activeWaypoint,
    directorIntent,
    lastWaypoint,
    reaction,
    showcaseRun,
  ]);

  useEffect(() => {
    if (
      activeSpeech !== null ||
      reaction !== null ||
      showcaseRun !== null ||
      nowPlaying === null ||
      !hasBuddyShowcaseRuntimeTrigger(nowPlaying)
    ) {
      return;
    }
    if (nowPlaying.id && runtimeShowcaseEventIds.includes(nowPlaying.id)) {
      return;
    }

    const nowMs = Date.now();
    if (nowMs < nextRuntimeShowcaseAtMs) {
      const timer = window.setTimeout(
        () => startShowcase(true),
        nextRuntimeShowcaseAtMs - nowMs,
      );
      return () => window.clearTimeout(timer);
    }

    startShowcase(true);
  }, [
    activeSpeech,
    runtimeShowcaseEventIds,
    nextRuntimeShowcaseAtMs,
    nowPlaying,
    reaction,
    showcaseRun,
    startShowcase,
  ]);

  useEffect(() => {
    if (!showcaseRun) return;
    const nowMs = Date.now();
    const elapsedMs = nowMs - showcaseRun.phaseStartedAtMs;
    const remainingMs = Math.max(
      0,
      BUDDY_SHOWCASE_PHASE_DURATIONS_MS[showcaseRun.phase] - elapsedMs,
    );
    const timer = window.setTimeout(() => {
      const currentNowMs = Date.now();
      const advanced = advanceBuddyShowcasePhase({
        run: showcaseRun,
        nowMs: currentNowMs,
      });
      setShowcaseRun(advanced);
      if (!advanced) {
        setNextIdleShowcaseAtMs(currentNowMs + BUDDY_SHOWCASE_IDLE_COOLDOWN_MS);
      }
    }, remainingMs + 16);
    return () => window.clearTimeout(timer);
  }, [showcaseRun]);

  useEffect(() => {
    const runDirector = () => {
      const nowMs = Date.now();
      const activeRecentKinds = activeRecentIntentKinds(
        recentDirectorIntents,
        nowMs,
      );
      setRecentDirectorIntents((recentIntents) => {
        const activeIntents = recentIntents.filter(
          (intent) => intent.untilMs > nowMs,
        );
        return activeIntents.length === recentIntents.length
          ? recentIntents
          : activeIntents;
      });

      if (showcaseRun || activeSpeech) {
        setDirectorIntent(null);
        return;
      }

      if (directorIntent) {
        const ageMs = nowMs - directorIntentStartedAtMs;
        const intentDurationMs = Number.isFinite(directorIntent.durationMs)
          ? directorIntent.durationMs
          : DIRECTOR_MIN_INTENT_HOLD_MS;
        const intentHoldMs = Math.max(
          DIRECTOR_MIN_INTENT_HOLD_MS,
          intentDurationMs,
        );
        if (ageMs < intentHoldMs) {
          return;
        }
      }

      const nextIntent = chooseBuddyWorldIntent({
        world,
        previousIntent: directorIntent,
        nowMs,
        activeSpeechVisible: activeSpeech !== null,
        showcaseActive: showcaseRun !== null,
        localReactionVisible: reaction !== null,
        reducedMotion,
        recentIntentKinds: activeRecentKinds,
      });
      if (!nextIntent) {
        setDirectorIntent(null);
        return;
      }

      const speechAllowed =
        nextIntent.speech !== null && nowMs >= nextDirectorSpeechAtMs;
      const intent = speechAllowed
        ? nextIntent
        : { ...nextIntent, speech: null };
      setDirectorIntent(intent);
      setDirectorIntentStartedAtMs(nowMs);
      setRecentDirectorIntents((recentIntents) =>
        rememberRecentIntentKind(recentIntents, intent.kind, nowMs),
      );
      if (speechAllowed) {
        setNextDirectorSpeechAtMs(nowMs + directorSpeechCooldownMs(intent));
      }
      setLastWaypoint(null);
    };

    runDirector();
    const timer = window.setInterval(
      runDirector,
      reducedMotion ? DIRECTOR_INTENT_TICK_MS * 2 : DIRECTOR_INTENT_TICK_MS,
    );
    return () => window.clearInterval(timer);
  }, [
    activeSpeech,
    directorIntent,
    directorIntentStartedAtMs,
    nextDirectorSpeechAtMs,
    reaction,
    recentDirectorIntents,
    reducedMotion,
    showcaseRun,
    world,
  ]);

  useEffect(() => {
    let frame = 0;
    let raf = 0;
    const render = () => {
      if (document.hidden) {
        raf = window.requestAnimationFrame(render);
        return;
      }

      frame += 1;
      const canvas = canvasRef.current;
      const ctx = canvas?.getContext("2d");
      if (canvas && ctx) {
        const rect = canvas.getBoundingClientRect();
        const cssWidth = Math.max(1, Math.round(rect.width || 720));
        const cssHeight = Math.max(
          1,
          Math.round(rect.height || (compact ? 190 : 260)),
        );
        const ratio = window.devicePixelRatio || 1;
        const targetWidth = Math.round(cssWidth * ratio);
        const targetHeight = Math.round(cssHeight * ratio);
        if (canvas.width !== targetWidth || canvas.height !== targetHeight) {
          canvas.width = targetWidth;
          canvas.height = targetHeight;
        }
        ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
        drawBuddyWorld({
          ctx,
          world,
          palette,
          frame,
          width: cssWidth,
          height: cssHeight,
          compact,
          reducedMotion,
        });
        if (showcaseRun) {
          drawShowcaseEvent({
            ctx,
            run: showcaseRun,
            world,
            palette,
            frame,
            width: cssWidth,
            height: cssHeight,
            compact,
            reducedMotion,
          });
        }
      }
      raf = window.requestAnimationFrame(render);
    };
    raf = window.requestAnimationFrame(render);
    return () => window.cancelAnimationFrame(raf);
  }, [compact, palette, reducedMotion, showcaseRun, world]);

  const handleCelestialClick = () => {
    if (!showcaseRun) {
      setActiveWaypointIndex(
        Math.max(
          0,
          waypoints.findIndex((point) => point.id === "celestial"),
        ),
      );
    }
    if (world.phase === "night") {
      onCare("sleep");
      if (!showcaseRun) {
        setReaction("Buddy curls up under the moon and saves energy.");
      }
      return;
    }
    onCare("play", "scroll");
    if (!showcaseRun) {
      setReaction("Buddy catches a warm sunbeam and opens the focus scroll.");
    }
  };

  const handleWeatherClick = () => {
    if (!showcaseRun) {
      setActiveWaypointIndex(
        Math.max(
          0,
          waypoints.findIndex((point) => point.id === "weather"),
        ),
      );
    }
    if (world.weather === "storm") {
      onOpenPage({ type: "stats" });
      if (!showcaseRun) {
        setReaction("Buddy marked the storm front for investigation.");
      }
      return;
    }
    if (world.weather === "rain") {
      onOpenPage({ type: "knowledge_graph" });
      if (!showcaseRun) {
        setReaction("Buddy follows the rain into the memory garden.");
      }
      return;
    }
    onCare("pet");
    if (!showcaseRun) {
      setReaction("Buddy chirps back at the sky.");
    }
  };

  const handleHomeClick = () => {
    if (!showcaseRun) {
      setActiveWaypointIndex(
        Math.max(
          0,
          waypoints.findIndex((point) => point.id === "home"),
        ),
      );
    }
    if (homeDoorDisabled) {
      if (!showcaseRun) {
        setReaction("Buddy is already home.");
      }
      return;
    }
    onOpenPage({ type: "buddy" });
    if (!showcaseRun) {
      setReaction("Buddy opens the front door.");
    }
  };

  const showcasePose =
    showcaseRun !== null && showcaseRun.phase !== "travel"
      ? showcaseRun.pose
      : null;
  const directorPose = effectiveDirectorIntent?.pose ?? null;
  const characterPose: BuddyScenePose =
    showcasePose ?? directorPose ?? randomPose;
  const directorSpeech = effectiveDirectorIntent?.speech ?? null;
  const speechOverride = resolveBuddyWorldSpeechOverride({
    activeSpeechText: activeSpeech?.text ?? null,
    showcaseActive: showcaseRun !== null,
    showcaseSpeech: showcaseRun?.speech ?? null,
    directorSpeech,
    reaction,
  });
  const speechSource = activeSpeech
    ? "active"
    : showcaseRun
      ? "showcase"
      : directorSpeech
        ? "director"
        : reaction
          ? "reaction"
          : "none";

  return (
    <section
      className={classNames(styles.scene, { [styles.compact]: compact })}
      data-phase={world.phase}
      data-weather={world.weather}
      data-atmosphere-mood={world.atmosphere.mood}
      data-world-mood={world.atmosphere.mood}
      data-world-layers={world.atmosphere.layers.join(" ") || "none"}
      data-vitality={world.vitality}
      data-showcase={showcaseRun?.kind ?? "none"}
      data-showcase-phase={showcaseRun?.phase ?? "idle"}
      data-buddy-intent={effectiveDirectorIntent?.kind ?? "none"}
      data-speech-priority="backend-showcase-director-local"
      data-speech-source={speechSource}
      data-speech-text={speechOverride ?? undefined}
      data-testid="buddy-world"
      aria-label={`Buddy virtual scene: ${world.phaseLabel}. ${world.vitalityLabel}.`}
    >
      <canvas
        ref={canvasRef}
        className={styles.canvas}
        data-testid="buddy-world-canvas"
      />

      <button
        type="button"
        className={classNames(styles.hotspot, styles.celestialHotspot)}
        style={{ left: `${world.celestialX}%`, top: `${world.celestialY}%` }}
        onClick={handleCelestialClick}
        aria-label={`${world.celestialAction} with ${world.celestialLabel}`}
        title={`${world.celestialAction} with ${world.celestialLabel}`}
      />

      <button
        type="button"
        className={classNames(styles.hotspot, styles.weatherHotspot)}
        style={{ left: `${world.weatherX}%`, top: `${world.weatherY}%` }}
        onClick={handleWeatherClick}
        aria-label={`Interact with ${world.weatherLabel}`}
        title={world.weatherLabel}
      />

      <button
        type="button"
        className={classNames(styles.hotspot, styles.homeHotspot)}
        style={{ left: `${HOME_HOTSPOT.x}%`, top: `${HOME_HOTSPOT.y}%` }}
        onClick={handleHomeClick}
        aria-label={
          homeDoorDisabled ? "Buddy home entrance" : "Open Buddy home"
        }
        title={homeDoorDisabled ? "Buddy is home" : "Open Buddy home"}
      />

      {world.objects.map((item) => (
        <button
          key={item.id}
          type="button"
          className={classNames(styles.objectHotspot, TONE_CLASS[item.tone])}
          style={{ left: `${item.x}%`, top: `${item.y}%` }}
          onClick={() => {
            if (!showcaseRun) {
              setActiveWaypointIndex(
                Math.max(
                  0,
                  waypoints.findIndex((point) => point.id === item.id),
                ),
              );
            }
            onOpenPage(item.page);
            if (!showcaseRun) {
              setReaction(`Buddy hops toward ${item.label.toLowerCase()}.`);
            }
          }}
          aria-label={`Open ${item.label}`}
          title={`${item.label}: ${item.description}`}
        >
          <span className={styles.objectTooltip}>
            <span className={styles.objectLabel}>{item.label}</span>
            <span className={styles.objectValue}>{item.value}</span>
          </span>
        </button>
      ))}

      {lastWaypoint && (
        <div
          className={styles.waypointPing}
          style={{ left: `${lastWaypoint.x}%`, top: `${lastWaypoint.y}%` }}
          aria-hidden
        />
      )}

      <BuddyCharacter
        state={state}
        stage={stage}
        palette={palette}
        displaySize={compact ? 230 : 282}
        bubblePosition={bubblePosition}
        randomizeBubblePosition={false}
        sceneXPercent={characterSceneX}
        sceneYPercent={characterSceneY}
        sceneDepthScale={characterDepthScale}
        scenePose={characterPose}
        speechText={speechOverride}
        speechControls={activeSpeech ? activeSpeech.controls : undefined}
        onCanvasEvent={onCanvasEvent}
        onSpeechControl={activeSpeech ? onSpeechControl : undefined}
      />

      {setupNeeded && (
        <div className={styles.setupDock}>
          {SETUP_MODE_ACTIONS.map((item) => (
            <button
              key={item.mode}
              type="button"
              className={styles.sceneButton}
              onClick={() => onRunMode(item.mode)}
            >
              {item.label}
            </button>
          ))}
          <button
            type="button"
            className={styles.sceneButtonGhost}
            onClick={onDismissSetup}
          >
            Later
          </button>
        </div>
      )}

      <div className={styles.careDock} aria-label="Buddy scene care actions">
        <button
          type="button"
          className={styles.sceneButton}
          aria-label="Water Buddy garden"
          onClick={() => onCare("feed")}
        >
          🍜
        </button>
        <button
          type="button"
          className={styles.sceneButton}
          aria-label="Hunt bugs with Buddy"
          onClick={() => onCare("play", "bug")}
        >
          🐛
        </button>
        <button
          type="button"
          className={styles.sceneButton}
          aria-label="Clean Buddy"
          onClick={() => onCare("clean")}
        >
          🧼
        </button>
        <button
          type="button"
          className={styles.sceneButton}
          aria-label="Let Buddy rest"
          onClick={() => onCare("sleep")}
        >
          😴
        </button>
      </div>
    </section>
  );
};
