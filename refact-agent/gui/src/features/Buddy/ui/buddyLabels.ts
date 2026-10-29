import { humanizeIdentifier as humanizeWords } from "../../../utils/displayNames";

export const BUDDY_FAILURE_LABELS: Partial<Record<string, string>> = {
  llm_error: "Model error",
  model_error: "Model error",
  provider_error: "Provider error",
  tool_error: "Tool error",
  tool_failed: "Tool error",
  mcp_tool_error: "MCP tool error",
  browser_error: "Browser error",
  network_error: "Network error",
  connection_lost: "Connection lost",
  frontend_error: "Interface error",
  chat_error: "Chat error",
  task_failed: "Task failed",
  mode_config: "Mode setup",
  generic: "Other",
  unknown: "Other",
  error: "Error",
  model_unavailable: "Model unavailable",
  context_too_large: "Context too large",
  tool_unavailable: "Tool unavailable",
  invalid_request: "Invalid request",
  provider_transient: "Provider hiccup",
  provider_rate_limit: "Rate limited",
  authentication_failed: "Sign-in failed",
  billing_quota: "Quota reached",
  content_policy: "Blocked by content policy",
  cancelled: "Cancelled",
  network: "Network error",
  timeout: "Timed out",
  workflow_failed: "Workflow failed",
};

export const BUDDY_WORKFLOW_LABELS: Partial<Record<string, string>> = {
  commit_msg: "Commit messages",
  commit_message: "Commit messages",
  follow_up: "Follow-up suggestions",
  compression: "Chat compression",
  compress_trajectory: "Chat compression",
  memory_extract: "Memo extraction",
  memo_extraction: "Memo extraction",
  knowledge_update: "Knowledge updates",
  kg_enrich: "Knowledge enrichment",
  kg_deprecate: "Knowledge cleanup",
  title_generating: "Chat titles",
  title_generation: "Chat titles",
  refact_error_detective: "Error detective",
  refact_self_critic: "Self critic",
  refact_compile_sniffer: "Compile sniffer",
  buddy_daily_digest: "Daily digest",
  buddy_friday_retro: "Friday retro",
  buddy_idle_suggester: "Idle suggestions",
  buddy_onboarding: "Onboarding",
  buddy_pr_issue_matchmaker: "PR and issue matchmaker",
  buddy_refactor_hunter: "Refactor hunter",
  buddy_skill_author: "Skill author",
  buddy_test_coverage_watcher: "Test coverage watcher",
  buddy_security_whisperer: "Security whisperer",
  buddy_setup_coach: "Setup coach",
  buddy_dependency_radar: "Dependency radar",
  buddy_docs_gardener: "Docs gardener",
  buddy_architecture_drift_watcher: "Architecture drift watcher",
  buddy_memory_gardener: "Memory gardener",
  buddy_knowledge_conflict_resolver: "Knowledge conflict resolver",
  buddy_behavior_learner: "Behavior learner",
  buddy_user_habit_coach: "Habit coach",
  buddy_model_cost_optimizer: "Model cost optimizer",
  buddy_voice: "Speech lines",
};

export const BUDDY_RULE_LABELS: Partial<Record<string, string>> = {
  "task_health:stuck": "Stuck tasks",
  "task_health:abandoned": "Abandoned tasks",
  "task_health:cluster": "Overlapping tasks",
  "task:stuck": "Stuck tasks",
  "task:abandoned": "Abandoned tasks",
  "task_memory:orphan": "Orphaned task memories",
  "trajectory:cleanup": "Chat cleanup",
  "trajectory:clutter": "Chat clutter",
  "provider:default_model_missing": "Missing default model",
  "provider:default_missing": "Missing default model",
  "provider:broken_ref": "Broken model references",
  "memory:garden": "Memory garden",
  "memory:conflict": "Conflicting memories",
  "memory:recurring": "Recurring memories",
  "diag:cluster": "Repeated errors",
  "diag:signal": "Error signals",
  "config_drift:mode_overlap": "Overlapping modes",
  "config_drift:skill_trigger": "Weak skill triggers",
  "customization:mode_overlap": "Overlapping modes",
  "customization:skill_trigger_weak": "Weak skill triggers",
  "customization:agents_md_gap": "AGENTS.md gaps",
  "integration:mcp_auth": "MCP sign-in",
  "integration:failing": "Failing integrations",
  "mcp:auth_expiring": "MCP sign-in expiring",
  "chat_recap:retry": "Chat retries",
  "chat:retry_streak": "Chat retry streaks",
  "completion:failed": "Failed completions",
  "completion:partial": "Partial completions",
  "git:pressure": "Uncommitted changes",
  "git:widening": "Growing diffs",
  memory_ops_batch: "Memory batches",
  "memory_ops_batch:merge_exact_duplicate": "Exact duplicate memories",
  "memory_ops_batch:merge_near_duplicate": "Near-duplicate memories",
  "memory_ops_batch:archive": "Memory archiving",
  "memory_ops_batch:review": "Memory reviews",
  "memory_ops_batch:delete": "Memory deletion",
  "memory_ops_batch:create": "Memory creation",
  "memory_ops_batch:digest": "Memory digests",
  "memory_ops_batch:maintenance": "Memory maintenance",
  buddy_memory_garden: "Memory garden",
  "quest:start_setup": "Setup reminder",
};

