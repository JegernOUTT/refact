import { NAMES, PALETTES, SIGNALS, SKILLS, STAGES } from "./constants";
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
    statusText: "",
    statusOpacity: 0,
    statusTargetOpacity: 0,
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

function resolveStageFromXP(xp: number): number {
  let stage = 0;
  for (let i = STAGES.length - 1; i >= 0; i--) {
    if (xp >= STAGES[i].xpThreshold) {
      stage = i;
      break;
    }
  }
  return stage;
}

function checkNewSkills(state: BuddySemanticState): {
  newSkills: string[];
  updatedLog: LogEntry[];
} {
  const newSkills: string[] = [];
  let updatedLog = state.log;
  for (const skill of SKILLS) {
    if (
      !state.skills.includes(skill.id) &&
      state.progress.xp >= skill.xpThreshold
    ) {
      newSkills.push(skill.id);
      updatedLog = addLogEntry(updatedLog, {
        icon: "⭐",
        message: `Learned: ${skill.name}`,
        timestamp: makeLogTimestamp(),
      });
    }
  }
  return { newSkills, updatedLog };
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
      const def: SignalDef | undefined = SIGNALS[action.signalType];
      if (def === undefined) return state;

      const xpGain = def.xp;
      const newXP = state.progress.xp + xpGain;
      const newStage = resolveStageFromXP(newXP);
      const stageEvolved = newStage > state.progress.stage;

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

      const logEntry: LogEntry = {
        icon: def.icon,
        message: action.signalType.replace(/_/g, " "),
        timestamp: makeLogTimestamp(),
        ...(xpGain > 0 ? { xpGained: `+${xpGain}xp` } : {}),
      };

      let newLog = addLogEntry(state.log, logEntry);

      if (stageEvolved) {
        newLog = addLogEntry(newLog, {
          icon: "🌟",
          message: `EVOLVED → ${STAGES[newStage].name}`,
          timestamp: makeLogTimestamp(),
        });
      }

      const baseState: BuddySemanticState = {
        ...state,
        mood: newMood,
        personality: newPersonality,
        progress: { xp: newXP, stage: newStage },
        activity: {
          mood: def.mood,
          animationType: def.animationType,
          lastSignalTime: Date.now(),
          lastSignalType: action.signalType,
        },
        log: newLog,
      };

      const { newSkills, updatedLog } = checkNewSkills(baseState);
      return {
        ...baseState,
        skills: [...baseState.skills, ...newSkills],
        log: updatedLog,
      };
    }

    case "add_xp": {
      const newXP = state.progress.xp + action.amount;
      const newStage = resolveStageFromXP(newXP);
      const baseState = {
        ...state,
        progress: { xp: newXP, stage: newStage },
      };
      const { newSkills, updatedLog } = checkNewSkills(baseState);
      return {
        ...baseState,
        skills: [...baseState.skills, ...newSkills],
        log: updatedLog,
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
