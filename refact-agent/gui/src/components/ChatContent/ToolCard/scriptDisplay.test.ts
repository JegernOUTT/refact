import { describe, expect, it } from "vitest";
import { pythonCall } from "../../../__tests__/pythonFixtures";
import type {
  PythonCallRecord,
  PythonRunExtra,
} from "../../../services/refact/types";
import {
  CODE_FOLDED_MAX_ROWS,
  CODE_LINE_MAX_CHARS,
  CODE_PREVIEW_MAX_CHARS,
  codeTitleLine,
  cutCodeLine,
  foldAnchors,
  htmlLineTokens,
  isInsideDir,
  pathKey,
  previewTitle,
  type CallStatusKey,
  aggregateCallStatus,
  calleeAtColumn,
  callDiffStats,
  foldCodeRows,
  lexicalCallees,
  limitSentence,
  placeCalls,
  runSummary,
  runTitle,
  scriptArgs,
  scriptArtifacts,
  scriptChanges,
  stateLines,
  scriptStatusView,
  scriptTitle,
  splitDisplayPath,
} from "./pythonDisplay";

const call = (patch: Partial<PythonCallRecord>): PythonCallRecord => ({
  ...pythonCall,
  ...patch,
});
const chunk = (file: string, add = "a\n") => ({
  file_name: file,
  file_action: "edit",
  line1: 1,
  line2: 1,
  lines_remove: "",
  lines_add: add,
});
const lines = (count: number) =>
  Array.from({ length: count }, (_, index) => `x${index + 1} = ${index}`);

/** Shared with the engine's `code_preview` tests: code, expected title line. */
export const TITLE_VECTORS: [string, string | undefined][] = [
  ["# Count TODO markers\nimport os\nx = 1", "# Count TODO markers"],
  [
    "#!/usr/bin/env python\nimport os\nfrom a import b\n\nx = compute()\n",
    "x = compute()",
  ],
  [
    "import os\n\ndef f():\n    return 1\n\nresult = f()\nprint(result)",
    "result = f()",
  ],
  ["import os\nimport json", "import os"],
  ["#\n#  \nvalue = 1", "value = 1"],
  ["   \n\t\n", undefined],
  ["# only a comment", "# only a comment"],
  ["## Heading style\nx=1", "## Heading style"],
  ['"""Docstring"""\nimport os\nrows = load()', "rows = load()"],
  ["@decorator\ndef f(): pass\nawait f()", "await f()"],
  ["if x:\n    y = 1", "if x:"],
  ["# Title\r\nx = 1\r\n", "# Title"],
  ["x = 1  # trailing", "x = 1  # trailing"],
  ["  # indented comment\nx = 1", "# indented comment"],
  ["async def main():\n    pass\nawait main()", "await main()"],
  ["# a\n# b", "# a"],
  ["#!/bin/sh\n# note", undefined],
];

describe("codeTitleLine", () => {
  it.each(TITLE_VECTORS)("%j", (code, expected) => {
    expect(codeTitleLine(code)).toBe(expected);
  });
});

describe("scriptTitle", () => {
  it("prefers the description, collapsed to one plain line", () => {
    expect(
      scriptTitle({ description: "  Count\n  **TODO**  ", code: "x = 1" }),
    ).toEqual({ text: "Count **TODO**", mono: false });
  });
  it("falls back from an empty description to the stored one", () => {
    expect(
      scriptTitle({ description: " ", storedDescription: "Stored", code: "x" }),
    ).toEqual({ text: "Stored", mono: false });
  });
  it("uses a leading comment in plain font, statements in monospace", () => {
    expect(scriptTitle({ code: "# Survey files\nx = 1" })).toEqual({
      text: "Survey files",
      mono: false,
    });
    expect(scriptTitle({ code: "import os\nresult = 1" })).toEqual({
      text: "result = 1",
      mono: true,
    });
  });
  it("gives the label for comment-only code", () => {
    expect(scriptTitle({ code: "#!/bin/sh\n# note" }).text).toBe("Run script");
    expect(scriptTitle({ code: "## Heading style\nx=1" })).toEqual({
      text: "Heading style",
      mono: false,
    });
  });
  it("matches the engine preview of the same code", () => {
    for (const [code, line] of TITLE_VECTORS) {
      if (line === undefined) continue;
      expect(scriptTitle({ code })).toEqual(previewTitle(line));
      expect(scriptTitle({ codePreview: line })).toEqual(scriptTitle({ code }));
    }
  });
  it("cuts the code line to the preview length on a code-point boundary", () => {
    const title = scriptTitle({ code: `x = "${"😀".repeat(200)}"` });
    expect(Array.from(title.text)).toHaveLength(CODE_PREVIEW_MAX_CHARS);
    expect(title.text.endsWith("…")).toBe(true);
    expect(title.text).not.toMatch(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])/);
  });
  it("never mentions Python on its own", () => {
    expect(scriptTitle({}).text).not.toMatch(/python/i);
  });
});