export const BUDDY_TRAIT_LABELS: Partial<Record<string, string>> = {
  playfulness: "Playfulness",
  chaos: "Chaos",
  sociability: "Sociability",
  curiosity: "Curiosity",
  resilience: "Resilience",
};

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;
const LONG_HEX = /[0-9a-f]{24,}/i;
const OPAQUE_SEGMENT = /^(?=.*\d)[0-9a-z_-]{16,}$/i;

function isOpaque(segment: string): boolean {
  return (
    UUID.test(segment) || LONG_HEX.test(segment) || OPAQUE_SEGMENT.test(segment)
  );
}

function lowerFirst(value: string): string {
  return value.charAt(0).toLowerCase() + value.slice(1);
}

export function humanizeBuddyKey(
  key: string | null | undefined,
  fallback = "Other",
): string {
  const trimmed = key?.trim();
  if (!trimmed) return fallback;
  const segments = trimmed
    .split(":")
    .map((segment) => segment.trim())
    .filter((segment) => segment.length > 0 && !isOpaque(segment));
  if (segments.length === 0) return fallback;
  const [head, ...rest] = segments.map(humanizeWords);
  if (rest.length === 0) return head;
  return `${head}: ${rest.map(lowerFirst).join(", ")}`;
}

function lookup(
  map: Partial<Record<string, string>>,
  key: string | null | undefined,
  fallback: string,
): string {
  const trimmed = key?.trim();
  if (!trimmed) return fallback;
  return (
    map[trimmed] ?? map[trimmed.toLowerCase()] ?? humanizeBuddyKey(trimmed)
  );
}

export function buddyFailureLabel(key: string | null | undefined): string {
  return lookup(BUDDY_FAILURE_LABELS, key, "Other");
}

export function buddyWorkflowLabel(key: string | null | undefined): string {
  return lookup(BUDDY_WORKFLOW_LABELS, key, "Background work");
}

export function buddyTraitLabel(key: string | null | undefined): string {
  return lookup(BUDDY_TRAIT_LABELS, key, "Trait");
}

export function buddyRuleLabel(key: string | null | undefined): string {
  const trimmed = key?.trim();
  if (!trimmed) return "Saved rule";
  const known = BUDDY_RULE_LABELS[trimmed];
  if (known) return known;
  const ruleKey = trimmed.split(":").slice(0, 2).join(":");
  return BUDDY_RULE_LABELS[ruleKey] ?? humanizeBuddyKey(trimmed, "Saved rule");
}

export interface BuddyIdentifierLabel {
  label: string;
  full: string;
}

const KIND_LABELS: Partial<Record<string, string>> = {
  chat: "Chat",
  check: "Check",
  memory: "Memory",
  worktree: "Worktree",
  run: "Run",
  task: "Task",
  report: "Report",
  work: "Report",
};

const ID_TOKEN = /^(?=.*\d)[0-9a-z]{6,}$/i;

function shortId(value: string): string {
  const token = UUID.test(value)
    ? undefined
    : value.split(/[_-]+/).find((part) => ID_TOKEN.test(part));
  const compact = (token ?? value).replace(/-/g, "");
  return compact.length <= 8 ? compact : compact.slice(0, 8);
}

export function distinctBuddyLabels(labels: readonly string[]): string[] {
  const totals = new Map<string, number>();
  for (const label of labels) totals.set(label, (totals.get(label) ?? 0) + 1);
  const seen = new Map<string, number>();
  return labels.map((label) => {
    if ((totals.get(label) ?? 0) < 2) return label;
    const index = (seen.get(label) ?? 0) + 1;
    seen.set(label, index);
    return `${label} #${index}`;
  });
}

export function humanizeBuddyIdentifier(
  value: string | null | undefined,
): BuddyIdentifierLabel {
  const full = value?.trim() ?? "";
  if (!full) return { label: "", full };

  const uri = /^refact:\/\/([^/?#]+)\/?([^?#]*)/i.exec(full);
  if (uri) {
    const kind = uri[1].toLowerCase();
    const rest = uri[2].split("/").filter(Boolean);
    const kindLabel = KIND_LABELS[kind] ?? humanizeWords(kind);
    const last = rest.at(-1);
    if (!last) return { label: kindLabel, full };
    const tail = isOpaque(last) ? shortId(last) : last;
    return { label: `${kindLabel} ${tail}`, full };
  }

  const prefixed =
    /^([a-z][a-z_]*?)[-_:]([0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}|[0-9a-f]{16,})$/i.exec(
      full,
    );
  if (prefixed) {
    const kind = prefixed[1].toLowerCase();
    const kindLabel = KIND_LABELS[kind] ?? humanizeWords(kind);
    return { label: `${kindLabel} ${shortId(prefixed[2])}`, full };
  }

  const uuid = UUID.exec(full);
  if (uuid) {
    const head = full.slice(0, uuid.index).replace(/[-_:/\s]+$/, "");
    const kindLabel = head ? KIND_LABELS[head] ?? humanizeWords(head) : "";
    const id = shortId(uuid[0]);
    return { label: kindLabel ? `${kindLabel} ${id}` : id, full };
  }
  if (LONG_HEX.test(full) && /^[0-9a-f]+$/i.test(full)) {
    return { label: shortId(full), full };
  }

  return { label: full, full };
}
