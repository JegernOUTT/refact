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

  it("confirms and saves a long object field as an object", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <SchemaField
        field={{
          key: "credential_command",
          f_type: "string_long",
          f_label: "Credential Command",
          f_object: true,
          f_confirmation: true,
        }}
        value={{ command: "old-command", args: ["token"] }}
        onSave={onSave}
      />,
    );

    const input = screen.getByRole("textbox");
    expect(input).toHaveValue("args:\n  - token\ncommand: old-command");
    fireEvent.change(input, {
      target: { value: '{"command":"new-command","args":["token"]}' },
    });
    fireEvent.blur(input);

    await waitFor(() => {
      expect(confirm).toHaveBeenCalledWith(
        "Save changes to Credential Command?",
      );
      expect(onSave).toHaveBeenCalledWith("credential_command", {
        command: "new-command",
        args: ["token"],
      });
    });
    confirm.mockRestore();
  });

  it("does not save a confirmation field when confirmation is cancelled", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <SchemaField
        field={{
          key: "credential_command",
          f_type: "string_long",
          f_object: true,
          f_confirmation: true,
        }}
        value={{ command: "old-command" }}
        onSave={onSave}
      />,
    );

    const input = screen.getByDisplayValue("command: old-command");
    fireEvent.change(input, { target: { value: "command: new-command" } });
    fireEvent.blur(input);

    expect(confirm).toHaveBeenCalledOnce();
    expect(onSave).not.toHaveBeenCalled();
    confirm.mockRestore();
  });

  it("confirms and clears an object field with null", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <SchemaField
        field={{
          key: "credential",
          f_type: "string_long",
          f_object: true,
          f_confirmation: true,
        }}
        value={{ type: "command", command: "helper" }}
        onSave={onSave}
      />,
    );

    const input = screen.getByRole("textbox");
    fireEvent.change(input, { target: { value: "" } });
    fireEvent.blur(input);

    await waitFor(() => {
      expect(confirm).toHaveBeenCalledOnce();
      expect(onSave).toHaveBeenCalledWith("credential", null);
    });
    confirm.mockRestore();
  });
});
