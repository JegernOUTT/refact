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
  | "connection_lost"
  | "connection_restored"
  | "git_changes"
  | "care_feed"
  | "care_play"
  | "care_pet"
  | "care_sleep"
  | "care_clean"
  | "idle_timeout"
  | "stage_up"
  | "error";

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

export type BubblePosition = "top" | "left" | "right";

export type BuddyScenePose =
  | "idle"
  | "spin"
  | "bounce"
  | "look"
  | "stargaze"
  | "meditate"
  | "pounce"
  | "dance"
  | "shield"
  | "cheer"
  | "carry"
  | "dig"
  | "sleepy";

export type BuddyShowcaseKind =
  | "memory_firefly_night"
  | "stargazing_constellation";

export type BuddyShowcasePhase =
  | "travel"
  | "anticipate"
  | "showcase"
  | "react"
  | "cooldown";

export interface BuddyShowcaseTarget {
  id: string;
  x: number;
  y: number;
  label: string;
}

export interface BuddyShowcaseRun {
  id: string;
  kind: BuddyShowcaseKind;
  phase: BuddyShowcasePhase;
  target: BuddyShowcaseTarget;
  pose: BuddyScenePose;
  speech: string;
  seed: number;
  startedAtMs: number;
  phaseStartedAtMs: number;
}

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
  /** Where to position the speech bubble relative to the buddy. Default: "top" */
  bubblePosition?: BubblePosition;
  /** If true, each new saying picks top, left, or right at random. */
  randomizeBubblePosition?: boolean;
  compactBubble?: boolean;
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
  chat_id?: string | null;
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
  opportunities: BuddyOpportunity[];
}

export type HumorLevel = "off" | "light" | "normal";

export type AutonomyLevel = "read_only" | "suggest" | "safe_auto";

export interface ObserverToggles {
  task_health: boolean;
  trajectory_clutter: boolean;
  chat_pattern: boolean;
  customization_drift: boolean;
  memory_garden: boolean;
  mcp_auth: boolean;
  git_pressure: boolean;
  diagnostic_cluster: boolean;
  provider_health: boolean;
}

export type BuddyFactKind =
  | "task_stuck"
  | "task_abandoned"
  | "task_cluster_duplicate"
  | "trajectory_clutter"
  | "chat_retry_streak"
  | "memory_orphan"
  | "memory_stale_conflict"
  | "memory_recurring_lesson"
  | "mode_prompt_overlap"
  | "skill_trigger_weak"
  | "agents_md_gap_detected"
  | "default_model_missing"
  | "broken_model_reference"
  | "mcp_auth_expired"
  | "integration_failing"
  | "diagnostic_cluster"
  | "frontend_error_burst"
  | "git_diff_widening"
  | "uncommitted_pressure"
  | "worktree_hygiene";

export interface BuddyFact {
  kind: BuddyFactKind;
  key: string;
  source: string;
  payload: unknown;
  seen_at: string;
  confidence: number;
}

export type BuddyOpportunityKind =
  | "task_health"
  | "trajectory_cleanup"
  | "chat_recap"
  | "memory_garden"
  | "config_drift"
  | "agents_md_gap"
  | "provider_tuning"
  | "integration_fix"
  | "diagnostic_investigation"
  | "git_hygiene"
  | "worktree_cleanup";

export type BuddyPriority = "low" | "normal" | "high" | "critical";

export type OpportunityStatus =
  | "new"
  | "shown"
  | "dismissed"
  | "accepted"
  | "completed"
  | "expired";

export type DefaultsKind =
  | "chat_model"
  | "chat_light_model"
  | "chat_buddy_model"
  | "chat_thinking_model";

export type CustomizationKind =
  | "mode"
  | "skill"
  | "command"
  | "delegate"
  | "hook";

export type MarketKind = "mcp" | "skill" | "command" | "delegate";

export type PulseScope =
  | "all"
  | "tasks"
  | "trajectories"
  | "memory"
  | "providers"
  | "mcp"
  | "customization"
  | "diagnostics"
  | "git"
  | "worktrees";

export type BuddyPage =
  | { type: "buddy" }
  | { type: "stats" }
  | { type: "customization" }
  | { type: "providers" }
  | { type: "default_models" }
  | { type: "integrations" }
  | { type: "extensions" }
  | { type: "marketplace_hub" }
  | { type: "marketplace" }
  | { type: "skills_marketplace" }
  | { type: "commands_marketplace" }
  | { type: "delegates_marketplace" }
  | { type: "tasks_list" }
  | { type: "task_workspace"; task_id: string }
  | { type: "knowledge_graph" }
  | { type: "worktrees" }
  | { type: "setup_mode"; mode: string };

export interface InvestigationContext {
  fact_keys: string[];
  diagnostic_ids: string[];
  log_excerpt: string;
  config_summary: string;
  initial_user_message: string;
}

export type BuddyAction =
  | { kind: "open_page"; page: BuddyPage }
  | { kind: "launch_investigation_chat"; preload: InvestigationContext }
  | { kind: "draft_skill"; draft_id: string; label: string }
  | { kind: "draft_command"; draft_id: string; label: string }
  | { kind: "draft_delegate"; draft_id: string; label: string }
  | { kind: "draft_mode"; draft_id: string; label: string }
  | { kind: "draft_agents_md_patch"; content: string }
  | {
      kind: "draft_defaults_change";
      defaults_kind: DefaultsKind;
      patch: unknown;
    }
  | {
      kind: "draft_customization_change";
      customization_kind: CustomizationKind;
      id: string;
      patch: unknown;
    }
  | {
      kind: "offer_marketplace_install";
      market_kind: MarketKind;
      item_id: string;
    }
  | { kind: "create_pulse_report"; scope: PulseScope }
  | { kind: "dismiss" };

