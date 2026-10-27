import { describe, expect, it } from "vitest";
import { workFixture } from "../../../services/refact/workFixtures";
import type { WorkRow } from "../../../services/refact/workTypes";
import { buildCompactWorkControlPresentation as build } from "./compactWorkControl";
import type { CompactAgenticInput } from "./compactPresentation";
import { resumeReceipt } from "./resumeFixtures";

const names = [
  "agents_overview",
  "agent_list",
  "work_overview",
  "agent_status",
  "agent_wait",
  "agent_result",
  "work_inspect",
  "wait_agents",
  "agent_chat_summary",
  "task_overview",
  "task_questions_list",
  "agent_cancel",
  "agent_resume",
  "agent_approval",
  "pause_agent",
  "resume_agent",
  "restart_agent",
  "cancel_agent",
  "assign_agent",
  "merge_agent",
  "merge_ready_in_order",
  "pick_ab_winner",
];

function present(
  toolName: string,
  result?: unknown,
  overrides: Partial<CompactAgenticInput> = {},
) {
  const output = build({
    toolName,
    result,
    transportState: "succeeded",
    ...overrides,
  });
  expect(output).not.toBeNull();
  if (output === null) throw new Error("Expected a covered tool");
  expect(Array.from(output.headline).length).toBeLessThanOrEqual(160);
  expect(Array.from(output.stateLabel).length).toBeLessThanOrEqual(48);
  expect(output.facts.length).toBeLessThanOrEqual(3);
  for (const fact of output.facts) {
    expect(Array.from(fact.label).length).toBeLessThanOrEqual(48);
    expect(Array.from(fact.value).length).toBeLessThanOrEqual(240);
  }
  return output;
}

function facts(output: ReturnType<typeof present>) {
  return output.facts.map((fact) => `${fact.label}: ${fact.value}`).join("\n");
}

function row(
  state: WorkRow["state"],
  overrides: Partial<WorkRow> = {},
): WorkRow {
  return { ...workFixture.rows[0], state, ...overrides };
}

const questionTable =
  "# Unanswered Planner Questions\n\n| Card | Title | Question ID | Urgency | Question |\n|---|---|---|---|---|\n| T-1 | Check UTF-8 | abcdef12 | block | May I proceed? |";
const roster =
  "# Background Agents\n\n| ID | Kind | Status | Title | Tool | Tokens | Age | Last activity |\n|---|---|---|---|---|---|---|---|\n| bgagent-1 | subagent | ✋ waiting_for_approval | Check UTF-8 | shell | 0 | now | Waiting |";

function known(name: string): unknown {
  switch (name) {
    case "agents_overview":
      return "# Agents Overview\n- child — Check UTF-8 — 🟢 running — step 1";
    case "agent_list":
      return roster;
    case "work_overview":
      return workFixture;
    case "work_inspect":
      return {
        ...workFixture,
        row: row("waiting_user", { title: "Check UTF-8" }),
      };
    case "agent_status":
    case "agent_result":
    case "agent_wait":
      return { agent_id: "child", title: "Check UTF-8", status: "completed" };
    case "wait_agents":
      return "⚠️  Alerts: 0 stuck, 0 failed, 1 needing approval, 0 needing attention\n\nP1 ⏸️ T-1 Check UTF-8 | paused | now | approval\nshowing 1 of 9";
    case "agent_resume":
      return resumeReceipt();
    case "agent_approval":
      return { request_id: "request", decision: "pending" };
    case "merge_ready_in_order":
      return [{ card_id: "T-1", merged: true, conflict: false }];
    case "merge_agent":
      return { card_id: "T-1", merged: true, conflict: false };
    case "task_questions_list":
      return questionTable;
    case "agent_chat_summary":
      return "# Agent Chat Summary: T-1\n**Current state:** finished everything\n## Decisions\n- Keep Unicode intact";
    case "task_overview":
      return "# Task Overview: Unicode\n## Board\nP1: 0 planned, 1 doing, 0 done, 0 failed";
    default:
      return { accepted: true };
  }
}