describe("scriptStatusView", () => {
  it("maps statuses to shared labels and transcript states", () => {
    expect(scriptStatusView("cancelled")).toMatchObject({
      label: "Stopped",
      transcript: "cancelled",
    });
    expect(scriptStatusView("completed").transcript).toBe("done");
    expect(scriptStatusView("waiting").transcript).toBe("needs_you");
    expect(scriptStatusView("limit_exceeded").label).toBe("Limit exceeded");
    expect(scriptStatusView("paused_by_policy").label).toBe("Unknown status");
  });
});

describe("scriptArgs", () => {
  it("reads code and description, marking cut code", () => {
    expect(scriptArgs('{"code":"x = 1","description":"d"}')).toEqual({
      code: "x = 1",
      description: "d",
      cut: false,
    });
    const cut = scriptArgs('{"code":"print(1)\\nprint(2');
    expect(cut.cut).toBe(true);
    expect(cut.code).toBe("print(1)\nprint(2");
  });
});

describe("code view model", () => {
  it("places calls on valid lines only", () => {
    const placed = placeCalls(
      [call({ seq: 2, line: 2 }), call({ seq: 1, line: 9 }), call({ seq: 3 })],
      3,
    );
    expect([...placed.byLine.keys()]).toEqual([2]);
    expect(placed.unplaced.map((item) => item.seq)).toEqual([1, 3]);
  });
  it("aggregates the most significant status of a line", () => {
    const calls = [
      call({ call_id: "a", status: "ok" }),
      call({ call_id: "b", status: "failed" }),
    ];
    expect(aggregateCallStatus(calls)).toBe("failed");
    expect(
      aggregateCallStatus(
        [...calls, call({ call_id: "c", status: "running" })],
        new Set(["c"]),
      ),
    ).toBe("waiting");
  });
  it("finds known tool callees lexically, ignoring comments and methods", () => {
    const tools = new Set(["cat", "shell"]);
    const line = 'r = await cat(paths="a") + x.shell() # shell(';
    expect(lexicalCallees(line, tools)).toEqual([{ start: 10, end: 13 }]);
    expect(lexicalCallees("cat = 1", tools)).toEqual([]);
  });
  it.each([
    ['é = await shell(command="x")', 10],
    ['😀 = await shell(command="x")', 10],
    ['\t\tr = await shell(command="x")', 12],
    ['"😀😀" + await shell()', 13],
  ])("locates the callee from a code-point column in %j", (line, col) => {
    const range = calleeAtColumn(line, col);
    expect(line.slice(range?.start, range?.end)).toBe("shell");
  });
  it("finds no callee past the end of the line", () => {
    expect(calleeAtColumn("x = 1", 40)).toBeUndefined();
  });
  it("finds callees in a 1 MB line without scanning past the shown part", () => {
    const line = `r = await shell(command="${"x".repeat(1_000_000)}")`;
    expect(calleeAtColumn(line, 10)).toEqual({ start: 10, end: 15 });
    expect(cutCodeLine(line).shown).toHaveLength(CODE_LINE_MAX_CHARS);
    expect(cutCodeLine(line).hidden).toBe(line.length - CODE_LINE_MAX_CHARS);
    expect(lexicalCallees(line, new Set(["shell"]))).toEqual([
      { start: 10, end: 15 },
    ]);
  });
  it("cuts a line on a code-point boundary", () => {
    const line = `${"a".repeat(CODE_LINE_MAX_CHARS - 1)}😀tail`;
    const { shown } = cutCodeLine(line);
    expect(shown).toHaveLength(CODE_LINE_MAX_CHARS - 1);
  });
  it("shows short code whole", () => {
    expect(foldCodeRows(lines(10), [3])).toHaveLength(10);
  });
  it("keeps anchors, the error line and the last line with fold rows", () => {
    const rows = foldCodeRows(lines(30), [10, 20]);
    expect(rows.filter((row) => row.kind === "line")).toEqual([
      { kind: "line", number: 10 },
      { kind: "line", number: 20 },
      { kind: "line", number: 30 },
    ]);
    expect(rows.filter((row) => row.kind === "fold")).toHaveLength(3);
  });
  it("shows the first lines with one fold without anchors", () => {
    const rows = foldCodeRows(lines(25), []);
    expect(rows).toHaveLength(9);
    expect(rows.at(-1)).toEqual({ kind: "fold", from: 9, to: 25 });
  });
  it("keeps the most important lines when anchors exceed the budget", () => {
    const callLines = Array.from({ length: 40 }, (_, i) => i * 2 + 1);
    const lineStatus = new Map<number, CallStatusKey>(
      callLines.map((line) => [line, "ok"]),
    );
    lineStatus.set(71, "waiting");
    lineStatus.set(73, "running");
    lineStatus.set(75, "failed");
    lineStatus.set(77, "denied");
    const anchors = foldAnchors({ errorLine: 88, lineStatus, callLines });
    expect(anchors.slice(0, 5)).toEqual([88, 71, 73, 75, 77]);
    const rows = foldCodeRows(lines(90), anchors);
    const shown = rows.flatMap((row) =>
      row.kind === "line" ? [row.number] : [],
    );
    expect(shown).toHaveLength(CODE_FOLDED_MAX_ROWS);
    for (const line of [88, 71, 73, 75, 77, 1, 3, 5])
      expect(shown).toContain(line);
    expect(shown).not.toContain(79);
    expect(shown).not.toContain(90);
  });
  it("never folds the error line away, whatever else competes for rows", () => {
    const callLines = Array.from({ length: 60 }, (_, i) => i + 1);
    const lineStatus = new Map<number, CallStatusKey>(
      callLines.map((line) => [
        line,
        line % 3 === 0 ? "waiting" : line % 3 === 1 ? "running" : "failed",
      ]),
    );
    for (const errorLine of [1, 45, 59, 200, 399]) {
      const anchors = foldAnchors({ errorLine, lineStatus, callLines });
      expect(anchors[0]).toBe(errorLine);
      const shown = foldCodeRows(lines(400), anchors).flatMap((row) =>
        row.kind === "line" ? [row.number] : [],
      );
      expect(shown).toContain(errorLine);
    }
  });
  it("counts a line's diff stats like the Changes row", () => {
    expect(
      callDiffStats([
        call({ diffs: [chunk("/p/a.py", "# T\n\n- a\n- b\n\n")] }),
      ]),
    ).toBe("+5");
    expect(
      callDiffStats([
        call({ diffs: [{ ...chunk("/p/a.py", "x\n"), lines_remove: "y\n" }] }),
      ]),
    ).toBe("+1 −1");
    expect(callDiffStats([call({})])).toBeUndefined();
  });
});

