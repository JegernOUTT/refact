import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";
import { AGENTIC_TOOL_NAMES, lookupAgenticFamily } from "./agenticFamilies";
import { compactBoardNotesPresentation } from "./compactBoardNotes";
import {
  COMPACT_HEADLINE_CHARS,
  COMPACT_VALUE_CHARS,
  type CompactAgenticInput,
} from "./compactPresentation";

const INTENTS = [
  [
    "board_create",
    { title: "Fix login", card_id: "T-1" },
    "Fix login — create card",
  ],
  ["board_update", { card_id: "T-1" }, "T-1 — update card"],
  ["board_move", { card_id: "T-1", column: "done" }, "T-1 — move to done"],
  ["board_delete", { card_id: "T-1" }, "T-1 — delete card"],
  ["board_get", { card_id: "T-1" }, "T-1 — read board"],
  [
    "board_create_batch",
    { cards: [{ card_id: "T-1", title: "Fix login" }, { card_id: "T-2" }] },
    "Fix login + 1 more — create cards",
  ],
  ["mark_done", { card_id: "T-1" }, "T-1 — record as done"],
  ["mark_failed", { card_id: "T-1" }, "T-1 — record failure"],
  ["mark_done_batch", { items: [{ card_id: "T-1" }] }, "T-1 — record as done"],
  [
    "mark_failed_batch",
    { items: [{ card_id: "T-1" }] },
    "T-1 — record failures",
  ],
  ["ready_cards", {}, "find ready work"],
  ["task_state", { note: "Release work" }, "Release work — read task state"],
  ["task_start", { name: "Release work" }, "Release work — start planning"],
  ["task_verify_card", { card_id: "T-1" }, "T-1 — request verification"],
  ["agent_complete", { card_id: "T-1" }, "T-1 — report completion"],
  ["agent_fail", { card_id: "T-1" }, "T-1 — report failure"],
  [
    "doc_create",
    { name: "Release plan", slug: "release" },
    "Release plan — create document",
  ],
  ["doc_update", { slug: "release" }, "release — replace document text"],
  ["doc_append", { slug: "release" }, "release — append section"],
  ["doc_delete", { slug: "release" }, "release — delete document"],
  ["doc_pin", { slug: "release", pinned: false }, "release — unpin document"],
  ["doc_history", { slug: "release" }, "release — read document history"],
  [
    "task_mem_save",
    { title: "Lock order", content: "Take outer lock first" },
    "Lock order — save note",
  ],
  ["task_mem_get", { namespace: "card:T-1" }, "card:T-1 — recall notes"],
  ["task_mem_search", { query: "locking" }, "locking — search notes"],
  ["task_mem_inbox", {}, "read note inbox"],
  ["task_mem_pin", { memory_id: "locks", pinned: false }, "locks — unpin note"],
  ["task_mem_archive", { memory_id: "locks" }, "locks — archive note"],
  ["task_mem_unarchive", { memory_id: "locks" }, "locks — restore note"],
  ["task_mem_triage_done", {}, "record inbox review"],
] as const;

function present(input: Partial<CompactAgenticInput> = {}) {
  return compactBoardNotesPresentation({
    toolName: "board_create",
    transportState: "succeeded",
    ...input,
  });
}

function document(version = "7", body = "## Release\nApproved: true") {
  return `---\nname: "Release plan"\nslug: "release"\nkind: "plan"\ncreated_at: "2026-10-24T00:00:00Z"\nupdated_at: "2026-10-25T00:00:00Z"\nauthor_role: "planner"\npinned: false\nversion: ${version}\n---\n\n${body}`;
}

const BATCH_ARGS = {
  cards: [{ card_id: "T-1" }, { card_id: "T-2" }],
};

const BATCH_RESULT = [
  { card_id: "T-1", success: true },
  { card_id: "T-2", success: false, error: "Card already exists" },
];

