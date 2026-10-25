import { normalizeToolName } from "../../../utils/toolNameAliases";

export type AgenticFamily =
  | "spawn"
  | "message"
  | "overview"
  | "progress"
  | "inspect"
  | "control"
  | "goal"
  | "plan"
  | "task"
  | "doc"
  | "memory";

export type AgenticIconKey =
  | "spawn"
  | "message"
  | "overview"
  | "progress"
  | "inspect"
  | "control"
  | "goal"
  | "plan"
  | "task"
  | "doc"
  | "memory";

export type AgenticFamilyDescriptor = {
  family: AgenticFamily;
  icon: AgenticIconKey;
  label: string;
};

const LOCAL_COLLABORATION_FAMILIES: Record<string, AgenticFamilyDescriptor> = {
  agent_mailbox: { family: "inspect", icon: "inspect", label: "Check mailbox" },
  board_claim: { family: "task", icon: "task", label: "Claim card" },
  board_discover: { family: "task", icon: "task", label: "Discover board" },
  team_seat: { family: "task", icon: "task", label: "Team seat" },
  team_role_grant: {
    family: "control",
    icon: "control",
    label: "Team role authority",
  },
  finalize_card: { family: "task", icon: "task", label: "Finalize card" },
  claim_record: { family: "memory", icon: "memory", label: "Record claim" },
  claim_list: { family: "memory", icon: "memory", label: "Read claims" },
  disagreement_file: {
    family: "task",
    icon: "task",
    label: "File disagreement",
  },
  disagreement_list: {
    family: "task",
    icon: "task",
    label: "Read disagreements",
  },
  disagreement_adjudicate: {
    family: "task",
    icon: "task",
    label: "Adjudicate disagreement",
  },
  causal_trace: { family: "inspect", icon: "inspect", label: "Trace causes" },
  metrics: { family: "overview", icon: "overview", label: "Read metrics" },
  code_mode: {
    family: "control",
    icon: "control",
    label: "Run orchestration",
  },
};

export const LOCAL_COLLABORATION_TOOL_NAMES: readonly string[] = Object.keys(
  LOCAL_COLLABORATION_FAMILIES,
);

