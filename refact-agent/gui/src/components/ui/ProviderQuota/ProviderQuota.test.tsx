import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import styles from "./ProviderQuota.module.css";
import { ProviderQuota } from "./ProviderQuota";

describe("ProviderQuota", () => {
  it("renders zero usage with accessible progress semantics", () => {
    render(<ProviderQuota label="Tokens" usedPercent={0} value="0 / 1,000" />);

    const progress = screen.getByRole("progressbar", { name: "Tokens" });
    expect(progress).toHaveAttribute("aria-valuemin", "0");
    expect(progress).toHaveAttribute("max", "100");
    expect(progress).toHaveAttribute("value", "0");
    expect(progress).toHaveAttribute("aria-valuenow", "0");
  });

  it("renders partial usage without changing the supplied percentage", () => {
    render(
      <ProviderQuota label="Requests" usedPercent={37.5} value="375 / 1,000" />,
    );

    const progress = screen.getByRole("progressbar", { name: "Requests" });
    expect(progress).toHaveAttribute("value", "37.5");
    expect(progress).toHaveAttribute("aria-valuenow", "37.5");
  });

  it("clamps the visual and accessible percentage above 100", () => {
    render(
      <ProviderQuota
        label="Storage"
        usedPercent={125}
        value="125 GB / 100 GB"
      />,
    );

    const progress = screen.getByRole("progressbar", { name: "Storage" });
    expect(progress).toHaveAttribute("value", "100");
    expect(progress).toHaveAttribute("aria-valuenow", "100");
  });

  it("omits progress when no percentage is supplied", () => {
    render(<ProviderQuota label="Credits" value="Unlimited" />);

    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.getByText("Unlimited")).toBeInTheDocument();
  });

  it("renders tone, meta, and badge presentation", () => {
    render(
      <ProviderQuota
        badge={<span>Monthly</span>}
        label="Compute"
        meta="Resets in 4 days"
        tone="warning"
        usedPercent={80}
        value="80%"
      />,
    );

    const root = screen.getByText("Compute").closest(`.${styles.root}`);
    expect(root).toHaveClass(styles.toneWarning);
    expect(root).toHaveAttribute("data-tone", "warning");
    expect(screen.getByText("Resets in 4 days")).toHaveClass(styles.meta);
    expect(screen.getByText("Monthly").parentElement).toHaveClass(styles.badge);
  });
});
