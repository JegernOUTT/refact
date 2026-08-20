import { readFile } from "node:fs/promises";
import path from "node:path";

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type React from "react";
import { describe, expect, it, vi } from "vitest";

import { SettingsListEditor } from "./SettingsListEditor";
import type { SettingsListEditorItem } from "./SettingsListEditor";

const items: SettingsListEditorItem[] = [
  { id: "a", value: "sudo" },
  { id: "b", value: "doas" },
];

function renderEditor(
  overrides: Partial<React.ComponentProps<typeof SettingsListEditor>> = {},
) {
  const onAdd = vi.fn();
  const onChange = vi.fn();
  const onRemove = vi.fn();

  render(
    <SettingsListEditor
      addLabel="Add rule"
      items={items}
      onAdd={onAdd}
      onChange={onChange}
      onRemove={onRemove}
      {...overrides}
    />,
  );

  return { onAdd, onChange, onRemove };
}

describe("SettingsListEditor", () => {
  it("renders one input per item", () => {
    renderEditor();

    expect(screen.getByDisplayValue("sudo")).toBeInTheDocument();
    expect(screen.getByDisplayValue("doas")).toBeInTheDocument();
  });

  it("reports edits with the item id and the next value", async () => {
    const user = userEvent.setup();
    const { onChange } = renderEditor();

    await user.type(screen.getByDisplayValue("sudo"), "x");

    expect(onChange).toHaveBeenCalledWith("a", "sudox");
  });

  it("removes and adds items", async () => {
    const user = userEvent.setup();
    const { onAdd, onRemove } = renderEditor();

    await user.click(screen.getByRole("button", { name: "Remove doas" }));

    expect(onRemove).toHaveBeenCalledWith("b");

    await user.click(screen.getByRole("button", { name: "Add rule" }));

    expect(onAdd).toHaveBeenCalledTimes(1);
  });

  it("uses itemAriaLabel for the remove control when provided", () => {
    renderEditor({
      itemAriaLabel: (item, index) => `Delete rule ${index + 1}: ${item.value}`,
    });

    expect(
      screen.getByRole("button", { name: "Delete rule 1: sudo" }),
    ).toBeInTheDocument();
  });

  it("hides remove and add controls and disables inputs in read-only mode", () => {
    renderEditor({ readOnly: true });

    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    expect(screen.getByDisplayValue("sudo")).toBeDisabled();
    expect(screen.getByDisplayValue("doas")).toBeDisabled();
  });

  it("renders the empty label when there are no items", () => {
    renderEditor({ emptyLabel: "No rules yet.", items: [] });

    expect(screen.getByText("No rules yet.")).toBeInTheDocument();
  });

  it("keeps rows fluid and pins the remove control to the icon size token", async () => {
    const css = await readFile(
      path.resolve(__dirname, "SettingsListEditor.module.css"),
      "utf8",
    );

    const inputBlock = /\.input\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";

    expect(inputBlock).toContain("width: 100%;");
    expect(inputBlock).toContain("height: var(--rf-control-h);");
    expect(inputBlock).not.toMatch(/max-width/);
    expect(css).not.toMatch(/max-width/);
    expect(css).toContain("grid-template-columns: minmax(0, 1fr) auto;");
    expect(css).toContain("height: var(--rf-control-h-icon-sm);");
    expect(css).toContain("font-family: var(--rf-font-mono);");
  });
});
