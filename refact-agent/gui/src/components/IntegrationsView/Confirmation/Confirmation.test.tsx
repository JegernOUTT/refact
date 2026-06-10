import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, waitFor } from "../../../utils/test-utils";
import type { IntegrationFieldValue, ToolConfirmation } from "../../../services/refact";
import { Confirmation } from "./Confirmation";

function getInput(container: HTMLElement, field: keyof ToolConfirmation) {
  const input = container.querySelector<HTMLInputElement>(
    `input[data-field="${field}"]`,
  );

  if (!input) {
    throw new Error(`Missing ${field} input`);
  }

  return input;
}

describe("Confirmation", () => {
  it("preserves edits across confirmation tables", async () => {
    const serverConfirmation: ToolConfirmation = {
      ask_user: ["ask-server"],
      deny: ["deny-server"],
    };
    const onChange = vi.fn();

    function Wrapper() {
      const [confirmationByUser, setConfirmationByUser] =
        useState<ToolConfirmation | null>(null);

      return (
        <Confirmation
          confirmationByUser={confirmationByUser}
          confirmationFromValues={serverConfirmation}
          defaultConfirmationObject={{
            ask_user_default: ["ask-default"],
            deny_default: ["deny-default"],
          }}
          onChange={(fieldKey, fieldValue: IntegrationFieldValue) => {
            onChange(fieldKey, fieldValue);
            setConfirmationByUser(fieldValue as ToolConfirmation);
          }}
        />
      );
    }

    const { container } = render(<Wrapper />);

    const askUserInput = getInput(container, "ask_user");
    expect(askUserInput).toHaveValue("ask-server");
    expect(getInput(container, "deny")).toHaveValue("deny-server");

    fireEvent.change(askUserInput, { target: { value: "ask-edited" } });

    await waitFor(() => {
      expect(onChange).toHaveBeenLastCalledWith("confirmation", {
        ask_user: ["ask-edited"],
        deny: ["deny-server"],
      });
    });

    const denyInput = getInput(container, "deny");

    fireEvent.change(denyInput, { target: { value: "deny-edited" } });

    await waitFor(() => {
      expect(onChange).toHaveBeenLastCalledWith("confirmation", {
        ask_user: ["ask-edited"],
        deny: ["deny-edited"],
      });
    });
  });

  it("renders default confirmation when user and server values are absent", () => {
    const onChange = vi.fn();
    const { container } = render(
      <Confirmation
        confirmationByUser={null}
        confirmationFromValues={null}
        defaultConfirmationObject={{
          ask_user_default: ["ask-default"],
          deny_default: ["deny-default"],
        }}
        onChange={onChange}
      />,
    );

    expect(getInput(container, "ask_user")).toHaveValue("ask-default");
    expect(getInput(container, "deny")).toHaveValue("deny-default");
  });
});
