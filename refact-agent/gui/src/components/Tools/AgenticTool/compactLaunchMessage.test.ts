import { describe, expect, test } from "vitest";
import { buildCompactLaunchMessagePresentation as build } from "./compactLaunchMessage";
import type { CompactAgenticInput } from "./compactPresentation";
import { boardReceipt, receiptState } from "./messageReceipt";

function present(input: Partial<CompactAgenticInput> = {}) {
  return build({
    toolName: "agent_message",
    transportState: "succeeded",
    ...input,
  });
}

function mail(outcome: string, ttl_secs: unknown = null) {
  return {
    delivery_receipt: {
      kind: "agent_message",
      delivery_id: "delivery-1",
      outcome,
      ttl_secs,
    },
  };
}

function launch(execution_state: unknown = "queued") {
  return {
    card_id: "T-1",
    agent_id: "agent-1",
    agent_chat_id: "chat-1",
    execution_state,
  };
}

const CARD_OUTCOMES = [
  "queued",
  "delivered",
  "duplicate",
  "no_live_session",
  "skipped",
  "excluded",
  "failed",
] as const;
function card(outcome: (typeof CARD_OUTCOMES)[number], index = 0) {
  return {
    card_id: `T-${index}`,
    title: `Work ${index}`,
    outcome,
    reached: outcome === "queued" || outcome === "delivered",
  };
}

function broadcast(cards = CARD_OUTCOMES.map(card)) {
  return {
    delivery_receipts: {
      kind: "task_broadcast",
      notified: cards.filter((row) => row.reached).length,
      total: cards.length,
      cards,
    },
  };
}

