import { describe, expect, test } from "vitest";

import {
  isValidBackgroundAgent,
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
      model: "openai/gpt-5.6-terra",
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
      model: "openai/gpt-5.6-terra",
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
      model: "openai/gpt-5.6-terra",
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
      model: "openai/gpt-5.6-terra",
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

  test("round-trips a complete camelCase summary to the snake_case contract", () => {
    const normalized = normalizeBackgroundAgentSummary({
      agentId: "agent-3",
      parentChatId: "parent-3",
      childChatId: "child-3",
      kind: "subagent",
      status: "waiting_for_approval",
      title: "Check the model",
      progress: "Waiting for review",
      stepCount: 4,
      lastActivity: "2026-01-01T00:01:00Z",
      targetFiles: ["src/a.ts"],
      editedFiles: ["src/b.ts"],
      diffSummary: "One file changed",
      conflictSummary: null,
      resultSummary: null,
      error: null,
      startedAt: "2026-01-01T00:00:00Z",
      finishedAt: null,
      changeSeq: 7,
      model: "anthropic/claude-sonnet-4",
      modelType: "thinking",
      currentTool: "search_pattern",
      goalSummary: "Validate the wire contract",
      planPresent: true,
      worktreeBranch: "refact/task/agent-3",
      mergeStatus: "pending",
      pendingQuestions: 1,
      questions: [
        {
          id: "question-3",
          text: "Proceed?",
          answer: null,
          askedAt: "2026-01-01T00:01:00Z",
          answeredAt: null,
        },
      ],
      tokensUsed: 456,
      costUsd: 0.12,
    });

    expect(normalized).toEqual({
      agent_id: "agent-3",
      parent_chat_id: "parent-3",
      child_chat_id: "child-3",
      kind: "subagent",
      status: "waiting_for_approval",
      title: "Check the model",
      progress: "Waiting for review",
      step_count: 4,
      last_activity: "2026-01-01T00:01:00Z",
      target_files: ["src/a.ts"],
      edited_files: ["src/b.ts"],
      diff_summary: "One file changed",
      conflict_summary: null,
      result_summary: null,
      error: null,
      started_at: "2026-01-01T00:00:00Z",
      finished_at: null,
      change_seq: 7,
      model: "anthropic/claude-sonnet-4",
      model_type: "thinking",
      current_tool: "search_pattern",
      goal_summary: "Validate the wire contract",
      plan_present: true,
      worktree_branch: "refact/task/agent-3",
      merge_status: "pending",
      pending_questions: 1,
      questions: [
        {
          id: "question-3",
          text: "Proceed?",
          answer: null,
          asked_at: "2026-01-01T00:01:00Z",
          answered_at: null,
        },
      ],
      tokens_used: 456,
      cost_usd: 0.12,
    });
  });

  test("normalizes valid question entries independently", () => {
    const normalized = normalizeBackgroundAgentSummary({
      ...requiredFields,
      questions: [
        { id: "snake", text: "Snake case", asked_at: "2026-01-01" },
        { id: "camel", text: "Camel case", answeredAt: null },
        { id: "invalid" },
        null,
      ],
    });

    expect(normalized.questions).toEqual([
      { id: "snake", text: "Snake case", asked_at: "2026-01-01" },
      { id: "camel", text: "Camel case", answered_at: null },
    ]);
  });

  test("requires parent chat ids and defaults missing optional values", () => {
    expect(
      isValidBackgroundAgent({
        agent_id: "agent-4",
        kind: "subagent",
        status: "running",
      }),
    ).toBe(false);
    expect(
      isValidBackgroundAgent({
        agentId: "agent-5",
        kind: "subagent",
        status: "running",
      }),
    ).toBe(false);

    const { title: _title, ...legacy } = requiredFields;
    const normalized = normalizeBackgroundAgentSummary({
      ...legacy,
      step_count: null,
      change_seq: null,
    });

    expect(normalized).toMatchObject({
      title: "",
      step_count: 0,
      change_seq: 0,
      plan_present: false,
    });
  });

  test("floors and bounds integer fields", () => {
    const normalized = normalizeBackgroundAgentSummary({
      ...requiredFields,
      step_count: 2.9,
      change_seq: Number.MAX_VALUE,
      pending_questions: 1.8,
      tokens_used: 2.4,
    });

    expect(normalized).toMatchObject({
      step_count: 2,
      change_seq: Number.MAX_SAFE_INTEGER,
      pending_questions: 1,
      tokens_used: 2,
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
      change_seq: 0,
      plan_present: false,
      model_type: undefined,
      questions: undefined,
    });
  });
});
