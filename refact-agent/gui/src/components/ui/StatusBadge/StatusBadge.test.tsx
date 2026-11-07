import { render, screen } from "@testing-library/react";
import { Play } from "lucide-react";
import { describe, expect, it } from "vitest";

import { StatusBadge } from "./StatusBadge";
import {
  getFileStatusBadgeProps,
  getPriorityStatusBadgeProps,
} from "./statusBadgeRecipe";

describe("StatusBadge", () => {
  it("renders mapped file status labels with accessible names", () => {
    render(<StatusBadge status="ADDED" />);

    const badge = screen.getByLabelText("Added file");

    expect(badge).toHaveTextContent("Added");
    expect(badge).toHaveAttribute("data-tone", "success");
    expect(getFileStatusBadgeProps("MODIFIED")).toMatchObject({
      ariaLabel: "Modified file",
      tone: "warning",
    });
  });

  it("allows explicit label, aria label, tone, variant, size, and icon", () => {
    render(
      <StatusBadge
        ariaLabel="Worker is queued"
        icon={Play}
        label="Waiting"
        size="md"
        status="queued"
        tone="accent"
        variant="outline"
      />,
    );

    const badge = screen.getByLabelText("Worker is queued");

    expect(badge).toHaveTextContent("Waiting");
  });

  it("maps priority status helpers", () => {
    expect(getPriorityStatusBadgeProps("critical")).toMatchObject({
      ariaLabel: "Critical priority",
      tone: "danger",
    });
    expect(getPriorityStatusBadgeProps("high")).toMatchObject({
      ariaLabel: "High priority",
      tone: "warning",
    });
  });
});
