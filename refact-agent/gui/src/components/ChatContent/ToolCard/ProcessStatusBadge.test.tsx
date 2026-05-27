import { describe, expect, test } from "vitest";
import { render, screen } from "@testing-library/react";
import { Theme } from "@radix-ui/themes";

import { ProcessStatusBadge } from "./ProcessStatusBadge";

describe("ProcessStatusBadge", () => {
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
