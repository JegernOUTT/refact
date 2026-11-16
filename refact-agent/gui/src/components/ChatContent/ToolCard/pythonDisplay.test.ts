import { describe, expect, it } from "vitest";
import {
  approvalCommand,
  editPair,
  formatCallDuration,
  splitPathList,
  clampLines,
  orderedArgEntries,
  parseCutPythonArgs,
  parsePythonArgs,
  prettyJson,
  relativeToRoot,
  summarizePythonArgs,
} from "./pythonDisplay";

const ROOT = "/tmp/project";

describe("summarizePythonArgs", () => {
  it("prefers the command", () => {
    expect(
      summarizePythonArgs(
        JSON.stringify({ path: "/x", command: "sleep 2", workdir: "/x" }),
      ),
    ).toBe("sleep 2");
  });
  it.each([
    [{ path: `${ROOT}/src/a.py`, content: "x" }, "src/a.py"],
    [{ paths: `${ROOT}/a.py,${ROOT}/b.py` }, "a.py, b.py"],
    [{ paths: [`${ROOT}/a.py`, "/elsewhere/b.py"] }, "a.py, /elsewhere/b.py"],
    [{ file_path: `${ROOT}/README.md` }, "README.md"],
    [{ pattern: "**/*.py", max_results: 10 }, "**/*.py"],
    [{ query: "auth flow" }, "auth flow"],
    [{ url: "https://example.com" }, "https://example.com"],
    [{ symbols: "Foo,Bar" }, "Foo,Bar"],
    [{ limit: 3, label: "first string" }, "first string"],
    [{ limit: 3 }, ""],
  ])("summarizes %j", (args, expected) => {
    expect(summarizePythonArgs(JSON.stringify(args), ROOT)).toBe(expected);
  });
  it("cuts long values to about 80 characters on one line", () => {
    const summary = summarizePythonArgs(
      JSON.stringify({ command: `echo ${"a".repeat(200)}\nnext` }),
    );
    expect(Array.from(summary)).toHaveLength(80);
    expect(summary.endsWith("…")).toBe(true);
    expect(summary).not.toContain("\n");
  });
  it("never shows raw JSON for a cut object and salvages known fields", () => {
    const cut = `{"path":"${ROOT}/notes.txt","content":"long text that was cut`;
    expect(summarizePythonArgs(cut, ROOT)).toBe("notes.txt");
    expect(summarizePythonArgs('{"content":"abc')).toBe("abc…");
    expect(summarizePythonArgs('{"a')).toBe("");
  });
  it("shows plain text previews as text", () => {
    expect(summarizePythonArgs("ls -la")).toBe("ls -la");
  });
});

describe("approval helpers", () => {
  it("parses only JSON objects", () => {
    expect(parsePythonArgs('{"a":1}')).toEqual({ a: 1 });
    expect(parsePythonArgs("[1]")).toBeNull();
    expect(parsePythonArgs('{"a":')).toBeNull();
    expect(parsePythonArgs("plain")).toBeNull();
  });
  it("hides a command that only repeats the tool name", () => {
    expect(approvalCommand("create_textdoc", "create_textdoc", null)).toBe(
      undefined,
    );
    expect(approvalCommand(null, "shell", { command: "rm -rf x" })).toBe(
      "rm -rf x",
    );
    expect(approvalCommand("git push", "shell", null)).toBe("git push");
  });
  it("orders command and path keys first", () => {
    expect(
      orderedArgEntries({ content: "c", other: 1, path: "/p", command: "x" }),
    ).toEqual([
      ["command", "x"],
      ["path", "/p"],
      ["content", "c"],
      ["other", 1],
    ]);
  });
  it("relativizes only inside the root", () => {
    expect(relativeToRoot(`${ROOT}/a`, `${ROOT}/`)).toBe("a");
    expect(relativeToRoot(`${ROOT}-other/a`, ROOT)).toBe(`${ROOT}-other/a`);
    expect(relativeToRoot("/a", undefined)).toBe("/a");
  });
});

