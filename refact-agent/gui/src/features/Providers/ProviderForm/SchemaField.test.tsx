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
  it.each([false, true])(
    "saves select changes and surfaces failure: %s",
    async (fails) => {
      const onSave = fails
        ? vi.fn().mockRejectedValue(new Error("failed"))
        : vi.fn().mockResolvedValue(undefined);
      const { user } = render(
        <SchemaField
          field={{
            key: "backend",
            f_type: "string",
            f_default: "oauth",
            f_options: [
              { value: "oauth", label: "Subscription" },
              { value: "bedrock", label: "Amazon Bedrock" },
            ],
          }}
          value={undefined}
          onSave={onSave}
        />,
      );
      await user.keyboard("{Tab}{Enter}");
      await user.click(screen.getByRole("option", { name: "Amazon Bedrock" }));
      await waitFor(() =>
        expect(onSave).toHaveBeenCalledWith("backend", "bedrock"),
      );
      expect(
        await screen.findByText(fails ? "Error" : "Saved"),
      ).toBeInTheDocument();
    },
  );

  it("leaves masked secrets unchanged on blur and supports replace and explicit clear", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <SchemaField
        field={{
          key: "aws_secret_access_key",
          f_type: "string",
          f_label: "AWS secret",
        }}
        value="***"
        onSave={onSave}
      />,
    );
    const input = screen.getByLabelText("AWS secret");
    fireEvent.focus(input);
    fireEvent.blur(input);
    expect(onSave).not.toHaveBeenCalled();
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "replacement" } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith(
        "aws_secret_access_key",
        "replacement",
      ),
    );
    await user.click(screen.getByRole("button", { name: "Clear saved value" }));
    await waitFor(() =>
      expect(onSave).toHaveBeenLastCalledWith("aws_secret_access_key", ""),
    );
  });

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

  it("disables secret actions and never saves when disabled", () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <SchemaField
        field={{ key: "api_key", f_type: "string", f_label: "API Key" }}
        value="***"
        disabled
        onSave={onSave}
      />,
    );

    const input = screen.getByLabelText("API Key");
    const reveal = screen.getByRole("button", { name: "Reveal" });
    const clear = screen.getByRole("button", { name: "Clear saved value" });

    expect(input).toBeDisabled();
    expect(input).toHaveAttribute("type", "password");
    expect(reveal).toBeDisabled();
    expect(clear).toBeDisabled();

    fireEvent.click(reveal);
    fireEvent.click(clear);
    fireEvent.blur(input);

    expect(input).toHaveAttribute("type", "password");
    expect(onSave).not.toHaveBeenCalled();
  });
});