describe("changes and artifacts", () => {
  const base = {
    calls: [
      call({ diffs: [chunk("/p/src/a.py")] }),
      call({ seq: 2, diffs: [chunk("/p/src/a.py"), chunk("/p/b.md")] }),
    ],
    files_written: [
      { path: "/p/src/a.py", bytes: 5, diffs: [] },
      { path: "/w/artifacts/out.json", bytes: 51, diffs: [] },
    ],
    artifacts: [{ path: "/w/artifacts/out.json", bytes: 51 }],
  };
  it("lists every changed file once and leaves artifacts out", () => {
    const changes = scriptChanges(base, undefined, "/w/artifacts");
    expect(changes.map((change) => change.path)).toEqual([
      "/p/src/a.py",
      "/p/b.md",
    ]);
    expect(changes[0].chunks).toHaveLength(1);
  });
  it("lists artifacts once", () => {
    expect(scriptArtifacts(base, "/w/artifacts")).toEqual([
      { path: "/w/artifacts/out.json", bytes: 51 },
    ]);
  });
  it("normalises dot segments and Windows paths", () => {
    expect(isInsideDir("/w/artifacts/../repo/x.py", "/w/artifacts")).toBe(
      false,
    );
    expect(isInsideDir("/w/artifacts/./a/b.png", "/w/artifacts")).toBe(true);
    expect(pathKey("C:\\Repo\\src\\a.py")).toBe(pathKey("c:/repo/src/a.py"));
    const changes = scriptChanges(
      {
        calls: [],
        files_written: [
          { path: "C:\\Repo\\a.py", bytes: 1, diffs: [] },
          { path: "c:/repo/a.py", bytes: 2, diffs: [] },
          { path: "c:/repo/lib/a.py", bytes: 3, diffs: [] },
        ],
      },
      undefined,
      undefined,
    );
    expect(changes.map((change) => change.path)).toEqual([
      "C:\\Repo\\a.py",
      "c:/repo/lib/a.py",
    ]);
  });
  it("splits paths basename first", () => {
    expect(splitDisplayPath("/p/src/models/a.py", ["/p"])).toEqual({
      name: "a.py",
      dir: "src/models",
    });
  });
});

