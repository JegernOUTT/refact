import { describe, expect, test } from "vitest";
import { buildAgenticToolViewModel } from "./agenticToolModel";
import {
  AGENTIC_TOOL_NAMES,
  BOARD_RUNTIME_TOOL_NAMES,
  DEDICATED_TOOL_CARD_NAMES,
  hasDedicatedToolCard,
  isAgenticToolName,
  lookupAgenticFamily,
} from "./agenticFamilies";

type Case = {
  name: string;
  toolName: string;
  args: string;
  content: string | null;
  hasResult: boolean;
  toolFailed?: boolean;
  family: string;
  expect: (model: NonNullable<ReturnType<typeof build>>) => void;
};

function build(input: {
  toolName: string;
  args?: string;
  content?: string | null;
  hasResult?: boolean;
  toolFailed?: boolean;
  isBusy?: boolean;
  historical?: boolean;
}) {
  return buildAgenticToolViewModel({
    toolName: input.toolName,
    argumentsRaw: input.args ?? "{}",
    content: input.content ?? null,
    hasResult: input.hasResult ?? true,
    toolFailed: input.toolFailed,
    isBusy: input.isBusy,
    historical: input.historical,
  });
}

const OVERVIEW_OUTPUT = [
  "# Agents Overview",
  "",
  "You are agent 11111111-2222-3333-4444-555555555555.",
  "",
  "- abc12345 — Fix the pond — running — step 3 — tool: cat",
  "- def67890 — Write docs — completed — step 9 — tool: write",
].join("\n");

const AGENT_LIST_OUTPUT = [
  "# Background Agents",
  "",
  "| ID | Kind | Status | Title | Tool | Tokens | Age | Last activity |",
  "|---|---|---|---|---|---|---|---|",
  "| aaa | subagent | running | Ship it | cat | 12 | 3m | now |",
  "",
  "Total: 1 (running: 1).",
].join("\n");

const AGENT_STATUS_OUTPUT = [
  "# Subagent: Ship it",
  "- Status: running",
  "- Agent ID: 11111111-2222-3333-4444-555555555555",
  "- Kind: subagent",
  "- Model: gpt-5",
  "",
  "Child trajectory: [view](refact://chat/child-chat-1)",
].join("\n");

