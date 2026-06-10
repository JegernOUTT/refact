import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "../../../utils/test-utils";
import { KeyValueTable } from "./KeyValueTable";
import { ParametersTable } from "./ParametersTable";

function inputs(container: HTMLElement, field: string) {
  return Array.from(
    container.querySelectorAll<HTMLInputElement>(
      `input[data-field="${field}"]`,
    ),
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

  it("uses fresh key/value row identity after add delete add", async () => {
    const onChange = vi.fn();
    const { container, user } = render(
      <KeyValueTable initialData={{ EXISTING: "value" }} onChange={onChange} />,
    );

    await user.click(screen.getByRole("button", { name: /add row/i }));
    await waitFor(() => {
      expect(onChange).toHaveBeenLastCalledWith({ EXISTING: "value", "1": "" });
    });

    const firstAddedRowId = inputs(container, "key")[1].dataset.rowId;
    await user.click(screen.getAllByRole("button", { name: "Remove" })[0]);
    await waitFor(() => {
      expect(onChange).toHaveBeenLastCalledWith({ "1": "" });
    });

    await user.click(screen.getByRole("button", { name: /add row/i }));
    await waitFor(() => {
      expect(onChange).toHaveBeenLastCalledWith({ "1": "", "2": "" });
    });

    const keyInputs = inputs(container, "key");
    expect(keyInputs.map((input) => input.value)).toEqual(["1", "2"]);
    expect(keyInputs[0].dataset.rowId).toBe(firstAddedRowId);
    expect(keyInputs[1].dataset.rowId).not.toBe(firstAddedRowId);
  });

  it("validates duplicate key/value rows without overwriting data", async () => {
    const onChange = vi.fn();
    const { container, user } = render(
      <KeyValueTable
        initialData={{ FIRST: "one", SECOND: "two" }}
        onChange={onChange}
      />,
    );

    const keyInputs = inputs(container, "key");
    await user.clear(keyInputs[1]);
    await user.type(keyInputs[1], "FIRST");

    expect(
      await screen.findAllByText('Duplicate key "FIRST" is already used.'),
    ).toHaveLength(2);
    expect(onChange).not.toHaveBeenCalledWith({ FIRST: "two" });
    expect(onChange).not.toHaveBeenCalledWith({ FIRST: "one" });

    await user.clear(keyInputs[1]);
    await user.type(keyInputs[1], "THIRD");

    await waitFor(() => {
      expect(onChange).toHaveBeenLastCalledWith({ FIRST: "one", THIRD: "two" });
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
      await screen.findByText(
        'The value "BadName" must be written in snake case.',
      ),
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(onToolParameters).toHaveBeenLastCalledWith([
        { name: "BadName", description: "", type: "string" },
      ]);
    });
  });
});