describe("limitSentence", () => {
  const extra = (
    state: Partial<PythonRunExtra["state"]>,
    type = "OutputLimit",
  ) =>
    ({
      error: { type, message: "", traceback: "" },
      state: {
        restored: false,
        reset_reason: null,
        persisted: false,
        bytes: 0,
        ...state,
      },
    }) as PythonRunExtra;
  it("names the limit once and keeps the engine's sentence", () => {
    expect(
      limitSentence(
        extra({
          reset_reason:
            "run limit exceeded: the session was restored to the state saved before this run; variables set in this run are lost",
        }),
      ),
    ).toBe(
      "Output limit exceeded: the session was restored to the state saved before this run; variables set in this run are lost.",
    );
    expect(limitSentence(extra({}, "MemoryError"))).toMatch(
      /^Run limit exceeded: the session was lost/,
    );
  });
});

describe("stateLines", () => {
  const extra = (
    state: Partial<PythonRunExtra["state"]>,
    status: PythonRunExtra["status"] = "completed",
  ) => ({
    status,
    state: {
      restored: false,
      reset_reason: null,
      persisted: false,
      bytes: 0,
      ...state,
    },
  });
  it("turns the engine's not-saved clause into one warning sentence", () => {
    expect(
      stateLines(
        extra({
          not_saved:
            "not saved (disk full); variables come from the last successful save",
        }),
      ),
    ).toEqual({
      notSaved:
        "State not saved (disk full); variables come from the last successful save.",
      notes: [],
    });
    expect(stateLines(extra({ not_saved: "disk full" })).notSaved).toBe(
      "State not saved: disk full.",
    );
  });
  const cases: [string, Partial<PythonRunExtra["state"]>, string[]][] = [
    ["neither", {}, []],
    [
      "a note",
      { note: "the session holds 18.0 MB of variables" },
      ["The session holds 18.0 MB of variables."],
    ],
    [
      "notes",
      { notes: ["variables were not copied from chat a: busy", "second."] },
      ["Variables were not copied from chat a: busy.", "Second."],
    ],
    ["both", { note: "large", notes: ["forked"] }, ["Large.", "Forked."]],
    ["blank ones", { note: " ", notes: ["", "kept"] }, ["Kept."]],
  ];
  it.each(cases)("lists %s one per line", (_, state, notes) => {
    expect(stateLines(extra(state))).toEqual({ notes });
  });
  it("leaves out the note a limit sentence already says", () => {
    expect(
      stateLines(
        extra(
          { note: "run limit exceeded: rolled back", notes: ["evicted"] },
          "limit_exceeded",
        ),
      ),
    ).toEqual({ notes: ["Evicted."] });
  });
});

describe("workspace runs", () => {
  const run = {
    run_id: "r1",
    status: "cancelled",
    started_at_ms: 1,
    finished_at_ms: 4201,
    calls_total: 0,
    background: true,
  };
  it("titles runs from description, code preview or id", () => {
    expect(runTitle({ ...run, description: "Sum" }).text).toBe("Sum");
    expect(runTitle({ ...run, code_preview: "x = 1" })).toEqual({
      text: "x = 1",
      mono: true,
    });
    expect(runTitle(run).text).toBe("Run r1");
  });
  it("orders the meta so truncation loses the least", () => {
    expect(runSummary({ ...run, calls_total: 2 })).toEqual({
      lead: "Stopped · 4.2 s",
      tail: "2 calls · background",
    });
    expect(
      runSummary({ ...run, status: "background", finished_at_ms: null }),
    ).toEqual({ lead: "Background", tail: "" });
  });
  it("titles a run from its preview like the card", () => {
    expect(runTitle({ ...run, code_preview: "# Count TODOs" })).toEqual({
      text: "Count TODOs",
      mono: false,
    });
  });
});

describe("htmlLineTokens", () => {
  it("keeps text and only plain colour values", () => {
    expect(
      htmlLineTokens(
        '<span style="color:#F97583">await</span><span style="color:red;background:url(x)"> x</span><span style="color: expression(alert(1))">()</span><span style="color:var(--shiki-x)"><b>!</b></span>',
      ),
    ).toEqual([
      { text: "await", color: "#F97583" },
      { text: " x" },
      { text: "()" },
      { text: "!", color: "var(--shiki-x)" },
    ]);
  });
  it("never turns markup in text into elements", () => {
    expect(htmlLineTokens("&lt;img src=x onerror=1&gt;")).toEqual([
      { text: "<img src=x onerror=1>" },
    ]);
  });
});