const CASES: Case[] = [
  {
    name: "agent_message renders sender to recipient",
    toolName: "agent_message",
    args: JSON.stringify({
      to: "bgagent-7",
      text: "Please rebase",
      expects_reply: true,
    }),
    content: "Message queued for bgagent-7.",
    hasResult: true,
    family: "message",
    expect: (model) => {
      expect(model.message?.recipient).toBe("bgagent-7");
      expect(model.message?.text).toBe("Please rebase");
      expect(model.message?.expectsReply).toBe(true);
      expect(model.headline).toBe("Agent message → bgagent-7");
    },
  },
  {
    name: "agents_overview parses agent rows",
    toolName: "agents_overview",
    args: "{}",
    content: OVERVIEW_OUTPUT,
    hasResult: true,
    family: "overview",
    expect: (model) => {
      expect(model.agents).toHaveLength(2);
      expect(model.agents[0].agentId).toBe("abc12345");
      expect(model.agents[0].status).toBe("running");
      expect(model.subtitle).toContain("2 agents");
    },
  },
  {
    name: "agent_list parses markdown table",
    toolName: "agent_list",
    args: JSON.stringify({ status: "running" }),
    content: AGENT_LIST_OUTPUT,
    hasResult: true,
    family: "overview",
    expect: (model) => {
      expect(model.agents).toHaveLength(1);
      expect(model.agents[0].title).toBe("Ship it");
    },
  },
  {
    name: "progress_report shows status line",
    toolName: "progress_report",
    args: JSON.stringify({ status_line: "Half done" }),
    content: "Progress updated: Half done",
    hasResult: true,
    family: "progress",
    expect: (model) => {
      expect(model.message?.text).toBe("Half done");
      expect(model.message?.recipient).toBe("parent");
    },
  },
  {
    name: "agent_status extracts details and child chat",
    toolName: "agent_status",
    args: JSON.stringify({
      agent_id: "11111111-2222-3333-4444-555555555555",
    }),
    content: AGENT_STATUS_OUTPUT,
    hasResult: true,
    family: "inspect",
    expect: (model) => {
      expect(model.childChatId).toBe("child-chat-1");
      expect(model.details.map((detail) => detail.label)).toContain("Status");
      expect(model.headline).toBe("Agent status: …55555555");
      expect(model.agentIds).toEqual(["11111111-2222-3333-4444-555555555555"]);
    },
  },
  {
    name: "agent_wait pending has no result",
    toolName: "agent_wait",
    args: JSON.stringify({ agent_id: "abc" }),
    content: null,
    hasResult: false,
    family: "inspect",
    expect: (model) => {
      expect(model.status).toBe("pending");
      expect(model.statusLabel).toBe("awaiting result");
    },
  },
  {
    name: "agent_result failure surfaces error text",
    toolName: "agent_result",
    args: JSON.stringify({ agent_id: "abc" }),
    content: "Background agent 'abc' not found in this chat.",
    hasResult: true,
    toolFailed: true,
    family: "inspect",
    expect: (model) => {
      expect(model.status).toBe("failed");
      expect(model.errorText).toContain("not found");
      expect(model.body).toBeNull();
    },
  },
  {
    name: "agent_cancel is a control family",
    toolName: "agent_cancel",
    args: JSON.stringify({ agent_id: "abc", reason: "no longer needed" }),
    content: "✓ Cancel requested for: abc.",
    hasResult: true,
    family: "control",
    expect: (model) => {
      expect(model.subtitle).toContain("no longer needed");
    },
  },
  {
    name: "agent_approval maps to control",
    toolName: "agent_approval",
    args: JSON.stringify({ agent_id: "abc", approved: true }),
    content: "Approved",
    hasResult: true,
    family: "control",
    expect: (model) => {
      expect(model.icon).toBe("control");
    },
  },
  {
    name: "agent_resume maps to control",
    toolName: "agent_resume",
    args: JSON.stringify({ agent_id: "abc" }),
    content: "Resumed",
    hasResult: true,
    family: "control",
    expect: (model) => {
      expect(model.family).toBe("control");
    },
  },
  {
    name: "work_overview maps to overview",
    toolName: "work_overview",
    args: "{}",
    content: "# Work Overview\n\nNothing in flight.",
    hasResult: true,
    family: "overview",
    expect: (model) => {
      expect(model.body).toBe("Nothing in flight.");
    },
  },
  {
    name: "work_inspect maps to inspect",
    toolName: "work_inspect",
    args: JSON.stringify({ id: "w-1" }),
    content: "# Work Inspect\n- Status: open",
    hasResult: true,
    family: "inspect",
    expect: (model) => {
      expect(model.details[0]).toEqual({ label: "Status", value: "open" });
    },
  },
  {
    name: "set_goal shows summary headline",
    toolName: "set_goal",
    args: JSON.stringify({ content: "Ship the agentic tool cards" }),
    content: "Goal installed.",
    hasResult: true,
    family: "goal",
    expect: (model) => {
      expect(model.headline).toBe("Set goal: Ship the agentic tool cards");
    },
  },
  {
    name: "update_goal is goal family",
    toolName: "update_goal",
    args: JSON.stringify({ note: "Narrowed scope" }),
    content: "Goal updated.",
    hasResult: true,
    family: "goal",
    expect: (model) => {
      expect(model.headline).toBe("Update goal: Narrowed scope");
    },
  },
  {
    name: "get_goal is goal family",
    toolName: "get_goal",
    args: "{}",
    content: "- Status: active",
    hasResult: true,
    family: "goal",
    expect: (model) => {
      expect(model.details).toHaveLength(1);
    },
  },
  {
    name: "validate_goal is goal family",
    toolName: "validate_goal",
    args: "{}",
    content: "- Verdict: met",
    hasResult: true,
    family: "goal",
    expect: (model) => {
      expect(model.icon).toBe("goal");
    },
  },
  {
    name: "pause_goal is control family",
    toolName: "pause_goal",
    args: JSON.stringify({ reason: "waiting on CI" }),
    content: "Paused.",
    hasResult: true,
    family: "control",
    expect: (model) => {
      expect(model.subtitle).toContain("waiting on CI");
    },
  },
  {
    name: "snooze_goal is control family",
    toolName: "snooze_goal",
    args: JSON.stringify({ minutes: 30 }),
    content: "Snoozed.",
    hasResult: true,
    family: "control",
    expect: (model) => {
      expect(model.family).toBe("control");
    },
  },
  {
    name: "set_plan is plan family",
    toolName: "set_plan",
    args: JSON.stringify({ summary: "Wire the registry" }),
    content: "Plan installed.",
    hasResult: true,
    family: "plan",
    expect: (model) => {
      expect(model.headline).toBe("Set plan: Wire the registry");
    },
  },
  {
    name: "update_plan is plan family",
    toolName: "update_plan",
    args: JSON.stringify({ note: "Added tests" }),
    content: "Plan updated.",
    hasResult: true,
    family: "plan",
    expect: (model) => {
      expect(model.icon).toBe("plan");
    },
  },
  {
    name: "get_plan is plan family",
    toolName: "get_plan",
    args: "{}",
    content: "- Version: 3",
    hasResult: true,
    family: "plan",
    expect: (model) => {
      expect(model.details[0].value).toBe("3");
    },
  },
  {
    name: "spawn_agent is spawn family",
    toolName: "spawn_agent",
    args: JSON.stringify({ task: "Fix the flaky test" }),
    content: "Spawned.",
    hasResult: true,
    family: "spawn",
    expect: (model) => {
      expect(model.headline).toBe("Spawn agent: Fix the flaky test");
    },
  },
  {
    name: "task_broadcast is message family",
    toolName: "task_broadcast",
    args: JSON.stringify({ text: "Board frozen", to: "all" }),
    content: "Broadcast sent.",
    hasResult: true,
    family: "message",
    expect: (model) => {
      expect(model.message?.recipient).toBe("all");
    },
  },
  {
    name: "board_get is task family",
    toolName: "board_get",
    args: "{}",
    content: "# Board\n\nNo cards.",
    hasResult: true,
    family: "task",
    expect: (model) => {
      expect(model.headline).toBe("Board");
    },
  },
  {
    name: "mark_done is task family",
    toolName: "mark_done",
    args: JSON.stringify({ card_id: "T-1" }),
    content: "Card T-1 done.",
    hasResult: true,
    family: "task",
    expect: (model) => {
      expect(model.family).toBe("task");
    },
  },
];

