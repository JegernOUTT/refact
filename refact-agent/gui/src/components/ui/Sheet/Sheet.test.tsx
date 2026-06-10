import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const tokensCss = readFileSync(join(process.cwd(), "src/styles/tokens.css"), "utf8");
const sheetCss = readFileSync(
  join(process.cwd(), "src/components/ui/Sheet/Sheet.module.css"),
  "utf8",
);

const tokenValue = (name: string) => {
  const match = tokensCss.match(new RegExp(`${name}:\\s*(\\d+);`));
  return match ? Number(match[1]) : Number.NaN;
};

describe("Sheet", () => {
  it("stacks sheet content above its overlay", () => {
    expect(tokenValue("--rf-z-overlay")).toBeLessThan(
      tokenValue("--rf-z-modal"),
    );
    expect(sheetCss).toMatch(/\.overlay[\s\S]*z-index:\s*var\(--rf-z-overlay, 600\)/);
    expect(sheetCss).toMatch(/\.content[\s\S]*z-index:\s*var\(--rf-z-modal, 700\)/);
  });
});
