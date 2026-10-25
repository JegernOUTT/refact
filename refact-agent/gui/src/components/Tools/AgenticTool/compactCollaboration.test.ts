import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test, vi } from "vitest";
import { JSON_SCHEMA, load } from "js-yaml";
import { normalizeToolName } from "../../../utils/toolNameAliases";
import {
  AGENTIC_TOOL_NAMES,
  BOARD_RUNTIME_TOOL_NAMES,
  LOCAL_COLLABORATION_TOOL_NAMES,
  agenticCardLabel,
  hasDedicatedToolCard,
  lookupAgenticFamily,
} from "./agenticFamilies";
import { compactBoardNotesPresentation } from "./compactBoardNotes";
import { buildCompactCollaborationPresentation as present } from "./compactCollaboration";
import type { CompactAgenticInput } from "./compactPresentation";

vi.hoisted(() => {
  vi.resetModules();
});

vi.mock("js-yaml", async (importOriginal) => {
  const actual = await importOriginal<typeof import("js-yaml")>();
  return { ...actual, load: vi.fn(actual.load) };
});

const INVENTORY = [
  ["agent_mailbox", "inspect", "Mailbox check requested"],
  ["board_claim", "task", "Claim requested"],
  ["board_discover", "task", "Board discovery requested"],
  ["team_seat", "task", "Team seat change requested"],
  ["team_role_grant", "control", "Role grant requested"],
  ["finalize_card", "task", "Finalization requested"],
  ["claim_record", "memory", "Claim recording requested"],
  ["claim_list", "memory", "Claim lookup requested"],
  ["disagreement_file", "task", "Disagreement filing requested"],
  ["disagreement_list", "task", "Disagreement lookup requested"],
  ["disagreement_adjudicate", "task", "Adjudication requested"],
  ["causal_trace", "inspect", "Causal trace requested"],
  ["metrics", "overview", "Project metrics requested"],
  ["code_mode", "control", "Orchestration requested"],
] as const;

const CLAIM = {
  type: "task_board_claim",
  action: "claim",
  task_id: "task-1",
  card_id: "T-1",
  claimant_chat_id: "chat-1",
  idempotency_key: "retry-1",
  already_held: false,
  rev: 2,
};

const FINALIZE = {
  type: "task_card_finalize",
  task_id: "task-1",
  card_id: "T-1",
  finalized: true,
  decision: { blocks: [] },
};

const SEAT_ARGS = { team: "ship-it", role: "implementer", action: "claim" };
const SEAT = {
  ...SEAT_ARGS,
  member_chat_id: "chat-1",
  result: "claimed",
  outcome: null,
  members: [],
};

const GRANT_ARGS = {
  team: "ship-it",
  role: "implementer",
  authority: "mail",
  member_agent_id: "agent-1",
  recipient_chat_id: "chat-2",
};
const GRANT = { ...GRANT_ARGS, member_chat_id: null, recipient_agent_id: null };

const METRICS = {
  snapshot_id: "snapshot-1",
  seq: 0,
  project: "project-1",
  entries: [],
  total: 0,
  series_total: 0,
  explanations_total: 0,
  returned: 0,
  truncated: false,
  generated_at_ms: 1,
};

const OBSERVATIONS: CompactAgenticInput[] = [
  {
    toolName: "board_claim",
    arguments: { card_id: "T-1" },
    result: CLAIM,
    transportState: "succeeded",
  },
  {
    toolName: "finalize_card",
    arguments: { card_id: "T-1" },
    result: FINALIZE,
    transportState: "succeeded",
  },
  {
    toolName: "team_seat",
    arguments: SEAT_ARGS,
    extra: { team_seat: SEAT },
    transportState: "succeeded",
  },
  {
    toolName: "team_role_grant",
    arguments: GRANT_ARGS,
    extra: { team_role_grant: GRANT },
    transportState: "succeeded",
  },
  { toolName: "metrics", result: METRICS, transportState: "succeeded" },
  {
    toolName: "agent_mailbox",
    extra: {
      mailbox: { total: 0, offset: 0, limit: 20, next_offset: null, rows: [] },
    },
    transportState: "succeeded",
  },
];

