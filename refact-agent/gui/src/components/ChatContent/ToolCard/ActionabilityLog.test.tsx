import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";

import type { ActionabilityDiagnostics } from "../../../services/refact/browser";
import { render } from "../../../utils/test-utils";
import { ActionabilityLog } from "./ActionabilityLog";

const DIAGNOSTICS: ActionabilityDiagnostics = {
  call_log: [
    'waiting for locator("role=button[name=Submit]")',
    'locator resolved to <button class="btn">Submit</button>',
    "attempting click action",
    "element is not stable",
    "retrying click action",
    '<div class="overlay">…</div> intercepts pointer events',
  ],
  timed_out: true,
  elapsed_ms: 5031,
  attempts: 4,
  attached: true,
  visible: true,
  stable: false,
  enabled: true,
  receives_events: false,
  intercepting_element: '<div class="overlay">…</div>',
};

describe("ActionabilityLog", () => {
  test("renders an ordered failed call log with the final reason emphasized", () => {
    render(
      <ActionabilityLog
        diagnostics={DIAGNOSTICS}
        failed
        retryCount={3}
        stepIndex={3}
      />,
    );

    const view = screen.getByTestId("actionability-log");
    expect(
      within(view).getByRole("button", { name: "Step 4 actionability" }),
    ).toHaveAttribute("aria-expanded", "true");
    const entries = within(view).getAllByRole("listitem");
    expect(entries.map((entry) => entry.textContent)).toEqual(
      DIAGNOSTICS.call_log,
    );
    expect(entries.at(-1)).toHaveAttribute("data-final", "true");
    expect(within(view).getByText("5031 ms elapsed")).toBeInTheDocument();
    expect(within(view).getByText("4 attempts")).toBeInTheDocument();
    expect(within(view).getByText("3 retries")).toBeInTheDocument();
  });

  test("reflects pass, fail, and not-checked states with interception context", () => {
    render(<ActionabilityLog diagnostics={DIAGNOSTICS} failed />);

    expect(screen.getByTestId("actionability-state-attached")).toHaveAttribute(
      "data-result",
      "pass",
    );
    expect(screen.getByTestId("actionability-state-stable")).toHaveAttribute(
      "data-result",
      "fail",
    );
    expect(
      screen.getByTestId("actionability-state-receives-events"),
    ).toHaveAttribute("data-result", "fail");
    expect(screen.getByTestId("actionability-state-editable")).toHaveAttribute(
      "data-result",
      "not-checked",
    );
    expect(screen.getByText("Intercepting element")).toBeInTheDocument();
    expect(
      screen.getAllByText(DIAGNOSTICS.intercepting_element ?? ""),
    ).toHaveLength(1);
  });

  test("keeps successful retry logs collapsed until requested", async () => {
    const user = userEvent.setup();
    render(<ActionabilityLog diagnostics={DIAGNOSTICS} failed={false} />);

    const trigger = screen.getByRole("button", { name: "Actionability" });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(
      screen.queryByRole("list", { name: "Actionability call log" }),
    ).not.toBeInTheDocument();

    await user.click(trigger);

    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(
      screen.getByRole("list", { name: "Actionability call log" }),
    ).toBeInTheDocument();
  });

  test("renders nothing for absent or incomplete payloads", () => {
    const { rerender } = render(<ActionabilityLog diagnostics={null} failed />);
    expect(screen.queryByTestId("actionability-log")).not.toBeInTheDocument();

    expect(() =>
      rerender(
        <ActionabilityLog
          diagnostics={{ call_log: ["waiting for locator"] }}
          failed
        />,
      ),
    ).not.toThrow();
    expect(screen.queryByTestId("actionability-log")).not.toBeInTheDocument();
  });
});
