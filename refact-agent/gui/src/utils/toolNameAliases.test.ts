import { describe, expect, it } from "vitest";
import { formatToolDisplayName, normalizeToolName } from "./toolNameAliases";

describe("toolNameAliases", () => {
  it.each([
    ["t_cat", "cat"],
    ["t_tree", "tree"],
    ["t_glob", "glob"],
    ["t_plan", "strategic_planning"],
    ["t_delegate", "subagent"],
    ["t_regex_search", "search_pattern"],
    ["t_symbol_def", "search_symbol_definition"],
    ["t_semantic_search", "search_semantic"],
    ["t_write", "create_textdoc"],
    ["t_patch", "update_textdoc"],
    ["t_patch_re", "update_textdoc_regex"],
    ["t_patch_ln", "update_textdoc_by_lines"],
    ["t_patch_at", "update_textdoc_anchored"],
    ["t_apply", "apply_patch"],
    ["t_undo", "undo_textdoc"],
    ["t_ask", "ask_questions"],
    ["t_set_tasks", "tasks_set"],
    ["t_finish", "task_done"],
    ["t_review", "code_review"],
    ["t_research", "deep_research"],
    ["t_save_knowledge", "create_knowledge"],
    ["tool_search", "mcp_tool_search"],
    ["github_create_issue", "mcp_github_create_issue"],
    ["postgres_query", "mcp_postgres_query"],
    ["Glob", "glob"],
    ["Grep", "search_pattern"],
    ["LS", "tree"],
  ])("normalizes Claude Code augmented alias %s", (name, expected) => {
    expect(normalizeToolName(name)).toBe(expected);
  });

  it.each([
    ["Task", "subagent"],
    ["Bash", "shell"],
    ["Glob", "glob"],
    ["Grep", "search_pattern"],
    ["LS", "tree"],
    ["Read", "cat"],
    ["Write", "create_textdoc"],
    ["Edit", "update_textdoc"],
    ["MultiEdit", "update_textdoc_by_lines"],
    ["TodoRead", "tasks_set"],
    ["TodoWrite", "tasks_set"],
    ["NotebookRead", "cat"],
    ["NotebookEdit", "update_textdoc"],
    ["WebFetch", "web"],
    ["WebSearch", "web_search"],
  ])("normalizes native Claude Code tool name %s", (name, expected) => {
    expect(normalizeToolName(name)).toBe(expected);
  });

  it("formats display names after canonicalization", () => {
    expect(formatToolDisplayName("t_regex_search")).toBe("Search Pattern");
    expect(formatToolDisplayName("WebSearch")).toBe("Web Search");
    expect(formatToolDisplayName("github_create_issue")).toBe(
      "MCP Github Create Issue",
    );
  });
});