describe("text helpers", () => {
  it("pretty prints JSON objects only", () => {
    expect(prettyJson('{"a":[1]}')).toBe('{\n  "a": [\n    1\n  ]\n}');
    expect(prettyJson('"src/__init__.py"')).toBeUndefined();
    expect(prettyJson("not json")).toBeUndefined();
  });
  it("clamps lines", () => {
    expect(clampLines("a\nb\nc\n", 2)).toEqual({
      total: 3,
      clipped: true,
      head: "a\nb",
      full: "a\nb\nc",
    });
  });
});

describe("parseCutPythonArgs", () => {
  it("recovers complete members and closes a cut string", () => {
    const cut = `{"path":"${ROOT}/a.py","old_str":"from x import y\\nfrom`;
    expect(parseCutPythonArgs(cut)).toEqual({
      path: `${ROOT}/a.py`,
      old_str: "from x import y\nfrom…",
    });
  });
  it("drops a dangling key", () => {
    expect(parseCutPythonArgs('{"path":"/a","conte')).toEqual({ path: "/a" });
    expect(parseCutPythonArgs('{"path":"/a","content":')).toEqual({
      path: "/a",
    });
    expect(parseCutPythonArgs('{"path":"/a","n":12')).toEqual({
      path: "/a",
      n: 12,
    });
  });
  it("closes nested containers", () => {
    expect(parseCutPythonArgs('{"paths":["/a","/b')).toEqual({
      paths: ["/a", "/b…"],
    });
  });
  it("gives up on hopeless input", () => {
    expect(parseCutPythonArgs('{"a')).toBeNull();
    expect(parseCutPythonArgs("plain")).toBeNull();
  });
});

describe("round 2 helpers", () => {
  it("re-indents JSON without changing numbers, duplicate keys or key order", () => {
    const text =
      '{"z":9007199254740993,"a":1.10,"a":2,"s":"x, y: {z}","e":[],"o":{}}';
    expect(prettyJson(text)).toBe(
      [
        "{",
        '  "z": 9007199254740993,',
        '  "a": 1.10,',
        '  "a": 2,',
        '  "s": "x, y: {z}",',
        '  "e": [],',
        '  "o": {}',
        "}",
      ].join("\n"),
    );
    expect(prettyJson('{"q":"say \\"hi\\", ok"}')).toBe(
      '{\n  "q": "say \\"hi\\", ok"\n}',
    );
  });
  it("relativizes Windows paths and picks the deepest of several roots", () => {
    expect(relativeToRoot("C:\\work\\proj\\src\\a.py", "c:/work/proj")).toBe(
      "src\\a.py",
    );
    expect(relativeToRoot("C:/work/proj/b.py", "C:\\work\\proj\\")).toBe(
      "b.py",
    );
    expect(
      relativeToRoot("/w/a/sub/x.py", ["/elsewhere", "/w/a", "/w/a/sub"]),
    ).toBe("x.py");
    expect(relativeToRoot("/w/b/x.py", ["/w/a", "/w/b"])).toBe("x.py");
    expect(relativeToRoot("/w/abc/x.py", ["/w/a"])).toBe("/w/abc/x.py");
  });
  it("does not split a single path that contains a comma", () => {
    expect(splitPathList("/tmp/a, b.txt")).toEqual(["/tmp/a, b.txt"]);
    expect(splitPathList("/tmp/a.py,/tmp/b.py")).toEqual([
      "/tmp/a.py",
      "/tmp/b.py",
    ]);
    expect(
      summarizePythonArgs(
        JSON.stringify({ paths: `${ROOT}/notes, final.md` }),
        ROOT,
      ),
    ).toBe("notes, final.md");
  });
  it("finds an uncut edit pair and refuses a cut one", () => {
    expect(
      editPair({ path: "/a", old_str: "x\n", replacement: "y\n" }),
    ).toEqual({
      before: "x\n",
      after: "y\n",
    });
    expect(
      editPair({ old_str: "x… (+40 chars)", replacement: "y" }),
    ).toBeUndefined();
    expect(editPair({ old_str: "x", replacement: "y…" })).toBeUndefined();
    expect(editPair({ path: "/a", content: "x" })).toBeUndefined();
  });
  it("formats durations up to hours", () => {
    expect(formatCallDuration(250)).toBe("250 ms");
    expect(formatCallDuration(2500)).toBe("2.5 s");
    expect(formatCallDuration(125_000)).toBe("2 min 5 s");
    expect(formatCallDuration(7_200_000)).toBe("2 h 0 min");
  });
});