const FAMILY_TABLE: Partial<Record<string, AgenticFamilyDescriptor>> = {
  spawn_agent: { family: "spawn", icon: "spawn", label: "Spawn agent" },
  task_spawn_agent: { family: "spawn", icon: "spawn", label: "Spawn agent" },
  spawn_ab: { family: "spawn", icon: "spawn", label: "Spawn A/B agents" },
  agent_message: { family: "message", icon: "message", label: "Agent message" },
  agent_peer_grant: {
    family: "message",
    icon: "message",
    label: "Peer authority",
  },
  task_broadcast: { family: "message", icon: "message", label: "Broadcast" },
  agent_steer: { family: "message", icon: "message", label: "Steer agent" },
  agents_overview: {
    family: "overview",
    icon: "overview",
    label: "Agents overview",
  },
  agent_list: { family: "overview", icon: "overview", label: "Agent list" },
  work_overview: {
    family: "overview",
    icon: "overview",
    label: "Work overview",
  },
  progress_report: {
    family: "progress",
    icon: "progress",
    label: "Progress report",
  },
  agent_status: { family: "inspect", icon: "inspect", label: "Agent status" },
  agent_wait: { family: "inspect", icon: "inspect", label: "Agent wait" },
  agent_result: { family: "inspect", icon: "inspect", label: "Agent result" },
  work_inspect: { family: "inspect", icon: "inspect", label: "Work inspect" },
  agent_cancel: { family: "control", icon: "control", label: "Agent cancel" },
  agent_approval: {
    family: "control",
    icon: "control",
    label: "Agent approval",
  },
  agent_resume: { family: "control", icon: "control", label: "Agent resume" },
  pause_goal: { family: "control", icon: "control", label: "Pause goal" },
  snooze_goal: { family: "control", icon: "control", label: "Snooze goal" },
  set_goal: { family: "goal", icon: "goal", label: "Set goal" },
  update_goal: { family: "goal", icon: "goal", label: "Update goal" },
  get_goal: { family: "goal", icon: "goal", label: "Get goal" },
  validate_goal: { family: "goal", icon: "goal", label: "Validate goal" },
  set_plan: { family: "plan", icon: "plan", label: "Set plan" },
  update_plan: { family: "plan", icon: "plan", label: "Update plan" },
  get_plan: { family: "plan", icon: "plan", label: "Get plan" },
  task_state: { family: "task", icon: "task", label: "Task state" },
  board_get: { family: "task", icon: "task", label: "Task board" },
  board_create: { family: "task", icon: "task", label: "Create card" },
  board_update: { family: "task", icon: "task", label: "Update card" },
  board_move: { family: "task", icon: "task", label: "Move card" },
  board_delete: { family: "task", icon: "task", label: "Delete card" },
  ready_cards: { family: "task", icon: "task", label: "Ready cards" },
  task_board_authority: {
    family: "control",
    icon: "control",
    label: "Board authority",
  },
  mark_done: { family: "task", icon: "task", label: "Mark card done" },
  mark_failed: { family: "task", icon: "task", label: "Mark card failed" },
  spawn_agents_batch: {
    family: "spawn",
    icon: "spawn",
    label: "Spawn agents batch",
  },
  wait_agents: { family: "inspect", icon: "inspect", label: "Wait for agents" },
  agent_chat_summary: {
    family: "inspect",
    icon: "inspect",
    label: "Agent chat summary",
  },
  task_overview: {
    family: "overview",
    icon: "overview",
    label: "Task overview",
  },
  task_questions_list: {
    family: "overview",
    icon: "overview",
    label: "Task questions",
  },
  agent_ask_planner: {
    family: "message",
    icon: "message",
    label: "Ask planner",
  },
  planner_reply: { family: "message", icon: "message", label: "Planner reply" },
  agent_update: { family: "progress", icon: "progress", label: "Agent update" },
  restart_agent: { family: "control", icon: "control", label: "Restart agent" },
  resume_agent: { family: "control", icon: "control", label: "Resume agent" },
  pause_agent: { family: "control", icon: "control", label: "Pause agent" },
  cancel_agent: { family: "control", icon: "control", label: "Cancel agent" },
  assign_agent: { family: "control", icon: "control", label: "Assign agent" },
  merge_agent: { family: "control", icon: "control", label: "Merge agent" },
  merge_ready_in_order: {
    family: "control",
    icon: "control",
    label: "Merge ready cards",
  },
  pick_ab_winner: {
    family: "control",
    icon: "control",
    label: "Pick A/B winner",
  },
  agent_complete: { family: "task", icon: "task", label: "Agent complete" },
  agent_fail: { family: "task", icon: "task", label: "Agent fail" },
  board_create_batch: {
    family: "task",
    icon: "task",
    label: "Create cards",
  },
  mark_done_batch: { family: "task", icon: "task", label: "Mark cards done" },
  mark_failed_batch: {
    family: "task",
    icon: "task",
    label: "Mark cards failed",
  },
  task_start: { family: "task", icon: "task", label: "Start task" },
  task_verify_card: { family: "task", icon: "task", label: "Verify card" },
  doc_create: { family: "doc", icon: "doc", label: "Create task document" },
  doc_update: { family: "doc", icon: "doc", label: "Update task document" },
  doc_append: { family: "doc", icon: "doc", label: "Append task document" },
  doc_delete: { family: "doc", icon: "doc", label: "Delete task document" },
  doc_pin: { family: "doc", icon: "doc", label: "Pin task document" },
  doc_history: { family: "doc", icon: "doc", label: "Task document history" },
  task_mem_save: {
    family: "memory",
    icon: "memory",
    label: "Save task memory",
  },
  task_mem_get: { family: "memory", icon: "memory", label: "Get task memory" },
  task_mem_search: {
    family: "memory",
    icon: "memory",
    label: "Search task memory",
  },
  task_mem_inbox: {
    family: "memory",
    icon: "memory",
    label: "Task memory inbox",
  },
  task_mem_pin: { family: "memory", icon: "memory", label: "Pin task memory" },
  task_mem_archive: {
    family: "memory",
    icon: "memory",
    label: "Archive task memory",
  },
  task_mem_unarchive: {
    family: "memory",
    icon: "memory",
    label: "Unarchive task memory",
  },
  task_mem_triage_done: {
    family: "memory",
    icon: "memory",
    label: "Task memory triaged",
  },
  ...LOCAL_COLLABORATION_FAMILIES,
};

