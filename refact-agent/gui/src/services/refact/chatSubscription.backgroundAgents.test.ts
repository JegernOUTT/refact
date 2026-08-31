import { describe, expect, test } from "vitest";

import {
  normalizeBackgroundAgentSummary,
  type BackgroundAgentSummaryWire,
} from "./chatSubscription";

const requiredFields = {
  agent_id: "agent-1",
  parent_chat_id: "parent-1",
  child_chat_id: "child-1",
  kind: "subagent" as const,
  status: "running" as const,
  title: "Inspect agents",
  progress: null,
  last_activity: null,
  diff_summary: null,
  conflict_summary: null,
  result_summary: null,
  error: null,
  started_at: null,
  finished_at: null,
};

describe("normalizeBackgroundAgentSummary", () => {
  test("normalizes every extended snake_case field", () => {
    const normalized = normalizeBackgroundAgentSummary({
      ...requiredFields,
      target_files: ["src/a.ts"],
      edited_files: ["src/b.ts"],
      step_count: 2,
      change_seq: 3,
      model_type: "thinking",
      current_tool: "search_pattern",
      goal_summary: "Find data flow",
      plan_present: true,
      worktree_branch: "refact/subagent/agent-1",
      merge_status: "pending",
      pending_questions: 1,
      questions: [
        {
          id: "question-1",
          text: "Should I continue?",
          answer: null,
          asked_at: "2026-01-01T00:00:00Z",
          answered_at: null,
        },
      ],
      tokens_used: 123,
      cost_usd: 0.45,
    });

    expect(normalized).toMatchObject({
      model_type: "thinking",
      current_tool: "search_pattern",
      goal_summary: "Find data flow",
      plan_present: true,
      worktree_branch: "refact/subagent/agent-1",
      merge_status: "pending",
      pending_questions: 1,
      tokens_used: 123,
      cost_usd: 0.45,
    });
    expect(normalized.questions).toHaveLength(1);
  });

  test("normalizes all extended camelCase fields and sanitizes malformed values", () => {
    const camelCaseAgent = {
      agentId: "agent-2",
      parentChatId: "parent-1",
      childChatId: "child-2",
      kind: "subagent" as const,
      status: "running" as const,
      title: "Inspect agents",
      progress: null,
      lastActivity: null,
      targetFiles: ["src/a.ts"],
      editedFiles: ["src/b.ts"],
      diffSummary: null,
      conflictSummary: null,
      resultSummary: null,
      error: null,
      startedAt: null,
      finishedAt: null,
      stepCount: 2,
      changeSeq: 3,
      modelType: "light",
      currentTool: "cat",
      goalSummary: "Read files",
      planPresent: false,
      worktreeBranch: "refact/subagent/agent-2",
      mergeStatus: "not-valid",
      pendingQuestions: -1,
      questions: [{ id: "missing-text" }],
      tokensUsed: -3,
      costUsd: -0.5,
    };

    const normalized = normalizeBackgroundAgentSummary(
      camelCaseAgent as BackgroundAgentSummaryWire,
    );

    expect(normalized).toMatchObject({
      agent_id: "agent-2",
      parent_chat_id: "parent-1",
      child_chat_id: "child-2",
      model_type: "light",
      current_tool: "cat",
      goal_summary: "Read files",
      plan_present: false,
      worktree_branch: "refact/subagent/agent-2",
      merge_status: null,
      pending_questions: 0,
      questions: [],
      tokens_used: 0,
      cost_usd: 0,
    });
  });

  test("keeps legacy summaries compatible when extended fields are absent", () => {
    const normalized = normalizeBackgroundAgentSummary({
      ...requiredFields,
      target_files: null,
      edited_files: null,
      step_count: null,
      change_seq: null,
    });

    expect(normalized).toMatchObject({
      target_files: [],
      edited_files: [],
      step_count: 0,
      change_seq: -1,
      model_type: undefined,
      questions: undefined,
    });
  });
});
