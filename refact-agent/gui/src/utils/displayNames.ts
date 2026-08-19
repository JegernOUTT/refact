/**
 * Shared humanizing lookup for internal identifiers that reach user-facing
 * chrome (audit N-54). Three formats leak today: snake_case mode/status ids
 * (`task_planner`, `needs_work`), snake_case event kinds (`goal_pursuit`,
 * `nudge`), and PascalCase engine error categories (`ProviderTransient`,
 * a Rust enum variant). Route every such identifier through here instead
 * of rendering it verbatim.
 */

const CURATED: Record<string, string | undefined> = {
  // chat modes
  agent: "Agent",
  ask: "Ask",
  explore: "Explore",
  no_tools: "No tools",
  task_planner: "Task Planner",
  task_agent: "Task Agent",
  // goal statuses
  active: "Active",
  verifying: "Verifying",
  paused: "Paused",
  completed: "Completed",
  stopped: "Stopped",
  budget_exhausted: "Budget exhausted",
  no_progress: "No progress",
  transferred: "Transferred",
  // goal pursuit / event kinds
  goal_pursuit: "Goal pursuit",
  needs_work: "Needs work",
  nudge: "Nudge",
  checkpoint: "Checkpoint",
  // engine error categories (Rust UserErrorCategory variants)
  ProviderTransient: "Temporary provider issue",
  ProviderPermanent: "Provider error",
  InvalidRequest: "Invalid request",
  ModelUnavailable: "Model unavailable",
  ContextOverflow: "Context too large",
  RateLimited: "Rate limited",
  AuthenticationError: "Authentication error",
  NetworkError: "Network error",
  StreamCorrupted: "Interrupted response",
  Unknown: "Unknown error",
};

const capitalize = (value: string): string =>
  value.length === 0 ? value : value[0].toUpperCase() + value.slice(1);

/**
 * Humanize an internal identifier for display: curated label when known,
 * otherwise snake_case / kebab-case / PascalCase / camelCase is split into
 * sentence case ("needs_work" -> "Needs work", "ProviderTransient" ->
 * "Provider transient"). Never returns the raw identifier format.
 */
export function humanizeIdentifier(raw: string): string {
  const trimmed = raw.trim();
  if (trimmed.length === 0) return trimmed;
  const curated = CURATED[trimmed];
  if (curated !== undefined) return curated;
  const words = trimmed
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
  if (words.length === 0) return trimmed;
  return capitalize(words.join(" "));
}
