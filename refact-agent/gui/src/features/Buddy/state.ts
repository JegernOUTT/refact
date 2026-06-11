import { NAMES, PALETTES, SIGNALS } from "./constants";
import type {
  BuddySemanticState,
  BuddyAnimState,
  LogEntry,
  SignalDef,
} from "./types";

export function randomName(): string {
  return NAMES[Math.floor(Math.random() * NAMES.length)];
}

export function randomPaletteIndex(): number {
  return Math.floor(Math.random() * PALETTES.length);
}

export function createInitialSemanticState(): BuddySemanticState {
  return {
    name: randomName(),
    paletteIndex: randomPaletteIndex(),
    born: Date.now(),
    mood: {
      happiness: 65,
      energy: 80,
      curiosity: 55,
      anxiety: 0,
      boredom: 0,
      affection: 0,
    },
    personality: {
      playfulness: 40,
      confidence: 20,
      clinginess: 30,
      resilience: 20,
      chaos: 25,
      sociability: 35,
      curiosity: 45,
    },
    progress: { xp: 0, stage: 0 },
    activity: {
      mood: "idle",
      animationType: "idle",
      lastSignalTime: 0,
      lastSignalType: null,
    },
    skills: [],
    log: [],
  };
}

export function createInitialAnimState(): BuddyAnimState {
  return {
    frame: 0,
    blinkTick: 0,
    nextBlinkAt: 80 + Math.random() * 160,
    blinking: false,
    blinkFrames: 0,
    bobPhase: 0,
    celebrationTimer: 0,
    shakeIntensity: 0,
    eyeLookX: 0,
    eyeLookY: 0,
    cursorTargetX: 0,
    cursorTargetY: 0,
    eyeStyle: "normal",
    eyeStyleTimer: 0,
    squashX: 1,
    squashY: 1,
    squashTargetX: 1,
    squashTargetY: 1,
    sparks: [],
    floatingEmojis: [],
    sleepParticles: [],
    orbitingOrbs: [],
    afterimages: [],
    speedLines: [],
    groundFX: [],
    screenFlash: 0,
    screenGlitch: 0,
    mouseProximity: 0,
    mouseAngle: 0,
    mouseOnBuddy: false,
    mouseSpeed: 0,
    headTilt: 0,
    breathScale: 0,
    hoverGlow: 0,
    nuzzleOffsetX: 0,
    nuzzleOffsetY: 0,
    mouseNearTimer: 0,
    dragging: false,
    petCount: 0,
    idleAction: "none",
    idleActionTimer: 0,
    earState: 0,
    earAnimProgress: 0,
    errorStreak: 0,
    successStreak: 0,
    heat: 0,
    combo: { count: 0, signalType: null, displayTimer: 0, rainbowHue: 0 },
    signalHistory: [],
    stageQuirkTick: 0,
    quirkActive: false,
    quirkType: "",
    quirkEndFrame: 0,
    phaseAlpha: 1,
    shadowClone: null,
    levitationOffset: 0,
    auraPulseIntensity: 0,
    walkOffsetX: 0,
    walkTargetX: 0,
    walkDirection: 1,
    walkSpeed: 0,
    walking: false,
    walkPhase: 0,
    toyActive: false,
    toyType: null,
    toyAnimPhase: 0,
    toyDurationTimer: 0,
    moodType: "idle",
    statusText: "",
    statusOpacity: 0,
    statusTargetOpacity: 0,
    statusTimer: 0,
    activeScene: "",
    activeSceneVariant: "",
    activeSceneTimer: 0,
    lidClose: 0,
    lidBase: 0,
    blinkQueue: 0,
    slowBlinkTimer: 0,
    pupilDilation: 0.45,
    saccadeFrames: 0,
    gazeSettleFrames: 40,
    breathPhase: 0,
    pantTimer: 0,
    recentIdleActions: [],
    nextIdleBias: {},
    idleActionTotal: 0,
    sighCooldown: 0,
    rareActionCooldown: 300,
    walkVel: 0,
    walkLeanFrames: 0,
    stumbleCooldown: 600,
    zoomiesDashesLeft: 0,
    earTwitchTimer: 0,
    earTwitchSide: 1,
    shiverTimer: 0,
    blushTimer: 0,
    beats: [],
    petSessionCount: 0,
    lastPetFrame: -9999,
    temperamentSeed: 0,
    facing: 1,
    facingLerp: 1,
    tailPhase: 0,
    tailEnergy: 0.4,
    tailDroop: 0,
    armPose: "rest",
    armAnimPhase: 0,
    wingFlap: 0,
    dancePhase: 0,
    sweatTimer: 0,
    veinTimer: 0,
    cheekPuffTimer: 0,
    auraTimer: 0,
    envTickCooldown: 240,
  };
}