describe("compact work/control name coverage", () => {
  it.each(names)(
    "keeps %s content-first with recognized reports and args",
    (name) => {
      const output = present(name, known(name), {
        arguments: { card_id: "T-1", title: "Check UTF-8" },
      });
      expect(output.headline).toContain("Check UTF-8");
      expect(output.headline).not.toContain("bgagent-");
      expect(output.tone).not.toBe("positive");
    },
  );

  it.each(names)(
    "keeps %s unknown/malformed/prose outcomes neutral",
    (name) => {
      for (const result of [
        undefined,
        null,
        "",
        "{bad",
        [],
        {},
        42,
        false,
        "Completed, verified, merged and pushed successfully!",
        {
          status: "success",
          summary: "all passed",
          merged: "true",
          conflict: false,
        },
      ]) {
        const output = present(name, result, { arguments: "{bad" });
        expect(output.tone).toBe("neutral");
        expect(output.stateLabel).toMatch(/unconfirmed/i);
      }
    },
  );

  it.each(names)(
    "preserves %s transport failure without claiming target failure",
    (name) => {
      const output = present(name, known(name), { transportState: "failed" });
      expect(output.stateLabel).toBe("Request failed · target unknown");
      expect(facts(output)).toContain("May still be running unchanged");
    },
  );

  it.each(names)("normalizes historical t_ aliases for %s", (name) => {
    expect(present(`t_${name}`, known(name))).toEqual(
      present(name, known(name)),
    );
  });

  it("does not claim other domains or inherited object names", () => {
    for (const toolName of [
      "shell",
      "pause_goal",
      "set_plan",
      "constructor",
      "toString",
      "subagent",
    ]) {
      expect(build({ toolName, transportState: "succeeded" })).toBeNull();
    }
  });

  it.each(["pending", "running", "unknown"] as const)(
    "keeps %s transport distinct from a child state",
    (transportState) => {
      const output = present(
        "agent_result",
        { agent_id: "x", status: "completed" },
        { transportState },
      );
      expect(output.stateLabel).not.toContain("Completed");
      expect(output.tone).not.toBe("positive");
    },
  );
});

describe("work and roster snapshots", () => {
  it("limits page counts and keeps partial and unknown provenance", () => {
    const output = present("work_overview", {
      ...workFixture,
      total: 100,
      truncated: true,
    });
    expect(facts(output)).toContain(
      "1 of 100 reported work items · more pages",
    );
    expect(facts(output)).toContain("partial · historical snapshot");
    expect(facts(output)).toContain("not all work");
    const unknown = present("work_overview", {
      ...workFixture,
      coverage: "unknown",
    });
    expect(facts(unknown)).toContain("unknown · historical snapshot");
  });

  it("keeps essential mixed failures, approvals and held work on the closed row", () => {
    const rows = [row("failed"), row("waiting_user"), row("running")];
    const output = present("work_overview", {
      ...workFixture,
      rows,
      returned: 3,
      total: 30,
      truncated: true,
    });
    expect(output.stateLabel).toBe("Failures · Held work");
    expect(output.tone).toBe("danger");
    const mixedRoster = `${roster}\n| second | subagent | ❌ failed | Retry tests | - | 0 | now | - |\n| third | subagent | queued | Waiting | - | 0 | now | - |`;
    expect(present("agent_list", mixedRoster).stateLabel).toBe(
      "Failures · Approval needed",
    );
  });

  it("never counts missing or unrecognized states as running", () => {
    const output = present(
      "agents_overview",
      "# Agents Overview\n- a — First — running\n- b — Second\n- c — Third — verified",
    );
    expect(output.stateLabel).toBe("Mixed / unknown states");
    expect(facts(output)).toContain("3 rows shown");
    expect(facts(output)).toContain("Unknown beyond the returned rows");
  });

  it("keeps selected work separate from the empty children page", () => {
    const output = present("work_inspect", {
      ...workFixture,
      rows: [],
      returned: 0,
      total: 0,
      row: row("failed", { title: "Fix Unicode", freshness: "stale" }),
    });
    expect(output.headline).toBe("Inspect work: Fix Unicode");
    expect(output.stateLabel).toBe("Failed snapshot");
    expect(facts(output)).toContain("Stale");
  });

  it("does not reveal withheld titles or claim zero means safe files", () => {
    const output = present("work_inspect", {
      ...workFixture,
      row: row("queued", { title: "Hidden title", disclosure: "withheld" }),
    });
    expect(output.headline).toContain("withheld by privacy policy");
    expect(JSON.stringify(output)).not.toContain("Hidden title");
    const empty = present("work_overview", {
      ...workFixture,
      rows: [],
      returned: 0,
      total: 0,
    });
    expect(empty.tone).toBe("neutral");
    expect(facts(empty)).not.toMatch(/safe|no conflicts/iu);
  });

  it("rejects invalid pages and reports attention without inventing a reason", () => {
    expect(
      present("work_overview", { ...workFixture, returned: 5 }).stateLabel,
    ).toContain("unconfirmed");
    expect(
      present("work_overview", {
        ...workFixture,
        rows: [row("queued", { attention: true })],
      }).stateLabel,
    ).toBe("Needs attention");
  });

  it("uses the worktree page unit without fabricating an agent count", () => {
    const output = present("work_overview", {
      ...workFixture,
      page_unit: "worktrees",
      rows: [],
      roots: [workFixture.rows[0].root],
    });
    expect(facts(output)).toContain("1 of 1 reported worktrees");
    expect(output.stateLabel).toBe("Snapshot received");
  });
});

