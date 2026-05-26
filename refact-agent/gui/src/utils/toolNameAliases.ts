import type { ToolCall } from "../services/refact/types";

const CC_TOOL_ALIASES: Partial<Record<string, string>> = {
  patch_ln: "update_textdoc_by_lines",
  patch_at: "update_textdoc_anchored",
  patch_re: "update_textdoc_regex",
  patch: "update_textdoc",
  write: "create_textdoc",
  undo: "undo_textdoc",
  apply: "apply_patch",
  add_workspace: "add_workspace_folder",
  symbol_def: "search_symbol_definition",
  semantic_search: "search_semantic",
  regex_search: "search_pattern",
  plan: "strategic_planning",
  research: "deep_research",
  review: "code_review",
  delegate: "subagent",
  set_tasks: "tasks_set",
  finish: "task_done",
  ask: "ask_questions",
  render_controls: "buddy_render_controls",
  get_context: "buddy_get_internal_context",
  open_setup_flow: "buddy_open_setup_flow",
  launch_investigation: "buddy_launch_investigation",
  create_issue: "buddy_create_issue",
  create_draft: "buddy_create_draft",
  get_logs: "buddy_get_logs",
  open_view: "buddy_open_view",
  say: "buddy_say",
  merge_worktree: "worktree_merge",
  ctx_probe: "compress_chat_probe",
  ctx_apply: "compress_chat_apply",
  switch_mode: "handoff_to_mode",
  save_knowledge: "create_knowledge",
  hist_search: "search_trajectories",
  hist_get: "get_trajectory_context",
  load_skill: "activate_skill",
  unload_skill: "deactivate_skill",
};

const CLAUDE_CODE_TOOL_ALIASES: Partial<Record<string, string>> = {
  task: "subagent",
  bash: "shell",
  glob: "search_pattern",
  grep: "search_pattern",
  ls: "tree",
  read: "cat",
  write: "create_textdoc",
  edit: "update_textdoc",
  multi_edit: "update_textdoc_by_lines",
  todoread: "tasks_set",
  todo_read: "tasks_set",
  todowrite: "tasks_set",
  todo_write: "tasks_set",
  notebookread: "cat",
  notebook_read: "cat",
  notebookedit: "update_textdoc",
  notebook_edit: "update_textdoc",
  webfetch: "web",
  web_fetch: "web",
  websearch: "web_search",
  web_search: "web_search",
};

const BARE_MCP_TOOL_ALIASES: Partial<Record<string, string>> = {
  call: "mcp_call",
  tool_search: "mcp_tool_search",
  github_create_issue: "mcp_github_create_issue",
  github_create_pull_request: "mcp_github_create_pull_request",
  postgres_query: "mcp_postgres_query",
};

function toSnakeCase(name: string): string {
  return name
    .trim()
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .replace(/[^A-Za-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .replace(/_+/g, "_")
    .toLowerCase();
}

export function normalizeToolName(
  name: string | undefined,
): string | undefined {
  if (name === undefined) return undefined;
  const normalized = toSnakeCase(name);
  if (!normalized) return name;
  if (normalized.startsWith("t_")) {
    const base = normalized.slice(2);
    return CC_TOOL_ALIASES[base] ?? base;
  }
  return (
    CC_TOOL_ALIASES[normalized] ??
    CLAUDE_CODE_TOOL_ALIASES[normalized] ??
    BARE_MCP_TOOL_ALIASES[normalized] ??
    normalized
  );
}

export function normalizeToolCall(toolCall: ToolCall): ToolCall {
  const normalizedName = normalizeToolName(toolCall.function.name);
  if (normalizedName === toolCall.function.name) return toolCall;
  return {
    ...toolCall,
    function: {
      ...toolCall.function,
      name: normalizedName,
    },
  };
}

export function isToolName(
  name: string | undefined,
  expectedName: string,
): boolean {
  return normalizeToolName(name) === expectedName;
}

export function formatToolDisplayName(name: string | undefined): string {
  const normalized = normalizeToolName(name) ?? "tool";
  return normalized
    .replace(/^mcp_/, "mcp_")
    .split("_")
    .filter(Boolean)
    .map((part) =>
      part === "mcp" ? "MCP" : part[0].toUpperCase() + part.slice(1),
    )
    .join(" ");
}