export interface BuddyOpportunityLinks {
  chat_ids: string[];
  task_ids: string[];
  memory_ids: string[];
  config_paths: string[];
  page?: BuddyPage | null;
}

export interface BuddyOpportunity {
  id: string;
  kind: BuddyOpportunityKind;
  summary: string;
  priority: BuddyPriority;
  confidence: number;
  fact_keys: string[];
  cooldown_key: string;
  cooldown_secs: number;
  status: OpportunityStatus;
  proposed_actions: BuddyAction[];
  humor?: string | null;
  humor_allowed: boolean;
  related: BuddyOpportunityLinks;
  created_at: string;
  expires_at: string;
  resolved_at?: string | null;
}

export type BuddyActionResult =
  | { kind: "open_page"; navigate_to: BuddyPage }
  | { kind: "launch_investigation_chat"; chat_id: string }
  | {
      kind: "draft";
      draft_kind: DraftKind;
      draft_id: string;
      label?: string;
      defaults_kind?: DefaultsKind;
    }
  | { kind: "dismiss" }
  | {
      kind: "marketplace_install";
      market_kind: MarketKind;
      item_id: string;
      success?: boolean;
      error?: string | null;
    };

export interface BuddyOpportunityAcceptResponse {
  snapshot: BuddySnapshot;
  action_result: BuddyActionResult;
}

export interface TaskPulse {
  total: number;
  stuck: number;
  abandoned: number;
  by_status: Record<string, number>;
}

export interface TrajectoryPulse {
  total: number;
  untitled: number;
  oldest_age_days: number;
}

export interface MemoryPulse {
  total: number;
  orphan: number;
  stale_conflicts: number;
}

export interface ProviderPulse {
  defaults_ok: boolean;
  broken_refs: number;
  quota_warnings: number;
}

export interface McpPulse {
  total: number;
  failing: number;
  auth_expiring: number;
}

export interface CustomizationPulse {
  modes: number;
  skills: number;
  commands: number;
  subagents: number;
  hooks: number;
}

export interface DiagnosticPulse {
  last_hour: number;
  top_error_types: string[];
}

export interface GitPulse {
  uncommitted_files: number;
  diff_lines_4h: number;
  branches: number;
}

export interface WorktreePulse {
  total_registered: number;
  total_discovered: number;
  total: number;
  clean: number;
  dirty: number;
  unknown: number;
  stale: number;
  conflicted: number;
  shared: number;
  abandoned_clean: number;
  changed_files: number;
  additions: number;
  deletions: number;
  missing_registry_paths: number;
  unregistered_cache_dirs: number;
  merged_branches: number;
  newest_age_hours?: number | null;
  oldest_age_hours?: number | null;
  disk_usage_bytes?: number | null;
}

export interface BuddyPulse {
  generated_at?: string | null;
  tasks: TaskPulse;
  trajectories: TrajectoryPulse;
  memory: MemoryPulse;
  providers: ProviderPulse;
  mcp: McpPulse;
  customization: CustomizationPulse;
  diagnostics: DiagnosticPulse;
  git: GitPulse;
  worktrees: WorktreePulse;
  humor?: string | null;
}

export type DraftKind =
  | "skill"
  | "command"
  | "delegate"
  | "mode"
  | "agents_md"
  | "defaults_model"
  | "hook"
  | "pulse_report";

export interface BuddyDraft {
  id: string;
  kind: DraftKind;
  title: string;
  yaml_or_json: string;
  explanation: string;
  created_at: string;
  expires_at: string;
}

export interface BuddySettings {
  enabled: boolean;
  auto_diagnostics: boolean;
  auto_issue_creation: boolean;
  personality_prompt: string | null;
  proactive_enabled: boolean;
  message_observation_enabled: boolean;
  housekeeping_enabled: boolean;
  humor_enabled: boolean;
  humor_level: HumorLevel;
  autonomy_level: AutonomyLevel;
  quiet_mode: boolean;
  observers: ObserverToggles;
}

export interface BuddySnapshot {
  state: BuddyState;
  settings: BuddySettings;
  enabled: boolean;
  recent_diagnostics?: DiagnosticContext[];
  active_speech?: BuddySpeechItem | null;
  runtime_queue?: BuddyRuntimeEvent[];
  now_playing?: BuddyRuntimeEvent | null;
  pulse?: BuddyPulse | null;
  opportunities?: BuddyOpportunity[];
  active_drafts?: BuddyDraft[];
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
  description?: string | null;
  source: string;
  status:
    | "started"
    | "progress"
    | "completed"
    | "failed"
    | "info"
    | "streaming";
  progress?: number | null;
  dedupe_key?: string | null;
  priority: string;
  created_at: string;
  ttl_ms?: number | null;
  speech_text?: string | null;
  scene?: string | null;
  duration_hint?: number | null;
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
  | { event_type: "NavigationRequest"; page: BuddyPage }
  | { event_type: "OpportunityProduced"; opportunity: BuddyOpportunity }
  | {
      event_type: "OpportunityResolved";
      opportunity_id: string;
      status: OpportunityStatus;
    }
  | { event_type: "PulseUpdated"; pulse: BuddyPulse }
  | { event_type: "DraftCreated"; draft: BuddyDraft }
  | { event_type: "DraftConsumed"; draft_id: string }
  | { event_type: "DraftRemoved"; draft_id: string };
