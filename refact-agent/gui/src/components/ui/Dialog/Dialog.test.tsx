import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const tokensCss = readFileSync(
  join(process.cwd(), "src/styles/tokens.css"),
  "utf8",
);
const dialogCss = readFileSync(
  join(process.cwd(), "src/components/ui/Dialog/Dialog.module.css"),
  "utf8",
);

const tokenValue = (name: string) => {
  const match = tokensCss.match(new RegExp(`${name}:\\s*(\\d+);`));
  return match ? Number(match[1]) : Number.NaN;
};

describe("Dialog", () => {
  it("stacks modal content above its overlay", () => {
    expect(tokenValue("--rf-z-overlay")).toBeLessThan(
      tokenValue("--rf-z-modal"),
    );
    expect(dialogCss).toMatch(
      /\.overlay[\s\S]*z-index:\s*var\(--rf-z-overlay, 600\)/,
    );
    expect(dialogCss).toMatch(
      /\.content[\s\S]*z-index:\s*var\(--rf-z-modal, 700\)/,
    );
  });

  it("clamps height to the viewport without an invalid min(auto) fallback", () => {
    expect(dialogCss).not.toContain("var(--rf-overlay-max-height, auto)");
    expect(dialogCss).toContain(
      "--rf-overlay-viewport-max-height: calc(100dvh - var(--rf-space-5))",
    );
    expect(dialogCss).toMatch(
      /max-height:\s*min\(\s*var\(--rf-overlay-ideal-height\),\s*var\(--rf-overlay-viewport-max-height\)/,
    );
  });
});
