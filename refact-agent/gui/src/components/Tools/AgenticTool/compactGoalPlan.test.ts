import { describe, expect, test } from "vitest";
import { compactGoalPlanPresentation } from "./compactGoalPlan";
import {
  COMPACT_HEADLINE_CHARS,
  COMPACT_LABEL_CHARS,
  COMPACT_STATE_CHARS,
  COMPACT_VALUE_CHARS,
  type CompactAgenticInput,
} from "./compactPresentation";

function present(toolName: string, input: Partial<CompactAgenticInput> = {}) {
  const presentation = compactGoalPlanPresentation({
    toolName,
    transportState: "succeeded",
    ...input,
  });
  expect(presentation).not.toBeNull();
  if (presentation === null) throw new Error("Expected goal/plan presentation");
  return presentation;
}

function goal(overrides: Record<string, unknown> = {}) {
  return {
    content: "# Goal\n\nShip the pond with evidence",
    status: "active",
    active: true,
    version: 1,
    delta_count: 3,
    turns_used: 8,
    tokens_used: 1500,
    no_progress_turns: 1,
    cost_used_cents: 12,
    latest_verdict: null,
    gaps: [],
    budget: { cooldown_ms: 30_000, no_progress_token_threshold: 50 },
    wait: null,
    snoozed_until_ms: null,
    checkpoint: null,
    ...overrides,
  };
}

function plan(overrides: Record<string, unknown> = {}) {
  return {
    content:
      "# Plan\n\nProtect the pond’s API contract\n- Read\n- Test\n- Ship",
    version: 1,
    mode: "task_planner",
    created_at_ms: 7,
    delta_count: 200,
    checkpoint: null,
    ...overrides,
  };
}

describe("compact goal/plan intent", () => {
  test.each([
    ["set_goal", { content: "# Goal\nShip the pond" }, "Ship the pond"],
    ["update_goal", { note: " Keep\nexisting evidence " }, "Keep"],
    ["set_plan", { content: "# Plan\n- Protect the API" }, "Protect the API"],
    [
      "set_plan",
      { path: "/workspace/approved-plan.md" },
      "/workspace/approved-plan.md",
    ],
    [
      "set_plan",
      { summary: "Protect signatures", content: "Long plan" },
      "Protect signatures",
    ],
    [
      "update_plan",
      { note: "Add streaming coverage" },
      "Add streaming coverage",
    ],
    [
      "pause_goal",
      { reason: "Waiting on the user’s decision" },
      "Waiting on the user’s decision",
    ],
    [
      "snooze_goal",
      { reason: "Waiting for CI", minutes: 15 },
      "Waiting for CI",
    ],
  ])("uses content first for %s", (toolName, args, expected) => {
    expect(
      present(toolName, { arguments: JSON.stringify(args) }).headline,
    ).toBe(expected);
  });

  test.each([
    ["set_goal", "A goal for this chat"],
    ["update_goal", "Changes to the current goal"],
    ["get_goal", "The current goal"],
    ["validate_goal", "Check the goal’s success criteria"],
    ["pause_goal", "Hold goal pursuit until resumed"],
    ["snooze_goal", "Hold goal pursuit briefly"],
    ["set_plan", "A plan for the work ahead"],
    ["update_plan", "Changes to the current plan"],
    ["get_plan", "The current plan"],
  ])("handles absent or malformed %s arguments", (toolName, expected) => {
    for (const args of [
      undefined,
      "{",
      [],
      { note: " \n ", content: 42, reason: false },
    ]) {
      expect(present(toolName, { arguments: args }).headline).toBe(expected);
    }
  });

  test("normalizes historical aliases and ignores unrelated tools", () => {
    expect(
      present("t_set_goal", { arguments: { content: "Keep the goal" } })
        .headline,
    ).toBe("Keep the goal");
    expect(
      compactGoalPlanPresentation({
        toolName: "subagent",
        transportState: "succeeded",
      }),
    ).toBeNull();
  });

  test("keeps long Unicode and hostile strings bounded, inert and unchanged at the source", () => {
    const content = '<script>alert("passed")</script> 🧪世界e\u0301'.repeat(
      5000,
    );
    const args = Object.freeze({ note: content });
    const presentation = present("update_goal", {
      arguments: args,
      result: { seq: 3, truncated: true },
    });
    expect(presentation.headline).toContain('<script>alert("passed")</script>');
    expect(Array.from(presentation.headline)).toHaveLength(
      COMPACT_HEADLINE_CHARS,
    );
    expect(presentation.headline).not.toMatch(
      /[\uD800-\uDBFF](?![\uDC00-\uDFFF])/u,
    );
    expect(args.note).toBe(content);
    expect(presentation.tone).toBe("warning");
  });
});