describe("compact launch, message and progress presentations", () => {
  test.each([
    ["spawn_agent", { task: "Fix parser" }, "Fix parser"],
    ["task_spawn_agent", { title: "Fix parser" }, "Fix parser"],
    ["spawn_ab", { card_title: "Compare parsers" }, "Compare parsers"],
    ["spawn_agents_batch", { title: "Fix parser batch" }, "Fix parser batch"],
    [
      "agent_message",
      { text: "Please check the parser" },
      "Please check the parser",
    ],
    [
      "agent_steer",
      { message: "Keep the parser scoped" },
      "Keep the parser scoped",
    ],
    ["task_broadcast", { message: "Pause parser edits" }, "Pause parser edits"],
    [
      "planner_reply",
      { answer: "Keep the smaller parser" },
      "Keep the smaller parser",
    ],
    ["agent_ask_planner", { question: "Which parser?" }, "Which parser?"],
    [
      "progress_report",
      { status_line: "Parser tests added" },
      "Parser tests added",
    ],
    ["agent_update", { message: "Parser tests added" }, "Parser tests added"],
  ] as const)(
    "content comes first for %s and its aliases",
    (toolName, args, headline) => {
      for (const name of [
        toolName,
        `t_${toolName}`,
        toolName
          .split("_")
          .map((part) => part[0].toUpperCase() + part.slice(1))
          .join(""),
      ]) {
        const result = present({
          toolName: name,
          arguments: JSON.stringify(args),
        });
        expect(result?.headline).toBe(headline);
        expect(result?.headline).not.toMatch(
          /^(Subagent|Agent message|Progress report):/u,
        );
        expect(result?.stateLabel).toBe("Outcome unconfirmed");
      }
    },
  );

  test.each([
    "subagent",
    "delegate",
    "agent_status",
    "agent_peer_grant",
    "shell",
    "unknown",
  ])("leaves %s to its owning adapter", (toolName) => {
    expect(present({ toolName })).toBeNull();
  });

  test.each([
    "spawn_agent",
    "task_spawn_agent",
    "spawn_ab",
    "spawn_agents_batch",
    "agent_message",
    "agent_steer",
    "task_broadcast",
    "planner_reply",
    "agent_ask_planner",
    "progress_report",
    "agent_update",
  ])("transport overrides receipt-looking data for %s", (toolName) => {
    for (const transportState of [
      "pending",
      "running",
      "failed",
      "unknown",
    ] as const) {
      const result = present({
        toolName,
        arguments: { task: "work", text: "work", status_line: "work" },
        result: launch("running"),
        extra: mail("consumed"),
        transportState,
      });
      expect(result?.stateLabel).toBe(
        transportState === "failed"
          ? "Request failed"
          : transportState === "unknown"
            ? "Outcome unconfirmed"
            : toolName.includes("spawn")
              ? "Requesting"
              : "Awaiting receipt",
      );
      expect(result?.tone).toBe(
        transportState === "failed" ? "danger" : "neutral",
      );
    }
  });

  test.each([
    ["queued", "Queued", "neutral"],
    ["running", "Running", "active"],
    ["completed", "Child finished · unverified", "neutral"],
    ["failed", "Child failed", "danger"],
    ["cancelled", "Child cancelled", "warning"],
    ["success", "Outcome unconfirmed", "neutral"],
    ["done", "Outcome unconfirmed", "neutral"],
    ["toString", "Outcome unconfirmed", "neutral"],
    ["__proto__", "Outcome unconfirmed", "neutral"],
    [null, "Outcome unconfirmed", "neutral"],
  ])(
    "explicit launch state %s is not transport success",
    (execution_state, stateLabel, tone) => {
      for (const toolName of ["spawn_agent", "task_spawn_agent", "spawn_ab"]) {
        const result =
          toolName === "spawn_ab"
            ? {
                card_id: "T-1",
                execution_state,
                variant_a: launch(),
                variant_b: launch(),
              }
            : launch(execution_state);
        expect(
          present({ toolName, result: JSON.stringify(result) }),
        ).toMatchObject({ stateLabel, tone });
      }
    },
  );

  test("a recognized launch without execution state is acceptance only", () => {
    const result = {
      card_id: "T-1",
      agent_id: "agent-1",
      agent_chat_id: "chat-1",
      model: "provided-model",
      card_title: "Fix parser",
    };
    expect(present({ toolName: "spawn_agent", result })).toMatchObject({
      headline: "Fix parser",
      stateLabel: "Launch accepted",
      tone: "neutral",
    });
    expect(present({ toolName: "spawn_agent", result })?.facts).toContainEqual({
      label: "Model",
      value: "provided-model",
    });
  });

  test.each([
    { execution_state: "running" },
    { card_id: "T-1", execution_state: "running" },
    { ...launch(), agent_chat_id: "" },
    { ...launch(), agent_id: 42 },
    "# Subagent: success\n- Status: running",
    "Started 2 variants for card T-1.",
    "success, done, completed, verified",
  ])("malformed and legacy launches stay neutral %#", (result) => {
    expect(present({ toolName: "spawn_agent", result })).toMatchObject({
      stateLabel: "Outcome unconfirmed",
      tone: "neutral",
    });
  });

  test("raw IDs do not replace useful launch intent or invent a name", () => {
    expect(
      present({
        toolName: "spawn_agent",
        arguments: { card_id: "T-1" },
        result: launch(),
      })?.headline,
    ).toBe("Requested work");
    expect(
      present({
        toolName: "spawn_agent",
        arguments: { task: "Fix parser", card_id: "T-1" },
        result: launch(),
      })?.headline,
    ).toBe("Fix parser");
  });

  test("batch receipt counts describe only returned rows, never omitted work", () => {
    const rows = [
      { card_id: "T-1", success: true, agent_chat_id: "chat-1" },
      { card_id: "T-2", success: false, error: "denied" },
    ];
    for (const result of [
      rows,
      JSON.stringify(rows),
      { waves: [], results: rows },
    ]) {
      const output = present({
        toolName: "spawn_agents_batch",
        arguments: {
          cards: [{ card_id: "T-1" }, { card_id: "T-2" }, { card_id: "T-3" }],
        },
        result,
      });
      expect(output).toMatchObject({
        stateLabel: "Some launches failed",
        tone: "warning",
      });
      expect(output?.facts).toContainEqual({
        label: "Returned receipts",
        value: "1 accepted · 1 failed; other requests unconfirmed",
      });
      expect(JSON.stringify(output)).not.toContain("2/3");
    }
    expect(
      present({
        toolName: "spawn_agents_batch",
        result: [{ card_id: "T-1", success: true }],
      }),
    ).toMatchObject({ stateLabel: "Launches accepted", tone: "neutral" });
    expect(
      present({
        toolName: "spawn_agents_batch",
        result: [{ card_id: "T-1", success: false, error: "denied" }],
      }),
    ).toMatchObject({ stateLabel: "Launches failed", tone: "danger" });
  });

  test.each([
    [],
    { results: [] },
    [{ card_id: "T-1", success: "true" }],
    [{ card_id: "T-1", success: true, error: "failed" }],
    [{ card_id: "T-1", success: true, agent_chat_id: 123 }],
    [{ success: true }],
    [{ card_id: "T-1", success: true }, null],
    { summary: "all launches completed" },
  ])("rejects unsupported batch receipt data %#", (result) => {
    expect(present({ toolName: "spawn_agents_batch", result })).toMatchObject({
      stateLabel: "Outcome unconfirmed",
      tone: "neutral",
    });
  });

  test.each([
    "queued",
    "delivered",
    "consumed",
    "duplicate",
    "expired",
    "superseded",
    "failed",
  ])("preserves %s mail semantics", (outcome) => {
    const extra = mail(outcome, 30);
    const result = present({
      arguments: { to: "child", text: "Review parser" },
      result: "Message queued for child.",
      extra,
    });
    expect(result?.stateLabel).toBe(
      outcome === "failed"
        ? "Delivery failed"
        : outcome[0].toUpperCase() + outcome.slice(1),
    );
    expect(result?.facts).toContainEqual({
      label: "Delivery deadline",
      value: "30s unless landed",
    });
    expect(result?.stateLabel).not.toMatch(/completed|verified|answer/iu);
    expect(boardReceipt("agent_message", extra)).not.toBeNull();
  });

  test.each(CARD_OUTCOMES)(
    "preserves %s steer semantics and provided recipient title",
    (outcome) => {
      const extra = {
        delivery_receipt: {
          kind: "agent_steer",
          ...card(outcome),
          title: "Fix parser",
        },
      };
      const output = present({
        toolName: "agent_steer",
        arguments: { card_id: "T-0", message: "Keep scope" },
        extra,
      });
      expect(output?.facts[0]).toEqual({
        label: "Recipient",
        value: "Fix parser",
      });
      expect(output?.stateLabel).toBe(
        (
          {
            no_live_session: "No live session",
            failed: "Delivery failed",
          } as Partial<Record<string, string>>
        )[outcome] ?? outcome[0].toUpperCase() + outcome.slice(1),
      );
    },
  );

  test("broadcast retains mixed outcomes and validated total without claiming consumption", () => {
    const output = present({
      toolName: "task_broadcast",
      arguments: { message: "Pause edits" },
      extra: broadcast(),
    });
    expect(output).toMatchObject({
      headline: "Pause edits",
      stateLabel: "2/7 reached",
      tone: "warning",
    });
    expect(output?.facts).toContainEqual({
      label: "Delivery receipts",
      value:
        "1 queued · 1 delivered · 1 duplicate · 1 no live session · 1 skipped · 1 excluded · 1 delivery failed",
    });
    expect(output?.facts).toHaveLength(2);
    expect(
      present({
        toolName: "task_broadcast",
        extra: broadcast([card("queued")]),
      })?.stateLabel,
    ).toBe("1/1 queued");
    expect(
      present({
        toolName: "task_broadcast",
        extra: broadcast([card("delivered")]),
      })?.stateLabel,
    ).toBe("1/1 delivered");
    expect(
      present({ toolName: "task_broadcast", extra: broadcast([]) })?.stateLabel,
    ).toBe("0/0 reached");
  });

  test.each([
    { kind: "task_broadcast", notified: 1, total: 2, cards: [card("queued")] },
    { kind: "task_broadcast", notified: 0, total: 1, cards: [card("queued")] },
    { kind: "task_broadcast", notified: 2, total: 1, cards: [card("queued")] },
    {
      kind: "task_broadcast",
      notified: 1.1,
      total: 1,
      cards: [card("queued")],
    },
    { kind: "task_broadcast", notified: -1, total: 0, cards: [] },
    {
      kind: "task_broadcast",
      notified: 1,
      total: 1,
      cards: [{ ...card("failed"), reached: true }],
    },
    {
      kind: "task_broadcast",
      notified: 1,
      total: 1,
      cards: [{ ...card("queued"), title: {} }],
    },
    {
      kind: "task_broadcast",
      notified: 1,
      total: 1,
      cards: [{ ...card("queued"), reason: [] }],
    },
    {
      kind: "task_broadcast",
      notified: 1,
      total: 1,
      cards: [{ ...card("queued"), outcome: "consumed" }],
    },
  ])(
    "rejects inconsistent broadcast payload %# in shared parser and compact summary",
    (delivery_receipts) => {
      const extra = { delivery_receipts };
      expect(boardReceipt("task_broadcast", extra)).toBeNull();
      expect(present({ toolName: "task_broadcast", extra })).toMatchObject({
        stateLabel: "Outcome unconfirmed",
        tone: "neutral",
      });
    },
  );

  test.each([
    { kind: "agent_steer", ...card("queued"), reached: false },
    { kind: "agent_steer", ...card("queued"), outcome: "teleported" },
    { kind: "task_broadcast", ...card("queued") },
    "queued",
  ])("preserves detailed steer guards %#", (delivery_receipt) => {
    expect(
      present({ toolName: "agent_steer", extra: { delivery_receipt } })
        ?.stateLabel,
    ).toBe("Outcome unconfirmed");
  });

  test.each([
    0,
    -1,
    1.5,
    Number.NaN,
    Number.POSITIVE_INFINITY,
    Number.MAX_SAFE_INTEGER + 1,
    "30",
    {},
    [],
  ])("preserves TTL validation for %s", (ttl) => {
    expect(boardReceipt("agent_message", mail("queued", ttl))).toBeNull();
    expect(present({ extra: mail("queued", ttl) })?.stateLabel).toBe(
      "Outcome unconfirmed",
    );
  });

  test.each([undefined, null, 1, Number.MAX_SAFE_INTEGER])(
    "accepts supported TTL %s",
    (ttl) => {
      expect(present({ extra: mail("queued", ttl) })?.stateLabel).toBe(
        "Queued",
      );
    },
  );

  test.each([
    [
      "agent_message",
      { to: "child", text: "hello" },
      "Message queued for child.",
      "Queued",
      "Message queued",
    ],
    [
      "agent_message",
      { to: "parent", text: "hello" },
      "Note sent to parent.",
      "Note accepted",
      "Note accepted",
    ],
    [
      "agent_message",
      { to: "parent", text: "hello", expects_reply: true },
      "Question q-123 sent to parent.",
      "Question recorded",
      "Question recorded",
    ],
    [
      "agent_message",
      { to: "child", text: "hello", reply_to: "q-123" },
      "Answer recorded for finished agent child.",
      "Answer recorded",
      "Answer recorded",
    ],
    [
      "progress_report",
      { status_line: "done" },
      "Progress updated: done",
      "Progress recorded",
      "Progress recorded",
    ],
  ] as const)(
    "uses only existing exact legacy %s patterns",
    (toolName, args, result, compact, detail) => {
      expect(present({ toolName, arguments: args, result })?.stateLabel).toBe(
        compact,
      );
      expect(receiptState(toolName, args, result, null).label).toBe(detail);
      expect(
        present({
          toolName,
          arguments: args,
          result: `${result}\nEverything succeeded!`,
        })?.stateLabel,
      ).toBe("Outcome unconfirmed");
    },
  );

  test.each([
    [
      "agent_message",
      { to: "child", text: "hello" },
      "Message queued for other.",
    ],
    [
      "agent_message",
      { to: "parent", text: "hello" },
      "Question q-123 sent to parent.",
    ],
    [
      "agent_message",
      { to: "child", text: "hello" },
      "Answer recorded for finished agent child.",
    ],
    ["progress_report", { status_line: "old" }, "Progress updated: new"],
    [
      "agent_update",
      { card_id: "T-1", message: "done" },
      "Added status update to card T-1",
    ],
    [
      "agent_ask_planner",
      { question: "Which parser?" },
      "Question recorded for planner.",
    ],
    ["planner_reply", { answer: "Proceed" }, "Reply recorded and delivered"],
    ["task_broadcast", { message: "Pause" }, "Broadcast sent to 5 agents"],
    ["agent_message", {}, '{"status":"completed"}'],
    ["agent_message", {}, "success done verified delivered"],
  ] as const)(
    "unsupported %s prose stays neutral",
    (toolName, args, result) => {
      expect(present({ toolName, arguments: args, result })).toMatchObject({
        stateLabel: "Outcome unconfirmed",
        tone: "neutral",
      });
    },
  );

  test("a question receipt is not an answer or a blocking wait", () => {
    const output = present({
      arguments: { to: "parent", text: "Which parser?", expects_reply: true },
      result: "Question q-123 sent to parent.",
    });
    expect(output?.stateLabel).toBe("Question recorded");
    expect(output?.facts).toContainEqual({
      label: "Reply",
      value: "Requested; this receipt is not an answer",
    });
    expect(
      present({
        toolName: "agent_ask_planner",
        arguments: { question: "Which parser?", urgency: "block" },
      })?.stateLabel,
    ).not.toMatch(/blocked|waiting|answered/iu);
  });

  test("reported progress stays explicitly self-reported, including done prose", () => {
    const output = present({
      toolName: "progress_report",
      arguments: { status_line: "Done and verified" },
      result: "Progress updated: Done and verified",
    });
    expect(output).toMatchObject({
      headline: "Done and verified",
      stateLabel: "Progress recorded",
      tone: "neutral",
    });
    expect(output?.facts).toContainEqual({
      label: "Evidence",
      value: "Self-reported activity, not verified completion",
    });
  });

  test.each(["{bad", "[]", "null", 5, false])(
    "malformed arguments %s do not establish an outcome",
    (args) => {
      expect(
        present({ arguments: args, extra: mail("delivered") }),
      ).toMatchObject({
        headline: "Requested message",
        stateLabel: "Outcome unconfirmed",
        tone: "neutral",
      });
    },
  );

  test("malformed typed mail preserves only the existing exact text rule", () => {
    const extra = {
      delivery_receipt: {
        kind: "agent_message",
        delivery_id: "",
        outcome: "landed",
      },
    };
    expect(
      present({
        arguments: { to: "child", text: "hello" },
        result: "Message queued for child.",
        extra,
      })?.stateLabel,
    ).toBe("Queued");
    expect(present({ extra })?.stateLabel).toBe("Outcome unconfirmed");
    expect(present({ extra: mail("followup_saved") })?.stateLabel).toBe(
      "Outcome unconfirmed",
    );
  });

  test("prefers supplied recipient labels and shows a raw target only when needed", () => {
    expect(
      present({
        arguments: {
          text: "Review parser",
          to: "raw-agent-id",
          recipient_title: "Parser fixes",
        },
      })?.facts[0],
    ).toEqual({ label: "Recipient", value: "Parser fixes" });
    expect(
      present({ arguments: { text: "Review parser", to: "raw-agent-id" } })
        ?.facts[0],
    ).toEqual({ label: "Recipient", value: "raw-agent-id" });
  });

  test("bounds malicious text and all facts without parsing markup or promoting prose", () => {
    const hostile =
      '<script>alert("done")</script> [success](javascript:bad) ' +
      "🦊".repeat(500);
    const args = {
      text: hostile,
      recipient_title: hostile,
      expects_reply: true,
    };
    const output = present({ arguments: args, extra: mail("queued", 30) });
    expect(output?.headline).toContain('<script>alert("done")</script>');
    expect(output?.headline).toContain("javascript:bad");
    expect(Array.from(output?.headline ?? "")).toHaveLength(160);
    expect(output?.facts).toHaveLength(3);
    for (const fact of output?.facts ?? []) {
      expect(Array.from(fact.label).length).toBeLessThanOrEqual(48);
      expect(Array.from(fact.value).length).toBeLessThanOrEqual(240);
    }
    expect(output).toMatchObject({ stateLabel: "Queued", tone: "neutral" });
    expect(args.text).toBe(hostile);
  });
});
