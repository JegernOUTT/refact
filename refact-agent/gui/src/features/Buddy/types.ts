import React from "react";

export interface Palette {
  name: string;
  body: string;
  light: string;
  dark: string;
  belly: string;
  eyeDark: string;
  outline: string;
  rosy: string;
  accent: string;
}

export interface Stage {
  name: string;
  emoji: string;
  xpThreshold: number;
  tagline: string;
}

export interface SignalDef {
  mood: MoodType;
  animationType: AnimType;
  xp: number;
  icon: string;
  statusTexts: string[];
  isError: boolean;
  isWin: boolean;
}

export interface SkillDef {
  id: string;
  name: string;
  icon: string;
  xpThreshold: number;
}

export interface ToyDef {
  statusMessage: string;
  xp: number;
  energyRestore?: number;
}

export type EyeStyle =
  | "normal"
  | "star"
  | "heart"
  | "spiral"
  | "teary"
  | "angry"
  | "X"
  | "squint"
  | "uwu";

export type AnimType =
  | "idle"
  | "work"
  | "think"
  | "absorb"
  | "celebrate"
  | "shake"
  | "eat"
  | "sleep"
  | "perk";

export type MoodType =
  | "idle"
  | "working"
  | "focused"
  | "thinking"
  | "learning"
  | "curious"
  | "happy"
  | "celebrate"
  | "concerned"
  | "alert"
  | "eating"
  | "sleepy";

export type IdleActionType =
  | "none"
  | "hover"
  | "curious"
  | "startled"
  | "lookBack"
  | "lookAround"
  | "stretch"
  | "yawn"
  | "tap"
  | "fidget"
  | "walk"
  | "playDuck"
  | "playDice"
  | "drinkCoffee"
  | "playBug"
  | "readScroll"
  | "doze"
  | "confidentPose";

export type ToyType = "duck" | "dice" | "coffee" | "bug" | "scroll";

export type GroundFXType = "impact" | "crack" | "dust";

export type SignalType =
  | "user_message"
  | "chat_started"
  | "chat_completed"
  | "chat_error"
  | "streaming"
  | "generating"
  | "tool_used"
  | "tool_failed"
  | "tool_confirmation"
  | "edit_applied"
  | "search_done"
  | "browser_action"
  | "title_generating"
  | "commit_msg"
  | "memory_extract"
  | "knowledge_update"
  | "indexing"
  | "vecdb_building"
  | "ast_parsing"
  | "compression"
  | "task_created"
  | "task_completed"
  | "task_failed"
  | "checkpoint_saved"
  | "skill_learned"
  | "balance_low"
  | "connection_lost"
  | "connection_restored"
  | "git_changes"
  | "idle_timeout";

export interface MoodStats {
  happiness: number;
  energy: number;
  curiosity: number;
  anxiety: number;
  boredom: number;
  affection: number;
}

export interface PersonalityStats {
  playfulness: number;
  confidence: number;
  clinginess: number;
  resilience: number;
}

export interface LogEntry {
  icon: string;
  message: string;
  timestamp: string;
  xpGained?: string;
}

export interface BuddyActivity {
  mood: MoodType;
  animationType: AnimType;
  lastSignalTime: number;
  lastSignalType: string | null;
}

export interface BuddySemanticState {
  name: string;
  paletteIndex: number;
  born: number;
  mood: MoodStats;
  personality: PersonalityStats;
  progress: {
    xp: number;
    stage: number;
  };
  activity: BuddyActivity;
  skills: string[];
  log: LogEntry[];
}

export interface Spark {
  x: number;
  y: number;
  velocityX: number;
  velocityY: number;
  life: number;
  color: string;
}

export interface FloatingEmoji {
  emoji: string;
  x: number;
  y: number;
  velocityX: number;
  velocityY: number;
  life: number;
}

export interface SleepParticle {
  x: number;
  y: number;
  velocityY: number;
  velocityX: number;
  life: number;
}

export interface OrbitingOrb {
  emoji: string;
  angle: number;
  radius: number;
  speed: number;
  life: number;
}

export interface Afterimage {
  x: number;
  y: number;
  alpha: number;
  life: number;
}

export interface SpeedLine {
  x: number;
  y: number;
  velocityX: number;
  velocityY: number;
  angle: number;
  length: number;
  life: number;
}

export interface GroundFX {
  x: number;
  y: number;
  type: GroundFXType;
  life: number;
  frame: number;
}

export interface ShadowClone {
  x: number;
  y: number;
  alpha: number;
  life: number;
}

export interface ComboState {
  count: number;
  signalType: string | null;
  displayTimer: number;
  rainbowHue: number;
}

export interface SignalHistoryEntry {
  signalType: string;
  timestamp: number;
}

export interface BuddyAnimState {
  frame: number;
  blinkTick: number;
  nextBlinkAt: number;
  blinking: boolean;
  blinkFrames: number;
  bobPhase: number;
  celebrationTimer: number;
  shakeIntensity: number;
  eyeLookX: number;
  eyeLookY: number;
  cursorTargetX: number;
  cursorTargetY: number;
  eyeStyle: EyeStyle;
  eyeStyleTimer: number;
  squashX: number;
  squashY: number;
  squashTargetX: number;
  squashTargetY: number;
  sparks: Spark[];
  floatingEmojis: FloatingEmoji[];
  sleepParticles: SleepParticle[];
  orbitingOrbs: OrbitingOrb[];
  afterimages: Afterimage[];
  speedLines: SpeedLine[];
  groundFX: GroundFX[];
  screenFlash: number;
  screenGlitch: number;
  mouseProximity: number;
  mouseAngle: number;
  mouseOnBuddy: boolean;
  mouseSpeed: number;
  headTilt: number;
  breathScale: number;
  hoverGlow: number;
  nuzzleOffsetX: number;
  nuzzleOffsetY: number;
  mouseNearTimer: number;
  dragging: boolean;
  petCount: number;
  idleAction: IdleActionType;
  idleActionTimer: number;
  earState: number;
  earAnimProgress: number;
  errorStreak: number;
  successStreak: number;
  heat: number;
  combo: ComboState;
  signalHistory: SignalHistoryEntry[];
  stageQuirkTick: number;
  quirkActive: boolean;
  quirkType: string;
  phaseAlpha: number;
  shadowClone: ShadowClone | null;
  levitationOffset: number;
  auraPulseIntensity: number;
  walkOffsetX: number;
  walkTargetX: number;
  walkDirection: number;
  walkSpeed: number;
  walking: boolean;
  walkPhase: number;
  toyActive: boolean;
  toyType: ToyType | null;
  toyAnimPhase: number;
  toyDurationTimer: number;
  statusText: string;
  statusOpacity: number;
  statusTargetOpacity: number;
}

export interface ColorMap {
  body: string;
  light: string;
  dark: string;
  belly: string;
  outline: string;
  eyeDark: string;
  black: string;
  white: string;
  rosy: string;
  accent: string;
  green: string;
  gold: string;
}

export type BuddyEvent =
  | { type: "xp_gained"; amount: number; newTotal: number }
  | { type: "stage_evolved"; stage: number; name: string }
  | { type: "skill_unlocked"; skillId: string; skillName: string }
  | { type: "semantic_update"; patch: Partial<BuddySemanticState> };

export interface BuddyCanvasProps {
  state: BuddySemanticState;
  onEvent?: (event: BuddyEvent) => void;
  width?: number;
  height?: number;
  className?: string;
  style?: React.CSSProperties;
}