describe("compact plan receipts", () => {
  test.each(["set_plan", "set_goal"])(
    "%s accepts a base without claiming completed work",
    (toolName) => {
      expect(
        present(toolName, {
          arguments: { content: "Ship the pond" },
          result: '{"version":1,"supersedes":null}',
        }),
      ).toEqual({
        headline: "Ship the pond",
        stateLabel: toolName === "set_plan" ? "Plan accepted" : "Goal accepted",
        tone: "neutral",
        facts: [{ label: "Base version", value: "1" }],
      });
    },
  );

  test.each(["update_plan", "update_goal"])(
    "%s keeps delta sequence separate from base version",
    (toolName) => {
      const presentation = present(toolName, {
        arguments: { note: "Retain the approved base" },
        result: {
          seq: 201,
          truncated: false,
          version: 999,
          steps_completed: 99,
        },
      });
      expect(presentation.stateLabel).toBe("Update accepted");
      expect(presentation.facts).toEqual([
        { label: "Update sequence", value: "201" },
      ]);
      expect(presentation.tone).toBe("neutral");
      expect(
        present(toolName, { result: { seq: 202, truncated: true } }).stateLabel,
      ).toBe("Update truncated");
    },
  );

  test("separates base, delta count and checkpoint revision without parsing prose steps", () => {
    const projection = plan({
      checkpoint: {
        revision: 180,
        folded_deltas: 180,
        approved_message_id: "base-id",
        approved_version: 1,
        approved_by: "task_planner",
        approved_at_ms: 7,
      },
      history: [{ seq: 200, content: "All 42 steps passed" }],
    });
    expect(
      present("get_plan", { result: JSON.stringify({ plan: projection }) }),
    ).toEqual({
      headline: "Protect the pond’s API contract",
      stateLabel: "Plan snapshot",
      tone: "neutral",
      facts: [
        { label: "Base version", value: "1" },
        { label: "Updates", value: "200" },
        { label: "Checkpoint", value: "Update 180 · 180 folded updates" },
      ],
    });
  });

  test("supports a historical typed plan without checkpoint and humanizes mode", () => {
    expect(
      present("get_plan", { result: { plan: plan({ delta_count: 0 }) } }).facts,
    ).toEqual([
      { label: "Base version", value: "1" },
      { label: "Updates", value: "0" },
      { label: "Mode", value: "Task Planner" },
    ]);
  });

  test.each([
    { revision: 200, folded_deltas: 180, approved_version: 12 },
    { revision: -1, folded_deltas: 180, approved_version: 1 },
    { revision: 180, folded_deltas: 201, approved_version: 1 },
    { revision: 180, folded_deltas: 180, approved_version: 1 },
  ])("does not promote malformed or mismatched checkpoint %#", (checkpoint) => {
    expect(
      present("get_plan", {
        result: { plan: plan({ checkpoint }) },
      }).facts.some((fact) => fact.label === "Checkpoint"),
    ).toBe(false);
  });

  test.each(["get_plan", "get_goal"])(
    "%s distinguishes a known empty projection",
    (toolName) => {
      const key = toolName === "get_plan" ? "plan" : "goal";
      expect(present(toolName, { result: { [key]: null } }).stateLabel).toBe(
        `No ${key}`,
      );
      expect(present(toolName, { result: {} }).stateLabel).toBe(
        "Result unconfirmed",
      );
    },
  );
});

