import { describe, expect, it } from "vitest";
import path from "node:path";
import { readFile } from "node:fs/promises";
import { screen } from "@testing-library/react";

import { render } from "../../../utils/test-utils";
import { SettingItem } from "./SettingItem";

function getSettingItem(title: string) {
  const heading = screen.getByRole("heading", { name: title });
  return heading.closest("div[class*='item']");
}

describe("SettingItem", () => {
  it("renders copy, control, and save status", () => {
    render(
      <SettingItem
        title="Theme"
        description="Choose a display theme."
        saveStatus="saved"
        control={<button type="button">Change theme</button>}
      />,
    );

    expect(screen.getByRole("heading", { name: "Theme" })).toBeTruthy();
    expect(screen.getByText("Choose a display theme.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Change theme" })).toBeTruthy();
    expect(screen.getByRole("status")).toHaveTextContent("Saved");
  });

  it("uses row layout by default and supports explicit stack layout", () => {
    render(
      <>
        <SettingItem
          title="Row item"
          control={<button type="button">Row</button>}
        />
        <SettingItem
          layout="stack"
          title="Stack item"
          control={<button type="button">Stack</button>}
        />
      </>,
    );

    expect(getSettingItem("Row item")?.className).toContain("row");
    expect(getSettingItem("Stack item")?.className).toContain("stack");
  });

  it("keeps the row fluid and bounds width with absolute lengths only", async () => {
    const css = await readFile(
      path.resolve(__dirname, "SettingItem.module.css"),
      "utf8",
    );

    const itemBlock = /\.item\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";

    // Fluid row doctrine: the row fills its section grid track, so `.item`
    // carries no max-width at all (neither the old 52rem measure cap nor a
    // percentage cap, which would make intrinsic sizing cyclic).
    expect(itemBlock).not.toMatch(/max-width/);
    expect(css).not.toContain("max-width: var(--rf-settings-measure);");

    // Containment still comes from min-width: 0 plus the control cap.
    expect(itemBlock).toContain("min-width: 0;");
    expect(css).toContain("--rf-setting-item-control-max: 360px;");
    expect(css).toContain("max-width: var(--rf-setting-item-control-max);");

    // Any max-width that remains must be an absolute length (never a
    // percentage) so long setting lists stay cheap to lay out.
    expect(css).not.toMatch(/max-width:\s*(min\(\s*)?[\d.]*%/);
    expect(css).not.toMatch(/max-width:\s*(min|max|clamp)\(/);
  });
});
