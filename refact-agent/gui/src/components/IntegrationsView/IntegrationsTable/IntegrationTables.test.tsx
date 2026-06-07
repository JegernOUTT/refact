import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "../../../utils/test-utils";
import { KeyValueTable } from "./KeyValueTable";
import { ParametersTable } from "./ParametersTable";

function inputs(container: HTMLElement, field: string) {
  return Array.from(
    container.querySelectorAll<HTMLInputElement>(`input[data-field="${field}"]`),
  );
}

describe("integration table editors", () => {
  it("adds and removes key/value rows", async () => {
    const onChange = vi.fn();
    const { user } = render(
      <KeyValueTable initialData={{ EXISTING: "value" }} onChange={onChange} />,
    );

    await user.click(screen.getByRole("button", { name: /add row/i }));

    await waitFor(() => {
      expect(onChange).toHaveBeenLastCalledWith({ EXISTING: "value", "1": "" });
    });

    const removeButtons = screen.getAllByRole("button", { name: "Remove" });
    await user.click(removeButtons[0]);

    await waitFor(() => {
      expect(onChange).toHaveBeenLastCalledWith({ "1": "" });
    });
  });

  it("advances to the next parameter cell on Enter", () => {
    const onToolParameters = vi.fn();
    const { container } = render(
      <ParametersTable
        initialData={[
          { name: "first_name", description: "First", type: "string" },
          { name: "second_name", description: "Second", type: "string" },
        ]}
        onToolParameters={onToolParameters}
      />,
    );

    const nameInputs = inputs(container, "name");
    nameInputs[0].focus();

    fireEvent.keyDown(nameInputs[0], { key: "Enter" });

    expect(document.activeElement).toBe(nameInputs[1]);
    expect(onToolParameters).not.toHaveBeenCalled();
  });

  it("validates parameter names as snake_case", async () => {
    const onToolParameters = vi.fn();
    const { container, user } = render(
      <ParametersTable
        initialData={[{ name: "valid_name", description: "", type: "string" }]}
        onToolParameters={onToolParameters}
      />,
    );

    const nameInput = inputs(container, "name")[0];
    await user.clear(nameInput);
    await user.type(nameInput, "BadName");
    fireEvent.blur(nameInput);

    expect(
      await screen.findByText('The value "BadName" must be written in snake case.'),
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(onToolParameters).toHaveBeenLastCalledWith([
        { name: "BadName", description: "", type: "string" },
      ]);
    });
  });
});