describe("inspection and wait evidence", () => {
  it("keeps planner wait mode and permissively parsed missing states unconfirmed", () => {
    const output = present("wait_agents", known("wait_agents"));
    expect(output.stateLabel).toBe("Approval needed");
    expect(output.headline).toContain("Check UTF-8");
    expect(facts(output)).toContain("1 card rows shown · coverage unknown");
    expect(facts(output)).toContain(
      "not proof of child completion or planner wake-up",
    );
    const unknown = present("wait_agents", {
      rows: [{ card_id: "T-1", title: "Unknown state" }],
    });
    expect(unknown.stateLabel).toBe("Wait snapshot · state unconfirmed");
    expect(unknown.tone).toBe("neutral");
    expect(facts(unknown)).toContain("No recognized alert counts");
    const proseState = present("wait_agents", {
      rows: [
        { card_id: "T-1", status: "not failed; just discussing failed tests" },
      ],
    });
    expect(proseState.stateLabel).toBe("Wait snapshot · state unconfirmed");
    const alerts = present(
      "wait_agents",
      "⚠️  Alerts: 2 stuck, 1 failed, 3 needing approval, 0 needing attention\nshowing 0 of 10",
    );
    expect(alerts.stateLabel).toBe("Failures · Blocked");
    expect(alerts.tone).toBe("danger");
    expect(facts(alerts)).toContain("1 failed · 2 stuck");
    expect(facts(alerts)).toContain("Approval count unconfirmed");
  });

  it.each(["agent_status", "agent_result", "agent_wait"])(
    "uses the existing explicit Status field for %s",
    (name) => {
      const output = present(
        name,
        "# Subagent: Check Unicode\n- Status: ✅ completed\n- Agent ID: child\nAll tests verified (untrusted prose).",
      );
      expect(output.stateLabel.toLowerCase()).toContain(
        "completed · not verified",
      );
      expect(output.tone).toBe("neutral");
    },
  );

  it("does not turn paused into failed", () => {
    const output = present(
      "agent_status",
      "# Subagent: Check Unicode\n- Status: ⏸ paused (by parent, since 2026-10-25)\n- Agent ID: child",
    );
    expect(output.stateLabel).toBe("Paused");
    expect(output.tone).toBe("warning");
  });

  it("shows a wait ending after timeout, never a failed child from timeout prose", () => {
    const text =
      "⏰ Timed out after 1000ms — agent still running.\n\n# Subagent: Check Unicode\n- Status: 🟢 running\n- Agent ID: child";
    const output = present("agent_wait", text);
    expect(output.stateLabel).toBe("Wait ended · running snapshot");
    expect(output.tone).not.toBe("danger");
    expect(facts(output)).toContain(
      "does not mean the child failed or stopped",
    );
    const multiple = present(
      "agent_wait",
      "⏰ Timed out after 20ms — none finished.\n## Other waited agents\n- a: running\n- b: queued",
    );
    expect(multiple.stateLabel).toBe("Wait ended · state unconfirmed");
  });

  it.each(["agent_wait"])(
    "handles explicit timed_out and current approval independently for %s",
    (name) => {
      const output = present(name, {
        timed_out: true,
        agent_id: "a",
        status: "waiting_for_approval",
      });
      expect(output.stateLabel).toBe("Wait ended · approval needed");
      expect(output.tone).toBe("warning");
      expect(present(name, { timed_out: "true" }).stateLabel).toBe(
        "Wait ended · state unconfirmed",
      );
    },
  );

  it.each([
    [undefined, "Wait ended · time limit", "warning"],
    ["running", "Wait ended · time limit", "warning"],
    ["queued", "Wait ended · time limit", "warning"],
    ["waiting_for_approval", "Wait ended · approval needed", "warning"],
    ["failed", "Wait ended · failure reported", "danger"],
    ["completed", "Wait ended · time limit", "warning"],
  ])(
    "keeps exactly one Wait fact for a timed-out agent_wait with %s state",
    (status, stateLabel, tone) => {
      const receipt = { timed_out: true, agent_id: "a", status };
      for (const result of [receipt, JSON.stringify(receipt)]) {
        const output = present("agent_wait", result);
        expect(output.stateLabel).toBe(stateLabel);
        expect(output.tone).toBe(tone);
        expect(output.facts.filter((fact) => fact.label === "Wait")).toEqual([
          {
            label: "Wait",
            value:
              "The wait ended; this does not mean the child failed or stopped",
          },
        ]);
        expect(output.facts.map((fact) => fact.label)).toEqual(
          status === undefined ? ["Wait"] : ["Wait", "Evidence"],
        );
      }
    },
  );

  it.each(["rows", "agents", "statuses"])(
    "keeps one planner Wait fact and distinct approval evidence for timed-out %s reports",
    (key) => {
      const receipt = {
        timed_out: true,
        alerts: { stuck: 0, failed: 0, needing_approval: 1 },
        [key]: [{ card_id: "T-1", status: "waiting_for_approval" }],
      };
      for (const result of [receipt, JSON.stringify(receipt)]) {
        const output = present("wait_agents", result);
        expect(output.stateLabel).toBe("Approval needed");
        expect(output.tone).toBe("warning");
        expect(output.facts).toEqual([
          { label: "Snapshot", value: "1 card rows shown · coverage unknown" },
          {
            label: "Reported alerts",
            value: "0 failed · 0 stuck · 1 awaiting approval",
          },
          {
            label: "Wait",
            value:
              "Historical wait request snapshot; not proof of child completion or planner wake-up",
          },
        ]);
      }
    },
  );

  it("retains request-error priority, one timeout fact and independent approval evidence", () => {
    const output = present("agent_wait", {
      timed_out: true,
      agent_id: "a",
      status: "waiting_for_approval",
      error: "Request interrupted",
    });
    expect(output.stateLabel).toBe("Request error · target unknown");
    expect(output.tone).toBe("danger");
    expect(output.facts).toEqual([
      { label: "Request error", value: "Request interrupted" },
      {
        label: "Wait",
        value: "The wait ended; this does not mean the child failed or stopped",
      },
      {
        label: "Evidence",
        value: "Historical state snapshot · completion is not verification",
      },
    ]);
  });

  it("keeps question urgency visible and does not infer outcomes from a chat summary", () => {
    expect(present("task_questions_list", questionTable).stateLabel).toBe(
      "Planner questions pending",
    );
    expect(
      present("agent_chat_summary", known("agent_chat_summary")).stateLabel,
    ).toContain("unconfirmed");
  });
});