describe("local collaboration presentation inventory", () => {
  test("exports exactly the fourteen current built-ins, not retired aliases", () => {
    expect(LOCAL_COLLABORATION_TOOL_NAMES).toEqual(
      INVENTORY.map(([name]) => name),
    );
    for (const name of ["board_claim", "board_discover", "finalize_card"])
      expect(BOARD_RUNTIME_TOOL_NAMES).toContain(name);
  });

  test.each(INVENTORY)(
    "%s resolves to the existing %s family",
    (name, family, headline) => {
      const pascal = name
        .split("_")
        .map((part) => part[0].toUpperCase() + part.slice(1))
        .join("");
      for (const toolName of [name, `t_${name}`, pascal]) {
        expect(AGENTIC_TOOL_NAMES).toContain(name);
        expect(lookupAgenticFamily(toolName)?.family).toBe(family);
        expect(hasDedicatedToolCard(toolName)).toBe(true);
        expect(agenticCardLabel(toolName)).not.toBeNull();
        expect(
          compactBoardNotesPresentation({
            toolName,
            transportState: "succeeded",
          }),
        ).toBeNull();
        expect(
          present({
            toolName,
            transportState: "succeeded",
            result: "All done, authorized and verified!",
          }),
        ).toEqual({
          headline,
          stateLabel: "Outcome unconfirmed",
          tone: "neutral",
          facts: [],
        });
      }
    },
  );

  test.each(
    AGENTIC_TOOL_NAMES.filter((name) =>
      ["task", "doc", "memory"].includes(
        lookupAgenticFamily(name)?.family ?? "",
      ),
    ),
  )(
    "%s has exactly one compact adapter owner across normalized aliases",
    (name) => {
      const pascal = name
        .split("_")
        .map((part) => part[0].toUpperCase() + part.slice(1))
        .join("");
      for (const toolName of [
        name,
        `t_${name}`,
        pascal,
        `T_${pascal}`,
        name.replace(/_/gu, "-"),
      ]) {
        expect(normalizeToolName(toolName), toolName).toBe(name);
        const input: CompactAgenticInput = {
          toolName,
          transportState: "pending",
        };
        const collaborationOwned =
          LOCAL_COLLABORATION_TOOL_NAMES.includes(name);
        expect(
          {
            boardNotes: compactBoardNotesPresentation(input) !== null,
            collaboration: present(input) !== null,
          },
          toolName,
        ).toEqual({
          boardNotes: !collaborationOwned,
          collaboration: collaborationOwned,
        });
      }
    },
  );

  test.each([
    ["spawn", "spawn_agent task_spawn_agent spawn_ab spawn_agents_batch"],
    [
      "message",
      "agent_message agent_peer_grant task_broadcast agent_steer agent_ask_planner planner_reply",
    ],
    [
      "overview",
      "agents_overview agent_list work_overview task_overview task_questions_list",
    ],
    ["progress", "progress_report agent_update"],
    [
      "inspect",
      "agent_status agent_wait agent_result work_inspect wait_agents agent_chat_summary",
    ],
    [
      "control",
      "agent_cancel agent_approval agent_resume pause_goal snooze_goal task_board_authority restart_agent resume_agent pause_agent cancel_agent assign_agent merge_agent merge_ready_in_order pick_ab_winner",
    ],
    ["goal", "set_goal update_goal get_goal validate_goal"],
    ["plan", "set_plan update_plan get_plan"],
    [
      "task",
      "task_state board_get board_create board_update board_move board_delete ready_cards mark_done mark_failed agent_complete agent_fail board_create_batch mark_done_batch mark_failed_batch task_start task_verify_card",
    ],
    ["doc", "doc_create doc_update doc_append doc_delete doc_pin doc_history"],
    [
      "memory",
      "task_mem_save task_mem_get task_mem_search task_mem_inbox task_mem_pin task_mem_archive task_mem_unarchive task_mem_triage_done",
    ],
  ])("preserves original %s mappings", (family, names) => {
    for (const toolName of names.split(" ")) {
      expect(lookupAgenticFamily(toolName)?.family, toolName).toBe(family);
      expect(
        present({ toolName, transportState: "succeeded" }),
        toolName,
      ).toBeNull();
    }
  });

  test.each([
    "cat",
    "shell",
    "code_health",
    "mcp_team_seat",
    "team_create",
    "claim",
    "delegate",
    "subagent",
    "constructor",
    "__proto__",
    "",
  ])("leaves %s outside both adapters", (toolName) => {
    expect(present({ toolName, transportState: "succeeded" })).toBeNull();
    expect(
      compactBoardNotesPresentation({
        toolName,
        transportState: "succeeded",
      }),
    ).toBeNull();
  });
});