describe("compact goal projection", () => {
  test.each([
    ["active", "Active (snapshot)", "active"],
    ["verifying", "Verifying (snapshot)", "active"],
    ["paused", "Paused (snapshot)", "warning"],
    ["completed", "Completed (snapshot)", "neutral"],
    ["stopped", "Stopped (snapshot)", "neutral"],
    ["budget_exhausted", "Budget exhausted (snapshot)", "warning"],
    ["no_progress", "No progress (snapshot)", "warning"],
    ["transferred", "Transferred (snapshot)", "neutral"],
  ])(
    "preserves typed %s without inferring verification",
    (status, stateLabel, tone) => {
      const presentation = present("get_goal", {
        result: { goal: goal({ status }) },
      });
      expect(presentation.headline).toBe("Ship the pond with evidence");
      expect(presentation.stateLabel).toBe(stateLabel);
      expect(presentation.tone).toBe(tone);
      expect(presentation.facts[0]).toEqual({
        label: "Latest validation",
        value: "No verdict · 0 gaps",
      });
    },
  );

  test.each([
    ["met", "Criteria met (reported)"],
    ["unmet", "Criteria not met"],
    ["blocked", "Validation blocked"],
    ["unavailable", "Validation unavailable"],
    ["not_run", "Validation not run"],
    ["superseded", "Verdict superseded"],
    ["wait", "Waiting; no pass"],
    ["all passed", "Verdict unknown"],
    [undefined, "Verdict unknown"],
  ])(
    "reads only a typed latest verdict %s and gap count",
    (latest_verdict, expected) => {
      const presentation = present("get_goal", {
        result: { goal: goal({ latest_verdict, gaps: ["Tests", "Docs"] }) },
      });
      expect(presentation.facts[0]).toEqual({
        label: "Latest validation",
        value: `${expected} · 2 gaps`,
      });
      expect(presentation.stateLabel).toBe("Active (snapshot)");
      expect(presentation.tone).not.toBe("positive");
    },
  );

  test("does not invent criterion totals or execution evidence from unsupported fields", () => {
    const presentation = present("get_goal", {
      arguments: { criteria: [{ id: "C1", text: "tests pass" }] },
      result: {
        goal: goal({
          content: "Ship it: 8/8 criteria passed and 20 steps complete",
          criteria: [{ id: "C1", text: "tests pass" }],
          criteria_verdicts: [{ id: "C1", met: true }],
          gaps: [],
          latest_verdict: "met",
        }),
      },
    });
    expect(presentation.facts).toHaveLength(3);
    expect(presentation.facts.map((fact) => fact.label)).toEqual([
      "Latest validation",
      "Usage",
      "Budget limits",
    ]);
    expect(presentation.facts.map((fact) => fact.value).join(" ")).not.toMatch(
      /8\/8|20|1\/1|executed|verified/iu,
    );
    expect(presentation.tone).not.toBe("positive");
  });

  test.each([
    {},
    {
      max_turns: null,
      max_minutes: null,
      max_tokens: null,
      no_progress_turns: null,
    },
    {
      max_turns: 0,
      max_minutes: 0,
      max_tokens: 0,
      no_progress_turns: 0,
      max_cost_cents: 0,
    },
  ])("treats omitted, null and zero hard limits as unlimited %#", (budget) => {
    const presentation = present("get_goal", {
      result: { goal: goal({ budget }) },
    });
    expect(presentation.facts).toContainEqual({
      label: "Usage",
      value: "8 turns · 1500 tokens",
    });
    expect(presentation.facts).toContainEqual({
      label: "Budget limits",
      value: "No budget limits",
    });
    expect(JSON.stringify(presentation)).not.toMatch(
      /remaining|exhausted|\//iu,
    );
  });

  test("shows finite configured limits separately from usage without inferring exhaustion", () => {
    const presentation = present("get_goal", {
      result: {
        goal: goal({
          budget: {
            max_turns: 8,
            max_tokens: 1000,
            max_minutes: 12,
            max_cost_cents: 20,
            no_progress_turns: 3,
          },
        }),
      },
    });
    expect(presentation.facts).toContainEqual({
      label: "Budget limits",
      value:
        "8 turns · 1000 tokens · 12 minutes · 20 cents · 3 no-progress turns",
    });
    expect(presentation.stateLabel).toBe("Active (snapshot)");
  });

  test.each([
    undefined,
    null,
    { max_tokens: -1 },
    { max_turns: "8" },
    { max_minutes: Infinity },
  ])("does not invent limits for missing or invalid budget %#", (budget) => {
    const presentation = present("get_goal", {
      result: { goal: goal({ budget }) },
    });
    expect(presentation.facts).not.toContainEqual({
      label: "Budget limits",
      value: "No budget limits",
    });
    expect(presentation.facts[2]).toEqual({
      label: "Base version",
      value: "1",
    });
  });

  test.each([
    [
      { kind: "process", process_id: "proc-1" },
      "Waiting on a process (snapshot)",
    ],
    [{ kind: "child", agent_id: "agent-1" }, "Waiting on a child (snapshot)"],
    [{ kind: "event", name: "ci" }, "Waiting on an event (snapshot)"],
    [{ kind: "deadline", until_ms: 1 }, "Waiting on a deadline (snapshot)"],
  ])(
    "keeps a typed wait parked even with a historical deadline %#",
    (subject, stateLabel) => {
      const presentation = present("get_goal", {
        result: {
          goal: goal({ wait: { subject, since_ms: 0, deadline_ms: 1 } }),
        },
      });
      expect(presentation.stateLabel).toBe(stateLabel);
      expect(presentation.tone).toBe("neutral");
    },
  );

  test("does not override held states with wait, snooze or a past verdict", () => {
    const held = {
      latest_verdict: "met",
      wait: { subject: { kind: "event", name: "ci" } },
      snoozed_until_ms: 1,
    };
    for (const status of [
      "paused",
      "stopped",
      "transferred",
      "budget_exhausted",
    ]) {
      expect(
        present("get_goal", { result: { goal: goal({ ...held, status }) } })
          .stateLabel,
      ).not.toMatch(/waiting|snoozed|complete/iu);
    }
    expect(
      present("get_goal", { result: { goal: goal({ active: false }) } })
        .stateLabel,
    ).toBe("Not owned here (snapshot)");
    expect(
      present("get_goal", { result: { goal: goal({ snoozed_until_ms: 1 }) } })
        .stateLabel,
    ).toBe("Snoozed (snapshot)");
    expect(
      present("get_goal", {
        result: {
          goal: goal({ wait: { subject: { kind: "unknown", name: "ci" } } }),
        },
      }).stateLabel,
    ).toBe("Active (snapshot)");
  });
});