describe("approval and continuation", () => {
  it.each([
    ["pending", "Approval needed"],
    ["reject", "Permission denied"],
    ["rejected", "Permission denied"],
    ["approve", "Approved once · outcome unknown"],
    ["approved", "Approved once · outcome unknown"],
    ["expired", "Permission expired / cancelled"],
    ["cancelled", "Permission expired / cancelled"],
  ])(
    "uses the actual %s receipt, not the requested approval",
    (decision, stateLabel) => {
      const output = present(
        "agent_approval",
        { request_id: "r", decision, consumed: true, already_decided: true },
        { arguments: { action: "approve" } },
      );
      expect(output.stateLabel).toBe(stateLabel);
      expect(facts(output)).toContain("not a command execution result");
    },
  );

  it("recognizes pending list rows with omitted decisions but not an omitted decision receipt", () => {
    const result = { requests: [{ request_id: "r", run_generation: 3 }] };
    expect(
      present("agent_approval", result, { arguments: { action: "list" } })
        .stateLabel,
    ).toBe("Approval needed");
    expect(present("agent_approval", { request_id: "r" }).stateLabel).toBe(
      "Permission unconfirmed",
    );
    expect(present("agent_approval", { requests: [] }).stateLabel).toBe(
      "No requests in snapshot",
    );
    expect(
      present("agent_approval", {
        requests: [
          { request_id: "r", decision: "approved" },
          { request_id: "s", decision: "pending" },
          { request_id: "t", decision: "rejected" },
        ],
      }).stateLabel,
    ).toBe("Denied · approval needed");
  });

  it.each([
    { requests: [{}] },
    { request_id: "r", decision: "completed" },
    { request_id: "r", consumed: "true" },
  ])("does not weaken the existing approval validator: %j", (result) => {
    expect(present("agent_approval", result).stateLabel).toContain(
      "unconfirmed",
    );
  });

  it("shows the current execution cap and never turns queued continuation into running", () => {
    expect(present("agent_resume", resumeReceipt()).stateLabel).toBe(
      "Failed · execution cap reached",
    );
    const output = present(
      "agent_resume",
      resumeReceipt({ action: "resume", cap_stop: null, status: "queued" }),
    );
    expect(output.stateLabel).toBe("Queued");
    expect(facts(output)).toContain("queued does not mean running");
  });

  it("does not suppress approval while reporting caps or unavailable configuration", () => {
    const output = present(
      "agent_resume",
      resumeReceipt({ status: "waiting_for_approval" }),
    );
    expect(output.stateLabel).toBe("Approval needed · cap reached");
    const blocked = present(
      "agent_resume",
      resumeReceipt({
        status: "waiting_for_approval",
        cap_stop: null,
        resume_configuration_available: false,
      }),
    );
    expect(blocked.stateLabel).toBe("Approval needed · resume blocked");
    expect(
      present(
        "agent_resume",
        resumeReceipt({ cap_stop: null, original_scope_available: false }),
      ).stateLabel,
    ).toBe("Failed · resume unavailable");
  });

  it.each([
    { change_seq: -1 },
    { run_generation: 9 },
    { action: "continue" },
    { can_resume: "true" },
    { cap_stop: { key: "provider_quota" } },
  ])(
    "keeps the unchanged continuation parser fail-closed for %j",
    (overrides) => {
      expect(
        present("agent_resume", resumeReceipt(overrides)).stateLabel,
      ).toContain("unconfirmed");
    },
  );
});