describe("evidence-limited collaboration receipts", () => {
  test.each(INVENTORY)(
    "%s ignores unknown and malformed success schemas",
    (toolName) => {
      for (const result of [
        undefined,
        null,
        "",
        "{bad",
        [],
        "[]",
        true,
        1,
        {
          success: true,
          authorized: true,
          finalized: true,
          granted: true,
          status: "completed",
        },
      ]) {
        const view = present({
          toolName,
          arguments: "{bad",
          result,
          transportState: "succeeded",
        });
        expect(view?.stateLabel).toBe("Outcome unconfirmed");
        expect(view?.tone).toBe("neutral");
        expect(view?.headline).toContain("requested");
      }
    },
  );

  test.each(OBSERVATIONS)(
    "recognizes the narrow $toolName receipt",
    (input) => {
      const view = present(input);
      expect(view?.stateLabel).not.toBe("Outcome unconfirmed");
      expect(view?.tone).toBe("neutral");
      expect(view?.headline).not.toBe(input.toolName);
      expect(view?.headline).not.toMatch(
        /agent_mailbox|board_claim|team_role_grant|code_mode/u,
      );
      for (const transportState of [
        "pending",
        "running",
        "failed",
        "unknown",
      ] as const) {
        const gated = present({ ...input, transportState });
        expect(gated?.headline).toContain("requested");
        expect(gated?.stateLabel).toBe(
          {
            pending: "Awaiting result",
            running: "Running",
            failed: "Request failed",
            unknown: "Outcome unknown",
          }[transportState],
        );
      }
    },
  );

  test("records claim, retry and release without implying execution or verification", () => {
    const input = OBSERVATIONS[0];
    expect(present({ ...input, result: JSON.stringify(CLAIM) })?.headline).toBe(
      "Card claim recorded: T-1",
    );
    expect(
      present({ ...input, result: { ...CLAIM, already_held: true } })?.headline,
    ).toBe("Card claim already held: T-1");
    expect(
      present({
        ...input,
        arguments: { card_id: "T-1", release: true },
        result: { ...CLAIM, action: "release" },
      })?.headline,
    ).toBe("Card claim released: T-1");
    for (const argumentsValue of [
      { card_id: "T-other" },
      { card_id: "T-1", release: true },
      { card_id: "T-1", release: "false" },
      { card_id: "T-1", task_id: "other" },
      { card_id: "T-1", idempotency_key: "other" },
    ]) {
      expect(present({ ...input, arguments: argumentsValue })?.stateLabel).toBe(
        "Outcome unconfirmed",
      );
    }
    expect(
      present({ ...input, result: { ...CLAIM, already_held: "true" } })
        ?.stateLabel,
    ).toBe("Outcome unconfirmed");
  });

  test("distinguishes finalization refusal and rejects contradictory or untagged success", () => {
    const input = OBSERVATIONS[1];
    const blocks = [{ kind: "missing_evidence", fragment: "tests" }];
    expect(
      present({
        ...input,
        result: { ...FINALIZE, finalized: false, decision: { blocks } },
      }),
    ).toMatchObject({
      headline: "Card finalization refused: T-1",
      stateLabel: "Requirements unmet",
      tone: "warning",
      facts: [{ label: "Unmet requirements", value: "1" }],
    });
    for (const result of [
      { finalized: true },
      { ...FINALIZE, decision: { blocks } },
      { ...FINALIZE, finalized: "true" },
      { ...FINALIZE, card_id: "T-other" },
    ])
      expect(present({ ...input, result })?.stateLabel).toBe(
        "Outcome unconfirmed",
      );
    expect(present(input)?.stateLabel).toBe("Finalization receipt");
    expect(present(input)?.headline).not.toMatch(/verified|merged|pushed/iu);
  });

  test.each([
    ["claim", "already held", "Team seat claim recorded"],
    ["reclaim", "claimed", "Team seat claim recorded"],
    ["heartbeat", "heartbeat recorded", "Seat heartbeat recorded"],
    ["release", "released", "Team seat release recorded"],
    ["record_outcome", "outcome recorded", "Seat outcome recorded"],
  ])(
    "team seat %s remains a receipt, never authority",
    (action, result, headline) => {
      const args = { ...SEAT_ARGS, action, outcome: "verified" };
      const view = present({
        toolName: "team_seat",
        arguments: args,
        extra: { team_seat: { ...SEAT, ...args, result } },
        transportState: "succeeded",
      });
      expect(view?.headline).toBe(`${headline}: ship-it`);
      expect(view?.tone).toBe("neutral");
      expect(view?.stateLabel).toBe(
        action === "record_outcome" ? "Self-reported outcome" : "Seat receipt",
      );
      expect(
        present({
          toolName: "team_seat",
          arguments: args,
          extra: { team_seat: { ...SEAT, ...args, result: "authorized" } },
          transportState: "succeeded",
        })?.stateLabel,
      ).toBe("Outcome unconfirmed");
    },
  );

  test("a role grant receipt establishes no current permission or action", () => {
    const input = OBSERVATIONS[3];
    expect(present(input)).toMatchObject({
      headline: "Role grant receipt recorded",
      stateLabel: "Historical grant receipt",
      tone: "neutral",
      facts: [
        { label: "Team", value: "ship-it" },
        { label: "Role", value: "implementer" },
        {
          label: "Authority",
          value: "Historical receipt; not current permission",
        },
      ],
    });
    for (const grant of [
      { ...GRANT, authority: "control" },
      { ...GRANT, recipient_chat_id: "other" },
      { ...GRANT, member_chat_id: "both" },
      { granted: true },
    ])
      expect(
        present({ ...input, extra: { team_role_grant: grant } })?.stateLabel,
      ).toBe("Outcome unconfirmed");
    expect(
      present({ ...input, extra: undefined, result: GRANT })?.stateLabel,
    ).toBe("Outcome unconfirmed");
  });

  test("mailbox and metrics show bounded snapshot counts, not delivery or verification", () => {
    expect(present(OBSERVATIONS[5])?.facts).toEqual([
      { label: "Deliveries shown", value: "0" },
      { label: "Visible total", value: "0" },
      { label: "Offset", value: "0" },
    ]);
    expect(present(OBSERVATIONS[4])?.facts).toEqual([
      { label: "Entries shown", value: "0" },
      { label: "Metric series", value: "0" },
      { label: "Decision explanations", value: "0" },
    ]);
    expect(
      present({ ...OBSERVATIONS[4], result: { ...METRICS, returned: 1 } })
        ?.stateLabel,
    ).toBe("Outcome unconfirmed");
    expect(
      present({
        ...OBSERVATIONS[5],
        extra: {
          mailbox: { total: 0, offset: 0, limit: 1, rows: ["consumed"] },
        },
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
  });

  test("board discovery reads real YAML without granting board control", () => {
    const input: CompactAgenticInput = {
      toolName: "board_discover",
      transportState: "succeeded",
    };
    expect(
      present({
        ...input,
        result: "projection_unavailable: false\ncards: []\n",
      }),
    ).toMatchObject({
      headline: "Board snapshot checked",
      stateLabel: "Metadata only",
      tone: "neutral",
    });
    expect(
      present({
        ...input,
        result: "projection_unavailable: true\ncards: []\n",
      }),
    ).toMatchObject({
      headline: "Board discovery unavailable",
      stateLabel: "Projection unavailable",
      tone: "warning",
    });
    expect(
      present({
        ...input,
        arguments: { task_id: "task-1" },
        result:
          "task_id: task-1\nrev: 1\nowner_chat_id: owner\nreviewer_chat_ids: []\nreview_policy: independent_required\nclaims: {}\ndependency_overrides: {}\n",
      }),
    ).toMatchObject({
      headline: "Board roster checked",
      stateLabel: "Metadata only",
      tone: "neutral",
    });
    expect(
      present({
        ...input,
        arguments: { task_id: "task-1" },
        result: "projection_unavailable: false\ncards: []\n",
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
    expect(
      present({
        ...input,
        arguments: { task_id: "task-1" },
        result: {
          task_id: "task-1",
          rev: 1,
          reviewer_chat_ids: [],
          review_policy: {},
          claims: {},
          dependency_overrides: {},
        },
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
    for (const result of [
      "!!js/function '() => alert(1)'",
      "{broken",
      "projection_unavailable: 'false'\ncards: []",
      "- card: T-1",
    ])
      expect(present({ ...input, result })?.stateLabel).toBe(
        "Outcome unconfirmed",
      );
  });

  test("oversized board discovery skips YAML parsing and preserves the caller payload", () => {
    const result = "projection_unavailable: false\ncards: []\n#".padEnd(
      256_001,
      "x",
    );
    const input = Object.freeze({
      toolName: "board_discover",
      result,
      transportState: "succeeded" as const,
    });
    vi.mocked(load).mockClear();
    expect(present(input)).toMatchObject({
      headline: "Board discovery requested",
      stateLabel: "Outcome unconfirmed",
      tone: "neutral",
      facts: [],
    });
    expect(load).not.toHaveBeenCalled();
    expect(input.result).toBe(result);
    expect(input.result.length).toBe(256_001);
  });

  test("board discovery accepts the exact size boundary with the same safe YAML schema", () => {
    const result = "projection_unavailable: false\ncards: []\n#".padEnd(
      256_000,
      "x",
    );
    vi.mocked(load).mockClear();
    expect(
      present({
        toolName: "board_discover",
        result,
        transportState: "succeeded",
      }),
    ).toMatchObject({
      headline: "Board snapshot checked",
      stateLabel: "Metadata only",
      facts: [{ label: "Cards shown", value: "0" }],
    });
    expect(load).toHaveBeenCalledExactlyOnceWith(result, {
      schema: JSON_SCHEMA,
    });
    expect(result.length).toBe(256_000);
  });

  test.each([
    ["claim_record", "Claim recorded.\nClaim `c-1` [active]"],
    ["disagreement_file", "Disagreement filed."],
    ["disagreement_adjudicate", "Disagreement adjudicated."],
    ["causal_trace", "Replay reproduced: yes (0 mismatch(es))."],
    [
      "code_mode",
      "code_mode authorized 1 host calls\n- claim (board_claim) — ok",
    ],
  ])("%s prose never mints success", (toolName, result) => {
    expect(
      present({ toolName, result, transportState: "succeeded" }),
    ).toMatchObject({ stateLabel: "Outcome unconfirmed", tone: "neutral" });
  });
});

describe("collaboration failure and inert text", () => {
  test.each([
    ["already_claimed", "Claim unavailable"],
    ["scope_denied", "Request denied"],
    ["policy_unavailable", "Policy unavailable"],
    ["unavailable", "Unavailable"],
    ["dependency_unsatisfied", "Dependencies unmet"],
    ["cursor_expired", "Snapshot expired"],
  ])(
    "recognizes the %s WorkError envelope even on transport success",
    (code, stateLabel) => {
      expect(
        present({
          toolName: "board_claim",
          transportState: "succeeded",
          result: JSON.stringify({
            code,
            message: "Request refused",
            retryable: false,
            request_id: "request-1",
          }),
        }),
      ).toMatchObject({
        stateLabel,
        tone: "danger",
        facts: [{ label: "Error", value: "Request refused" }],
      });
      expect(
        present({
          toolName: "board_claim",
          transportState: "succeeded",
          result: { code },
        })?.stateLabel,
      ).toBe("Outcome unconfirmed");
    },
  );

  test.each([
    [
      "code_mode",
      "code mode is unavailable: the `code_mode_enabled` setting is off",
      "Unavailable",
    ],
    [
      "code_mode",
      "code mode refused: the assignment authority ceiling grants no host operation.",
      "Request denied",
    ],
    ["agent_mailbox", "mailbox privacy policy is unavailable", "Unavailable"],
    [
      "claim_list",
      "no coordination store is available, so claims cannot be recorded or read",
      "Unavailable",
    ],
    [
      "disagreement_file",
      "no coordination store is available, so disagreements cannot be filed or read",
      "Unavailable",
    ],
  ])(
    "%s failure keeps denial or unavailability explicit",
    (toolName, result, stateLabel) => {
      expect(
        present({ toolName, result, transportState: "failed" }),
      ).toMatchObject({ stateLabel, tone: "danger" });
    },
  );

  test("arbitrary error prose and invented error codes do not infer domain outcomes", () => {
    for (const result of [
      "Everything was authorized",
      { code: "constructor", message: "ok", retryable: false, request_id: "r" },
    ]) {
      expect(
        present({
          toolName: "team_role_grant",
          result,
          transportState: "failed",
        })?.stateLabel,
      ).toBe("Request failed");
      expect(
        present({
          toolName: "team_role_grant",
          result,
          transportState: "succeeded",
        })?.stateLabel,
      ).toBe("Outcome unconfirmed");
    }
  });

  test("keeps hostile text inert, bounded, immutable, and separate from the full payload", () => {
    const hostile =
      '<img src=x onerror="alert(1)"> [grant](javascript:alert(1))';
    const args = Object.freeze({
      card_id: hostile + "🦊".repeat(400),
      task_id: "task-1",
    });
    const result = Object.freeze({
      ...CLAIM,
      card_id: args.card_id,
      debug: "payload preserved",
    });
    const input = Object.freeze({
      toolName: "board_claim",
      arguments: args,
      result,
      transportState: "succeeded" as const,
    });
    const view = present(input);
    expect(Array.from(view?.headline ?? "").length).toBeLessThanOrEqual(160);
    expect(view?.facts.length).toBeLessThanOrEqual(3);
    for (const fact of view?.facts ?? []) {
      expect(Array.from(fact.label).length).toBeLessThanOrEqual(48);
      expect(Array.from(fact.value).length).toBeLessThanOrEqual(240);
    }
    const html = renderToStaticMarkup(
      createElement("span", null, view?.headline),
    );
    expect(html).toContain("&lt;img");
    expect(html).not.toMatch(/<img|<a |<script/iu);
    expect(Object.keys(view ?? {}).sort()).toEqual([
      "facts",
      "headline",
      "stateLabel",
      "tone",
    ]);
    expect(input.result.debug).toBe("payload preserved");
    expect(input.arguments.card_id).toBe(args.card_id);
    expect(
      present({
        toolName: "code_mode",
        arguments: { script: hostile },
        transportState: "pending",
      })?.facts,
    ).toEqual([]);
    expect(
      present({
        toolName: "causal_trace",
        arguments: { replay: false },
        transportState: "pending",
      })?.facts,
    ).toEqual([{ label: "Replay requested", value: "false" }]);
  });
});
