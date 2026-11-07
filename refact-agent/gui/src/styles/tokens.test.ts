import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const tokensCss = readFileSync(
  join(process.cwd(), "src/styles/tokens.css"),
  "utf8",
);

const tokenValue = (name: string) => {
  const match = tokensCss.match(new RegExp(`${name}:\\s*(\\d+);`));
  return match ? Number(match[1]) : Number.NaN;
};

describe("design tokens", () => {
  it("elevates only modal-owned transient overlays above modal content", () => {
    expect(tokenValue("--rf-z-popover")).toBeLessThan(
      tokenValue("--rf-z-modal"),
    );
    expect(tokenValue("--rf-z-popover-modal")).toBeGreaterThan(
      tokenValue("--rf-z-modal"),
    );
    expect(tokenValue("--rf-z-tooltip-modal")).toBeGreaterThan(
      tokenValue("--rf-z-popover-modal"),
    );
  });

  it("keeps the dark focus-ring token defined", () => {
    expect(tokensCss).toContain("--rf-focus-ring: rgba(127, 147, 216, 0.55);");
  });
});
