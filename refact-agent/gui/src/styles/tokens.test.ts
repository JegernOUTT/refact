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

  it("uses a softer dark accent value and matching soft tint", () => {
    expect(tokensCss).toContain("--rf-color-accent: #7f93d8;");
    expect(tokensCss).toContain(
      "--rf-color-accent-soft: rgba(127, 147, 216, 0.12);",
    );
    expect(tokensCss).toContain("var(--rf-color-accent) 12%,");
    expect(tokensCss).toContain("--rf-focus-ring: rgba(127, 147, 216, 0.55);");
  });

  it("overrides Radix blue accent variables in dark appearance", () => {
    expect(tokensCss).toContain(
      '[data-appearance="dark"][data-accent-color="blue"]',
    );
    expect(tokensCss).toContain("--accent-9: var(--rf-color-accent);");
    expect(tokensCss).toContain("--blue-9: var(--rf-color-accent);");
  });

  it("keeps light theme accent bound to Radix", () => {
    expect(tokensCss).toContain("--rf-color-accent: var(--accent-9, #006adc);");
  });
});
