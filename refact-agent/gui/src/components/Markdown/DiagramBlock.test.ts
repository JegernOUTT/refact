import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";

const stylesheet = readFileSync(
  "src/components/Markdown/DiagramBlock.module.css",
  "utf8",
);

function readRule(selector: string): string {
  const start = stylesheet.indexOf(`${selector} {`);
  const end = stylesheet.indexOf("}\n", start);
  return stylesheet.slice(start, end + 1);
}

describe("DiagramBlock sizing", () => {
  test("uses intrinsic dimensions and scrolls oversized diagrams", () => {
    const container = readRule(".diagram_container");
    expect(container).toContain("width: fit-content");
    expect(container).toContain("var(--rf-control-h-icon-sm)");
    expect(container).toContain("var(--rf-control-h-lg)");

    const canvas = readRule(".diagram_canvas");
    expect(canvas).toContain("width: fit-content");
    expect(canvas).toContain("max-height: clamp(240px, 70vh, 720px)");
    expect(canvas).toContain("overflow-y: auto");
    expect(canvas).not.toMatch(/^\s+height:/mu);

    const toolbar = readRule(".diagram_toolbar");
    expect(toolbar).toContain("position: absolute");

    expect(readRule(".diagram_canvas:focus-visible")).toContain(
      "var(--rf-focus-ring-w)",
    );
  });
});
