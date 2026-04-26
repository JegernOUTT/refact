export { BuddyCanvas } from "./BuddyCanvas";
export { useBuddyState } from "./hooks/useBuddyState";
export {
  createInitialSemanticState,
  createInitialAnimState,
  reduceSemanticState,
} from "./state";
export {
  SIGNALS,
  STAGES,
  PALETTES,
  NAMES,
  SKILLS,
  TOY_DEFS,
} from "./constants";
export type {
  BuddySemanticState,
  BuddyAnimState,
  BuddyActivity,
  BuddyEvent,
  BuddyCanvasProps,
  MoodStats,
  PersonalityStats,
  LogEntry,
  SignalType,
  EyeStyle,
  AnimType,
  MoodType,
  IdleActionType,
  ToyType,
  Palette,
  Stage,
  SignalDef,
  SkillDef,
  ToyDef,
} from "./types";
export type { BuddyStateHandle } from "./hooks/useBuddyState";
export type { SemanticAction } from "./state";
