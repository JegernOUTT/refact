import { render, screen } from "@testing-library/react";
import path from "node:path";
import { readFile } from "node:fs/promises";
import { Play } from "lucide-react";
import { describe, expect, it } from "vitest";

import styles from "./StatusBadge.module.css";
import {
  getAgentStatusBadgeProps,
  getFileStatusBadgeProps,
  getPriorityStatusBadgeProps,
} from "./statusBadgeRecipe";
import { StatusBadge } from "./StatusBadge";

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
    expect(badge).toHaveAttribute("data-tone", "accent");
    expect(badge.querySelector("svg")).toBeInTheDocument();
  });

  it("pulses only for running status", () => {
    const { rerender } = render(<StatusBadge pulse status="running" />);

    expect(screen.getByLabelText("Agent running")).toHaveClass(styles.pulse);

    rerender(<StatusBadge pulse status="success" />);

    expect(screen.getByLabelText("Agent success")).not.toHaveClass(styles.pulse);
    expect(getAgentStatusBadgeProps("running")).toMatchObject({
      pulse: true,
      tone: "accent",
    });
  });

  it("keeps pulse reduced-motion safe in CSS", async () => {
    const css = await readFile(
      path.resolve(__dirname, "StatusBadge.module.css"),
      "utf8",
    );

    expect(css).toContain("@media (prefers-reduced-motion: reduce)");
    expect(css).toContain("animation: none;");
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
