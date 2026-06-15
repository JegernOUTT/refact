import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "../../../utils/test-utils";
import { SchemaField } from "./SchemaField";

function textInput() {
  const input = screen.getByDisplayValue("initial");
  if (!(input instanceof HTMLInputElement)) {
    throw new Error("Expected text input");
  }
  return input;
}

describe("SchemaField", () => {
  it("commits string values on blur, not on each keystroke", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <SchemaField
        field={{ key: "base_url", f_type: "string", f_label: "Base URL" }}
        value="initial"
        onSave={onSave}
      />,
    );

    const input = textInput();
    await user.clear(input);
    await user.type(input, "updated");

    expect(onSave).not.toHaveBeenCalled();

    fireEvent.blur(input);

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith("base_url", "updated");
    });
  });

  it("commits numeric values on blur with integer coercion", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <SchemaField
        field={{ key: "max_output", f_type: "integer", f_label: "Max output" }}
        value={10}
        onSave={onSave}
      />,
    );

    const input = screen.getByDisplayValue("10");
    await user.clear(input);
    await user.type(input, "12.8");

    expect(onSave).not.toHaveBeenCalled();

    fireEvent.blur(input);

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith("max_output", 12);
    });
  });
});
