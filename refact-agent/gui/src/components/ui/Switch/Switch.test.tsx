import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";

import { render } from "../../../utils/test-utils";
import { Switch } from "./Switch";

describe("Switch", () => {
  it("renders a labelled compact switch", () => {
    render(<Switch label="Enable tools" defaultChecked />);

    const control = screen.getByRole("switch", { name: "Enable tools" });

    expect(control).toHaveAttribute("data-state", "checked");
    expect(control).toHaveAttribute("aria-checked", "true");
  });

  it("supports disabled state", () => {
    render(<Switch label="Feature flag" disabled />);

    const control = screen.getByRole("switch", { name: "Feature flag" });

    expect(control).toBeDisabled();
    expect(control).toHaveAttribute("data-disabled", "");
  });

  it("keeps the track border and checked fill distinct with a small thumb shadow", () => {
    const css = readFileSync(
      path.resolve(__dirname, "Switch.module.css"),
      "utf8",
    );
    const root = css.match(/\.root \{[^}]+\}/)?.[0] ?? "";
    const checked =
      css.match(/\.root\[data-state="checked"\] \{[^}]+\}/)?.[0] ?? "";
    const thumb = css.match(/\.thumb \{[^}]+\}/)?.[0] ?? "";

    expect(root).toContain(
      "border: var(--rf-hairline) solid var(--rf-color-muted);",
    );
    expect(checked).toContain("background: var(--rf-color-accent);");
    expect(thumb).toContain("box-shadow: var(--rf-elev-panel);");
    expect(css).not.toContain("--rf-elev-overlay");
    expect(css).toContain("outline-offset: var(--rf-focus-ring-offset);");
  });
});
