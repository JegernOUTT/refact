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
  isError: boolean;
  isWin: boolean;
  scene?: string;
  category?: "transient" | "active" | "speech";
  duration?: number;
  animVariant?: string;
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
  | "confidentPose"
  | "wave"
  | "spin"
  | "type_code"
  | "scratch"
  | "peekAround"
  | "sniff";

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
  | "care_feed"
  | "care_play"
  | "care_pet"
  | "care_sleep"
  | "care_clean"
  | "idle_timeout"
  | "stage_up";

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
  chaos: number;
  sociability: number;
  curiosity: number;
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

export interface BuddyControl {
  id: string;
  label: string;
  action: string;
  action_param?: string;
  style: string;
}

export interface BuddySpeechItem {
  id: string;
  text: string;
  mood: string;
  scope: string;
  persistent: boolean;
  ttl_seconds: number;
  dedupe_key?: string;
  created_at: string;
  controls: BuddyControl[];
  chat_id?: string;
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
  quirkEndFrame: number;
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
  moodType: MoodType;
  statusText: string;
  statusOpacity: number;
  statusTargetOpacity: number;
  statusTimer: number;
  activeScene: string;
  activeSceneVariant: string;
  activeSceneTimer: number;
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
  | { type: "petted" }
  | { type: "semantic_update"; patch: Partial<BuddySemanticState> };

export type BubblePosition = "above" | "left" | "right";

export interface BuddyCanvasProps {
  state: BuddySemanticState;
  onEvent?: (event: BuddyEvent) => void;
  displaySize?: number;
  className?: string;
  style?: React.CSSProperties;
  /** Override speech bubble text (from runtime/backend), takes priority over canvas statusText */
  speechOverride?: string | null;
  /** Buttons rendered inside the speech bubble */
  speechControls?: BuddyControl[];
  /** Called when a speech bubble button is clicked */
  onSpeechControlClick?: (ctrl: BuddyControl) => void;
  /** Where to position the speech bubble relative to the buddy. Default: "above" */
  bubblePosition?: BubblePosition;
}

export interface BuddyIdentity {
  name: string;
  created_at: string;
  palette_index: number;
}

export interface BuddyProgression {
  stage: number;
  stage_name: string;
  level: number;
  xp: number;
  xp_next: number;
}

export interface BuddyNeeds {
  hunger: number;
  energy: number;
  hygiene: number;
  boredom: number;
  affection: number;
}

export interface BuddyCondition {
  sleeping: boolean;
  hungry: boolean;
  sleepy: boolean;
  dirty: boolean;
  bored: boolean;
  lonely: boolean;
}

export interface BuddyEvolutionState {
  care_score: number;
  neglect_score: number;
  open_seconds: number;
  last_evolved_at: string | null;
}

export interface BuddyPetState {
  needs: BuddyNeeds;
  condition: BuddyCondition;
  evolution: BuddyEvolutionState;
}

export interface BuddyPersonalityTraits {
  playfulness: number;
  chaos: number;
  sociability: number;
  curiosity: number;
  resilience: number;
}

export interface BuddyPersonalityProfile {
  archetype_id: string;
  archetype_label: string;
  vibe: string;
  summary: string;
  prompt: string;
  traits: BuddyPersonalityTraits;
}

export interface BuddySkillLedger {
  unlocked: string[];
  locked: string[];
}

export interface BuddyWorkflowSummary {
  workflow_id: string;
  last_run: string | null;
  run_count: number;
  last_outcome: string | null;
}

export interface BuddySemanticSnapshot {
  mood: string;
  focus: string;
  headline: string;
  last_active: string;
}

export interface BuddyActivityEntry {
  icon: string;
  title: string;
  description: string;
  timestamp: string;
  activity_type: string;
}

export interface BuddySuggestion {
  id: string;
  suggestion_type: string;
  title: string;
  description: string;
  created_at: string;
  dismissed: boolean;
  controls: BuddyControl[];
  quest?: BuddyQuest | null;
}

export interface BuddyQuest {
  id: string;
  quest_type: string;
  title: string;
  description: string;
  icon: string;
  created_at: string;
  accepted_at: string;
  status: string;
  completed_at?: string | null;
  progress: number;
  goal: number;
  baseline: number;
  reward_xp: number;
  controls: BuddyControl[];
}

export interface BuddyState {
  identity: BuddyIdentity;
  progression: BuddyProgression;
  skills: BuddySkillLedger;
  workflow_summaries: BuddyWorkflowSummary[];
  semantic: BuddySemanticSnapshot;
  recent_activities: BuddyActivityEntry[];
  suggestion_state: BuddySuggestion[];
  pet: BuddyPetState;
  personality: BuddyPersonalityProfile;
  active_quest?: BuddyQuest | null;
}

export interface BuddySettings {
  enabled: boolean;
  auto_diagnostics: boolean;
  auto_issue_creation: boolean;
  personality_prompt: string | null;
  proactive_enabled: boolean;
}

export interface BuddySnapshot {
  state: BuddyState;
  settings: BuddySettings;
  enabled: boolean;
  recent_diagnostics?: DiagnosticContext[];
  active_speech?: BuddySpeechItem | null;
  runtime_queue?: BuddyRuntimeEvent[];
  now_playing?: BuddyRuntimeEvent | null;
}

export type BuddyCareAction = "feed" | "play" | "pet" | "sleep" | "clean";

export interface BuddyCareRequest {
  action: BuddyCareAction;
  toy?: string;
}

export interface BuddyCareResponse {
  message: string;
  snapshot: BuddySnapshot;
}

export interface BuddyPersonalityRerollResponse {
  snapshot: BuddySnapshot;
}

export interface BuddyQuestAcceptResponse {
  snapshot: BuddySnapshot;
  suggestion: BuddySuggestion;
}

export interface BuddyConversationMeta {
  chat_id: string;
  title: string;
  created_at: string;
  last_message_at: string | null;
  message_count: number;
}

export interface BuddyConversationEntry {
  id: string;
  kind: "chat" | "setup" | "workflow" | "system";
  title: string;
  created_at: string;
  updated_at: string;
  status: "active" | "completed" | "failed";
  message_count: number;
  icon: string;
  badge: string | null;
}

export interface BuddyThreadMeta {
  is_buddy_chat: boolean;
  buddy_chat_kind: string;
  workflow_id: string | null;
}

export interface DiagnosticContext {
  error_type: string;
  error_message: string;
  source_file: string | null;
  tool_name: string | null;
  chat_id: string | null;
  diagnostic_id?: string;
  collected_at: string;
  severity: "low" | "medium" | "high" | "critical";
}

export interface BuddyRuntimeEvent {
  id: string;
  signal_type: string;
  title: string;
  description?: string;
  source: string;
  status:
    | "started"
    | "progress"
    | "completed"
    | "failed"
    | "info"
    | "streaming";
  progress?: number;
  dedupe_key?: string;
  priority: string;
  created_at: string;
  ttl_ms?: number;
  speech_text?: string;
  scene?: string;
  duration_hint?: number;
  persistent?: boolean;
  controls?: BuddyControl[];
  chat_id?: string;
  /** True when the user has explicitly dismissed this event. Persisted server-side. */
  dismissed?: boolean;
}

export type BuddySSEEvent =
  | { event_type: "StateUpdated"; state: BuddyState }
  | { event_type: "ActivityAdded"; activity: BuddyActivityEntry }
  | { event_type: "SuggestionAdded"; suggestion: BuddySuggestion }
  | { event_type: "SuggestionDismissed"; suggestion_id: string }
  | { event_type: "SettingsChanged"; settings: BuddySettings }
  | { event_type: "DiagnosticAdded"; diagnostic: DiagnosticContext }
  | { event_type: "RuntimeEvent"; event: BuddyRuntimeEvent }
  | { event_type: "SpeechUpdated"; speech: BuddySpeechItem }
  | { event_type: "NavigationRequest"; view: string; params?: unknown };
