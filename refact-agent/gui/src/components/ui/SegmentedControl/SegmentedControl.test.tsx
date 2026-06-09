import { describe, expect, it } from "vitest";
import path from "node:path";
import { readFile } from "node:fs/promises";
import { screen } from "@testing-library/react";

import { render } from "../../../utils/test-utils";
import { SegmentedControl } from "./SegmentedControl";

const options = [
  { value: "compact", label: "Compact" },
  { value: "regular", label: "Regular" },
  { value: "roomy", label: "Roomy" },
];

function GlobeIcon() {
  return (
    <svg aria-hidden="true" data-testid="globe-icon" viewBox="0 0 16 16">
      <circle cx="8" cy="8" r="6" />
    </svg>
  );
}

describe("SegmentedControl", () => {
  it("renders icon-only segments with accessible names", () => {
    render(
      <SegmentedControl
        options={[
          {
            value: "global",
            label: <GlobeIcon />,
            iconOnly: true,
            ariaLabel: "Global scope",
          },
          { value: "project", label: "Project" },
        ]}
        value="global"
        onValueChange={() => undefined}
      />,
    );

    expect(screen.getByRole("radio", { name: "Global scope" })).toBeChecked();
    expect(screen.getByTestId("globe-icon")).toBeInTheDocument();
  });

  it("positions the selected indicator from segment variables", () => {
    const { container } = render(
      <SegmentedControl
        options={options}
        value="regular"
        onValueChange={() => undefined}
      />,
    );

    const root = screen.getByRole("radiogroup");
    const indicator = container.querySelector("span[aria-hidden='true']");

    expect(root).toHaveStyle({
      "--rf-segment-count": "3",
      "--rf-segment-index": "1",
    });
    expect(indicator).not.toBeNull();
  });

  it("keeps text segments labelled by their visible text", () => {
    render(
      <SegmentedControl
        options={options}
        value="compact"
        onValueChange={() => undefined}
      />,
    );

    expect(screen.getByRole("radio", { name: "Compact" })).toBeChecked();
    expect(screen.getByRole("radio", { name: "Regular" })).not.toBeChecked();
    expect(screen.getByText("Roomy")).toBeVisible();
  });

  it("aligns indicator geometry with the shared segment content inset", async () => {
    const css = await readFile(
      path.resolve(__dirname, "SegmentedControl.module.css"),
      "utf8",
    );

    expect(css).toContain("--rf-segment-padding: var(--rf-space-2xs);");
    expect(css).toContain("top: var(--rf-segment-padding);");
    expect(css).toContain("left: var(--rf-segment-padding);");
    expect(css).toContain("2 * var(--rf-segment-padding)");
    expect(css).toContain("gap: var(--rf-space-1);");
    expect(css).toContain("line-height: 0;");
  });
});