describe("compactBoardNotesPresentation", () => {
  test("covers exactly the current task, document and memory family inventory", () => {
    const names = AGENTIC_TOOL_NAMES.filter((name) =>
      ["task", "doc", "memory"].includes(
        lookupAgenticFamily(name)?.family ?? "",
      ),
    );
    expect(INTENTS.map(([name]) => name).sort()).toEqual(names.sort());
  });

  test.each(INTENTS)(
    "%s uses useful request intent",
    (toolName, args, expected) => {
      const presentation = present({
        toolName,
        arguments: args,
        transportState: "pending",
      });
      expect(presentation).toMatchObject({
        headline: expected,
        stateLabel: "Awaiting result",
        tone: "neutral",
      });
      expect(presentation?.facts.length).toBeLessThanOrEqual(3);
      expect(presentation?.headline).not.toContain(toolName);
      expect(
        present({
          toolName,
          arguments: JSON.stringify(args),
          transportState: "pending",
        }),
      ).toEqual(presentation);
    },
  );

  test.each(INTENTS)(
    "%s remains neutral for arbitrary success prose",
    (toolName, args) => {
      expect(
        present({
          toolName,
          arguments: args,
          result: "Success! Verified, saved, created, done and applied.",
        }),
      ).toMatchObject({
        stateLabel: "Outcome unconfirmed",
        tone: "neutral",
      });
    },
  );

  test.each(["T_BoardCreate", "BoardCreate", "board-create"])(
    "normalizes compatibility name %s locally",
    (toolName) => {
      expect(
        present({ toolName, arguments: { title: "Fix login" } })?.headline,
      ).toBe("Fix login — create card");
    },
  );

  test.each([
    "shell",
    "board_claim",
    "finalize_card",
    "doc_get",
    "doc_list",
    "agent_finish",
    "toString",
    "constructor",
    "__proto__",
  ])("leaves %s to its own adapter", (toolName) => {
    expect(present({ toolName })).toBeNull();
  });

  test("keeps transport failure, running and unknown above valid domain receipts", () => {
    for (const [transportState, stateLabel, tone] of [
      ["failed", "Request failed", "danger"],
      ["pending", "Awaiting result", "neutral"],
      ["running", "Running", "active"],
      ["unknown", "Outcome unknown", "neutral"],
    ] as const) {
      expect(
        present({
          toolName: "board_create_batch",
          arguments: BATCH_ARGS,
          result: BATCH_RESULT,
          transportState,
        }),
      ).toMatchObject({ stateLabel, tone, facts: [] });
    }
  });

  test("reports actual per-card batch receipts without hiding partial failure", () => {
    expect(
      present({
        toolName: "board_create_batch",
        arguments: BATCH_ARGS,
        result: JSON.stringify(BATCH_RESULT),
      }),
    ).toMatchObject({
      stateLabel: "Partial batch receipt",
      tone: "warning",
      facts: [
        { label: "Receipts returned", value: "2 of 2 requested" },
        { label: "Requests accepted", value: "1; 1 failed" },
      ],
    });
    expect(
      present({
        toolName: "board_create_batch",
        arguments: BATCH_ARGS,
        result: BATCH_RESULT.slice(0, 1),
      })?.facts[0].value,
    ).toBe("1 of 2 requested");
    expect(
      present({
        toolName: "board_create_batch",
        arguments: BATCH_ARGS,
        result: BATCH_RESULT.slice(0, 1),
      })?.stateLabel,
    ).toBe("Partial batch receipt");
  });

  test("complete creation receipt confirms only card creation", () => {
    const result = BATCH_ARGS.cards.map(({ card_id }) => ({
      card_id,
      success: true,
    }));
    expect(
      present({ toolName: "board_create_batch", arguments: BATCH_ARGS, result })
        ?.stateLabel,
    ).toBe("Cards created");
    expect(
      present({
        toolName: "board_update",
        arguments: { card_id: "T-1" },
        result,
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
  });

  test.each([
    [{ card_id: "unrelated", success: true }],
    [{ card_id: "T-1", success: "true" }],
    [{ card_id: "T-1", success: true, error: "Failed write" }],
    [{ card_id: "T-1", success: false }],
    [
      { card_id: "T-1", success: true },
      { card_id: "T-1", success: true },
    ],
    [null],
  ])("does not confirm malformed or unrelated batch %j", (...rows) => {
    expect(
      present({
        toolName: "board_create_batch",
        arguments: BATCH_ARGS,
        result: rows,
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
  });

  test("mark-done acceptance does not establish verification", () => {
    const args = { items: [{ card_id: "T-1", report: "Tests passed" }] };
    const presentation = present({
      toolName: "mark_done_batch",
      arguments: args,
      result: [{ card_id: "T-1", success: true }],
    });
    expect(presentation).toMatchObject({
      stateLabel: "Recorded; not verified",
      tone: "neutral",
      facts: expect.arrayContaining([
        { label: "Verification", value: "Not established by bookkeeping" },
      ]) as unknown,
    });
    expect(
      present({
        toolName: "mark_done",
        arguments: { card_id: "T-1" },
        result:
          "✅ **Card marked as done:** Fix login\n\n**Report:**\nTests passed",
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
  });

  test("mark-failed receipt does not imply tool failure or successful work", () => {
    const presentation = present({
      toolName: "mark_failed_batch",
      arguments: { items: [{ card_id: "T-1" }] },
      result: [{ card_id: "T-1", success: true }],
    });
    expect(presentation).toMatchObject({
      stateLabel: "Batch receipt",
      tone: "neutral",
    });
    expect(presentation?.headline).toBe("T-1 — record failures");
  });

  test("a malformed requested item cannot receive a successful placeholder receipt", () => {
    expect(
      present({
        toolName: "board_create_batch",
        arguments: { cards: [null] },
        result: [{ card_id: "<item 1>", success: true }],
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
    expect(
      present({
        toolName: "board_create_batch",
        arguments: { cards: [null] },
        result: [
          { card_id: "<item 1>", success: false, error: "Missing card_id" },
        ],
      })?.stateLabel,
    ).toBe("Partial batch receipt");
  });

  test("board snapshot reports only returned selection and exact revision", () => {
    const presentation = present({
      toolName: "board_get",
      arguments: { filter: { column: "planned" } },
      result: "rev: 42\ncards:\n- id: T-1\n  column: planned\n",
    });
    expect(presentation).toMatchObject({
      stateLabel: "Board received",
      facts: [
        { label: "Cards returned", value: "1" },
        { label: "Board revision", value: "42" },
        { label: "Coverage", value: "Returned selection only" },
      ],
    });
    expect(
      present({ toolName: "board_get", result: { cards: [], rev: 0 } })
        ?.facts[1].value,
    ).toBe("0");
    expect(
      present({
        toolName: "board_get",
        result: { cards: [], delta_revision: 8 },
      }),
    ).toMatchObject({ stateLabel: "Outcome unconfirmed", facts: [] });
  });

  test("card snapshot must match the requested card and cannot verify its work", () => {
    const result = {
      id: "T-1",
      title: "Fix login",
      column: "done",
      priority: "P1",
      verifier_report: { passed: true },
    };
    expect(
      present({ toolName: "board_get", arguments: { card_id: "T-1" }, result }),
    ).toMatchObject({ stateLabel: "Card received", tone: "neutral" });
    expect(
      present({ toolName: "board_get", arguments: { card_id: "T-2" }, result })
        ?.stateLabel,
    ).toBe("Outcome unconfirmed");
    expect(
      present({
        toolName: "board_move",
        arguments: { card_id: "T-1", column: "done" },
        result,
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
  });

  test("ready count comes from returned ready table, not unrelated blocked rows or heading", () => {
    const result =
      "# Ready Cards (99)\n\n| Card | Title | Priority | Depends On | Brief |\n|------|-------|----------|------------|-------|\n| T-1 | Fix login | P1 | None | Request |\n\n## Blocked\n| Card | Title |\n|------|-------|\n| T-2 | Wait |\n";
    expect(present({ toolName: "ready_cards", result })?.facts[0]).toEqual({
      label: "Ready cards returned",
      value: "1",
    });
  });

  test.each([
    "| version | updated_at | snapshot_at | state | path |",
    "| version | updated_at | snapshot_at | state | path |\n| broken |\n",
    "| version | updated_at | snapshot_at | state | path |\n|---:|---|---|---|---|\n| seven | at | at | history | /a |",
    "| version | updated_at | snapshot_at | state | path |\n|---:|---|---|---|---|\n| 7 | missing cells |",
  ])(
    "does not treat malformed history tables as known receipts: %s",
    (result) => {
      expect(present({ toolName: "doc_history", result })).toMatchObject({
        stateLabel: "Outcome unconfirmed",
        facts: [],
      });
    },
  );

  test.each(["doc_create", "doc_update", "doc_append", "doc_pin"])(
    "%s reads exact stored metadata without fabricating an applied operation",
    (toolName) => {
      expect(
        present({
          toolName,
          arguments: { slug: "release", pinned: false, version: 123 },
          result: document(),
        }),
      ).toMatchObject({
        stateLabel: "Document received",
        tone: "neutral",
        facts: [
          { label: "Version", value: "7" },
          { label: "Kind", value: "plan" },
          { label: "Pinned", value: "false" },
        ],
      });
    },
  );

  test("document versions remain exact, zero preserved and omissions not backfilled", () => {
    for (const [version, expected] of [
      ["0", "0"],
      ['"0007"', "0007"],
      ['"18446744073709551615"', "18446744073709551615"],
    ]) {
      expect(
        present({
          toolName: "doc_update",
          arguments: { slug: "release" },
          result: document(version),
        })?.facts[0].value,
      ).toBe(expected);
    }
    for (const result of [
      document().replace("version: 7\n", ""),
      document("9007199254740993"),
      document().replace('slug: "release"', 'slug: "other"'),
    ]) {
      expect(
        present({
          toolName: "doc_update",
          arguments: { slug: "release", version: 3 },
          result,
        }),
      ).toMatchObject({ stateLabel: "Outcome unconfirmed", facts: [] });
    }
    expect(
      present({
        toolName: "doc_delete",
        arguments: { slug: "release" },
        result: document(),
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
    expect(
      present({
        toolName: "doc_pin",
        arguments: { slug: "release", pinned: true },
        result: document(),
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
  });

  test("history counts retained rows, not the latest version or entire historical coverage", () => {
    const rows = Array.from(
      { length: 20 },
      (_, index) =>
        `| ${
          index + 40
        } | updated | snapshot | history | /history/${index}.md |`,
    );
    const result = [
      "| version | updated_at | snapshot_at | state | path |",
      "|---:|---|---|---|---|",
      ...rows,
    ].join("\n");
    expect(
      present({
        toolName: "doc_history",
        arguments: { slug: "release" },
        result,
      }),
    ).toMatchObject({
      stateLabel: "History received",
      facts: [
        { label: "Snapshots returned", value: "20" },
        { label: "Coverage", value: "Retained snapshots, not all versions" },
      ],
    });
  });

  test("actual saved-memory prose is not upgraded into a typed save outcome", () => {
    const presentation = present({
      toolName: "task_mem_save",
      arguments: {
        content: "Take outer lock first",
        kind: "gotcha",
        namespace: "card:T-1",
      },
      result:
        "Memory saved successfully.\nFile: /task/memories/locks.md\nTask: task-1\nRole: agents\nKind: gotcha\nNamespace: card:T-1",
    });
    expect(presentation).toMatchObject({
      headline: "Take outer lock first — save note",
      stateLabel: "Outcome unconfirmed",
      tone: "neutral",
      facts: [
        { label: "Requested kind", value: "gotcha" },
        { label: "Requested scope", value: "card:T-1" },
      ],
    });
  });

  test("search counts the returned page and retains historical trust", () => {
    const hit = {
      path: "/task/memories/locks.md",
      title: "Locks",
      kind: "gotcha",
      snippet: "Take outer lock first",
      score: 0.8,
      provenance: { trust: "untrusted-historical" },
    };
    expect(
      present({
        toolName: "task_mem_search",
        arguments: { query: "locking", top_k: 500 },
        result: JSON.stringify([hit]),
      }),
    ).toMatchObject({
      stateLabel: "Search results received",
      facts: [
        { label: "Matches returned", value: "1" },
        { label: "Coverage", value: "Returned page, not all stored notes" },
        { label: "Trust", value: "Historical notes; not verified facts" },
      ],
    });
    expect(
      present({ toolName: "task_mem_search", result: [] })?.facts[0].value,
    ).toBe("0");
    expect(
      present({ toolName: "task_mem_save", result: [hit] })?.stateLabel,
    ).toBe("Outcome unconfirmed");
    expect(
      present({
        toolName: "task_mem_search",
        result: [{ ...hit, provenance: {} }],
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
  });

  test.each([null, true, 42, [], "{", "[]", '{"title":', { title: {} }])(
    "handles malformed arguments %j without confirming a receipt",
    (args) => {
      expect(
        present({
          toolName: "doc_update",
          arguments: args,
          result: document(),
        }),
      ).toMatchObject({
        stateLabel: "Outcome unconfirmed",
        tone: "neutral",
        facts: [],
      });
    },
  );

  test.each([
    null,
    true,
    42,
    "{",
    "---\nversion: [\n---\n",
    { success: true, verified: true },
    { rev: 0, cards: [null] },
  ])("handles unknown result %j without inventing board facts", (result) => {
    expect(present({ toolName: "board_get", result })).toMatchObject({
      stateLabel: "Outcome unconfirmed",
      tone: "neutral",
      facts: [],
    });
  });

  test("does not use unrelated extra metadata as authority or success evidence", () => {
    expect(
      present({
        toolName: "mark_done",
        arguments: { card_id: "T-1" },
        extra: {
          success: true,
          verified: true,
          board: { title: "Other task" },
        },
      }),
    ).toMatchObject({
      headline: "T-1 — record as done",
      stateLabel: "Outcome unconfirmed",
    });
  });

  test("hostile HTML stays inert plain text and inputs are not mutated", () => {
    const args = Object.freeze({
      title: '<img src=x onerror="alert(1)">',
      note: "<script>danger()</script>",
    });
    const input = Object.freeze({
      toolName: "board_create",
      arguments: args,
      transportState: "pending" as const,
    });
    const presentation = compactBoardNotesPresentation(input);
    const markup = renderToStaticMarkup(
      createElement("span", null, presentation?.headline),
    );
    expect(markup).toContain("&lt;img");
    expect(markup).not.toContain("<img");
    expect(presentation?.facts[0].value).toBe(args.note);
    expect(input.arguments).toBe(args);
  });

  test("body text cannot replace document metadata or execute YAML tags", () => {
    expect(
      present({
        toolName: "doc_update",
        arguments: { slug: "release" },
        result: document("7", "version: 999\npinned: true"),
      })?.facts[0].value,
    ).toBe("7");
    expect(
      present({
        toolName: "doc_update",
        arguments: { slug: "release" },
        result: document().replace(
          'name: "Release plan"',
          "name: !!js/function 'function() { return 1; }'",
        ),
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
    expect(
      present({
        toolName: "doc_update",
        arguments: { slug: "release" },
        result: document().replace(
          'name: "Release plan"',
          `name: "${"x".repeat(20_000)}"`,
        ),
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
  });

  test("bounds Unicode and huge input without losing the requested verb", () => {
    const presentation = present({
      toolName: "task_mem_save",
      arguments: {
        content: "🐾".repeat(100_000),
        namespace: "範囲".repeat(50_000),
        pinned: false,
      },
      result: "saved ".repeat(100_000),
    });
    expect(Array.from(presentation?.headline ?? "").length).toBeLessThanOrEqual(
      COMPACT_HEADLINE_CHARS,
    );
    expect(presentation?.headline).toMatch(/… — save note$/u);
    expect(presentation?.headline).not.toMatch(
      /[\uD800-\uDBFF](?![\uDC00-\uDFFF])/u,
    );
    expect(presentation?.facts.length).toBeLessThanOrEqual(3);
    for (const fact of presentation?.facts ?? [])
      expect(Array.from(fact.value).length).toBeLessThanOrEqual(
        COMPACT_VALUE_CHARS,
      );
    expect(presentation?.stateLabel).toBe("Outcome unconfirmed");
    expect(
      present({
        toolName: "board_get",
        result: `rev: 0\ncards: []\n${" ".repeat(300_000)}`,
      })?.stateLabel,
    ).toBe("Outcome unconfirmed");
  });
});
