import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type React from "react";
import { describe, expect, it, vi } from "vitest";

import type { SettingsListEditorItem } from "./SettingsListEditor";
import { SettingsListEditor } from "./SettingsListEditor";

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
});