describe("compact validation evidence boundaries", () => {
  test.each([
    ["passed", "Criteria met", "positive"],
    ["failed", "Criteria not met", "danger"],
    ["blocked", "Validation blocked", "warning"],
    ["unavailable", "Validation unavailable", "warning"],
    ["not_run", "Validation not run", "neutral"],
    ["superseded", "Verdict superseded", "warning"],
  ])(
    "preserves the structured %s verdict despite contradicting prose",
    (evidence_verdict, stateLabel, tone) => {
      const presentation = present("validate_goal", {
        result:
          "GOAL MET. All tests passed. 8/8 criteria complete. Report: /tmp/result.json",
        extra: { validate_goal: { evidence_verdict } },
      });
      expect(presentation.stateLabel).toBe(stateLabel);
      expect(presentation.tone).toBe(tone);
      expect(presentation.facts).toEqual([
        {
          label: "Evidence scope",
          value:
            evidence_verdict === "passed" || evidence_verdict === "failed"
              ? "Criteria verdict; executed checks are separate"
              : "No applied criteria verdict",
        },
      ]);
    },
  );

  test.each([
    undefined,
    {},
    { validate_goal: {} },
    { validate_goal: { verdict: "met" } },
    { validate_goal: { evidence_verdict: "ok" } },
    { goal_verification: { verdict: "passed" } },
  ])("missing evidence stays unknown, never not-run or passed %#", (extra) => {
    const presentation = present("validate_goal", {
      result: "GOAL MET — passed!",
      extra,
    });
    expect(presentation.stateLabel).toBe("Verdict unknown");
    expect(presentation.tone).toBe("neutral");
    expect(presentation.facts).toEqual([]);
  });

  test("uses the actual wait marker only alongside a structured not-run verdict", () => {
    const extra = {
      validate_goal: { evidence_verdict: "not_run", verdict: "wait" },
    };
    expect(present("validate_goal", { extra }).stateLabel).toBe(
      "Waiting; no pass",
    );
    expect(
      present("validate_goal", {
        extra: { validate_goal: { verdict: "wait" } },
      }).stateLabel,
    ).toBe("Verdict unknown");
    expect(
      present("validate_goal", {
        extra: {
          validate_goal: {
            ...extra.validate_goal,
            evidence_verdict: "blocked",
          },
        },
      }).stateLabel,
    ).toBe("Validation blocked");
  });

  test.each(["pause_goal", "snooze_goal"])(
    "%s historical text cannot mint a typed state",
    (toolName) => {
      const result =
        toolName === "pause_goal"
          ? "Goal pursuit paused. The user can resume it with goal_control(resume)."
          : "Goal pursuit snoozed for 4 minutes; it resumes automatically afterwards or on the next user message.";
      expect(present(toolName, { result }).stateLabel).toBe(
        "Result unconfirmed",
      );
      expect(
        present(toolName, { result: { status: "completed", success: true } })
          .tone,
      ).toBe("neutral");
    },
  );

  test.each([
    ["pending", "Awaiting result", "neutral"],
    ["running", "Running", "active"],
    ["failed", "Request failed", "danger"],
    ["unknown", "Outcome unknown", "neutral"],
  ] as const)(
    "%s transport never promotes a received payload",
    (transportState, stateLabel, tone) => {
      for (const toolName of [
        "set_goal",
        "update_goal",
        "get_goal",
        "validate_goal",
        "set_plan",
        "update_plan",
        "get_plan",
        "pause_goal",
        "snooze_goal",
      ]) {
        const presentation = present(toolName, {
          transportState,
          result: {
            version: 1,
            supersedes: null,
            seq: 1,
            truncated: false,
            goal: goal({ status: "completed" }),
            plan: plan(),
          },
          extra: { validate_goal: { evidence_verdict: "passed" } },
        });
        expect(presentation.stateLabel).toBe(stateLabel);
        expect(presentation.tone).toBe(tone);
        expect(presentation.facts).toEqual([]);
      }
    },
  );

  test.each([
    ["set_goal", { version: 0, supersedes: null }],
    ["set_plan", { version: 1, supersedes: "old" }],
    ["update_goal", { seq: -1, truncated: false }],
    ["update_plan", { seq: 1, truncated: "false" }],
    ["get_goal", { goal: goal({ status: "passed" }) }],
    ["get_goal", { goal: goal({ turns_used: NaN }) }],
    ["get_goal", { goal: goal({ gaps: ["gap", 5] }) }],
    ["get_goal", { goal: goal({ version: 1.5 }) }],
    ["get_plan", { plan: plan({ delta_count: Number.MAX_SAFE_INTEGER + 1 }) }],
    ["get_plan", { plan: plan({ mode: null }) }],
  ])("keeps malformed %s receipts unconfirmed %#", (toolName, result) => {
    const presentation = present(toolName, { result });
    expect(presentation.stateLabel).toBe("Result unconfirmed");
    expect(presentation.tone).toBe("neutral");
    expect(presentation.facts).toEqual([]);
  });

  test("unknown, unstructured and unsupported envelopes never mint success or counters", () => {
    for (const toolName of [
      "set_goal",
      "update_goal",
      "get_goal",
      "set_plan",
      "update_plan",
      "get_plan",
    ]) {
      for (const result of [
        undefined,
        "{",
        "[]",
        [],
        1,
        { success: true, status: "completed", count: 8 },
        { goal_snapshot: goal({ status: "completed" }) },
        "All steps passed",
      ]) {
        expect(present(toolName, { result })).toMatchObject({
          stateLabel: "Result unconfirmed",
          tone: "neutral",
          facts: [],
        });
      }
    }
  });

  test("returns at most three bounded facts and does not mutate full evidence or projections", () => {
    const result = {
      goal: goal({ gaps: ["🧪".repeat(1000)], latest_verdict: "unmet" }),
    };
    const before = JSON.stringify(result);
    const extra = {
      validate_goal: { evidence_verdict: "passed" },
      evidence_report: "/reports/goal.json",
    };
    const evidenceBefore = JSON.stringify(extra);
    const presentations = [
      present("get_goal", { result }),
      present("get_plan", { result: { plan: plan() } }),
      present("validate_goal", {
        result: "Full verified evidence remains in details",
        extra,
      }),
    ];
    for (const presentation of presentations) {
      expect(presentation.facts.length).toBeLessThanOrEqual(3);
      expect(Array.from(presentation.stateLabel).length).toBeLessThanOrEqual(
        COMPACT_STATE_CHARS,
      );
      for (const fact of presentation.facts) {
        expect(Array.from(fact.label).length).toBeLessThanOrEqual(
          COMPACT_LABEL_CHARS,
        );
        expect(Array.from(fact.value).length).toBeLessThanOrEqual(
          COMPACT_VALUE_CHARS,
        );
      }
    }
    expect(JSON.stringify(result)).toBe(before);
    expect(JSON.stringify(extra)).toBe(evidenceBefore);
  });
});
