import { describe, expect, it } from "vitest";
import path from "node:path";
import { readFile } from "node:fs/promises";
import { screen } from "@testing-library/react";
import { Circle } from "lucide-react";

import { render } from "../../../utils/test-utils";
import { Icon } from "../Icon";
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

function CustomLabel() {
  return <span>Custom component label</span>;
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

  it("auto-detects intrinsic svg and shared Icon labels as icon-only", () => {
    render(
      <SegmentedControl
        options={[
          {
            value: "svg",
            label: (
              <svg aria-hidden="true" viewBox="0 0 16 16">
                <circle cx="8" cy="8" r="6" />
              </svg>
            ),
          },
          { value: "icon", label: <Icon icon={Circle} size="sm" /> },
        ]}
        value="icon"
        onValueChange={() => undefined}
      />,
    );

    expect(screen.getByRole("radio", { name: "svg" })).not.toBeChecked();
    expect(screen.getByRole("radio", { name: "icon" })).toBeChecked();
  });

  it("does not infer icon-only layout for custom component labels", () => {
    render(
      <SegmentedControl
        options={[
          { value: "custom", label: <CustomLabel /> },
          { value: "text", label: "Text label" },
        ]}
        value="custom"
        onValueChange={() => undefined}
      />,
    );

    expect(
      screen.getByRole("radio", { name: "Custom component label" }),
    ).toBeChecked();
    expect(screen.queryByRole("radio", { name: "custom" })).toBeNull();
  });

  it("renders an empty disabled root without an indicator", () => {
    const { container } = render(
      <SegmentedControl
        aria-label="Empty segments"
        options={[]}
        value="missing"
        onValueChange={() => undefined}
      />,
    );

    const root = screen.getByRole("radiogroup", { name: "Empty segments" });

    expect(root).toHaveAttribute("aria-disabled", "true");
    expect(root).toHaveStyle({
      "--rf-segment-count": "1",
      "--rf-segment-index": "0",
    });
    expect(container.querySelector("span[aria-hidden='true']")).toBeNull();
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

  it("sizes to content while keeping equal segment tracks without gutter tails", async () => {
    const css = await readFile(
      path.resolve(__dirname, "SegmentedControl.module.css"),
      "utf8",
    );
    const root = css.match(/\.root \{[^}]+\}/)?.[0] ?? "";
    const segment = css.match(/\.segment \{[^}]+\}/)?.[0] ?? "";
    const label = css.match(/\.label \{[^}]+\}/)?.[0] ?? "";

    expect(root).toContain("display: inline-grid;");
    expect(root).toContain("grid-auto-columns: minmax(0, 1fr);");
    expect(root).toContain("max-width: 100%;");
    expect(segment).toContain("display: flex;");
    expect(label).toContain("overflow: hidden;");
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
