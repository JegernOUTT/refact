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

  it("bounds width with absolute lengths so long setting lists stay cheap to lay out", async () => {
    const css = await readFile(
      path.resolve(__dirname, "SettingItem.module.css"),
      "utf8",
    );

    expect(css).toContain("max-width: var(--rf-settings-measure);");
    expect(css).not.toMatch(/max-width:\s*(min\(\s*)?100%/);
  });
});
