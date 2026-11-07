import { Theme } from "@radix-ui/themes";
import { render, screen } from "@testing-library/react";
import { describe, expect, test } from "vitest";

import { ProcessStatusBadge } from "./ProcessStatusBadge";

describe("ProcessStatusBadge", () => {
  test.each([
    ["running", "running"],
    ["running_in_background", "background"],
  ] as const)("%s status renders distinctly", (status, label) => {
    render(
      <Theme>
        <ProcessStatusBadge status={status} />
      </Theme>,
    );

    const badge = screen.getByTestId(`exec-status-${status}`);
    expect(badge).toHaveTextContent(label);
  });

  test("unknown process status renders a neutral fallback", () => {
    render(
      <Theme>
        <ProcessStatusBadge status="paused" />
      </Theme>,
    );

    expect(screen.getByTestId("exec-status-paused")).toHaveTextContent(
      "unknown",
    );
  });
});