function makeLogTimestamp(): string {
  const d = new Date();
  return [d.getHours(), d.getMinutes(), d.getSeconds()]
    .map((n) => String(n).padStart(2, "0"))
    .join(":");
}

function addLogEntry(log: LogEntry[], entry: LogEntry): LogEntry[] {
  return [entry, ...log].slice(0, 40);
}

export type SemanticAction =
  | { kind: "signal"; signalType: string }
  | { kind: "add_xp"; amount: number }
  | { kind: "pet" }
  | { kind: "rename"; name: string }
  | { kind: "next_palette" }
  | { kind: "reset" }
  | { kind: "patch"; patch: Partial<BuddySemanticState> };

export function reduceSemanticState(
  state: BuddySemanticState,
  action: SemanticAction,
): BuddySemanticState {
  switch (action.kind) {
    case "signal": {
      const def = SIGNALS[action.signalType] as SignalDef | undefined;
      if (def === undefined) return state;

      const xpGain = def.xp;
      const newXP = state.progress.xp + xpGain;
      // Stage is backend-authoritative; never advance it locally from signals.
      // Stage only updates via snapshot sync (the "patch" action).

      const moodDelta = def.isError
        ? { happiness: -9, anxiety: 18, energy: -2 }
        : def.isWin
          ? { happiness: 6, anxiety: -6, energy: -3 }
          : { happiness: 0, anxiety: 0, energy: -3 };

      const newMood = {
        ...state.mood,
        happiness: Math.max(
          0,
          Math.min(100, state.mood.happiness + moodDelta.happiness),
        ),
        energy: Math.max(
          0,
          Math.min(100, state.mood.energy + moodDelta.energy),
        ),
        anxiety: Math.max(
          0,
          Math.min(100, state.mood.anxiety + moodDelta.anxiety),
        ),
        boredom: 0,
      };

      const resilienceDelta = def.isError ? 0.5 : 0;
      const confidenceDelta = def.isWin ? 0.5 : 0;
      const newPersonality = {
        ...state.personality,
        resilience: Math.min(
          100,
          state.personality.resilience + resilienceDelta,
        ),
        confidence: Math.min(
          100,
          state.personality.confidence + confidenceDelta,
        ),
      };

      return {
        ...state,
        mood: newMood,
        personality: newPersonality,
        // Keep existing stage — stage advances only via backend snapshot sync
        progress: { xp: newXP, stage: state.progress.stage },
        activity: {
          mood: def.mood,
          animationType: def.animationType,
          lastSignalTime: Date.now(),
          lastSignalType: action.signalType,
        },
      };
    }

    case "add_xp": {
      // Local XP accumulation for mood tracking; stage is backend-authoritative.
      const newXP = state.progress.xp + action.amount;
      return {
        ...state,
        progress: { xp: newXP, stage: state.progress.stage },
      };
    }

    case "pet": {
      return {
        ...state,
        mood: {
          ...state.mood,
          affection: Math.min(100, state.mood.affection + 8),
          happiness: Math.min(100, state.mood.happiness + 3),
        },
        personality: {
          ...state.personality,
          playfulness: Math.min(100, state.personality.playfulness + 0.5),
        },
        log: addLogEntry(state.log, {
          icon: "💕",
          message: "petted",
          timestamp: makeLogTimestamp(),
        }),
      };
    }

    case "rename": {
      return { ...state, name: action.name };
    }

    case "next_palette": {
      return {
        ...state,
        paletteIndex: (state.paletteIndex + 1) % PALETTES.length,
      };
    }

    case "reset": {
      return createInitialSemanticState();
    }

    case "patch": {
      return { ...state, ...action.patch };
    }
  }
}