describe("control and merge outcomes", () => {
  it.each([
    "pause_agent",
    "resume_agent",
    "restart_agent",
    "cancel_agent",
    "assign_agent",
    "pick_ab_winner",
    "agent_cancel",
    "merge_agent",
  ])("keeps %s acceptance distinct from actual state", (name) => {
    const output = present(
      name,
      { accepted: true, status: "running" },
      { arguments: { card_id: "T-1" } },
    );
    expect(output.stateLabel).toBe("Request accepted · unconfirmed");
    expect(facts(output)).toContain(
      "does not prove the requested state change",
    );
  });

  it("keeps local merge, conflict, error and unknown outcomes distinct", () => {
    const report = { card_id: "T-1", merged: true, conflict: false };
    const local = present("merge_ready_in_order", [report]);
    expect(local.stateLabel).toBe("Merged locally");
    expect(local.tone).toBe("neutral");
    expect(facts(local)).toContain("push and verification unconfirmed");
    const conflict = present("merge_ready_in_order", [
      report,
      { card_id: "T-2", merged: false, conflict: true },
    ]);
    expect(conflict.stateLabel).toBe("Merge conflict reported");
    const error = present("merge_ready_in_order", [
      {
        ...report,
        merged: false,
        error: "Verification failed; automatically reverted",
      },
    ]);
    expect(error.stateLabel).toBe("Merge error reported");
    expect(
      present("merge_ready_in_order", [{ ...report, merged: false }])
        .stateLabel,
    ).toBe("Merge not confirmed");
    expect(
      present(
        "merge_agent",
        "# Agent Work Merged\nSuccessfully merged and pushed",
      ).stateLabel,
    ).toContain("unconfirmed");
    expect(
      present("merge_agent", { ...report, merged: "true" }).stateLabel,
    ).toContain("unconfirmed");
  });

  it("keeps request error distinct from a target that may still run", () => {
    const output = present("resume_agent", {
      accepted: true,
      error: "Permission denied",
    });
    expect(output.stateLabel).toBe("Request error · target unknown");
    expect(output.tone).toBe("danger");
    expect(output.stateLabel).not.toBe("Failed snapshot");
  });

  it("uses only caller-visible titles and bounds hostile Unicode text without interpreting it", () => {
    const title = `<script>alert(1)</script>\n${"🐾界".repeat(5000)}`;
    const output = present("pause_agent", "All verified", {
      arguments: JSON.stringify({ card_id: "T-1", title }),
      extra: { title: "Not authorized as a title source" },
    });
    expect(output.headline).toContain("<script>alert(1)</script>");
    expect(output.headline).not.toContain("\n");
    expect(output.headline).toMatch(/…$/u);
    expect(output.headline).not.toMatch(/[\uD800-\uDBFF]$/u);
    expect(output.stateLabel).toContain("unconfirmed");
    expect(
      present("agent_status", undefined, {
        arguments: { agent_id: "raw-live-id" },
        extra: {
          background_agents: { "raw-live-id": { title: "secret lookup" } },
        },
      }).headline,
    ).toBe("Check child status");
  });
});