/// Board-runtime tool names that must always resolve to a dedicated family
/// card rather than generic raw output.
export const BOARD_RUNTIME_TOOL_NAMES: readonly string[] = [
  "spawn_agent",
  "spawn_ab",
  "spawn_agents_batch",
  "check_agents",
  "wait_agents",
  "restart_agent",
  "resume_agent",
  "pause_agent",
  "cancel_agent",
  "assign_agent",
  "merge_agent",
  "merge_ready_in_order",
  "pick_ab_winner",
  "task_broadcast",
  "agent_steer",
  "agent_ask_planner",
  "planner_reply",
  "agent_update",
  "agent_complete",
  "agent_fail",
  "agent_chat_summary",
  "task_overview",
  "task_questions_list",
  "task_start",
  "task_verify_card",
  "board_get",
  "board_create",
  "board_create_batch",
  "board_update",
  "board_move",
  "board_delete",
  "ready_cards",
  "board_claim",
  "board_discover",
  "finalize_card",
  "task_board_authority",
  "mark_done",
  "mark_done_batch",
  "mark_failed",
  "mark_failed_batch",
  "doc_create",
  "doc_update",
  "doc_append",
  "doc_delete",
  "doc_pin",
  "doc_history",
  "task_mem_save",
  "task_mem_get",
  "task_mem_search",
  "task_mem_inbox",
  "task_mem_pin",
  "task_mem_archive",
  "task_mem_unarchive",
  "task_mem_triage_done",
];

export const AGENTIC_TOOL_NAMES: readonly string[] = Object.keys(FAMILY_TABLE);

/// Tool names that ToolsContent routes to their own purpose-built card before
/// the agentic family lookup runs.
export const DEDICATED_TOOL_CARD_NAMES: readonly string[] = [
  "subagent",
  "delegate",
  "check_agents",
  "agent_pulse",
  "agent_diff",
  "doc_list",
  "doc_get",
  "agent_finish",
  "task_done",
];

/// Accessible card label rendered for each agentic family. Every family must
/// resolve to a dedicated card here; the inventory test fails otherwise.
export const AGENTIC_CARD_LABELS: Record<AgenticFamily, string> = {
  spawn: "Agent spawn receipt",
  message: "Agent message receipt",
  overview: "Agents overview",
  progress: "Progress receipt",
  inspect: "Agent inspection",
  control: "Agent control receipt",
  goal: "Goal receipt",
  plan: "Plan receipt",
  task: "Task board receipt",
  doc: "Task document receipt",
  memory: "Task memory receipt",
};

/// Tool names whose dedicated card differs from their family default.
const TOOL_CARD_LABELS: Partial<Record<string, string>> = {
  agent_update: "Agent message receipt",
  work_overview: "Work snapshot",
  work_inspect: "Work snapshot",
  agent_approval: "Child command approval",
  agent_resume: "Child resume",
  pause_goal: "Goal receipt",
  snooze_goal: "Goal receipt",
};

export function agenticCardLabel(toolName: string | undefined): string | null {
  if (!toolName) return null;
  const normalized = normalizeToolName(toolName);
  if (!normalized) return null;
  const override = TOOL_CARD_LABELS[normalized];
  if (override !== undefined) return override;
  const descriptor = FAMILY_TABLE[normalized];
  return descriptor ? AGENTIC_CARD_LABELS[descriptor.family] : null;
}

export function lookupAgenticFamily(
  toolName: string | undefined,
): AgenticFamilyDescriptor | null {
  if (!toolName) return null;
  const normalized = normalizeToolName(toolName);
  if (!normalized) return null;
  return FAMILY_TABLE[normalized] ?? null;
}

export function isAgenticToolName(toolName: string | undefined): boolean {
  return lookupAgenticFamily(toolName) !== null;
}

export function hasDedicatedToolCard(toolName: string | undefined): boolean {
  if (!toolName) return false;
  const normalized = normalizeToolName(toolName) ?? toolName;
  return (
    isAgenticToolName(normalized) ||
    DEDICATED_TOOL_CARD_NAMES.includes(normalized)
  );
}
