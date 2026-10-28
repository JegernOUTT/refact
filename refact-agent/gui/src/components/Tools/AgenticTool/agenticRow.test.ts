import { describe, expect, test } from "vitest";
import { buildAgenticRow } from "./agenticRow";
import type { AgentLookup, AgenticRowInput } from "./agenticRowShared";

const AGENT_ID = "bgagent-4d9f75bc-cc74-4b5a-b60b-7e73d411b9b9";

const lookup: AgentLookup = (id) =>
  AGENT_ID.startsWith(id)
    ? { title: "Subagent: Migrate the report family", childChatId: "child-1" }
    : null;

function row(partial: Partial<AgenticRowInput> & { toolName: string }) {
  return buildAgenticRow({
    args: {},
    content: null,
    extra: undefined,
    transport: "succeeded",
    lookup,
    ...partial,
  });
}

describe("buildAgenticRow agent_message", () => {
  test("resolves the recipient title and stays quiet once sent", () => {
    const result = row({
      toolName: "agent_message",
      args: { to: AGENT_ID, text: "First line\nsecond line" },
      content: `Message queued for ${AGENT_ID}.`,
    });
    expect(result.verb).toBe("Message");
    expect(result.separator).toBe("arrow");
    expect(result.target).toBe("Migrate the report family");
    expect(result.meta).toBe("First line");
    expect(result.status).toBe("done");
    expect(result.statusLabel).toBeNull();
    expect(result.facts).toEqual([
      { label: "To", value: "Migrate the report family" },
      { label: "Delivery", value: "Sent" },
    ]);
    expect(result.markdown).toBe("First line\nsecond line");
    expect(result.childChatId).toBe("child-1");
  });

  test("falls back to a short id and reports a delivery receipt", () => {
    const result = row({
      toolName: "agent_message",
      args: { to: "unknown-agent-0123456789", text: "hi" },
      content: "Message delivered to unknown-agent-0123456789.",
    });
    expect(result.target).toBe("…23456789");
    expect(result.facts).toContainEqual({
      label: "Delivery",
      value: "Delivered",
    });
  });

  test("failed transport shows the Failed pill with the reason", () => {
    const result = row({
      toolName: "agent_message",
      args: { to: "parent", text: "hi" },
      content: "no parent agent",
      transport: "failed",
    });
    expect(result.status).toBe("failed");
    expect(result.error).toBe("no parent agent");
    expect(result.target).toBe("Parent");
  });

  test("rejected deliveries fail with the engine reason", () => {
    const content = "Message NOT queued for x: the chat is sealed.";
    const result = row({
      toolName: "agent_message",
      args: { to: "x", text: "hi" },
      content,
    });
    expect(result.status).toBe("failed");
    expect(result.error).toBe(content);
  });
});

describe("buildAgenticRow agents_overview", () => {
  test("counts running and finished agents without filler", () => {
    const content = [
      "# Agents Overview",
      "",
      `- ${AGENT_ID.slice(
        0,
        16,
      )} — Subagent: Migrate the report family — 🟢 running — step 4 — tool: cat — targets: -`,
      "- bgagent-aaaa — Delegate: Docs — ✅ completed — step 9 — tool: - — targets: -",
    ].join("\n");
    const result = row({ toolName: "agents_overview", content });
    expect(result.verb).toBe("Agents");
    expect(result.target).toBe("1 running · 1 finished");
    expect(result.agents?.map((agent) => agent.title)).toEqual([
      "Migrate the report family",
      "Docs",
    ]);
    expect(result.agents?.[0].childChatId).toBe("child-1");
    expect(result.markdown).toBeNull();
  });

  test("an empty tree renders one line", () => {
    const result = row({
      toolName: "agents_overview",
      content:
        "# Agents Overview\n\nNo active or recently finished agents in this tree.",
    });
    expect(result.target).toBe("None active");
    expect(result.agents).toBeNull();
  });
});

describe("buildAgenticRow agent inspection", () => {
  test("agent_result maps the status and keeps the report markdown", () => {
    const content = [
      "# Agent Result: Subagent: Fix parser",
      "- Status: ❌ failed",
      "- Agent ID: x",
      "",
      "Parser still broken.",
      "- Error: compile error",
    ].join("\n");
    const result = row({
      toolName: "agent_result",
      args: { agent_id: "x" },
      content,
    });
    expect(result.verb).toBe("Result");
    expect(result.target).toBe("Fix parser");
    expect(result.status).toBe("failed");
    expect(result.markdown).toContain("Parser still broken.");
    expect(result.error).toBe("compile error");
  });

  test("agent_wait reports a timeout in meta, not as a failure pill", () => {
    const result = row({
      toolName: "agent_wait",
      args: { agent_ids: [AGENT_ID], timeout_ms: 30000 },
      content: JSON.stringify({
        target: "agents",
        outcome: "timed_out",
        evidence: "⏰ Timed out after 30000ms — none of 1 agents finished.",
      }),
    });
    expect(result.verb).toBe("Wait");
    expect(result.target).toBe("Migrate the report family");
    expect(result.meta).toBe("timed out after 30s");
    expect(result.status).toBe("done");
    expect(result.statusLabel).toBeNull();
  });
});

describe("buildAgenticRow goal and plan", () => {
  test("get_goal uses the first line, version and criteria", () => {
    const result = row({
      toolName: "get_goal",
      content: JSON.stringify({
        goal: {
          content: "Ship the pond\n\nDetails",
          status: "active",
          version: 2,
          delta_count: 1,
          turns_used: 3,
          tokens_used: 10,
          gaps: [],
          criteria: [{ id: "C1", text: "Tests pass" }],
        },
      }),
    });
    expect(result.verb).toBe("Goal");
    expect(result.target).toBe("Ship the pond");
    expect(result.meta).toBe("v2 · 1 criteria");
    expect(result.criteria).toEqual(["C1: Tests pass"]);
  });

  test("validate_goal status comes from the verdict", () => {
    const failed = row({
      toolName: "validate_goal",
      content: "gaps remain",
      extra: { validate_goal: { evidence_verdict: "failed" } },
    });
    expect(failed.status).toBe("failed");
    const passed = row({
      toolName: "validate_goal",
      content: "ok",
      extra: { validate_goal: { evidence_verdict: "passed" } },
    });
    expect(passed.status).toBe("done");
    expect(passed.statusLabel).toBeNull();
  });

  test("get_plan renders the plan markdown", () => {
    const result = row({
      toolName: "get_plan",
      content: JSON.stringify({
        plan: {
          content: "Step one",
          mode: "agent",
          version: 1,
          delta_count: 0,
        },
      }),
    });
    expect(result.verb).toBe("Plan");
    expect(result.target).toBe("Step one");
    expect(result.markdown).toBe("Step one");
  });
});