describe("buildAgenticToolViewModel", () => {
  test.each(CASES.map((testCase) => [testCase.name, testCase] as const))(
    "%s",
    (_name, testCase) => {
      const model = build({
        toolName: testCase.toolName,
        args: testCase.args,
        content: testCase.content,
        hasResult: testCase.hasResult,
        toolFailed: testCase.toolFailed,
      });
      expect(model).not.toBeNull();
      if (!model) return;
      expect(model.family).toBe(testCase.family);
      testCase.expect(model);
    },
  );

  test("returns null for non-agentic tools", () => {
    expect(build({ toolName: "cat" })).toBeNull();
    expect(build({ toolName: undefined as unknown as string })).toBeNull();
  });

  test("normalizes claude-code style aliases", () => {
    expect(lookupAgenticFamily("t_agent_message")?.family).toBe("message");
    expect(lookupAgenticFamily("AgentsOverview")?.family).toBe("overview");
    expect(isAgenticToolName("t_set_goal")).toBe(true);
  });

  test("flags malformed arguments without throwing", () => {
    const model = build({
      toolName: "agent_message",
      args: "{not json",
      content: "ok",
    });
    expect(model?.malformedArgs).toBe(true);
    expect(model?.argsJson).toBe("{not json");
    expect(model?.message).toBeNull();
  });

  test("tolerates array arguments and empty payloads", () => {
    const arrayArgs = build({ toolName: "agent_list", args: "[1,2]" });
    expect(arrayArgs?.malformedArgs).toBe(true);
    const emptyArgs = build({ toolName: "agent_list", args: "" });
    expect(emptyArgs?.malformedArgs).toBe(false);
    expect(emptyArgs?.argsJson).toBeNull();
  });

  test("marks running calls as pending with running label", () => {
    const model = build({
      toolName: "agent_wait",
      hasResult: false,
      isBusy: true,
    });
    expect(model?.status).toBe("pending");
    expect(model?.statusLabel).toBe("running");
  });

  test("labels historical snapshots", () => {
    const model = build({
      toolName: "agents_overview",
      content: OVERVIEW_OUTPUT,
      historical: true,
    });
    expect(model?.historical).toBe(true);
  });

  test("every registered tool name resolves to a family", () => {
    for (const toolName of AGENTIC_TOOL_NAMES) {
      expect(lookupAgenticFamily(toolName)).not.toBeNull();
      expect(build({ toolName })).not.toBeNull();
    }
  });

  test("every board-runtime tool has a dedicated card instead of raw output", () => {
    for (const toolName of BOARD_RUNTIME_TOOL_NAMES) {
      expect(hasDedicatedToolCard(toolName), toolName).toBe(true);
      expect(hasDedicatedToolCard(`t_${toolName}`), toolName).toBe(true);
    }
    for (const toolName of DEDICATED_TOOL_CARD_NAMES) {
      expect(hasDedicatedToolCard(toolName), toolName).toBe(true);
    }
    expect(hasDedicatedToolCard("cat")).toBe(false);
    expect(hasDedicatedToolCard(undefined)).toBe(false);
  });

  test("document and memory tools get bounded structured cards", () => {
    const doc = build({
      toolName: "doc_append",
      args: JSON.stringify({ doc_id: "design", content: "more" }),
      content: "# Appended design\n\n- lines: 12\nbody text",
    });
    expect(doc?.family).toBe("doc");
    expect(doc?.icon).toBe("doc");
    expect(doc?.headline).toBe("Appended design");
    expect(doc?.body).toBe("body text");
    const memory = build({
      toolName: "task_mem_search",
      args: JSON.stringify({ query: "locks" }),
      content: "no heading here",
    });
    expect(memory?.family).toBe("memory");
    expect(memory?.headline).toBe("Search task memory");
    const control = build({
      toolName: "merge_ready_in_order",
      args: JSON.stringify({ task_id: "task-1" }),
      hasResult: false,
    });
    expect(control?.family).toBe("control");
    expect(control?.status).toBe("pending");
  });
});
